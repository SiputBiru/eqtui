// Copyright (C) 2026 SiputBiru <hillsforrest03@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use std::cell::Cell;
use std::mem;
use std::os::raw::c_void;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use pipewire::channel::Receiver;
use pipewire::context::ContextRc;
use pipewire::main_loop::MainLoopRc;

use crate::pipeline::Pipeline;
use crate::state::{DeviceClass, NodeInfo, PwCommand, PwEvent};

use super::filter::{FilterHandle, create_eq_filter};
use super::links::{
    check_null_sink_input_source, create_device_output_links, remove_device_output_links,
};
use super::null_sink::{NullSinkHandle, NullSinkListenerData, bound_cb, create_null_sink};

pub fn run(tx: mpsc::Sender<PwEvent>, rx: Receiver<PwCommand>, pipeline: Arc<Pipeline>) {
    let mainloop = match MainLoopRc::new(None) {
        Ok(ml) => ml,
        Err(e) => {
            let _ = tx.send(PwEvent::Error(format!("mainloop: {e}")));
            return;
        }
    };

    let context = match ContextRc::new(&mainloop, None) {
        Ok(ctx) => ctx,
        Err(e) => {
            let _ = tx.send(PwEvent::Error(format!("context: {e}")));
            return;
        }
    };

    let core = match context.connect_rc(None) {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(PwEvent::Error(format!("connect: {e}")));
            return;
        }
    };

    let registry = match core.get_registry_rc() {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(PwEvent::Error(format!("registry: {e}")));
            return;
        }
    };

    let nodes: Rc<std::cell::RefCell<Vec<NodeInfo>>> = Rc::new(std::cell::RefCell::new(Vec::new()));

    let nodes_reg_add = nodes.clone();
    let nodes_reg_rem = nodes.clone();
    let _reg_listener = registry
        .add_listener_local()
        .global(move |global| {
            if let Some(props) = &global.props {
                let class = props.get(&pipewire::keys::MEDIA_CLASS).unwrap_or("");
                if class == "Audio/Sink" || class == "Audio/Source" {
                    let name = props
                        .get(&pipewire::keys::NODE_NAME)
                        .unwrap_or("?")
                        .to_string();
                    let description = props
                        .get(&pipewire::keys::NODE_DESCRIPTION)
                        .unwrap_or("")
                        .to_string();

                    let device_class = if class == "Audio/Source" {
                        DeviceClass::Input
                    } else if name.to_lowercase().contains("headphone")
                        || name.to_lowercase().contains("headset")
                        || description.to_lowercase().contains("headphone")
                        || description.to_lowercase().contains("headset")
                    {
                        DeviceClass::Headphone
                    } else {
                        DeviceClass::Speaker
                    };

                    nodes_reg_add.borrow_mut().push(NodeInfo {
                        id: global.id,
                        name,
                        description,
                        class: device_class,
                    });
                }
            }
        })
        .global_remove(move |id| {
            nodes_reg_rem.borrow_mut().retain(|n| n.id != id);
        })
        .register();

    let tx_snapshot = tx.clone();
    let nodes_timer = nodes.clone();

    // Declare cells BEFORE the timer so they can be captured.
    // Use Rc so the timer closure and the null-sink listener can share.
    let null_sink_id_cell: Rc<Cell<Option<u32>>> = Rc::new(Cell::new(None));

    let ns_timer = null_sink_id_cell.clone();
    let timer = mainloop.loop_().add_timer(move |_| {
        let list: Vec<NodeInfo> = nodes_timer.borrow().iter().cloned().collect();
        let _ = tx_snapshot.send(PwEvent::NodeList(list));

        // Poll whether an audio source is linked to the null sink's
        // playback ports.  `pw-link -I` lists all links as
        //   {out_id}:{out_port} -> {in_id}:{in_port}
        // Checks if any link targets the null sink's input.
        if let Some(ns_id) = ns_timer.get() {
            let has_source = check_null_sink_input_source(ns_id);
            let _ = tx_snapshot.send(PwEvent::NullSinkInputState { has_source });
        }
    });
    timer.update_timer(Some(Duration::from_millis(500)), None);

    let core_raw = core.as_raw_ptr().cast::<pipewire_sys::pw_core>();
    let filter_cell: Cell<Option<FilterHandle>> = Cell::new(None);
    let nullsink_cell: Cell<Option<NullSinkHandle>> = Cell::new(None);

    // Create the virtual null-audio-sink BEFORE the equalizer filter.
    // Attaching a proxy-listener that fires when the proxy is bound to a
    // server-side global id; that callback then creates the filter wired
    // to the null sink's monitor ports. This ordering ensures wiremix can
    // discover eqtui as a selectable Audio/Sink.
    let nullsink_handle = create_null_sink(core_raw, &tx);

    if let Some(mut handle) = nullsink_handle {
        // Heap-allocate listener data. This Box is leaked into a raw
        // pointer and freed during shutdown (NullSinkHandle::destroy).
        let listener_data = Box::new(NullSinkListenerData {
            tx: tx.clone(),
            core_raw,
            pipeline: pipeline.clone(),
            // Safety: cell pointers live on the stack in run(), which
            // outlives the mainloop (only quits on Terminate).
            filter_cell_ptr: (&raw const filter_cell).cast_mut(),
            null_sink_id_cell_ptr: Rc::as_ptr(&null_sink_id_cell).cast_mut(),
            filter_created: Cell::new(false),
        });
        let data_ptr = Box::into_raw(listener_data);

        // Allocate spa_hook for the proxy listener.
        let listener_box = Box::new(unsafe { mem::zeroed::<libspa_sys::spa_hook>() });
        let listener_ptr = Box::into_raw(listener_box);

        // Set up pw_proxy_events with the bound callback.  When the
        // null-sink proxy is bound, bound_cb reads the global id and
        // creates the equalizer filter wired to it.
        let mut events_box = Box::new(unsafe { mem::zeroed::<pipewire_sys::pw_proxy_events>() });
        events_box.version = pipewire_sys::PW_VERSION_PROXY_EVENTS;
        events_box.bound = Some(bound_cb);
        let events_ptr = Box::into_raw(events_box);

        // Safety: proxy is non-null (create_null_sink guarantees this).
        // listener_ptr and events_ptr point to freshly allocated,
        // heap-stable memory that outlives the proxy (freed on destroy).
        // data_ptr holds cloned/ref-counted resources valid for the
        // mainloop lifetime.
        unsafe {
            pipewire_sys::pw_proxy_add_listener(
                handle.proxy,
                listener_ptr,
                events_ptr,
                data_ptr.cast::<c_void>(),
            );
        }

        // Stash the listener pointers in the handle for cleanup.
        handle.listener_ptr = listener_ptr;
        handle.events_ptr = events_ptr;
        handle.data_ptr = data_ptr;
        nullsink_cell.set(Some(handle));
    } else {
        let _ = tx.send(PwEvent::NullSinkError(
            "failed to create null-audio-sink node".into(),
        ));
        // Fallback: create filter without a null sink target so the
        // equalizer remains functional even without wiremix visibility.
        if let Some(handle) = create_eq_filter(core_raw, &pipeline, &tx, None) {
            filter_cell.set(Some(handle));
        } else {
            return;
        }
    }

    let mainloop_cmd = mainloop.clone();

    // Spawn a dedicated thread for pw-link subprocess calls so the
    // PipeWire mainloop thread never blocks on fork/exec/waitpid.
    let (link_tx, link_rx) = mpsc::channel::<(u32, u32, bool)>();
    let link_worker = std::thread::spawn(move || {
        for (filter_id, device_id, connect) in link_rx {
            if connect {
                create_device_output_links(filter_id, device_id);
            } else {
                remove_device_output_links(filter_id, device_id);
            }
        }
    });

    let cmd_receiver = rx.attach(mainloop.loop_(), move |cmd| match cmd {
        PwCommand::Terminate => {
            // Teardown order: deactivate/destroy filter first, then destroy the
            // null-audio-sink. The filter consumer must be torn down before the
            // source node to avoid dangling PipeWire references.
            if let Some(handle) = filter_cell.take() {
                // Safety: running on the mainloop thread while the core is
                // still connected. The filter pointer and its allocations
                // are valid — FilterHandle::destroy deactivates, disconnects,
                // and frees all resources.
                unsafe {
                    handle.destroy();
                }
            }
            if let Some(handle) = nullsink_cell.take() {
                // Safety: running on the mainloop thread while the core
                // is still connected. pw_proxy_destroy frees the client-side
                // proxy and destroys the server-side node.
                unsafe {
                    handle.destroy();
                }
            }
            mainloop_cmd.quit();
        }
        PwCommand::ConnectDevice { filter_id, node_id } => {
            tracing::info!(
                filter_id,
                device_id = node_id,
                "Connecting device to filter"
            );
            let _ = link_tx.send((filter_id, node_id, true));
        }
        PwCommand::DisconnectDevice { filter_id, node_id } => {
            tracing::info!(
                filter_id,
                device_id = node_id,
                "Disconnecting device from filter"
            );
            let _ = link_tx.send((filter_id, node_id, false));
        }
    });

    let _ = tx.send(PwEvent::Connected);

    mainloop.run();

    // Drop the command receiver to release the last sender, signalling the
    // link-worker thread to exit. Join the worker before returning so no
    // pw-link subprocess is left behind.
    drop(cmd_receiver);
    if let Err(e) = link_worker.join() {
        tracing::error!("pw-link worker thread panicked: {e:?}");
    }
}

// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use std::cell::Cell;
use std::mem;
use std::os::raw::c_void;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use pipewire::channel::Receiver;
use pipewire::context::ContextRc;
use pipewire::main_loop::MainLoopRc;

use crate::effects::AudioEq;
use crate::pipeline::{Pipeline, SAMPLE_RATE};
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

    let null_sink_id_cell: Rc<Cell<Option<u32>>> = Rc::new(Cell::new(None));
    let ns_id_atomic: Arc<AtomicU32> = Arc::new(AtomicU32::new(0));

    // Timer callback — runs on PW mainloop. Node list is cheap (clone a Vec).
    // The null-sink input check is offloaded to a dedicated thread to avoid
    // blocking the audio thread with a fork/exec of pw-link -I.
    let ns_timer = null_sink_id_cell.clone();
    let ns_atomic = ns_id_atomic.clone();
    let timer = mainloop.loop_().add_timer(move |_| {
        let list: Vec<NodeInfo> = nodes_timer.borrow().iter().cloned().collect();
        let _ = tx_snapshot.send(PwEvent::NodeList(list));

        // Sync the null sink ID (set by the PW registry listener on this
        // same thread) so the checker thread can see it.
        ns_atomic.store(ns_timer.get().unwrap_or(0), Ordering::Release);
    });
    timer.update_timer(Some(Duration::from_millis(500)), None);

    // Dedicated thread — runs pw-link -I off the PipeWire mainloop so the
    // audio thread is never blocked by fork/exec/waitpid.
    let ns_checker_tx = tx.clone();
    let ns_checker = ns_id_atomic.clone();
    std::thread::Builder::new()
        .name("null-sink-checker".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(500));
                let ns_id = ns_checker.load(Ordering::Acquire);
                if ns_id > 0 {
                    let has_source = check_null_sink_input_source(ns_id);
                    let event = match has_source {
                        Some(true) => PwEvent::NullSinkInputState { has_source: true },
                        Some(false) => PwEvent::NullSinkInputState { has_source: false },
                        None => PwEvent::NullSinkInputUnknown,
                    };
                    // Exit cleanly when the receiver is dropped (daemon shutting down)
                    if ns_checker_tx.send(event).is_err() {
                        break;
                    }
                }
            }
        })
        .expect("failed to spawn null-sink-checker thread");

    let core_raw = core.as_raw_ptr().cast::<pipewire_sys::pw_core>();
    let filter_cell: Cell<Option<FilterHandle>> = Cell::new(None);
    let nullsink_cell: Cell<Option<NullSinkHandle>> = Cell::new(None);

    let audio_eq = Box::into_raw(Box::new(AudioEq::new(SAMPLE_RATE)));

    let nullsink_handle = create_null_sink(core_raw, &tx);

    if let Some(mut handle) = nullsink_handle {
        let listener_data = Box::new(NullSinkListenerData {
            tx: tx.clone(),
            core_raw,
            pipeline: pipeline.clone(),
            audio_eq,
            filter_cell_ptr: (&raw const filter_cell).cast_mut(),
            null_sink_id_cell_ptr: Rc::as_ptr(&null_sink_id_cell).cast_mut(),
            filter_created: Cell::new(false),
        });
        let data_ptr = Box::into_raw(listener_data);

        let listener_box = Box::new(unsafe { mem::zeroed::<libspa_sys::spa_hook>() });
        let listener_ptr = Box::into_raw(listener_box);

        let mut events_box = Box::new(unsafe { mem::zeroed::<pipewire_sys::pw_proxy_events>() });
        events_box.version = pipewire_sys::PW_VERSION_PROXY_EVENTS;
        events_box.bound = Some(bound_cb);
        let events_ptr = Box::into_raw(events_box);

        unsafe {
            pipewire_sys::pw_proxy_add_listener(
                handle.proxy,
                listener_ptr,
                events_ptr,
                data_ptr.cast::<c_void>(),
            );
        }

        handle.listener_ptr = listener_ptr;
        handle.events_ptr = events_ptr;
        handle.data_ptr = data_ptr;
        nullsink_cell.set(Some(handle));
    } else {
        let _ = tx.send(PwEvent::NullSinkError(
            "failed to create null-audio-sink node".into(),
        ));
        if let Some(handle) = create_eq_filter(core_raw, &pipeline, &tx, None, audio_eq) {
            filter_cell.set(Some(handle));
        } else {
            unsafe {
                drop(Box::from_raw(audio_eq));
            }
            return;
        }
    }

    let mainloop_cmd = mainloop.clone();

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
            if let Some(handle) = filter_cell.take() {
                unsafe {
                    handle.destroy();
                }
            }
            if let Some(handle) = nullsink_cell.take() {
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
        PwCommand::UpdateEq { bands } => {
            let audio_eq = unsafe { &mut *audio_eq };
            audio_eq.set_bands(&bands, SAMPLE_RATE);
            tracing::info!(count = bands.len(), "EQ bands updated on mainloop");
        }
    });

    let _ = tx.send(PwEvent::Connected);

    mainloop.run();

    drop(cmd_receiver);
    if let Err(e) = link_worker.join() {
        tracing::error!("pw-link worker thread panicked: {e:?}");
    }

    unsafe {
        drop(Box::from_raw(audio_eq));
    }
}

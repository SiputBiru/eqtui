// Copyright (C) 2026 SiputBiru <hillsforrest03@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use std::{
    cell::Cell,
    ffi::CString,
    os::raw::c_void,
    ptr,
    sync::{Arc, mpsc},
};

use crate::effects::AudioEq;
use crate::pipeline::Pipeline;
use crate::state::PwEvent;

use super::filter::{FilterHandle, create_eq_filter};
use super::props::Props;

// Data passed to the null-sink proxy listener callbacks.
// This Box is leaked into a raw pointer and freed only during shutdown.
// The bound callback creates the initial equalizer filter once the
// server-assigned global id arrives.
pub(crate) struct NullSinkListenerData {
    pub(crate) tx: mpsc::Sender<PwEvent>,
    pub(crate) core_raw: *mut pipewire_sys::pw_core,
    pub(crate) pipeline: Arc<Pipeline>,
    pub(crate) audio_eq: *mut AudioEq,
    pub(crate) filter_cell_ptr: *mut Cell<Option<FilterHandle>>,
    pub(crate) null_sink_id_cell_ptr: *mut Cell<Option<u32>>,
    pub(crate) filter_created: Cell<bool>,
}

// NullSinkHandle — holds the null-audio-sink proxy and all listener
// resources created via pw_proxy_add_listener. Must be destroyed on the
// PipeWire mainloop thread before the core is disconnected.
pub(crate) struct NullSinkHandle {
    pub(crate) proxy: *mut pipewire_sys::pw_proxy,
    pub(crate) listener_ptr: *mut libspa_sys::spa_hook,
    pub(crate) events_ptr: *mut pipewire_sys::pw_proxy_events,
    pub(crate) data_ptr: *mut NullSinkListenerData,
}

impl NullSinkHandle {
    /// Destroy the null-audio-sink proxy and all listener resources.
    ///
    /// # Safety
    /// Must be called from the `PipeWire` mainloop thread before the core
    /// is disconnected. All stored raw pointers must be valid (non-null)
    /// or null (which are safely ignored).
    pub(crate) unsafe fn destroy(self) {
        // Safety: caller guarantees this runs on the mainloop thread
        // while the core is still connected. pw_proxy_destroy frees the
        // client-side proxy and, because object.linger is not set,
        // destroys the server-side node as well.
        unsafe {
            if !self.proxy.is_null() {
                pipewire_sys::pw_proxy_destroy(self.proxy);
            }
            // Free listener allocations in reverse order of dependency.
            if !self.data_ptr.is_null() {
                drop(Box::from_raw(self.data_ptr));
            }
            if !self.events_ptr.is_null() {
                drop(Box::from_raw(self.events_ptr));
            }
            if !self.listener_ptr.is_null() {
                drop(Box::from_raw(self.listener_ptr));
            }
        }
    }
}

// Proxy listener callback — fires when the null-sink proxy is bound to a
// server-side global id. Enables learning the null sink's real node id
// so the equalizer filter can be wired to it.
//
// Safety: called by PipeWire on the mainloop thread after the proxy is
// bound. `data` is a valid pointer to a NullSinkListenerData Box that
// outlives the callback (freed only at shutdown).
pub(crate) unsafe extern "C" fn bound_cb(data: *mut c_void, global_id: u32) {
    unsafe {
        let nd = &*data.cast::<NullSinkListenerData>();

        // Store the global id for manual linking later.
        (*nd.null_sink_id_cell_ptr).set(Some(global_id));

        // Inform the TUI that the null sink is now live with its real id.
        let _ = nd.tx.send(PwEvent::NullSinkCreated {
            module_id: global_id,
        });

        // Only create the initial filter once. Device routing is handled
        // by ConnectDevice / DisconnectDevice commands from the TUI.
        if !nd.filter_created.get() {
            nd.filter_created.set(true);
            let handle = create_eq_filter(
                nd.core_raw,
                &nd.pipeline,
                &nd.tx,
                Some(global_id),
                nd.audio_eq,
            );
            if let Some(h) = handle {
                (*nd.filter_cell_ptr).set(Some(h));
            }
            // Monitor links are created by state_changed_cb when the
            // filter reaches STREAMING state (ports guaranteed ready).
        }
    }
}

// Create a virtual null-audio-sink node via the adapter factory.
// This node exposes media.class=Audio/Sink, making it visible to
// wiremix as a selectable output while passing audio through silently.
//
// Returns a handle for later cleanup on shutdown.
pub(crate) fn create_null_sink(
    core_raw: *mut pipewire_sys::pw_core,
    tx: &mpsc::Sender<PwEvent>,
) -> Option<NullSinkHandle> {
    // Build properties for the adapter factory — these determine the
    // node's identity and behaviour in the PipeWire graph.
    let props = Props::new("factory.name", "support.null-audio-sink");
    props.set("media.class", "Audio/Sink");
    props.set("node.name", "eqtui");
    props.set("node.description", "eqtui (Virtual Sink)");
    props.set("audio.position", "FL,FR");
    props.set("monitor.channel-volumes", "false");
    props.set("monitor.passthrough", "true");
    // Lowest session priority so the null sink does not steal the
    // default-sink role from the user's real output device.
    props.set("priority.session", "0");
    // Mark as passive so WirePlumber doesn't auto-connect new streams to
    // it unless explicitly requested by the user.
    props.set("node.passive", "true");
    // Mark as virtual so WirePlumber excludes this node from default-sink
    // selection. Without this, WirePlumber may promote the null sink to
    // the system default despite priority.session=0.
    props.set("node.virtual", "true");

    let factory_cstr = CString::new("adapter").expect("factory name should not contain null bytes");
    let type_cstr =
        CString::new("PipeWire:Interface:Node").expect("type string should not contain null bytes");

    // Safety: core_raw is a valid pointer obtained from a live CoreRc on
    // the PipeWire mainloop thread. pw_core is opaque in the bindings
    // but its C layout begins with pw_proxy, which begins with
    // spa_interface — the cast is therefore sound.
    // All CString pointers remain live for the duration of the FFI call.
    // props.into_raw() transfers ownership of the pw_properties into
    // pw_core_create_object (PipeWire copies the dict internally).
    let iface = core_raw.cast::<libspa_sys::spa_interface>();
    let methods = unsafe { (*iface).cb.funcs.cast::<pipewire_sys::pw_core_methods>() };
    let Some(create_fn) = (unsafe { (*methods).create_object }) else {
        let _ = tx.send(PwEvent::NullSinkError(
            "core create_object method not available".into(),
        ));
        return None;
    };

    let proxy_ptr = unsafe {
        create_fn(
            (*iface).cb.data,
            factory_cstr.as_ptr(),
            type_cstr.as_ptr(),
            pipewire_sys::PW_VERSION_NODE,
            props.into_raw().cast::<libspa_sys::spa_dict>(),
            0,
        )
    };

    if proxy_ptr.is_null() {
        let _ = tx.send(PwEvent::NullSinkError(
            "pw_core_create_object for null-audio-sink returned NULL".into(),
        ));
        return None;
    }

    // The returned void pointer is actually a pw_proxy.
    // The bound_cb will learn the real (server-assigned) global id when the
    // proxy is bound; NullSinkCreated is sent from there, not here.
    let proxy = proxy_ptr.cast::<pipewire_sys::pw_proxy>();

    Some(NullSinkHandle {
        proxy,
        listener_ptr: ptr::null_mut(),
        events_ptr: ptr::null_mut(),
        data_ptr: ptr::null_mut(),
    })
}

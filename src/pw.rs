use std::cell::Cell;
use std::ffi::CString;
use std::mem;
use std::os::raw::{c_char, c_void};
use std::ptr;
use std::rc::Rc;
use std::slice;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use pipewire::channel::Receiver;
use pipewire::context::ContextRc;
use pipewire::main_loop::MainLoopRc;
use pipewire::spa;

use crate::pipeline::Pipeline;
use crate::state::{DeviceClass, NodeInfo, PwCommand, PwEvent};

const DEFAULT_SAMPLE_RATE: u32 = 48000;
const DEFAULT_CHANNELS: u32 = 2;
const DEFAULT_N_SAMPLES: u32 = 1024;
const DSP_NODE_NAME: &str = "eqtui-dsp";

// Thin helpers for pw_properties — PipeWire copies strings internally, so
// CString temporaries are safe to drop after each call.
pub(crate) struct Props(*mut pipewire_sys::pw_properties);

impl Props {
    pub(crate) fn new(key: &str, val: &str) -> Self {
        let k = CString::new(key).expect("Props::new key should not contain null bytes");
        let v = CString::new(val).expect("Props::new val should not contain null bytes");
        let p = unsafe {
            pipewire_sys::pw_properties_new(k.as_ptr(), v.as_ptr(), ptr::null::<c_char>())
        };
        Self(p)
    }

    pub(crate) fn set(&self, key: &str, val: &str) {
        let k = CString::new(key).expect("Props::set key should not contain null bytes");
        let v = CString::new(val).expect("Props::set val should not contain null bytes");
        unsafe {
            pipewire_sys::pw_properties_set(self.0, k.as_ptr(), v.as_ptr());
        }
    }

    pub(crate) fn into_raw(self) -> *mut pipewire_sys::pw_properties {
        let p = self.0;
        mem::forget(self);
        p
    }
}

impl Drop for Props {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                pipewire_sys::pw_properties_free(self.0);
            }
        }
    }
}

// Filter callbacks
struct FilterData {
    pipeline: Arc<Pipeline>,
    filter_ptr: *mut pipewire_sys::pw_filter,
    null_sink_id: Option<u32>,
    in_left: *mut c_void,
    in_right: *mut c_void,
    out_left: *mut c_void,
    out_right: *mut c_void,
    tx: mpsc::Sender<PwEvent>,
    monitor_links_created: Cell<bool>,
    filter_ready_sent: Cell<bool>,
}

pub(crate) fn process_buffers(
    pipeline: &Pipeline,
    in_l: *mut f32,
    in_r: *mut f32,
    out_l: *mut f32,
    out_r: *mut f32,
    n_samples: usize,
) {
    if in_l.is_null() || in_r.is_null() || out_l.is_null() || out_r.is_null() {
        return;
    }

    let align = mem::align_of::<f32>();
    if !(in_l as usize).is_multiple_of(align)
        || !(in_r as usize).is_multiple_of(align)
        || !(out_l as usize).is_multiple_of(align)
        || !(out_r as usize).is_multiple_of(align)
    {
        return;
    }

    let left_in = unsafe { slice::from_raw_parts(in_l, n_samples) };
    let right_in = unsafe { slice::from_raw_parts(in_r, n_samples) };
    let left_out = unsafe { slice::from_raw_parts_mut(out_l, n_samples) };
    let right_out = unsafe { slice::from_raw_parts_mut(out_r, n_samples) };

    pipeline.process(left_in, right_in, left_out, right_out);
}

unsafe extern "C" fn process_cb(data: *mut c_void, _position: *mut libspa_sys::spa_io_position) {
    unsafe {
        let fd = &*data.cast::<FilterData>();

        let in_left =
            pipewire_sys::pw_filter_get_dsp_buffer(fd.in_left, DEFAULT_N_SAMPLES).cast::<f32>();
        let in_right =
            pipewire_sys::pw_filter_get_dsp_buffer(fd.in_right, DEFAULT_N_SAMPLES).cast::<f32>();
        let out_left =
            pipewire_sys::pw_filter_get_dsp_buffer(fd.out_left, DEFAULT_N_SAMPLES).cast::<f32>();
        let out_right =
            pipewire_sys::pw_filter_get_dsp_buffer(fd.out_right, DEFAULT_N_SAMPLES).cast::<f32>();

        process_buffers(
            &fd.pipeline,
            in_left,
            in_right,
            out_left,
            out_right,
            DEFAULT_N_SAMPLES as usize,
        );
    }
}

unsafe extern "C" fn state_changed_cb(
    data: *mut c_void,
    _old: pipewire_sys::pw_filter_state,
    new: pipewire_sys::pw_filter_state,
    _error: *const c_char,
) {
    unsafe {
        let fd = &*(data as *const FilterData);
        let state_str = state_name_for(new);
        let filter_state = match state_str {
            "UNCONNECTED" => crate::state::FilterState::Unconnected,
            "CONNECTING" => crate::state::FilterState::Connecting,
            "PAUSED" => crate::state::FilterState::Paused,
            "STREAMING" => crate::state::FilterState::Streaming,
            "ERROR" => crate::state::FilterState::Error(String::new()),
            _ => crate::state::FilterState::Unconnected,
        };
        let _ = fd.tx.send(PwEvent::FilterStateChanged(filter_state));

        // When the filter reaches PAUSED state, its ports are registered.
        // We link here because STREAMING may never be reached if there
        // are no links to pull/push data.
        if (new == pipewire_sys::pw_filter_state_PW_FILTER_STATE_PAUSED
            || new == pipewire_sys::pw_filter_state_PW_FILTER_STATE_STREAMING)
            && !fd.monitor_links_created.get()
        {
            let filter_id = pipewire_sys::pw_filter_get_node_id(fd.filter_ptr);
            if filter_id != 0 && filter_id != pipewire_sys::PW_ID_ANY {
                fd.monitor_links_created.set(true);
                tracing::info!(filter_id, "Filter reached {}, creating links", state_str);

                // Send the filter's node ID to the TUI so it can issue
                // ConnectDevice / DisconnectDevice commands.
                if !fd.filter_ready_sent.get() {
                    fd.filter_ready_sent.set(true);
                    let _ = fd.tx.send(PwEvent::FilterReady { node_id: filter_id });
                }

                // Capture from null sink monitor ports
                if let Some(ns_id) = fd.null_sink_id {
                    create_monitor_links(ns_id, filter_id);
                }

                // Output links are created on-demand by the TUI via
                // ConnectDevice / DisconnectDevice commands.
            } else {
                tracing::warn!(
                    filter_id,
                    "Filter reached {}, but ID is not yet valid",
                    state_str
                );
            }
        }
    }
}

pub(crate) fn state_name_for(s: pipewire_sys::pw_filter_state) -> &'static str {
    if s == pipewire_sys::pw_filter_state_PW_FILTER_STATE_UNCONNECTED {
        "UNCONNECTED"
    } else if s == pipewire_sys::pw_filter_state_PW_FILTER_STATE_CONNECTING {
        "CONNECTING"
    } else if s == pipewire_sys::pw_filter_state_PW_FILTER_STATE_PAUSED {
        "PAUSED"
    } else if s == pipewire_sys::pw_filter_state_PW_FILTER_STATE_STREAMING {
        "STREAMING"
    } else if s == pipewire_sys::pw_filter_state_PW_FILTER_STATE_ERROR {
        "ERROR"
    } else {
        "?"
    }
}

// FilterHandle — bundles all pointers needed for teardown / recreation
#[expect(dead_code, reason = "used via Cell<Option<FilterHandle>> in run()")]
struct FilterHandle {
    filter: *mut pipewire_sys::pw_filter,
    port_in_l: *mut c_void,
    port_in_r: *mut c_void,
    port_out_l: *mut c_void,
    port_out_r: *mut c_void,
    filter_data_ptr: *mut FilterData,
    // Heap-allocated spa_hook — must outlive the filter.
    // Freed AFTER filter_destroy.
    listener_ptr: *mut libspa_sys::spa_hook,
    // Heap-allocated pw_filter_events — must outlive the filter.
    events_ptr: *mut pipewire_sys::pw_filter_events,
}

impl FilterHandle {
    unsafe fn destroy(self) {
        unsafe {
            pipewire_sys::pw_filter_set_active(self.filter, false);
            pipewire_sys::pw_filter_disconnect(self.filter);
            // filter_destroy cleans up PipeWire's internal hook references —
            // must happen BEFORE we free our listener and events heap allocations.
            pipewire_sys::pw_filter_destroy(self.filter);
            if !self.filter_data_ptr.is_null() {
                drop(Box::from_raw(self.filter_data_ptr));
            }
            if !self.listener_ptr.is_null() {
                drop(Box::from_raw(self.listener_ptr));
            }
            if !self.events_ptr.is_null() {
                drop(Box::from_raw(self.events_ptr));
            }
        }
    }
}

// Data passed to the null-sink proxy listener callbacks.
// This Box is leaked into a raw pointer and freed only during shutdown.
// The bound callback creates the initial equalizer filter once the
// server-assigned global id arrives.
struct NullSinkListenerData {
    tx: mpsc::Sender<PwEvent>,
    core_raw: *mut pipewire_sys::pw_core,
    pipeline: Arc<Pipeline>,
    filter_cell_ptr: *mut Cell<Option<FilterHandle>>,
    null_sink_id_cell_ptr: *mut Cell<Option<u32>>,
    filter_created: Cell<bool>,
}

// NullSinkHandle — holds the null-audio-sink proxy and all listener
// resources created via pw_proxy_add_listener. Must be destroyed on the
// PipeWire mainloop thread before the core is disconnected.
struct NullSinkHandle {
    proxy: *mut pipewire_sys::pw_proxy,
    listener_ptr: *mut libspa_sys::spa_hook,
    events_ptr: *mut pipewire_sys::pw_proxy_events,
    data_ptr: *mut NullSinkListenerData,
}

impl NullSinkHandle {
    /// Destroy the null-audio-sink proxy and all listener resources.
    ///
    /// # Safety
    /// Must be called from the `PipeWire` mainloop thread before the core
    /// is disconnected. All stored raw pointers must be valid (non-null)
    /// or null (which are safely ignored).
    unsafe fn destroy(self) {
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
// server-side global id. This is how we learn the null sink's real node id
// so the equalizer filter can be wired to it.
//
// Safety: called by PipeWire on the mainloop thread after the proxy is
// bound. `data` is a valid pointer to a NullSinkListenerData Box that
// outlives the callback (freed only at shutdown).
unsafe extern "C" fn bound_cb(data: *mut c_void, global_id: u32) {
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
            let handle = create_eq_filter(nd.core_raw, &nd.pipeline, &nd.tx, Some(global_id));
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
fn create_null_sink(
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
    // proxy is bound; we send NullSinkCreated from there, not here.
    let proxy = proxy_ptr.cast::<pipewire_sys::pw_proxy>();

    Some(NullSinkHandle {
        proxy,
        listener_ptr: ptr::null_mut(),
        events_ptr: ptr::null_mut(),
        data_ptr: ptr::null_mut(),
    })
}

// Filter creation
fn create_eq_filter(
    core_raw: *mut pipewire_sys::pw_core,
    pipeline: &Arc<Pipeline>,
    tx: &mpsc::Sender<PwEvent>,
    null_sink_id: Option<u32>,
) -> Option<FilterHandle> {
    // Follow EasyEffects' pattern: do NOT set media.class on pw_filter nodes.
    // Wiremix's monitor_node() only binds nodes with an exact media.class match
    // on "Audio/Sink" / "Audio/Source" / "Stream/*".  Without media.class, the
    // node is skipped entirely and wiremix never tries to enumerate PortConfig
    // (which pw_filter nodes don't support), avoiding the crash:
    //   "enum params id:11 (Spa:Enum:ParamId:PortConfig) failed"
    let props = Props::new("media.type", "Audio");
    props.set("media.category", "Duplex");
    props.set("media.role", "DSP");
    props.set("node.name", DSP_NODE_NAME);
    props.set("node.description", "eqtui (Processor)");
    // Mark as virtual so WirePlumber doesn't auto-promote this filter to
    // the default sink, which would steal audio streams and disrupt other
    // PipeWire clients (e.g. wiremix) that are monitoring the graph.
    props.set("node.virtual", "true");
    // Lowest session priority – extra guard against becoming default.
    props.set("priority.session", "0");

    let name_cstr =
        CString::new(DSP_NODE_NAME).expect("static filter name should not contain null");
    let filter =
        unsafe { pipewire_sys::pw_filter_new(core_raw, name_cstr.as_ptr(), props.into_raw()) };

    if filter.is_null() {
        let _ = tx.send(PwEvent::Error("pw_filter_new failed".into()));
        return None;
    }

    let in_left = unsafe {
        let p = Props::new("port.name", "input_FL");
        p.set("object.path", "input_FL");
        p.set("audio.channel", "FL");
        p.set("format.dsp", "32 bit float mono audio");
        pipewire_sys::pw_filter_add_port(
            filter,
            libspa_sys::SPA_DIRECTION_INPUT,
            pipewire_sys::pw_filter_port_flags_PW_FILTER_PORT_FLAG_MAP_BUFFERS,
            0,
            p.into_raw(),
            ptr::null_mut(),
            0,
        )
    };
    let in_right = unsafe {
        let p = Props::new("port.name", "input_FR");
        p.set("object.path", "input_FR");
        p.set("audio.channel", "FR");
        p.set("format.dsp", "32 bit float mono audio");
        pipewire_sys::pw_filter_add_port(
            filter,
            libspa_sys::SPA_DIRECTION_INPUT,
            pipewire_sys::pw_filter_port_flags_PW_FILTER_PORT_FLAG_MAP_BUFFERS,
            0,
            p.into_raw(),
            ptr::null_mut(),
            0,
        )
    };
    let out_left = unsafe {
        let p = Props::new("port.name", "output_FL");
        p.set("object.path", "output_FL");
        p.set("audio.channel", "FL");
        p.set("format.dsp", "32 bit float mono audio");
        pipewire_sys::pw_filter_add_port(
            filter,
            libspa_sys::SPA_DIRECTION_OUTPUT,
            pipewire_sys::pw_filter_port_flags_PW_FILTER_PORT_FLAG_MAP_BUFFERS,
            0,
            p.into_raw(),
            ptr::null_mut(),
            0,
        )
    };
    let out_right = unsafe {
        let p = Props::new("port.name", "output_FR");
        p.set("object.path", "output_FR");
        p.set("audio.channel", "FR");
        p.set("format.dsp", "32 bit float mono audio");
        pipewire_sys::pw_filter_add_port(
            filter,
            libspa_sys::SPA_DIRECTION_OUTPUT,
            pipewire_sys::pw_filter_port_flags_PW_FILTER_PORT_FLAG_MAP_BUFFERS,
            0,
            p.into_raw(),
            ptr::null_mut(),
            0,
        )
    };

    if in_left.is_null() || in_right.is_null() || out_left.is_null() || out_right.is_null() {
        let _ = tx.send(PwEvent::Error("pw_filter_add_port failed".into()));
        return None;
    }

    let filter_data = Box::new(FilterData {
        pipeline: pipeline.clone(),
        filter_ptr: filter,
        null_sink_id,
        in_left,
        in_right,
        out_left,
        out_right,
        tx: tx.clone(),
        monitor_links_created: Cell::new(false),
        filter_ready_sent: Cell::new(false),
    });
    let filter_data_ptr = Box::into_raw(filter_data);

    let mut events = Box::new(unsafe { mem::zeroed::<pipewire_sys::pw_filter_events>() });
    events.version = pipewire_sys::PW_VERSION_FILTER_EVENTS;
    events.process = Some(process_cb);
    events.state_changed = Some(state_changed_cb);
    let events_ptr = Box::into_raw(events);

    let listener_box = Box::new(unsafe { mem::zeroed::<libspa_sys::spa_hook>() });
    let listener_ptr = Box::into_raw(listener_box);
    unsafe {
        pipewire_sys::pw_filter_add_listener(
            filter,
            listener_ptr,
            events_ptr,
            filter_data_ptr.cast::<c_void>(),
        );
    }

    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(DEFAULT_SAMPLE_RATE);
    audio_info.set_channels(DEFAULT_CHANNELS);
    let mut position = [0u32; spa::param::audio::MAX_CHANNELS];
    position[0] = libspa_sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = libspa_sys::SPA_AUDIO_CHANNEL_FR;
    audio_info.set_position(position);

    let values: Vec<u8> = match spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(spa::pod::Object {
            type_: libspa_sys::SPA_TYPE_OBJECT_Format,
            id: libspa_sys::SPA_PARAM_EnumFormat,
            properties: audio_info.into(),
        }),
    ) {
        Ok(v) => v.0.into_inner(),
        Err(e) => {
            let _ = tx.send(PwEvent::Error(format!("SPA pod serialization failed: {e}")));
            return None;
        }
    };

    let Some(pod_ref) = spa::pod::Pod::from_bytes(&values) else {
        let _ = tx.send(PwEvent::Error("pod from_bytes failed".into()));
        return None;
    };

    let pod_ptr = ptr::from_ref::<spa::pod::Pod>(pod_ref).cast::<libspa_sys::spa_pod>();
    let mut params = [pod_ptr];

    let ret = unsafe {
        pipewire_sys::pw_filter_connect(
            filter,
            pipewire_sys::pw_filter_flags_PW_FILTER_FLAG_RT_PROCESS,
            params.as_mut_ptr(),
            1,
        )
    };

    if ret != 0 {
        let _ = tx.send(PwEvent::Error(format!("filter_connect failed: {ret}")));
        return None;
    }

    unsafe {
        pipewire_sys::pw_filter_set_active(filter, true);
    }

    Some(FilterHandle {
        filter,
        port_in_l: in_left,
        port_in_r: in_right,
        port_out_l: out_left,
        port_out_r: out_right,
        filter_data_ptr,
        listener_ptr,
        events_ptr,
    })
}

fn create_monitor_links(out_node_id: u32, in_node_id: u32) {
    let links = [("monitor_FL", "input_FL"), ("monitor_FR", "input_FR")];

    for (out_port, in_port) in &links {
        let out_spec = format!("{out_node_id}:{out_port}");
        let in_spec = format!("{in_node_id}:{in_port}");

        // Skip if the link already exists (e.g. left over from a previous
        // run that didn't tear down cleanly).  This avoids the noisy
        // "failed to link ports: File exists" error from pw-link.
        if link_exists(&out_spec, &in_spec) {
            tracing::debug!(%out_spec, %in_spec, "Monitor link already exists, skipping");
            continue;
        }

        tracing::info!(%out_spec, %in_spec, "Calling pw-link for monitor link");

        let status = std::process::Command::new("pw-link")
            .arg(&out_spec)
            .arg(&in_spec)
            .status();

        match status {
            Ok(s) if s.success() => {
                tracing::info!(%out_spec, %in_spec, "pw-link monitor success");
            }
            Ok(s) => {
                tracing::error!(%out_spec, %in_spec, "pw-link monitor failed with status: {s}");
            }
            Err(e) => {
                tracing::error!(%out_spec, %in_spec, "failed to execute pw-link: {e}");
            }
        }
    }
}

/// Create `PipeWire` links from the DSP filter's output ports to a target
/// device's playback ports using `pw-link`.
fn create_device_output_links(filter_id: u32, device_id: u32) {
    let links = [("output_FL", "playback_FL"), ("output_FR", "playback_FR")];

    for (out_port, in_port) in &links {
        let out_spec = format!("{filter_id}:{out_port}");
        let in_spec = format!("{device_id}:{in_port}");

        // Skip if the link already exists to avoid the "File exists" error.
        if link_exists(&out_spec, &in_spec) {
            tracing::debug!(%out_spec, %in_spec, "Output link already exists, skipping");
            continue;
        }

        tracing::info!(%out_spec, %in_spec, "Calling pw-link for output link");

        let status = std::process::Command::new("pw-link")
            .arg(&out_spec)
            .arg(&in_spec)
            .status();

        match status {
            Ok(s) if s.success() => {
                tracing::info!(%out_spec, %in_spec, "pw-link output success");
            }
            Ok(s) => {
                tracing::error!(%out_spec, %in_spec, "pw-link output failed with status: {s}");
            }
            Err(e) => {
                tracing::error!(%out_spec, %in_spec, "failed to execute pw-link: {e}");
            }
        }
    }
}

/// Remove `PipeWire` links between the DSP filter's output ports and a
/// target device's playback ports using `pw-link -d`.
fn remove_device_output_links(filter_id: u32, device_id: u32) {
    let links = [("output_FL", "playback_FL"), ("output_FR", "playback_FR")];

    for (out_port, in_port) in &links {
        let out_spec = format!("{filter_id}:{out_port}");
        let in_spec = format!("{device_id}:{in_port}");

        tracing::info!(%out_spec, %in_spec, "Calling pw-link -d to remove output link");

        let status = std::process::Command::new("pw-link")
            .arg("-d")
            .arg(&out_spec)
            .arg(&in_spec)
            .status();

        match status {
            Ok(s) if s.success() => {
                tracing::info!(%out_spec, %in_spec, "pw-link -d success");
            }
            Ok(s) => {
                tracing::error!(%out_spec, %in_spec, "pw-link -d failed with status: {s}");
            }
            Err(e) => {
                tracing::error!(%out_spec, %in_spec, "failed to execute pw-link -d: {e}");
            }
        }
    }
}

/// Check whether a `PipeWire` link already exists between two ports by
/// parsing `pw-link -l` output.  Returns `true` if a link from `out_spec`
/// to `in_spec` is already present in the graph.
fn link_exists(out_spec: &str, in_spec: &str) -> bool {
    let Ok(output) = std::process::Command::new("pw-link")
        .arg("-l")
        .output()
    else {
        return false;
    };

    let text = String::from_utf8_lossy(&output.stdout);

    // pw-link -l output lists each link in both directions:
    //   port_A
    //     |-> port_B          (A is the output)
    //     |<- port_B          (B is the output)
    // We scan for a line matching out_spec followed by a line
    // containing "|->" and in_spec.
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        if line.trim() == out_spec {
            if let Some(next_line) = lines.next() {
                if next_line.trim().starts_with("|->") && next_line.contains(in_spec) {
                    return true;
                }
            }
        }
    }
    false
}

/// Check whether any `PipeWire` link routes audio INTO the null sink's
/// `playback_FL` or `playback_FR` ports.  Returns `true` if at least one
/// audio source is connected to the null-sink input.
fn check_null_sink_input_source(null_sink_id: u32) -> bool {
    let Ok(output) = std::process::Command::new("pw-link").arg("-I").output() else {
        return false;
    };

    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().any(|line| {
        line.contains(&format!("-> {null_sink_id}:playback_FL"))
            || line.contains(&format!("-> {null_sink_id}:playback_FR"))
    })
}

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

    let nodes_reg = nodes.clone();
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

                    nodes_reg.borrow_mut().push(NodeInfo {
                        id: global.id,
                        name,
                        description,
                        class: device_class,
                    });
                }
            }
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
        // We check if any link targets the null sink's input.
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
    // We attach a proxy-listener that fires when the proxy is bound to a
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

    let _cmd_receiver = rx.attach(mainloop.loop_(), move |cmd| match cmd {
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
            create_device_output_links(filter_id, node_id);
        }
        PwCommand::DisconnectDevice { filter_id, node_id } => {
            tracing::info!(
                filter_id,
                device_id = node_id,
                "Disconnecting device from filter"
            );
            remove_device_output_links(filter_id, node_id);
        }
    });

    let _ = tx.send(PwEvent::Connected);

    mainloop.run();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_name() {
        assert_eq!(
            state_name_for(pipewire_sys::pw_filter_state_PW_FILTER_STATE_UNCONNECTED),
            "UNCONNECTED"
        );
        assert_eq!(
            state_name_for(pipewire_sys::pw_filter_state_PW_FILTER_STATE_STREAMING),
            "STREAMING"
        );
    }

    #[test]
    fn test_process_buffers_null_checks() {
        let pipeline = Pipeline::new(48000.0);
        // Should return early and not panic
        process_buffers(
            &pipeline,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            1024,
        );
    }

    #[test]
    fn test_process_buffers_alignment_checks() {
        let pipeline = Pipeline::new(48000.0);
        // Create misaligned pointer by using an odd address.
        // Use without_provenance_mut (Strict Provenance) — these are
        // synthetic pointers only used for alignment checking, so they
        // don't need real provenance.
        let misaligned = ptr::without_provenance_mut::<f32>(0x0123_4567);
        let valid = ptr::without_provenance_mut::<f32>(0x0123_4568); // assuming 4-byte align is met by 8
        process_buffers(&pipeline, misaligned, valid, valid, valid, 1024);
    }
}

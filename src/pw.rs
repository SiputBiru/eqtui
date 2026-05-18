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

// Thin helpers for pw_properties — PipeWire copies strings internally, so
// CString temporaries are safe to drop after each call.
pub(crate) struct Props(*mut pipewire_sys::pw_properties);

impl Props {
    pub(crate) fn new(key: &str, val: &str) -> Self {
        let k = CString::new(key).unwrap();
        let v = CString::new(val).unwrap();
        let p = unsafe {
            pipewire_sys::pw_properties_new(k.as_ptr(), v.as_ptr(), ptr::null::<c_char>())
        };
        Self(p)
    }

    pub(crate) fn set(&self, key: &str, val: &str) {
        let k = CString::new(key).unwrap();
        let v = CString::new(val).unwrap();
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
    in_left: *mut c_void,
    in_right: *mut c_void,
    out_left: *mut c_void,
    out_right: *mut c_void,
    tx: mpsc::Sender<PwEvent>,
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
        let state_str = state_name_for(new).to_string();
        let _ = fd.tx.send(PwEvent::FilterStateChanged(state_str));
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
#[allow(dead_code)]
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

// Filter creation
fn create_eq_filter(
    core_raw: *mut pipewire_sys::pw_core,
    pipeline: &Arc<Pipeline>,
    tx: &mpsc::Sender<PwEvent>,
    target_node_id: Option<u32>,
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
    props.set("node.name", "eqtui");
    props.set("node.description", "eqtui Equalizer");
    props.set("node.autoconnect", "true");
    // Mark as virtual so WirePlumber doesn't auto-promote this filter to
    // the default sink, which would steal audio streams and disrupt other
    // PipeWire clients (e.g. wiremix) that are monitoring the graph.
    props.set("node.virtual", "true");
    // Lowest session priority – extra guard against becoming default.
    props.set("priority.session", "0");
    if let Some(id) = target_node_id {
        props.set("node.target", &id.to_string());
    }

    let name_cstr = CString::new("eqtui").unwrap();
    let filter =
        unsafe { pipewire_sys::pw_filter_new(core_raw, name_cstr.as_ptr(), props.into_raw()) };

    if filter.is_null() {
        let _ = tx.send(PwEvent::Error("pw_filter_new failed".into()));
        return None;
    }

    let in_left = unsafe {
        let p = Props::new("port.name", "input_FL");
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
        in_left,
        in_right,
        out_left,
        out_right,
        tx: tx.clone(),
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

    let values: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(spa::pod::Object {
            type_: libspa_sys::SPA_TYPE_OBJECT_Format,
            id: libspa_sys::SPA_PARAM_EnumFormat,
            properties: audio_info.into(),
        }),
    )
    .unwrap()
    .0
    .into_inner();

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

    let _ = tx.send(PwEvent::NullSinkCreated { module_id: 0 });

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
    let timer = mainloop.loop_().add_timer(move |_| {
        let list: Vec<NodeInfo> = nodes_timer.borrow().iter().cloned().collect();
        let _ = tx_snapshot.send(PwEvent::NodeList(list));
    });
    timer.update_timer(Some(Duration::from_millis(500)), None);

    let core_raw = core.as_raw_ptr().cast::<pipewire_sys::pw_core>();
    let filter_cell: Cell<Option<FilterHandle>> = Cell::new(None);

    match create_eq_filter(core_raw, &pipeline, &tx, None) {
        Some(handle) => {
            filter_cell.set(Some(handle));
        }
        None => {
            return;
        }
    }

    let mainloop_cmd = mainloop.clone();
    let pipeline_cmd = pipeline.clone();
    let tx_cmd = tx.clone();

    let _cmd_receiver = rx.attach(mainloop.loop_(), move |cmd| match cmd {
        PwCommand::Terminate => {
            if let Some(handle) = filter_cell.take() {
                unsafe {
                    handle.destroy();
                }
            }
            mainloop_cmd.quit();
        }
        PwCommand::SetTarget { node_id } => {
            // Tear down old filter
            if let Some(handle) = filter_cell.take() {
                unsafe {
                    handle.destroy();
                }
            }
            // Recreate with new target device
            match create_eq_filter(core_raw, &pipeline_cmd, &tx_cmd, Some(node_id)) {
                Some(handle) => {
                    filter_cell.set(Some(handle));
                    let _ = tx_cmd.send(PwEvent::FilterStateChanged("RECONNECTING".into()));
                }
                None => {
                    let _ = tx_cmd.send(PwEvent::Error(
                        "failed to recreate filter for target change".into(),
                    ));
                }
            }
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
        // Create misaligned pointer by using an odd address
        let misaligned = 0x0123_4567 as *mut f32;
        let valid = 0x0123_4568 as *mut f32; // assuming 4-byte align is met by 8
        process_buffers(&pipeline, misaligned, valid, valid, valid, 1024);
    }
}

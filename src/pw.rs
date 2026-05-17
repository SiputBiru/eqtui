use std::cell::RefCell;
use std::os::raw::c_void;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use pipewire::channel::Receiver;
use pipewire::context::ContextRc;
use pipewire::main_loop::MainLoopRc;
use pipewire::spa;

use crate::pipeline::Pipeline;
use crate::pw_filter_ffi;
use crate::state::{DeviceClass, NodeInfo, PwCommand, PwEvent};

const DEFAULT_SAMPLE_RATE: u32 = 48000;
const DEFAULT_CHANNELS: u32 = 2;
const DEFAULT_N_SAMPLES: u32 = 1024;

struct FilterData {
    pipeline: Arc<Pipeline>,
    in_left: *mut c_void,
    in_right: *mut c_void,
    out_left: *mut c_void,
    out_right: *mut c_void,
    tx: mpsc::Sender<PwEvent>,
}

unsafe extern "C" fn process_cb(data: *mut c_void, _position: *mut pw_filter_ffi::spa_io_position) {
    unsafe {
        let fd = &*(data as *const FilterData);

        let in_left = pw_filter_ffi::filter_get_dsp_buffer(fd.in_left, DEFAULT_N_SAMPLES);
        let in_right = pw_filter_ffi::filter_get_dsp_buffer(fd.in_right, DEFAULT_N_SAMPLES);
        let out_left = pw_filter_ffi::filter_get_dsp_buffer(fd.out_left, DEFAULT_N_SAMPLES);
        let out_right = pw_filter_ffi::filter_get_dsp_buffer(fd.out_right, DEFAULT_N_SAMPLES);

        // Prevent panic: Check for null pointers
        if in_left.is_null() || in_right.is_null() || out_left.is_null() || out_right.is_null() {
            return;
        }

        // Prevent panic: Check for proper f32 memory alignment
        let align = std::mem::align_of::<f32>();
        if (in_left as usize) % align != 0
            || (in_right as usize) % align != 0
            || (out_left as usize) % align != 0
            || (out_right as usize) % align != 0
        {
            return;
        }

        let n = DEFAULT_N_SAMPLES as usize;
        let left_in = std::slice::from_raw_parts(in_left, n);
        let right_in = std::slice::from_raw_parts(in_right, n);
        let left_out = std::slice::from_raw_parts_mut(out_left, n);
        let right_out = std::slice::from_raw_parts_mut(out_right, n);

        fd.pipeline.process(left_in, right_in, left_out, right_out);
    }
}

unsafe extern "C" fn state_changed_cb(
    data: *mut c_void,
    _old: pw_filter_ffi::pw_filter_state,
    new: pw_filter_ffi::pw_filter_state,
    _error: *const std::os::raw::c_char,
) {
    unsafe {
        let fd = &*(data as *const FilterData);
        let state_str = state_name_for(new).to_string();
        let _ = fd.tx.send(PwEvent::FilterStateChanged(state_str));
    }
}

fn state_name_for(s: pw_filter_ffi::pw_filter_state) -> &'static str {
    if s == pw_filter_ffi::PW_FILTER_STATE_UNCONNECTED {
        "UNCONNECTED"
    } else if s == pw_filter_ffi::PW_FILTER_STATE_CONNECTING {
        "CONNECTING"
    } else if s == pw_filter_ffi::PW_FILTER_STATE_PAUSED {
        "PAUSED"
    } else if s == pw_filter_ffi::PW_FILTER_STATE_STREAMING {
        "STREAMING"
    } else if s == pw_filter_ffi::PW_FILTER_STATE_ERROR {
        "ERROR"
    } else {
        "?"
    }
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

    let nodes: Rc<RefCell<Vec<NodeInfo>>> = Rc::new(RefCell::new(Vec::new()));

    let nodes_reg = nodes.clone();
    let _reg_listener = registry
        .add_listener_local()
        .global(move |global| {
            if let Some(props) = &global.props {
                let class = props.get(&*pipewire::keys::MEDIA_CLASS).unwrap_or("");
                if class == "Audio/Sink" || class == "Audio/Source" {
                    let name = props
                        .get(&*pipewire::keys::NODE_NAME)
                        .unwrap_or("?")
                        .to_string();
                    let description = props
                        .get(&*pipewire::keys::NODE_DESCRIPTION)
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

    // --- filter setup ---

    let props = unsafe {
        let p = pw_filter_ffi::properties_new("media.class", "Audio/Sink");
        pw_filter_ffi::properties_set(p, "node.name", "eqtui");
        pw_filter_ffi::properties_set(p, "node.description", "eqtui Equalizer");
        p
    };

    let filter = unsafe {
        pw_filter_ffi::filter_new(
            core.as_raw_ptr() as *mut pw_filter_ffi::pw_core,
            "eqtui",
            props,
        )
    };

    if filter.is_null() {
        let _ = tx.send(PwEvent::Error("pw_filter_new failed".into()));
        return;
    }

    // Name filter ports so pw-link can target them predictably.
    let in_left_props = unsafe {
        let p = pw_filter_ffi::properties_new("port.name", "input_FL");
        pw_filter_ffi::properties_set(p, "audio.channel", "FL");
        pw_filter_ffi::properties_set(p, "format.dsp", "32 bit float mono audio");
        p
    };
    let in_right_props = unsafe {
        let p = pw_filter_ffi::properties_new("port.name", "input_FR");
        pw_filter_ffi::properties_set(p, "audio.channel", "FR");
        pw_filter_ffi::properties_set(p, "format.dsp", "32 bit float mono audio");
        p
    };
    let out_left_props = unsafe {
        let p = pw_filter_ffi::properties_new("port.name", "output_FL");
        pw_filter_ffi::properties_set(p, "audio.channel", "FL");
        pw_filter_ffi::properties_set(p, "format.dsp", "32 bit float mono audio");
        p
    };
    let out_right_props = unsafe {
        let p = pw_filter_ffi::properties_new("port.name", "output_FR");
        pw_filter_ffi::properties_set(p, "audio.channel", "FR");
        pw_filter_ffi::properties_set(p, "format.dsp", "32 bit float mono audio");
        p
    };

    let in_left = unsafe {
        pw_filter_ffi::filter_add_port(
            filter,
            libspa_sys::SPA_DIRECTION_INPUT,
            pw_filter_ffi::PW_FILTER_PORT_FLAG_MAP_BUFFERS,
            0,
            in_left_props,
            std::ptr::null_mut(),
            0,
        )
    };

    let in_right = unsafe {
        pw_filter_ffi::filter_add_port(
            filter,
            libspa_sys::SPA_DIRECTION_INPUT,
            pw_filter_ffi::PW_FILTER_PORT_FLAG_MAP_BUFFERS,
            0,
            in_right_props,
            std::ptr::null_mut(),
            0,
        )
    };

    let out_left = unsafe {
        pw_filter_ffi::filter_add_port(
            filter,
            libspa_sys::SPA_DIRECTION_OUTPUT,
            pw_filter_ffi::PW_FILTER_PORT_FLAG_MAP_BUFFERS,
            0,
            out_left_props,
            std::ptr::null_mut(),
            0,
        )
    };

    let out_right = unsafe {
        pw_filter_ffi::filter_add_port(
            filter,
            libspa_sys::SPA_DIRECTION_OUTPUT,
            pw_filter_ffi::PW_FILTER_PORT_FLAG_MAP_BUFFERS,
            0,
            out_right_props,
            std::ptr::null_mut(),
            0,
        )
    };

    let filter_data = Box::new(FilterData {
        pipeline: pipeline.clone(),
        in_left,
        in_right,
        out_left,
        out_right,
        tx: tx.clone(),
    });

    let mut events: pw_filter_ffi::pw_filter_events = unsafe { std::mem::zeroed() };
    events.version = pipewire_sys::PW_VERSION_FILTER_EVENTS;
    events.process = Some(process_cb);
    events.state_changed = Some(state_changed_cb);

    let mut listener: pw_filter_ffi::spa_hook = unsafe { std::mem::zeroed() };
    unsafe {
        pw_filter_ffi::filter_add_listener(
            filter,
            &mut listener,
            &events,
            Box::into_raw(filter_data) as *mut c_void,
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

    let pod_ref = match spa::pod::Pod::from_bytes(&values) {
        Some(p) => p,
        None => {
            let _ = tx.send(PwEvent::Error("pod from_bytes failed".into()));
            return;
        }
    };

    let pod_ptr = pod_ref as *const spa::pod::Pod as *const libspa_sys::spa_pod;
    let mut params = [pod_ptr];

    let ret = unsafe {
        pw_filter_ffi::filter_connect(
            filter,
            pw_filter_ffi::PW_FILTER_FLAG_RT_PROCESS,
            params.as_mut_ptr(),
            1,
        )
    };

    if ret != 0 {
        let _ = tx.send(PwEvent::Error(format!("filter_connect failed: {ret}")));
        return;
    }

    unsafe {
        pw_filter_ffi::filter_set_active(filter, true);
    }

    // --- command channel ---

    let mainloop_cmd = mainloop.clone();
    let _cmd_receiver = rx.attach(mainloop.loop_(), move |cmd| match cmd {
        PwCommand::Terminate => {
            unsafe {
                pw_filter_ffi::filter_set_active(filter, false);
                pw_filter_ffi::filter_disconnect(filter);
                pw_filter_ffi::filter_destroy(filter);
            }
            mainloop_cmd.quit();
        }
    });

    let _ = tx.send(PwEvent::Connected);

    mainloop.run();
}

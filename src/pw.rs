use std::cell::RefCell;
use std::os::raw::c_void;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use pipewire::channel::Receiver;
use pipewire::context::ContextRc;
use pipewire::main_loop::MainLoopRc;

use crate::pipeline::Pipeline;
use crate::pw_filter_ffi;
use crate::state::{NodeInfo, PwCommand, PwEvent};

const DEFAULT_SAMPLE_RATE: f32 = 48000.0;
const DEFAULT_N_SAMPLES: u32 = 1024;

struct FilterData {
    pipeline: Arc<Pipeline>,
    in_left: *mut c_void,
    in_right: *mut c_void,
    out_left: *mut c_void,
    out_right: *mut c_void,
}

unsafe extern "C" fn process_cb(data: *mut c_void, _position: *mut pw_filter_ffi::spa_io_position) {
    unsafe {
        let fd = &*(data as *const FilterData);

        let in_left = pw_filter_ffi::filter_get_dsp_buffer(fd.in_left, DEFAULT_N_SAMPLES);
        let in_right = pw_filter_ffi::filter_get_dsp_buffer(fd.in_right, DEFAULT_N_SAMPLES);
        let out_left = pw_filter_ffi::filter_get_dsp_buffer(fd.out_left, DEFAULT_N_SAMPLES);
        let out_right = pw_filter_ffi::filter_get_dsp_buffer(fd.out_right, DEFAULT_N_SAMPLES);

        let n = DEFAULT_N_SAMPLES as usize;
        let left_in = std::slice::from_raw_parts(in_left, n);
        let right_in = std::slice::from_raw_parts(in_right, n);
        let left_out = std::slice::from_raw_parts_mut(out_left, n);
        let right_out = std::slice::from_raw_parts_mut(out_right, n);

        fd.pipeline
            .process(left_in, right_in, left_out, right_out);
    }
}

unsafe extern "C" fn state_changed_cb(
    data: *mut c_void,
    _old: pw_filter_ffi::pw_filter_state,
    new: pw_filter_ffi::pw_filter_state,
    _error: *const std::os::raw::c_char,
) {
    if new == pw_filter_ffi::PW_FILTER_STATE_ERROR {
        eprintln!("[eqtui] pw_filter entered error state");
    }
    let _ = data;
}

pub fn run(tx: mpsc::Sender<PwEvent>, rx: Receiver<PwCommand>) {
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
                    nodes_reg.borrow_mut().push(NodeInfo {
                        id: global.id,
                        name,
                        description,
                        class: class.to_string(),
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

    let pipeline = Arc::new(Pipeline::new(DEFAULT_SAMPLE_RATE));

    let filter = unsafe {
        pw_filter_ffi::filter_new(
            core.as_raw_ptr() as *mut pw_filter_ffi::pw_core,
            "eqtui",
            std::ptr::null_mut(),
        )
    };

    if filter.is_null() {
        let _ = tx.send(PwEvent::Error("pw_filter_new failed".into()));
        return;
    }

    let in_left = unsafe {
        pw_filter_ffi::filter_add_port(
            filter,
            libspa_sys::SPA_DIRECTION_INPUT,
            pw_filter_ffi::PW_FILTER_PORT_FLAG_MAP_BUFFERS,
            0,
            std::ptr::null_mut(),
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
            std::ptr::null_mut(),
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
            std::ptr::null_mut(),
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
            std::ptr::null_mut(),
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

    let ret = unsafe {
        pw_filter_ffi::filter_connect(
            filter,
            pw_filter_ffi::PW_FILTER_FLAG_RT_PROCESS,
            std::ptr::null_mut(),
            0,
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

// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use std::cell::Cell;
use std::ffi::CString;
use std::mem;
use std::os::raw::{c_char, c_void};
use std::ptr;
use std::sync::Arc;
use std::sync::mpsc;

use pipewire::spa;

use crate::effects::AudioEq;
use crate::pipeline::{Pipeline, SAMPLE_RATE};
use crate::state::{FilterState, PwEvent};

use super::links::create_monitor_links;
use super::props::Props;

const DEFAULT_CHANNELS: u32 = 2;

// Shared by process_cb — kept pub(crate) so tests can check the expected buffer size.
pub(crate) const DEFAULT_N_SAMPLES: u32 = 1024;

// Used when setting node.name / CString in create_eq_filter.
const DSP_NODE_NAME: &str = "eqtui-dsp";

// Filter callbacks
struct FilterData {
    pipeline: Arc<Pipeline>,
    audio_eq: *mut AudioEq,
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

unsafe extern "C" fn process_cb(data: *mut c_void, position: *mut libspa_sys::spa_io_position) {
    unsafe {
        let fd = &mut *data.cast::<FilterData>();

        let n_samples = if position.is_null() {
            DEFAULT_N_SAMPLES
        } else {
            (*position).clock.duration as u32
        };

        if n_samples == 0 {
            return;
        }

        let in_left = pipewire_sys::pw_filter_get_dsp_buffer(fd.in_left, n_samples).cast::<f32>();
        let in_right = pipewire_sys::pw_filter_get_dsp_buffer(fd.in_right, n_samples).cast::<f32>();
        let out_left = pipewire_sys::pw_filter_get_dsp_buffer(fd.out_left, n_samples).cast::<f32>();
        let out_right =
            pipewire_sys::pw_filter_get_dsp_buffer(fd.out_right, n_samples).cast::<f32>();

        if in_left.is_null() || in_right.is_null() || out_left.is_null() || out_right.is_null() {
            return;
        }

        let n = n_samples as usize;
        let preamp = f32::from_bits(
            fd.pipeline
                .preamp
                .load(std::sync::atomic::Ordering::Acquire),
        );
        let bypassed = fd
            .pipeline
            .bypass
            .load(std::sync::atomic::Ordering::Acquire);

        if bypassed {
            for i in 0..n {
                *out_left.add(i) = *in_left.add(i) * preamp;
                *out_right.add(i) = *in_right.add(i) * preamp;
            }
        } else {
            let audio_eq = &mut *fd.audio_eq;
            audio_eq.process(in_left, in_right, out_left, out_right, n, preamp);
        }

        let mut max_l = 0.0_f32;
        let mut max_r = 0.0_f32;
        for i in 0..n {
            let abs_l = (*out_left.add(i)).abs();
            let abs_r = (*out_right.add(i)).abs();
            if abs_l > max_l {
                max_l = abs_l;
            }
            if abs_r > max_r {
                max_r = abs_r;
            }
        }
        fd.pipeline
            .peak_l
            .store(max_l.to_bits(), std::sync::atomic::Ordering::Release);
        fd.pipeline
            .peak_r
            .store(max_r.to_bits(), std::sync::atomic::Ordering::Release);
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
            "CONNECTING" => FilterState::Connecting,
            "PAUSED" => FilterState::Paused,
            "STREAMING" => FilterState::Streaming,
            "ERROR" => FilterState::Error(String::new()),
            _ => FilterState::Unconnected,
        };
        let _ = fd.tx.send(PwEvent::FilterStateChanged(filter_state));

        if (new == pipewire_sys::pw_filter_state_PW_FILTER_STATE_PAUSED
            || new == pipewire_sys::pw_filter_state_PW_FILTER_STATE_STREAMING)
            && !fd.monitor_links_created.get()
        {
            let filter_id = pipewire_sys::pw_filter_get_node_id(fd.filter_ptr);
            if filter_id != 0 && filter_id != pipewire_sys::PW_ID_ANY {
                fd.monitor_links_created.set(true);
                tracing::info!(filter_id, "Filter reached {}, creating links", state_str);

                if !fd.filter_ready_sent.get() {
                    fd.filter_ready_sent.set(true);
                    let _ = fd.tx.send(PwEvent::FilterReady { node_id: filter_id });
                }

                if let Some(ns_id) = fd.null_sink_id {
                    create_monitor_links(ns_id, filter_id);
                }
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

#[expect(dead_code, reason = "used via Cell<Option<FilterHandle>> in run()")]
pub(crate) struct FilterHandle {
    filter: *mut pipewire_sys::pw_filter,
    port_in_l: *mut c_void,
    port_in_r: *mut c_void,
    port_out_l: *mut c_void,
    port_out_r: *mut c_void,
    filter_data_ptr: *mut FilterData,
    listener_ptr: *mut libspa_sys::spa_hook,
    events_ptr: *mut pipewire_sys::pw_filter_events,
}

impl FilterHandle {
    pub(crate) unsafe fn destroy(self) {
        unsafe {
            pipewire_sys::pw_filter_set_active(self.filter, false);
            pipewire_sys::pw_filter_disconnect(self.filter);
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

unsafe fn add_dsp_port(
    filter: *mut pipewire_sys::pw_filter,
    name: &str,
    channel: &str,
    direction: libspa_sys::spa_direction,
) -> *mut c_void {
    let p = Props::new("port.name", name);
    p.set("object.path", name);
    p.set("audio.channel", channel);
    p.set("format.dsp", "32 bit float mono audio");
    unsafe {
        pipewire_sys::pw_filter_add_port(
            filter,
            direction,
            pipewire_sys::pw_filter_port_flags_PW_FILTER_PORT_FLAG_MAP_BUFFERS,
            0,
            p.into_raw(),
            ptr::null_mut(),
            0,
        )
    }
}

// Filter creation
pub(crate) fn create_eq_filter(
    core_raw: *mut pipewire_sys::pw_core,
    pipeline: &Arc<Pipeline>,
    tx: &mpsc::Sender<PwEvent>,
    null_sink_id: Option<u32>,
    audio_eq: *mut AudioEq,
) -> Option<FilterHandle> {
    let props = Props::new("media.type", "Audio");
    props.set("media.category", "Duplex");
    props.set("media.role", "DSP");
    props.set("node.name", DSP_NODE_NAME);
    props.set("node.description", "eqtui (Processor)");
    props.set("node.virtual", "true");
    props.set("priority.session", "0");

    let name_cstr =
        CString::new(DSP_NODE_NAME).expect("static filter name should not contain null");
    let filter =
        unsafe { pipewire_sys::pw_filter_new(core_raw, name_cstr.as_ptr(), props.into_raw()) };

    if filter.is_null() {
        let _ = tx.send(PwEvent::Error("pw_filter_new failed".into()));
        return None;
    }

    let in_left =
        unsafe { add_dsp_port(filter, "input_FL", "FL", libspa_sys::SPA_DIRECTION_INPUT) };
    let in_right =
        unsafe { add_dsp_port(filter, "input_FR", "FR", libspa_sys::SPA_DIRECTION_INPUT) };
    let out_left =
        unsafe { add_dsp_port(filter, "output_FL", "FL", libspa_sys::SPA_DIRECTION_OUTPUT) };
    let out_right =
        unsafe { add_dsp_port(filter, "output_FR", "FR", libspa_sys::SPA_DIRECTION_OUTPUT) };

    if in_left.is_null() || in_right.is_null() || out_left.is_null() || out_right.is_null() {
        let _ = tx.send(PwEvent::Error("pw_filter_add_port failed".into()));
        return None;
    }

    let filter_data = Box::new(FilterData {
        pipeline: pipeline.clone(),
        audio_eq,
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
    #[allow(clippy::cast_sign_loss)]
    audio_info.set_rate(SAMPLE_RATE as u32);
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

    let handle = FilterHandle {
        filter,
        port_in_l: in_left,
        port_in_r: in_right,
        port_out_l: out_left,
        port_out_r: out_right,
        filter_data_ptr,
        listener_ptr,
        events_ptr,
    };

    if ret != 0 {
        let _ = tx.send(PwEvent::Error(format!("filter_connect failed: {ret}")));
        unsafe {
            handle.destroy();
        }
        return None;
    }

    unsafe {
        pipewire_sys::pw_filter_set_active(filter, true);
    }

    Some(handle)
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
}

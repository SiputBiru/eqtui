//! Thin wrappers around `pipewire_sys::pw_filter_*` C API.
//!
//! The `pipewire` crate (v0.9.x) does not expose `pw_filter` — only `StreamBox`.
//! EasyEffects uses `pw_filter` to insert a processing node directly into the
//! PipeWire graph (lower latency, correct architecture).
//!
//! `pipewire-sys` generates all FFI declarations via bindgen. This module
//! just provides Rust-idiomatic wrappers and re-exports the necessary types.
//!
//! Plan to upstream to `pipewire-rs` once proven stable.

#![allow(unsafe_op_in_unsafe_fn)]
#![allow(dead_code, unused_imports)]

use std::ffi::CString;
use std::os::raw::{c_char, c_void};

pub use pipewire_sys::{
    pw_buffer, pw_core, pw_filter, pw_filter_events, pw_filter_flags, pw_filter_port_flags,
    pw_filter_state, pw_properties,
};

/// Create a new `pw_properties` with a single key-value pair.
///
/// # Safety
/// The returned pointer must be freed with `properties_free`.
pub unsafe fn properties_new(key: &str, value: &str) -> *mut pw_properties {
    let c_key = CString::new(key).expect("key must not contain interior NUL");
    let c_value = CString::new(value).expect("value must not contain interior NUL");
    pipewire_sys::pw_properties_new(
        c_key.as_ptr(),
        c_value.as_ptr(),
        std::ptr::null::<c_char>(),
    )
}

/// Set a property on an existing `pw_properties`.
///
/// # Safety
/// `props` must be a valid, non-null pointer returned by `properties_new`.
pub unsafe fn properties_set(props: *mut pw_properties, key: &str, value: &str) {
    let c_key = CString::new(key).expect("key must not contain interior NUL");
    let c_value = CString::new(value).expect("value must not contain interior NUL");
    pipewire_sys::pw_properties_set(props, c_key.as_ptr(), c_value.as_ptr());
}

/// Free a `pw_properties`.
///
/// # Safety
/// `props` must be a valid, non-null pointer returned by `properties_new`.
pub unsafe fn properties_free(props: *mut pw_properties) {
    pipewire_sys::pw_properties_free(props);
}
pub use pipewire_sys::{
    pw_filter_flags_PW_FILTER_FLAG_RT_PROCESS as PW_FILTER_FLAG_RT_PROCESS,
    pw_filter_port_flags_PW_FILTER_PORT_FLAG_MAP_BUFFERS as PW_FILTER_PORT_FLAG_MAP_BUFFERS,
    pw_filter_state_PW_FILTER_STATE_CONNECTING as PW_FILTER_STATE_CONNECTING,
    pw_filter_state_PW_FILTER_STATE_ERROR as PW_FILTER_STATE_ERROR,
    pw_filter_state_PW_FILTER_STATE_PAUSED as PW_FILTER_STATE_PAUSED,
    pw_filter_state_PW_FILTER_STATE_STREAMING as PW_FILTER_STATE_STREAMING,
    pw_filter_state_PW_FILTER_STATE_UNCONNECTED as PW_FILTER_STATE_UNCONNECTED,
};

pub use libspa_sys::{spa_direction, spa_hook, spa_io_position, spa_pod};

pub unsafe fn filter_new(
    core: *mut pw_core,
    name: &str,
    props: *mut pw_properties,
) -> *mut pw_filter {
    let c_name = CString::new(name).expect("filter name must not contain interior NUL");
    pipewire_sys::pw_filter_new(core, c_name.as_ptr(), props)
}

pub unsafe fn filter_add_port(
    filter: *mut pw_filter,
    direction: spa_direction,
    flags: pw_filter_port_flags,
    port_data_size: usize,
    props: *mut pw_properties,
    params: *mut *const spa_pod,
    n_params: u32,
) -> *mut c_void {
    pipewire_sys::pw_filter_add_port(
        filter, direction, flags, port_data_size, props, params, n_params,
    )
}

pub unsafe fn filter_connect(
    filter: *mut pw_filter,
    flags: pw_filter_flags,
    params: *mut *const spa_pod,
    n_params: u32,
) -> i32 {
    pipewire_sys::pw_filter_connect(filter, flags, params, n_params)
}

pub unsafe fn filter_add_listener(
    filter: *mut pw_filter,
    listener: *mut spa_hook,
    events: *const pw_filter_events,
    data: *mut c_void,
) {
    pipewire_sys::pw_filter_add_listener(filter, listener, events, data);
}

pub unsafe fn filter_dequeue_buffer(port_data: *mut c_void) -> *mut pw_buffer {
    pipewire_sys::pw_filter_dequeue_buffer(port_data)
}

pub unsafe fn filter_queue_buffer(port_data: *mut c_void, buffer: *mut pw_buffer) -> i32 {
    pipewire_sys::pw_filter_queue_buffer(port_data, buffer)
}

pub unsafe fn filter_get_dsp_buffer(port_data: *mut c_void, n_samples: u32) -> *mut f32 {
    pipewire_sys::pw_filter_get_dsp_buffer(port_data, n_samples) as *mut f32
}

pub unsafe fn filter_set_active(filter: *mut pw_filter, active: bool) -> i32 {
    pipewire_sys::pw_filter_set_active(filter, active)
}

pub unsafe fn filter_destroy(filter: *mut pw_filter) {
    pipewire_sys::pw_filter_destroy(filter);
}

pub unsafe fn filter_get_node_id(filter: *mut pw_filter) -> u32 {
    pipewire_sys::pw_filter_get_node_id(filter)
}

pub unsafe fn filter_disconnect(filter: *mut pw_filter) -> i32 {
    pipewire_sys::pw_filter_disconnect(filter)
}

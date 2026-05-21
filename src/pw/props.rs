use std::ffi::CString;
use std::mem;
use std::os::raw::c_char;
use std::ptr;

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

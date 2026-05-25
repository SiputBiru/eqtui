// Copyright (C) 2026 SiputBiru <hillsforrest03@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Default sample rate for the DSP pipeline (48 kHz).
/// `PipeWire` negotiates this format via SPA; all DSP is computed at this rate.
pub const SAMPLE_RATE: f32 = 48_000.0;

pub struct Pipeline {
    pub(crate) bypass: AtomicBool,
    pub(crate) preamp: AtomicU32,
    pub(crate) peak_l: AtomicU32,
    pub(crate) peak_r: AtomicU32,
}

impl Pipeline {
    pub fn new(_sample_rate: f32) -> Self {
        Self {
            bypass: AtomicBool::new(false),
            preamp: AtomicU32::new(1.0_f32.to_bits()),
            peak_l: AtomicU32::new(0.0_f32.to_bits()),
            peak_r: AtomicU32::new(0.0_f32.to_bits()),
        }
    }

    /// Process audio through the pipeline.
    ///
    /// # Safety
    /// `in_l`, `in_r`, `out_l`, and `out_r` must be valid for reads/writes of `n` samples.
    pub unsafe fn process(
        &self,
        in_l: *const f32,
        in_r: *const f32,
        out_l: *mut f32,
        out_r: *mut f32,
        n: usize,
    ) {
        debug_assert!(!in_l.is_null(), "Input left buffer is null");
        debug_assert!(!in_r.is_null(), "Input right buffer is null");
        debug_assert!(!out_l.is_null(), "Output left buffer is null");
        debug_assert!(n > 0, "Process called with zero samples");

        let preamp = if self.bypass.load(Ordering::Acquire) {
            1.0
        } else {
            f32::from_bits(self.preamp.load(Ordering::Acquire))
        };

        unsafe {
            for i in 0..n {
                *out_l.add(i) = *in_l.add(i) * preamp;
                *out_r.add(i) = *in_r.add(i) * preamp;
            }
        }

        let mut max_l = 0.0_f32;
        let mut max_r = 0.0_f32;
        for i in 0..n {
            let abs_l = unsafe { (*out_l.add(i)).abs() };
            let abs_r = unsafe { (*out_r.add(i)).abs() };
            if abs_l > max_l {
                max_l = abs_l;
            }
            if abs_r > max_r {
                max_r = abs_r;
            }
        }

        self.peak_l.store(max_l.to_bits(), Ordering::Release);
        self.peak_r.store(max_r.to_bits(), Ordering::Release);
    }

    pub fn peaks(&self) -> (f32, f32) {
        (
            f32::from_bits(self.peak_l.load(Ordering::Acquire)),
            f32::from_bits(self.peak_r.load(Ordering::Acquire)),
        )
    }

    pub fn set_preamp(&self, gain_db: f32) {
        let linear = 10.0_f32.powf(gain_db / 20.0);
        self.preamp.store(linear.to_bits(), Ordering::Release);
    }

    pub fn set_bypass(&self, bypass: bool) {
        self.bypass.store(bypass, Ordering::Release);
    }

    pub fn is_bypassed(&self) -> bool {
        self.bypass.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bypass_passthrough() {
        let p = Pipeline::new(SAMPLE_RATE);
        p.set_bypass(true);
        assert!(p.is_bypassed());

        let input = vec![0.7_f32; 64];
        let mut lo = vec![0.0_f32; 64];
        let mut ro = vec![0.0_f32; 64];
        unsafe {
            p.process(
                input.as_ptr(),
                input.as_ptr(),
                lo.as_mut_ptr(),
                ro.as_mut_ptr(),
                input.len(),
            );
        }
        assert_eq!(lo, input);
    }

    #[test]
    fn set_bypass_roundtrip() {
        let p = Pipeline::new(SAMPLE_RATE);
        assert!(!p.is_bypassed());
        p.set_bypass(true);
        assert!(p.is_bypassed());
        p.set_bypass(false);
        assert!(!p.is_bypassed());
    }

    #[test]
    fn peak_measurement() {
        let p = Pipeline::new(SAMPLE_RATE);
        let left_in = [0.5_f32, -0.8_f32, 0.3_f32];
        let right_in = [0.1_f32, 0.2_f32, -0.9_f32];
        let mut left_out = [0.0_f32; 3];
        let mut right_out = [0.0_f32; 3];

        p.set_bypass(true);
        unsafe {
            p.process(
                left_in.as_ptr(),
                right_in.as_ptr(),
                left_out.as_mut_ptr(),
                right_out.as_mut_ptr(),
                left_in.len(),
            );
        };

        let (pk_l, pk_r) = p.peaks();
        assert!((pk_l - 0.8).abs() < 1e-6);
        assert!((pk_r - 0.9).abs() < 1e-6);
    }

    #[test]
    fn preamp_applies_gain() {
        let p = Pipeline::new(SAMPLE_RATE);
        p.set_preamp(-6.0);

        let input = vec![0.5_f32; 64];
        let mut lo = vec![0.0_f32; 64];
        let mut ro = vec![0.0_f32; 64];
        unsafe {
            p.process(
                input.as_ptr(),
                input.as_ptr(),
                lo.as_mut_ptr(),
                ro.as_mut_ptr(),
                input.len(),
            );
        };
        // -6dB preamp = 10^(-6/20) ≈ 0.501 linear gain
        // input 0.5 * 0.501 ≈ 0.251
        let expected = 0.251_f32;
        assert!((lo[0] - expected).abs() < 0.01);
    }
}

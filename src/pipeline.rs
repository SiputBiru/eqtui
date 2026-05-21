use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use color_eyre::eyre::Context;

use crate::AppResult;
use crate::effects::EffectPlugin;
use crate::effects::equalizer::Equalizer;
use crate::state::EqBand;

/// Default sample rate for the DSP pipeline (48 kHz).
/// `PipeWire` negotiates this format via SPA; all DSP is computed at this rate.
pub const SAMPLE_RATE: f32 = 48_000.0;

pub struct Pipeline {
    eq: Equalizer,
    bypass: AtomicBool,
    preamp: AtomicU32,
    peak_l: AtomicU32,
    peak_r: AtomicU32,
}

impl Pipeline {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            eq: Equalizer::new(sample_rate),
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
        let preamp = f32::from_bits(self.preamp.load(Ordering::Relaxed));

        if self.bypass.load(Ordering::Relaxed) {
            unsafe {
                for i in 0..n {
                    *out_l.add(i) = *in_l.add(i) * preamp;
                    *out_r.add(i) = *in_r.add(i) * preamp;
                }
            }
        } else {
            unsafe { self.eq.process(in_l, in_r, out_l, out_r, n) };
            unsafe {
                for i in 0..n {
                    *out_l.add(i) *= preamp;
                    *out_r.add(i) *= preamp;
                }
            }
        }

        let mut max_l = 0.0_f32;
        unsafe {
            for i in 0..n {
                let abs = (*out_l.add(i)).abs();
                if abs > max_l {
                    max_l = abs;
                }
            }
        }

        let mut max_r = 0.0_f32;
        unsafe {
            for i in 0..n {
                let abs = (*out_r.add(i)).abs();
                if abs > max_r {
                    max_r = abs;
                }
            }
        }

        self.peak_l.store(max_l.to_bits(), Ordering::Relaxed);
        self.peak_r.store(max_r.to_bits(), Ordering::Relaxed);
    }

    pub fn peaks(&self) -> (f32, f32) {
        (
            f32::from_bits(self.peak_l.load(Ordering::Relaxed)),
            f32::from_bits(self.peak_r.load(Ordering::Relaxed)),
        )
    }

    pub fn set_preamp(&self, gain_db: f32) {
        let linear = 10.0_f32.powf(gain_db / 20.0);
        self.preamp.store(linear.to_bits(), Ordering::Relaxed);
    }

    pub fn set_bands(&self, bands: Vec<EqBand>, sample_rate: f32) -> AppResult<()> {
        self.eq
            .set_bands(&bands, sample_rate)
            .wrap_err("Pipeline failed to set EQ bands")
    }

    pub fn set_bypass(&self, bypass: bool) {
        self.bypass.store(bypass, Ordering::Relaxed);
    }

    pub fn is_bypassed(&self) -> bool {
        self.bypass.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::FilterType;

    #[test]
    fn bypass_passthrough() {
        let p = Pipeline::new(SAMPLE_RATE);
        p.set_bypass(true);
        assert!(p.is_bypassed());

        let input = vec![0.7_f32; 64];
        let mut lo = vec![0.0_f32; 64];
        let mut ro = vec![0.0_f32; 64];
        unsafe { p.process(input.as_ptr(), input.as_ptr(), lo.as_mut_ptr(), ro.as_mut_ptr(), input.len()) };
        assert_eq!(lo, input);
    }

    #[test]
    fn process_with_eq_produces_finite_output() {
        let p = Pipeline::new(SAMPLE_RATE);
        let bands = vec![EqBand {
            frequency: 500.0,
            gain: 3.0,
            q: 1.0,
            filter_type: FilterType::Peak,
        }];
        p.set_bands(bands, SAMPLE_RATE).unwrap();

        let input = vec![0.3_f32; 256];
        let mut lo = vec![0.0_f32; 256];
        let mut ro = vec![0.0_f32; 256];
        unsafe { p.process(input.as_ptr(), input.as_ptr(), lo.as_mut_ptr(), ro.as_mut_ptr(), input.len()) };
        assert!(lo.iter().all(|s| s.is_finite()));
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
    fn preamp_prevents_clipping() {
        let p = Pipeline::new(SAMPLE_RATE);
        // User's Filter 8: PK Fc 4573 Hz Gain 16.00 dB Q 0.400
        let bands = vec![EqBand {
            frequency: 4573.0,
            gain: 16.0,
            q: 0.4,
            filter_type: FilterType::Peak,
        }];
        p.set_bands(bands, SAMPLE_RATE).unwrap();

        // With -16.1dB preamp, total gain should be slightly below 0dB (1.0)
        p.set_preamp(-16.1);

        let n = 1024;
        let freq = 4573.0;
        let input: Vec<f32> = (0..n)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let idx = i as f32;
                (2.0 * std::f32::consts::PI * freq * idx / SAMPLE_RATE).sin()
            })
            .collect();
        let mut lo = vec![0.0_f32; n];
        let mut ro = vec![0.0_f32; n];
        unsafe {
            p.process(
                input.as_ptr(),
                input.as_ptr(),
                lo.as_mut_ptr(),
                ro.as_mut_ptr(),
                input.len(),
            );
        };

        let max_val = lo.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
        println!("Max output value with preamp: {}", max_val);
        assert!(max_val <= 1.0, "Expected no clipping (<= 1.0) but got {}", max_val);
    }
}

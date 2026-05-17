use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::effects::equalizer::Equalizer;
use crate::effects::EffectPlugin;
use crate::state::EqBand;

pub struct Pipeline {
    eq: Equalizer,
    bypass: AtomicBool,
    peak_l: AtomicU32,
    peak_r: AtomicU32,
}

impl Pipeline {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            eq: Equalizer::new(sample_rate),
            bypass: AtomicBool::new(false),
            peak_l: AtomicU32::new(0.0_f32.to_bits()),
            peak_r: AtomicU32::new(0.0_f32.to_bits()),
        }
    }

    pub fn process(
        &self,
        left_in: &[f32],
        right_in: &[f32],
        left_out: &mut [f32],
        right_out: &mut [f32],
    ) {
        if self.bypass.load(Ordering::Relaxed) {
            let n = left_in.len().min(left_out.len());
            left_out[..n].copy_from_slice(&left_in[..n]);
            right_out[..n].copy_from_slice(&right_in[..n]);
        } else {
            self.eq.process(left_in, right_in, left_out, right_out);
        }

        // Calculate and store peaks
        let mut max_l = 0.0_f32;
        for &sample in left_out.iter() {
            let abs = sample.abs();
            if abs > max_l {
                max_l = abs;
            }
        }

        let mut max_r = 0.0_f32;
        for &sample in right_out.iter() {
            let abs = sample.abs();
            if abs > max_r {
                max_r = abs;
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

    pub fn set_bands(&self, bands: Vec<EqBand>, sample_rate: f32) {
        self.eq.set_bands(&bands, sample_rate);
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
        let p = Pipeline::new(48000.0);
        p.set_bypass(true);
        assert!(p.is_bypassed());

        let input = vec![0.7_f32; 64];
        let mut lo = vec![0.0_f32; 64];
        let mut ro = vec![0.0_f32; 64];
        p.process(&input, &input, &mut lo, &mut ro);
        assert_eq!(lo, input);
    }

    #[test]
    fn process_with_eq_produces_finite_output() {
        let p = Pipeline::new(48000.0);
        let bands = vec![EqBand {
            frequency: 500.0,
            gain: 3.0,
            q: 1.0,
            filter_type: FilterType::Peak,
        }];
        p.set_bands(bands, 48000.0);

        let input = vec![0.3_f32; 256];
        let mut lo = vec![0.0_f32; 256];
        let mut ro = vec![0.0_f32; 256];
        p.process(&input, &input, &mut lo, &mut ro);
        assert!(lo.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn set_bypass_roundtrip() {
        let p = Pipeline::new(48000.0);
        assert!(!p.is_bypassed());
        p.set_bypass(true);
        assert!(p.is_bypassed());
        p.set_bypass(false);
        assert!(!p.is_bypassed());
    }

    #[test]
    fn peak_measurement() {
        let p = Pipeline::new(48000.0);
        let left_in = vec![0.5_f32, -0.8_f32, 0.3_f32];
        let right_in = vec![0.1_f32, 0.2_f32, -0.9_f32];
        let mut left_out = vec![0.0_f32; 3];
        let mut right_out = vec![0.0_f32; 3];

        p.set_bypass(true);
        p.process(&left_in, &right_in, &mut left_out, &mut right_out);

        let (pk_l, pk_r) = p.peaks();
        assert!((pk_l - 0.8).abs() < 1e-6);
        assert!((pk_r - 0.9).abs() < 1e-6);
    }
}

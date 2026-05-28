// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use crate::state::{EqBand, FilterType};

struct BiquadCoeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

#[derive(Clone)]
struct BiquadState {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Default for BiquadState {
    fn default() -> Self {
        Self {
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }
}

/// Biquad equalizer with no internal synchronization.
/// All access must happen from a single thread.
pub struct AudioEq {
    coeffs: Vec<BiquadCoeffs>,
    states_l: Vec<BiquadState>,
    states_r: Vec<BiquadState>,
}

impl AudioEq {
    pub fn new(_sample_rate: f32) -> Self {
        Self {
            coeffs: Vec::new(),
            states_l: Vec::new(),
            states_r: Vec::new(),
        }
    }

    pub fn set_bands(&mut self, bands: &[EqBand], sample_rate: f32) {
        self.coeffs = bands
            .iter()
            .map(|b| biquad_coefficients(b, sample_rate))
            .collect();
        let len = self.coeffs.len();
        self.states_l.resize(len, BiquadState::default());
        self.states_r.resize(len, BiquadState::default());
    }

    /// Process `n` audio samples through the biquad chain with preamp.
    ///
    /// # Safety
    /// `in_l`, `in_r`, `out_l`, `out_r` must be valid for reads/writes of `n` samples.
    #[allow(
        clippy::many_single_char_names,
        reason = "short variable names like n/l/r/s/y are standard notation for biquad filter math"
    )]
    pub unsafe fn process(
        &mut self,
        in_l: *const f32,
        in_r: *const f32,
        out_l: *mut f32,
        out_r: *mut f32,
        n: usize,
        preamp: f32,
    ) {
        if self.coeffs.is_empty() {
            unsafe {
                for i in 0..n {
                    *out_l.add(i) = *in_l.add(i) * preamp;
                    *out_r.add(i) = *in_r.add(i) * preamp;
                }
            }
            return;
        }

        unsafe {
            for i in 0..n {
                let mut l = *in_l.add(i);
                let mut r = *in_r.add(i);

                for (band_i, coeffs) in self.coeffs.iter().enumerate() {
                    let s = &mut self.states_l[band_i];
                    let mut y = coeffs.b0 * l + coeffs.b1 * s.x1 + coeffs.b2 * s.x2
                        - coeffs.a1 * s.y1
                        - coeffs.a2 * s.y2;

                    if y.abs() < 1.0e-15 {
                        y = 0.0;
                    }

                    s.x2 = s.x1;
                    s.x1 = l;
                    s.y2 = s.y1;
                    s.y1 = y;
                    l = y;
                }

                for (band_i, coeffs) in self.coeffs.iter().enumerate() {
                    let s = &mut self.states_r[band_i];
                    let mut y = coeffs.b0 * r + coeffs.b1 * s.x1 + coeffs.b2 * s.x2
                        - coeffs.a1 * s.y1
                        - coeffs.a2 * s.y2;

                    if y.abs() < 1.0e-15 {
                        y = 0.0;
                    }

                    s.x2 = s.x1;
                    s.x1 = r;
                    s.y2 = s.y1;
                    s.y1 = y;
                    r = y;
                }

                *out_l.add(i) = l * preamp;
                *out_r.add(i) = r * preamp;
            }
        }
    }
}

fn biquad_coefficients(band: &EqBand, sample_rate: f32) -> BiquadCoeffs {
    use std::f32::consts::PI;

    let freq = band.frequency.clamp(10.0, sample_rate * 0.49);
    let gain_linear = 10.0_f32.powf(band.gain / 40.0);
    let w0 = 2.0 * PI * freq / sample_rate;
    let cos_w0 = w0.cos();
    let sin_w0 = w0.sin();
    let alpha = sin_w0 / (2.0 * band.q.max(0.01));

    let (b0, b1, b2, a1, a2) = match band.filter_type {
        FilterType::Peak => {
            let a0 = 1.0 + alpha / gain_linear;
            let a0_inv = 1.0 / a0;

            let b0 = (1.0 + alpha * gain_linear) * a0_inv;
            let b1 = (-2.0 * cos_w0) * a0_inv;
            let b2 = (1.0 - alpha * gain_linear) * a0_inv;
            let a1 = (-2.0 * cos_w0) * a0_inv;
            let a2 = (1.0 - alpha / gain_linear) * a0_inv;

            (b0, b1, b2, a1, a2)
        }
        FilterType::LowShelf => {
            let a = gain_linear + 1.0;
            let b = gain_linear - 1.0;
            let c = 2.0 * gain_linear.sqrt() * alpha;

            let a0 = a + b * cos_w0 + c;
            let a0_inv = 1.0 / a0;

            let b0 = gain_linear * (a - b * cos_w0 + c) * a0_inv;
            let b1 = 2.0 * gain_linear * (b - a * cos_w0) * a0_inv;
            let b2 = gain_linear * (a - b * cos_w0 - c) * a0_inv;
            let a1 = -2.0 * (b + a * cos_w0) * a0_inv;
            let a2 = (a + b * cos_w0 - c) * a0_inv;

            (b0, b1, b2, a1, a2)
        }
        FilterType::HighShelf => {
            let a = gain_linear + 1.0;
            let b = gain_linear - 1.0;
            let c = 2.0 * gain_linear.sqrt() * alpha;

            let a0 = a - b * cos_w0 + c;
            let a0_inv = 1.0 / a0;

            let b0 = gain_linear * (a + b * cos_w0 + c) * a0_inv;
            let b1 = -2.0 * gain_linear * (b + a * cos_w0) * a0_inv;
            let b2 = gain_linear * (a + b * cos_w0 - c) * a0_inv;
            let a1 = 2.0 * (b - a * cos_w0) * a0_inv;
            let a2 = (a - b * cos_w0 - c) * a0_inv;

            (b0, b1, b2, a1, a2)
        }
    };

    BiquadCoeffs { b0, b1, b2, a1, a2 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::SAMPLE_RATE;

    fn rms(samples: &[f32]) -> f32 {
        let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
        #[allow(clippy::cast_precision_loss)]
        let len = samples.len() as f32;
        (sum_sq / len).sqrt()
    }

    #[test]
    fn passthrough_no_bands() {
        let mut eq = AudioEq::new(SAMPLE_RATE);
        let input = vec![0.5_f32; 128];
        let mut lo = vec![0.0_f32; 128];
        let mut ro = vec![0.0_f32; 128];
        unsafe {
            eq.process(
                input.as_ptr(),
                input.as_ptr(),
                lo.as_mut_ptr(),
                ro.as_mut_ptr(),
                input.len(),
                1.0,
            );
        }
        assert_eq!(lo, input);
        assert_eq!(ro, input);
    }

    #[test]
    fn unity_gain_peak() {
        let mut eq = AudioEq::new(SAMPLE_RATE);
        eq.set_bands(
            &[EqBand {
                frequency: 1000.0,
                gain: 0.0,
                q: 1.0,
                filter_type: FilterType::Peak,
            }],
            SAMPLE_RATE,
        );
        let n = 1024;
        let input = vec![0.5_f32; n];
        let mut lo = vec![0.0_f32; n];
        let mut ro = vec![0.0_f32; n];
        unsafe {
            eq.process(
                input.as_ptr(),
                input.as_ptr(),
                lo.as_mut_ptr(),
                ro.as_mut_ptr(),
                input.len(),
                1.0,
            );
        }
        assert!((rms(&input) - rms(&lo)).abs() < 0.1);
    }

    #[test]
    fn positive_gain_boosts() {
        let mut eq = AudioEq::new(SAMPLE_RATE);
        eq.set_bands(
            &[EqBand {
                frequency: 1000.0,
                gain: 6.0,
                q: 1.0,
                filter_type: FilterType::Peak,
            }],
            SAMPLE_RATE,
        );
        let n = 4096;
        let freq = 1000.0;
        let sr = SAMPLE_RATE;
        let input: Vec<f32> = (0..n)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let idx = i as f32;
                (2.0 * std::f32::consts::PI * freq * idx / sr).sin()
            })
            .collect();
        let mut lo = vec![0.0_f32; n];
        let mut ro = vec![0.0_f32; n];
        unsafe {
            eq.process(
                input.as_ptr(),
                input.as_ptr(),
                lo.as_mut_ptr(),
                ro.as_mut_ptr(),
                input.len(),
                1.0,
            );
        }
        assert!(
            rms(&lo) > rms(&input) * 1.3,
            "expected boost, out_rms={:.3}",
            rms(&lo)
        );
    }

    #[test]
    fn negative_gain_cuts() {
        let mut eq = AudioEq::new(SAMPLE_RATE);
        eq.set_bands(
            &[EqBand {
                frequency: 1000.0,
                gain: -6.0,
                q: 1.0,
                filter_type: FilterType::Peak,
            }],
            SAMPLE_RATE,
        );
        let n = 4096;
        let freq = 1000.0;
        let sr = SAMPLE_RATE;
        let input: Vec<f32> = (0..n)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let idx = i as f32;
                (2.0 * std::f32::consts::PI * freq * idx / sr).sin()
            })
            .collect();
        let mut lo = vec![0.0_f32; n];
        let mut ro = vec![0.0_f32; n];
        unsafe {
            eq.process(
                input.as_ptr(),
                input.as_ptr(),
                lo.as_mut_ptr(),
                ro.as_mut_ptr(),
                input.len(),
                1.0,
            );
        }
        assert!(
            rms(&lo) < rms(&input) * 0.7,
            "expected cut, out_rms={:.3}",
            rms(&lo)
        );
    }

    #[test]
    fn multiple_bands_chain() {
        let mut eq = AudioEq::new(SAMPLE_RATE);
        let bands = vec![
            EqBand {
                frequency: 100.0,
                gain: 3.0,
                q: 1.0,
                filter_type: FilterType::LowShelf,
            },
            EqBand {
                frequency: 1000.0,
                gain: -4.0,
                q: 1.0,
                filter_type: FilterType::Peak,
            },
            EqBand {
                frequency: 8000.0,
                gain: 2.0,
                q: 0.7,
                filter_type: FilterType::HighShelf,
            },
        ];
        eq.set_bands(&bands, SAMPLE_RATE);
        let n = 512;
        let input = vec![0.3_f32; n];
        let mut lo = vec![0.0_f32; n];
        let mut ro = vec![0.0_f32; n];
        unsafe {
            eq.process(
                input.as_ptr(),
                input.as_ptr(),
                lo.as_mut_ptr(),
                ro.as_mut_ptr(),
                input.len(),
                1.0,
            );
        }
        assert!(lo.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn low_shelf_boosts_bass() {
        let mut eq = AudioEq::new(SAMPLE_RATE);
        eq.set_bands(
            &[EqBand {
                frequency: 200.0,
                gain: 6.0,
                q: 0.71,
                filter_type: FilterType::LowShelf,
            }],
            SAMPLE_RATE,
        );
        let n = 4096;
        let freq = 50.0;
        let sr = SAMPLE_RATE;
        let input: Vec<f32> = (0..n)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let idx = i as f32;
                (2.0 * std::f32::consts::PI * freq * idx / sr).sin()
            })
            .collect();
        let mut lo = vec![0.0_f32; n];
        let mut ro = vec![0.0_f32; n];
        unsafe {
            eq.process(
                input.as_ptr(),
                input.as_ptr(),
                lo.as_mut_ptr(),
                ro.as_mut_ptr(),
                input.len(),
                1.0,
            );
        }
        assert!(
            rms(&lo) > 1.3,
            "low shelf should boost bass, got {:.3}",
            rms(&lo)
        );
    }
}

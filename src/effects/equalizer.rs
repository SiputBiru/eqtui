// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use crate::state::{EqBand, FilterType};

pub(crate) struct BiquadCoeffs {
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

/// Convert parametric EQ band parameters to biquad filter coefficients using
/// the [Audio EQ Cookbook](https://webaudio.github.io/Audio-EQ-Cookbook/Audio-EQ-Cookbook.txt)
/// formulas (Robert Bristow-Johnson).
///
/// # Bilinear Transform
///
/// The cookbook designs an analog prototype filter, then maps it to the digital
/// domain via the bilinear transform.  The centre frequency is pre-warped so
/// the digital filter matches the target analogue frequency:
///
///   w0 = 2π · f / fs
///
/// # Intermediate Variables
///
///   gain_linear = 10^(gain_dB / 40)   :  peak/shelf gain, linear scale
///   alpha       = sin(w0) / (2 · Q)   :  bandwidth parameter
///
/// # Filter Types
///
/// **Peak (PK)**: boosts or cuts a band around w0.  At DC and Nyquist the
/// response is unity (0 dB).  `a1` intentionally equals `b1`.
///
/// **LowShelf (LS)**: applies constant gain below w0, transitioning to unity
/// above.  The transition slope is set by Q.
///
/// **HighShelf (HS)**: applies constant gain above w0, transitioning to unity
/// below.  The transition slope is set by Q.
///
/// The resulting coefficients (b0, b1, b2, a1, a2) are normalised by a0 so
/// the `a0` term in the denominator is implicitly 1.  They feed directly into
/// the difference equation:
///
///   y[n] = b0·x[n] + b1·x[n-1] + b2·x[n-2] − a1·y[n-1] − a2·y[n-2]
#[allow(
    clippy::doc_markdown,
    reason = "math notation / filter type names in equations"
)]
pub(crate) fn biquad_coefficients(band: &EqBand, sample_rate: f32) -> BiquadCoeffs {
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

/// Compute the magnitude response (in dB) of a biquad filter at a given frequency.
///
/// # Background: The Biquad Transfer Function
///
/// The biquad difference equation is:
///
///   y[n] = b0·x[n] + b1·x[n-1] + b2·x[n-2] − a1·y[n-1] − a2·y[n-2]
///
/// In the z-domain this becomes the transfer function:
///
///   H(z) = (b0 + b1·z⁻¹ + b2·z⁻²) / (1 + a1·z⁻¹ + a2·z⁻²)
///
/// Evaluating the frequency response means setting z = e^{jω} where
/// ω = 2π·f / fs, producing H(e^{jω}): a complex number whose magnitude
/// |H| is the gain at frequency f.
///
/// # Current Implementation: Direct Rectangular Form
///
/// The real and imaginary parts of numerator and denominator are computed
/// separately:
///
///   Numerator:   b0 + b1·e^{-jω} + b2·e^{-2jω}
///              = (b0 + b1·cos ω + b2·cos 2ω) − j·(b1·sin ω + b2·sin 2ω)
///
///   Denominator: 1 + a1·e^{-jω} + a2·e^{-2jω}
///              = (1 + a1·cos ω + a2·cos 2ω) − j·(a1·sin ω + a2·sin 2ω)
///
/// Then:
///
///   |H|² = (N_real² + N_imag²) / (D_real² + D_imag²)
///   |H|_dB = 10 · log₁₀(|H|²)
///
/// This form produces accurate results at all frequencies because it
/// never subtracts nearly-equal large numbers.
///
/// # Superseded: The Expanded Form
///
/// Algebraically expanding |H(e^{jω})|² yields the closed form:
///
///   |H|² = (b0² + b1² + b2² + 2(b0b1 + b1b2)cos ω + 2b0b2 cos 2ω) /
///          (1 + a1² + a2² + 2(a1 + a1a2)cos ω + 2a2 cos 2ω)
///
/// This is algebraically equivalent but loses precision at frequencies far
/// from a band's centre.  At those frequencies cos ω ≈ 1, causing num and
/// den to shrink to near zero as a difference of much larger terms.  f32
/// does not have enough mantissa bits to resolve the result.
/// For a peaking filter at 1 kHz:
///
///   |H(20 Hz)|² ≈ 1.0  →  0 dB
///
/// The expanded form produces:
///
///   Numerator   ≈  1.09 + 3.59 + 0.75 − 7.25 + 1.81 = 0.0007   (true: 0.00025)
///   Denominator ≈  1.00 + 3.59 + 0.83 − 7.25 + 1.82 = 0.0003   (true: 0.00026)
///
/// Five large terms cancel to produce a tiny difference: f32 precision
/// cannot preserve the ratio.  The direct rectangular form avoids this by
/// keeping the computation in separate real/imaginary components, never
/// forcing a large cancellation.
#[allow(
    clippy::similar_names,
    clippy::doc_markdown,
    reason = "math notation uses cos_w/sin_w convention and variables like N_real² in equations"
)]
pub(crate) fn biquad_magnitude_db(coeffs: &BiquadCoeffs, freq_hz: f32, sample_rate: f32) -> f32 {
    use std::f32::consts::PI;

    let w = 2.0 * PI * freq_hz / sample_rate;
    let cos_w = w.cos();
    let cos_2w = (2.0 * w).cos();
    let sin_w = w.sin();
    let sin_2w = (2.0 * w).sin();

    let n_real = coeffs.b0 + coeffs.b1 * cos_w + coeffs.b2 * cos_2w;
    let n_imag = -(coeffs.b1 * sin_w + coeffs.b2 * sin_2w);
    let d_real = 1.0 + coeffs.a1 * cos_w + coeffs.a2 * cos_2w;
    let d_imag = -(coeffs.a1 * sin_w + coeffs.a2 * sin_2w);

    let n_mag_sq = n_real * n_real + n_imag * n_imag;
    let d_mag_sq = d_real * d_real + d_imag * d_imag;

    if d_mag_sq <= 0.0 {
        return -100.0;
    }

    let mag_sq = (n_mag_sq / d_mag_sq).max(f32::EPSILON);
    10.0 * mag_sq.log10()
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

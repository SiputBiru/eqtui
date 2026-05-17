use std::sync::RwLock;

use crate::effects::EffectPlugin;
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

pub struct Equalizer {
    bands: RwLock<Vec<BiquadCoeffs>>,
    states_l: RwLock<Vec<BiquadState>>,
    states_r: RwLock<Vec<BiquadState>>,
    bypass: RwLock<bool>,
}

impl Equalizer {
    pub fn new(sample_rate: f32) -> Self {
        let _ = sample_rate;
        Self {
            bands: RwLock::new(Vec::new()),
            states_l: RwLock::new(Vec::new()),
            states_r: RwLock::new(Vec::new()),
            bypass: RwLock::new(false),
        }
    }

    pub fn set_bands(&self, bands: &[EqBand], sample_rate: f32) {
        let coeffs: Vec<BiquadCoeffs> = bands
            .iter()
            .map(|b| biquad_coefficients(b, sample_rate))
            .collect();

        let len = coeffs.len();
        *self.bands.write().unwrap() = coeffs;
        *self.states_l.write().unwrap() = vec![BiquadState::default(); len];
        *self.states_r.write().unwrap() = vec![BiquadState::default(); len];
    }
}

impl EffectPlugin for Equalizer {
    fn name(&self) -> &str {
        "Equalizer"
    }

    fn process(
        &self,
        left_in: &[f32],
        right_in: &[f32],
        left_out: &mut [f32],
        right_out: &mut [f32],
    ) {
        let n = left_in.len().min(left_out.len());

        if *self.bypass.read().unwrap() {
            left_out[..n].copy_from_slice(&left_in[..n]);
            right_out[..n].copy_from_slice(&right_in[..n]);
            return;
        }

        let bands = self.bands.read().unwrap();
        if bands.is_empty() {
            left_out[..n].copy_from_slice(&left_in[..n]);
            right_out[..n].copy_from_slice(&right_in[..n]);
            return;
        }

        let mut states_l = self.states_l.write().unwrap();
        let mut states_r = self.states_r.write().unwrap();

        for i in 0..n {
            let mut l = left_in[i];
            let mut r = right_in[i];

            for (band_i, coeffs) in bands.iter().enumerate() {
                let s = &mut states_l[band_i];
                let y = coeffs.b0 * l + coeffs.b1 * s.x1 + coeffs.b2 * s.x2
                    - coeffs.a1 * s.y1 - coeffs.a2 * s.y2;
                s.x2 = s.x1;
                s.x1 = l;
                s.y2 = s.y1;
                s.y1 = y;
                l = y;
            }

            for (band_i, coeffs) in bands.iter().enumerate() {
                let s = &mut states_r[band_i];
                let y = coeffs.b0 * r + coeffs.b1 * s.x1 + coeffs.b2 * s.x2
                    - coeffs.a1 * s.y1 - coeffs.a2 * s.y2;
                s.x2 = s.x1;
                s.x1 = r;
                s.y2 = s.y1;
                s.y1 = y;
                r = y;
            }

            left_out[i] = l;
            right_out[i] = r;
        }
    }

    fn bypass(&self) -> bool {
        *self.bypass.read().unwrap()
    }

    fn set_bypass(&mut self, bypass: bool) {
        *self.bypass.write().unwrap() = bypass;
    }

    fn reset(&self) {
        for s in self.states_l.write().unwrap().iter_mut() {
            *s = BiquadState::default();
        }
        for s in self.states_r.write().unwrap().iter_mut() {
            *s = BiquadState::default();
        }
    }
}

fn biquad_coefficients(band: &EqBand, sample_rate: f32) -> BiquadCoeffs {
    use std::f32::consts::PI;

    let freq = band.frequency.clamp(10.0, sample_rate * 0.49);
    let gain_linear = 10.0_f32.powf(band.gain / 20.0);
    let w0 = 2.0 * PI * freq / sample_rate;
    let cos_w0 = w0.cos();
    let sin_w0 = w0.sin();
    let alpha = sin_w0 / (2.0 * band.q.max(0.01));

    let (b0, b1, b2, a1, a2) = match band.filter_type {
        FilterType::Peak => {
            let a = 1.0 + alpha / gain_linear;
            let b = 1.0 + alpha * gain_linear;
            let b_inv = 1.0 / b;

            let b0 = (1.0 + alpha * gain_linear) * b_inv;
            let b1 = (-2.0 * cos_w0) / b;
            let b2 = (1.0 - alpha * gain_linear) / b;
            let a1 = (2.0 * cos_w0) / a;
            let a2 = -(1.0 - alpha / gain_linear) / a;

            (b0, b1, b2, -a1, -a2)
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

            (b0, b1, b2, -a1, -a2)
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

            (b0, b1, b2, -a1, -a2)
        }
    };

    BiquadCoeffs { b0, b1, b2, a1, a2 }
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

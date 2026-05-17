use crate::effects::EffectPlugin;
use crate::state::{EqBand, FilterType};

pub struct Equalizer {
    pub bands: Vec<EqBand>,
    bypass: bool,
    sample_rate: f32,
    // biquad state per band per channel
    biquads_l: Vec<BiquadState>,
    biquads_r: Vec<BiquadState>,
}

struct BiquadState {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Equalizer {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            bands: Vec::new(),
            bypass: false,
            sample_rate,
            biquads_l: Vec::new(),
            biquads_r: Vec::new(),
        }
    }

    pub fn set_bands(&mut self, bands: Vec<EqBand>) {
        self.bands = bands;
        self.recompute_coefficients();
    }

    fn recompute_coefficients(&mut self) {
        self.biquads_l.clear();
        self.biquads_r.clear();

        for band in &self.bands {
            let coeffs = biquad_coefficients(band, self.sample_rate);
            self.biquads_l.push(BiquadState::new(&coeffs));
            self.biquads_r.push(BiquadState::new(&coeffs));
        }
    }
}

impl EffectPlugin for Equalizer {
    fn name(&self) -> &str {
        "Equalizer"
    }

    fn process(
        &mut self,
        left_in: &[f32],
        right_in: &[f32],
        left_out: &mut [f32],
        right_out: &mut [f32],
    ) {
        let n = left_in.len().min(left_out.len());

        if self.bypass || self.biquads_l.is_empty() {
            left_out[..n].copy_from_slice(&left_in[..n]);
            right_out[..n].copy_from_slice(&right_in[..n]);
            return;
        }

        for i in 0..n {
            let mut l = left_in[i];
            let mut r = right_in[i];

            for biq in &mut self.biquads_l {
                l = biq.process(l);
            }
            for biq in &mut self.biquads_r {
                r = biq.process(r);
            }

            left_out[i] = l;
            right_out[i] = r;
        }
    }

    fn bypass(&self) -> bool {
        self.bypass
    }

    fn set_bypass(&mut self, bypass: bool) {
        self.bypass = bypass;
    }

    fn reset(&mut self) {
        for biq in &mut self.biquads_l {
            biq.reset();
        }
        for biq in &mut self.biquads_r {
            biq.reset();
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

#[derive(Clone, Copy)]
struct BiquadCoeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl BiquadState {
    fn new(c: &BiquadCoeffs) -> Self {
        Self {
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
            b0: c.b0,
            b1: c.b1,
            b2: c.b2,
            a1: c.a1,
            a2: c.a2,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

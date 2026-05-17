use crate::effects::equalizer::Equalizer;
use crate::effects::EffectPlugin;
use crate::state::EqBand;

pub struct Pipeline {
    eq: Equalizer,
    bypass: bool,
}

impl Pipeline {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            eq: Equalizer::new(sample_rate),
            bypass: false,
        }
    }

    pub fn process(
        &self,
        left_in: &[f32],
        right_in: &[f32],
        left_out: &mut [f32],
        right_out: &mut [f32],
    ) {
        if self.bypass {
            let n = left_in.len().min(left_out.len());
            left_out[..n].copy_from_slice(&left_in[..n]);
            right_out[..n].copy_from_slice(&right_in[..n]);
            return;
        }

        self.eq.process(left_in, right_in, left_out, right_out);
    }

    pub fn set_bands(&self, bands: Vec<EqBand>, sample_rate: f32) {
        self.eq.set_bands(&bands, sample_rate);
    }

    pub fn toggle_bypass(&mut self) {
        self.bypass = !self.bypass;
    }

    pub fn is_bypassed(&self) -> bool {
        self.bypass
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::FilterType;

    #[test]
    fn bypass_passthrough() {
        let mut p = Pipeline::new(48000.0);
        p.toggle_bypass();
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
    fn toggle_bypass_roundtrip() {
        let mut p = Pipeline::new(48000.0);
        assert!(!p.is_bypassed());
        p.toggle_bypass();
        assert!(p.is_bypassed());
        p.toggle_bypass();
        assert!(!p.is_bypassed());
    }
}

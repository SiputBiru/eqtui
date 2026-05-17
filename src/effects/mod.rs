pub mod equalizer;

pub trait EffectPlugin {
    fn name(&self) -> &str;
    fn process(&self, left_in: &[f32], right_in: &[f32], left_out: &mut [f32], right_out: &mut [f32]);
    fn bypass(&self) -> bool;
    fn set_bypass(&self, bypass: bool);
    fn reset(&self);
}

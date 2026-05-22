// Copyright (C) 2026 SiputBiru <hillsforrest03@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

pub mod equalizer;

pub trait EffectPlugin {
    fn name(&self) -> &str;
    /// Process `n` samples. Raw pointers are used to safely handle
    /// `PipeWire` in-place processing where input and output buffers may alias.
    ///
    /// # Safety
    /// `in_l`, `in_r`, `out_l`, and `out_r` must be valid for reads/writes of `n` samples.
    unsafe fn process(
        &self,
        in_l: *const f32,
        in_r: *const f32,
        out_l: *mut f32,
        out_r: *mut f32,
        n: usize,
    );
    fn bypass(&self) -> bool;
    fn set_bypass(&self, bypass: bool);
    fn reset(&self);
}

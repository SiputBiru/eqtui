// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

pub mod app;
pub mod autoeq;
pub mod cli;
pub mod client;
pub mod daemon;
pub mod effects;
pub mod event;
pub mod handler;
pub mod pipeline;
pub mod profiles;
pub mod protocol;
pub mod pw;
pub mod state;
pub mod tui;

pub type AppResult<T> = color_eyre::Result<T>;

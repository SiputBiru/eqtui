pub mod app;
pub mod config;
pub mod effects;
pub mod event;
pub mod handler;
pub mod pipeline;
pub mod pw;
pub mod pw_filter_ffi;
pub mod state;
pub mod tui;

pub type AppResult<T> = color_eyre::Result<T>;

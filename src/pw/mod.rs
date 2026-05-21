//! `PipeWire` integration — audio graph setup, DSP filter, and link management.
//!
//! # Architecture
//!
//! This module runs on a dedicated **`PipeWire` mainloop thread** spawned in
//! `main.rs`. It communicates with the main TUI thread via two channels:
//!
//! - `mpsc::Sender<PwEvent>` → TUI (node list, state changes, errors)
//! - `Receiver<PwCommand>` ← TUI (connect, disconnect, terminate)
//!
//! # Safety
//!
//! This module contains extensive `unsafe` FFI with `PipeWire`'s C API.
//! Each `unsafe` block is justified with a `// SAFETY:` comment. Callback
//! functions (`process_cb`, `state_changed_cb`, `bound_cb`) are `unsafe
//! extern "C"` and invoked by `PipeWire` on its mainloop thread.

mod filter;
mod links;
mod null_sink;
mod props;
mod run;

pub use run::run;

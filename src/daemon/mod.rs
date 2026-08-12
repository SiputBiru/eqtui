// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

//! Daemon process — owns the `PipeWire` audio pipeline and serves
//! TUI/CLI clients over a Unix-domain socket.
//!
//! The daemon is split into focused submodules:
//!
//! - [`state`] — the shared `DaemonState` (one `Mutex<StatusSnapshot>` +
//!   client registry) and all event-driven mutation of it.
//! - [`auth`] — peer-credential verification for accepted connections.
//! - [`transport`] — per-client connection handling and framing.
//! - [`dispatch`] — request → state mutation / command dispatch.
//! - [`lifecycle`] — startup (lock, threads, accept loop) and the orderly
//!   shutdown sequence.

pub mod auth;
pub mod dispatch;
pub mod lifecycle;
pub mod state;
pub mod transport;

pub use lifecycle::run_daemon;
pub use state::DaemonState;

# Changelog

All notable changes to eqtui are documented here.

---

## [0.1.1-alpha.6] — 2026-05-25

### Audio Engine

- **Zero-lock RT path:** Removed `std::sync::RwLock` from the real-time audio
  thread — EQ processing now runs lock-free on the PipeWire mainloop,
  eliminating xruns during EQ changes.
- **Merged peak detection:** Single-pass peak scan replacing two separate
  loops (~30–40% less overhead per buffer).
- **Folded preamp:** Preamp applied in the same loop as EQ output instead
  of a separate O(n) pass.
- **ARM atomics fix:** Replaced `Relaxed` ordering with `Release`/`Acquire`
  on peak meter atomics — peak meters now work correctly on ARM (Apple
  Silicon, Raspberry Pi, AWS Graviton).
- **`pw-link -I` off mainloop:** Moved the null-sink input source check to a
  dedicated thread, preventing `fork`/`exec`/`waitpid` from blocking the
  PipeWire audio thread and causing periodic glitches.

### Daemon

- **State persistence:** Daemon auto-saves its runtime state (bands, preamp,
  bypass, connected devices) to `~/.local/share/eqtui/state.toml` after
  every change and restores it on startup. Survives crashes and SIGKILLs.
- **TUI reconnection:** Exponential-backoff retry loop (1s → 2s → 4s →
  8s capped, 30s total) when the daemon disconnects. The TUI stays alive,
  shows a `Reconnecting...` status, and resumes automatically.
- **Daemon connection indicator:** New `Daemon:` line in the monitoring
  panel — green `Connected`, yellow `Reconnecting...`, red `Disconnected`.
- **Orphan cleanup:** Auto-launched daemon processes are sent `SIGTERM` if
  they fail to start within the timeout.
- **Log truncation:** Daemon log now starts fresh each session
  (`.truncate(true)` instead of `.append(true)`).
- **Graceful shutdown:** SIGTERM/SIGINT triggers clean PipeWire teardown
  (destroy null-sink and filter nodes, remove socket).
- **PipeWire recovery:** Daemon auto-shuts-down on PipeWire disconnect;
  TUI reconnects and restores state automatically.

### TUI

- **Preamp display:** Preamp value shown above L/R peak meters in the
  monitoring panel.
- **Expanded hints:** Status bar now shows `b` Bypass, `{}` Profile, `r`
  Reset, `:` Command, `v` Visual in Normal mode.
- **Filter-not-ready notification:** Pressing `C` before the PipeWire
  filter is ready now shows a notification instead of silence.
- **Source detection:** `pw-link -I` failures are now distinguished from
  genuine "no source" — the panel shows `Source: ?` when the state can't
  be determined.

### CLI

- Added `--help` and `--version` flags.
- Updated `uds` dependency from 0.4 to 0.4.2.

### Bug Fixes

- Profile `:w` no longer silently swallows write errors — shows
  `Failed to save: ...` notification on disk-full or permission errors.
- TUI device state now updated *after* daemon confirmation, preventing
  phantom connected/disconnected states.
- Bypass mode no longer applies preamp attenuation (unity gain).
- Self-connect guard: pressing `C` on the null-sink or filter itself is
  rejected with a notification.
- Duplicate `connected_devices` entries prevented on rapid double-`C`.
- Float test tolerance relaxed (`f32::EPSILON` → `1e-3`) to prevent
  flaky failures near -60 dB.
- Safe regex capture access in PEQ parser (`caps.get(1)` instead of
  `caps[1]` indexing).

### Refactoring

- Consolidated 11 standalone default-value functions in `config.rs` into
  typed `impl` blocks with `const` defaults.
- Extracted `update_external_profiles()` in `profiles.rs` (−12 duplicated
  lines).
- Extracted `bump_band()` in `handler/normal.rs` (−30 duplicated lines).
- Memoized regex compilation in PEQ parser (`LazyLock<Regex>`).
- Added `DaemonConnection` enum replacing `daemon_connected: bool`.

---

## [0.1.1-alpha.5] and earlier

Initial development releases — daemon/TUI architecture, parametric EQ
engine, Vim-inspired keybindings, AutoEQ PEQ import, profile system,
and PipeWire integration.

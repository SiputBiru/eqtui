# Changelog

All notable changes to eqtui are documented here.

---

## [0.1.3] — 2026-08-11

### Daemon — harden IPC authorization

- **Validate `XDG_RUNTIME_DIR`** before trusting it (exists, is a directory,
  owned by the effective uid, no group/other access). Refuse to start on an
  unsafe runtime dir — fail closed.
- **Private `0700` socket directory** (`$XDG_RUNTIME_DIR/eqtui/`) and an
  explicit `0600` socket node regardless of umask.
- **`SO_PEERCRED` check on every accepted connection** (`uds` crate): peers
  with a foreign uid are rejected and logged.
- **Only unlink a stale socket** if it really is a socket — never a regular
  file or symlink planted at the socket path.
- **Shared path computation** extracted to `src/paths.rs` so daemon and client
  cannot drift.

### Daemon — request validation

- **Bound request lines at 64 KiB** — an oversized line gets an error response
  and the connection is closed (no memory-exhaustion from rogue clients).
- **Validate `SetBands` / `SetPreamp`** before mutating state: band count
  capped at 31, and frequency/gain/Q/preamp must be finite and in range.
  Rejections carry a reason and leave state untouched; empty band lists stay
  legal (clear EQ).
- **NaN peak guard** — non-finite audio input never reaches the peak meters
  (the last good reading is retained).
- Range constants live in `src/state.rs`, shared with the PEQ parser.

### Daemon — lock-file ordering

- **PID is written only after acquiring the flock** — a second instance can no
  longer truncate the running daemon's PID before discovering it failed to
  take the lock. A failed second start now reports the running PID.

### Daemon — orderly shutdown lifecycle

- **Track and join every daemon-owned thread** (pw, bridge, peak,
  null-sink-checker, client handlers) in dependency order; the null-sink
  checker is cancelled via the shared shutdown flag before it can fork
  `pw-link` during teardown.
- **Client handlers are unblocked by closing their streams** on shutdown; the
  socket is removed and the daemon exits cleanly.
- **One shared shutdown flag** (`Arc<AtomicBool>`) observed by the pw thread
  and the daemon.
- **Auto-launched daemons are kept as `Child` handles**: fail-fast on early
  exit via `try_wait`, handle-based `kill`+`wait` on timeout (no bare-PID
  signal, no PID-reuse footgun), and opportunistic reaping in
  `try_read_event` (no zombie).

### Daemon — single-mutex status snapshots

- **Nine field mutexes → one `Mutex<StatusSnapshot>`**: `get_status()` reads a
  snapshot that is internally consistent by construction and acquires a single
  lock. Documented `status → clients` lock order; the real-time audio path
  keeps its lock-free `Pipeline` atomics (intentional dual-write for preamp /
  bypass).

### Daemon — confirmed device routing

- **`pw-link` wrappers return real outcomes** (`Ok`/`Benign`/`Failed`) and
  both channel legs are attempted even if one fails; the link worker reports
  results back as `LinkResult` events.
- **Devices are pending until confirmed**: `connected_devices` only ever holds
  confirmed links; `pending_devices` is exposed in `DaemonStatus` (additive,
  serde-default) and shown in the TUI as a "connecting…" indicator.
- **Dead pw thread → error response** with pending state rolled back;
  vanished devices are pruned from routing state on each node-list cycle.

### Profiles — persistence reliability

- **Atomic saves**: `profiles.toml` is written to a sibling temp file, synced,
  then renamed — readers never see a truncated file, and serialization
  failures now propagate instead of writing a tombstone.
- **Loud recovery on load**: first-run, unreadable, and corrupt files are
  handled distinctly; corrupt files are backed up to `.bak` before fresh
  defaults, and every failure path warns.
- **Restore last active profile**: `profiles.toml` records the active profile
  index (additive, serde-default); the TUI restores the last selected profile
  on startup (clamped) and persists it on switch.
- **Logging**: dropped the world-readable `/tmp` fallback (fails with a clear
  message) and added single-generation rotation at 5 MiB for the TUI log.

### PEQ parser — line-numbered diagnostics

- **Malformed filter lines no longer silently become defaults**:
  `parse_filter_line` returns a reason (bad token, shifted layout,
  non-finite/out-of-range value, unsupported type) and every skipped line is
  collected as a line-numbered `PeqWarning` on the preset.
- **Warnings are surfaced**: `eqtui load` prints them to stderr and profile
  loads log them. Tolerance preserved — one bad filter line never nukes the
  whole file; only an invalid preamp hard-fails.

### Client — protocol error taxonomy

- **New `ClientError`** (`Disconnected` / `Timeout` / `Malformed` / `Io`):
  EOF is now classified as `Disconnected` in both `request()` and
  `try_read_event()` (previously misreported as "Unexpected data" or silently
  treated as "no event"); malformed frames are named, not dropped.
- **The TUI reconnect loop can now distinguish** a dead daemon (reconnect with
  backoff) from a slow one (keep polling) from garbage (resync on the next
  line).

### Structure — daemon module split

- **`src/daemon.rs` split into focused submodules**: `state` (shared state +
  event handling), `auth` (peer credentials), `transport` (per-client
  connection handling), `dispatch` (request dispatch), and `lifecycle`
  (startup + shutdown). Behavior is unchanged.

---

## [0.1.2] — 2026-07-10

### Logging Rework — stderr & systemd-friendly

- **Dropped the log file** (`~/.local/share/eqtui/eqtui.log`).  Logs are now
  routed by mode:
  - **Daemon / CLI** → stderr (terminal or `journalctl` under systemd).
  - **TUI** → `~/.local/share/eqtui/eqtui-tui.log` (alternate screen must
    not be polluted by stray stderr output).
- **`RUST_LOG` support** — control verbosity at runtime.  Defaults to
  `eqtui=info`.  Example: `RUST_LOG=eqtui=debug eqtui daemon`.
- **Centralised logging init** in `src/logging.rs` — removes ~20 lines of
  boilerplate from `main.rs`.
- **Removed dead code** in `daemon::run_daemon()` — was reopening the log
  file that `main()` already set up; did nothing useful.
- **Updated README** — logging behaviour, `RUST_LOG` usage, and a guide
  for running as a systemd user service.

---

## [0.1.1-alpha.7] — 2026-05-29

### Debloat — `daemon.rs`

- **Removed double-fork daemonization** (`init()`). The daemon no longer
  performs POSIX double-fork — it runs as a normal foreground process
  spawned by the TUI or systemd.
- **Removed state persistence** (`save_state`/`restore_state`). The daemon
  no longer auto-saves EQ bands, preamp, or bypass to `state.toml` on
  every change.
- **Removed signal handler + watcher thread**. Clean shutdown is handled
  by a single `AtomicBool` checked in the accept loop.
- **Removed rate limiter** from client handler (unnecessary for local socket).
- **Removed peer credentials check** (`uds` dependency). `$XDG_RUNTIME_DIR`
  already enforces user isolation.
- **Removed `MAX_CLIENTS` limit** and `MAX_BANDS` validation.
- **Removed `catch_unwind` wrapper** — panics propagate naturally.
- **Removed `Response` helpers** and `send_resp()` — inlined.
- **Merged `run()` + `init()` + `run_daemon()`** into a single
  `run_daemon()` entry point.
- **File shrunk from 786 lines to 532 lines** (−32%).

### Debloat — `config.rs` (removed)

- **Deleted `config.rs`** (153 lines). The config system was dead code:
  `Config` was deserialized from `~/.config/eqtui/config.toml` but
  **never read** by any key handler. Handlers used hardcoded keys directly.
  Removing it eliminates `serde::Deserialize` usage in this module and
  simplifies the startup path.
- Removed `pub config: Arc<Config>` field from `App` struct.
- Simplified `App::new(client)` and `App::new_test()` signatures.
- Updated handler tests to use `App::new_test()` without config.
- Updated README to remove config.toml references and "Customizing Keys"
  section.
- **Saves 153 lines + `toml` serde overhead.**

### Debloat — dependencies (`regex` removed)

- **Replaced `regex` crate with manual string parsing** in `autoeq/parser.rs`.
  The two regex patterns (`^Preamp:\s+...` and `Filter\s+\d+:\s+ON\s+...`)
  were replaced with `strip_prefix`, `strip_suffix`, `split`, and
  `split_whitespace` — roughly the same LOC, zero external dependencies.
- **Removed `regex = "1.12.3"` from `Cargo.toml`** — drops the transitive
  dependency tree (`regex-automata`, `regex-syntax`, `aho-corasick`,
  `memchr`), speeding up compile times and shrinking the binary.

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

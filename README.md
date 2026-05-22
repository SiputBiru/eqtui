# eqtui

A keyboard-driven parametric EQ for PipeWire that lives in the terminal.
Built with [Ratatui](https://ratatui.rs).

[EasyEffects](https://github.com/wwmm/easyeffects) is great, but sometimes I just
want a simple EQ — not a full DSP pipeline with a GTK(now Qt) UI. And also
i just want to learn little bit about DSP stuff.

Runs as a background daemon so the EQ keeps going even after closing the TUI.

## Quick Start

```bash
cargo install eqtui
eqtui daemon    # start the engine (background)
eqtui           # open the TUI
```

Close with `q` — the EQ keeps running. Re-attach anytime with `eqtui attach`.

## Features (the short version)

- **Daemon/TUI split** — EQ engine stays alive when the UI closes
- **Parametric EQ** — frequency, gain, Q, filter type per band
- **AutoEQ import** — `:load` any PEQ file from AutoEQ / Squiglink
- **Profile system** — save/switch presets with `:w`
- **Vim-ish controls** — Normal/Insert/Visual/Command modes
- **No GTK, no Qt** — just a terminal and PipeWire

## Config & Profiles

Settings are stored in standard `XDG` locations.

- **Config:** `~/.config/eqtui/config.toml`
- **Profiles:** `~/.config/eqtui/profiles.toml`
- **Logs:** `~/.local/share/eqtui/eqtui.log`

### Profile System

There are 5 profile slots available for saving different presets.

- **Saving:** Use `:w` to save the current EQ bands and preamp gain to the active slot.
- **Switching:** Navigate between profiles in the TUI (number keys or Tab).
- **Persistence:** The daemon loads these profiles automatically on startup.
- **External Files:** Profiles can be linked to external PEQ files. These profiles are **read-only** and display an `[RO]` indicator in the TUI.

### Profile File Format

The `profiles.toml` file contains an array of 5 profiles. A profile can either define its own `bands` or link to an external `path`.

**Example with external file:**
```toml
[[profiles]]
name = "AutoEQ Preset"
path = "@eqs/CVJVIVIANS1_Filters.txt"
```

**Example with inline data:**
```toml
[[profiles]]
name = "Custom Tune"
preamp = -6.0
[[profiles.bands]]
frequency = 100.0
gain = 3.5
q = 0.7
filter_type = "LowShelf"
```

**Fields:**
- `path`: (Optional) Portable path to an external PEQ file. Use `@` for paths relative to the config directory.
- `preamp`: Global gain offset in dB.
- `bands`: List of EQ filters (ignored if `path` is set).
    - `frequency`: Center frequency in Hz.
    - `gain`: Boost or cut in dB.
    - `q`: Quality factor (bandwidth).
    - `filter_type`: Either `"Peak"`, `"LowShelf"`, or `"HighShelf"`.

### Customizing Keys

You can change the default controls in your `config.toml`:

```toml
[keys.normal]
toggle_bypass = 'b'
add_band = 'a'
delete_band = 'd'

[keys.insert]
confirm = '\n'
cancel = '\x1b'
```

## Background Process Details

The daemon uses a few standard Linux mechanisms to work correctly:

- **XDG_RUNTIME_DIR:** The Unix socket is placed here. The daemon will not start if this variable is missing.
- **User Check:** The daemon only accepts connections from the same user ID that started it.
- **File Locking:** Uses a lock file to make sure only one daemon instance runs at a time.
- **POSIX Daemon:** Uses standard `fork` and `setsid` to detach from the terminal.

## Install from Source

```bash
git clone https://github.com/SiputBiru/eqtui
cd eqtui
cargo build --release
```

Needs PipeWire and a Nerd Font.

---

[Project by SiputBiru](LICENSE) — patches welcome but no promises :^)

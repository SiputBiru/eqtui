# eqtui

> Terminal-native audio effects processor for PipeWire.  
> Like EasyEffects, but in your terminal. Vim keybindings included.

## Motivation

**EasyEffects** is the de facto Linux audio effects suite — 33 plugins, PipeWire-native, feature-complete. But it's tied to Qt6/QML and the GTK desktop ecosystem. If you live in the terminal, switch window managers, or just want a lightweight, keyboard-driven audio toolbox, there's nothing.

**eqtui** is toolbox. Same PipeWire pipeline architecture, same effect chain, but rendered entirely in the terminal with `ratatui`. No Qt, no GTK, no DE dependency. Add AutoEQ integration as a first-class feature (import headphone correction profiles with one keypress).

Built with [ratatui] and [PipeWire].

[ratatui]: https://ratatui.rs
[PipeWire]: https://pipewire.org

## Features

- **PipeWire-native** — no PulseAudio compatibility layer, no JACK bridge
- **Parametric equalizer** — per-band frequency, gain, Q, and filter type
- **Vim-inspired modes** — Normal, Insert, Visual, Command
- **Keyboard-first** — every action reachable without a mouse
- **Configurable keybindings** — TOML config, customize everything
- **AutoEQ integration** — import headphone correction profiles directly
- **Single static binary** — no runtime dependencies beyond PipeWire
- **Lightweight** — sub-10MB binary, minimal RAM at idle

## Prerequisites

- **Linux** with [PipeWire](https://pipewire.org) running
- A [Nerd Font](https://www.nerdfonts.com) for icons (optional, but recommended)
- Rust toolchain if building from source

## Installation

### Cargo (crates.io)

```bash
cargo install eqtui
```

### Arch Linux (AUR)

```bash
paru -S eqtui-bin
# or
paru -S eqtui-git
```

### Build from source

```bash
git clone https://github.com/SiputBiru/eqtui
cd eqtui
cargo build --release
# binary at target/release/eqtui
```

## Quick Start

```bash
eqtui
```

Press `Tab` / `l` to switch panels. Press `q` to quit.

## Keybindings

### Global

| Key | Action |
|-----|--------|
| `Tab` / `l` | Next panel |
| `Shift+Tab` / `h` | Previous panel |
| `q` / `Ctrl+c` | Quit |
| `Esc` | Return to Normal mode |

### EQ — Normal Mode

| Key | Action |
|-----|--------|
| `j` / `k` / `↑` / `↓` | Next / previous band |
| `h` / `l` / `←` / `→` | Next / previous column |
| `a` | Add band below cursor |
| `dd` | Delete selected band |
| `gg` | Jump to first band |
| `G` | Jump to last band |
| `b` | Toggle EQ bypass |
| `r` | Reset selected band |
| `R` | Reset all bands |
| `i` | Enter Insert mode |
| `v` | Enter Visual mode |
| `:` | Enter Command mode |
| `Ctrl+a` | Import AutoEQ preset |

### EQ — Insert Mode

| Key | Action |
|-----|--------|
| `Enter` | Confirm, return to Normal |
| `Esc` | Cancel, return to Normal |
| `+` / `-` | Bump value ± step |

### EQ — Visual Mode

| Key | Action |
|-----|--------|
| `j` / `k` | Extend selection |
| `d` | Delete selected bands |

### EQ — Command Mode

| Command | Action |
|---------|--------|
| `:w` | Save preset |
| `:w my-preset` | Save as named preset |
| `:q` | Quit |
| `:flat` | Reset all bands to 0 dB |
| `:graphic-eq` | Switch to graphic EQ mode |

## Configuration

Config file at `~/.config/eqtui/config.toml`:

```toml
# ~/.config/eqtui/config.toml

layout = "SpaceBetween"

[keys.normal]
add_band = "a"
delete_band = "d"
insert_mode = "i"
command_mode = ":"
visual_mode = "v"
toggle_bypass = "b"

[keys.insert]
confirm = "Enter"
cancel = "Esc"
bump_up = "+"
bump_down = "-"
```

## Roadmap

| Phase | Status | Focus |
|-------|--------|-------|
| **0** | ✅ Complete | PipeWire connection, node listing TUI |
| **1** | 🚧 Next | Equalizer engine + vim-mode TUI + config system |
| **2** | ⬜ | AutoEQ integration (CSV/PEQ import, fuzzy search) |
| **3** | ⬜ | More effects (compressor, gate, reverb…) + preset system + notifications |
| **4** | ⬜ | Visualization (EQ curve graph, spectrum analyzer, level meters) + CLI mode |
| **5** | ⬜ | Packaging (AUR, Nix, binary releases, CI/CD) |

See [ROADMAP.md](ROADMAP.md) for full details.

## License

MIT © [SiputBiru](mailto:hillsforrest03@gmail.com)

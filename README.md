# eqtui

Terminal-native audio effects processor for PipeWire.  
A keyboard-driven alternative to EasyEffects, rendered with Ratatui.

<p align="left">
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/Rust-2024_edition-orange.svg" alt="Rust 2024"></a>
  <a href="https://ratatui.rs"><img src="https://img.shields.io/badge/TUI-Ratatui-red.svg" alt="Ratatui"></a>
  <a href="https://pipewire.org"><img src="https://img.shields.io/badge/PipeWire-0.10-blue.svg" alt="PipeWire"></a>
  <img src="https://img.shields.io/badge/Platform-Linux-green.svg" alt="Linux">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-lightgrey.svg" alt="MIT"></a>
  <a href="https://github.com/SiputBiru/eqtui/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/SiputBiru/eqtui/ci.yml?branch=main&label=CI" alt="CI"></a>
</p>

---

## Demo

[Insert Demo/GIF Here]

---

## Overview

Audio processing in the Linux ecosystem often relies on graphical toolkits such as GTK or Qt. For environments centered around terminal interfaces or minimalist window managers, eqtui provides an alternative for audio effect management.

The application utilizes a PipeWire-native architecture to insert processing nodes directly into the audio graph. This approach enables audio processing between applications and hardware sinks without graphical desktop environment dependencies.

## Features

- **PipeWire Integration** — Implements the `pw_filter` API for direct insertion into the audio graph.
- **Parametric Equalizer** — Controls frequency, gain, Q, and filter type per band.
- **Vim-Inspired Interface** — Modal interaction (Normal, Insert, Visual, Command) for keyboard-driven operation.
- **AutoEQ Support** — Imports headphone correction profiles from the AutoEQ project.
- **Resource Usage** — Distributed as a static binary with low memory and CPU requirements.
- **Configuration** — Customizable keybindings and settings via TOML.

### Architecture

```
 [Spotify] ─┐
 [Firefox] ─┤                   eqtui
 [mpd    ] ─┤  ┌──────────────────────────────────────┐
            ├──►  Null Sink       pw_filter           ├──► [Real Speakers]
            │    Audio/Sink      (no media.class)      │
            │    wiremix ✓       wiremix ignores       │
            └──────────────────────────────────────────┘
```

eqtui uses the **EasyEffects pattern** to integrate into PipeWire without
disrupting audio mixers like wiremix:

1. **Virtual Null Sink** — A real PipeWire node (`media.class=Audio/Sink`) created
   via the `support.null-audio-sink` adapter factory. Appears as "eqtui Equalizer" in
   system sound settings and supports full `PortConfig` enumeration so mixers can
   monitor it without errors.

2. **Internal `pw_filter`** — A lightweight PipeWire filter with no `media.class`
   that processes audio silently behind the scenes. Catches audio from the null
   sink's monitor ports, applies the EQ chain, and outputs to the default sink.
   Invisible to mixers so it never triggers PortConfig-related crashes.

3. **Direct C FFI** — The filter and null sink use raw `pipewire_sys` / `libspa_sys`
   bindings for capabilities not yet exposed by safe `pipewire-rs`.

## Prerequisites

- **Linux** with [PipeWire](https://pipewire.org) installed and running.
- A [Nerd Font](https://www.nerdfonts.com) for interface icons (recommended).
- Rust toolchain for building from source.

## Installation

### Cargo

```bash
cargo install eqtui
```

### Build from Source

```bash
git clone https://github.com/SiputBiru/eqtui
cd eqtui
cargo build --release
# Executable located at target/release/eqtui
```

## Usage

Start the application:

```bash
eqtui
```

### Interface Navigation

- `Tab` / `l`: Cycle to the next panel.
- `Shift+Tab` / `h`: Cycle to the previous panel.
- `q` / `Ctrl+c`: Exit the application.
- `Esc`: Return to Normal mode.

### Keybindings

| Mode | Key | Action |
|------|-----|--------|
| **Normal** | `j` / `k` / `↑` / `↓` | Navigate bands |
| | `h` / `l` / `←` / `→` | Navigate columns |
| | `a` | Add new band |
| | `dd` | Delete selected band |
| | `b` | Toggle bypass |
| | `r` / `R` | Reset band / Reset all |
| | `Ctrl+a` | Import AutoEQ preset |
| **Insert** | `Enter` / `Esc` | Confirm / Cancel changes |
| | `+` / `-` | Adjust values |
| **Visual** | `j` / `k` | Extend selection |
| | `d` | Delete selection |
| **Command** | `:w` | Save preset |
| | `:flat` | Reset all bands to 0 dB |

## Configuration

Settings are managed via `~/.config/eqtui/config.toml`. Keybindings and layout preferences are configurable.

```toml
# Example config.toml
layout = "SpaceBetween"

[keys.normal]
add_band = "a"
toggle_bypass = "b"
```

## Roadmap

| Phase | Focus | Status |
|-------|-------|--------|
| **1** | Core Equalizer Engine & TUI | Complete |
| **2A** | Virtual Null Sink | Complete |
| **2B** | AutoEQ Integration | Planned |
| **3** | Dynamic Preset System & Additional Effects | Planned |
| **4** | Real-time Visualizations & Spectrum Analysis | Planned |

## Contributing

Community contributions are accepted. Pull requests or issue reports can be submitted via the GitHub repository.

## License

[MIT](LICENSE) © SiputBiru

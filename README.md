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

## Install from Source

```bash
git clone https://github.com/SiputBiru/eqtui
cd eqtui
cargo build --release
```

Needs PipeWire and a Nerd Font.

---

[Project by SiputBiru](LICENSE) — patches welcome but no promises :^)

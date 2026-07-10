// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

fn main() -> eqtui::AppResult<()> {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map_or("attach", std::string::String::as_str);

    eqtui::logging::init(mode)?;
    color_eyre::install()?;

    eqtui::cli::dispatch(mode, &args)
}

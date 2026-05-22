// Copyright (C) 2026 SiputBiru <hillsforrest03@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style, Stylize},
    symbols,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType},
};

use crate::app::App;
pub fn render(_app: &App, frame: &mut Frame, area: Rect) {
    // PEQdB Diamond β target dataset (transformed to log10 space)
    let target_raw: &[(f64, f64)] = &[
        (20.00, 82.981),
        (22.29, 82.935),
        (25.28, 82.865),
        (28.69, 82.772),
        (32.55, 82.647),
        (36.93, 82.479),
        (41.90, 82.255),
        (47.53, 81.957),
        (53.93, 81.565),
        (61.19, 81.060),
        (69.42, 80.429),
        (78.76, 79.669),
        (89.36, 78.795),
        (101.39, 77.848),
        (115.03, 76.927),
        (130.51, 76.015),
        (148.07, 75.204),
        (168.00, 74.535),
        (190.61, 74.019),
        (216.26, 73.625),
        (245.36, 73.350),
        (278.38, 73.191),
        (315.84, 73.081),
        (358.34, 72.984),
        (406.56, 73.009),
        (461.27, 73.305),
        (523.34, 73.628),
        (593.77, 73.913),
        (673.67, 74.221),
        (764.32, 74.524),
        (867.17, 74.721),
        (983.87, 74.952),
        (1116.26, 75.471),
        (1266.47, 76.197),
        (1436.90, 76.992),
        (1630.26, 77.960),
        (1849.64, 79.336),
        (2098.54, 80.820),
        (2380.94, 82.454),
        (2701.33, 83.629),
        (3064.85, 84.087),
        (3477.27, 84.041),
        (3945.20, 83.612),
        (4476.10, 83.046),
        (5078.43, 82.674),
        (5761.82, 82.630),
        (6537.18, 82.741),
        (7416.87, 82.911),
        (8414.94, 82.514),
        (9547.31, 81.399),
        (10832.07, 80.067),
        (12289.71, 78.464),
        (13943.51, 76.771),
        (15819.85, 75.144),
        (17948.68, 73.499),
        (20000.00, 72.027),
    ];

    let target_data: Vec<(f64, f64)> = target_raw.iter().map(|(f, g)| (f.log10(), *g)).collect();

    let datasets = vec![
        Dataset::default()
            .name("PEQdB Diamond β Target")
            // .marker(symbols::Marker::Braille)
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().magenta().bold())
            .data(&target_data),
    ];

    let x_axis = Axis::default()
        .title("Frequency (Hz)".white())
        .style(Style::default().gray())
        // log10(20) approx 1.30, log10(20000) approx 4.30
        .bounds([20.0f64.log10(), 20000.0f64.log10()])
        .labels(["20", "100", "1k", "10k", "20k"]);

    let y_axis = Axis::default()
        .title("Gain (dB)".white())
        .style(Style::default().gray())
        .bounds([60.0, 100.0])
        .labels(["60", "80", "100"]);
    // .bounds([70.0, 90.0])
    // .labels(["70", "80", "90"]);

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .title(" Frequency Response [WIP] - Placeholder")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .x_axis(x_axis)
        .y_axis(y_axis);

    frame.render_widget(chart, area);
}

// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{self, LineGauge, Paragraph};

use crate::app::{App, DaemonConnection, Mode};

pub fn render_monitoring(app: &App, frame: &mut Frame, area: Rect) {
    let block = widgets::Block::default()
        .title(" Monitoring ")
        .borders(widgets::Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [text_area, meters_area] =
        Layout::vertical([Constraint::Length(4), Constraint::Fill(1)]).areas(inner);

    // Text Stats
    let pw_status = if app.pw_connected {
        "Connected".green()
    } else {
        "Disconnected".red()
    };
    let state_color = match app.filter_state {
        crate::state::FilterState::Streaming => Color::Green,
        crate::state::FilterState::Connecting => Color::Yellow,
        crate::state::FilterState::Error(_) => Color::Red,
        _ => Color::DarkGray,
    };

    let stats = vec![
        Line::from(vec![Span::raw("Core: "), pw_status]),
        Line::from(vec![
            Span::raw("Daemon: "),
            match app.daemon {
                DaemonConnection::Connected => {
                    Span::styled("Connected", Style::default().fg(Color::Green))
                }
                DaemonConnection::Reconnecting => {
                    Span::styled("Reconnecting...", Style::default().fg(Color::Yellow).bold())
                }
                DaemonConnection::Disconnected => {
                    Span::styled("Disconnected", Style::default().fg(Color::Red).bold())
                }
            },
        ]),
        Line::from(vec![
            Span::raw("Source: "),
            if !app.null_sink_source_known {
                Span::styled("?", Style::default().fg(Color::Yellow))
            } else if app.null_sink.has_source() {
                Span::styled("active", Style::default().fg(Color::Green))
            } else {
                Span::raw("---").dark_gray()
            },
        ]),
        Line::from(vec![
            Span::raw("State: "),
            match &app.filter_state {
                crate::state::FilterState::Error(_) => Span::styled(
                    "ERROR — PipeWire disconnected, restart daemon",
                    Style::default().fg(Color::Red).bold(),
                ),
                other => Span::styled(other.to_string(), Style::default().fg(state_color).bold()),
            },
        ]),
        Line::from(vec![
            Span::raw("Outputs: "),
            if app.connected_devices.is_empty() {
                Span::raw("none").dark_gray()
            } else {
                Span::styled(
                    format!("{}", app.connected_devices.len()),
                    Style::default().fg(Color::Cyan),
                )
            },
        ]),
        Line::from(vec![
            Span::raw("Null Sink: "),
            if app.null_sink.is_loaded() {
                Span::styled(
                    format!("Loaded (ID {})", app.null_sink.module_id().unwrap_or(0)),
                    Style::default().fg(Color::Cyan),
                )
            } else if app.null_sink_missing {
                Span::styled(
                    "FAILED — no audio source",
                    Style::default().fg(Color::Red).bold(),
                )
            } else {
                Span::raw("Not loaded").dark_gray()
            },
        ]),
    ];
    frame.render_widget(Paragraph::new(stats), text_area);

    // Meters (Vertical stack for sidebar)
    let [preamp_area, l_area, r_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(meters_area);

    let preamp_line = Line::from(vec![
        Span::raw("Preamp: "),
        Span::styled(
            format!("{:.1} dB", app.preamp),
            Style::default().fg(Color::Cyan),
        ),
    ]);
    frame.render_widget(Paragraph::new(preamp_line), preamp_area);

    let ratio_l = f64::from(((app.peak_l - (-60.0)) / 60.0).clamp(0.0, 1.0));
    let ratio_r = f64::from(((app.peak_r - (-60.0)) / 60.0).clamp(0.0, 1.0));

    let gauge_l = LineGauge::default()
        .filled_style(Style::default().fg(Color::Green))
        .unfilled_style(Style::default().fg(Color::DarkGray))
        .label(format!("L {:.1}dB", app.peak_l))
        .ratio(ratio_l)
        .filled_symbol(symbols::line::THICK_HORIZONTAL)
        .unfilled_symbol(symbols::line::THICK_HORIZONTAL);
    frame.render_widget(gauge_l, l_area);

    let gauge_r = LineGauge::default()
        .filled_style(Style::default().fg(Color::Green))
        .unfilled_style(Style::default().fg(Color::DarkGray))
        .label(format!("R {:.1}dB", app.peak_r))
        .ratio(ratio_r)
        .filled_symbol(symbols::line::THICK_HORIZONTAL)
        .unfilled_symbol(symbols::line::THICK_HORIZONTAL);
    frame.render_widget(gauge_r, r_area);
}

pub fn render_hints(app: &App, frame: &mut Frame, area: Rect) {
    let mut spans = vec![];

    // Mode-specific hints
    match app.mode {
        Mode::Normal => {
            spans.extend(vec![
                Span::from("j/k, ↕").bold(),
                Span::from(" Row | "),
                Span::from("h/l, ↔").bold(),
                Span::from(" Col | "),
                Span::from("+/-").bold(),
                Span::from(" Bump | "),
                Span::from("i").bold(),
                Span::from(" Edit | "),
                Span::from("a").bold(),
                Span::from(" New | "),
                Span::from("dd").bold(),
                Span::from(" Del | "),
                Span::from("C").bold(),
                Span::from(" Conn | "),
                Span::from("b").bold(),
                Span::from(" Bypass | "),
                Span::from("{}").bold(),
                Span::from(" Prof | "),
                Span::from("r").bold(),
                Span::from(" Rst | "),
                Span::from(":").bold(),
                Span::from(" Cmd | "),
                Span::from("v").bold(),
                Span::from(" Vis | "),
                Span::from("Tab").bold(),
                Span::from(" Focus | "),
                Span::from("q").bold(),
                Span::from(" Quit"),
            ]);
        }
        Mode::Insert => {
            spans.extend(vec![
                Span::from("Type").bold(),
                Span::from(" Value | "),
                Span::from("Enter").bold(),
                Span::from(" Save | "),
                Span::from("Esc").bold(),
                Span::from(" Cancel"),
            ]);
        }
        Mode::Command => {
            let value = app.command_input.value();
            let cursor = app.command_input.cursor();

            spans.push(Span::from(":").bold());

            // Text before cursor
            let before: String = value.chars().take(cursor).collect();
            if !before.is_empty() {
                spans.push(Span::styled(before, Style::default().fg(Color::Yellow)));
            }

            // Character at cursor (to apply blink)
            let char_at_cursor = value.chars().nth(cursor).unwrap_or(' ');
            spans.push(Span::styled(
                char_at_cursor.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::REVERSED)
                    .add_modifier(Modifier::SLOW_BLINK),
            ));

            // Text after cursor
            let after: String = value.chars().skip(cursor + 1).collect();
            if !after.is_empty() {
                spans.push(Span::styled(after, Style::default().fg(Color::Yellow)));
            }
        }
        Mode::Visual => {}
    }

    let p = Paragraph::new(Line::from(spans)).style(Style::default().fg(Color::Gray));
    frame.render_widget(p, area);
}

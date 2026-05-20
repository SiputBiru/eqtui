use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{self, LineGauge, Paragraph};

use crate::app::{App, Mode};

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
    let state_color = match app.filter_state.as_str() {
        "STREAMING" => Color::Green,
        "CONNECTING" => Color::Yellow,
        "ERROR" => Color::Red,
        _ => Color::DarkGray,
    };

    let stats = vec![
        Line::from(vec![Span::raw("Core: "), pw_status]),
        Line::from(vec![
            Span::raw("Source: "),
            if app.null_sink_has_source {
                Span::styled("active", Style::default().fg(Color::Green))
            } else {
                Span::raw("---").dark_gray()
            },
        ]),
        Line::from(vec![
            Span::raw("State: "),
            Span::styled(&app.filter_state, Style::default().fg(state_color).bold()),
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
            if app.null_sink_loaded {
                Span::styled(
                    format!("Loaded (ID {})", app.null_sink_module_id.unwrap_or(0)),
                    Style::default().fg(Color::Cyan),
                )
            } else {
                Span::raw("Not loaded").dark_gray()
            },
        ]),
    ];
    frame.render_widget(Paragraph::new(stats), text_area);

    // Meters (Vertical stack for sidebar)
    let [l_area, r_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(meters_area);

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
                Span::from(" Add | "),
                Span::from("dd").bold(),
                Span::from(" Del | "),
                Span::from("C").bold(),
                Span::from(" Connect | "),
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
            spans.extend(vec![
                Span::from(":").bold(),
                Span::styled(&app.command_input, Style::default().fg(Color::Yellow)),
            ]);
        }
        Mode::Visual => {}
    }

    let p = Paragraph::new(Line::from(spans)).style(Style::default().fg(Color::Gray));
    frame.render_widget(p, area);
}

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{self, LineGauge, Paragraph};
use ratatui::Frame;

use crate::app::{App, Mode};

pub fn render_monitoring(app: &App, frame: &mut Frame, area: Rect) {
    let block = widgets::Block::default()
        .title(" Monitoring ")
        .borders(widgets::Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [text_area, meters_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Fill(1),
    ]).areas(inner);

    // 1. Text Stats
    let pw_status = if app.pw_connected { "Connected".green() } else { "Disconnected".red() };
    let state_color = match app.filter_state.as_str() {
        "STREAMING" => Color::Green,
        "CONNECTING" => Color::Yellow,
        "ERROR" => Color::Red,
        _ => Color::DarkGray,
    };
    
    let stats = vec![
        Line::from(vec![Span::raw("PW: "), pw_status]),
        Line::from(vec![Span::raw("State: "), Span::styled(&app.filter_state, Style::default().fg(state_color).bold())]),
        Line::from(vec![Span::raw("Output: "), if let Some(id) = app.bound_output_id { Span::styled(format!("ID {}", id), Style::default().fg(Color::Cyan)) } else { Span::raw("default").dark_gray() }]),
    ];
    frame.render_widget(Paragraph::new(stats), text_area);

    // 2. Meters (Vertical stack for sidebar)
    let [l_area, r_area] = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(meters_area);
    
    let ratio_l = ((app.peak_l - (-60.0)) / 60.0).clamp(0.0, 1.0) as f64;
    let ratio_r = ((app.peak_r - (-60.0)) / 60.0).clamp(0.0, 1.0) as f64;

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
                Span::from("j/k, ↕").bold(), Span::from(" Row | "),
                Span::from("h/l, ↔").bold(), Span::from(" Col | "),
                Span::from("+/-").bold(), Span::from(" Bump | "),
                Span::from("i").bold(), Span::from(" Edit | "),
                Span::from("a").bold(), Span::from(" Add | "),
                Span::from("dd").bold(), Span::from(" Del | "),
                Span::from("Tab").bold(), Span::from(" Focus | "),
                Span::from("q").bold(), Span::from(" Quit"),
            ]);
        }
        Mode::Insert => {
            spans.extend(vec![
                Span::from("Type").bold(), Span::from(" Value | "),
                Span::from("Enter").bold(), Span::from(" Save | "),
                Span::from("Esc").bold(), Span::from(" Cancel"),
            ]);
        }
        Mode::Command => {
            spans.extend(vec![
                Span::from(":").bold(),
                Span::styled(&app.command_input, Style::default().fg(Color::Yellow)),
            ]);
        }
        _ => {}
    };

    let p = Paragraph::new(Line::from(spans)).style(Style::default().fg(Color::Gray));
    frame.render_widget(p, area);
}

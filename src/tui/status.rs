use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{LineGauge, Paragraph};
use ratatui::Frame;

use crate::app::{App, Mode};

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let [text_area, meters_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
    ]).areas(area);

    let mut spans = vec![];

    // Connection status
    let pw_status = if app.pw_connected { "Connected" } else { "Disconnected" };
    spans.push(Span::styled(
        format!("PW: {pw_status}"),
        if app.pw_connected {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::Red)
        },
    ));
    spans.push(Span::raw(" | "));

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

    let p = Paragraph::new(Line::from(spans))
        .centered()
        .style(Style::default().fg(Color::Gray));
    frame.render_widget(p, text_area);

    // --- LineGauge Meters ---
    let [_, meter_l_area, _, meter_r_area, _] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(20),
        Constraint::Length(4),
        Constraint::Length(20),
        Constraint::Fill(1),
    ]).areas(meters_area);

    let ratio_l = ((app.peak_l - (-60.0)) / 60.0).clamp(0.0, 1.0) as f64;
    let ratio_r = ((app.peak_r - (-60.0)) / 60.0).clamp(0.0, 1.0) as f64;

    let gauge_l = LineGauge::default()
        .filled_style(Style::default().fg(Color::Green))
        .unfilled_style(Style::default().fg(Color::DarkGray))
        .label(format!("L {:.1}dB", app.peak_l))
        .ratio(ratio_l)
        .filled_symbol(symbols::line::THICK_HORIZONTAL)
        .unfilled_symbol(symbols::line::THICK_HORIZONTAL);
    frame.render_widget(gauge_l, meter_l_area);

    let gauge_r = LineGauge::default()
        .filled_style(Style::default().fg(Color::Green))
        .unfilled_style(Style::default().fg(Color::DarkGray))
        .label(format!("R {:.1}dB", app.peak_r))
        .ratio(ratio_r)
        .filled_symbol(symbols::line::THICK_HORIZONTAL)
        .unfilled_symbol(symbols::line::THICK_HORIZONTAL);
    frame.render_widget(gauge_r, meter_r_area);
}

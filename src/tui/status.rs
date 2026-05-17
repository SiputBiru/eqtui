use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, Mode};

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let mut spans = vec![];

    // Connection status
    let pw_status = if app.pw_connected { "Connected" } else { "Disconnected" };
    spans.push(Span::styled(format!("PW: {pw_status}"), if app.pw_connected { Style::default().fg(Color::Green) } else { Style::default().fg(Color::Red) }));
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
    
    frame.render_widget(p, area);
}

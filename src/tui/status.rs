use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, Mode};

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let pw_status = if app.pw_connected {
        Span::styled("connected", Style::default().fg(Color::Green))
    } else {
        Span::styled("disconnected", Style::default().fg(Color::Red))
    };

    let mode = match app.mode {
        Mode::Normal => Span::styled("NORMAL", Style::default().fg(Color::Yellow)),
        Mode::Insert => Span::styled("INSERT", Style::default().fg(Color::Cyan)),
        Mode::Visual => Span::styled("VISUAL", Style::default().fg(Color::Magenta)),
        Mode::Command => Span::styled("COMMAND", Style::default().fg(Color::Cyan)),
    };

    let mut spans = vec![
        Span::raw(" PW: "),
        pw_status,
        Span::raw("  |  "),
        Span::raw(format!("nodes: {}  |  ", app.nodes.len())),
        mode,
        Span::raw("  |  q:quit"),
    ];

    if app.mode == Mode::Command {
        spans.push(Span::raw("  |  :"));
        spans.push(Span::styled(
            &app.command_input,
            Style::default().fg(Color::Yellow),
        ));
    }

    let p = Paragraph::new(Line::from(spans))
        .style(Style::default().fg(Color::Gray).bg(Color::DarkGray));
    frame.render_widget(p, area);
}

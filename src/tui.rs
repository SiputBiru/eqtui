use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::state::AppState;

pub fn render(frame: &mut Frame, state: &AppState) {
    let area = frame.area();

    // Split into main area + status bar
    let [main_area, status_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);

    let [list_area, info_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(3)]).areas(main_area);

    // ---------- node list ----------
    let items: Vec<ListItem> = state
        .nodes
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let icon = if node.class == "Audio/Sink" {
                "\u{1f50a} " // 🔊 speaker
            } else {
                "\u{1f399}  " // 🎙 mic
            };
            let label = format!("{icon}{node}");
            if i == state.selected {
                ListItem::new(label).style(Style::default().fg(Color::Yellow))
            } else {
                ListItem::new(label)
            }
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(" Devices ")
            .borders(Borders::ALL),
    );

    frame.render_widget(list, list_area);

    // ---------- info panel ----------
    let info = if let Some(node) = state.nodes.get(state.selected) {
        vec![
            format!("  id:      {}", node.id),
            format!("  name:    {}", node.name),
            format!("  class:   {}", node.class_label()),
        ]
    } else {
        vec!["(no devices)".into()]
    };

    let info_para = Paragraph::new(info.join("\n"))
        .block(Block::default().title(" Details ").borders(Borders::ALL));
    frame.render_widget(info_para, info_area);

    // ---------- status bar ----------
    let pw_status = if state.pw_connected {
        "connected"
    } else {
        "disconnected"
    };
    let status_text = format!(
        " PW: {}  |  {}  |  q:quit  ↑↓:navigate ",
        pw_status, state.status
    );
    let status = Paragraph::new(Line::from(status_text))
        .style(Style::default().fg(Color::Gray).bg(Color::DarkGray));
    frame.render_widget(status, status_area);
}

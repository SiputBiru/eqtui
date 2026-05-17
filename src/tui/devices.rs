use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::App;

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let [list_area, info_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(4)]).areas(area);

    let items: Vec<ListItem> = app
        .nodes
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let icon = if node.class == "Audio/Sink" {
                "\u{1f50a} "
            } else {
                "\u{1f399}  "
            };
            let label = format!("{icon}{node}");
            if i == app.nodes_selected {
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

    let info = if let Some(node) = app.nodes.get(app.nodes_selected) {
        vec![
            format!("  id:      {}", node.id),
            format!("  name:    {}", node.name),
            format!("  class:   {}", node.class_label()),
        ]
    } else {
        vec!["(no devices)".into()]
    };

    let p = Paragraph::new(info.join("\n"))
        .block(Block::default().title(" Info ").borders(Borders::ALL));
    frame.render_widget(p, info_area);
}

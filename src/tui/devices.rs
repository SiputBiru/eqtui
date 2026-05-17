use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Row, Table, TableState};

use crate::app::{App, FocusedBlock};

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let is_focused = app.focused_block == FocusedBlock::Devices;

    let header = Row::new(["Class", "Name", "ID"])
        .style(if is_focused {
            Style::default().fg(Color::Yellow).bold()
        } else {
            Style::default()
        })
        .bottom_margin(1);

    let rows: Vec<Row> = app
        .nodes
        .iter()
        .map(|node| {
            let icon = node.class.icon();
            Row::new(vec![
                Cell::new(icon),
                Cell::new(node.to_string()),
                Cell::new(node.id.to_string()),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(5),
        Constraint::Fill(1),
        Constraint::Length(8),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .title(" Devices ")
                .title_style(if is_focused {
                    Style::default().bold()
                } else {
                    Style::default()
                })
                .borders(Borders::ALL)
                .border_type(if is_focused {
                    BorderType::Thick
                } else {
                    BorderType::Plain
                })
                .border_style(if is_focused {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default()
                }),
        )
        .row_highlight_style(if is_focused {
            Style::default().fg(Color::White).bg(Color::DarkGray)
        } else {
            Style::default()
        });

    let mut state = TableState::default().with_selected(app.nodes_selected);
    frame.render_stateful_widget(table, area, &mut state);
}

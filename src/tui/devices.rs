// Copyright (C) 2026 SiputBiru <hillsforrest03@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Row, Table, TableState};

use crate::app::{App, FocusedBlock};

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let is_focused = app.focused_block == FocusedBlock::Devices;

    let header = Row::new(["Class", "Name", "ID", "Conn"])
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
            let is_eqtui = node.description.contains("eqtui");
            let icon = node.class.icon();

            let mut name = node.to_string();
            if is_eqtui {
                name.push_str(" [this app]");
            }

            let name_cell = if is_eqtui {
                Cell::new(name).style(
                    Style::default()
                        .add_modifier(Modifier::BOLD)
                        .fg(Color::Cyan),
                )
            } else if app.is_device_connected(node.id) {
                Cell::new(format!(" {name}")).style(Style::default().fg(Color::Green))
            } else {
                Cell::new(name)
            };

            Row::new(vec![
                Cell::new(format!("  {} ", icon)),
                name_cell,
                Cell::new(node.id.to_string()),
                if app.is_device_connected(node.id) {
                    Cell::new("C").style(Style::default().fg(Color::Green))
                } else {
                    Cell::new("-").dark_gray()
                },
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(5),
        Constraint::Fill(1),
        Constraint::Length(8),
        Constraint::Length(5),
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

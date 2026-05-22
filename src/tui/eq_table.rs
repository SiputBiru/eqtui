use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table, TableState};

use crate::app::{App, FocusedBlock, Mode};
use crate::state::FilterType;

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let is_focused = matches!(app.focused_block, FocusedBlock::Pipeline);

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .split(area);

    render_tabs(app, frame, chunks[0], is_focused);
    render_table(app, frame, chunks[1], is_focused);
}

fn render_tabs(app: &App, frame: &mut Frame, area: Rect, focused: bool) {
    let spans: Vec<Span> = app
        .profiles
        .iter()
        .enumerate()
        .flat_map(|(i, p)| {
            // Append [RO] if the profile has an external path (read-only)
            let name = if p.path.is_some() {
                format!(" {} [RO] ", p.name)
            } else {
                format!(" {} ", p.name)
            };
            let style = if i == app.active_profile {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else if focused {
                Style::default().fg(Color::Gray)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            vec![Span::styled(name, style)]
        })
        .collect();

    let p = Paragraph::new(Line::from(spans));
    frame.render_widget(p, area);
}

fn render_table(app: &App, frame: &mut Frame, area: Rect, is_focused: bool) {
    let bands = &app.eq.bands;
    let selected = app.eq.band_selected;

    let header = Row::new(["#", "Frequency", "Gain", "Q", "Type"])
        .style(if is_focused {
            Style::default().fg(Color::Yellow).bold()
        } else {
            Style::default().fg(Color::Yellow)
        })
        .bottom_margin(1); // Add breathing room

    let rows: Vec<Row> = bands
        .iter()
        .enumerate()
        .map(|(i, band)| {
            let freq = if band.frequency >= 1000.0 {
                format!("{:.1}kHz", band.frequency / 1000.0)
            } else {
                format!("{:.0}Hz", band.frequency)
            };
            let gain = format!("{:+0.1}dB", band.gain);
            let q = format!("{:.2}", band.q);
            let ftype = match band.filter_type {
                FilterType::Peak => "PK",
                FilterType::LowShelf => "LS",
                FilterType::HighShelf => "HS",
            };

            let is_editing = is_focused && app.mode == Mode::Insert && i == selected;

            let mut cells = vec![
                Cell::new(format!("{}", i + 1)),
                Cell::new(if is_editing && app.eq.column_selected == 1 {
                    format!("{}█", app.eq.cell_input.value())
                } else {
                    freq
                }),
                Cell::new(if is_editing && app.eq.column_selected == 2 {
                    format!("{}█", app.eq.cell_input.value())
                } else {
                    gain
                }),
                Cell::new(if is_editing && app.eq.column_selected == 3 {
                    format!("{}█", app.eq.cell_input.value())
                } else {
                    q
                }),
                Cell::new(if is_editing && app.eq.column_selected == 4 {
                    format!("{}█", app.eq.cell_input.value())
                } else {
                    ftype.to_string()
                }),
            ];

            if i == selected && is_focused {
                let col = app.eq.column_selected;
                if col < cells.len() {
                    let style = if app.mode == Mode::Insert {
                        Style::default().fg(Color::White).bg(Color::Cyan)
                    } else {
                        Style::default().fg(Color::White).bg(Color::DarkGray)
                    };
                    cells[col] = cells[col].clone().style(style);
                }
            }

            Row::new(cells)
        })
        .collect();

    let widths = [
        Constraint::Length(4),
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Length(6),
    ];

    let mut block = Block::default()
        .title(" Equalizer ")
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
        });

    if let Some((msg, _)) = &app.notification {
        block = block.title_bottom(Line::from(Span::styled(
            format!(" {} ", msg),
            Style::default().fg(Color::Green),
        )));
    }

    let table = Table::new(rows, widths).header(header).block(block);

    let mut state = TableState::default().with_selected(selected);
    frame.render_stateful_widget(table, area, &mut state);
}

use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Cell, Row, Table, TableState};
use ratatui::Frame;

use crate::app::{App, FocusedBlock, Mode};
use crate::state::FilterType;

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let bands = &app.eq_bands;
    let selected = app.eq_band_selected;

    let header = Row::new(["#", "Frequency", "Gain", "Q", "Type"])
        .style(Style::default().fg(Color::Yellow));

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
            Row::new(vec![
                Cell::new(format!("{}", i + 1)),
                Cell::new(freq),
                Cell::new(gain),
                Cell::new(q),
                Cell::new(ftype),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(4),
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Length(6),
    ];

    let is_focused = matches!(app.focused_block, FocusedBlock::Pipeline);
    let is_normal = app.mode == Mode::Normal;

    let highlight = if is_focused && is_normal {
        Style::default().fg(Color::Black).bg(Color::Yellow)
    } else {
        Style::default()
    };

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .title(" Equalizer ")
                .borders(Borders::ALL),
        )
        .row_highlight_style(highlight);

    let mut state = TableState::default().with_selected(selected);
    frame.render_stateful_widget(table, area, &mut state);
}

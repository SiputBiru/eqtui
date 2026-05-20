use std::io;
use std::panic;

use crossterm::cursor;
use crossterm::event::DisableMouseCapture;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::{Constraint, Layout};

use crate::AppResult;
use crate::app::App;
use crate::event::EventHandler;

pub mod devices;
pub mod eq_table;
pub mod graph;
pub mod status;

pub struct Tui<B: Backend> {
    terminal: Terminal<B>,
    pub events: EventHandler,
}

impl<B: Backend> Tui<B>
where
    <B as Backend>::Error: Send + Sync + 'static,
{
    pub fn new(terminal: Terminal<B>, events: EventHandler) -> Self {
        Self { terminal, events }
    }

    pub fn init(&mut self) -> AppResult<()> {
        terminal::enable_raw_mode()?;
        crossterm::execute!(io::stdout(), EnterAlternateScreen, cursor::Hide)?;

        let panic_hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            Self::reset().ok();
            panic_hook(info);
        }));

        self.terminal.clear()?;
        Ok(())
    }

    pub fn draw<F>(&mut self, render: F) -> AppResult<()>
    where
        F: FnOnce(&mut ratatui::Frame),
    {
        self.terminal.draw(render)?;
        Ok(())
    }

    fn reset() -> AppResult<()> {
        terminal::disable_raw_mode()?;
        crossterm::execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
        Ok(())
    }

    pub fn exit(&mut self) -> AppResult<()> {
        Self::reset()?;
        self.terminal.show_cursor()?;
        Ok(())
    }
}

pub fn render(app: &App, frame: &mut ratatui::Frame) {
    let area = frame.area();

    let [main_content, hint_area] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)])
        .margin(1)
        .areas(area);

    let [sidebar_area, main_view_area] =
        Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)])
            .areas(main_content);

    let [devices_area, bands_area, monitoring_area] = Layout::vertical([
        Constraint::Fill(2),
        Constraint::Fill(3),
        Constraint::Length(8),
    ])
    .areas(sidebar_area);

    devices::render(app, frame, devices_area);
    eq_table::render(app, frame, bands_area);
    status::render_monitoring(app, frame, monitoring_area);
    status::render_hints(app, frame, hint_area);

    graph::render(app, frame, main_view_area);
}

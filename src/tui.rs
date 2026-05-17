use std::io;
use std::panic;

use crossterm::cursor;
use crossterm::event::DisableMouseCapture;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::Backend;
use ratatui::Terminal;

use crate::event::EventHandler;
use crate::AppResult;

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

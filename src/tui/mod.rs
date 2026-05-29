// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

use std::io;
use std::panic;
use std::time;
use std::time::Duration;

use crossterm::cursor;
use crossterm::event::DisableMouseCapture;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::{Constraint, Layout};

use crate::AppResult;
use crate::app::{App, DaemonConnection};
use crate::client::DaemonClient;
use crate::event;
use crate::event::EventHandler;
use crate::handler;

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
        Constraint::Length(9),
    ])
    .areas(sidebar_area);

    devices::render(app, frame, devices_area);
    eq_table::render(app, frame, bands_area);
    status::render_monitoring(app, frame, monitoring_area);
    status::render_hints(app, frame, hint_area);

    graph::render(app, frame, main_view_area);
}

/// Attach the TUI to the daemon and enter the main event loop
pub fn attach() -> AppResult<()> {
    tracing::info!("Connecting to daemon...");
    let client = DaemonClient::connect()?;

    let mut app = App::new(client);

    if let Err(e) = app.full_sync() {
        tracing::warn!(%e, "Initial full_sync failed - starting with defaults");
    }

    let backend = ratatui::backend::CrosstermBackend::new(io::stdout());

    let terminal = ratatui::Terminal::new(backend)?;
    let events = EventHandler::new();
    let mut tui = Tui::new(terminal, events);
    tui.init()?;

    tracing::info!("Entering TUI main loop");

    while app.running {
        if let Err(e) = app.drain_events() {
            tracing::warn!(%e, "Daemon connection lost - reconnecting with backoff");

            app.notify("Daemon disconnected - reconnecting...");
            app.daemon = DaemonConnection::Reconnecting;

            tui.draw(|frame| render(&app, frame))?;

            let mut delay = Duration::from_secs(1);
            let max_delay = Duration::from_secs(8);
            let max_total = Duration::from_secs(30);
            let start = time::Instant::now();

            let reconnected = loop {
                match app.reconnect() {
                    Ok(()) => break true,
                    Err(e) => {
                        if start.elapsed() >= max_total {
                            tracing::error!(%e, "Reconnect failed after 30s");
                            app.notify(format!("Reconnect failed: {e}"));
                            break false;
                        }
                        tracing::warn!(%e, "Reconnect failed — retrying in {delay:?}");
                        app.notify(format!("Reconnecting in {}s...", delay.as_secs()));
                        tui.draw(|frame| render(&app, frame))?;
                        std::thread::sleep(delay);
                        delay = (delay * 2).min(max_delay);
                    }
                }
            };

            if reconnected {
                app.daemon = DaemonConnection::Connected;
                tracing::info!("Reconnected to daemon");
                app.notify("Reconnected - bands and preamp restored");
            } else {
                app.daemon = DaemonConnection::Disconnected;
                app.notify("Connection lost - exiting");
                tui.draw(|frame| render(&app, frame))?;
                break;
            }
        }

        match tui.events.next()? {
            event::Event::Tick => app.tick(),
            event::Event::Key(key) => handler::dispatch(key, &mut app),
            event::Event::Resize(_, _) => {}
        }

        tui.draw(|frame| render(&app, frame))?;
    }

    tui.exit()?;
    tracing::info!("TUI exited - daemon keeps running");
    Ok(())
}

// Copyright (C) 2026 SiputBiru <hillsforrest03@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

#![expect(
    dead_code,
    reason = "scaffolded code for future features (Resize event, etc.)"
)]

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::AppResult;
use crossterm::event::{self, Event as CrosstermEvent, KeyEventKind};

const TICK_FPS: f64 = 30.0;
// const TICK_FPS: f64 = 90.0;

#[derive(Clone, Debug)]
pub enum Event {
    Tick,
    Key(crossterm::event::KeyEvent),
    Resize(u16, u16),
}

#[derive(Debug)]
pub struct EventHandler {
    sender: mpsc::Sender<Event>,
    receiver: mpsc::Receiver<Event>,
}

impl EventHandler {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        let actor = EventThread::new(sender.clone());
        thread::spawn(|| actor.run());
        Self { sender, receiver }
    }

    pub fn next(&self) -> AppResult<Event> {
        Ok(self.receiver.recv()?)
    }
}

impl Default for EventHandler {
    fn default() -> Self {
        Self::new()
    }
}

struct EventThread {
    sender: mpsc::Sender<Event>,
}

impl EventThread {
    fn new(sender: mpsc::Sender<Event>) -> Self {
        Self { sender }
    }

    fn run(self) {
        let tick_interval = Duration::from_secs_f64(1.0 / TICK_FPS);
        let mut last_tick = Instant::now();

        loop {
            let timeout = tick_interval.saturating_sub(last_tick.elapsed());
            if timeout == Duration::ZERO {
                last_tick = Instant::now();
                if self.sender.send(Event::Tick).is_err() {
                    break;
                }
            }

            if event::poll(timeout).unwrap_or(false) {
                let Ok(event) = event::read() else { continue };
                match event {
                    CrosstermEvent::Key(key) if key.kind == KeyEventKind::Press => {
                        if self.sender.send(Event::Key(key)).is_err() {
                            break;
                        }
                    }
                    CrosstermEvent::Resize(w, h) => {
                        if self.sender.send(Event::Resize(w, h)).is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

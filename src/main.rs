use std::sync::mpsc;
use std::sync::Arc;

use eqtui::{
    app::App,
    config::Config,
    event::EventHandler,
    handler,
    state::{PwCommand, PwEvent},
    tui::Tui,
    AppResult,
};
use ratatui::backend::CrosstermBackend;

fn main() -> AppResult<()> {
    color_eyre::install()?;

    let config = Arc::new(Config::new(None));

    let (to_tui, from_pw) = mpsc::channel::<PwEvent>();
    let (to_pw, from_tui) = pipewire::channel::channel::<PwCommand>();

    let pw_handle = std::thread::spawn(move || {
        eqtui::pw::run(to_tui, from_tui);
    });

    let backend = CrosstermBackend::new(std::io::stdout());
    let terminal = ratatui::Terminal::new(backend)?;
    let events = EventHandler::new();
    let mut tui = Tui::new(terminal, events);

    tui.init()?;

    let mut app = App::new(config);

    while app.running {
        while let Ok(event) = from_pw.try_recv() {
            app.handle_pw_event(event);
        }

        match tui.events.next()? {
            eqtui::event::Event::Tick => app.tick(),
            eqtui::event::Event::Key(key) => handler::dispatch(key, &mut app),
            eqtui::event::Event::Resize(_, _) => {}
        }

        tui.draw(|frame| render(frame, &app))?;
    }

    tui.exit()?;

    let _ = to_pw.send(PwCommand::Terminate);
    pw_handle.join().ok();

    Ok(())
}

fn render(frame: &mut ratatui::Frame, app: &App) {
    use ratatui::layout::{Constraint, Layout};
    use ratatui::style::{Color, Style};
    use ratatui::text::Line;
    use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

    let area = frame.area();

    let [main_area, status_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);

    let [list_area, info_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(3)]).areas(main_area);

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

    let list = List::new(items).block(Block::default().title(" Devices ").borders(Borders::ALL));
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

    let info_para = Paragraph::new(info.join("\n"))
        .block(Block::default().title(" Details ").borders(Borders::ALL));
    frame.render_widget(info_para, info_area);

    let pw_status = if app.pw_connected {
        "connected"
    } else {
        "disconnected"
    };
    let nodes_count = app.nodes.len();
    let mode_label = match app.mode {
        eqtui::app::Mode::Normal => "NORMAL",
        eqtui::app::Mode::Insert => "INSERT",
        eqtui::app::Mode::Visual => "VISUAL",
        eqtui::app::Mode::Command => "COMMAND",
    };
    let status_text = format!(
        " PW: {}  |  nodes: {}  |  {}  |  q:quit",
        pw_status, nodes_count, mode_label
    );
    let status = Paragraph::new(Line::from(status_text))
        .style(Style::default().fg(Color::Gray).bg(Color::DarkGray));
    frame.render_widget(status, status_area);
}

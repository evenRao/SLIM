//! Terminal initialization, cleanup, and main rendering loop.

use crate::app::App;
use crate::event::handle_key_event;
use crate::ui;
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use rusqlite::Connection;
use slim_core::model::actor::Actor;
use std::io::stdout;
use std::time::{Duration, Instant};

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            std::io::stdout(),
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
    }
}

/// Run the interactive SLIM Terminal User Interface.
pub fn run(conn: &Connection, actor: Actor) -> anyhow::Result<()> {
    run_with_login(conn, Some(actor))
}

pub fn run_with_login(conn: &Connection, actor: Option<Actor>) -> anyhow::Result<()> {
    let mut app = match actor {
        Some(actor) => App::new(conn, actor)?,
        None => App::new_locked(conn)?,
    };
    enable_raw_mode()?;
    let _guard = TerminalGuard;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Set panic hook to ensure terminal state is always restored
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            std::io::stdout(),
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        original_hook(panic_info);
    }));

    let res = run_loop(&mut terminal, &mut app);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    res
}

fn run_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> anyhow::Result<()> {
    let mut refreshed = Instant::now();
    while !app.should_quit {
        if refreshed.elapsed() >= Duration::from_secs(1) {
            if let Err(error) = app.revalidate_actor() {
                app.authenticated = false;
                app.open_user_modal();
                app.set_notification(crate::app::NotificationLevel::Error, error.to_string());
            }
            if app.authenticated
                && app.active_modal.is_none()
                && let Err(error) = app.refresh_data()
            {
                app.set_notification(crate::app::NotificationLevel::Error, error.to_string());
            }
            refreshed = Instant::now();
        }
        terminal.draw(|frame| ui::render(app, frame))?;

        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(key) => handle_key_event(app, key),
                Event::Paste(text) => crate::event::handle_paste(app, &text),
                _ => {}
            }
        }
    }
    Ok(())
}

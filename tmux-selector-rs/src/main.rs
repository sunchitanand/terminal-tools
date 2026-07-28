mod app;
mod config;
mod ssh;
mod ui;

use anyhow::Result;
use app::{Action, App};
use config::Config;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::prelude::*;
use ssh::{Remote, SessionAction};
use std::io::{self, Write};
use ui::MouseMap;

/// What the event loop asks the outer driver to do after breaking out of the
/// alternate screen (interactive things that need the real terminal).
enum Outcome {
    Quit,
    Attach(SessionAction),
    Rename { old: String },
    Delete { name: String },
    BulkDelete { names: Vec<String> },
    NewSession,
    None,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let host = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "sunchit-cd2.aka.corp.amazon.com".to_string());
    let use_mosh = args.get(2).map(|s| s == "--mosh").unwrap_or(false)
        || std::env::var("USE_MOSH").map(|v| v == "1").unwrap_or(false);

    let remote = Remote::new(host.clone(), use_mosh);
    let mut cfg = Config::load(&host);

    // Hidden debug flag: fetch + print the session list, no TUI. Used to smoke
    // test the SSH layer without entering the alternate screen.
    if args.iter().any(|a| a == "--list") {
        let sessions = fetch_and_persist(&remote, &mut cfg);
        for s in &sessions {
            println!(
                "{}\trunning={}\tactivity={:?}\tdir={}",
                s.name, s.running, s.activity, s.dir
            );
        }
        return Ok(());
    }

    // Deploy the cmux env helper once, before entering the TUI.
    remote.deploy_helper();

    let sessions = fetch_and_persist(&remote, &mut cfg);
    let mut app = App::new(sessions);

    // Enter TUI.
    let mut terminal = setup_terminal()?;

    let result = run(&mut terminal, &mut app, &remote, &mut cfg);

    restore_terminal(&mut terminal)?;
    result
}

fn fetch_and_persist(remote: &Remote, cfg: &mut Config) -> Vec<ssh::Session> {
    let entries = cfg.entries();
    let sessions = remote.fetch_sessions(&entries);
    // Persist merged view back to config (mirrors zsh write-back).
    for s in &sessions {
        cfg.upsert(&s.name, &s.dir);
    }
    let _ = cfg.save();
    sessions
}

type Term = Terminal<CrosstermBackend<io::Stdout>>;

fn setup_terminal() -> Result<Term> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Term) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

/// Main loop. Draws, reads keys, and — for interactive actions — leaves the
/// alternate screen, performs the action, then re-enters and refreshes.
fn run(terminal: &mut Term, app: &mut App, remote: &Remote, cfg: &mut Config) -> Result<()> {
    // Persisted across frames so mouse events can hit-test against the last
    // rendered layout.
    let mut map = MouseMap::default();

    loop {
        terminal.draw(|f| {
            ui::render(
                f,
                &ui::RenderCtx {
                    app,
                    host_short: remote.short_host(),
                },
                &mut map,
            )
        })?;

        let outcome = match event::read()? {
            Event::Key(key) if key.kind == event::KeyEventKind::Press => handle_key(app, key),
            Event::Mouse(m) => handle_mouse(app, m, &map),
            _ => Outcome::None,
        };

        match outcome {
            Outcome::Quit => return Ok(()),
            Outcome::None => {}
            interactive => {
                // Suspend TUI, do the interactive thing, then resume.
                suspend(terminal)?;
                let should_exit = perform(interactive, app, remote, cfg)?;
                if should_exit {
                    return Ok(());
                }
                resume(terminal)?;
            }
        }
    }
}

/// Translate a mouse event into an outcome. Scroll navigates; left-click on a
/// row selects it (and runs the action if it was already the cursor row);
/// left-click on the action bar switches the current action.
fn handle_mouse(app: &mut App, m: MouseEvent, map: &MouseMap) -> Outcome {
    match m.kind {
        MouseEventKind::ScrollUp => {
            app.nav_up();
            Outcome::None
        }
        MouseEventKind::ScrollDown => {
            app.nav_down();
            Outcome::None
        }
        MouseEventKind::Down(MouseButton::Left) => {
            // Action bar takes priority (it sits below the table).
            if let Some(action) = map.hit_action(m.column, m.row) {
                app.action = action;
                // Clicking "delete N" while sessions are picked fires the bulk
                // delete immediately (matches the keyboard flow).
                if action == Action::Delete && !app.picked.is_empty() {
                    return decide_action(app);
                }
                return Outcome::None;
            }
            // A click on the marker cell (tick/dot) toggles the bulk-pick.
            if let Some(sel) = map.hit_marker(m.column, m.row) {
                app.toggle_pick_at(sel);
                return Outcome::None;
            }
            if let Some(sel) = map.hit_row(m.column, m.row) {
                if app.cursor == sel {
                    // Second click on the already-selected row: run the action.
                    return decide_action(app);
                }
                app.cursor = sel;
            }
            Outcome::None
        }
        _ => Outcome::None,
    }
}

/// Translate a keypress into an outcome, mutating pure UI state directly.
fn handle_key(app: &mut App, key: KeyEvent) -> Outcome {
    match key.code {
        KeyCode::Up => {
            app.nav_up();
            Outcome::None
        }
        KeyCode::Down => {
            app.nav_down();
            Outcome::None
        }
        KeyCode::Char(' ') => {
            app.toggle_pick();
            app.nav_down();
            Outcome::None
        }
        KeyCode::Tab => {
            app.action = app.action.next();
            Outcome::None
        }
        KeyCode::BackTab => {
            app.action = app.action.prev();
            Outcome::None
        }
        KeyCode::Enter => decide_action(app),
        KeyCode::Backspace => {
            app.backspace_search();
            Outcome::None
        }
        KeyCode::Esc => {
            app.escape();
            Outcome::None
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Outcome::Quit,
        KeyCode::Char('q') if app.search.is_empty() => Outcome::Quit,
        KeyCode::Char(c) => {
            app.push_search(c);
            Outcome::None
        }
        _ => Outcome::None,
    }
}

/// Resolve the current action against the cursor into a concrete Outcome.
fn decide_action(app: &mut App) -> Outcome {
    // After acting, action resets to Attach (matches zsh).
    let action = app.action;
    app.action = Action::Attach;

    match action {
        Action::Attach => {
            if app.on_new() {
                Outcome::NewSession
            } else if app.current_running() {
                match app.current_name() {
                    Some(name) => Outcome::Attach(SessionAction::Attach { name }),
                    None => Outcome::None,
                }
            } else {
                // Not running: create with the stored dir if we have one.
                match app.current_name() {
                    Some(name) => {
                        let dir = app.current_dir().unwrap_or_default();
                        if dir.is_empty() {
                            Outcome::Attach(SessionAction::New { name })
                        } else {
                            Outcome::Attach(SessionAction::NewInDir { name, dir })
                        }
                    }
                    None => Outcome::None,
                }
            }
        }
        Action::Rename => {
            if app.on_new() {
                return Outcome::None;
            }
            match app.current_name() {
                Some(old) => Outcome::Rename { old },
                None => Outcome::None,
            }
        }
        Action::Delete => {
            if !app.picked.is_empty() {
                return Outcome::BulkDelete {
                    names: app.picked_names(),
                };
            }
            if app.on_new() {
                return Outcome::None;
            }
            match app.current_name() {
                Some(name) => Outcome::Delete { name },
                None => Outcome::None,
            }
        }
    }
}

/// Perform an interactive outcome outside the alternate screen. Returns
/// Ok(true) if the program should exit afterwards (attach detaches → exit).
fn perform(outcome: Outcome, app: &mut App, remote: &Remote, cfg: &mut Config) -> Result<bool> {
    match outcome {
        Outcome::NewSession => {
            print!("\r\n  \x1b[36mSession name (project/name):\x1b[0m ");
            io::stdout().flush().ok();
            if let Some(name) = read_line()? {
                let name = name.trim().to_string();
                if !name.is_empty() {
                    cfg.upsert(&name, "");
                    let _ = cfg.save();
                    remote.run_interactive(SessionAction::New { name })?;
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Outcome::Attach(action) => {
            remote.run_interactive(action)?;
            Ok(true)
        }
        Outcome::Rename { old } => {
            print!("\r\n  \x1b[36mNew name for \x1b[35m{old}\x1b[36m:\x1b[0m ");
            io::stdout().flush().ok();
            if let Some(new_name) = read_line()? {
                let new_name = new_name.trim().to_string();
                if !new_name.is_empty() {
                    let old_info = app
                        .sessions
                        .iter()
                        .find(|s| s.name == old)
                        .map(|s| (s.running, s.dir.clone()));
                    if let Some((running, dir)) = old_info {
                        if running {
                            remote.rename_session(&old, &new_name);
                        }
                        cfg.remove(&old);
                        cfg.upsert(&new_name, &dir);
                        let _ = cfg.save();
                    }
                    refresh(app, remote, cfg);
                }
            }
            Ok(false)
        }
        Outcome::Delete { name } => {
            print!("\r\n  \x1b[36mDelete \x1b[35m{name}\x1b[36m? (y/n):\x1b[0m ");
            io::stdout().flush().ok();
            if confirm_yes()? {
                if let Some(s) = app.sessions.iter().find(|s| s.name == name) {
                    if s.running {
                        remote.kill_session(&name);
                    }
                }
                cfg.remove(&name);
                let _ = cfg.save();
                refresh(app, remote, cfg);
            }
            Ok(false)
        }
        Outcome::BulkDelete { names } => {
            print!(
                "\r\n  \x1b[36mDelete \x1b[35m{}\x1b[36m selected session(s)? (y/n):\x1b[0m ",
                names.len()
            );
            io::stdout().flush().ok();
            if confirm_yes()? {
                let running: Vec<String> = names
                    .iter()
                    .filter(|n| app.sessions.iter().any(|s| &s.name == *n && s.running))
                    .cloned()
                    .collect();
                remote.kill_sessions(&running);
                for n in &names {
                    cfg.remove(n);
                }
                let _ = cfg.save();
                app.clear_picked();
                refresh(app, remote, cfg);
            }
            Ok(false)
        }
        Outcome::Quit | Outcome::None => Ok(false),
    }
}

/// Re-fetch sessions and rebuild display, resetting search/action.
fn refresh(app: &mut App, remote: &Remote, cfg: &mut Config) {
    let sessions = fetch_and_persist(remote, cfg);
    app.set_sessions(sessions);
    app.search.clear();
    app.action = Action::Attach;
    if app.cursor >= app.selectable.len() {
        app.cursor = app.selectable.len().saturating_sub(1);
    }
}

// --- Terminal suspend / resume around interactive shell-outs ---

fn suspend(terminal: &mut Term) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn resume(terminal: &mut Term) -> Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    terminal.clear()?;
    terminal.hide_cursor()?;
    Ok(())
}

// --- Cooked-mode line/confirm input (used while suspended) ---

fn read_line() -> Result<Option<String>> {
    let mut line = String::new();
    let n = io::stdin().read_line(&mut line)?;
    if n == 0 {
        Ok(None)
    } else {
        Ok(Some(line))
    }
}

fn confirm_yes() -> Result<bool> {
    // We're in cooked mode here; read one line and check its first char.
    match read_line()? {
        Some(s) => Ok(s.trim_start().starts_with('y')),
        None => Ok(false),
    }
}

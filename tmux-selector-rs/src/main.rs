mod app;
mod config;
mod ssh;
mod ui;

use anyhow::Result;
use app::{Action, App, Prompt};
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
use std::io::{self};
use ui::MouseMap;

/// What the event loop should do next. Two flavours:
/// - `Attach` / `NewSessionNamed` hand the terminal to ssh (they leave the
///   alternate screen and then the program exits after detach).
/// - `Exec*` run a quick non-interactive ssh call *without* leaving the TUI —
///   the confirmation and result stay inside the fullscreen UI.
enum Outcome {
    Quit,
    Attach(SessionAction),
    NewSessionNamed { name: String },
    ExecDelete { names: Vec<String> },
    ExecRename { old: String, new: String },
    ExecMove { names: Vec<String>, target: String },
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

/// Main loop. Draws, reads events, and routes them. Confirmations and text
/// input are handled as in-TUI modals; only attach/new-session (which must
/// hand the terminal to ssh) leave the fullscreen UI.
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
            Event::Key(key) if key.kind == event::KeyEventKind::Press => {
                if app.prompt.is_active() {
                    handle_prompt_key(app, key)
                } else {
                    handle_key(app, key)
                }
            }
            // Mouse is ignored while a modal is open.
            Event::Mouse(m) if !app.prompt.is_active() => handle_mouse(app, m, &map),
            _ => Outcome::None,
        };

        match outcome {
            Outcome::Quit => return Ok(()),
            Outcome::None => {}

            // These run a quick ssh call in-place and redraw — no screen leave.
            Outcome::ExecDelete { names } => {
                exec_delete(app, remote, cfg, terminal, &mut map, names)?;
            }
            Outcome::ExecRename { old, new } => {
                exec_rename(app, remote, cfg, old, new);
            }
            Outcome::ExecMove { names, target } => {
                exec_move(app, remote, cfg, names, target);
            }

            // These hand the terminal to ssh, then the program exits on detach.
            Outcome::Attach(action) => {
                suspend(terminal)?;
                remote.run_interactive(action)?;
                return Ok(());
            }
            Outcome::NewSessionNamed { name } => {
                cfg.upsert(&name, "");
                let _ = cfg.save();
                suspend(terminal)?;
                remote.run_interactive(SessionAction::New { name })?;
                return Ok(());
            }
        }
    }
}

/// Draw a transient status line (e.g. "Deleting 3…") before a blocking ssh
/// call, so the UI doesn't look frozen.
fn draw_status(
    terminal: &mut Term,
    app: &mut App,
    remote: &Remote,
    map: &mut MouseMap,
    msg: &str,
) -> Result<()> {
    app.status = Some(msg.to_string());
    terminal.draw(|f| {
        ui::render(
            f,
            &ui::RenderCtx {
                app,
                host_short: remote.short_host(),
            },
            map,
        )
    })?;
    app.status = None;
    Ok(())
}

fn exec_delete(
    app: &mut App,
    remote: &Remote,
    cfg: &mut Config,
    terminal: &mut Term,
    map: &mut MouseMap,
    names: Vec<String>,
) -> Result<()> {
    app.cancel_prompt();
    draw_status(terminal, app, remote, map, &format!("Deleting {}…", names.len()))?;
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
    Ok(())
}

fn exec_rename(app: &mut App, remote: &Remote, cfg: &mut Config, old: String, new: String) {
    app.cancel_prompt();
    if new.is_empty() || new == old {
        return;
    }
    if let Some((running, dir)) = app
        .sessions
        .iter()
        .find(|s| s.name == old)
        .map(|s| (s.running, s.dir.clone()))
    {
        if running {
            remote.rename_session(&old, &new);
        }
        cfg.remove(&old);
        cfg.upsert(&new, &dir);
        let _ = cfg.save();
    }
    refresh(app, remote, cfg);
}

fn exec_move(app: &mut App, remote: &Remote, cfg: &mut Config, names: Vec<String>, target: String) {
    app.cancel_prompt();
    if target.trim().is_empty() {
        return;
    }
    for old in &names {
        let new_name = app::moved_name(old, &target);
        if new_name == *old {
            continue; // already in that project
        }
        if let Some((running, dir)) = app
            .sessions
            .iter()
            .find(|s| &s.name == old)
            .map(|s| (s.running, s.dir.clone()))
        {
            if running {
                // Pure relabel — tmux session name is unrelated to
                // pane_current_path, so the remote dir is untouched.
                remote.rename_session(old, &new_name);
            }
            cfg.remove(old);
            cfg.upsert(&new_name, &dir);
        }
    }
    let _ = cfg.save();
    app.clear_picked();
    refresh(app, remote, cfg);
}

/// Route a keypress to the active modal prompt. Enter confirms, Esc cancels.
fn handle_prompt_key(app: &mut App, key: KeyEvent) -> Outcome {
    // Ctrl-C always aborts the whole app.
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Outcome::Quit;
    }
    match key.code {
        KeyCode::Esc => {
            app.cancel_prompt();
            Outcome::None
        }
        KeyCode::Enter => match app.prompt.clone() {
            Prompt::ConfirmDelete { names } => Outcome::ExecDelete { names },
            Prompt::Rename { old, buffer } => Outcome::ExecRename {
                old,
                new: buffer.trim().to_string(),
            },
            Prompt::MoveTo { names, .. } => match app.move_selected_project() {
                Some(target) => Outcome::ExecMove { names, target },
                None => {
                    app.cancel_prompt();
                    Outcome::None
                }
            },
            Prompt::NewSession { buffer } => {
                let name = buffer.trim().to_string();
                if name.is_empty() {
                    app.cancel_prompt();
                    Outcome::None
                } else {
                    app.cancel_prompt();
                    Outcome::NewSessionNamed { name }
                }
            }
            Prompt::None => Outcome::None,
        },
        // Confirm dialogs also accept y / n directly.
        KeyCode::Char('y') | KeyCode::Char('Y')
            if matches!(app.prompt, Prompt::ConfirmDelete { .. }) =>
        {
            if let Prompt::ConfirmDelete { names } = app.prompt.clone() {
                Outcome::ExecDelete { names }
            } else {
                Outcome::None
            }
        }
        KeyCode::Char('n') | KeyCode::Char('N')
            if matches!(app.prompt, Prompt::ConfirmDelete { .. }) =>
        {
            app.cancel_prompt();
            Outcome::None
        }
        // In the move picker, up/down cycle the target project.
        KeyCode::Up => {
            app.move_prev();
            Outcome::None
        }
        KeyCode::Down => {
            app.move_next();
            Outcome::None
        }
        KeyCode::Backspace => {
            app.prompt_backspace();
            Outcome::None
        }
        KeyCode::Char(c) => {
            app.prompt_push(c);
            Outcome::None
        }
        _ => Outcome::None,
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
                // Clicking "delete N" / "move N" while sessions are picked
                // fires the bulk action immediately (matches the keyboard flow).
                if !app.picked.is_empty()
                    && matches!(action, Action::Delete | Action::Move)
                {
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

/// Resolve the current action against the cursor. Attach hands off to ssh;
/// rename/move/delete/new open an in-TUI modal (returning `Outcome::None` —
/// the modal drives the rest).
fn decide_action(app: &mut App) -> Outcome {
    // After acting, action resets to Attach (matches zsh).
    let action = app.action;
    app.action = Action::Attach;

    match action {
        Action::Attach => {
            if app.on_new() {
                app.open_new_session();
                Outcome::None
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
            if let Some(old) = app.current_name() {
                app.open_rename(old);
            }
            Outcome::None
        }
        Action::Move => {
            // Bulk move when sessions are picked; else the cursor session.
            let names = if !app.picked.is_empty() {
                app.picked_names()
            } else if app.on_new() {
                return Outcome::None;
            } else {
                match app.current_name() {
                    Some(name) => vec![name],
                    None => return Outcome::None,
                }
            };
            app.open_move(names);
            Outcome::None
        }
        Action::Delete => {
            let names = if !app.picked.is_empty() {
                app.picked_names()
            } else if app.on_new() {
                return Outcome::None;
            } else {
                match app.current_name() {
                    Some(name) => vec![name],
                    None => return Outcome::None,
                }
            };
            app.open_confirm_delete(names);
            Outcome::None
        }
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

// --- Terminal suspend (handing off to ssh) ---

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

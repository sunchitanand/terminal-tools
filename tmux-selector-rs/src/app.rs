//! Application state: the session list, the derived display rows (headers +
//! sessions + the "new" entry), search filtering, cursor navigation, action
//! cycling, and bulk selection.

use crate::ssh::Session;
use std::collections::HashSet;

/// A row in the rendered list.
#[derive(Clone, Debug)]
pub enum Row {
    /// The "+ New session" entry.
    New,
    /// A project group header (the text before `/`, or "other").
    Header(String),
    /// A session row; the index points into `App::sessions`.
    Session(usize),
}

/// Which action Enter will perform. Tab cycles through these.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Attach,
    Rename,
    Move,
    Delete,
}

impl Action {
    pub fn next(self) -> Action {
        match self {
            Action::Attach => Action::Rename,
            Action::Rename => Action::Move,
            Action::Move => Action::Delete,
            Action::Delete => Action::Attach,
        }
    }
    pub fn prev(self) -> Action {
        match self {
            Action::Attach => Action::Delete,
            Action::Rename => Action::Attach,
            Action::Move => Action::Rename,
            Action::Delete => Action::Move,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Action::Attach => "attach",
            Action::Rename => "rename",
            Action::Move => "move",
            Action::Delete => "delete",
        }
    }
}

/// A modal prompt drawn over the list. While active, key input is routed to it
/// instead of the list, so confirmations/inputs stay inside the TUI rather than
/// dropping back to the shell.
#[derive(Clone, Debug)]
pub enum Prompt {
    None,
    /// y/n confirmation before deleting these sessions.
    ConfirmDelete { names: Vec<String> },
    /// Text input for renaming a single session (buffer pre-filled with `old`).
    Rename { old: String, buffer: String },
    /// Target-project picker when moving these sessions. Instead of typing, the
    /// user cycles `candidates` with up/down; `selected` is the current choice.
    MoveTo {
        names: Vec<String>,
        candidates: Vec<String>,
        selected: usize,
    },
    /// Text input for a new session name.
    NewSession { buffer: String },
}

impl Prompt {
    pub fn is_active(&self) -> bool {
        !matches!(self, Prompt::None)
    }
}

pub struct App {
    pub sessions: Vec<Session>,
    pub rows: Vec<Row>,
    /// Indices into `rows` that are selectable (New + Session rows).
    pub selectable: Vec<usize>,
    /// Cursor position as an index into `selectable`.
    pub cursor: usize,
    pub search: String,
    pub action: Action,
    /// Bulk-selection set, keyed by session name (stable across refetch).
    pub picked: HashSet<String>,
    /// Active modal prompt, if any.
    pub prompt: Prompt,
    /// Transient status line (e.g. "Deleting 3…") shown during a blocking op.
    pub status: Option<String>,
}

impl App {
    pub fn new(sessions: Vec<Session>) -> Self {
        let mut app = App {
            sessions,
            rows: Vec::new(),
            selectable: Vec::new(),
            cursor: 0,
            search: String::new(),
            action: Action::Attach,
            picked: HashSet::new(),
            prompt: Prompt::None,
            status: None,
        };
        app.rebuild();
        app
    }

    pub fn set_sessions(&mut self, sessions: Vec<Session>) {
        self.sessions = sessions;
        self.rebuild();
    }

    /// Rebuild `rows` and `selectable` from `sessions`. Groups by project
    /// prefix, orders projects by their most-recently-active session (most
    /// recent group first, "other" last), and orders sessions within a project
    /// by activity descending.
    pub fn rebuild(&mut self) {
        self.rows.clear();
        self.selectable.clear();

        // "+ New session" first.
        self.rows.push(Row::New);

        // Group session indices by project.
        let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
        let mut index_of: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for (i, s) in self.sessions.iter().enumerate() {
            let proj = match s.name.split_once('/') {
                Some((p, _)) => p.to_string(),
                None => "other".to_string(),
            };
            match index_of.get(&proj) {
                Some(&gi) => groups[gi].1.push(i),
                None => {
                    index_of.insert(proj.clone(), groups.len());
                    groups.push((proj, vec![i]));
                }
            }
        }

        // Each group's rank = the max activity_ts across its sessions, so a
        // project bubbles to the top whenever any of its sessions was used
        // recently. Sort by that rank descending; "other" is always pinned
        // last regardless of recency.
        let group_rank = |idxs: &[usize]| -> i64 {
            idxs.iter()
                .map(|&i| self.sessions[i].activity_ts)
                .max()
                .unwrap_or(0)
        };
        groups.sort_by(|a, b| match (a.0.as_str(), b.0.as_str()) {
            ("other", "other") => std::cmp::Ordering::Equal,
            ("other", _) => std::cmp::Ordering::Greater,
            (_, "other") => std::cmp::Ordering::Less,
            _ => group_rank(&b.1)
                .cmp(&group_rank(&a.1))
                // Tie-break alphabetically for stable ordering (e.g. offline
                // projects whose sessions all have activity_ts 0).
                .then_with(|| a.0.cmp(&b.0)),
        });

        for (proj, mut idxs) in groups {
            self.rows.push(Row::Header(proj));
            // Sort sessions by activity timestamp descending (latest first).
            idxs.sort_by(|&a, &b| self.sessions[b].activity_ts.cmp(&self.sessions[a].activity_ts));
            for i in idxs {
                self.rows.push(Row::Session(i));
            }
        }

        // Recompute selectable rows.
        for (ri, row) in self.rows.iter().enumerate() {
            if matches!(row, Row::New | Row::Session(_)) {
                self.selectable.push(ri);
            }
        }

        if self.cursor >= self.selectable.len() {
            self.cursor = self.selectable.len().saturating_sub(1);
        }
    }

    // --- Search matching ---

    /// Case-insensitive subsequence match.
    pub fn fuzzy(haystack: &str, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        let hay: Vec<char> = haystack.to_lowercase().chars().collect();
        let need: Vec<char> = needle.to_lowercase().chars().collect();
        let mut ni = 0;
        for &hc in &hay {
            if ni < need.len() && hc == need[ni] {
                ni += 1;
            }
        }
        ni == need.len()
    }

    /// Does the selectable at `sel_idx` match the current search?
    /// The "New" row never matches when searching.
    pub fn matches(&self, sel_idx: usize) -> bool {
        if self.search.is_empty() {
            return true;
        }
        let ri = self.selectable[sel_idx];
        match &self.rows[ri] {
            Row::New => false,
            Row::Session(si) => Self::fuzzy(&self.sessions[*si].name, &self.search),
            Row::Header(_) => false,
        }
    }

    /// Does a given row match search? Headers match if any child session does.
    pub fn row_visible(&self, ri: usize) -> bool {
        if self.search.is_empty() {
            return true;
        }
        match &self.rows[ri] {
            Row::New => false, // hidden while searching
            Row::Session(si) => Self::fuzzy(&self.sessions[*si].name, &self.search),
            Row::Header(_) => {
                // Visible if any following session (until next header) matches.
                for r in &self.rows[ri + 1..] {
                    match r {
                        Row::Header(_) => break,
                        Row::Session(si) => {
                            if Self::fuzzy(&self.sessions[*si].name, &self.search) {
                                return true;
                            }
                        }
                        Row::New => {}
                    }
                }
                false
            }
        }
    }

    /// Number of *visible* session rows in the group starting at header `ri`.
    pub fn group_visible_count(&self, ri: usize) -> usize {
        let mut n = 0;
        for r in ri + 1..self.rows.len() {
            match &self.rows[r] {
                Row::Header(_) => break,
                Row::Session(_) if self.row_visible(r) => n += 1,
                _ => {}
            }
        }
        n
    }

    /// Is the session at `ri` the last *visible* session in its group? Used to
    /// choose the tree connector (└ vs ├).
    pub fn is_last_visible_in_group(&self, ri: usize) -> bool {
        for r in ri + 1..self.rows.len() {
            match &self.rows[r] {
                Row::Header(_) => return true,
                Row::Session(_) if self.row_visible(r) => return false,
                _ => {}
            }
        }
        true
    }

    // --- Navigation (skips non-matching rows when searching) ---

    pub fn nav_up(&mut self) {
        if self.selectable.is_empty() {
            return;
        }
        let start = self.cursor;
        loop {
            self.cursor = if self.cursor == 0 {
                self.selectable.len() - 1
            } else {
                self.cursor - 1
            };
            if self.cursor == start {
                break;
            }
            if self.search.is_empty() || self.matches(self.cursor) {
                break;
            }
        }
    }

    pub fn nav_down(&mut self) {
        if self.selectable.is_empty() {
            return;
        }
        let start = self.cursor;
        loop {
            self.cursor = if self.cursor + 1 >= self.selectable.len() {
                0
            } else {
                self.cursor + 1
            };
            if self.cursor == start {
                break;
            }
            if self.search.is_empty() || self.matches(self.cursor) {
                break;
            }
        }
    }

    /// Move the cursor to the first selectable that matches the search.
    pub fn jump_to_first_match(&mut self) {
        for i in 0..self.selectable.len() {
            if self.matches(i) {
                self.cursor = i;
                return;
            }
        }
    }

    /// Toggle the bulk-pick state of a session by its selectable index (used by
    /// mouse clicks on the marker column). No-op for the "New" row.
    pub fn toggle_pick_at(&mut self, sel_idx: usize) {
        let Some(&ri) = self.selectable.get(sel_idx) else {
            return;
        };
        if let Row::Session(si) = &self.rows[ri] {
            let name = self.sessions[*si].name.clone();
            if self.picked.contains(&name) {
                self.picked.remove(&name);
            } else {
                self.picked.insert(name);
            }
        }
    }

    // --- Current-row accessors ---

    fn current_row(&self) -> Option<&Row> {
        self.selectable.get(self.cursor).map(|&ri| &self.rows[ri])
    }

    pub fn on_new(&self) -> bool {
        matches!(self.current_row(), Some(Row::New))
    }

    pub fn current_session(&self) -> Option<&Session> {
        match self.current_row() {
            Some(Row::Session(si)) => self.sessions.get(*si),
            _ => None,
        }
    }

    pub fn current_name(&self) -> Option<String> {
        self.current_session().map(|s| s.name.clone())
    }

    pub fn current_running(&self) -> bool {
        self.current_session().map(|s| s.running).unwrap_or(false)
    }

    pub fn current_dir(&self) -> Option<String> {
        self.current_session().map(|s| s.dir.clone())
    }

    // --- Modal prompt lifecycle ---

    pub fn open_new_session(&mut self) {
        self.prompt = Prompt::NewSession {
            buffer: String::new(),
        };
    }

    pub fn open_rename(&mut self, old: String) {
        self.prompt = Prompt::Rename {
            buffer: old.clone(),
            old,
        };
    }

    pub fn open_move(&mut self, names: Vec<String>) {
        // Candidate target projects = existing projects, plus "other" so a
        // session can be moved out of any project into the ungrouped bucket.
        let mut candidates = self.project_names();
        candidates.push("other".to_string());
        // Start on the project the (first) session is currently in, if any, so
        // the initial highlight is a sensible no-op rather than a stray move.
        let current_proj = names
            .first()
            .and_then(|n| n.split_once('/').map(|(p, _)| p.to_string()))
            .unwrap_or_else(|| "other".to_string());
        let selected = candidates
            .iter()
            .position(|c| *c == current_proj)
            .unwrap_or(0);
        self.prompt = Prompt::MoveTo {
            names,
            candidates,
            selected,
        };
    }

    /// Cycle the move-target selection up/down (wraps). No-op for other prompts.
    pub fn move_prev(&mut self) {
        if let Prompt::MoveTo {
            candidates,
            selected,
            ..
        } = &mut self.prompt
        {
            if !candidates.is_empty() {
                *selected = if *selected == 0 {
                    candidates.len() - 1
                } else {
                    *selected - 1
                };
            }
        }
    }

    pub fn move_next(&mut self) {
        if let Prompt::MoveTo {
            candidates,
            selected,
            ..
        } = &mut self.prompt
        {
            if !candidates.is_empty() {
                *selected = (*selected + 1) % candidates.len();
            }
        }
    }

    /// The currently highlighted target project in a move prompt.
    pub fn move_selected_project(&self) -> Option<String> {
        if let Prompt::MoveTo {
            candidates,
            selected,
            ..
        } = &self.prompt
        {
            candidates.get(*selected).cloned()
        } else {
            None
        }
    }

    pub fn open_confirm_delete(&mut self, names: Vec<String>) {
        self.prompt = Prompt::ConfirmDelete { names };
    }

    pub fn cancel_prompt(&mut self) {
        self.prompt = Prompt::None;
    }

    /// Append a char to the active text prompt's buffer (no-op for confirm and
    /// for the move picker, which is arrow-driven).
    pub fn prompt_push(&mut self, c: char) {
        match &mut self.prompt {
            Prompt::Rename { buffer, .. } | Prompt::NewSession { buffer } => buffer.push(c),
            _ => {}
        }
    }

    /// Delete the last char of the active text prompt's buffer.
    pub fn prompt_backspace(&mut self) {
        match &mut self.prompt {
            Prompt::Rename { buffer, .. } | Prompt::NewSession { buffer } => {
                buffer.pop();
            }
            _ => {}
        }
    }

    /// Distinct project names currently in use, sorted, "other" excluded. Used
    /// to show the user their existing projects when moving a session.
    pub fn project_names(&self) -> Vec<String> {
        let mut set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for s in &self.sessions {
            if let Some((proj, _)) = s.name.split_once('/') {
                set.insert(proj.to_string());
            }
        }
        set.into_iter().collect()
    }

    // --- Bulk selection ---

    pub fn toggle_pick(&mut self) {
        if let Some(name) = self.current_name() {
            if self.picked.contains(&name) {
                self.picked.remove(&name);
            } else {
                self.picked.insert(name);
            }
        }
    }

    pub fn is_picked(&self, name: &str) -> bool {
        self.picked.contains(name)
    }

    pub fn picked_names(&self) -> Vec<String> {
        self.picked.iter().cloned().collect()
    }

    pub fn clear_picked(&mut self) {
        self.picked.clear();
    }

    /// Staged Esc: clear search, then selection, then reset action. Returns
    /// true if anything was cleared.
    pub fn escape(&mut self) -> bool {
        if !self.search.is_empty() {
            self.search.clear();
            self.cursor = 0;
            true
        } else if !self.picked.is_empty() {
            self.picked.clear();
            true
        } else if self.action != Action::Attach {
            self.action = Action::Attach;
            true
        } else {
            false
        }
    }

    pub fn push_search(&mut self, c: char) {
        self.search.push(c);
        self.jump_to_first_match();
    }

    pub fn backspace_search(&mut self) {
        if self.search.pop().is_some() {
            if self.search.is_empty() {
                self.cursor = 0;
            } else {
                self.jump_to_first_match();
            }
        }
    }
}

/// The bare session name without its project prefix. `alpha/foo` -> `foo`;
/// an ungrouped `bar` -> `bar`.
pub fn session_suffix(name: &str) -> &str {
    name.split_once('/').map(|(_, s)| s).unwrap_or(name)
}

/// Compute the new full name when moving `name` into `target_project`. The
/// suffix is preserved. An empty (or "other") target strips the prefix so the
/// session falls into the ungrouped "other" bucket. The project part is trimmed
/// of surrounding whitespace and any stray slashes the user typed.
pub fn moved_name(name: &str, target_project: &str) -> String {
    let suffix = session_suffix(name);
    let proj = target_project.trim().trim_matches('/');
    if proj.is_empty() || proj == "other" {
        suffix.to_string()
    } else {
        format!("{proj}/{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::Session;

    fn sess(name: &str, running: bool, activity_ts: i64) -> Session {
        Session {
            name: name.to_string(),
            running,
            created: String::new(),
            activity: String::new(),
            activity_ts,
            dir: String::new(),
        }
    }

    #[test]
    fn fuzzy_subsequence() {
        assert!(App::fuzzy("replay/mainline", "rml"));
        assert!(App::fuzzy("replay/mainline", "MAIN"));
        assert!(!App::fuzzy("replay/mainline", "xyz"));
        assert!(App::fuzzy("anything", "")); // empty matches
    }

    #[test]
    fn move_picker_cycles_and_starts_on_current_project() {
        let app_sessions = vec![
            sess("alpha/x", true, 3),
            sess("beta/y", true, 2),
            sess("gamma/z", true, 1),
        ];
        let mut app = App::new(app_sessions);
        // Move beta/y: candidates are the projects (sorted) + "other", and the
        // initial selection lands on the session's current project "beta".
        app.open_move(vec!["beta/y".to_string()]);
        if let Prompt::MoveTo { candidates, .. } = &app.prompt {
            assert_eq!(candidates, &["alpha", "beta", "gamma", "other"]);
        } else {
            panic!("expected MoveTo prompt");
        }
        assert_eq!(app.move_selected_project().as_deref(), Some("beta"));

        // Down moves to gamma, then other, then wraps to alpha.
        app.move_next();
        assert_eq!(app.move_selected_project().as_deref(), Some("gamma"));
        app.move_next();
        assert_eq!(app.move_selected_project().as_deref(), Some("other"));
        app.move_next();
        assert_eq!(app.move_selected_project().as_deref(), Some("alpha"));

        // Up from alpha wraps back to other.
        app.move_prev();
        assert_eq!(app.move_selected_project().as_deref(), Some("other"));
    }

    #[test]
    fn move_selected_project_none_when_not_moving() {
        let app = App::new(vec![sess("alpha/x", true, 1)]);
        assert_eq!(app.move_selected_project(), None);
    }

    #[test]
    fn moved_name_preserves_suffix() {
        // Move keeps the part after "/", only swaps the project prefix.
        assert_eq!(moved_name("alpha/foo", "beta"), "beta/foo");
        // Ungrouped session gains a project.
        assert_eq!(moved_name("loose", "beta"), "beta/loose");
        // Target "other" or empty strips the prefix into the ungrouped bucket.
        assert_eq!(moved_name("alpha/foo", "other"), "foo");
        assert_eq!(moved_name("alpha/foo", ""), "foo");
        // Stray slashes / whitespace the user typed are trimmed.
        assert_eq!(moved_name("alpha/foo", "  beta/  "), "beta/foo");
    }

    #[test]
    fn session_suffix_extracts_tail() {
        assert_eq!(session_suffix("alpha/foo"), "foo");
        assert_eq!(session_suffix("loose"), "loose");
    }

    #[test]
    fn project_names_are_distinct_sorted_no_other() {
        let app = App::new(vec![
            sess("beta/x", true, 1),
            sess("alpha/y", true, 1),
            sess("beta/z", true, 1),
            sess("loose", true, 1), // ungrouped -> excluded
        ]);
        assert_eq!(app.project_names(), vec!["alpha", "beta"]);
    }

    fn headers(app: &App) -> Vec<String> {
        app.rows
            .iter()
            .filter_map(|r| match r {
                Row::Header(h) => Some(h.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn grouping_ties_break_alphabetically_other_last() {
        // Equal activity -> alphabetical tie-break; "other" pinned last.
        let app = App::new(vec![
            sess("zeta/a", true, 1),
            sess("alpha/b", true, 1),
            sess("standalone", true, 1), // no slash -> "other"
        ]);
        assert_eq!(headers(&app), vec!["alpha", "zeta", "other"]);
    }

    #[test]
    fn projects_ordered_by_most_recent_session() {
        // beta has the single most-recent session, so it bubbles to the top
        // even though alpha sorts first alphabetically. gamma is oldest.
        let app = App::new(vec![
            sess("alpha/a1", true, 200),
            sess("alpha/a2", true, 100),
            sess("beta/b1", true, 999), // newest anywhere
            sess("gamma/g1", true, 50),
            sess("solo", true, 500), // -> "other", always last
        ]);
        assert_eq!(headers(&app), vec!["beta", "alpha", "gamma", "other"]);
    }

    #[test]
    fn sessions_within_group_sorted_by_activity_desc() {
        let app = App::new(vec![
            sess("p/old", true, 100),
            sess("p/new", true, 500),
            sess("p/mid", true, 300),
        ]);
        let names: Vec<String> = app
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::Session(i) => Some(app.sessions[*i].name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["p/new", "p/mid", "p/old"]);
    }

    #[test]
    fn nav_wraps_and_skips_headers() {
        let mut app = App::new(vec![sess("p/a", true, 1), sess("q/b", true, 1)]);
        // selectable = [New, p/a, q/b] -> 3 entries.
        assert_eq!(app.selectable.len(), 3);
        assert_eq!(app.cursor, 0);
        app.nav_up(); // wrap to last
        assert_eq!(app.cursor, 2);
        app.nav_down(); // wrap to first
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn escape_is_staged() {
        let mut app = App::new(vec![sess("p/a", true, 1)]);
        app.search = "a".into();
        app.picked.insert("p/a".into());
        app.action = Action::Delete;

        assert!(app.escape()); // clears search first
        assert!(app.search.is_empty());
        assert_eq!(app.picked.len(), 1);

        assert!(app.escape()); // then selection
        assert!(app.picked.is_empty());
        assert_eq!(app.action, Action::Delete);

        assert!(app.escape()); // then action reset
        assert_eq!(app.action, Action::Attach);

        assert!(!app.escape()); // nothing left
    }

    #[test]
    fn toggle_pick_by_name() {
        let mut app = App::new(vec![sess("p/a", true, 1)]);
        app.cursor = 1; // p/a
        app.toggle_pick();
        assert!(app.is_picked("p/a"));
        app.toggle_pick();
        assert!(!app.is_picked("p/a"));
    }

    #[test]
    fn toggle_pick_at_by_index() {
        let mut app = App::new(vec![sess("p/a", true, 1), sess("p/b", true, 1)]);
        // selectable = [New(0), p/a(1), p/b(2)]. Index 0 is New -> no-op.
        app.toggle_pick_at(0);
        assert!(app.picked.is_empty());
        app.toggle_pick_at(1);
        assert!(app.is_picked("p/a"));
        app.toggle_pick_at(1);
        assert!(!app.is_picked("p/a"));
        // Out-of-range index is a safe no-op.
        app.toggle_pick_at(99);
        assert!(app.picked.is_empty());
    }

    #[test]
    fn search_hides_new_row() {
        let mut app = App::new(vec![sess("p/a", true, 1)]);
        app.push_search('a');
        // New row (selectable[0]) should not match.
        assert!(!app.matches(0));
        assert!(app.matches(1));
    }
}

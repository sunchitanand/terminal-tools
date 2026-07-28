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
    Delete,
}

impl Action {
    pub fn next(self) -> Action {
        match self {
            Action::Attach => Action::Rename,
            Action::Rename => Action::Delete,
            Action::Delete => Action::Attach,
        }
    }
    pub fn prev(self) -> Action {
        match self {
            Action::Attach => Action::Delete,
            Action::Rename => Action::Attach,
            Action::Delete => Action::Rename,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Action::Attach => "attach",
            Action::Rename => "rename",
            Action::Delete => "delete",
        }
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
        };
        app.rebuild();
        app
    }

    pub fn set_sessions(&mut self, sessions: Vec<Session>) {
        self.sessions = sessions;
        self.rebuild();
    }

    /// Rebuild `rows` and `selectable` from `sessions`. Groups by project
    /// prefix, sorts projects alphabetically ("other" last), and orders
    /// sessions within a project by activity descending.
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

        // Sort project names alphabetically, "other" last.
        groups.sort_by(|a, b| match (a.0.as_str(), b.0.as_str()) {
            ("other", "other") => std::cmp::Ordering::Equal,
            ("other", _) => std::cmp::Ordering::Greater,
            (_, "other") => std::cmp::Ordering::Less,
            (x, y) => x.cmp(y),
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
    fn grouping_sorts_projects_other_last() {
        let app = App::new(vec![
            sess("zeta/a", true, 1),
            sess("alpha/b", true, 1),
            sess("standalone", true, 1), // no slash -> "other"
        ]);
        // rows[0] = New. Then headers alpha, zeta, other.
        let headers: Vec<String> = app
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::Header(h) => Some(h.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(headers, vec!["alpha", "zeta", "other"]);
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

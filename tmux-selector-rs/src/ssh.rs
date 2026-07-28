//! Remote tmux operations over ssh (and optionally mosh for the final attach).
//! Mirrors the behavior of the zsh script's fetch_sessions / run_remote.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::process::Command;

/// Path & environment for cmux socket forwarding, if running inside cmux.
pub struct CmuxEnv {
    pub local_sock: String,
    pub remote_sock: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub panel_id: String,
    pub surface_id: String,
}

impl CmuxEnv {
    /// Detect cmux from the environment, matching the zsh script's probe.
    pub fn detect() -> Option<CmuxEnv> {
        let env_sock = std::env::var("CMUX_SOCKET_PATH").ok();
        let default_sock = std::env::var("HOME")
            .map(|h| format!("{h}/Library/Application Support/cmux/cmux.sock"))
            .unwrap_or_default();

        let sock = match &env_sock {
            Some(s) if is_socket(s) => s.clone(),
            _ if is_socket(&default_sock) => default_sock,
            _ => return None,
        };

        Some(CmuxEnv {
            local_sock: sock,
            remote_sock: "/tmp/cmux.sock".to_string(),
            workspace_id: env_or_empty("CMUX_WORKSPACE_ID"),
            tab_id: env_or_empty("CMUX_TAB_ID"),
            panel_id: env_or_empty("CMUX_PANEL_ID"),
            surface_id: env_or_empty("CMUX_SURFACE_ID"),
        })
    }

    /// `export CMUX_...=...;` prefix run before the remote command.
    fn export_prefix(&self) -> String {
        format!(
            "export CMUX_SOCKET_PATH={} CMUX_WORKSPACE_ID={} CMUX_TAB_ID={} CMUX_PANEL_ID={} CMUX_SURFACE_ID={};",
            self.remote_sock, self.workspace_id, self.tab_id, self.panel_id, self.surface_id
        )
    }

    /// tmux global env setup so new panes inherit the forwarded socket.
    fn tmux_env(&self, rtmux: &str) -> String {
        format!(
            "{rtmux} set-environment -g CMUX_SOCKET_PATH '{}' 2>/dev/null;\
             {rtmux} set-option -g update-environment 'CMUX_SOCKET_PATH CMUX_WORKSPACE_ID CMUX_TAB_ID CMUX_PANEL_ID CMUX_SURFACE_ID' 2>/dev/null;",
            self.remote_sock
        )
    }
}

/// A tmux session as known to us: from the config, from the live server, or both.
#[derive(Clone, Debug)]
pub struct Session {
    pub name: String,
    pub running: bool,
    pub created: String,
    pub activity: String,
    pub activity_ts: i64,
    pub dir: String,
}

pub struct Remote {
    pub host: String,
    pub rtmux: String,
    ssh_opts: Vec<String>,
    use_mosh: bool,
    cmux: Option<CmuxEnv>,
}

impl Remote {
    pub fn new(host: String, use_mosh: bool) -> Self {
        Remote {
            host,
            rtmux: "PATH=/usr/local/bin:/apollo/env/envImprovement/bin:$PATH tmux -u".to_string(),
            ssh_opts: vec![
                "-o".into(),
                "ServerAliveInterval=5".into(),
                "-o".into(),
                "ServerAliveCountMax=3".into(),
                "-o".into(),
                "LogLevel=ERROR".into(),
            ],
            use_mosh,
            cmux: CmuxEnv::detect(),
        }
    }

    pub fn short_host(&self) -> &str {
        self.host.split('.').next().unwrap_or(&self.host)
    }

    /// One-shot ssh returning captured stdout (no tty).
    fn ssh_capture(&self, remote_cmd: &str) -> Result<String> {
        let out = Command::new("ssh")
            .args(&self.ssh_opts)
            .arg(&self.host)
            .arg(remote_cmd)
            .output()
            .context("spawning ssh")?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Deploy the per-session env helper once at startup (only under cmux).
    /// Avoids quoting issues with `/` in session names.
    pub fn deploy_helper(&self) {
        if self.cmux.is_none() {
            return;
        }
        let script = r#"mkdir -p ~/.cmux-kiro && cat > ~/.cmux-kiro/set-env.sh << 'CMUXEOF'
#!/bin/bash
sess="$1"; shift
export PATH="/usr/local/bin:/apollo/env/envImprovement/bin:$PATH"
while [[ $# -ge 2 ]]; do
    tmux set-environment -t "$sess" "$1" "$2"
    shift 2
done
CMUXEOF
chmod +x ~/.cmux-kiro/set-env.sh"#;
        let _ = self.ssh_capture(script);
    }

    /// Fetch live sessions and merge with the config-declared order.
    pub fn fetch_sessions(&self, config_entries: &[(String, String)]) -> Vec<Session> {
        let rt = &self.rtmux;
        let probe = format!(
            "rm -f /tmp/cmux.sock 2>/dev/null\n\
             {rt} ls -F '#{{session_name}}|#{{session_created}}|#{{session_activity}}' 2>/dev/null\n\
             echo '==='\n\
             {rt} ls -F '#{{session_name}}|#{{pane_current_path}}' 2>/dev/null"
        );
        let raw = self.ssh_capture(&probe).unwrap_or_default();
        parse_and_merge(&raw, config_entries)
    }

    pub fn kill_session(&self, name: &str) {
        let cmd = format!("{} kill-session -t {}", self.rtmux, sh_quote(name));
        let _ = self.ssh_capture(&cmd);
    }

    /// Kill several sessions in a single ssh round-trip.
    pub fn kill_sessions(&self, names: &[String]) {
        if names.is_empty() {
            return;
        }
        let cmd = names
            .iter()
            .map(|n| format!("{} kill-session -t {} 2>/dev/null;", self.rtmux, sh_quote(n)))
            .collect::<String>();
        let _ = self.ssh_capture(&cmd);
    }

    pub fn rename_session(&self, old: &str, new: &str) {
        let cmd = format!(
            "{} rename-session -t {} {}",
            self.rtmux,
            sh_quote(old),
            sh_quote(new)
        );
        let _ = self.ssh_capture(&cmd);
    }

    /// The `-R` socket-forward args, if under cmux.
    fn cmux_ssh_extra(&self) -> Vec<String> {
        match &self.cmux {
            Some(c) => vec![
                "-o".into(),
                "StreamLocalBindUnlink=yes".into(),
                "-o".into(),
                "ControlPath=none".into(),
                "-R".into(),
                format!("{}:{}", c.remote_sock, c.local_sock),
            ],
            None => vec![],
        }
    }

    /// Build the full remote command string for an attach/new, wiring cmux env.
    fn build_remote_cmd(&self, action: &SessionAction) -> String {
        let mut prefix = String::new();
        let mut tmux_env = String::new();
        let mut per_session = String::new();

        if let Some(c) = &self.cmux {
            prefix = c.export_prefix();
            tmux_env = c.tmux_env(&self.rtmux);
            // For attach, set per-session vars via the helper (handles `/`).
            if let SessionAction::Attach { name } = action {
                per_session = format!(
                    "~/.cmux-kiro/set-env.sh {} CMUX_SOCKET_PATH '{}' CMUX_WORKSPACE_ID '{}' CMUX_TAB_ID '{}' CMUX_PANEL_ID '{}' CMUX_SURFACE_ID '{}';",
                    sh_quote(name),
                    c.remote_sock,
                    c.workspace_id,
                    c.tab_id,
                    c.panel_id,
                    c.surface_id
                );
            }
        }

        let core = action.tmux_cmd(&self.rtmux);
        format!("{prefix} {tmux_env} {per_session} {core}")
    }

    /// Run the interactive attach/new. Blocks until the session detaches or the
    /// user quits. Auto-reconnects (attach) if the ssh link drops unexpectedly.
    ///
    /// Returns after a clean detach (rc==0) or when the user Ctrl-C's the loop.
    pub fn run_interactive(&self, action: SessionAction) -> Result<()> {
        if self.use_mosh {
            let full = self.build_remote_cmd(&action);
            let err = Command::new("mosh")
                .arg(&self.host)
                .arg("--")
                .arg("bash")
                .arg("-lc")
                .arg(&full)
                .status();
            let _ = err;
            return Ok(());
        }

        let full = self.build_remote_cmd(&action);
        let reconnect_name = action.session_name().to_string();
        let reconnect_action = SessionAction::Attach {
            name: reconnect_name,
        };
        let reconnect_cmd = self.build_remote_cmd(&reconnect_action);

        // First attempt uses the original command (new or attach).
        let rc = self.ssh_tty(&full);
        if rc == Some(0) {
            return Ok(());
        }

        // Retry loop: always attach. Ctrl-C during the wait terminates the
        // process (terminal is already restored), which stops reconnecting.
        loop {
            eprint!(
                "\r\n  \x1b[33m⚡ Connection lost. Reconnecting in 3s...\x1b[0m (Ctrl+C to stop)\r\n"
            );
            std::thread::sleep(std::time::Duration::from_secs(3));
            let rc = self.ssh_tty(&reconnect_cmd);
            if rc == Some(0) {
                return Ok(());
            }
        }
    }

    /// ssh with a tty allocated; returns the child exit code.
    fn ssh_tty(&self, remote_cmd: &str) -> Option<i32> {
        let mut cmd = Command::new("ssh");
        cmd.args(&self.ssh_opts)
            .args(self.cmux_ssh_extra())
            .arg("-t")
            .arg(&self.host)
            .arg(remote_cmd);
        cmd.status().ok().and_then(|s| s.code())
    }
}

/// A session action to launch interactively.
pub enum SessionAction {
    Attach { name: String },
    New { name: String },
    NewInDir { name: String, dir: String },
}

impl SessionAction {
    fn session_name(&self) -> &str {
        match self {
            SessionAction::Attach { name }
            | SessionAction::New { name }
            | SessionAction::NewInDir { name, .. } => name,
        }
    }

    fn tmux_cmd(&self, rtmux: &str) -> String {
        match self {
            SessionAction::Attach { name } => {
                format!("{rtmux} attach -t {}", sh_quote(name))
            }
            SessionAction::New { name } => {
                format!("{rtmux} new -s {}", sh_quote(name))
            }
            SessionAction::NewInDir { name, dir } => {
                // cd into dir then exec the login shell.
                let inner = format!("cd {} && exec $SHELL", sh_quote(dir));
                format!("{rtmux} new -s {} {}", sh_quote(name), sh_quote(&inner))
            }
        }
    }
}

/// Parse the `ls` probe output and merge with config order.
fn parse_and_merge(raw: &str, config_entries: &[(String, String)]) -> Vec<Session> {
    let mut run_created: HashMap<String, String> = HashMap::new();
    let mut run_activity: HashMap<String, String> = HashMap::new();
    let mut run_activity_ts: HashMap<String, i64> = HashMap::new();
    let mut run_dirs: HashMap<String, String> = HashMap::new();
    let mut live_order: Vec<String> = Vec::new();

    let mut in_dirs = false;
    for line in raw.lines() {
        if line == "===" {
            in_dirs = true;
            continue;
        }
        if line.is_empty() {
            continue;
        }
        if !in_dirs {
            // name|created|activity
            let mut parts = line.splitn(3, '|');
            let (Some(name), Some(created), Some(activity)) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let cts: i64 = created.parse().unwrap_or(0);
            let ats: i64 = activity.parse().unwrap_or(0);
            run_created.insert(name.to_string(), fmt_ts(cts));
            run_activity.insert(name.to_string(), fmt_ts(ats));
            run_activity_ts.insert(name.to_string(), ats);
            if !live_order.contains(&name.to_string()) {
                live_order.push(name.to_string());
            }
        } else {
            let mut parts = line.splitn(2, '|');
            let (Some(name), Some(dir)) = (parts.next(), parts.next()) else {
                continue;
            };
            run_dirs.insert(name.to_string(), dir.to_string());
        }
    }

    let mut out: Vec<Session> = Vec::new();
    let mut seen: HashMap<String, ()> = HashMap::new();

    // Config-declared sessions first, in config order.
    for (name, cfg_dir) in config_entries {
        seen.insert(name.clone(), ());
        if run_created.contains_key(name) {
            out.push(Session {
                name: name.clone(),
                running: true,
                created: run_created.get(name).cloned().unwrap_or_default(),
                activity: run_activity.get(name).cloned().unwrap_or_default(),
                activity_ts: *run_activity_ts.get(name).unwrap_or(&0),
                dir: run_dirs.get(name).cloned().unwrap_or_default(),
            });
        } else {
            out.push(Session {
                name: name.clone(),
                running: false,
                created: String::new(),
                activity: String::new(),
                activity_ts: 0,
                dir: cfg_dir.clone(),
            });
        }
    }

    // Live sessions not in config, sorted by name (matches zsh ${(ko)}).
    let mut extra: Vec<String> = live_order
        .into_iter()
        .filter(|n| !seen.contains_key(n))
        .collect();
    extra.sort();
    for name in extra {
        out.push(Session {
            name: name.clone(),
            running: true,
            created: run_created.get(&name).cloned().unwrap_or_default(),
            activity: run_activity.get(&name).cloned().unwrap_or_default(),
            activity_ts: *run_activity_ts.get(&name).unwrap_or(&0),
            dir: run_dirs.get(&name).cloned().unwrap_or_default(),
        });
    }

    out
}

fn fmt_ts(ts: i64) -> String {
    if ts == 0 {
        return String::new();
    }
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(ts, 0).single() {
        Some(dt) => dt.format("%b %d %H:%M").to_string(),
        None => String::new(),
    }
}

/// Single-quote a string for POSIX sh, escaping embedded single quotes.
fn sh_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

fn is_socket(path: &str) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        std::fs::metadata(path)
            .map(|m| m.file_type().is_socket())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

fn env_or_empty(key: &str) -> String {
    std::env::var(key).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sh_quote_plain() {
        assert_eq!(sh_quote("replay/mainline"), "'replay/mainline'");
    }

    #[test]
    fn sh_quote_embedded_single_quote() {
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn tmux_cmd_attach_quotes_slash_names() {
        let a = SessionAction::Attach {
            name: "replay/mainline".into(),
        };
        assert_eq!(
            a.tmux_cmd("tmux"),
            "tmux attach -t 'replay/mainline'".to_string()
        );
    }

    #[test]
    fn tmux_cmd_new_in_dir() {
        let a = SessionAction::NewInDir {
            name: "proj/x".into(),
            dir: "/home/u/work".into(),
        };
        // Inner `cd '/home/u/work' && exec $SHELL` is itself sh-quoted, so the
        // dir's single quotes become the '\'' escape sequence.
        assert_eq!(
            a.tmux_cmd("tmux"),
            r#"tmux new -s 'proj/x' 'cd '\''/home/u/work'\'' && exec $SHELL'"#
        );
    }

    #[test]
    fn parse_merges_config_order_with_live_state() {
        // Two config sessions; one is live, one is not. Plus a live session
        // not present in config.
        let raw = "\
proj/a|1700000000|1700000100\n\
extra/z|1700000200|1700000300\n\
===\n\
proj/a|/home/u/a\n\
extra/z|/home/u/z\n";
        let cfg = vec![
            ("proj/a".to_string(), "/cfg/a".to_string()),
            ("proj/b".to_string(), "/cfg/b".to_string()),
        ];
        let sessions = parse_and_merge(raw, &cfg);

        // Config order first: proj/a (running), proj/b (not), then extra/z.
        assert_eq!(sessions.len(), 3);
        assert_eq!(sessions[0].name, "proj/a");
        assert!(sessions[0].running);
        assert_eq!(sessions[0].dir, "/home/u/a"); // live dir wins

        assert_eq!(sessions[1].name, "proj/b");
        assert!(!sessions[1].running);
        assert_eq!(sessions[1].dir, "/cfg/b"); // config dir for offline

        assert_eq!(sessions[2].name, "extra/z");
        assert!(sessions[2].running);
    }

    #[test]
    fn parse_handles_empty() {
        let sessions = parse_and_merge("===\n", &[]);
        assert!(sessions.is_empty());
    }
}

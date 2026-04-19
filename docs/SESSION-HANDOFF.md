# Terminal Tools — Session Handoff Notes

## Repo
- **GitHub**: https://github.com/sunchitanand/terminal-tools
- **Local**: ~/Documents/projects/terminal-tools (alias `tt`)
- **Scripts symlinked to**: ~/.local/bin/

## What We Built

### 1. tmux-selector (`tmux-selector`)
TUI for managing remote tmux sessions over SSH. Compact table layout with fuzzy search, project grouping, session create/rename/delete.

**Aliases**: `ccc` (clouddesk2), `c2` (agenthost), `c3` (ml)

**Status**: ✅ Working — table layout, fuzzy search, viewport scrolling, mosh support (`--mosh` flag)

### 2. kiro-selector (`kiro-selector`)
TUI for managing local kiro-cli sessions. Reads from kiro-cli SQLite database, shows sessions grouped by project with last topic, chat count, timestamps.

**Status**: ✅ Working

### 3. cmux-tunnel (`cmux-tunnel`)
Persistent SSH tunnel for cmux socket forwarding. **DEPRECATED** — replaced by inline socket forwarding in tmux-selector.

### 4. Claude Code cmux hooks (`claude-cmux/`)
Hooks in `~/.claude/settings.json` that update cmux sidebar for Claude Code sessions.

**Status**: ✅ Working locally. Remote needs testing.

## Current Active Issue: cmux Sidebar for Remote tmux Sessions

### The Problem
When connecting to cloud desktops via `c3`/`c2`/`ccc` (tmux-selector), the cmux sidebar should update with kiro/claude session info. This requires:
1. Socket forwarding (`-R /tmp/cmux.sock:<local_socket>`)
2. CMUX env vars set in the remote tmux session environment
3. A precmd hook on the remote to refresh env vars into running shells

### What Works
- Socket forwarding via SSH `-R` in `run_remote()` ✅
- `CMUX_PREFIX` exports env vars into the SSH shell ✅
- `cmux ping` returns PONG on remote ✅
- `cmux notify` works from remote ✅
- `cmux set-status` works from remote ✅
- precmd hook on remote refreshes CMUX vars from tmux env ✅

### What's Broken
- **`tmux set-environment -t <session>` is failing silently** when called from the SSH command in `run_remote()`. The session name (e.g. `replay/mainline`) contains `/` which causes quoting issues across the SSH boundary.
- When run manually on the remote, `tmux set-environment -t "replay/mainline" CMUX_WORKSPACE_ID 'value'` works fine.
- The issue is shell escaping: the command is built as a string in zsh, passed to `ssh -t $HOST "..."`, and the quotes get mangled.
- Latest attempt: removed all quoting (`-t $sess`). Untested.

### Root Cause
The `run_remote()` function builds a command string that gets passed through two shells (local zsh → ssh → remote bash). Session names with `/` need proper escaping through both layers. The `2>/dev/null` on each command suppresses errors, making debugging hard.

### Suggested Fix
1. Remove `2>/dev/null` temporarily to see actual errors
2. Test the exact command string that SSH receives by echoing instead of exec'ing
3. Consider using a helper script on the remote instead of inline commands:
   ```bash
   # Remote: ~/.cmux-kiro/set-env.sh <session> <ws_id> <tab_id> <panel_id> <surface_id>
   tmux set-environment -t "$1" CMUX_WORKSPACE_ID "$2"
   tmux set-environment -t "$1" CMUX_TAB_ID "$3"
   # etc.
   ```
   Then call: `ssh ... "$HOST" "~/.cmux-kiro/set-env.sh '$sess' '$ws' '$tab' '$panel' '$surface'; tmux attach ..."`

### Architecture Overview
```
Mac (cmux) → SSH -R /tmp/cmux.sock → Cloud Desktop (tmux)
                                        ├── tmux session A (CMUX_WORKSPACE_ID=X)
                                        │   ├── pane 1 (kiro) → hooks → cmux socket → sidebar tab X
                                        │   └── pane 2 (claude) → hooks → cmux socket → sidebar tab X
                                        └── tmux session B (CMUX_WORKSPACE_ID=Y)
                                            └── pane 1 (kiro) → hooks → cmux socket → sidebar tab Y
```

Each cmux workspace → one SSH connection → one tmux session → one set of CMUX IDs.
The precmd hook in remote .zshrc refreshes CMUX vars from tmux session env on every prompt.

## Other Completed Work

### Ghostty Config (`~/.config/ghostty/config`)
- Monaco font, iTerm2 Smoooooth theme, font-thicken, white foreground
- Split dimming, window-save-state, macos-option-as-alt, copy-on-select
- Quick terminal (Option+Space)
- scrollback-limit reduced to 1MB

### tmux Optimizations (all cloud desktops)
- `escape-time 0` — removes 500ms Esc delay
- `extended-keys on` + `extended-keys-format csi-u` — Shift+Enter works
- `smcup@:rmcup@` terminal override — Ghostty native scrollback
- tmux upgraded to 3.6a on c2 and c3
- Ghostty terminfo installed on all hosts

### Mosh
- Installed on all 3 cloud desktops (1.3.2 on ccc/AL2, 1.4.0 on c2+c3/AL2023)
- `--mosh` flag in tmux-selector
- Not used by default (breaks Ghostty scrollbar)

### kiro-cmux (AmznCmuxKiroTools)
- Installed locally and on all cloud desktops
- Hooks injected into all kiro agents
- `k` alias uses `--agent cmux`
- precmd hook on remotes: `cmux_refresh_env()` in .zshrc

### cmux Settings (`~/.config/cmux/settings.json`)
- `openTerminalLinksInCmuxBrowser: true`
- `interceptTerminalOpenCommandInCmuxBrowser: true`

### SSH Config
- ControlMaster/ControlPath/ControlPersist on cloud desktop hosts
- Note: ControlMaster conflicts with socket forwarding, so `run_remote()` uses `-o ControlPath=none`

## Files Modified Outside Repo
- `~/.zshrc` — aliases, precmd hooks removed (tunnel auto-start), k/kr aliases
- `~/.config/ghostty/config` — terminal settings
- `~/.config/cmux/settings.json` — cmux app settings
- `~/.claude/settings.json` — Claude Code hooks
- `~/.claude/cmux-title.sh` — dynamic workspace title script
- Remote `~/.tmux.conf` — escape-time, extended-keys, smcup/rmcup
- Remote `~/.zshrc` — CMUX_SOCKET_PATH export, precmd hook
- Remote `~/.claude/settings.json` — Claude Code hooks
- Remote `~/.kiro/agents/*.json` — cmux hooks injected

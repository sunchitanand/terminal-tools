# cmux + Kiro Integration with tmux-selector

## How it works

When you run `c3`/`c2`/`ccc` from inside cmux, the tmux-selector script:
1. Detects cmux socket (from `CMUX_SOCKET_PATH` or default location)
2. Adds `-R /tmp/cmux.sock:<local-socket>` to SSH opts (reverse-forwards the socket)
3. Passes all CMUX env vars (`WORKSPACE_ID`, `TAB_ID`, `PANEL_ID`, `SURFACE_ID`) to the remote shell
4. Sets per-session tmux environment so new panes inherit the vars
5. A `precmd` hook on the remote auto-refreshes CMUX vars from tmux env on every prompt

Kiro sessions inside tmux talk to cmux via the forwarded socket → sidebar updates, titles, notifications.

When run outside cmux (e.g. iTerm2), the script works normally without sidebar integration.

## One-time setup

### 1. Install kiro-cmux locally (on Mac)

```bash
[[ "$(curl -s -b ~/.midway/cookie https://midway-auth.amazon.com/api/session-status 2>/dev/null | jq -r .authenticated)" == "true" ]] || mwinit
git clone ssh://git.amazon.com/pkg/AmznCmuxKiroTools ~/.cmux-kiro
~/.cmux-kiro/setup.sh
```

Select all agents when prompted. This injects cmux hooks into all your kiro agent configs.

### 2. Setup each remote host

```bash
# From inside cmux on your Mac:
kmux setup-remote dev-dsk-sunchit-2c-da3c3b2e.us-west-2.amazon.com
kmux setup-remote dev-dsk-sunchit-2a-0fd3f6d1.us-west-2.amazon.com
kmux setup-remote sunchit-cd2.aka.corp.amazon.com
```

Then SSH into each host and run `~/.cmux-kiro/setup.sh` to inject hooks into remote agents (the interactive picker needs a real terminal).

### 3. Verify hooks are injected

```bash
# On remote:
grep -c "cmux" ~/.kiro/agents/AmazonBuilderCoreAIAgents-amzn-builder.json
# Should be > 0
```

## Daily usage

```bash
# From cmux terminal on Mac:
c3                    # tmux-selector, pick a session
# Inside remote tmux:
k                     # start kiro — sidebar updates automatically
```

New tmux panes inherit CMUX vars automatically. Existing panes refresh vars on next prompt (via precmd hook).

## How multiple sessions work

- Each cmux workspace has its own `WORKSPACE_ID`, `TAB_ID`, `PANEL_ID`
- When you `c3` from workspace A → session X gets workspace A's IDs
- When you `c3` from workspace B → session Y gets workspace B's IDs
- IDs are set per-tmux-session (not global), so they don't overwrite each other
- The `precmd` hook refreshes vars from tmux session env on every prompt

**Caveat**: All panes within the same tmux session share one set of IDs → one sidebar entry. For separate sidebar tracking per kiro session, use separate cmux workspaces.

## Verify it works

On the remote, inside tmux:
```bash
echo "WS=$CMUX_WORKSPACE_ID TAB=$CMUX_TAB_ID PANEL=$CMUX_PANEL_ID"
~/bin/cmux ping                    # should say PONG
~/bin/cmux notify --title "test" --body "hello"   # should appear in cmux sidebar
```

## Troubleshooting

| Problem | Fix |
|---|---|
| `cmux ping` → "Connection refused" | Detach tmux, reconnect via `c3` — socket forward needs fresh SSH connection |
| `cmux ping` → "No such file or directory" | Socket not forwarded — make sure you ran `c3` from cmux (not iTerm2) |
| `CMUX_TAB_ID` empty | Open a new tmux pane (`Ctrl+B %`) or run `source ~/.zshrc` |
| Sidebar updates go to wrong workspace | The tmux session has stale IDs — detach and reconnect via `c3` from the correct cmux workspace |
| Hooks not firing | Check agent has hooks: `grep -c "cmux" ~/.kiro/agents/<agent>.json`. If 0, run `~/.cmux-kiro/setup.sh` |
| `amzn-builder` has no hooks | Run `~/.cmux-kiro/setup.sh` and select it in the picker |
| Local kiro sidebar not working | Same fix — run `~/.cmux-kiro/setup.sh` locally, select all agents |

## Technical details

### tmux-selector changes

In `run_remote()`, when inside cmux:
- SSH gets `-o StreamLocalBindUnlink=yes -o ControlPath=none -R /tmp/cmux.sock:<local_socket>`
- `CMUX_PREFIX` exports all env vars into the remote shell
- Per-session `tmux set-environment -t '=<session>'` sets vars for new panes
- `update-environment` config tells tmux to refresh on reattach

### Remote precmd hook (in ~/.zshrc)

```bash
cmux_refresh_env() { eval $(PATH=/usr/local/bin:$PATH tmux show-environment -s 2>/dev/null | grep CMUX) 2>/dev/null; }
precmd_functions+=(cmux_refresh_env)
```

Auto-refreshes CMUX vars from tmux session environment on every prompt.

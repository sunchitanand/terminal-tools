# cmux Sidebar Integration with tmux-selector

## How it works

When you run `c3`/`c2`/`ccc` from inside cmux, the tmux-selector script:
1. Detects `CMUX_SOCKET_PATH` env var (set by cmux)
2. Creates a local symlink to the cmux socket (workaround for spaces in path)
3. Adds `-R /tmp/cmux-fwd.sock:<local-socket>` to SSH opts (reverse-forwards the socket)
4. During session fetch, sets `CMUX_SOCKET_PATH=/tmp/cmux-fwd.sock` in tmux's global environment
5. Attaches to your chosen tmux session with the socket forwarded

Kiro sessions inside tmux can then talk to cmux via the forwarded socket → sidebar updates, titles, notifications.

## One-time setup per remote host

```bash
# From inside cmux on your Mac:
kmux setup-remote dev-dsk-sunchit-2c-da3c3b2e.us-west-2.amazon.com
kmux setup-remote dev-dsk-sunchit-2a-0fd3f6d1.us-west-2.amazon.com
kmux setup-remote sunchit-cd2.aka.corp.amazon.com
```

This installs:
- cmux/nc shims in `~/bin/` on the remote
- kiro-cmux hooks into all agent configs
- kask (warm ACP client for fast AI titles/notifications)
- CR detection

## Daily usage

```bash
# From cmux terminal on Mac:
c3                          # opens tmux-selector, pick a session
# Inside the remote tmux session:
export CMUX_SOCKET_PATH=/tmp/cmux-fwd.sock   # needed for existing sessions
k                           # start kiro with sidebar integration
```

New tmux panes/windows automatically inherit `CMUX_SOCKET_PATH` — no export needed.

## For existing sessions (first time after connecting)

Existing shells don't inherit the env var. Run once per pane:
```bash
export CMUX_SOCKET_PATH=/tmp/cmux-fwd.sock
```

Then restart kiro (`k` or `kiro-cli chat --resume`).

Or broadcast to all panes at once:
```bash
for pane in $(PATH=/usr/local/bin:$PATH tmux list-panes -a -F '#{pane_id}'); do
    PATH=/usr/local/bin:$PATH tmux send-keys -t "$pane" "export CMUX_SOCKET_PATH=/tmp/cmux-fwd.sock" Enter
done
```

## Verify it works

```bash
# On the remote, inside tmux:
~/bin/cmux ping              # should say PONG
echo $CMUX_SOCKET_PATH       # should say /tmp/cmux-fwd.sock
```

## Troubleshooting

| Problem | Fix |
|---|---|
| `cmux ping` says "Connection refused" | Detach tmux, exit SSH, reconnect via `c3` — socket forward needs a fresh connection |
| `CMUX_SOCKET_PATH` is empty | Run `export CMUX_SOCKET_PATH=/tmp/cmux-fwd.sock` or open a new tmux pane |
| `/tmp/cmux-fwd.sock` doesn't exist | You connected from outside cmux (iTerm2 etc.) — reconnect from cmux |
| Sidebar not updating after starting kiro | Stop kiro, verify `cmux ping` works, restart kiro |
| `setup-remote` says "All agents already have cmux hooks" but they don't | SSH into the host and run `~/.cmux-kiro/setup.sh` manually, select all agents |

## What the tmux-selector script does (technical)

In `tmux-selector`, at the top:
```bash
# If inside cmux, add socket forwarding to all SSH connections
if [[ -n "${CMUX_SOCKET_PATH:-}" && -S "${CMUX_SOCKET_PATH:-}" ]]; then
    CMUX_REMOTE_SOCK="/tmp/cmux-fwd.sock"
    CMUX_LOCAL_SOCK="/tmp/cmux-local-$$.sock"
    ln -sf "${CMUX_SOCKET_PATH}" "$CMUX_LOCAL_SOCK"
    SSH_OPTS+=(-o StreamLocalBindUnlink=yes -R "${CMUX_REMOTE_SOCK}:${CMUX_LOCAL_SOCK}")
fi
```

During fetch, injects env into tmux:
```bash
tmux set-environment -g CMUX_SOCKET_PATH '/tmp/cmux-fwd.sock'
tmux set-environment -g CMUX_WORKSPACE_ID '...'
tmux set-option -g update-environment 'CMUX_SOCKET_PATH CMUX_WORKSPACE_ID ...'
```

During attach, disables ControlMaster so the `-R` forward works:
```bash
ssh -o ControlPath=none $SSH_OPTS -t $HOST "tmux attach ..."
```

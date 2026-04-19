# Claude Code + cmux Sidebar Integration

## What we set up

Claude Code hooks in `~/.claude/settings.json` that call cmux commands to update the sidebar.

### Hooks

| Event | Sidebar Action |
|---|---|
| SessionStart | Orange "Claude ready" status pill |
| UserPromptSubmit | Renames workspace to prompt text (first 50 chars), orange "Working" pill, clears notifications |
| Stop | Green "Done" pill + desktop notification |
| Notification | Purple "Waiting" pill + desktop notification |

### Files

- `~/.claude/settings.json` — hooks config (merged with existing settings)
- `~/.claude/cmux-title.sh` — script that reads prompt from stdin JSON and renames workspace

## Setup / Reproduce

### 1. Add hooks to Claude settings

Merge the hooks from `hooks.json` into `~/.claude/settings.json`:

```bash
# View the hooks to add:
cat ~/Documents/projects/terminal-tools/claude-cmux/hooks.json
```

Or run:
```python
python3 << 'EOF'
import json

with open('/Users/sunchit/.claude/settings.json') as f:
    d = json.load(f)

with open('/Users/sunchit/Documents/projects/terminal-tools/claude-cmux/hooks.json') as f:
    hooks = json.load(f)

d['hooks'] = hooks

with open('/Users/sunchit/.claude/settings.json', 'w') as f:
    json.dump(d, f, indent=2)

print("Hooks merged")
EOF
```

### 2. Install the title script

```bash
cp ~/Documents/projects/terminal-tools/claude-cmux/cmux-title.sh ~/.claude/cmux-title.sh
chmod +x ~/.claude/cmux-title.sh
```

### 3. Restart Claude Code

New sessions will have sidebar integration.

## Remote sessions

The same socket forwarding from our tmux-selector handles Claude Code on remote hosts. As long as `CMUX_SOCKET_PATH` is set (via the precmd hook) and the socket is forwarded (via `c3`/`c2`/`ccc`), Claude Code sidebar works remotely too.

For remote hosts, copy the hooks and script:
```bash
scp ~/.claude/cmux-title.sh <host>:~/.claude/
# Then merge hooks into remote ~/.claude/settings.json
```

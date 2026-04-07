# terminal-tools

A collection of TUI scripts for terminal power users.

## Scripts

### tmux-selector

A fuzzy TUI for managing tmux sessions on remote hosts over SSH. Sessions are grouped by project, sorted alphabetically, with sessions sorted by most recently active. Supports fuzzy search, rename, delete, and new session creation.

**Features:**
- Project-grouped session list with live `●/○` running indicators
- Started and last active timestamps
- Fuzzy search (type anywhere)
- Viewport scrolling for long lists
- Remembers session directories across restarts (local JSON cache)

**Dependencies:** `zsh`, `tmux`, `ssh`, `python3`

**Usage:**
```bash
tmux-selector [host]
# default host: sunchit-cd2.aka.corp.amazon.com
```

**Install:**
```bash
cp tmux-selector ~/.local/bin/tmux-selector
chmod +x ~/.local/bin/tmux-selector
# add alias to ~/.zshrc:
alias ccc='tmux-selector user@your-host.com'
```

**Keybinds:**
| Key | Action |
|-----|--------|
| `↑↓` | Navigate |
| `Enter` | Attach / create session |
| `Space` | Rename session |
| `d` | Delete session |
| Type anything | Fuzzy search |
| `Esc` | Clear search |
| `q` | Quit |

---

### kiro-selector

A TUI for managing [kiro-cli](https://kiro.dev) sessions by directory. Reads session history from kiro-cli's SQLite database and shows sessions grouped by project with last topic, chat count, and last updated time.

**Features:**
- Project-grouped directory list
- Shows last conversation topic, chat count, last active time
- Live `●` indicator for directories with running kiro-cli processes
- Fuzzy search across directory path and last topic
- Open new session or resume existing

**Dependencies:** `zsh`, `python3`, `sqlite3`, `kiro-cli`

**Usage:**
```bash
kiro-selector
```

**Install:**
```bash
cp kiro-selector ~/.local/bin/kiro-selector
chmod +x ~/.local/bin/kiro-selector
alias k='kiro-selector'
```

**Keybinds:**
| Key | Action |
|-----|--------|
| `↑↓` | Navigate |
| `Enter` | Open new kiro session in directory |
| `r` | Resume last session |
| `R` | Resume with session picker |
| `d` | Delete all sessions for directory |
| Type anything | Fuzzy search |
| `Esc` | Clear search |
| `q` | Quit |

## License

MIT

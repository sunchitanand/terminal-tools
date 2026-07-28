# tmux-selector (Rust)

Rust/ratatui rewrite of the zsh `tmux-selector` TUI. Feature-parity port — the
old script stays in place and keeps working until you flip your alias over.

## Why

- Flicker-free rendering via a real double-buffered TUI (ratatui).
- Auto-scrolling viewport — cursor never runs off-screen with many sessions.
- Same on-disk config (`~/.tmux-projects.json`) — fully interchangeable with
  the zsh version, including slash-named sessions and multi-host layout.
- Same cmux socket-forwarding + per-session env helper for sidebar updates.
- Same SSH auto-reconnect loop after laptop sleep / link drop.

## Build

```sh
cd tmux-selector-rs
cargo build --release
# binary: target/release/tmux-selector
```

## Run (without changing your alias)

```sh
./target/release/tmux-selector [host] [--mosh]
# default host: sunchit-cd2.aka.corp.amazon.com
```

Debug: `--list` fetches and prints sessions, no TUI (used for smoke testing).

## Keys

| Key        | Action                                  |
|------------|-----------------------------------------|
| type       | live fuzzy search                       |
| ↑ / ↓      | move cursor (skips non-matches)         |
| Tab / ⇧Tab | cycle action: attach → rename → delete  |
| Enter      | run current action (Attach by default)  |
| Space      | pick / unpick for bulk delete           |
| Esc        | staged clear: search → selection → action |
| q          | quit (when not searching)               |
| Ctrl-C     | quit                                    |

Enter always attaches (or creates) unless Tab has cycled the action.

## Switching over

When you're happy, point your shell alias at the release binary. Nothing else
changes — the config file and remote behavior are identical.

## Tests

```sh
cargo test    # config round-trip, fuzzy match, grouping, nav, ssh parse/quote
```

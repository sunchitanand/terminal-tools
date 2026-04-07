# Terminal Tools — Steering Doc

## Repo Info
- **Repo**: https://github.com/sunchitanand/terminal-tools
- **Local path**: ~/Documents/projects/terminal-tools
- **Scripts live at**: ~/.local/bin (symlinked from repo)

## Adding a New Script

```bash
cd ~/Documents/projects/terminal-tools

# Copy or create the script
cp ~/.local/bin/my-new-script .
chmod +x my-new-script

# Symlink so it's live
ln -sf ~/Documents/projects/terminal-tools/my-new-script ~/.local/bin/my-new-script

# Commit and push
git add .
git commit -m "feat: add my-new-script"
git push
```

## Backing Up an Existing Script

```bash
cd ~/Documents/projects/terminal-tools
cp ~/.local/bin/script-name .
chmod +x script-name
ln -sf ~/Documents/projects/terminal-tools/script-name ~/.local/bin/script-name
git add .
git commit -m "feat: add script-name"
git push
```

## Syncing Changes After Editing

Scripts are symlinked, so edits in ~/.local/bin are already in the repo. Just commit:

```bash
cd ~/Documents/projects/terminal-tools
git add -A
git commit -m "fix: description of change"
git push
```

## Current Scripts

| Script | Symlink | Purpose |
|--------|---------|---------|
| tmux-selector | ~/.local/bin/tmux-selector | TUI for remote tmux sessions over SSH |
| kiro-selector | ~/.local/bin/kiro-selector | TUI for kiro-cli sessions by directory |

## Conventions
- All scripts are zsh (`#!/usr/bin/env zsh`)
- Scripts must be `chmod +x`
- Always symlink from repo to ~/.local/bin
- Update README.md when adding new scripts
- Commit messages follow conventional commits (feat/fix/docs/refactor)

## Dependencies
- zsh, python3, sqlite3 (all pre-installed on macOS)
- tmux (for tmux-selector, on remote hosts)
- kiro-cli (for kiro-selector)

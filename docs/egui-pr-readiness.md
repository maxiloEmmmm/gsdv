# Egui Rewrite PR Readiness

## Scope

This PR replaces the legacy Ratatui application shell with an egui/eframe desktop application.

Major changes:

- Adds `src/bin/gsdv-gui.rs` as the native GUI entrypoint.
- Adds `src/gui/` for the egui shell, design tokens, workspace data loading, reviewer adapter, and terminal host.
- Removes legacy TUI files: `src/main.rs`, `src/markdown_editor.rs`, `src/markdown_view.rs`, and `src/terminal_panel.rs`.
- Replaces direct TUI/PTY dependencies with `eframe`, `egui_commonmark`, direct `alacritty_terminal`, and `rfd`.
- Keeps reviewer domain logic under `src/reviewer/`, exposed to the GUI through snapshot/action APIs.

## User-Facing Behavior

Implemented:

- Multi-workspace rail with add, switch, close, and persisted active workspace.
- Zero-workspace empty state.
- Workspace outline with filter, context menus, create markdown, create folder, rename, delete, copy paths, reveal, and refresh.
- Agent-first center surface with embedded terminal-backed agent.
- Workspace-scoped terminal surface backed by direct `alacritty_terminal`.
- Markdown editor with dirty tracking, save, blocking dirty-file switch, and improved header/footer chrome.
- Markdown preview rendering unsaved editor content with stats footer.
- Fullscreen reviewer route with GSD/Git mode, four-column layout, filters, branch dialogs, dirty repo blocking, diff/full toggle, and keyboard navigation.
- Toast feedback for low-risk actions.
- Help and Settings dialogs.
- Persisted workspace state: workspace list, active workspace, rail collapse,
  agent kind/id/session metadata, Markdown outline collapse, favorites, and
  workspace sidecars. Older selected-file/center/reviewer fields may still be
  read for compatibility, but new saves do not rely on them.

## Keyboard Coverage

- `Cmd/Ctrl+1` Agent
- `Cmd/Ctrl+2` Terminal
- `Cmd/Ctrl+3` Editor
- `Cmd/Ctrl+4` Preview
- `Cmd/Ctrl+S` Save
- `Cmd/Ctrl+O` Add workspace
- `Cmd/Ctrl+H` Help
- `Cmd/Ctrl+,` Settings
- `Cmd/Ctrl+Left/Right` Switch workspace
- `Cmd/Ctrl+Enter` Open reviewer
- Reviewer: arrows, `Tab`, `Shift+Tab`, `R`, `F`, `B`, `Esc`

## Verification

Passing:

```sh
cargo fmt --check
cargo check
cargo test
```

Current test count:

- 31 unit tests passing.

Startup check:

```sh
cargo run --bin gsdv-gui
```

Observed:

- Native GUI binary compiles.
- App process starts without startup panic or missing-library error.

## Known Remaining Work

Requires manual visual confirmation:

- Final screenshot/recording pass against `docs/design/agent.png`, `docs/design/md.png`, and `docs/design/reviewer.png`.
- Pixel-level adjustments after reviewing actual native-window screenshots.

Known visual deltas are tracked in `docs/egui-design-delta.md`.

Manual QA checklist is tracked in `docs/egui-manual-qa.md`.

# Egui Manual QA

## Startup Check

Command:

```sh
cargo run --bin gsdv-gui
```

Observed result:

- Binary compiles.
- Native app process starts with `target/debug/gsdv-gui`.
- No startup panic or missing-library error was observed before stopping the process.

## Automated Smoke Check

Fixture:

- Temporary `GSDV_STORE_PATH`
- One temporary workspace
- One markdown file at `docs/release-plan.md`
- Stored center mode starts at `agent`

Automated actions sent through macOS System Events:

- `Cmd+3` editor
- `Cmd+S` save
- `Cmd+4` preview
- `Cmd+Enter` reviewer
- `Esc` return
- `Cmd+2` workspace terminal
- `Cmd+H` help
- `Esc` close help
- `Cmd+,` settings
- `Esc` close settings

Observed result:

- The app process stayed alive after the shortcut sequence.
- The process loaded AppKit/SystemAppearance resources.
- The process opened a PTY device, indicating the embedded terminal path initialized.

Automation limitation:

- `screencapture` failed in the current automation session with a display capture error.
- System Events saw the process but did not expose window metadata, so pixel-level visual assertions still require a user-side manual screenshot pass.

## Manual Visual Checklist

Use this checklist during screenshot/recording capture.

### Agent

- Workspace rail is visible and active workspace state is readable.
- Outline panel is narrow and scannable.
- Agent is the default center surface.
- Agent status card shows workspace, root, session, and state.
- Embedded terminal appears inside the Agent surface card.
- `Open Reviewer`, `Open Current Doc`, `Resume Session`, and `Open Terminal` are visible and functional.

### Workspace Terminal

- `Cmd/Ctrl+2` switches to Terminal.
- Terminal cwd is the active workspace root.
- Terminal failure state shows a card with Retry and a concrete error message.

### Markdown Editor

- `Cmd/Ctrl+3` switches to Editor.
- Header shows numbered label, title, file path, dirty badge when applicable, and Save action.
- `Cmd/Ctrl+S` saves and shows a toast.
- Empty/error states render as cards or error strips.

### Markdown Preview

- `Cmd/Ctrl+4` switches to Preview.
- Preview renders unsaved editor content.
- Header shows file path and unsaved-content badge when applicable.
- Footer shows word count, line count, reading time, and updated state.

### Reviewer

- `Cmd/Ctrl+Enter` opens Reviewer.
- GSD/Git toggle is visible.
- Four-column structure keeps the diff/full-file column dominant.
- Arrow keys and Tab navigate reviewer columns/rows.
- `R` reloads, `F` toggles diff/full, `B` opens branch dialog, `Esc` exits.

### Workspace And Dialogs

- Closing the last workspace shows the zero-workspace empty state instead of quitting.
- Help opens with `Cmd/Ctrl+H`.
- Settings opens with `Cmd/Ctrl+,`.
- Right-click outline menus support create, rename, delete, copy, reveal, and refresh.
- Toasts appear in the lower-right and expire automatically.

## Known Remaining Visual Deltas

See `docs/egui-design-delta.md`.

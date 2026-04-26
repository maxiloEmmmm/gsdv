# Reset Plan: Remove tmux and Rebuild as Single-Process Ratatui

## Goal

Replace the current tmux-driven multi-process architecture with one `gsdv` process that owns all UI state and renders every panel through Ratatui.

The replacement should keep the current product behavior:

- left outline / slide tree
- markdown preview
- editor/content terminal panel
- reviewer in GSD mode
- reviewer in git mode
- global-ish shortcuts such as `Ctrl+F1`, `Ctrl+F2`, `Ctrl+F3`

But it should remove these implementation dependencies:

- tmux windows
- tmux panes
- tmux key bindings
- pane titles as state
- temporary tmux windows
- repeated `gsdv` subcommands for panel switching
- temp files as primary UI state

## Why Reset

Most recent bugs came from tmux orchestration, not from core UI logic.

Observed failure classes:

- Stale tmux global key bindings pointed at an old project directory.
- `Ctrl+F1` depended on pane titles and temp-window parking state.
- Bad states like "two view panes and no editor pane" were possible.
- `editor-return` could restore an empty parked pane into the content area.
- `reviewer` and `markdown-view` required separate `gsdv` child processes.
- tmux session options, pane cwd, pane title, and temp state files could disagree.

These are structural issues. More patches will keep adding recovery paths rather than eliminating the failure mode.

## Target Architecture

One process:

```text
gsdv
  AppState
  EventLoop
  Layout
  Panels
    OutlinePanel
    OverviewPanel
    MarkdownPanel
    ReviewerPanel
    TerminalPanel(content/codex/editor/shell)
```

Ratatui owns the full screen layout.

`tui-term` owns terminal emulation inside terminal panels, following the `smux` example style:

```text
TerminalPanel
  portable-pty child process
  vt100 Parser
  tui_term::widget::PseudoTerminal render
  input encoder
  resize bridge
```

No tmux is involved.

## What Uses Native Ratatui

These panels should remain native Ratatui views:

- outline / slide tree
- overview / progress dashboard
- reviewer GSD mode
- reviewer git mode
- markdown preview
- help overlay
- command/status line

These are deterministic UI components and should not run as subprocesses.

## What Uses tui-term

Only interactive child processes should use `tui-term`:

- `codex`
- shell
- vim/nvim if we keep external editor behavior

Use the `tui-term` `smux` example as the baseline, not the simpler `ls` examples. The `smux` example is closer to what we need because it handles:

- long-running PTY processes
- keyboard forwarding
- resize
- redraw signaling
- multiple panes as a concept

## Proposed Module Layout

```text
src/
  app/
    mod.rs
    state.rs
    event_loop.rs
    layout.rs
    commands.rs
  panels/
    mod.rs
    outline.rs
    overview.rs
    markdown.rs
    reviewer.rs
    terminal.rs
    help.rs
  terminal/
    mod.rs
    pane.rs
    pty.rs
    input.rs
    render.rs
  reviewer/
    app.rs
    diff.rs
    git.rs
    provenance.rs
```

Notes:

- Existing `src/reviewer/*` can mostly survive, but should stop owning terminal lifecycle.
- Existing `src/markdown_view.rs` should become `panels/markdown.rs`.
- tmux-specific code in `src/main.rs` should be deleted, not moved.
- `main.rs` should become thin: parse args, initialize `App`, run event loop.

## App State

Central state should replace tmux pane state:

```rust
struct AppState {
    project_dir: PathBuf,
    focused_panel: PanelId,
    layout_mode: LayoutMode,
    outline: OutlineState,
    overview: OverviewState,
    markdown: MarkdownState,
    reviewer: ReviewerState,
    terminal: TerminalPanelState,
}
```

Important invariants:

- There is exactly one source of truth for current file.
- There is exactly one source of truth for current view mode.
- There is exactly one source of truth for current reviewer mode.
- No pane title or temp file should be required to decide what the UI is showing.

## Replacement for Current Modes

### Current tmux "content"

Replace with a `TerminalPanel` running `codex` or shell.

The current `content_panel_command()` becomes `TerminalPanel::spawn_codex(args)`.

### Current tmux "editor"

Two possible approaches:

1. Keep external vim/nvim inside `TerminalPanel`.
2. Later replace with native editor widget.

For this reset, use option 1. It is less scope.

### Current tmux "view"

Replace with native `MarkdownPanel`.

`Ctrl+F1` toggles:

```text
TerminalPanel(editor) <-> MarkdownPanel(current_file)
```

No subprocess `markdown-view`.
No `toggle-markdown-view` subcommand.
No `editor-return` subcommand.

### Reviewer

Reviewer becomes a native panel/view inside the same app.

`Ctrl+F3` should switch to reviewer git mode.

Clicking a phase should switch to reviewer GSD mode with that phase.

No reviewer tmux window.
No `reviewer` subcommand required for normal use.

## Shortcuts

Proposed global keys:

- `Ctrl+F1`: toggle current file editor/markdown preview
- `Ctrl+F2`: show help overlay
- `Ctrl+F3`: open reviewer in git mode
- `Esc`: close overlay or return focus to previous panel
- `Tab` / `Shift+Tab`: move focus between panels

Panel-local keys:

- Outline keeps `j/k`, arrows, enter, space expand/collapse.
- Reviewer keeps existing navigation.
- TerminalPanel forwards almost everything to child process unless wrapper key is reserved.

## Terminal Panel Design

Use a structure modeled after `tui-term`'s `smux` example:

```rust
struct TerminalPanel {
    parser: Arc<RwLock<vt100::Parser>>,
    writer: Sender<Bytes>,
    master: Box<dyn MasterPty>,
    child: Box<dyn Child>,
    title: String,
    focused: bool,
    last_area: Rect,
}
```

Responsibilities:

- spawn child process with PTY
- read PTY output on background thread
- feed bytes to `vt100::Parser`
- send terminal query replies if required by parser/library
- write encoded key input to PTY
- resize PTY and parser when panel area changes
- render `PseudoTerminal` into Ratatui frame

Input encoding should start from the `tui-term` `smux` example, then be extended for:

- Ctrl keys
- Alt keys
- Enter/backspace/delete
- arrows/home/end/page
- paste
- bracketed paste if needed
- function keys if needed

## Risk: Codex Compatibility

We already know simple `tui-term`/`ratkit` spikes can have issues with Codex terminal queries and color assumptions.

The reset should still use `tui-term smux` as the first implementation because it is the closest ratatui-native path. But it must include a compatibility gate:

Codex is acceptable only if:

- input prompt colors are correct enough
- typed input is visible
- Codex does not hang on terminal queries
- resize works
- paste works
- Ctrl+C behavior is acceptable
- alternate screen and scrollback do not corrupt layout

If this fails, do not reintroduce tmux. Instead replace the `TerminalPanel` backend behind the same interface:

```rust
trait TerminalBackend {
    fn spawn(...);
    fn resize(...);
    fn write_input(...);
    fn render(...);
}
```

Possible backend replacement:

- `alacritty_terminal`
- `wezterm-term`

The UI architecture should not depend on which terminal backend wins.

## Migration Plan

### Phase 1: Extract Native Panels

Move existing logic without changing behavior:

- move outline code out of `main.rs`
- move overview code out of `main.rs`
- move markdown view into a panel module
- keep reviewer module as-is internally

Success criteria:

- tests still pass
- no tmux behavior changed yet

### Phase 2: Introduce AppState and Native Layout

Create single-process layout that can render:

- outline on left
- overview on right/top
- placeholder content/editor area

At this stage, content can still be placeholder text.

Success criteria:

- app runs without tmux for outline + overview
- file selection state is in memory
- `Ctrl+F2` help works natively

### Phase 3: Add MarkdownPanel

Embed current markdown renderer as a native panel.

Success criteria:

- selecting a markdown file opens preview in main area
- no `markdown-view` subcommand
- no tmux panel swap

### Phase 4: Add TerminalPanel with tui-term

Implement `TerminalPanel` using `tui-term` `smux` pattern.

Start with `sh`, then test `codex`.

Success criteria:

- shell starts
- input works
- resize works
- process exits cleanly
- Codex starts

### Phase 5: Replace Editor/Content tmux Flow

Replace:

- `open_editor_panel`
- `open_view_panel`
- `toggle_markdown_view`
- `return_editor_panel`
- `editor_state` temp files
- `outline_active_file` temp file as primary state

With in-memory mode transitions:

```text
EditorTerminal(current_file) <-> MarkdownPanel(current_file)
ContentTerminal(codex)
```

Success criteria:

- `Ctrl+F1` always toggles reliably
- `:wq` cannot blank content because content is not a tmux pane
- no tmp window
- no pane title dependency

### Phase 6: Embed Reviewer

Move reviewer into main app as a native route/panel.

Modes:

- phase click -> GSD reviewer
- `Ctrl+F3` -> git reviewer
- `g` can still toggle modes when phase context exists

Success criteria:

- reviewer no longer opens a tmux window
- git reviewer works without GSD
- phase reviewer still works

### Phase 7: Delete tmux

Remove:

- tmux bootstrap
- tmux pane/window helpers
- tmux key bindings
- tmux state files
- hidden subcommands used only for tmux orchestration

Potentially keep hidden subcommands only if still useful for debugging.

Success criteria:

- `rg tmux src/` returns no production references
- app runs directly in current terminal
- all tests pass

## Validation Checklist

Core UI:

- open app from normal shell
- browse outline
- select project markdown file
- select `~` markdown file
- preview markdown
- toggle preview/editor
- open reviewer git mode
- open reviewer GSD mode from phase click

TerminalPanel:

- shell input
- vim/nvim input
- Codex input
- Ctrl+C
- paste
- resize
- child exit
- restart child
- Unicode rendering
- colors

Regression cases from current tmux implementation:

- no stale project-dir in shortcuts
- no empty content panel after `:wq`
- no two-view/no-editor bad state
- no temp window state mismatch
- no pane title dependency
- no need to press shortcut multiple times

## Cutover Rule

Do not delete tmux code until the single-process path can run:

- outline
- markdown preview
- git reviewer
- GSD reviewer
- shell TerminalPanel
- Codex TerminalPanel

If Codex fails in `tui-term`, keep the native app architecture and swap only the terminal backend.

## Final Target

The final app should be:

```text
one process
one event loop
one state tree
ratatui-native panels
tui-term-backed child terminal panel
zero tmux orchestration
```

This is the path that removes the class of bugs caused by tmux pane state drift.

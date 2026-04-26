# Egui UI Audit

## Goal

Collect the current UI requirements in `gsdv`, collapse duplicated UI patterns into a smaller component set, and recommend a pragmatic `egui` stack for a desktop rewrite.

Terminal embedding is in scope through direct `alacritty_terminal` integration.

## Current Product Scope

From the repository design notes and current app structure, the product is a single-process workspace tool with:

- multi-workspace tabs on the left
- one active workspace at a time
- an outline tree for markdown/navigation
- a center content area with agent, workspace terminal, editor, and preview views
- a fullscreen reviewer route
- a stack of per-workspace popups

Primary source references:

- [AGENTS.md](../AGENTS.md)
- [src/bin/gsdv-gui.rs](../src/bin/gsdv-gui.rs)
- [src/gui/app.rs](../src/gui/app.rs)
- [src/gui/data.rs](../src/gui/data.rs)
- [src/gui/terminal_host.rs](../src/gui/terminal_host.rs)
- [src/reviewer/app.rs](../src/reviewer/app.rs)

## UI Requirements

### Shell Structure

- Vertical workspace tabs at the left edge.
- Main workspace layout is `tabs | outline | center`.
- Overview panel is currently disabled and should not drive the rewrite scope.
- Per-workspace state must survive tab switching.
- Closing a workspace should switch left, then right, then enter the zero-workspace empty state if none remain.

Relevant code:

- [src/gui/app.rs](../src/gui/app.rs)
- [src/gui/data.rs](../src/gui/data.rs)

### Workspace Tabs

- Show all open workspaces.
- Mark active workspace.
- Surface agent status: unknown, idle, busy.
- Support left-click switching.
- Support right-click context menu.
- Support delete confirmation.

Relevant code:

- [src/gui/app.rs](../src/gui/app.rs)

### Outline Tree

- Display markdown files and selected home dirs.
- Expand/collapse directories.
- Preserve expansion state per workspace.
- Mark selected row.
- Mark currently active file.
- Open file on click/Enter.
- Right-click context menus for dirs and files.

Relevant code:

- [src/gui/app.rs](../src/gui/app.rs)
- [src/gui/data.rs](../src/gui/data.rs)

### Center Views

The center area hosts four mutually exclusive views:

- Agent panel
- Workspace terminal panel
- Markdown editor
- Markdown preview

For the current `egui` version:

- keep Agent as the default core surface
- keep Workspace Terminal as a first-class workspace surface
- keep Editor and Preview as document surfaces

Relevant code:

- [src/gui/app.rs](../src/gui/app.rs)
- [src/gui/terminal_host.rs](../src/gui/terminal_host.rs)

### Markdown Editor

- Edit plain markdown files
- Dirty-state tracking
- Save with explicit action
- Select all
- Copy/paste
- Mouse selection
- Double-click word selection
- Triple-click line selection
- Visible current line highlight

Relevant code:

- [src/gui/app.rs](../src/gui/app.rs)

### Markdown Preview

- Render parsed markdown in-app
- Scroll long documents
- Preserve support for headings, lists, quotes, tables, code, images, inline formatting
- Re-render when editor text changes

Relevant code:

- [src/gui/app.rs](../src/gui/app.rs)
- [src/gui/data.rs](../src/gui/data.rs)

### Reviewer

- Fullscreen route separate from workspace layout
- Two modes: GSD and Git
- Four-column information-dense layout
- Independent scrolling/selection across columns
- Diff/full-file modes
- Commit message panel
- Branch actions
- Loading/error/min-width states

Relevant code:

- [src/gui/app.rs](../src/gui/app.rs)
- [src/gui/reviewer_adapter.rs](../src/gui/reviewer_adapter.rs)
- [src/reviewer/app.rs](../src/reviewer/app.rs)

### Dialogs / Overlays

Current dialogs and overlays:

- Help
- Settings
- Add workspace directory picker
- Outline dir menu
- Outline file menu
- Workspace tab menu
- Workspace delete confirm
- New markdown dialog
- New folder dialog
- Rename dialog
- Delete markdown confirm
- Dirty repo warning
- Branch list
- Branch confirm
- Generic message
- Toast feedback

Relevant code:

- [src/gui/app.rs](../src/gui/app.rs)

## Duplicate Patterns To Collapse

The current GUI uses a smaller set of reusable egui menu, dialog, and toast patterns.

### 1. Menus

Collapse into one reusable context menu system:

- workspace tab menu
- outline dir menu
- outline file menu

Shared behavior:

- anchored to pointer position
- list of actions
- closes on outside click or action

### 2. Confirm Dialogs

Collapse into one reusable confirm modal:

- delete workspace
- delete markdown
- branch switch confirm

Shared behavior:

- title
- body text
- primary action
- cancel action

### 3. Message / Error Dialogs

Collapse into one reusable message modal or toast source:

- generic message popup
- dirty warning
- branch switch failure
- load failure

### 4. File Creation Flow

Collapse into one reusable input dialog:

- new markdown
- create workspace path picker in v2

### 5. Center Surface Types

Do not keep separate ad hoc render logic per surface if the structure is the same.

Unify around:

- `AgentSurface`
- `WorkspaceTerminalSurface`
- `DocumentEditorSurface`
- `DocumentPreviewSurface`
- `ReviewerRoute`

### 6. Row Selection Lists

The following are the same interaction class:

- workspace tabs
- outline entries
- reviewer columns
- branch list

They should share:

- selection model
- hover model
- context-menu affordance
- keyboard navigation hooks

## Proposed Egui Information Architecture

## App Frame

- `SidePanel::left`: workspace tabs
- `SidePanel::left` nested or `CentralPanel` split: outline
- `CentralPanel`: main content
- `TopBottomPanel`: optional toolbar/status later

For fullscreen reviewer:

- keep a route flag and swap `CentralPanel` content to `ReviewerPane`
- do not create a separate native window in v1

## Recommended Component Inventory

### Foundation

- `eframe` for app shell and native desktop integration
- `egui` for core widgets and input
- `serde` persistence for window/workspace UI state

### Layout

- Native `egui` panels for the default app frame
- `egui_extras::StripBuilder` for fixed split layouts
- `egui_extras::TableBuilder` for dense tabular/reviewer sections

### Tabs / Docking

Recommended now:

- custom workspace tab rail using plain `egui` widgets

Optional later:

- `egui_dock` if you want user-rearrangeable panes with stable docking today
- avoid `egui_tiles` in the first rewrite unless you explicitly want a more flexible tile tree and are willing to absorb more integration work

### Markdown Preview

Recommended:

- `egui_commonmark` or `egui_markdown`

Preference:

- start with `egui_commonmark` if you want a more established viewer-style integration with cache/image support
- use `egui_markdown` only if you prefer a smaller, simpler markdown widget and are comfortable with a younger crate

### Markdown Editing

Recommended now:

- start with native `egui::TextEdit::multiline`

Reason:

- current editor is markdown-focused, not a full code IDE
- this keeps v1 simple and removes a risky dependency

Only add a specialized editor crate later if you need:

- syntax highlighting
- gutters
- code folding
- complex selection semantics beyond current markdown needs

### Reviewer

Recommended:

- custom reviewer UI on top of `egui_extras::TableBuilder`
- custom diff panel using `ScrollArea`, `LayoutJob`, and colored `RichText`

Reason:

- reviewer behavior is highly app-specific already
- existing state/render logic can be reused without forcing it into a generic grid widget

### File Dialogs

Recommended:

- `rfd` for native open/save/select-directory dialogs

Fallback if you want a fully in-egui file picker:

- `egui-file-dialog`

### Notifications

Recommended:

- `egui-notify` for transient toasts

Use it for:

- save success/failure
- branch switch result
- background refresh/load errors

### Images

Recommended:

- `egui_extras::install_image_loaders`

## Recommended Rewrite Scope

## Version 1

Ship these first:

- app frame
- workspace tabs
- outline tree
- markdown editor
- markdown preview
- help modal
- create/open workspace via native dialog
- generic context menu
- generic confirm modal
- generic message/toast system
- fullscreen reviewer route

Current implementation notes:

- embedded terminals are implemented through direct `alacritty_terminal`
- workspace terminal is a center surface, not a popup
- workspace creation uses a native directory picker

## Version 2

- branch dialog polish
- richer keyboard shortcut map
- drag-and-drop open workspace/file
- optional dockable panes
- richer editor features

## Suggested State Split For Egui

- `AppUiState`
  - active workspace id
  - global modal stack
  - toasts
  - route

- `WorkspaceUiState`
  - project dir
  - outline tree state
  - active document
  - center pane kind
  - reviewer state

- `ModalState`
  - menu
  - confirm
  - prompt
  - message

This is intentionally smaller than the old terminal-era popup model.

## Recommended Crates

- `eframe`
- `egui`
- `egui_extras`
- `egui_commonmark`
- `rfd`
- `egui-notify`

Optional:

- `egui_dock`

Avoid in v1:

- PTY / terminal crates
- custom keyboard protocol parsers
- custom ANSI/OSC terminal compatibility layers

## Web Notes

Current docs reviewed during this audit:

- `egui` / `eframe` official repo: https://github.com/emilk/egui
- `egui_extras`: https://docs.rs/egui_extras
- `egui_dock`: https://docs.rs/egui_dock
- `egui_tiles`: https://docs.rs/egui_tiles
- `rfd`: https://docs.rs/rfd
- `egui_commonmark`: https://docs.rs/egui_commonmark
- `egui_markdown`: https://docs.rs/egui_markdown

Key takeaways from those docs:

- `eframe` is the official native/web app framework for `egui`
- `egui_extras` is the standard extension crate for tables, strips, and image loaders
- `egui_dock` is the more mature docking option today
- `egui_tiles` is explicitly positioned as a more flexible alternative but earlier in development
- `rfd` is cross-platform and explicitly supports macOS, Windows, Linux/BSD, and wasm

## Recommendation

Use a conservative stack for the rewrite:

- `eframe` + `egui`
- `egui_extras`
- `egui_commonmark`
- `rfd`
- `egui-notify`

Build the outline, editor, preview, reviewer, modal system, and workspace rail as app-owned components.

Do not start by introducing docking, embedded terminals, or a third-party tree widget.
Those are not required to replace the current product value, and they would reintroduce the exact kind of integration cost this rewrite is meant to remove.

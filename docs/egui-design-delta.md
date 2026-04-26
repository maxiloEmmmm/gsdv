# Egui Design Delta

## Scope

This note tracks remaining deltas between the current egui implementation and the three reference scenes in `docs/design/`.

## Agent Scene

Aligned:

- Workspace rail, outline, center surface hierarchy follows the reference composition.
- Agent is the default center surface and remains the operational hub.
- Agent state card includes workspace, root, session, and activity state.
- Agent content is backed by a mature embedded terminal rather than recreated with custom TUI logic.
- Action row exposes reviewer, current document, session resume, and workspace terminal routes.

Remaining visual delta:

- The terminal transcript area is real terminal content, so internal transcript layout is controlled by the terminal app.
- Agent task cards from the image mock are not implemented as native egui cards yet.

## Markdown Scene

Aligned:

- Editor and preview have explicit document headers with numbered labels, title, file path, status badges, and primary actions.
- Dirty editor state blocks unsafe file switching and is visible in editor and preview.
- Preview renders unsaved editor content.
- Context menus cover create markdown, create folder, rename, delete, copy paths, reveal, and refresh.
- Empty and error states are shown as explicit cards/strips instead of bare text.

Remaining visual delta:

- The editor uses egui's plain multiline text editor, so syntax highlighting and current-line rendering remain basic.
- Preview styling comes from `egui_commonmark`; exact markdown typography may differ from the mock.

## Reviewer Scene

Aligned:

- Reviewer is a fullscreen route, not a small side pane.
- GSD/Git mode switch, four-column layout, dominant diff column, metadata strip, filters, footer hints, branch dialog, dirty repo blocking, and too-narrow state are implemented.
- Reviewer keyboard navigation is centralized and mirrors pointer actions.

Remaining visual delta:

- Diff minimap and advanced changed-block jump controls are not native visual widgets yet.
- Branch dialogs are functional and styled, but not pixel-matched to the mock cards.

## Final Manual Run-Through Checklist

- Launch with `cargo run --bin gsdv-gui`.
- Verify zero-workspace state by using an empty `GSDV_STORE_PATH`.
- Add a workspace, switch Agent / Terminal / Editor / Preview with `Cmd/Ctrl+1-4`.
- Create, rename, delete, and reveal a markdown file from the outline menu.
- Edit a markdown file, confirm dirty state, save with `Cmd/Ctrl+S`, and preview unsaved content.
- Open Reviewer with `Cmd/Ctrl+Enter`; test arrows, `Tab`, `R`, `F`, `B`, and `Esc`.

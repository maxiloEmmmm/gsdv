# GUI Instructions

## Update And Render Model

`eframe::App::update` must stay split into two conceptual phases:

1. Lightweight event processing.
2. Layout and paint.

The event phase may drain channels, update small state, enqueue work, and dispatch
commands. It must stay within the configured max-FPS frame budget. If the budget is
spent, request another update and continue later.

Heavy work is forbidden in `update` and in every render/layout path. This includes
Markdown parsing, filesystem reads or writes, outline rebuilds, status file parsing,
script directory scanning, git work, process spawning, and similar slow operations.
When an event would trigger heavy work, dispatch it to the background runtime and
return immediately.

Background tasks must not mutate render-owned app state directly. They may only
produce immutable result events and send them back to the GUI event channel. The
next `update` drains those result events on the main thread and performs only the
small state merge needed by rendering.

A background result event is also the data-change request. Do not separately request
another render for the same logical change after the merge, because the current
`update` will paint after merging. Only request another update when there are more
queued result events than the current frame budget can process.

The render phase must be dumb: read already-prepared state, run egui layout, and
paint. Layout calculation is allowed. Parsing, IO, event handling, cache rebuilding,
diffing, persistence, process work, and clone-heavy data preparation are not allowed
in render paths.

## Keyboard Event Routing

Keyboard handling must be route-first, not a global shortcut blob.

- `handle_keyboard_shortcuts` is the single keyboard entrypoint.
- First call a base helper that reads real `GsdvGuiApp` state and handles global route-switching shortcuts: workspace terminal, notifications, agent/markdown toggle, reviewer route, and Helix drawer. If it returns a command, dispatch it and stop.
- Do not create shadow route enums or context structs that duplicate app state.
- If the base helper does not consume the event, dispatch to the active route: dialogs, notifications, workspace terminal drawer, reviewer Helix drawer, reviewer route, or workspace route.
- Workspace route then dispatches by active tab: Agent or Markdown.
- Agent tab owns its embedded terminal. Agent tab decides whether a shortcut is consumed locally or passed to its terminal path.
- Hovering the outline tree means outline-local shortcuts may consume input instead of the Agent tab.

# GSDV Layout And Interaction Handoff

## Purpose

This document is the design handoff version of the product spec.

It is intentionally optimized for:

- image-model UI generation
- layout planning
- interaction planning

It is intentionally not optimized for:

- final visual styling
- precise typography choices
- color palette decisions

Those can be decided later by the image model or UI design process.

## 1. Core Product Structure

The app is a single-window, multi-workspace desktop tool.

The core mental model is:

- one app window
- many workspaces
- one active workspace at a time
- each workspace owns:
  - one primary agent surface
  - one persistent workspace terminal
  - one outline tree
  - one active markdown document context
  - one reviewer context

The `agent` is the core surface of the product.

The `workspace terminal` is also a first-class workspace surface and is not removed from the product definition.

Markdown and reviewer are supporting but important surfaces around the workspace.

## 2. Main Window Layout

The default app layout is:

```text
| workspace rail | outline tree | main center surface |
```

### Region 1: Workspace Rail

This is a narrow vertical strip on the far left.

It shows:

- all open workspaces
- active workspace
- workspace activity state

It is always visible in normal workspace mode.

### Region 2: Outline Tree

This is a narrow-to-medium left panel next to the workspace rail.

It shows:

- markdown files in the current workspace
- directories
- optional home-root markdown sources such as `~`, `.codex`, `.agents`, `.claude`

It is always visible in normal workspace mode.

### Region 3: Main Center Surface

This is the dominant panel and takes most of the window width.

It can show one of several mutually exclusive center views:

- Agent
- Workspace Terminal
- Markdown Editor
- Markdown Preview

Only one center view is visible at a time.

## 3. Alternate Fullscreen Route

The app also has one special fullscreen route:

- Reviewer

When reviewer is open, it replaces the normal `outline + center` workspace content area.

The workspace rail still conceptually exists, but the main workspace content becomes the reviewer.

The reviewer is not a small side pane. It is a dedicated route.

## 4. Default Center View Priority

Within a workspace, the intended priority is:

1. Agent
2. Workspace Terminal
3. Markdown Editor
4. Markdown Preview

This means:

- the agent should feel like the workspace home
- terminal is a major alternate working mode
- editor and preview are document-specific supporting modes

## 5. Center Surface Behavior

### Agent View

This is the primary surface of the workspace.

It should communicate:

- which workspace the user is in
- whether the agent has a live or restorable session
- whether the agent is idle, busy, unavailable, or resumable

The design does not need to commit to the exact visual treatment of the agent transcript yet.

What matters for layout:

- it is the main surface
- it should be easy to return to
- it should feel central, not secondary

### Workspace Terminal View

This is the persistent shell context for the workspace.

It is not the same thing as the agent.

It should feel like:

- a long-lived workspace shell
- tied to the current workspace root
- something the user can leave and return to later

### Markdown Editor View

This is the editable markdown document surface.

It should clearly show:

- current file
- unsaved/dirty state
- editable content

### Markdown Preview View

This is the rendered reading surface for markdown.

It should clearly show:

- which file is being previewed
- rendered content
- independent scroll state

## 6. Reviewer Layout

Reviewer is a fullscreen route with a four-column layout.

Top to bottom structure:

```text
header
subheader/status row
4-column content body
footer/help row
```

### Reviewer Column Model

In `GSD` mode:

- column 1: change groups
- column 2: repos
- column 3: files
- column 4: diff or full-file view

In `Git` mode:

- column 1: repos
- column 2: commits
- column 3: files
- column 4: diff or full-file view

### Reviewer Width Emphasis

The first three columns are selection lists.

The fourth column is the dominant reading surface.

So the visual weight should be:

- narrow list column
- narrow list column
- narrow list column
- wide diff column

The reviewer should not collapse into equal-width columns.

### Reviewer States

Reviewer must have clearly distinct layouts for:

- loading
- error
- too narrow / minimum width not met
- normal content

## 7. Overlay Types

The app has layered overlays above the active workspace content.

There are three main overlay types:

### Context Menu

Used for:

- workspace tab actions
- outline directory actions
- outline file actions

Behavior:

- opens near the pointer
- small
- action-focused

### Modal Confirm / Input Dialog

Used for:

- close workspace
- delete markdown
- create markdown
- branch switch confirm

Behavior:

- centered
- compact
- blocks until resolved

### Large Modal / List Dialog

Used for:

- help
- branch list
- dirty repo warning
- larger messages

Behavior:

- centered
- larger than confirm dialogs
- may contain lists or explanatory text

## 8. Navigation Model

The app is hybrid mouse + keyboard.

Mouse is enough to use the product.

Keyboard is important for speed.

The design should make both plausible without requiring either one exclusively.

### Global Navigation

User can:

- switch workspace
- choose active center surface
- open reviewer
- open help
- open dialogs/menus

### Outline Navigation

User can:

- select rows
- expand/collapse directories
- open a file
- open file/directory context menus

### Center Navigation

Depends on active center view:

- agent: interact with agent session
- terminal: interact with workspace shell
- editor: edit text
- preview: read and scroll

### Reviewer Navigation

User can:

- move active column
- move row selection
- scroll each list column
- horizontally scroll long labels in list columns
- inspect diff content
- toggle diff/full-file mode
- jump between changed blocks in full-file mode
- trigger branch actions

## 9. Workspace Interaction Logic

Each workspace is an independent context bundle.

Switching workspace should switch all of these together:

- active agent context
- workspace terminal context
- outline state
- active file
- editor/preview state
- reviewer state
- popup stack

The app should not treat workspaces as only path labels.

They are stateful working containers.

## 10. Document Interaction Logic

### Opening a File

When the user opens a markdown file:

- editor becomes the default destination
- if preview mode is already active, the app may preserve preview mode when switching files

### Dirty File Protection

If the current markdown editor has unsaved changes:

- switching to another file should be blocked
- closing the editor should be blocked
- the app should show a clear warning

### Create Markdown

Flow:

1. user targets a directory
2. opens directory menu
3. chooses create markdown
4. enters file name
5. app normalizes `.md`
6. app creates and opens the file

### Delete Markdown

Flow:

1. user opens file menu
2. chooses delete
3. confirm dialog appears
4. if confirmed, file is removed
5. if deleted file was active, center surface exits document mode

## 11. Agent Interaction Logic

The exact final GUI for the agent can vary, but the interaction logic should remain:

- each workspace has one primary agent context
- agent can have a live or resumable session
- workspace tab state should expose agent activity at a glance
- agent status can be unknown, idle, busy, unavailable, or resumable

The design should make clear:

- whether this workspace has an active agent session
- whether it can be resumed
- whether work is currently happening

## 12. Workspace Terminal Interaction Logic

The workspace terminal is:

- per-workspace
- persistent
- separate from the agent

The design should make clear:

- terminal belongs to current workspace
- terminal can be left and returned to
- terminal is a center-mode peer of agent/editor/preview

Terminal UI should stay tied to the workspace terminal center mode or the agent surface, not to ad hoc modal terminals.

## 13. Reviewer Interaction Logic

### GSD Mode

The reviewer is reading structured work provenance.

The user flow is:

1. choose a change group
2. choose a repo bucket
3. choose a file
4. inspect diff/full content

### Git Mode

The reviewer is reading live repository history/state.

The user flow is:

1. choose a repo
2. choose a commit, including synthetic uncommitted changes if present
3. choose a file
4. inspect diff/full content

### Branch Switch Logic

Flow:

1. choose repo
2. request branch action
3. if repo is dirty, block with warning
4. else show branch choices
5. user chooses target branch
6. show confirm dialog
7. perform switch
8. refresh reviewer

## 14. Route Logic

Normal workspace mode:

```text
workspace rail + outline + center surface
```

Reviewer route:

```text
workspace rail + fullscreen reviewer content
```

Returning from reviewer should restore the previous center surface for that workspace.

This is important:

- reviewer is temporary route replacement
- not a permanent change to workspace center mode

## 15. Empty And Error States

Designs should account for:

- no workspace open
- no markdown files
- no active file
- editor load failure
- save failure
- terminal unavailable
- agent session unavailable
- reviewer loading
- reviewer error
- reviewer too narrow
- no branch choices
- dirty repo blocking branch switch

These should be explicit layouts, not invisible fallback behavior.

## 16. What The Image Model Needs To Get Right

The generated design does not need to guess colors or exact widget style perfectly.

It does need to get these structural truths right:

- app is workspace-first
- agent is center of the product
- workspace terminal still exists and matters
- outline is always a left-side navigator
- editor/preview are center-mode alternatives
- reviewer is a fullscreen route with four columns
- dialogs are layered and come in small / medium / large forms
- workspace switching swaps whole context bundles, not just file lists

## 17. What The Image Model Does Not Need To Lock Down

It does not need to finalize:

- color palette
- icon style
- font family
- exact shadows/borders/radii
- final spacing scale
- exact chat/transcript treatment for the agent

Those can remain flexible as long as the layout and interaction model above is respected.

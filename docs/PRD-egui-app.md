# GSDV Desktop App PRD

## Document Status

- Product: `gsdv`
- Target form: native desktop app built with `egui` / `eframe`
- Purpose of this PRD: serve as the single detailed product brief for UI design generation and implementation planning
- Scope: the whole application, including all currently shipped user-facing functionality that should survive the `egui` rewrite
- Product stance: `agent` is the core surface of the app
- Product stance: workspace-scoped global terminals remain part of the product definition

## 1. Product Summary

`gsdv` is a single-process, agent-centered workspace app for running and observing an AI coding agent inside each workspace, navigating Markdown project files, editing Markdown, previewing rendered Markdown, inspecting reviewer data derived from GSD and Git, and managing multiple project workspaces from one app shell.

The current Ratatui implementation already proves the product shape. The `egui` rewrite should preserve the product's information density and workflow speed while removing terminal-specific UI compatibility costs, without demoting the agent or removing the workspace-level terminal mental model.

The app is not a generic IDE. It is a focused project cockpit for:

- switching between multiple repos/workspaces
- running one primary agent session per workspace
- keeping a workspace-level global shell/terminal available
- browsing structured markdown content
- editing markdown quickly
- previewing rendered markdown in-app
- reviewing GSD provenance and Git diffs in a dense, navigable reviewer

## Core Mental Model

The product is organized around the following hierarchy:

- `App`
  - owns the global window shell and the set of open workspaces
- `Workspace`
  - is the primary unit of user context
  - owns one agent context, one global terminal context, one outline tree, one active document context, and one reviewer context
- `Agent`
  - is the primary center-surface workflow for a workspace
  - is not just metadata; it is a first-class working surface
- `Documents`
  - are supporting working materials for the active workspace and active agent
- `Reviewer`
  - is the inspection and validation surface tied to the active workspace

The intended relationship is:

- the `agent` is the operational core
- the `workspace` is the containment boundary
- markdown and reviewer are supporting surfaces around the active workspace and agent

## 2. Product Goals

### Primary Goals

- Preserve all core workflows from the current app in a desktop GUI.
- Preserve the agent as the primary product surface.
- Preserve one workspace-scoped global terminal per workspace.
- Reduce UI fragility caused by TUI input/color/terminal compatibility layers.
- Keep the product fast, dense, keyboard-friendly, and single-window.
- Make the reviewer and document workflows easier to understand visually.
- Support workspace persistence and fast resumption across launches.

### Secondary Goals

- Improve discoverability of commands and actions.
- Make contextual actions clearer through menus and dialogs instead of hidden terminal-era affordances.
- Create a cleaner foundation for future dockable panes and richer editing.

### Non-Goals For Initial `egui` Release

- Re-creating the old overview panel as a first-class priority
- Turning `gsdv` into a general-purpose code editor
- Reproducing terminal-era input quirks when a desktop-native interaction model is better

## 3. Target Users

### Primary User

- A developer working inside one or more Markdown-heavy project repos
- Uses GSD artifacts, project docs, and Git history as part of daily work
- Wants a focused review/edit/navigation tool rather than a full IDE

### Secondary User

- A reviewer or technical lead who mainly uses the app for reviewer inspection and document reading
- Less interested in editing and more interested in provenance, diffs, and repo state

## 4. Product Principles

- Dense, not bloated
- Fast to scan
- Keyboard-friendly but not keyboard-dependent
- Single-window, single-process
- Explicit state over hidden magic
- Workspace-first architecture
- Agent-first workflow
- Workspace owns agent, terminal, documents, and reviewer
- Reviewer and document workflows are first-class

## 5. Platform Scope

- macOS: required
- Linux: required
- Windows: desirable, not the primary constraint for v1

The design must not rely on terminal semantics.

## 6. High-Level Information Architecture

The app has one main window.

Primary shell:

- left rail: workspace tabs
- left content panel: outline / file tree
- main content surface: agent, workspace terminal, editor, or preview
- fullscreen route: reviewer
- layered overlays: dialogs, context menus, toasts

The old overview panel is intentionally not part of the current egui implementation and should not drive the first GUI release.

### Default Layout Geometry

The current egui app establishes a strong default geometry that should inform future UI work.

- workspace entries form a dedicated vertical rail on the far left
- the outline sits immediately to the right of the workspace rail
- the center surface occupies the remaining main area
- the historical overview panel remains out of scope

Current egui proportions that matter as design guidance:

- workspace rail is narrow and optimized for project identity plus activity state
- outline width is a fixed narrow-to-medium column optimized for scanning file names
- center view gets the bulk of the window width

Future work should preserve these relative priorities:

- workspace rail is narrow
- outline is narrow-to-medium
- center surface is dominant

### Surface Hierarchy

The shell should visually communicate three levels of hierarchy:

- level 1: active workspace
- level 2: active center surface within that workspace
- level 3: contextual overlays such as menus, confirms, and branch dialogs

## 7. Core Entities

### Workspace

A workspace represents one project root directory and carries its own UI state.

Per-workspace state includes:

- project root path
- workspace display name
- primary agent session / agent surface state
- global workspace terminal state
- expanded outline directories
- selected outline row
- outline scroll position
- active markdown file
- current center view
- reviewer state
- popup state
- agent status metadata
- persisted session metadata
- previous center view for route returns

### Document

A document is a markdown file within the workspace root or supported home-linked roots.

Document state includes:

- absolute path
- relative outline path
- loaded editor text
- dirty state
- preview render state

### Reviewer Session

A reviewer session is a per-workspace inspection state with:

- mode: GSD or Git
- active column
- row selections
- horizontal scroll positions
- diff/full-file mode
- diff scroll state
- selected repo / commit / file

### Agent Session

An agent session is a per-workspace first-class interaction surface.

Agent session state includes:

- agent provider / agent kind
- session id
- agent activity state
- current conversation surface state
- restore/resume metadata
- links to the workspace root and current supporting context

### Workspace Global Terminal

A workspace global terminal is the persistent shell/terminal context attached to a workspace.

Terminal state includes:

- current working directory
- process/session lifecycle
- visibility state
- relationship to the active workspace
- restoration / reuse behavior when switching back to the workspace

## 8. Functional Scope

## 8.1 App Shell

The app opens into a single native window.

Requirements:

- The shell must support multiple workspaces.
- The active workspace determines the agent surface, workspace terminal, outline, center document surface, reviewer session, and dialogs.
- The app must keep workspace state isolated when switching tabs.
- The app must allow running with zero open workspaces.

Empty-state requirements:

- If no workspaces are open, show an explicit empty state.
- The empty state must include a clear primary action to add/open a workspace.

Focus model requirements:

- the shell has a meaningful distinction between outline focus and center focus
- visual focus indication should survive the GUI rewrite
- center interactions should not accidentally clear workspace/tab context

## 8.2 Workspace Management

### Add Workspace

The app must allow adding a new workspace rooted at a directory.

Initial GUI approach:

- Use a native directory picker.
- Do not require a terminal-based "cd then Esc" workflow.

Requirements:

- If the chosen directory is already open, switch to the existing workspace instead of duplicating it.
- Workspace paths must be normalized before duplicate checks.

### Switch Workspace

Requirements:

- Clicking a workspace tab switches to it.
- Keyboard cycling should be supported.
- The previous workspace state must be preserved.
- Switching workspace must restore that workspace's previous center surface and popup/reviewer state as appropriate.

### Close Workspace

Requirements:

- A workspace can be closed from its context menu.
- If the closed workspace is active, activate the tab on the left when possible, otherwise the one on the right.
- If the last workspace is closed, the app enters the empty-workspace state instead of forcing quit.

### Persist Workspace List

Requirements:

- The app must persist the list of open workspaces.
- The app must persist the last active workspace index when possible.
- The app must persist any stored agent session id metadata already associated with each workspace.
- The app must preserve the concept of one global terminal context per workspace.
- The app should preserve enough state to return to the prior center view after route changes when possible.

## 8.3 Workspace Tabs

Each workspace is represented by a tab in the left rail.

Each tab must show:

- workspace name
- whether it is active
- agent activity state if known

States:

- active
- inactive
- unknown agent status
- idle agent status
- busy agent status

Interaction:

- left-click to activate
- right-click to open a workspace context menu
- context menu initially needs only one action: delete/close workspace

Visual goals:

- compact
- vertically stackable
- readable with long project names
- supports visible active state without relying on loud flashing

Layout and chrome details:

- tabs are stacked vertically, one row per workspace
- the rail includes a titled header area
- the first interactive tab row starts below the title row, not at the very top border
- active and status markers should remain visible even in narrow widths
- the workspace name may use compact path-derived naming rather than verbose absolute paths

Status presentation details derived from current behavior:

- unknown state has an explicit marker
- busy state uses stronger contrast than idle
- the active busy workspace is intentionally more visually prominent than inactive busy workspaces

## 8.4 Agent Surface

The agent is a core product surface and the default center view for a workspace.

Requirements:

- Every workspace has one primary agent surface.
- The agent surface is not optional in the product definition.
- When a workspace is activated, the user must be able to understand the state of its agent immediately.
- The agent surface must support session restoration when prior session metadata exists.
- The agent surface must visibly express unknown / idle / busy states.
- The agent surface must be treated as the operational hub of the workspace.

The agent surface must integrate with:

- workspace-level session persistence
- document opening and editing
- reviewer launch
- workspace activity indicators

The PRD does not force a specific rendering strategy for agent output in the first `egui` implementation, but the product requirement is that the agent remains a first-class center surface.

Layout expectations:

- the agent should occupy the dominant center region when active
- it should feel like the default "home" view for a workspace
- it should not be visually reduced to a small side pane or secondary inspector
- any supporting context around the agent should remain subordinate to the agent surface

## 8.5 Workspace Global Terminal

The workspace-scoped global terminal remains part of the product.

This terminal is distinct from:

- the primary agent surface
- dialogs and transient overlays
- reviewer-specific actions

Requirements:

- Each workspace owns one global terminal context.
- Switching workspaces switches terminal context with it.
- The global terminal must preserve workspace cwd semantics.
- The terminal must remain conceptually persistent across workspace switching.
- The user must be able to return to the workspace terminal as a primary center surface.

Product note:

- The initial engineering rollout may phase terminal implementation work, but the product definition does not remove the workspace-global terminal.

Layout expectations:

- the workspace terminal is a primary center surface alternative, parallel to agent, editor, and preview
- it should not be represented only as a temporary modal if the main product flow expects persistent return to it
- terminal UI belongs in the workspace terminal center surface or agent surface, not in ad hoc modal terminals

Terminal behavior expectations:

- the shell starts from the workspace root by default
- returning to a workspace should return to that workspace's shell context, not a shared global shell
- the app should expose terminal unavailability as a clear state instead of silently failing

## 8.6 Outline Tree

The outline is the left content tree for files and directories.

Requirements:

- Show markdown files relevant to the workspace.
- Show both workspace-root content and supported home-root content where applicable.
- Show directory nesting.
- Show expansion state.
- Support expand/collapse.
- Keep expansion state per workspace.
- Show selected row.
- Show active/open file distinctly from selected row.
- Support keyboard and pointer navigation.

Supported actions:

- select entry
- open markdown file
- expand/collapse directory
- open context menu for file
- open context menu for directory

Directory context menu actions:

- create new markdown file inside directory

File context menu actions:

- delete markdown file
- copy absolute path
- copy relative path

Empty state:

- If there are no markdown files, show a clear empty state inside the outline panel.

Layout and interaction details:

- the outline is rendered as a titled bordered panel
- the title visually reflects focus state
- directories and files have distinct visual treatment
- directories are not visually marked as "active file"
- selection and active-file state are separate concepts
- clicks on the panel title/border area should not accidentally select rows

Density details:

- rows are expected to be single-line by default
- nesting is communicated with indentation and prefix markers
- file names should remain scannable in a narrow panel

Content-source details:

- the outline is not limited to the current workspace root
- the outline may also expose a dedicated home-root section represented by `~`
- the home-root section may include direct markdown files from home plus curated subdirectories such as `.codex`, `.agents`, and `.claude`
- the home-root section should remain visually distinct from the workspace-root tree

Default expansion behavior:

- workspace top-level directories that are considered relevant should start expanded by default
- the home-root umbrella should also start expanded by default
- noisy or generated directories should be suppressed from the outline

Suppression rules:

- VCS, cache, build, package-manager, virtualenv, and other generated directories should not pollute the outline
- workspace-specific directories such as Rust `target` and JS `node_modules` should be hidden when they are clearly dependency/build artifacts

## 8.7 Document Opening Model

The app supports one active markdown document per workspace at a time.

Requirements:

- Selecting a markdown file opens it in the editor by default.
- If preview mode is currently active, switching files should preserve preview mode.
- If the active editor has unsaved changes and the user tries to switch to another file, block the switch and show a clear warning.

Center-surface transition rules:

- opening a file normally enters editor mode
- if preview is currently active, opening another file should preserve preview mode
- closing a clean editor returns the workspace to its default center surface rather than leaving an orphaned empty editor
- the app should remember the prior center surface when entering fullscreen reviewer and return appropriately on exit

## 8.8 Markdown Editor

The markdown editor is a core product surface.

Requirements:

- Open and display markdown file content.
- Allow direct editing.
- Track dirty state against last saved content.
- Save on explicit user action.
- Support select all.
- Support copy and paste.
- Support pointer-based selection.
- Support double-click word selection.
- Support triple-click line selection.
- Highlight current line.

Dirty-state rules:

- Dirty indicator must be visually obvious.
- Closing the editor with unsaved changes must be blocked by default.
- Switching away from the current document with unsaved changes must be blocked by default.

Error handling:

- If file load fails, show an explicit editor error state.
- If save fails, show a blocking error message or high-priority toast.

Layout and chrome details:

- the editor uses a titled bordered panel
- the title bar includes file name and key action hints in the current implementation
- dirty state is reflected in the title/border treatment
- current line highlight should remain subtle but always visible
- selection colors must remain legible on both dark and light terminal-derived themes; in GUI this should become a stable app theme rule

Interaction nuances from current behavior:

- editor scrolling follows mouse wheel
- editor copy should work via clipboard and fallback logic where possible
- select-all is a first-class operation
- double-click and triple-click semantics are intentional and should be preserved
- the editor should accept full-text paste events cleanly without breaking surrounding workspace state

## 8.9 Markdown Preview

The preview is the rendered view of the current markdown document.

Requirements:

- Render markdown inside the app.
- Support headings, paragraphs, lists, blockquotes, tables, inline emphasis, code spans, code blocks, rules, and image references.
- Allow scrolling through long documents.
- Refresh when editor text changes or when a file is loaded directly into preview.

Behavior:

- Preview can be toggled from the editor context.
- Preview must work from unsaved editor content, not only from file-on-disk content.

Empty state:

- If no markdown file is selected, show a clear preview empty state.

Layout details:

- preview is currently a three-part surface:
  - a one-line header with a `VIEW` label and file path
  - a scrollable body
  - a one-line footer with compact interaction hints

Design intent for GUI:

- keep this sense of top identity + central reading surface + bottom lightweight controls/status
- maintain a reading-optimized layout rather than making preview look like a generic text widget

Content rendering requirements:

- tables are first-class content and must render legibly
- blockquotes should remain visually distinct and continuous across wrapped lines
- inline code, emphasis, and strong emphasis must remain distinguishable without visual clutter
- preview scrolling should not disturb editor state

## 8.10 Reviewer

The reviewer is the most specialized surface in the app and must remain a first-class mode.

The reviewer is a fullscreen route inside the app window.

The reviewer is not a generic diff viewer. It is a structured inspection surface that fuses:

- GSD provenance
- repo grouping
- commit/file relationships
- patch/full-file inspection

### Reviewer Modes

Two reviewer modes exist:

- GSD mode
- Git mode

Requirements:

- If a phase context exists, the reviewer must allow toggling between GSD and Git modes.
- If phase context does not exist, Git mode remains available and GSD mode may be unavailable.

Mode-specific data meaning:

- `GSD` mode is phase/task/provenance-oriented
- `Git` mode is repo/commit/file-oriented

### Reviewer Layout

The reviewer uses four columns, but the meaning of the first three columns depends on mode.

In `GSD` mode:

- column 1: change groups
- column 2: repos
- column 3: files
- column 4: diff / full content

In `Git` mode:

- column 1: repos
- column 2: commits
- column 3: files
- column 4: diff / full content

Mode-specific interpretation details:

- in `GSD` mode, column 1 rows are derived from plans/tasks and may represent either plan-level or task-level provenance groupings
- in `Git` mode, column 1 rows are discovered recursively from git repositories under the workspace root, including nested repos
- in `Git` mode, uncommitted changes are surfaced as a synthetic top commit-like row ahead of real commit history

Requirements:

- Preserve this high-density four-column mental model.
- Each column must support its own selection and scrolling behavior.
- The fourth column must support diff mode and full-file mode.

Screen anatomy details:

- the reviewer has a one-line header
- a one-line subheader/status context row
- a multi-column content body
- a one-line footer shortcut/help row

Minimum width rule:

- the reviewer has an explicit minimum width threshold
- below that width, it should show a dedicated "needs wider area" state rather than collapsing into unreadable columns

Current width allocation guidance from implementation:

- column 1 uses roughly 15% of available width
- column 2 uses roughly 12.5%
- column 3 uses roughly 12.5%
- column 4 consumes the remaining width and is the dominant reading surface

This relative emphasis should be preserved in the GUI.

Column behavior details:

- columns 1 to 3 behave like structured selection lists
- column 4 behaves like a reading surface with its own scroll mode
- separators between columns should remain explicit enough to support dense scanning

### Reviewer States

The reviewer must support:

- loading state
- data error state
- too narrow / minimum width state
- empty content states

The reviewer route should never silently degrade into unreadable density. It must choose among:

- normal data view
- loading view
- error view
- width-too-small view

Data fallback rules:

- diff content may come from commit-backed patches
- working tree fallback
- non-commit fallback placeholder messaging
- explicit error messages when diff loading fails

### Reviewer Interactions

Requirements:

- select rows by mouse
- move active column
- move selected row
- scroll each column independently
- horizontal scrolling for long rows in the first three columns
- diff paging
- toggle full-file view
- jump between changed blocks in full-file mode
- reload data
- open branch actions

Selection behavior details:

- one active column at a time
- selected rows and hovered rows are distinct concepts
- clicking the diff column is meaningful and distinct from clicking list columns
- horizontal scrolling is intentionally reserved for the first three columns, where labels can exceed width
- keyboard navigation must remain symmetrical with pointer navigation
- tab/backtab style column stepping should remain plausible in GUI even if exact shortcuts evolve

### Diff Experience

Requirements:

- Render insertions, deletions, context lines, and hunks distinctly.
- Show commit message panel in Git mode when applicable.
- Preserve row numbering or line anchoring behavior in full-file mode.
- Align full-file view with diff context when switching modes.

Layout details for the diff column:

- the diff column begins with a title row
- in Git mode it may reserve a message panel for commit message lines before the diff body
- the commit message area must be capped so it cannot consume the whole diff viewport
- the diff body is the primary scroller in the fourth column

Visual requirements:

- inserts, deletes, hunks, and plain context need clearly distinct treatment
- stale diff content must never remain visible after shorter redraws

Source-specific requirements:

- commit-backed diffs should expose jump targets for hunk navigation
- full-file mode should overlay changed/deleted context where available
- untracked files should render as insertion-style full content
- when no meaningful diff exists, the UI should say so explicitly rather than rendering an empty pane

### Branch Actions

The reviewer must support branch switching actions on the selected repo.

Requirements:

- show branch list dialog
- show current branch
- show local and eligible remote branch choices
- block branch switching when repo is dirty
- show confirm dialog before switch
- show success/failure feedback

Dialog details:

- branch list should surface repo label and current branch before options
- branch list should be able to show a scrollable/selectable list of branch labels
- branch confirm should clearly show source and destination branch

Repo hygiene rule:

- branch switching must refuse to proceed when the selected repo has staged, unstaged, or untracked changes

## 8.11 Dialogs, Menus, and Notifications

The app has context menus, dialogs, and notifications. The GUI implementation should keep these as reusable egui interaction patterns.

### Context Menus

Required menus:

- workspace tab menu
- directory menu
- markdown file menu

Requirements:

- open at pointer location
- dismiss on outside click
- trigger the selected action

Sizing guidance from current product:

- workspace menu is small and tightly scoped
- outline file menu is larger because it includes multiple actions
- menus should remain compact and not appear as full dialogs

### Confirm Dialogs

Required confirms:

- close workspace
- delete markdown file
- branch switch

Requirements:

- consistent visual treatment
- primary and secondary actions
- support keyboard confirm/cancel

Layout guidance:

- dialogs are centered overlays
- confirm dialogs should be compact, not page-like
- destructive target context, such as file path or workspace name, must be visible in the dialog body

### Input Dialogs

Required initially:

- create markdown file

Requirements:

- inline validation
- file name normalization to `.md`
- duplicate file prevention

Layout guidance:

- creation dialogs should be short, single-purpose, and compact
- the focus should land directly in the text input control

### Message Feedback

Need two categories:

- transient toasts for low-risk success feedback
- blocking dialogs for destructive or failure states

Typical message cases:

- copied path
- file already exists
- markdown modified warning
- branch switch failed
- reviewer load error

Additional behavior guidance:

- low-risk status messages may be transient
- errors that block user progress should be modal or visually persistent enough to require acknowledgement

Dialog and overlay behavior:

- overlays are layered, not mutually exclusive by default
- newly opened overlays appear above older overlays
- dismissing the top overlay should not incorrectly collapse underlying route state

## 8.12 Clipboard Operations

The app must support clipboard access for:

- editor copy/paste
- copy absolute path
- copy relative path

If clipboard access fails, the app must surface a user-visible error.

## 8.13 Persistence

The app must persist enough UI state to make reopening the app feel continuous.

Required persistence:

- open workspace paths
- active workspace index
- agent session id metadata per workspace
- enough state to restore the agent-first workspace model

Preferred persistence:

- last active file per workspace
- outline expansion state per workspace
- last center view per workspace
- workspace terminal restoration metadata where technically feasible

The persistence format should remain explicit and debuggable.

Persistence nuance:

- session metadata may outlive live status availability
- the app should preserve known session ids even when the current status feed is temporarily missing

## 8.14 Agent Lifecycle and Status

The agent is not only status metadata. It has a real lifecycle inside each workspace.

Requirements:

- represent agent provider / kind at the workspace level
- preserve session id metadata
- support creating or restoring an agent session
- show at least unknown / idle / busy
- surface when stored session metadata exists but live status cannot currently be read
- support external hook/status integration when available

The agent lifecycle should be modeled as:

- no known session
- known session, status unavailable
- idle
- busy
- errored / unavailable

Status correction nuance:

- if external status says `busy` but transcript metadata proves the active turn was aborted, the app should prefer a recovered idle interpretation instead of showing a stale busy state

## 8.15 Agent Status Metadata

The current app reads external agent status snapshots and associates them with workspaces.

Requirements:

- preserve agent status as a workspace-level concept
- show at least unknown / idle / busy
- preserve hook-based status ingestion where practical

## 8.16 Module Relationships

The module relationships in the product should be understood as follows:

- `Workspace` is the top-level user context boundary.
- `Agent` is the primary operational surface within a workspace.
- `Workspace Global Terminal` is the persistent shell context for the same workspace.
- `Outline` is the structural navigator for workspace documents.
- `Editor` and `Preview` are document working surfaces subordinate to the active workspace.
- `Reviewer` is the validation and inspection route subordinate to the active workspace.

Operational relationships:

- the agent works inside a workspace
- the workspace terminal works inside the same workspace
- documents belong to the workspace and support agent work
- reviewer inspects the same workspace state and repo state
- workspace tabs switch the whole bundle together

This means the app should not be designed as unrelated panes. It should be designed as a workspace container whose submodules stay aligned.

## 8.17 Concrete Layout Rules

The following layout rules should be treated as product guidance, not incidental implementation details.

### Main Window

- the app should feel optimized for a wide desktop window
- the default composition assumes enough width for three primary vertical regions:
  - workspace rail
  - outline
  - dominant center surface

### Center Surface Switching

- center surfaces are mutually exclusive at any moment
- agent, workspace terminal, editor, and preview are peer center modes
- reviewer is a route replacement for the normal workspace center composition, not a small embedded panel
- switching center surfaces should not reset unrelated workspace state

### Popup Hierarchy

- popups layer above the active workspace view
- later popups render above earlier ones
- popup stack semantics matter and should be preserved conceptually in the GUI rewrite
- popup dismissal should be local to the popup stack and not accidentally close the reviewer route or workspace

### Default Popup Scale

Current product behavior suggests three popup scales:

- small context menus for pointer-local actions
- medium confirm/input dialogs for focused tasks
- large branch/help dialogs for list-heavy or instructional content

This size hierarchy should remain obvious in the redesign.

## 8.18 Reviewer Data Semantics

The reviewer UI should reflect the semantics of its underlying data, not only its layout.

### GSD Reviewer Data

- derives from phase directories under `.planning/phases` or workstream-specific phase roots
- groups work by plan and task provenance
- includes provenance status, summary status, commit provenance, file hints, and repo buckets
- may represent missing, bad, commit-backed, or fallback provenance states

### Git Reviewer Data

- discovers repos recursively from the workspace root
- skips obvious generated directories during repo discovery
- treats nested repos as first-class review units
- shows real commits plus a synthetic "uncommitted changes" item when dirty files exist

### Diff Semantics

- files can be commit-backed
- working-tree backed
- or fallback/non-commit backed
- the UI should surface these distinctions in labels, states, or explanatory copy where helpful

## 8.19 Refresh Behavior

The app currently refreshes outline and overview data periodically and agent status more frequently.

The desktop rewrite must support background refresh for:

- outline tree changes
- overview/status source data
- reviewer reloads when requested
- workspace activity status
- agent status and session restoration metadata

Requirements:

- UI must remain responsive during refresh
- transient reload states should be visible where necessary
- avoid full-window flicker

## 9. User Flows

## 9.1 Open App With Existing Workspaces

1. User launches the app.
2. App restores persisted workspaces.
3. App restores the last active workspace.
4. Workspace tabs, outline, and center panel render immediately.
5. Background refresh updates outline and activity state.

## 9.2 Add New Workspace

1. User triggers "Add workspace".
2. Native directory picker opens.
3. User selects a repo/workspace root.
4. If already open, app activates existing tab.
5. If new, app creates a new workspace tab and activates it.

## 9.3 Resume Or Start Agent Work In A Workspace

1. User opens the app or switches to a workspace.
2. The workspace restores its prior agent metadata if available.
3. The center surface defaults to the agent when appropriate.
4. The app shows whether the agent is idle, busy, unknown, or restorable.
5. The user can continue work in the same workspace without losing surrounding document and reviewer context.

## 9.4 Switch To Workspace Global Terminal

1. User is inside an active workspace.
2. User switches center view to the workspace global terminal.
3. The terminal opens in the workspace's shell context.
4. If the user leaves and returns later, the terminal context is still associated with that workspace.

## 9.5 Open and Edit Markdown

1. User selects a markdown file in outline.
2. Editor opens.
3. User edits content.
4. Dirty state appears.
5. User saves.
6. Dirty state clears.
7. Preview can be toggled at any time.

## 9.6 Create New Markdown File

1. User right-clicks a directory in outline.
2. Context menu shows "New markdown".
3. User enters a file name.
4. App normalizes `.md` extension.
5. App creates the file.
6. App opens the new file in editor.

## 9.7 Delete Markdown File

1. User right-clicks a file.
2. User chooses delete.
3. Confirm dialog appears.
4. If confirmed, file is deleted and outline refreshes.
5. If deleted file was active, clear document state and return to default center panel.

## 9.8 Review Project Data

1. User opens reviewer.
2. App enters fullscreen reviewer route.
3. Reviewer loads data for GSD or Git mode.
4. User navigates rows and columns.
5. User inspects diff or full file.
6. User exits back to previous workspace center panel.

## 9.9 Switch Branch From Reviewer

1. User selects repo in reviewer.
2. User opens branch action.
3. App loads current branch and choices.
4. If repo is dirty, show blocking dirty dialog.
5. Else show branch list dialog.
6. User selects target branch.
7. Confirm dialog appears.
8. If confirmed, app executes switch.
9. Reviewer refreshes and result is surfaced.

## 10. Navigation and Input Model

The GUI app must support both mouse-first and keyboard-first operation.

### Required Keyboard Support

- switch workspace
- switch between agent / terminal / editor / preview center views
- move outline selection
- expand/collapse outline dirs
- open selected file
- save editor
- toggle editor/preview
- open reviewer
- reviewer row/column movement
- reviewer reload
- confirm/cancel dialogs
- dismiss transient overlays

### Required Mouse Support

- workspace tab click
- outline click
- context menu invocation
- row selection in reviewer
- dialog button activation
- scrolling in long panels

Panel-specific expectations:

- mouse wheel scrolls preview
- mouse wheel scrolls editor
- mouse wheel scrolls reviewer columns based on hovered region
- right-click opens context menus in tabs and outline

### Input Philosophy

- Core actions must be reachable by obvious UI controls.
- Shortcuts are accelerators, not the only access path.

## 11. Empty, Error, and Edge States

The app must explicitly design for the following states:

- no workspaces open
- workspace path no longer exists
- agent session exists but is not currently resumable
- agent session metadata exists but status feed is unavailable
- workspace terminal unavailable
- no markdown files
- file read failure
- file write failure
- clipboard failure
- reviewer loading
- reviewer failed to load
- reviewer requires wider area
- no branch choices
- dirty repo blocks branch switch
- unsaved editor blocks close/switch
- stored workspace session id exists but live status is unavailable

Layout-sensitive edge states:

- reviewer popup dismissal must not accidentally exit reviewer route
- title rows and border rows are non-content chrome and should not receive row selection behavior
- modal actions should still work in relatively small windows without clipping the critical buttons

## 12. Visual and UX Direction

The GUI rewrite should feel like a serious desktop tool, not a toy dashboard.

Desired characteristics:

- dense but calm
- restrained color use
- strong hierarchy
- clear active states
- minimal decorative noise
- optimized for long sessions

The product should not mimic a terminal visually just because the old version was a TUI.

## 13. Accessibility and Usability

Requirements:

- readable default typography
- high-contrast selection states
- visible focus states
- minimum hit area for menu actions and dialog buttons
- no information encoded only by color
- support standard copy/paste/select text conventions where feasible

## 14. Performance Requirements

- App launch should feel immediate for a normal workspace count.
- Switching workspaces should not visibly rebuild the whole app.
- Large markdown previews must remain scrollable.
- Reviewer navigation must remain responsive even with large diffs.
- Background refresh must not freeze pointer or keyboard interactions.
- popup redraws must not leave stale visual artifacts behind

## 15. Technical Constraints

- single process
- Rust implementation
- `egui` / `eframe` app shell
- no dependency on tmux
- preserve current reviewer/domain logic where practical instead of rewriting all business logic
- preserve workspace-scoped agent and terminal concepts in the product architecture

## 16. Deferred Features

These are intentionally deferred from the initial `egui` release:

- terminal-based create-workspace flow
- terminal escape-sequence compatibility layers
- advanced docking and user-rearrangeable panes

## 17. Success Criteria

The rewrite is successful when:

- the agent remains the perceived center of the product
- the workspace-global terminal remains part of the product, even if implementation stages are phased
- multi-workspace behavior is preserved
- markdown editing and preview feel at least as usable as the TUI version
- reviewer remains dense, legible, and fully navigable
- destructive actions are clearer and safer than before
- no core workflow depends on terminal quirks

## 18. Release Scope Recommendation

### Version 1 Must Include

- app shell
- workspace persistence
- workspace tab rail
- agent-first center surface
- workspace-global terminal surface or a clearly scoped implementation-equivalent for it
- outline tree
- create/delete markdown
- copy absolute/relative path
- markdown editor
- markdown preview
- unified dialog/menu/toast system
- fullscreen reviewer
- branch list / dirty / confirm flows
- activity status display

### Version 1 Must Exclude

- temporary terminal-era compatibility workarounds that do not fit a desktop-native model

### Version 2 Candidates

- optional dockable panes
- richer editor features
- overview panel revival
- drag-and-drop workspace open
- richer status/history panel

## 19. Source Features Inventory Reference

This PRD was derived from the current product behavior in:

- [AGENTS.md](../AGENTS.md)
- [src/bin/gsdv-gui.rs](../src/bin/gsdv-gui.rs)
- [src/gui/app.rs](../src/gui/app.rs)
- [src/gui/data.rs](../src/gui/data.rs)
- [src/gui/terminal_host.rs](../src/gui/terminal_host.rs)
- [src/reviewer/app.rs](../src/reviewer/app.rs)
- [docs/egui-ui-audit.md](./egui-ui-audit.md)

This document should be treated as the design-generation source of truth for the `egui` rewrite unless a later product revision supersedes it.

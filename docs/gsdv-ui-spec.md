# GSDV UI Spec

> 历史说明：本文档描述旧 CLI status viewer，不是当前桌面 GUI 架构。
> 当前 egui workspace/product 行为见 `PRD-egui-app.md`，当前 UI 事件模型见
> `gui-event-architecture.md`。

## Purpose

`gsdv` is a lightweight Rust CLI that renders a live GSD project status view in a terminal without using a TUI framework.

The viewer is optimized for:

- continuous monitoring
- a fixed high-density layout
- polling-based updates every second by default
- operation against a caller-specified project root

## CLI

### Flags

- `--project-dir <DIR>`
  - Primary project root flag
  - Points at the repo or workspace root, not the `.planning/` directory itself
  - The viewer watches this directory continuously
- `-pd <DIR>`
  - Compatibility alias implemented via argument normalization
- `--gsd-dir <DIR>`
  - Backward-compatible alias
- `--refresh-secs <SECONDS>`
  - Poll interval
  - Default: `1`
- `--install`
  - Registers the current executable path for Codex hook callbacks

### Hook Registration

- Writes `~/.gsdv/hooks/agent-status-hook`
- Updates `~/.codex/config.toml` and `~/.codex/hooks.json`
- Does not copy the binary into a system install directory

## Refresh Model

The viewer re-polls on every refresh cycle.

This applies to all states:

- project directory exists and contains `.planning`
- project directory exists but `.planning` does not exist yet
- project directory itself does not exist yet

The tool must not exit just because the watched directory or `.planning` is missing. It remains open and switches state automatically when the directory structure appears.

## Working Directory Rule

All GSD reads must be scoped to the selected project root.

For every `gsd-tools` invocation:

- pass the chosen project root through `--cwd`
- set the spawned process `current_dir(...)` to the same project root

This is required because the user may launch `gsdv` from outside the target repo.

## Layout

The UI is a fixed 6-line terminal view.

### Normal GSD View

Line 1:

- project anchor
- optional workstream tabs

Examples:

```text
[gsdv]
[gsdv]  alpha 62% | beta 41% | release 88%
```

Line 2:

- milestone version
- milestone percent
- completed phases / total phases
- current phase
- open gaps

Example:

```text
MS v0.3 62%  5/8  Cur:P4 Execute  Gaps:1
```

Line 3:

- lifecycle rollup percentages

Example:

```text
D 100% | P 75% | E 50% | V 25%
```

Line 4:

- compressed phase status tokens
- only contiguous ranges may be merged

Example:

```text
P1-2_[✓]   P3_V[G3:2/3]   P4_E[V]   P5-8_D[PEV]
```

Line 5:

- current phase summary
- current phase title
- current phase percent
- plan/summary counts

Example:

```text
P4 "Progress CLI" 58%   plans 2/3   sums 1/3
```

Line 6:

- verify summary
- unresolved gaps
- manual UAT note when present

Example:

```text
Verify: P3 unresolved 1   manual UAT
```

### Fallback View: Missing `.planning`

Shown when the watched project directory exists but does not yet contain `.planning`.

```text
[gsdv]
without GSD
No .planning directory detected
project dir[/absolute/path]
Tab switching is hidden until workstreams exist
Press q to quit
```

### Fallback View: Missing Directory

Shown when the watched project directory itself does not yet exist.

```text
[gsdv]
without GSD
Watch directory does not exist yet
project dir[/absolute/path]
It will auto-detect the directory and .planning when they appear
Press q to quit
```

## Tabs

Tabs are shown only when multiple workstreams exist.

If there is only one workstream or flat mode is active:

- no tabs are rendered
- the first line contains only the project anchor

Tab labels include:

- workstream name
- workstream progress percent

Example:

```text
[gsdv]  alpha 62% | beta 41% | release 88%
```

Interaction:

- `Tab`: next workstream
- `Shift-Tab`: previous workstream

## Phase Token Rules

Phase tokens encode status compactly.

Examples:

- `P1-2_[✓]`
- `P3_V[G3:2/3]`
- `P4_E[V]`
- `P5-8_D[PEV]`

Meaning:

- `_` means done
- `D` means discuss-stage / not yet planned
- `P` means planned
- `E` means executing
- `V` means verifying or verify-state token
- `G` means gaps discovered during verification

### Compression Rule

Only contiguous phases with identical rendered status may be merged.

Allowed:

```text
P1-3_[✓]
P5-8_D[PEV]
```

Not allowed:

```text
P1-3,5-8_[✓]
```

If the current phase is inside a range, it must be split out as its own token.

## Data Sources

Primary data is derived from existing GSD artifacts and `gsd-tools`.

### Commands

- `init progress`
- `roadmap analyze`
- `state-snapshot`
- `audit-uat`
- `workstream progress`

### Files

- `.planning/ROADMAP.md`
- `.planning/STATE.md`
- `.planning/phases/.../*-PLAN.md`
- `.planning/phases/.../*-SUMMARY.md`
- `.planning/phases/.../*-UAT.md`
- `.planning/phases/.../*-VERIFICATION.md`

### Derived Values

- milestone percent
- current phase step
- lifecycle rollup percentages
- compressed phase token ranges
- unresolved gap counts
- manual UAT indicator

## Color Direction

Truecolor ANSI is used.

Palette:

- project / headings: light blue
- current / active: blue
- completed: green
- verify stage: amber
- unresolved gaps: red
- fallback / planned / helper copy: gray
- manual UAT note: orange

## Terminal Behavior

- alternate screen enabled
- raw mode enabled
- cursor hidden while active
- full-screen repaint on every refresh
- quit keys:
  - `q`
  - `Esc`
  - `Ctrl-C`

## Current Implementation Notes

- no TUI crate is used
- line output uses `\r\n` to avoid column drift in raw mode
- missing-directory and missing-`.planning` states are intentionally non-fatal
- install path is platform-sensitive

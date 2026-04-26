# Repository Guidelines

## Product Design

`gsdv` is a single-process egui desktop workspace tool for navigating Markdown project files, editing Markdown, previewing rendered Markdown, inspecting GSD/git reviewer data, and running embedded mature terminal surfaces.

The main UI is organized as:

```text
workspace rail | outline panel | active workspace view
```

Workspace entries are vertical on the left. Each workspace has its own root directory, route state, dialog stack, Markdown editor/preview state, reviewer state, agent terminal, and workspace terminal. Closing a workspace switches to the tab on the left if possible, otherwise the right; closing the final workspace exits.

The active workspace view contains an outline tree and a center view. Center modes are Agent, Terminal, Editor, Preview, and fullscreen Reviewer route.

## Technical Architecture

This app must stay free of tmux and legacy TUI dependencies. The architecture is:

- `GsdvGuiApp` 管理 workspace rail、active workspace、dialogs、route state，以及顶层 egui render flow。
- 所有影响 UI 可见状态的变化必须进入唯一 `AppEvent` 队列，并由 app event drain 消费。Producer 可以自行防抖、合并或丢弃高频信号，但不能从 callback、后台任务或独立通道直接修改 app UI 状态。
- `AppEvent` handler 必须保持轻量：只能更新内存中的渲染状态，或派发异步/后台工作；不能做阻塞 IO、重 CPU、递归扫描，或等待后台结果。慢工作完成后再投递新的 `AppEvent`。
- egui 原生输入由 input runtime 独立解析；主 drain 只接收解析后的 UI 命令、terminal 输入字节、截图结果等具体 `AppEvent`。
- Repaint request 只是 egui 唤醒信号，不能承载业务语义；任何影响渲染的状态变化都必须用事件表达。
- 持久化是独立业务副作用。Store 写入由 UI 状态 owner 标记 dirty，再交给持久化 writer 合并处理，不能直接在 `AppEvent` drain 中写盘。
- `WorkspaceViewData` 存储从 `~/.gsdv/store` 和文件系统 outline 加载的 workspace metadata。
- `Route` 选择 workspace-level mode，例如普通 workspace view 或 fullscreen reviewer。
- Workspace-level dialog 是只作用于 active workspace 的 egui modal window。
- `GuiTerminalHost` 通过直接 `alacritty_terminal` 集成嵌入子进程；绘制路径只渲染，不消费 PTY runtime events。
- Markdown editing 使用 egui native multiline text edit。
- Markdown preview 使用 `egui_commonmark`。
- Reviewer logic 位于 `src/reviewer/`，负责 git review、provenance、diff 和 branch switching。
- UI 事件循环、store writer、terminal PTY drain 的约束见 `docs/gui-event-architecture.md`。

New UI work should follow the design assets in `docs/design/`: white/light surface system, `#2563EB` primary, shallow borders, 8px spacing rhythm, and dense-but-calm desktop layout.

## Project Structure

- `src/bin/gsdv-gui.rs`: egui binary entrypoint.
- `src/gui/app.rs`: app shell, workspace rail, outline, routes, dialogs, editor, preview, reviewer host, terminal surfaces.
- `src/gui/data.rs`: initial workspace data, store persistence, outline loading.
- `src/gui/terminal_host.rs`: direct `alacritty_terminal` terminal host and renderer.
- `src/gui/theme.rs`: egui style and design tokens.
- `src/gui/reviewer_adapter.rs`: bridge from reviewer runtime to egui snapshots/actions.
- `src/scrolling.rs`: shared scroll helpers.
- `src/reviewer/`: reviewer UI, git logic, diff/provenance support.
- `docs/`: UI and design notes.
- `reset.md`: historical reset plan, not current source of truth.

## Build, Test, and Run

Use Cargo from the repository root:

```sh
cargo check
cargo test
cargo fmt
cargo run --bin gsdv-gui
```

`cargo check` verifies compilation. `cargo test` runs unit tests, including reviewer git fixture tests. `cargo fmt` applies Rust formatting.

## codex rust code

if need clone it to /tmp/codex-rs

## Coding Style

Use idiomatic Rust and `rustfmt`. Prefer explicit state structs and small helper modules. Keep UI state transitions centralized in `App` or the relevant route. Avoid hidden file state for UI behavior unless it is unavoidable.

Do not reintroduce tmux, tmux panes, tmux key bindings, pane titles, Ratatui, `portable-pty`, `vt100`, `tui-term`, or `tui-textarea`. Interactive processes should use `GuiTerminalHost`.

## Testing Guidelines

Add focused tests near the code they cover. Use names that describe behavior, for example:

- `workspace_name_uses_last_two_path_segments`
- `preview_mouse_wheel_scrolls_markdown_panel`
- `branch_choices_include_remote_without_local_same_name`

Use egui-friendly state/unit tests for UI logic and temporary git fixtures for reviewer branch/diff behavior.

## Workflow Notes

When changing shortcuts, routing, dialogs, terminal input, or scrolling, add a regression test when practical. These areas are easy to break because the app combines egui layout, terminal embedding, reviewer state, and mouse input.

For visible UI changes, include screenshots or a short terminal recording in PRs when practical. Summaries should mention behavior changes, tests run, and any known limitations.

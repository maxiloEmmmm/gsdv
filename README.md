# gsdv

## 概述

`gsdv` 是一个基于 `egui` 的桌面工作台，用来在 Markdown 项目里同时处理文件、任务流、Agent、终端、Reviewer 和脚本输出。

它的核心布局是：

```text
workspace rail | outline / work-flow | active workspace view
```

每个 workspace 都有独立的文件选择、Markdown 编辑状态、Agent 会话、终端、Reviewer 状态、通知上下文和对话框状态。

## 特色

- 单进程桌面应用: 不依赖 tmux、Ratatui 或旧 TUI pane。
- 多 workspace: 左侧 rail 管理多个项目，支持快速切换和关闭。
- Agent 优先: 每个 workspace 默认进入嵌入式 Agent 终端。
- Markdown 原生编辑: 文件树、编辑器、预览、最近文件、diff 历史一体化。
- Work-flow 任务树: 读取当前项目的 `./gsdv-spec`，渲染 project / task / step。
- Reviewer 路由: 全屏查看 GSD/git reviewer 数据，支持键盘导航和分支切换。
- 外置脚本工具: 支持 reviewer 脚本和 Agent 主界面的 extra tools。
- 自动集成: 启动时安装 agent status hook 和内置 `gsdv-wf` skill。
- 多语言界面: 支持 English / 中文 / 日本語。
- 密集桌面 UI: 面向长期工作流，不是展示页风格，启动就是工作台。

## 功能点

workspace: 左侧 rail 管理项目目录，每个项目独立保存 UI 状态。

outline: 展示当前 workspace 的 Markdown 文件树，支持打开、预览、新建、重命名、删除、收藏、复制路径、刷新和系统文件管理器定位。

work-flow: 在 outline 面板上方切换到 `Work-flow` tab 后，按 `./gsdv-spec` 渲染 project / task / step 树。

workflow project: project 行可折叠，右键可打开项目 root、重命名、新增 task、删除、复制逻辑路径。

workflow task: task 可进入专属 task 页面，右键可新增 step、重命名、删除、复制逻辑路径。

workflow step: step 可编辑对应 doc 和自身 desc，支持保存、重命名、删除、复制逻辑路径。

workflow path: 复制格式类似 `project > task > step` 或 `project > task > step > sub-step`。

Markdown editor: 原生多行编辑器，`Cmd/Ctrl+S` 保存当前 Markdown 或当前 workflow step 编辑内容。

Markdown preview: 渲染 Markdown 预览，支持从编辑器/预览之间快速切换。

recent Markdown: 记录最近打开的 Markdown 文件，支持快速回到文件。

Agent: 内嵌成熟 terminal 后端，当前支持 `codex` 和 `claude` 两种 agent 命令。

Agent session: 保存 workspace 级 session id，可 resume，也可清空后新开会话。

subagent: 支持创建子 Agent、切换子 Agent、移动到其它 workspace。

Agent input translate: 支持快速翻译 Agent 输入草稿，并可自动在空闲后触发翻译。

workspace terminal: 每个 workspace 有独立终端抽屉，cwd 指向 workspace 根目录。

Helix drawer: Reviewer 中可打开 Helix 抽屉处理文件目标，需要本机有 `hx`。

Reviewer: 全屏 route 查看 reviewer 数据，支持列间移动、行选择、diff/full view 切换、reload、复制 prompt 到 Agent。

branch switcher: Reviewer 中可打开分支切换器，支持本地分支和远程分支 checkout。

reviewer scripts: 在 reviewer 行右键执行 `.sh` 脚本，输出进入通知抽屉。

extra tools: Agent 主界面支持外置工具抽屉，扫描全局和 workspace 下的 `.sh` 脚本，支持 card 和 switch 两种展示。

notifications: 全局通知抽屉集中展示脚本输出、错误和运行状态。

settings: 配置主题、语言、FPS、Agent idle go、快捷回复、自动翻译、番茄钟、字体和网络代理。

i18n: UI 文案支持 English / 中文 / 日本語，设置后立即生效并保存。

proxy: 支持 HTTP / HTTPS / SOCKS 代理，并传给新启动的 Agent、终端、Helix 和脚本。

pomodoro: 内置轻量番茄钟，支持工作/休息状态和手动开始工作或休息。

screenshots: 支持 egui 截图，输出到 `target/gsdv-screenshots`。

app data: 应用状态默认保存在 `~/.gsdv`。

hooks: 启动时安装 `~/.gsdv/hooks/agent-status-hook`，并写入 Codex / Claude 对应 hook 配置。

skills: 启动时安装内置 `gsdv-wf` skill 到 `~/.codex/skills/gsdv-wf` 和 `~/.claude/skills/gsdv-wf`。

## gsdv-spec 约定

root: `./gsdv-spec/root.md` 是整个项目的根说明。

project: `./gsdv-spec/ps/<project>/root.md` 是某个 project 的说明。

task: `./gsdv-spec/ps/<project>/task-*.md` 是 task 文件。

steps: task 文件里的 `--steps--` 区块描述 step 树。

doc: task 文件里的 `--doc--` 区块描述顶层 step 对应的 doc 内容。

leaf step: 叶子 step 可以点开编辑；非叶子顶层 step 主要编辑 doc desc。

desc: step 自身 desc 和 doc desc 由 workflow 编辑器分别展示并一起保存。

## 外置工具约定

global tools: 放在 `~/.gsdv/extra`。

workspace tools: 放在 workspace 根目录。

script: 只扫描直属 `.sh` 文件。

metadata: 脚本通过 metadata 输出描述 UI 类型、action、输入框和刷新间隔。

type: `card` 表示卡片工具，`switch` 表示布尔开关工具。

action: action 按钮执行脚本，并通过环境变量传入当前动作。

input: 需要输入时，输入内容通过 `__gsdv_input` 传给脚本。

## 常用快捷键

add workspace: `Cmd/Ctrl+O`

settings: `Cmd/Ctrl+,`

help: macOS `Cmd+.` / `Alt+.`，其它平台 `Alt+.`

save: `Cmd/Ctrl+S`

copy workflow path: `Cmd/Ctrl+C`

agent / Markdown: `Cmd/Ctrl+W` 或 `Alt+W`

workspace terminal: `Cmd/Ctrl+T` 或 `Alt+T`

notifications: `Cmd/Ctrl+K` 或 `Alt+K`

extra tools: `Cmd/Alt+B`

screenshot: `Cmd/Ctrl+Shift+P`

main agent: `Cmd/Alt+1`

subagent 1: `Cmd/Alt+2`

subagent 2: `Cmd/Alt+3`

paste recent Markdown diffs: `Cmd/Alt+4`

translate Agent input: `Cmd/Alt+M`

apply Agent translation: `Cmd/Alt+N`

open Reviewer: `Cmd/Ctrl+Enter` 或 `Cmd/Ctrl/Alt+R`

exit Reviewer: `Esc` 或 `Cmd/Ctrl/Alt+R`

Reviewer navigation: `Left` / `Right` / `Tab` / `Shift+Tab` / `Up` / `Down`

Reviewer actions: `F` 切换 diff/full，`N` / `Shift+N` 跳转 block，`C` 复制 prompt，`R` reload，`B` 分支切换。

## 安装和启动

build: `cargo build --release --bin gsdv`

run: `cargo run --bin gsdv`

select agent: `cargo run --bin gsdv -- --agent codex`

pass agent args: `cargo run --bin gsdv -- --coder-arg --model --coder-arg gpt-5.3-codex`

format: `cargo fmt`

check: `cargo check`

test: `cargo test`

## 数据目录

store: `~/.gsdv/store`

hooks: `~/.gsdv/hooks`

reviewer scripts: `~/.gsdv/reviewer`

extra tools: `~/.gsdv/extra`

agent status: `~/.gsdv/agent-status.json`

codex skill: `~/.codex/skills/gsdv-wf`

claude skill: `~/.claude/skills/gsdv-wf`

## 依赖

required: Rust toolchain。

desktop: 需要 `eframe` 支持的桌面环境。

optional codex: 使用 Codex Agent 时需要 `codex` 命令在 `PATH`。

optional claude: 使用 Claude Agent 时需要 `claude` 命令在 `PATH`。

optional helix: 使用 Helix 抽屉时需要 `hx` 命令在 `PATH`。

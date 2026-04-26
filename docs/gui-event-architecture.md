# GUI 事件架构

## 核心规则

`GsdvGuiApp::update` 只有一个 UI 状态修改入口：

```text
process_update_events
  -> enqueue_update_events
  -> send_input_runtime_request
  -> drain_app_events
  -> handle_app_event
```

任何会影响界面可见状态的变化，都必须进入唯一 `AppEvent` 队列。
callback、后台任务、watcher callback、terminal producer 都不能直接修改
`GsdvGuiApp` 的 UI 状态。

`request_repaint` 和 `request_repaint_after` 只是唤醒 egui。它们不能承载业务语义。
如果下一帧渲染应该变化，就必须有事件，或者是当前 UI pass 已经明确修改了自己拥有的内存状态。

## Producer 边界

Producer 可以在投递 `AppEvent` 前自行防抖、合并或丢弃高频信号。
这些策略属于 producer，不属于核心队列。核心队列保持单通道 FIFO，不定义优先级 lane。

典型来源：

- filesystem watcher callback 投递 `AppEvent::FsWatch`
- 后台任务投递 `DocumentLoaded` 这类完成事件
- reviewer script 输出投递 `AppEvent::Notification`
- input runtime 投递 `InputUiCommand`、`InputTerminalBytes`、`ScreenshotCaptured`
- frame-local 到期检查投递 `ProcessFsWatchDirty` 这类轻量事件

producer 可以在投递事件后唤醒 egui，但 repaint 只表示“需要 update”，不表示业务状态。

## Input 边界

egui 原生输入快照不是 UI 状态事件。`GsdvGuiApp::update` 只能把
`InputState` 快照交给 input runtime，不能在主 drain 里解析 keyboard、terminal
bytes、截图完成或番茄钟输入。

input runtime 负责独立消费原生输入，并只把需要修改 UI 状态或触发 UI 副作用的结果
投回 `AppEvent`：

- 快捷键解析成 `InputUiCommand`
- reviewer diff 键盘动作解析成 `InputReviewerDiffAction`
- terminal 输入解析成 `InputTerminalBytes`
- 截图完成解析成 `ScreenshotCaptured`
- 番茄钟相关输入解析成 `PomodoroInputDetected`

没有产生 UI 状态变化的输入不需要投递事件。

## Drain 边界

`drain_app_events` 只做四件事：

1. `try_recv` 取一个事件
2. 交给 `handle_app_event`
3. 检查当前帧预算
4. 预算耗尽时请求下一次 repaint

这里不能判断业务细节，不能做 IO，不能扫描文件系统，不能解析大 payload，也不能等待后台结果。

## Handler 边界

`handle_app_event` 只能做这些事：

- 修改内存中的渲染状态或 UI 状态
- 标记 UI 状态 owner 自己的轻量 dirty flag
- 派发 async/background work
- 把 repaint 当作唤醒请求

这里不能做阻塞 IO、重 CPU、递归目录扫描、子进程等待或同步持久化。
慢工作必须 spawn，完成后再通过新的 `AppEvent` 回来。

## 持久化边界

workspace store 持久化不是 `AppEvent`。UI 状态 owner 在状态需要保存时调用
`mark_workspace_store_dirty`。store writer 负责合并 dirty 标记，并在后台任务中写入克隆出来的快照。

路径是：

```text
UI 状态改变
  -> 标记 dirty
  -> writer 防抖/合并
  -> 后台保存
```

连续 dirty 标记不能变成连续同步写盘。

## Terminal 边界

`GuiTerminalHost` 自己拥有 terminal runtime drainer，用来消费
`alacritty_terminal` 的原始 PTY event。这个 drainer 只处理 terminal 内部协议细节，
例如 color query、clipboard、text area size、child exit 和 OSC title。

原始 PTY event 不能进入 app event drain，也不能让 app 循环所有 host 来消费。
如果 terminal runtime 处理后需要影响界面，它只能投递粗粒度 `AppEvent::TerminalRuntime`：

- `Output`：终端有新输出，App 可以更新 agent busy watchdog
- `StateChanged`：标题、退出状态等可见摘要变化
- `Repaint`：bell、光标闪烁等只需要唤醒重绘

terminal 绘制路径不能 drain PTY runtime events。绘制路径可以画已经拥有的 terminal grid，
也可以提交 UI-only 的延迟剪贴板状态，但不能消费 terminal 内部队列。

## 禁止模式

- 在 `app_event_rx` 之外再加一条顶层 UI 状态 receiver
- 在 `process_update_events` 里直接 drain filesystem、notification、background 或 terminal 状态
- 用 repaint 代替状态事件
- 从 `handle_app_event` 里写磁盘
- 从 terminal runtime drainer 里扫 transcript、递归扫文件或解析大 JSON
- 从 terminal render code 里消费 PTY events

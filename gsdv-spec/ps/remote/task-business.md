梳理 remote 能力需要暴露的业务数据与 agent 操作语义。

## [x] 定义 workspace 元数据范围
确认 API 需要返回的 workspace 数据边界。

已明确需要包含：

- 所有 workspace
- 每个 workspace 的 row
- 每个 row 的 col
- 每个 col 下 agent 的数据

待确认：

- workspace 标识字段：`workspace_id`，使用 gsdv 现有 workspace list 的来源；由 `~/.gsdv/store` 的 `workspaces[].path` 加载成 `WorkspaceViewData.path` 后，取 `workspace_store_key(path)` 作为对外稳定标识
- row/col 标识字段：`row_index` 使用 `workspace.agent_rows` 当前数组下标；`col_index` 使用 `row.columns` 当前数组下标；列内部的 agent 使用 agent 自身 id
- agent 标识字段：`agent_id`；主 agent 使用 `WorkspaceViewData.agent_id`，subagent 使用 `SubagentViewData.agent_id`
- agent title 来源：remote 返回字段名为 `title`；主 agent 使用固定字符串 `main`，subagent 使用 `SubagentViewData.name`
- agent 当前状态：暂时不返回
- agent 所属路径、模型、会话信息：暂时不返回，当前只保留已确认的最小元数据

## [x] 定义 agent 输入能力
确认远程调用 agent 时的业务动作。

已明确需要支持：

- 给某个 agent 发送文本输入
- 给某个 agent 发送 esc
- 给某个 agent 发送图

待确认：

- 文本输入提交语义：复用现有右键快捷输入行为，先粘贴文本，再提交当前 Agent 输入；提交按键由 terminal host 决定，kitty keyboard protocol 下发送 `ESC [ 13 u`，否则发送 `\r`
- 文本输入实现约束：抽出共享封装，由右键快捷输入和 remote 文本输入共同调用，避免两边各自拼字节导致语义漂移
- esc 操作语义：不是发送键盘 Escape；remote 的 esc 动作需要达到现有 Ctrl/Cmd+C 中断效果，即向目标 agent terminal 发送 ETX `0x03`
- esc 实现约束：抽出共享 agent interrupt 封装，remote esc 和现有 Ctrl/Cmd+C 中断路径保持同一语义
- 图片传输格式：暂不定义，只记录现有 `copy_image` 需求
- 图片发送到 agent 的最终输入形态：如复用 `egui::Context::copy_image`，输入最终需要变成 `egui::ColorImage`；也就是需要宽、高和 RGBA 像素数据，或可解码成 RGBA 的 PNG/JPEG 等图片字节；`copy_image` 只负责写入系统剪贴板，不等同于已发送给 agent
- agent 不存在或未启动时的业务错误：直接返回找不到错误，不区分 agent id 不存在和 terminal host 未就绪

## [x] 定义 agent 输出订阅语义
确认 WebSocket 绑定某个 agent 后的数据流规则。

已明确需要支持：

- 连接后推送当前 agent 已有全部输出
- 后续输出更新继续推送

待确认：

- 输出格式：结构化 terminal 数据；Web 侧会用类似 terminal renderer 的 JS 库还原 terminal 效果，因此不能只返回纯文本
- 初始全量输出大小：不设额外上限，连接后推送当前 agent terminal buffer 全量结构化数据
- 增量推送消息格式：只推 append-only 新增行；以 terminal scrollback 底部新增行为准，不为原地改写、spinner、进度条、全屏 TUI repaint、光标移动修改旧行等场景额外推 patch
- agent 重启、关闭、切换 tab 时的连接行为：WebSocket 绑定 agent 本身；agent 重启或关闭时关闭 WebSocket；gsdv GUI 切换 tab 不影响连接

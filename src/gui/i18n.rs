use crate::gui::data::AppLanguage;

/// 返回所有可选界面语言，适用于设置页下拉框。
pub(crate) fn app_languages() -> &'static [AppLanguage] {
    &[
        AppLanguage::English,
        AppLanguage::Chinese,
        AppLanguage::Japanese,
    ]
}

/// 返回语言自身的显示名，避免当前语言不可读时无法切换。
pub(crate) fn language_label(language: AppLanguage) -> &'static str {
    match language {
        AppLanguage::English => "English",
        AppLanguage::Chinese => "中文",
        AppLanguage::Japanese => "日本語",
    }
}

/// 翻译静态 GUI 文案，未收录时保留英文源文本。
pub(crate) fn text(language: AppLanguage, source: &'static str) -> &'static str {
    match language {
        AppLanguage::English => source,
        AppLanguage::Chinese => chinese_text(source),
        AppLanguage::Japanese => japanese_text(source),
    }
}

/// 翻译带命名占位符的 GUI 文案，适用于少量动态数字或名称。
pub(crate) fn text_with_arg(
    language: AppLanguage,
    source: &'static str,
    placeholder: &'static str,
    value: impl AsRef<str>,
) -> String {
    text(language, source).replace(placeholder, value.as_ref())
}

/// 返回简体中文静态文案。
fn chinese_text(source: &'static str) -> &'static str {
    match source {
        "No file" => "无文件",
        "no workspace" => "无工作区",
        "[md]-Unsaved" => "[md]-未保存",
        "[md]-Saved" => "[md]-已保存",
        "[memo]-Error" => "[memo]-错误",
        "[memo]-Saved" => "[memo]-已保存",
        "WORKSPACES" => "工作区",
        "New Workspace" => "新建工作区",
        "New workspace" => "新建工作区",
        "Close workspace" => "关闭工作区",
        "Outline" => "大纲",
        "Work-flow" => "工作流",
        "No favorites" => "暂无收藏",
        "Loading workflow..." => "正在加载工作流...",
        "No workflow projects" => "暂无工作流项目",
        "Workflow path copied" => "工作流路径已复制",
        "Copy path" => "复制路径",
        "Open project" => "打开项目",
        "Open root.md" => "打开 root.md",
        "Rename project" => "重命名项目",
        "Add project" => "新增项目",
        "Add task" => "新增 task",
        "Delete project" => "删除项目",
        "Rename task" => "重命名 task",
        "Add step" => "新增 step",
        "Delete task" => "删除 task",
        "Rename step" => "重命名 step",
        "Merge steps" => "合并 step",
        "Add child step" => "新增子 step",
        "Delete step" => "删除 step",
        "Select a workflow task." => "选择一个工作流 task。",
        "Select a workflow step to edit." => "选择一个工作流 step 进行编辑。",
        "No active workspace." => "当前没有活动工作区。",
        "No steps" => "暂无 step",
        "Notifications" => "通知",
        "{count} lines" => "{count} 行",
        "Clear" => "清空",
        "No notifications" => "暂无通知",
        "Memo" => "备忘",
        "Open a workspace" => "打开一个工作区",
        "Add a project directory to start an agent session, browse Markdown, or inspect reviewer data." => {
            "添加项目目录后即可启动 Agent 会话、浏览 Markdown 或查看 reviewer 数据。"
        }
        "Add" => "添加",
        "Recent Markdown" => "最近 Markdown",
        "Unsaved Changes" => "未保存更改",
        "Unsaved Workflow" => "未保存工作流",
        "New Workflow Project" => "新建工作流项目",
        "New Workflow Task" => "新建工作流 Task",
        "New Workflow Step" => "新建工作流 Step",
        "Rename Workflow" => "重命名工作流",
        "Merge Workflow Steps" => "合并工作流 Step",
        "Confirm Workflow Delete" => "确认删除工作流",
        "Create Markdown" => "新建 Markdown",
        "Create Folder" => "新建文件夹",
        "Rename" => "重命名",
        "Confirm Delete" => "确认删除",
        "Close Workspace" => "关闭工作区",
        "Add Subagent" => "添加子 Agent",
        "Restart Agent" => "重启 Agent",
        "Switch Agent" => "切换 Agent",
        "Switch Theme" => "切换主题",
        "Agent Exited" => "Agent 已退出",
        "Help" => "帮助",
        "Settings" => "设置",
        "About gsdv" => "关于 gsdv",
        "Codex Auth" => "Codex 认证",
        "Switch Branch" => "切换分支",
        "Confirm Branch Switch" => "确认切换分支",
        "Confirm Script" => "确认脚本",
        "Branch Error" => "分支错误",
        "Branch" => "分支",
        "Uncommitted Changes" => "未提交更改",
        "You have unsaved changes" => "你有未保存更改",
        "Save before switching files, discard edits, or stay on the current file." => {
            "切换文件前保存、丢弃编辑，或留在当前文件。"
        }
        "You have unsaved workflow changes" => "你有未保存的工作流更改",
        "Save both panes before switching, discard edits, or stay here." => {
            "切换前保存两个编辑区、丢弃编辑，或留在这里。"
        }
        "Discard" => "丢弃",
        "Save" => "保存",
        "New workflow task" => "新建工作流 task",
        "New workflow step" => "新建工作流 step",
        "Merge workflow steps" => "合并工作流 step",
        "Merge" => "合并",
        "Rename workflow project" => "重命名工作流项目",
        "Rename workflow task" => "重命名工作流 task",
        "Rename workflow step" => "重命名工作流 step",
        "Delete workflow item?" => "删除工作流项？",
        "Create New Markdown File" => "新建 Markdown 文件",
        "Create New Folder" => "新建文件夹",
        "Delete markdown file?" => "删除 Markdown 文件？",
        "Close workspace?" => "关闭工作区？",
        "This workspace no longer exists." => "这个工作区已不存在。",
        "This removes the workspace from gsdv and deletes its workspace memo file under ~/.gsdv/workspaces/<workspace-hash>/memo.md. The project directory itself is not deleted." => {
            "这会从 gsdv 移除该工作区，并删除 ~/.gsdv/workspaces/<workspace-hash>/memo.md 下的工作区备忘文件。项目目录本身不会被删除。"
        }
        "Name" => "名称",
        "Common" => "常用",
        "Location" => "位置",
        "New name" => "新名称",
        "Current path" => "当前路径",
        "PROJECT" => "PROJECT",
        "TASK" => "TASK",
        "KEY" => "KEY",
        "DESC" => "DESC",
        "CURRENT" => "CURRENT",
        "Create" => "创建",
        "Delete" => "删除",
        "New subagent" => "新建子 Agent",
        "NAME" => "名称",
        "AGENT TYPE" => "AGENT 类型",
        "SESSION ID (OPTIONAL)" => "SESSION ID（可选）",
        "resume an existing session" => "恢复已有 session",
        "Restart agent?" => "重启 Agent？",
        "Resume keeps the stored session id. New clears the stored session and starts fresh." => {
            "Resume 保留已保存的 session id。New 会清空 session 并重新开始。"
        }
        "Restart New" => "新会话重启",
        "Restart Resume" => "续接重启",
        "Switch agent?" => "切换 Agent？",
        "Switching agent type clears this workspace's stored session id and starts a new session." => {
            "切换 Agent 类型会清空该工作区保存的 session id，并启动新会话。"
        }
        "Switch" => "切换",
        "Switch theme?" => "切换主题？",
        "The active agent will gracefully exit, then resume so it can reload terminal colors for {mode} mode." => {
            "当前 Agent 会先正常退出再恢复，以便重新加载 {mode} 模式的终端颜色。"
        }
        "Agent exited abnormally" => "Agent 异常退出",
        "The embedded agent process ended with {status}. You can try the same command outside gsdv to inspect the failure." => {
            "嵌入的 Agent 进程以 {status} 结束。你可以在 gsdv 外执行同一命令排查失败原因。"
        }
        "Command" => "命令",
        "If you confirm, gsdv will start this workspace's agent again without resume. Cancel leaves the terminal as-is." => {
            "确认后，gsdv 会不带 resume 重新启动此工作区的 Agent。取消则保持终端现状。"
        }
        "Start Without Resume" => "不续接启动",
        "Theme" => "主题",
        "Design-spec light theme" => "设计规范浅色主题",
        "Terminal backend" => "终端后端",
        "Workspace persistence" => "工作区持久化",
        "Agent status" => "Agent 状态",
        "Language" => "语言",
        "Interface language" => "界面语言",
        "Applied immediately and saved globally." => "立即生效，并保存为全局设置。",
        "Default Agent" => "默认 Agent",
        "New workspace agent" => "新工作区 Agent",
        "Used when adding new workspaces and when old workspace data has no agent type." => {
            "用于新建工作区，也用于旧工作区数据缺少 agent 类型时兜底。"
        }
        "Rendering" => "渲染",
        "Max FPS" => "最大 FPS",
        "Caps application-scheduled repaint requests." => "限制应用主动请求重绘的频率。",
        "Agent" => "Agent",
        "Auto go after idle minutes" => "空闲后自动 go 的分钟数",
        "Sends go once when a Busy agent has no new output." => {
            "当 Busy 状态 Agent 没有新输出时自动发送一次 go。"
        }
        "Custom quick replies" => "自定义快捷回复",
        "one reply per line" => "每行一条回复",
        "Shown as the second row in the Agent right-click menu." => {
            "显示在 Agent 右键菜单的第二行。"
        }
        "Auto translate Agent input after idle" => "空闲后自动翻译 Agent 输入",
        "Runs Cmd/Alt+M after the Agent draft stops changing for about 500 ms." => {
            "Agent 草稿停止变化约 500 ms 后自动执行 Cmd/Alt+M。"
        }
        "Allow Codex HTTP fallback after 5 WS failures" => {
            "允许 Codex 在 5 次 WS 失败后回退到 HTTP"
        }
        "When disabled, Codex Responses keeps rebuilding WebSocket connections instead of silently using HTTP." => {
            "禁用后，Codex Responses 会持续重建 WebSocket 连接，不会静默改用 HTTP。"
        }
        "Pomodoro" => "番茄钟",
        "Enable pomodoro" => "启用番茄钟",
        "Work minutes" => "工作分钟",
        "Rest minutes" => "休息分钟",
        "Warning remaining percent" => "剩余百分比提醒",
        "Rest shows a pixel cat; state resets when gsdv restarts." => {
            "休息时显示像素猫；重启 gsdv 后状态会重置。"
        }
        "Fonts" => "字体",
        "Default fonts" => "默认字体",
        "Terminal" => "终端",
        "Markdown editor" => "Markdown 编辑器",
        "Primary font" => "主字体",
        "Fallback font" => "备用字体",
        "Network proxy" => "网络代理",
        "HTTP / HTTPS / SOCKS proxy" => "HTTP / HTTPS / SOCKS 代理",
        "Leave empty to disable proxy." => "留空则禁用代理。",
        "Generate paired HTTP/SOCKS env vars" => "生成配对的 HTTP/SOCKS 环境变量",
        "When enabled, http:// also sets all_proxy=socks5://..., and socks5:// also sets http_proxy/https_proxy=http://...." => {
            "启用后，http:// 会同时设置 all_proxy=socks5://...，socks5:// 会同时设置 http_proxy/https_proxy=http://...。"
        }
        "Additional no_proxy entries" => "额外 no_proxy 条目",
        "Built in, always included" => "内置且始终包含",
        "Effective no_proxy" => "实际 no_proxy",
        "Proxy changes are saved. Close Settings to restart open terminals, agents, and Helix sessions immediately." => {
            "代理改动已保存。关闭设置后会立即重启已打开的终端、Agent 和 Helix 会话。"
        }
        "Proxy changes apply when terminal-backed sessions start." => {
            "代理改动会在终端后端会话启动时生效。"
        }
        "Codex auth" => "Codex 认证",
        "Account" => "账号",
        "Authorizing" => "认证中",
        "Auth" => "认证",
        "Waiting" => "等待中",
        "Waiting for browser authorization" => "等待浏览器授权",
        "Codex authorization complete" => "Codex 授权完成",
        "Codex authorization failed" => "Codex 授权失败",
        "Elapsed" => "已用时",
        "Authorization URL" => "授权 URL",
        "About" => "关于",
        "Keyboard Help" => "键盘帮助",
        "{shortcut} opens this panel. Esc closes it." => "{shortcut} 打开此面板，Esc 关闭。",
        "ACTIVE AREA" => "当前区域",
        "Current" => "当前",
        "or" => "或",
        "Global" => "全局",
        "Whole app" => "整个应用",
        "Show keyboard help" => "显示键盘帮助",
        "Available from every workspace surface" => "所有工作区界面可用",
        "Close popups or leave Reviewer" => "关闭弹窗或离开 Reviewer",
        "Dialog, popup, Reviewer route" => "弹窗、浮层、Reviewer 路由",
        "Add workspace" => "添加工作区",
        "When text fields are not focused" => "文本框未聚焦时",
        "Open settings" => "打开设置",
        "Capture an egui screenshot" => "截取 egui 屏幕",
        "Toggle notifications" => "切换通知",
        "Global output drawer" => "全局输出抽屉",
        "Remove all notification lines" => "清空所有通知行",
        "Notification drawer" => "通知抽屉",
        "Workspace" => "工作区",
        "Rail, outline, and center tabs" => "侧栏、大纲和中间标签",
        "Switch Busy workspace" => "切换 Busy 工作区",
        "Workspace rail" => "工作区侧栏",
        "Switch non-Busy workspace" => "切换非 Busy 工作区",
        "Select main agent" => "选择主 Agent",
        "Select first subagent" => "选择第一个子 Agent",
        "Select second subagent" => "选择第二个子 Agent",
        "Workspace center" => "工作区中间区域",
        "Paste recent Markdown diffs to Agent" => "将最近 Markdown diff 粘贴给 Agent",
        "Translate current Agent input" => "翻译当前 Agent 输入",
        "Apply last translation to Agent input" => "应用上次翻译到 Agent 输入",
        "Toggle Agent and Markdown" => "切换 Agent 和 Markdown",
        "Open files, folders, tabs, and rail entries" => "打开文件、文件夹、标签和侧栏项",
        "Left rail and outline panel" => "左侧栏和大纲面板",
        "Open file and folder actions" => "打开文件和文件夹操作",
        "Outline panel" => "大纲面板",
        "Markdown" => "Markdown",
        "Editor and preview" => "编辑器和预览",
        "Save active Markdown document" => "保存当前 Markdown 文档",
        "Toggle editor and preview" => "切换编辑器和预览",
        "Markdown surface" => "Markdown 界面",
        "Switch between preview and editor" => "在预览和编辑器之间切换",
        "Scroll the rendered preview" => "滚动渲染预览",
        "Preview panel" => "预览面板",
        "Return to the last Markdown mode" => "返回上次 Markdown 模式",
        "Center tabs" => "中间标签",
        "Embedded terminal surfaces" => "嵌入式终端界面",
        "Toggle workspace terminal drawer" => "切换工作区终端抽屉",
        "Workspace route" => "工作区路由",
        "Toggle Reviewer Helix drawer" => "切换 Reviewer Helix 抽屉",
        "Send input to the active terminal process" => "发送输入到活动终端进程",
        "Agent, workspace terminal, Helix drawer" => "Agent、工作区终端、Helix 抽屉",
        "Reviewer" => "Reviewer",
        "GSD and git inspection route" => "GSD 和 git 检查路由",
        "Open Reviewer from workspace" => "从工作区打开 Reviewer",
        "Exit Reviewer" => "退出 Reviewer",
        "Move between reviewer columns" => "在 reviewer 列之间移动",
        "Move selected reviewer row" => "移动选中的 reviewer 行",
        "Toggle full-file and diff view" => "切换整文件和 diff 视图",
        "Jump between full-file blocks" => "在整文件块之间跳转",
        "Copy selected reviewer prompt into Agent" => "复制选中的 reviewer prompt 到 Agent",
        "Reload reviewer data" => "重新加载 reviewer 数据",
        "Open branch switcher" => "打开分支切换器",
        "Check that the configured shell or Codex command is available." => {
            "检查已配置的 shell 或 Codex 命令是否可用。"
        }
        "Check that the configured shell is available." => "检查已配置的 shell 是否可用。",
        "Check that the Helix executable `hx` is available." => {
            "检查 Helix 可执行文件 `hx` 是否可用。"
        }
        "Starting Agent terminal..." => "正在启动 Agent 终端...",
        "Starting workspace terminal..." => "正在启动工作区终端...",
        "Starting Helix..." => "正在启动 Helix...",
        "Agent terminal failed to start" => "Agent 终端启动失败",
        "Workspace terminal failed to start" => "工作区终端启动失败",
        "Helix failed to start" => "Helix 启动失败",
        "No backend error was reported." => "后端没有返回错误信息。",
        "Retry" => "重试",
        "Open keyboard help" => "打开键盘帮助",
        "Rest" => "休息",
        "Start rest now" => "立即开始休息",
        "Pomodoro is disabled in settings" => "番茄钟已在设置中禁用",
        "Work" => "工作",
        "Start work now" => "立即开始工作",
        "Manual rest started for {minutes} minutes" => "手动进入休息 {minutes} 分钟",
        "Work ended, resting for {minutes} minutes" => "工作结束，进入休息 {minutes} 分钟",
        "Quiet now, continue resting" => "已安静，继续休息",
        "Rest ended, press any key to start work" => "休息结束，等待任意输入开始工作",
        "Working for {minutes} minutes" => "进入工作 {minutes} 分钟",
        "Input detected, waiting for quiet to continue rest" => "检测到输入，等待安静后继续休息",
        "Input detected, returning to work" => "检测到输入，准备回到工作",
        "Starting work for {minutes} minutes" => "立即开始工作 {minutes} 分钟",
        "Resting {time}" => "休息中 {time}",
        "No document selected" => "未选择文档",
        "Select a markdown file to preview rendered content." => {
            "选择一个 Markdown 文件以预览渲染内容。"
        }
        "Select a markdown file to start editing." => "选择一个 Markdown 文件开始编辑。",
        "No headings" => "暂无标题",
        "No recently viewed Markdown" => "暂无最近查看的 Markdown",
        "New markdown" => "新建 Markdown",
        "New folder" => "新建文件夹",
        "Attach directory" => "附加目录",
        "Remove attached directory" => "移除附加目录",
        "Directory not found" => "目录不存在",
        "Directory already attached" => "目录已附加",
        "Reveal in Finder" => "在 Finder 中显示",
        "Copy absolute path" => "复制绝对路径",
        "Copy relative path" => "复制相对路径",
        "Refresh" => "刷新",
        "Unfavorite" => "取消收藏",
        "Favorite" => "收藏",
        "Open in Editor" => "在编辑器中打开",
        "Open in Preview" => "在预览中打开",
        "Delete markdown" => "删除 Markdown",
        "Show outline" => "显示大纲",
        "Hide outline" => "隐藏大纲",
        "Copy session ID" => "复制 session ID",
        "Copy session ID (after chat)" => "复制 session ID（对话后）",
        "Restart agent" => "重启 Agent",
        "Switch agent" => "切换 Agent",
        "Set model" => "设置模型",
        "Set model provider" => "设置模型供应方",
        "Set work-dir" => "设置 work-dir",
        "Effort" => "推理强度",
        "Default effort" => "默认强度",
        "Fast mode" => "Fast 模式",
        "Default fast mode" => "默认 Fast 模式",
        "Agent model" => "Agent 模型",
        "Agent model provider" => "Agent 模型供应方",
        "Agent work-dir" => "Agent work-dir",
        "None" => "无",
        "Choose..." => "选择...",
        "MODEL (OPTIONAL)" => "模型（可选）",
        "MODEL PROVIDER (OPTIONAL)" => "模型供应方（可选）",
        "WORK-DIR (OPTIONAL)" => "work-dir（可选）",
        "EFFORT (OPTIONAL)" => "推理强度（可选）",
        "FAST MODE (OPTIONAL)" => "Fast 模式（可选）",
        "empty uses global default" => "留空使用全局默认",
        "empty passes no -c model_provider" => "留空则不传 -c model_provider",
        "empty uses workspace root" => "留空使用 workspace root",
        "Agent model set" => "Agent 模型已设置",
        "Agent model override cleared" => "Agent 模型覆盖已清除",
        "Agent model provider set" => "Agent 模型供应方已设置",
        "Agent model provider override cleared" => "Agent 模型供应方覆盖已清除",
        "Agent work-dir set" => "Agent work-dir 已设置",
        "Agent work-dir override cleared" => "Agent work-dir 覆盖已清除",
        "Agent effort set" => "Agent 推理强度已设置",
        "Agent effort override cleared" => "Agent 推理强度覆盖已清除",
        "Agent fast mode set" => "Agent Fast 模式已设置",
        "Agent fast mode override cleared" => "Agent Fast 模式覆盖已清除",
        "Move to workspace" => "移动到工作区",
        "No other workspace" => "没有其它工作区",
        "Move left" => "左移",
        "Move right" => "右移",
        "Move to head" => "移到开头",
        "Move to tail" => "移到末尾",
        "Remove" => "移除",
        "Saved markdown file" => "Markdown 文件已保存",
        "Save failed" => "保存失败",
        "No recent Markdown diffs" => "暂无最近 Markdown diff",
        "Markdown diffs pasted to Agent" => "Markdown diff 已粘贴到 Agent",
        "Agent not ready; Markdown diffs copied" => "Agent 未就绪；Markdown diff 已复制",
        "Helix executable `hx` was not found" => "未找到 Helix 可执行文件 `hx`",
        "Reviewer reloaded" => "Reviewer 已重新加载",
        "Codex authorized" => "Codex 已授权",
        "Codex auth failed" => "Codex 授权失败",
        "Created markdown file" => "Markdown 文件已创建",
        "Created folder" => "文件夹已创建",
        "Renamed item" => "项目已重命名",
        "Deleted markdown file" => "Markdown 文件已删除",
        "Copied absolute path" => "绝对路径已复制",
        "Copied relative path" => "相对路径已复制",
        "Outline favorites only" => "仅显示收藏",
        "Outline all files" => "显示全部文件",
        "Favorite added" => "已添加收藏",
        "Favorite removed" => "已取消收藏",
        "No recent Markdown files" => "暂无最近 Markdown 文件",
        "Workflow step not found" => "未找到工作流 step",
        "Workflow saved" => "工作流已保存",
        "Workflow save failed" => "工作流保存失败",
        "New workflow project" => "新建工作流项目",
        "Create root.md" => "创建 root.md",
        "Workflow updated" => "工作流已更新",
        "Workflow Update Failed" => "工作流更新失败",
        "No Agent input draft to translate" => "没有可翻译的 Agent 输入草稿",
        "No translatable Agent input text" => "没有可翻译的 Agent 输入文本",
        "No Agent input translation to apply" => "没有可应用的 Agent 输入翻译",
        "Translation belongs to another Agent slot" => "翻译属于另一个 Agent 槽位",
        "Agent input changed; translation discarded" => "Agent 输入已变化；翻译已丢弃",
        "Agent not ready" => "Agent 未就绪",
        "Image placeholder translation did not match source" => "图片占位符翻译与源文本不匹配",
        "Translating..." => "翻译中...",
        "Auto translating..." => "自动翻译中...",
        "No translation returned." => "没有返回翻译。",
        "Agent will restart to apply the theme" => "Agent 将重启以应用主题",
        "Agent restarted with resume" => "Agent 已续接重启",
        "Agent restarted with a new session" => "Agent 已用新会话重启",
        "Agent resumed after theme switch" => "主题切换后 Agent 已恢复",
        "Restarted {count} terminal-backed session(s) for proxy changes" => {
            "已为代理变更重启 {count} 个终端后端会话"
        }
        "Switched agent to {kind}" => "已切换 Agent 到 {kind}",
        "Session ID copied" => "Session ID 已复制",
        "No reviewer row selected to copy" => "没有选中可复制的 reviewer 行",
        "Copied and pasted to agent" => "已复制并粘贴到 Agent",
        "Copied, but agent terminal is unavailable" => "已复制，但 Agent 终端不可用",
        "No reviewer target for Helix" => "没有可用于 Helix 的 reviewer 目标",
        "Reviewer is not loaded for this workspace." => "此工作区尚未加载 reviewer。",
        "No repository is selected." => "未选择仓库。",
        "Loading branches for {repo}..." => "正在加载 {repo} 的分支...",
        "Switching {repo} to {branch}..." => "正在将 {repo} 切换到 {branch}...",
        "Uncommitted changes" => "未提交更改",
        "Commit, stash, or discard changes before switching branches." => {
            "切换分支前请先提交、stash 或丢弃更改。"
        }
        "Repository" => "仓库",
        "Filter branches..." => "筛选分支...",
        "From" => "从",
        "To" => "到",
        "Switching branches will reload reviewer data for this workspace." => {
            "切换分支会重新加载此工作区的 reviewer 数据。"
        }
        "Confirm" => "确认",
        "Back" => "返回",
        "Run {script}?" => "执行 {script}？",
        "No active workspace for reviewer script" => "没有可运行 reviewer 脚本的活动工作区",
        "Started {script} for {target}" => "已为 {target} 启动 {script}",
        "Add Workspace Failed" => "添加工作区失败",
        "Closing workspace..." => "正在关闭工作区...",
        "Close Workspace Failed" => "关闭工作区失败",
        "Subagent already exists in target workspace" => "目标工作区已存在该子 Agent",
        "Screenshot failed: {error}" => "截图失败：{error}",
        "Screenshot request file failed: {error}" => "截图请求文件失败：{error}",
        "Reveal Failed" => "显示失败",
        "Reviewer failed to load: {error}" => "Reviewer 加载失败：{error}",
        "Reviewer action failed: {error}" => "Reviewer 操作失败：{error}",
        "Reviewer git load failed: {error}" => "Reviewer git 数据加载失败：{error}",
        "Switched {repo} to {target}" => "已将 {repo} 切换到 {target}",
        "Branch switch failed" => "分支切换失败",
        "Create Markdown Failed" => "新建 Markdown 失败",
        "Create Folder Failed" => "新建文件夹失败",
        "Rename Failed" => "重命名失败",
        "Delete Markdown Failed" => "删除 Markdown 失败",
        "Failed to open browser: {error}" => "打开浏览器失败：{error}",
        "Cannot resolve parent directory." => "无法解析父目录。",
        "Need at least this many columns for the dense reviewer layout:" => {
            "密集 reviewer 布局至少需要这么多列宽："
        }
        "Script scan failed: {error}" => "脚本扫描失败：{error}",
        "No scripts in ~/.gsdv/reviewer" => "~/.gsdv/reviewer 中没有脚本",
        "Toggle light/dark mode" => "切换浅色/深色模式",
        "Extra tools scan failed" => "外置工具扫描失败",
        "No scripts" => "暂无脚本",
        "processing..." => "处理中...",
        "loading..." => "加载中...",
        "value must be true or false" => "值必须是 true 或 false",
        "input" => "输入",
        "Interrupt" => "中断",
        "global" => "全局",
        "workspace" => "工作区",
        "Automatic" => "自动",
        "Use default" => "使用默认",
        "Monospace" => "等宽",
        "Proportional" => "比例",
        "System fonts" => "系统字体",
        "No matching system fonts" => "没有匹配的系统字体",
        "filter fonts" => "筛选字体",
        "filter fallback fonts" => "筛选备用字体",
        "Close" => "关闭",
        "Cancel" => "取消",
        "OK" => "确定",
        _ => source,
    }
}

/// 返回日文静态文案。
fn japanese_text(source: &'static str) -> &'static str {
    match source {
        "No file" => "ファイルなし",
        "no workspace" => "ワークスペースなし",
        "[md]-Unsaved" => "[md]-未保存",
        "[md]-Saved" => "[md]-保存済み",
        "[memo]-Error" => "[memo]-エラー",
        "[memo]-Saved" => "[memo]-保存済み",
        "WORKSPACES" => "ワークスペース",
        "New Workspace" => "新規ワークスペース",
        "New workspace" => "新規ワークスペース",
        "Close workspace" => "ワークスペースを閉じる",
        "Outline" => "アウトライン",
        "Work-flow" => "ワークフロー",
        "No favorites" => "お気に入りなし",
        "Loading workflow..." => "ワークフローを読み込み中...",
        "No workflow projects" => "ワークフロープロジェクトなし",
        "Workflow path copied" => "ワークフローのパスをコピーしました",
        "Copy path" => "パスをコピー",
        "Open project" => "プロジェクトを開く",
        "Open root.md" => "root.md を開く",
        "Rename project" => "プロジェクト名を変更",
        "Add project" => "プロジェクトを追加",
        "Add task" => "task を追加",
        "Delete project" => "プロジェクトを削除",
        "Rename task" => "task 名を変更",
        "Add step" => "step を追加",
        "Delete task" => "task を削除",
        "Rename step" => "step 名を変更",
        "Merge steps" => "step を結合",
        "Add child step" => "子 step を追加",
        "Delete step" => "step を削除",
        "Select a workflow task." => "ワークフロー task を選択してください。",
        "Select a workflow step to edit." => "編集するワークフロー step を選択してください。",
        "No active workspace." => "アクティブなワークスペースがありません。",
        "No steps" => "step なし",
        "Notifications" => "通知",
        "{count} lines" => "{count} 行",
        "Clear" => "クリア",
        "No notifications" => "通知なし",
        "Memo" => "メモ",
        "Open a workspace" => "ワークスペースを開く",
        "Add a project directory to start an agent session, browse Markdown, or inspect reviewer data." => {
            "プロジェクトディレクトリを追加すると、Agent セッション、Markdown 閲覧、reviewer データ確認を始められます。"
        }
        "Add" => "追加",
        "Recent Markdown" => "最近の Markdown",
        "Unsaved Changes" => "未保存の変更",
        "Unsaved Workflow" => "未保存のワークフロー",
        "New Workflow Project" => "ワークフロープロジェクトを作成",
        "New Workflow Task" => "ワークフロー Task を作成",
        "New Workflow Step" => "ワークフロー Step を作成",
        "Rename Workflow" => "ワークフロー名を変更",
        "Merge Workflow Steps" => "ワークフロー Step を結合",
        "Confirm Workflow Delete" => "ワークフロー削除の確認",
        "Create Markdown" => "Markdown を作成",
        "Create Folder" => "フォルダを作成",
        "Rename" => "名前を変更",
        "Confirm Delete" => "削除の確認",
        "Close Workspace" => "ワークスペースを閉じる",
        "Add Subagent" => "サブ Agent を追加",
        "Restart Agent" => "Agent を再起動",
        "Switch Agent" => "Agent を切り替え",
        "Switch Theme" => "テーマを切り替え",
        "Agent Exited" => "Agent が終了しました",
        "Help" => "ヘルプ",
        "Settings" => "設定",
        "About gsdv" => "gsdv について",
        "Codex Auth" => "Codex 認証",
        "Switch Branch" => "ブランチを切り替え",
        "Confirm Branch Switch" => "ブランチ切り替えを確認",
        "Confirm Script" => "スクリプトを確認",
        "Branch Error" => "ブランチエラー",
        "Branch" => "ブランチ",
        "Uncommitted Changes" => "未コミットの変更",
        "You have unsaved changes" => "未保存の変更があります",
        "Save before switching files, discard edits, or stay on the current file." => {
            "ファイルを切り替える前に保存、破棄、または現在のファイルに留まってください。"
        }
        "You have unsaved workflow changes" => "未保存のワークフロー変更があります",
        "Save both panes before switching, discard edits, or stay here." => {
            "切り替える前に両方のペインを保存、破棄、またはここに留まってください。"
        }
        "Discard" => "破棄",
        "Save" => "保存",
        "New workflow task" => "ワークフロー task を作成",
        "New workflow step" => "ワークフロー step を作成",
        "Merge workflow steps" => "ワークフロー step を結合",
        "Merge" => "結合",
        "Rename workflow project" => "ワークフロープロジェクト名を変更",
        "Rename workflow task" => "ワークフロー task 名を変更",
        "Rename workflow step" => "ワークフロー step 名を変更",
        "Delete workflow item?" => "ワークフロー項目を削除しますか？",
        "Create New Markdown File" => "Markdown ファイルを作成",
        "Create New Folder" => "フォルダを作成",
        "Delete markdown file?" => "Markdown ファイルを削除しますか？",
        "Close workspace?" => "ワークスペースを閉じますか？",
        "This workspace no longer exists." => "このワークスペースは存在しません。",
        "This removes the workspace from gsdv and deletes its workspace memo file under ~/.gsdv/workspaces/<workspace-hash>/memo.md. The project directory itself is not deleted." => {
            "gsdv からこのワークスペースを削除し、~/.gsdv/workspaces/<workspace-hash>/memo.md のメモファイルも削除します。プロジェクトディレクトリ自体は削除されません。"
        }
        "Name" => "名前",
        "Common" => "よく使う",
        "Location" => "場所",
        "New name" => "新しい名前",
        "Current path" => "現在のパス",
        "PROJECT" => "PROJECT",
        "TASK" => "TASK",
        "KEY" => "KEY",
        "DESC" => "DESC",
        "CURRENT" => "CURRENT",
        "Create" => "作成",
        "Delete" => "削除",
        "New subagent" => "サブ Agent を作成",
        "NAME" => "名前",
        "AGENT TYPE" => "AGENT 種別",
        "SESSION ID (OPTIONAL)" => "SESSION ID（任意）",
        "resume an existing session" => "既存の session を再開",
        "Restart agent?" => "Agent を再起動しますか？",
        "Resume keeps the stored session id. New clears the stored session and starts fresh." => {
            "Resume は保存済み session id を保持します。New は session を消去して新規開始します。"
        }
        "Restart New" => "新規で再起動",
        "Restart Resume" => "再開して再起動",
        "Switch agent?" => "Agent を切り替えますか？",
        "Switching agent type clears this workspace's stored session id and starts a new session." => {
            "Agent 種別を切り替えると、このワークスペースの保存済み session id を消去して新しい session を開始します。"
        }
        "Switch" => "切り替え",
        "Switch theme?" => "テーマを切り替えますか？",
        "The active agent will gracefully exit, then resume so it can reload terminal colors for {mode} mode." => {
            "アクティブな Agent は正常終了してから再開し、{mode} モードのターミナル色を再読み込みします。"
        }
        "Agent exited abnormally" => "Agent が異常終了しました",
        "The embedded agent process ended with {status}. You can try the same command outside gsdv to inspect the failure." => {
            "埋め込み Agent プロセスは {status} で終了しました。gsdv 外で同じコマンドを実行して失敗を確認できます。"
        }
        "Command" => "コマンド",
        "If you confirm, gsdv will start this workspace's agent again without resume. Cancel leaves the terminal as-is." => {
            "確認すると、gsdv は resume なしでこのワークスペースの Agent を再起動します。キャンセルするとターミナルはそのままです。"
        }
        "Start Without Resume" => "resume なしで開始",
        "Theme" => "テーマ",
        "Design-spec light theme" => "デザイン仕様のライトテーマ",
        "Terminal backend" => "ターミナルバックエンド",
        "Workspace persistence" => "ワークスペース保存",
        "Agent status" => "Agent 状態",
        "Language" => "言語",
        "Interface language" => "UI 言語",
        "Applied immediately and saved globally." => "すぐに反映され、全体設定として保存されます。",
        "Default Agent" => "既定 Agent",
        "New workspace agent" => "新規ワークスペース Agent",
        "Used when adding new workspaces and when old workspace data has no agent type." => {
            "新規ワークスペース作成時と、古いワークスペースに agent 種別がない場合に使います。"
        }
        "Rendering" => "描画",
        "Max FPS" => "最大 FPS",
        "Caps application-scheduled repaint requests." => {
            "アプリが要求する再描画頻度を制限します。"
        }
        "Agent" => "Agent",
        "Auto go after idle minutes" => "アイドル後に自動 go する分数",
        "Sends go once when a Busy agent has no new output." => {
            "Busy の Agent に新しい出力がないとき、一度だけ go を送信します。"
        }
        "Custom quick replies" => "カスタムクイック返信",
        "one reply per line" => "1 行に 1 件",
        "Shown as the second row in the Agent right-click menu." => {
            "Agent の右クリックメニューの 2 行目に表示されます。"
        }
        "Auto translate Agent input after idle" => "アイドル後に Agent 入力を自動翻訳",
        "Runs Cmd/Alt+M after the Agent draft stops changing for about 500 ms." => {
            "Agent 下書きが約 500 ms 変化しないと Cmd/Alt+M を実行します。"
        }
        "Allow Codex HTTP fallback after 5 WS failures" => {
            "WS 失敗 5 回後の Codex HTTP フォールバックを許可"
        }
        "When disabled, Codex Responses keeps rebuilding WebSocket connections instead of silently using HTTP." => {
            "無効時、Codex Responses は HTTP へ静かに切り替えず WebSocket 接続を再構築し続けます。"
        }
        "Pomodoro" => "ポモドーロ",
        "Enable pomodoro" => "ポモドーロを有効化",
        "Work minutes" => "作業時間（分）",
        "Rest minutes" => "休憩時間（分）",
        "Warning remaining percent" => "残り割合の警告",
        "Rest shows a pixel cat; state resets when gsdv restarts." => {
            "休憩中はピクセル猫を表示します。gsdv 再起動で状態はリセットされます。"
        }
        "Fonts" => "フォント",
        "Default fonts" => "既定フォント",
        "Terminal" => "ターミナル",
        "Markdown editor" => "Markdown エディタ",
        "Primary font" => "メインフォント",
        "Fallback font" => "代替フォント",
        "Network proxy" => "ネットワークプロキシ",
        "HTTP / HTTPS / SOCKS proxy" => "HTTP / HTTPS / SOCKS プロキシ",
        "Leave empty to disable proxy." => "空のままにするとプロキシを無効化します。",
        "Generate paired HTTP/SOCKS env vars" => "対応する HTTP/SOCKS 環境変数を生成",
        "When enabled, http:// also sets all_proxy=socks5://..., and socks5:// also sets http_proxy/https_proxy=http://...." => {
            "有効時は http:// が all_proxy=socks5://... も設定し、socks5:// が http_proxy/https_proxy=http://... も設定します。"
        }
        "Additional no_proxy entries" => "追加の no_proxy エントリ",
        "Built in, always included" => "内蔵、常に含む",
        "Effective no_proxy" => "有効な no_proxy",
        "Proxy changes are saved. Close Settings to restart open terminals, agents, and Helix sessions immediately." => {
            "プロキシ変更は保存済みです。設定を閉じると、開いているターミナル、Agent、Helix セッションをすぐ再起動します。"
        }
        "Proxy changes apply when terminal-backed sessions start." => {
            "プロキシ変更はターミナル系セッションの起動時に反映されます。"
        }
        "Codex auth" => "Codex 認証",
        "Account" => "アカウント",
        "Authorizing" => "認証中",
        "Auth" => "認証",
        "Waiting" => "待機中",
        "Waiting for browser authorization" => "ブラウザ認証を待機中",
        "Codex authorization complete" => "Codex 認証が完了しました",
        "Codex authorization failed" => "Codex 認証に失敗しました",
        "Elapsed" => "経過",
        "Authorization URL" => "認証 URL",
        "About" => "概要",
        "Keyboard Help" => "キーボードヘルプ",
        "{shortcut} opens this panel. Esc closes it." => {
            "{shortcut} でこのパネルを開き、Esc で閉じます。"
        }
        "ACTIVE AREA" => "現在の領域",
        "Current" => "現在",
        "or" => "または",
        "Global" => "グローバル",
        "Whole app" => "アプリ全体",
        "Show keyboard help" => "キーボードヘルプを表示",
        "Available from every workspace surface" => "すべてのワークスペース画面で利用可能",
        "Close popups or leave Reviewer" => "ポップアップを閉じる、または Reviewer を離れる",
        "Dialog, popup, Reviewer route" => "ダイアログ、ポップアップ、Reviewer ルート",
        "Add workspace" => "ワークスペースを追加",
        "When text fields are not focused" => "テキスト欄にフォーカスがない時",
        "Open settings" => "設定を開く",
        "Capture an egui screenshot" => "egui スクリーンショットを撮る",
        "Toggle notifications" => "通知を切り替え",
        "Global output drawer" => "グローバル出力ドロワー",
        "Remove all notification lines" => "すべての通知行を削除",
        "Notification drawer" => "通知ドロワー",
        "Workspace" => "ワークスペース",
        "Rail, outline, and center tabs" => "レール、アウトライン、中央タブ",
        "Switch Busy workspace" => "Busy ワークスペースへ切り替え",
        "Workspace rail" => "ワークスペースレール",
        "Switch non-Busy workspace" => "非 Busy ワークスペースへ切り替え",
        "Select main agent" => "メイン Agent を選択",
        "Select first subagent" => "1 番目のサブ Agent を選択",
        "Select second subagent" => "2 番目のサブ Agent を選択",
        "Workspace center" => "ワークスペース中央",
        "Paste recent Markdown diffs to Agent" => "最近の Markdown diff を Agent に貼り付け",
        "Translate current Agent input" => "現在の Agent 入力を翻訳",
        "Apply last translation to Agent input" => "前回の翻訳を Agent 入力に適用",
        "Toggle Agent and Markdown" => "Agent と Markdown を切り替え",
        "Open files, folders, tabs, and rail entries" => {
            "ファイル、フォルダ、タブ、レール項目を開く"
        }
        "Left rail and outline panel" => "左レールとアウトラインパネル",
        "Open file and folder actions" => "ファイルとフォルダの操作を開く",
        "Outline panel" => "アウトラインパネル",
        "Markdown" => "Markdown",
        "Editor and preview" => "エディタとプレビュー",
        "Save active Markdown document" => "現在の Markdown 文書を保存",
        "Toggle editor and preview" => "エディタとプレビューを切り替え",
        "Markdown surface" => "Markdown 画面",
        "Switch between preview and editor" => "プレビューとエディタを切り替え",
        "Scroll the rendered preview" => "レンダリングプレビューをスクロール",
        "Preview panel" => "プレビューパネル",
        "Return to the last Markdown mode" => "直前の Markdown モードへ戻る",
        "Center tabs" => "中央タブ",
        "Embedded terminal surfaces" => "埋め込みターミナル画面",
        "Toggle workspace terminal drawer" => "ワークスペースターミナルドロワーを切り替え",
        "Workspace route" => "ワークスペースルート",
        "Toggle Reviewer Helix drawer" => "Reviewer Helix ドロワーを切り替え",
        "Send input to the active terminal process" => "アクティブなターミナルプロセスへ入力を送る",
        "Agent, workspace terminal, Helix drawer" => {
            "Agent、ワークスペースターミナル、Helix ドロワー"
        }
        "Reviewer" => "Reviewer",
        "GSD and git inspection route" => "GSD と git の検査ルート",
        "Open Reviewer from workspace" => "ワークスペースから Reviewer を開く",
        "Exit Reviewer" => "Reviewer を終了",
        "Move between reviewer columns" => "reviewer 列間を移動",
        "Move selected reviewer row" => "選択した reviewer 行を移動",
        "Toggle full-file and diff view" => "全文表示と diff 表示を切り替え",
        "Jump between full-file blocks" => "全文ブロック間を移動",
        "Copy selected reviewer prompt into Agent" => {
            "選択した reviewer プロンプトを Agent にコピー"
        }
        "Reload reviewer data" => "reviewer データを再読み込み",
        "Open branch switcher" => "ブランチ切り替えを開く",
        "Check that the configured shell or Codex command is available." => {
            "設定された shell または Codex コマンドが利用可能か確認してください。"
        }
        "Check that the configured shell is available." => {
            "設定された shell が利用可能か確認してください。"
        }
        "Check that the Helix executable `hx` is available." => {
            "Helix 実行ファイル `hx` が利用可能か確認してください。"
        }
        "Starting Agent terminal..." => "Agent ターミナルを起動中...",
        "Starting workspace terminal..." => "ワークスペースターミナルを起動中...",
        "Starting Helix..." => "Helix を起動中...",
        "Agent terminal failed to start" => "Agent ターミナルの起動に失敗しました",
        "Workspace terminal failed to start" => "ワークスペースターミナルの起動に失敗しました",
        "Helix failed to start" => "Helix の起動に失敗しました",
        "No backend error was reported." => "バックエンドエラーは報告されていません。",
        "Retry" => "再試行",
        "Open keyboard help" => "キーボードヘルプを開く",
        "Rest" => "休憩",
        "Start rest now" => "今すぐ休憩を開始",
        "Pomodoro is disabled in settings" => "ポモドーロは設定で無効です",
        "Work" => "作業",
        "Start work now" => "今すぐ作業を開始",
        "Manual rest started for {minutes} minutes" => "{minutes} 分の休憩を手動開始",
        "Work ended, resting for {minutes} minutes" => "作業終了、{minutes} 分休憩します",
        "Quiet now, continue resting" => "静かになりました。休憩を続けます",
        "Rest ended, press any key to start work" => "休憩終了。任意の入力で作業を開始します",
        "Working for {minutes} minutes" => "{minutes} 分の作業を開始",
        "Input detected, waiting for quiet to continue rest" => {
            "入力を検出。静かになるまで待って休憩を続けます"
        }
        "Input detected, returning to work" => "入力を検出。作業に戻ります",
        "Starting work for {minutes} minutes" => "{minutes} 分の作業を今すぐ開始",
        "Resting {time}" => "休憩中 {time}",
        "No document selected" => "ドキュメント未選択",
        "Select a markdown file to preview rendered content." => {
            "レンダリング内容をプレビューする Markdown ファイルを選択してください。"
        }
        "Select a markdown file to start editing." => {
            "編集する Markdown ファイルを選択してください。"
        }
        "No headings" => "見出しなし",
        "No recently viewed Markdown" => "最近表示した Markdown はありません",
        "New markdown" => "Markdown を作成",
        "New folder" => "フォルダを作成",
        "Attach directory" => "ディレクトリを追加",
        "Remove attached directory" => "追加ディレクトリを解除",
        "Directory not found" => "ディレクトリが見つかりません",
        "Directory already attached" => "ディレクトリは追加済みです",
        "Reveal in Finder" => "Finder で表示",
        "Copy absolute path" => "絶対パスをコピー",
        "Copy relative path" => "相対パスをコピー",
        "Refresh" => "更新",
        "Unfavorite" => "お気に入り解除",
        "Favorite" => "お気に入り",
        "Open in Editor" => "エディタで開く",
        "Open in Preview" => "プレビューで開く",
        "Delete markdown" => "Markdown を削除",
        "Show outline" => "アウトラインを表示",
        "Hide outline" => "アウトラインを非表示",
        "Copy session ID" => "session ID をコピー",
        "Copy session ID (after chat)" => "session ID をコピー（チャット後）",
        "Restart agent" => "Agent を再起動",
        "Switch agent" => "Agent を切り替え",
        "Set model" => "モデルを設定",
        "Set model provider" => "モデルプロバイダを設定",
        "Set work-dir" => "work-dir を設定",
        "Effort" => "推論強度",
        "Default effort" => "既定の強度",
        "Fast mode" => "Fast モード",
        "Default fast mode" => "既定の Fast モード",
        "Agent model" => "Agent モデル",
        "Agent model provider" => "Agent モデルプロバイダ",
        "Agent work-dir" => "Agent work-dir",
        "None" => "なし",
        "Choose..." => "選択...",
        "MODEL (OPTIONAL)" => "モデル（任意）",
        "MODEL PROVIDER (OPTIONAL)" => "モデルプロバイダ（任意）",
        "WORK-DIR (OPTIONAL)" => "work-dir（任意）",
        "EFFORT (OPTIONAL)" => "推論強度（任意）",
        "FAST MODE (OPTIONAL)" => "Fast モード（任意）",
        "empty uses global default" => "空欄ならグローバル既定を使用",
        "empty passes no -c model_provider" => "空欄なら -c model_provider を渡しません",
        "empty uses workspace root" => "空欄なら workspace root を使用",
        "Agent model set" => "Agent モデルを設定しました",
        "Agent model override cleared" => "Agent モデル指定を解除しました",
        "Agent model provider set" => "Agent モデルプロバイダを設定しました",
        "Agent model provider override cleared" => "Agent モデルプロバイダ指定を解除しました",
        "Agent work-dir set" => "Agent work-dir を設定しました",
        "Agent work-dir override cleared" => "Agent work-dir 指定を解除しました",
        "Agent effort set" => "Agent 推論強度を設定しました",
        "Agent effort override cleared" => "Agent 推論強度指定を解除しました",
        "Agent fast mode set" => "Agent Fast モードを設定しました",
        "Agent fast mode override cleared" => "Agent Fast モード指定を解除しました",
        "Move to workspace" => "ワークスペースへ移動",
        "No other workspace" => "他のワークスペースなし",
        "Move left" => "左へ移動",
        "Move right" => "右へ移動",
        "Move to head" => "先頭へ移動",
        "Move to tail" => "末尾へ移動",
        "Remove" => "削除",
        "Saved markdown file" => "Markdown ファイルを保存しました",
        "Save failed" => "保存に失敗しました",
        "No recent Markdown diffs" => "最近の Markdown diff はありません",
        "Markdown diffs pasted to Agent" => "Markdown diff を Agent に貼り付けました",
        "Agent not ready; Markdown diffs copied" => "Agent 未準備。Markdown diff をコピーしました",
        "Helix executable `hx` was not found" => "Helix 実行ファイル `hx` が見つかりません",
        "Reviewer reloaded" => "Reviewer を再読み込みしました",
        "Codex authorized" => "Codex 認証が完了しました",
        "Codex auth failed" => "Codex 認証に失敗しました",
        "Created markdown file" => "Markdown ファイルを作成しました",
        "Created folder" => "フォルダを作成しました",
        "Renamed item" => "項目名を変更しました",
        "Deleted markdown file" => "Markdown ファイルを削除しました",
        "Copied absolute path" => "絶対パスをコピーしました",
        "Copied relative path" => "相対パスをコピーしました",
        "Outline favorites only" => "お気に入りのみ表示",
        "Outline all files" => "すべてのファイルを表示",
        "Favorite added" => "お気に入りに追加しました",
        "Favorite removed" => "お気に入りを解除しました",
        "No recent Markdown files" => "最近の Markdown ファイルはありません",
        "Workflow step not found" => "ワークフロー step が見つかりません",
        "Workflow saved" => "ワークフローを保存しました",
        "Workflow save failed" => "ワークフロー保存に失敗しました",
        "New workflow project" => "ワークフロープロジェクトを作成",
        "Create root.md" => "root.md を作成",
        "Workflow updated" => "ワークフローを更新しました",
        "Workflow Update Failed" => "ワークフロー更新に失敗しました",
        "No Agent input draft to translate" => "翻訳する Agent 入力下書きがありません",
        "No translatable Agent input text" => "翻訳可能な Agent 入力テキストがありません",
        "No Agent input translation to apply" => "適用する Agent 入力翻訳がありません",
        "Translation belongs to another Agent slot" => "翻訳は別の Agent スロットに属しています",
        "Agent input changed; translation discarded" => {
            "Agent 入力が変わったため翻訳を破棄しました"
        }
        "Agent not ready" => "Agent は未準備です",
        "Image placeholder translation did not match source" => {
            "画像プレースホルダ翻訳が元テキストと一致しません"
        }
        "Translating..." => "翻訳中...",
        "Auto translating..." => "自動翻訳中...",
        "No translation returned." => "翻訳が返りませんでした。",
        "Agent will restart to apply the theme" => "テーマ適用のため Agent を再起動します",
        "Agent restarted with resume" => "Agent を resume 付きで再起動しました",
        "Agent restarted with a new session" => "Agent を新しい session で再起動しました",
        "Agent resumed after theme switch" => "テーマ切替後に Agent を再開しました",
        "Restarted {count} terminal-backed session(s) for proxy changes" => {
            "プロキシ変更のため {count} 個のターミナル系セッションを再起動しました"
        }
        "Switched agent to {kind}" => "Agent を {kind} に切り替えました",
        "Session ID copied" => "Session ID をコピーしました",
        "No reviewer row selected to copy" => "コピーする reviewer 行が選択されていません",
        "Copied and pasted to agent" => "コピーして Agent に貼り付けました",
        "Copied, but agent terminal is unavailable" => {
            "コピーしましたが Agent ターミナルは利用できません"
        }
        "No reviewer target for Helix" => "Helix 用の reviewer 対象がありません",
        "Reviewer is not loaded for this workspace." => {
            "このワークスペースでは reviewer が読み込まれていません。"
        }
        "No repository is selected." => "リポジトリが選択されていません。",
        "Loading branches for {repo}..." => "{repo} のブランチを読み込み中...",
        "Switching {repo} to {branch}..." => "{repo} を {branch} に切り替え中...",
        "Uncommitted changes" => "未コミットの変更",
        "Commit, stash, or discard changes before switching branches." => {
            "ブランチ切り替え前にコミット、stash、または変更を破棄してください。"
        }
        "Repository" => "リポジトリ",
        "Filter branches..." => "ブランチを絞り込み...",
        "From" => "変更前",
        "To" => "変更先",
        "Switching branches will reload reviewer data for this workspace." => {
            "ブランチを切り替えると、このワークスペースの reviewer データを再読み込みします。"
        }
        "Confirm" => "確認",
        "Back" => "戻る",
        "Run {script}?" => "{script} を実行しますか？",
        "No active workspace for reviewer script" => {
            "reviewer スクリプト用のアクティブなワークスペースがありません"
        }
        "Started {script} for {target}" => "{target} 向けに {script} を開始しました",
        "Add Workspace Failed" => "ワークスペース追加に失敗しました",
        "Closing workspace..." => "ワークスペースを閉じています...",
        "Close Workspace Failed" => "ワークスペースを閉じられませんでした",
        "Subagent already exists in target workspace" => {
            "対象ワークスペースに同じサブ Agent が既に存在します"
        }
        "Screenshot failed: {error}" => "スクリーンショットに失敗しました: {error}",
        "Screenshot request file failed: {error}" => {
            "スクリーンショット要求ファイルに失敗しました: {error}"
        }
        "Reveal Failed" => "表示に失敗しました",
        "Reviewer failed to load: {error}" => "Reviewer 読み込みに失敗しました: {error}",
        "Reviewer action failed: {error}" => "Reviewer 操作に失敗しました: {error}",
        "Reviewer git load failed: {error}" => "Reviewer git データ読み込みに失敗しました: {error}",
        "Switched {repo} to {target}" => "{repo} を {target} に切り替えました",
        "Branch switch failed" => "ブランチ切り替えに失敗しました",
        "Create Markdown Failed" => "Markdown 作成に失敗しました",
        "Create Folder Failed" => "フォルダ作成に失敗しました",
        "Rename Failed" => "名前変更に失敗しました",
        "Delete Markdown Failed" => "Markdown 削除に失敗しました",
        "Failed to open browser: {error}" => "ブラウザを開けませんでした: {error}",
        "Cannot resolve parent directory." => "親ディレクトリを解決できません。",
        "Need at least this many columns for the dense reviewer layout:" => {
            "密な reviewer レイアウトには少なくともこの列幅が必要です:"
        }
        "Script scan failed: {error}" => "スクリプトスキャンに失敗しました: {error}",
        "No scripts in ~/.gsdv/reviewer" => "~/.gsdv/reviewer にスクリプトはありません",
        "Toggle light/dark mode" => "ライト/ダークモードを切り替え",
        "Extra tools scan failed" => "外部ツールのスキャンに失敗しました",
        "No scripts" => "スクリプトなし",
        "processing..." => "処理中...",
        "loading..." => "読み込み中...",
        "value must be true or false" => "値は true または false である必要があります",
        "input" => "入力",
        "Interrupt" => "中断",
        "global" => "グローバル",
        "workspace" => "ワークスペース",
        "Automatic" => "自動",
        "Use default" => "既定を使用",
        "Monospace" => "等幅",
        "Proportional" => "プロポーショナル",
        "System fonts" => "システムフォント",
        "No matching system fonts" => "一致するシステムフォントはありません",
        "filter fonts" => "フォントを絞り込み",
        "filter fallback fonts" => "代替フォントを絞り込み",
        "Close" => "閉じる",
        "Cancel" => "キャンセル",
        "OK" => "OK",
        _ => source,
    }
}

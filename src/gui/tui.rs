//! 原生终端入口。
//!
//! TUI 只负责输入、布局和 ANSI 差量绘制；workspace、Agent、PTY、Hook、
//! 番茄钟和持久化继续由 `GsdvGuiApp` 与唯一 `AppEvent` 队列持有。

use super::*;
use crate::gui::terminal_host::{
    TerminalTuiCursor, TerminalTuiFrame, TerminalTuiMouseKind, TerminalTuiOutputMatch,
    TerminalTuiOutputSource, TerminalTuiOutputState, TerminalTuiUpdate,
};
use anyhow::{Context, Result};
use base64::Engine as _;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as TerminalEvent, KeyCode, KeyEvent,
    KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, ModifierKeyCode, MouseButton, MouseEvent,
    MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::style::Print;
use crossterm::terminal::{
    self, BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate, EnterAlternateScreen,
    LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::{execute, queue};
use std::collections::BTreeMap;
use std::io::{Stdout, Write, stdout};
use std::path::PathBuf;
use unicode_width::UnicodeWidthChar;

#[path = "tui/column_browser.rs"]
mod column_browser;
#[path = "tui/markdown.rs"]
mod markdown;
#[path = "tui/menu.rs"]
mod menu;
#[path = "tui/pomodoro.rs"]
mod pomodoro;
#[path = "tui/render.rs"]
mod render;
#[path = "tui/settings.rs"]
mod settings;
#[path = "tui/workflow.rs"]
mod workflow;

use column_browser::*;
use markdown::*;
use menu::*;
use pomodoro::*;
use render::*;
use settings::*;
use workflow::*;

const TUI_POLL_INTERVAL: Duration = Duration::from_millis(20);
const TUI_MIN_WIDTH: u16 = 48;
const TUI_MIN_HEIGHT: u16 = 12;
const TUI_SIDEBAR_WIDTH: u16 = 27;
const TUI_COMPACT_SIDEBAR_WIDTH: u16 = 19;
const TUI_SPLITTER_SIZE: u16 = 1;
const TUI_MIN_PANE_WIDTH: u16 = 12;
const TUI_MIN_PANE_HEIGHT: u16 = 4;
const TUI_COLLAPSED_ROW_HEIGHT: u16 = 1;
const TUI_COLLAPSED_COLUMN_WIDTH: u16 = 3;
const TUI_BUSY_ANIMATION_INTERVAL: Duration = Duration::from_millis(250);

/// TUI 表单提交时创建新 Agent 的字段。
///
/// 适用场景：新增 tab、row 或 column。示例：`name=reviewer -> 新 subagent`。
#[derive(Debug, Clone)]
pub(super) struct TuiAgentFormValue {
    /// Agent 显示名称。
    pub name: String,
    /// Agent 实现类型。
    pub kind: AgentKind,
    /// 可选模型覆盖。
    pub model: String,
    /// 可选 Codex provider 覆盖。
    pub model_provider: String,
    /// 可选推理强度。
    pub effort: String,
    /// 可选 fast mode 覆盖。
    pub fast_mode: Option<bool>,
    /// 可选 Agent 工作目录。
    pub work_dir: String,
    /// 可选代码查找目录。
    pub helix_work_dir: String,
    /// 可选恢复 session id。
    pub session_id: String,
}

/// TUI 解析完成后投递到唯一 AppEvent 队列的业务动作。
///
/// 适用场景：右键菜单或表单确认。示例：停止 Agent -> `StopAgent`。
#[derive(Debug, Clone)]
pub(super) enum TuiAppAction {
    /// 选择 workspace。
    SelectWorkspace(usize),
    /// 选择一个 pane 及其 Agent tab。
    SelectAgent {
        /// workspace 索引。
        workspace_index: usize,
        /// Agent 行索引。
        row_index: usize,
        /// Agent 列索引。
        column_index: usize,
        /// main 或 subagent 槽位。
        slot: AgentSlotId,
    },
    /// 选择专注列表 Agent，并复用现有跨 workspace 定位逻辑。
    SelectSpotlightAgent {
        workspace_index: usize,
        slot: AgentSlotId,
    },
    /// 切换 workspace 聚焦标记。
    ToggleWorkspaceFocus(usize),
    /// 切换左侧 workspace 聚焦过滤。
    ToggleWorkspaceFilter,
    /// 从输入路径新增本地 workspace。
    AddWorkspace(PathBuf),
    /// 展开 outline 目录并触发后台刷新。
    ExpandOutlineDirectory { index: usize, key: PathBuf },
    /// 在共享 DocumentState 中打开 Markdown。
    OpenMarkdown { index: usize, path: PathBuf },
    /// 打开 Agent 输出里识别出的 URL 或路径。
    OpenTerminalOutput {
        /// workspace 索引。
        index: usize,
        /// 输出所属 Agent 槽位。
        slot: AgentSlotId,
        /// 只做语法识别、尚未分类的点击目标。
        click: TerminalOutputClick,
    },
    /// 在当前 Agent 工作目录打开 Helix。
    OpenWorkspaceHelix { index: usize },
    /// 保存普通 Markdown 文档。
    SaveMarkdown {
        index: usize,
        path: PathBuf,
        text: String,
    },
    /// 打开 Workflow target 并初始化共享编辑器状态。
    OpenWorkflowTarget {
        index: usize,
        target: WorkflowSelectionTarget,
    },
    /// 保存 Workflow task 或 step 片段。
    SaveWorkflowMarkdown {
        index: usize,
        target: WorkflowSelectionTarget,
        text: String,
    },
    /// 通过共享后台链修改 Workflow 项目、任务或步骤。
    WorkflowMutation {
        /// 修改所属 workspace。
        index: usize,
        /// 已校验的 Workflow 修改请求。
        request: WorkflowMutationRequest,
    },
    /// 显示已有 Workflow Agent。
    OpenWorkflowAgent {
        /// 所属 workspace。
        index: usize,
        /// 右键目标。
        target: WorkflowSelectionTarget,
    },
    /// 创建 Workflow Agent。
    AddWorkflowAgent {
        /// 所属 workspace。
        index: usize,
        /// 稳定目标和 sidecar。
        binding: WorkflowAgentBinding,
        /// 复用普通 Agent 表单值。
        value: TuiAgentFormValue,
    },
    /// 销毁 Workflow Agent。
    DestroyWorkflowAgent {
        /// 所属 workspace。
        index: usize,
        /// 稳定目标。
        target: data::WorkflowAgentTarget,
    },
    /// 切换 socket 图片画廊当前项。
    MoveImageGallery(isize),
    /// 关闭并清空 socket 图片画廊。
    CloseImageGallery,
    /// 修改本地 workspace 根目录。
    ChangeWorkspaceDirectory { index: usize, path: PathBuf },
    /// 确认关闭 workspace。
    CloseWorkspace(usize),
    /// 启动停止态 Agent。
    StartAgent { index: usize, slot: AgentSlotId },
    /// 停止 Agent。
    StopAgent { index: usize, slot: AgentSlotId },
    /// 重启 Agent，可选择恢复 session。
    RestartAgent {
        /// workspace 索引。
        index: usize,
        /// Agent 槽位。
        slot: AgentSlotId,
        /// 是否恢复原 session。
        resume: bool,
    },
    /// 修改专注登记等级；None 表示取消登记。
    SetSpotlight {
        /// workspace 索引。
        index: usize,
        /// Agent 槽位。
        slot: AgentSlotId,
        /// 1-3 等级或取消。
        level: Option<u8>,
    },
    /// 切换 Agent 类型并开启新 session。
    SwitchAgentKind {
        /// workspace 索引。
        index: usize,
        /// Agent 槽位。
        slot: AgentSlotId,
        /// 新 Agent 类型。
        kind: AgentKind,
    },
    /// 设置模型覆盖。
    SetAgentModel {
        index: usize,
        slot: AgentSlotId,
        value: String,
    },
    /// 设置模型 provider 覆盖。
    SetAgentModelProvider {
        index: usize,
        slot: AgentSlotId,
        value: String,
    },
    /// 设置推理强度覆盖。
    SetAgentEffort {
        index: usize,
        slot: AgentSlotId,
        value: Option<String>,
    },
    /// 设置 fast mode 覆盖。
    SetAgentFastMode {
        index: usize,
        slot: AgentSlotId,
        value: Option<bool>,
    },
    /// 设置 Agent 工作目录。
    SetAgentWorkDir {
        index: usize,
        slot: AgentSlotId,
        value: String,
    },
    /// 设置代码查找目录。
    SetAgentHelixWorkDir {
        index: usize,
        slot: AgentSlotId,
        value: String,
    },
    /// 重命名 subagent。
    RenameSubagent {
        index: usize,
        id: String,
        name: String,
    },
    /// 删除 subagent。
    RemoveSubagent { index: usize, id: String },
    /// 在既有 column 新增 subagent。
    AddAgent {
        /// workspace 索引。
        index: usize,
        /// 目标 column id。
        column_id: String,
        /// 表单值。
        value: TuiAgentFormValue,
    },
    /// 新增 row 并在其首列创建 Agent。
    AddRowWithAgent {
        index: usize,
        after_row: usize,
        value: TuiAgentFormValue,
    },
    /// 新增 column 并创建 Agent。
    AddColumnWithAgent {
        index: usize,
        row_index: usize,
        value: TuiAgentFormValue,
    },
    /// 删除 Agent row。
    RemoveRow { index: usize, row_index: usize },
    /// 删除 Agent column。
    RemoveColumn { index: usize, column_id: String },
    /// 折叠或展开 Agent row。
    SetRowCollapsed {
        index: usize,
        row_index: usize,
        collapsed: bool,
    },
    /// 折叠或展开 Agent column。
    SetColumnCollapsed {
        /// workspace 索引。
        index: usize,
        /// 行索引。
        row_index: usize,
        /// 列索引。
        column_index: usize,
        /// 新折叠状态。
        collapsed: bool,
    },
    /// 仅保留当前 Agent column 展开。
    CollapseOtherColumns {
        /// workspace 索引。
        index: usize,
        /// Agent row 索引。
        row_index: usize,
        /// 保持展开的 column 索引。
        column_index: usize,
    },
    /// 仅保留当前 Agent row 展开。
    CollapseOtherRows {
        /// workspace 索引。
        index: usize,
        /// 保持展开的 row 索引。
        row_index: usize,
    },
    /// 等宽当前 row 的 columns。
    EqualizeColumns { index: usize, row_index: usize },
    /// 等高所有 rows。
    EqualizeRows { index: usize },
    /// 在当前 column 内把 subagent tab 左移一位。
    MoveSubagentLeft {
        /// workspace 索引。
        index: usize,
        /// column 稳定 id。
        column_id: String,
        /// subagent id。
        id: String,
    },
    /// 在当前 column 内把 subagent tab 右移一位。
    MoveSubagentRight {
        /// workspace 索引。
        index: usize,
        /// column 稳定 id。
        column_id: String,
        /// subagent id。
        id: String,
    },
    /// 把 subagent tab 移到当前 column 首部。
    MoveSubagentToHead {
        /// workspace 索引。
        index: usize,
        /// column 稳定 id。
        column_id: String,
        /// subagent id。
        id: String,
    },
    /// 把 subagent tab 移到当前 column 尾部。
    MoveSubagentToTail {
        /// workspace 索引。
        index: usize,
        /// column 稳定 id。
        column_id: String,
        /// subagent id。
        id: String,
    },
    /// 把 subagent 移到同一 row 的相邻 column。
    MoveSubagentToAdjacentColumn {
        /// workspace 索引。
        index: usize,
        /// 当前 row 索引。
        row_index: usize,
        /// 当前 column 索引。
        column_index: usize,
        /// subagent id。
        id: String,
        /// `-1` 表示左侧，`1` 表示右侧。
        direction: isize,
    },
    /// 把 subagent 移到相邻 row 的第一列。
    MoveSubagentToAdjacentRow {
        /// workspace 索引。
        index: usize,
        /// 当前 row 索引。
        row_index: usize,
        /// 当前 column 索引。
        column_index: usize,
        /// subagent id。
        id: String,
        /// `-1` 表示上方，`1` 表示下方。
        direction: isize,
    },
    /// 把 subagent 移到另一个 workspace。
    MoveSubagentToWorkspace {
        /// 来源 workspace 索引。
        index: usize,
        /// subagent id。
        id: String,
        /// 目标 workspace 索引。
        target_index: usize,
    },
    /// 拖动相邻可见 columns 的分隔线。
    ResizeColumns {
        /// workspace 索引。
        index: usize,
        /// Agent row 索引。
        row_index: usize,
        /// 分隔线左侧 column 索引。
        left_index: usize,
        /// 分隔线右侧 column 索引。
        right_index: usize,
        /// 本次拖动字符差。
        delta: i16,
        /// 可见 columns 可用字符总宽度。
        content_width: u16,
    },
    /// 拖动相邻可见 rows 的分隔线。
    ResizeRows {
        /// workspace 索引。
        index: usize,
        /// 分隔线上方 row 索引。
        top_index: usize,
        /// 分隔线下方 row 索引。
        bottom_index: usize,
        /// 本次拖动字符差。
        delta: i16,
        /// 可见 rows 可用字符总高度。
        content_height: u16,
    },
    /// 展开 workspace 的所有 Agent rows/columns。
    ExpandAllAgents { index: usize },
    /// 按现有 Agent terminal 键协议提交特殊键。
    TerminalKey {
        /// workspace 索引。
        index: usize,
        /// Agent 槽位。
        slot: AgentSlotId,
        /// 白名单 terminal 键。
        key: TerminalRemoteKey,
    },
    /// 使用现有 terminal mode 处理鼠标滚轮。
    ScrollAgent {
        /// workspace 索引。
        index: usize,
        /// Agent 槽位。
        slot: AgentSlotId,
        /// 正数向上、负数向下。
        delta: i32,
        /// pane 内字符坐标。
        pointer_cell: Option<(u16, u16)>,
    },
    /// 使用现有 terminal mode 处理 workspace terminal 鼠标滚轮。
    ScrollWorkspace {
        /// workspace 索引。
        index: usize,
        /// 正数向上、负数向下。
        delta: i32,
        /// terminal 内字符坐标。
        pointer_cell: Option<(u16, u16)>,
    },
    /// 使用现有 terminal mode 处理 Helix 鼠标滚轮。
    ScrollHelix {
        /// workspace 索引。
        index: usize,
        /// 正数向上、负数向下。
        delta: i32,
        /// Helix 内字符坐标。
        pointer_cell: Option<(u16, u16)>,
    },
    /// 把主鼠标动作转发给 Helix terminal。
    HelixMouse {
        /// workspace 索引。
        index: usize,
        /// 按下、拖动或松开。
        kind: TerminalTuiMouseKind,
        /// Helix terminal 内零基字符列。
        column: u16,
        /// Helix terminal 内零基字符行。
        row: u16,
    },
    /// 开关持久化番茄钟功能。
    SetPomodoroEnabled(bool),
    /// 开关专注等级模式。
    SetSpotlightRankMode(bool),
    /// 立即开始休息。
    StartPomodoroRest,
    /// 开始或重置工作计时。
    StartPomodoroWork,
    /// 更新持久化的番茄工作、休息和告警阈值。
    UpdatePomodoroSettings {
        /// 工作分钟数。
        work_minutes: u16,
        /// 休息分钟数。
        rest_minutes: u16,
        /// 剩余时间告警百分比。
        warning_remaining_percent: u8,
    },
    /// 更新网络代理配置，并让已打开的 PTY 重新读取环境变量。
    UpdateNetworkSettings {
        /// 是否向子进程注入代理环境变量。
        proxy_enabled: bool,
        /// HTTP、HTTPS 或 SOCKS 主代理 URL。
        proxy: String,
        /// 追加到内置规则后的 no_proxy 条目。
        no_proxy: String,
        /// 是否同时生成 HTTP(S) 与 SOCKS 代理环境变量。
        mirror_proxy_protocol: bool,
    },
}

/// AppEvent 完成后台分类后交给 TUI 的一次视图导航。
///
/// 适用场景：点击 Agent 输出。示例：Rust 文件 -> Helix，Markdown -> Dir。
#[derive(Debug, Clone)]
pub(super) enum TuiNavigationRequest {
    /// 在目录工作台打开 Markdown。
    Markdown {
        /// 目标 workspace。
        workspace_index: usize,
        /// workspace 相对或绝对文档路径。
        path: PathBuf,
    },
    /// 切换到 workspace 的 Helix host。
    Helix {
        /// 目标 workspace。
        workspace_index: usize,
    },
}

impl GsdvGuiApp {
    /// 应用 TUI 动作，所有可见业务状态仍由 AppEvent drain 修改。
    ///
    /// 适用场景：TUI 菜单确认。示例：`StopAgent -> stop_agent_slot`。
    pub(super) fn apply_tui_action(&mut self, ctx: &egui::Context, action: TuiAppAction) {
        match action {
            TuiAppAction::SelectWorkspace(index) => self.switch_workspace(index),
            TuiAppAction::SelectAgent {
                workspace_index,
                row_index,
                column_index,
                slot,
            } => {
                self.switch_workspace(workspace_index);
                if let Some(workspace) = self.workspaces.get_mut(workspace_index)
                    && let Some(column) = workspace
                        .agent_rows
                        .get_mut(row_index)
                        .and_then(|row| row.columns.get_mut(column_index))
                {
                    column.active_slot = slot.to_column_slot();
                }
                self.set_agent_focus(workspace_index, row_index, column_index);
                self.set_active_agent_slot(workspace_index, slot.clone());
                self.ensure_agent_terminal_host(ctx, workspace_index, &slot);
            }
            TuiAppAction::SelectSpotlightAgent {
                workspace_index,
                slot,
            } => {
                self.prepare_spotlight_workspace(workspace_index, slot.clone());
                self.ensure_agent_terminal_host(ctx, workspace_index, &slot);
            }
            TuiAppAction::ToggleWorkspaceFocus(index) => self.toggle_workspace_focus(index),
            TuiAppAction::ToggleWorkspaceFilter => self.toggle_workspace_focus_filter(),
            TuiAppAction::AddWorkspace(path) => {
                let existing = self
                    .workspaces
                    .iter()
                    .map(|item| item.path.clone())
                    .collect();
                self.spawn_workspace_add_task(ctx, path, self.default_agent_kind, existing);
            }
            TuiAppAction::ExpandOutlineDirectory { index, key } => {
                if let Some(workspace) = self.workspaces.get_mut(index) {
                    set_outline_directory_expanded(&mut workspace.outline, &key);
                }
                self.spawn_outline_refresh_tasks(ctx, BTreeSet::from([index]));
            }
            TuiAppAction::OpenMarkdown { index, path } => {
                self.switch_workspace(index);
                self.open_file_now(path);
            }
            TuiAppAction::OpenTerminalOutput { index, slot, click } => {
                if index != self.active_workspace {
                    // 触发条件：Spotlight pane 指向非当前 workspace 的 Agent。
                    // 不能丢弃点击：pane 已可见且用户明确点击了它的输出。
                    // 先复用跨 workspace 定位，防止分类结果因 active 不同被忽略。
                    self.prepare_spotlight_workspace(index, slot.clone());
                }
                match click {
                    TerminalOutputClick::PathCandidate(click) => {
                        let Some(workspace) = self.agent_metadata_for_slot(index, &slot) else {
                            return;
                        };
                        let helix_work_dir = self.agent_slot_helix_work_dir(index, &slot);
                        self.spawn_tui_terminal_output_path_classify_task(
                            ctx,
                            index,
                            click,
                            helix_work_dir,
                            workspace,
                        );
                    }
                    click => {
                        self.apply_classified_tui_terminal_output_click(ctx, click, None);
                    }
                }
            }
            TuiAppAction::OpenWorkspaceHelix { index } => {
                self.switch_workspace(index);
                self.open_workspace_helix_drawer(ctx);
            }
            TuiAppAction::SaveMarkdown { index, path, text } => {
                self.switch_workspace(index);
                let loaded = self.documents.get(index).is_some_and(|document| {
                    document.path.as_ref() == Some(&path) && document.loading_path.is_none()
                });
                if !loaded {
                    self.open_file_now(path);
                    return;
                }
                if let Some(document) = self.documents.get_mut(index) {
                    document.text = text;
                }
                self.save_active_document();
            }
            TuiAppAction::OpenWorkflowTarget { index, target } => {
                self.switch_workspace(index);
                self.open_workflow_target_now(ctx, target);
            }
            TuiAppAction::SaveWorkflowMarkdown {
                index,
                target,
                text,
            } => {
                self.switch_workspace(index);
                self.open_workflow_target_now(ctx, target.clone());
                if let Some(state) = self.workflow_states.get_mut(index) {
                    match &target {
                        WorkflowSelectionTarget::Task { .. } => {
                            if let Some(editor) = state.task_editor.as_mut() {
                                editor.task_text = text;
                            }
                        }
                        WorkflowSelectionTarget::Step { .. } => {
                            if let Some(editor) = state.editor.as_mut() {
                                editor.step_text = text;
                            }
                        }
                        WorkflowSelectionTarget::WorkspaceRoot { .. }
                        | WorkflowSelectionTarget::Project { .. } => return,
                    }
                }
                self.save_active_workflow_step(ctx, None);
            }
            TuiAppAction::WorkflowMutation { index, request } => {
                self.switch_workspace(index);
                self.request_workflow_mutation(ctx, request);
            }
            TuiAppAction::OpenWorkflowAgent { index, target } => {
                self.switch_workspace(index);
                self.request_workflow_agent(ctx, target);
            }
            TuiAppAction::AddWorkflowAgent {
                index,
                binding,
                value,
            } => {
                self.switch_workspace(index);
                self.add_tui_workflow_agent(ctx, index, binding, value);
            }
            TuiAppAction::DestroyWorkflowAgent { index, target } => {
                self.destroy_workflow_agent(ctx, index, &target);
            }
            TuiAppAction::MoveImageGallery(delta) => {
                self.image_gallery.move_by(delta);
                self.request_app_repaint();
            }
            TuiAppAction::CloseImageGallery => {
                self.image_gallery.close();
                self.request_app_repaint();
            }
            TuiAppAction::ChangeWorkspaceDirectory { index, path } => {
                let Some(workspace) = self.workspaces.get(index) else {
                    return;
                };
                let old_path = workspace.path.clone();
                let kind = workspace.agent_kind;
                let existing = self
                    .workspaces
                    .iter()
                    .enumerate()
                    .map(|(item_index, item)| (item_index, item.path.clone()))
                    .collect();
                self.spawn_workspace_directory_change_task(
                    ctx, index, old_path, path, kind, existing,
                );
            }
            TuiAppAction::CloseWorkspace(index) => self.close_workspace(ctx, index),
            TuiAppAction::StartAgent { index, slot } => self.start_agent_slot(ctx, index, slot),
            TuiAppAction::StopAgent { index, slot } => self.stop_agent_slot(index, slot),
            TuiAppAction::RestartAgent {
                index,
                slot,
                resume,
            } => self.restart_agent_slot(ctx, index, slot, resume),
            TuiAppAction::SetSpotlight { index, slot, level } => match level {
                Some(level) => self.set_agent_spotlight_level(index, slot, level),
                None => self.remove_agent_spotlight(index, slot),
            },
            TuiAppAction::SwitchAgentKind { index, slot, kind } => {
                self.switch_agent_slot_kind(ctx, index, slot, kind)
            }
            TuiAppAction::SetAgentModel { index, slot, value } => {
                self.set_agent_slot_model(ctx, index, slot, value)
            }
            TuiAppAction::SetAgentModelProvider { index, slot, value } => {
                self.set_agent_slot_model_provider(ctx, index, slot, value)
            }
            TuiAppAction::SetAgentEffort { index, slot, value } => {
                self.set_agent_slot_effort(ctx, index, slot, value)
            }
            TuiAppAction::SetAgentFastMode { index, slot, value } => {
                self.set_agent_slot_fast_mode(ctx, index, slot, value)
            }
            TuiAppAction::SetAgentWorkDir { index, slot, value } => {
                self.set_agent_slot_work_dir(ctx, index, slot, value)
            }
            TuiAppAction::SetAgentHelixWorkDir { index, slot, value } => {
                self.set_agent_slot_helix_work_dir(index, slot, value)
            }
            TuiAppAction::RenameSubagent { index, id, name } => {
                self.rename_subagent(index, &id, name)
            }
            TuiAppAction::RemoveSubagent { index, id } => self.remove_subagent(ctx, index, &id),
            TuiAppAction::AddAgent {
                index,
                column_id,
                value,
            } => self.add_tui_agent(ctx, index, column_id, value),
            TuiAppAction::AddRowWithAgent {
                index,
                after_row,
                value,
            } => {
                if let Some(column_id) = self.add_agent_row(index, after_row) {
                    self.add_tui_agent(ctx, index, column_id, value);
                }
            }
            TuiAppAction::AddColumnWithAgent {
                index,
                row_index,
                value,
            } => {
                if let Some(column_id) = self.add_agent_column(index, row_index) {
                    self.add_tui_agent(ctx, index, column_id, value);
                }
            }
            TuiAppAction::RemoveRow { index, row_index } => {
                self.remove_agent_row(ctx, index, row_index)
            }
            TuiAppAction::RemoveColumn { index, column_id } => {
                self.remove_agent_column(ctx, index, &column_id)
            }
            TuiAppAction::SetRowCollapsed {
                index,
                row_index,
                collapsed,
            } => self.set_agent_row_collapsed(index, row_index, collapsed),
            TuiAppAction::SetColumnCollapsed {
                index,
                row_index,
                column_index,
                collapsed,
            } => self.set_agent_column_collapsed(index, row_index, column_index, collapsed),
            TuiAppAction::CollapseOtherColumns {
                index,
                row_index,
                column_index,
            } => self.collapse_other_agent_columns(index, row_index, column_index),
            TuiAppAction::CollapseOtherRows { index, row_index } => {
                self.collapse_other_agent_rows(index, row_index)
            }
            TuiAppAction::EqualizeColumns { index, row_index } => {
                self.equalize_agent_columns(index, row_index)
            }
            TuiAppAction::EqualizeRows { index } => self.equalize_agent_rows(index),
            TuiAppAction::MoveSubagentLeft {
                index,
                column_id,
                id,
            } => self.move_subagent_left(ctx, index, &column_id, &id),
            TuiAppAction::MoveSubagentRight {
                index,
                column_id,
                id,
            } => self.move_subagent_right(ctx, index, &column_id, &id),
            TuiAppAction::MoveSubagentToHead {
                index,
                column_id,
                id,
            } => self.move_subagent_to_head(ctx, index, &column_id, &id),
            TuiAppAction::MoveSubagentToTail {
                index,
                column_id,
                id,
            } => self.move_subagent_to_tail(ctx, index, &column_id, &id),
            TuiAppAction::MoveSubagentToAdjacentColumn {
                index,
                row_index,
                column_index,
                id,
                direction,
            } => self.move_subagent_to_adjacent_column(
                ctx,
                index,
                row_index,
                column_index,
                &id,
                direction,
            ),
            TuiAppAction::MoveSubagentToAdjacentRow {
                index,
                row_index,
                column_index,
                id,
                direction,
            } => self.move_subagent_to_adjacent_row(
                ctx,
                index,
                row_index,
                column_index,
                &id,
                direction,
            ),
            TuiAppAction::MoveSubagentToWorkspace {
                index,
                id,
                target_index,
            } => self.move_subagent_to_workspace(ctx, index, &id, target_index),
            TuiAppAction::ResizeColumns {
                index,
                row_index,
                left_index,
                right_index,
                delta,
                content_width,
            } => self.resize_tui_agent_columns(
                index,
                row_index,
                left_index,
                right_index,
                delta,
                content_width,
            ),
            TuiAppAction::ResizeRows {
                index,
                top_index,
                bottom_index,
                delta,
                content_height,
            } => self.resize_tui_agent_rows(index, top_index, bottom_index, delta, content_height),
            TuiAppAction::ExpandAllAgents { index } => {
                if let Some(workspace) = self.workspaces.get_mut(index) {
                    for row in &mut workspace.agent_rows {
                        row.collapsed = false;
                        for column in &mut row.columns {
                            column.collapsed = false;
                        }
                    }
                }
                self.normalize_agent_focus(index);
                self.persist_workspaces();
                self.request_app_repaint();
            }
            TuiAppAction::TerminalKey { index, slot, key } => {
                self.ensure_agent_terminal_host(ctx, index, &slot);
                if let Some(host) = self
                    .terminal_hosts
                    .get_mut(index)
                    .and_then(|hosts| hosts.agents.get_mut(&slot))
                    .and_then(|host_slot| host_slot.host.as_mut())
                {
                    host.send_remote_key_direct(key);
                }
            }
            TuiAppAction::ScrollAgent {
                index,
                slot,
                delta,
                pointer_cell,
            } => {
                if let Some(host) = self
                    .terminal_hosts
                    .get_mut(index)
                    .and_then(|hosts| hosts.agents.get_mut(&slot))
                    .and_then(|host_slot| host_slot.host.as_mut())
                {
                    host.scroll_direct(delta, pointer_cell);
                }
            }
            TuiAppAction::ScrollWorkspace {
                index,
                delta,
                pointer_cell,
            } => {
                if let Some(host) = self
                    .terminal_hosts
                    .get_mut(index)
                    .and_then(|hosts| hosts.workspace.as_mut())
                {
                    host.scroll_direct(delta, pointer_cell);
                }
            }
            TuiAppAction::ScrollHelix {
                index,
                delta,
                pointer_cell,
            } => {
                if let Some(host) = self
                    .terminal_hosts
                    .get_mut(index)
                    .and_then(|hosts| hosts.helix.as_mut())
                {
                    host.scroll_direct(delta, pointer_cell);
                }
            }
            TuiAppAction::HelixMouse {
                index,
                kind,
                column,
                row,
            } => {
                if let Some(host) = self
                    .terminal_hosts
                    .get_mut(index)
                    .and_then(|hosts| hosts.helix.as_mut())
                {
                    host.send_tui_primary_mouse_direct(kind, column, row);
                }
            }
            TuiAppAction::SetPomodoroEnabled(enabled) => {
                self.runtime_settings.pomodoro_enabled = enabled;
                self.pending_runtime_settings_save = true;
                self.pomodoro.start_working(Instant::now());
                self.request_app_repaint();
            }
            TuiAppAction::SetSpotlightRankMode(enabled) => self.set_spotlight_rank_mode(enabled),
            TuiAppAction::StartPomodoroRest => {
                if self.runtime_settings.pomodoro_enabled {
                    self.pomodoro.start_resting(Instant::now());
                    self.request_app_repaint();
                }
            }
            TuiAppAction::StartPomodoroWork => {
                if self.runtime_settings.pomodoro_enabled {
                    self.pomodoro.start_working(Instant::now());
                    self.request_app_repaint();
                }
            }
            TuiAppAction::UpdatePomodoroSettings {
                work_minutes,
                rest_minutes,
                warning_remaining_percent,
            } => {
                self.runtime_settings.pomodoro_work_minutes =
                    work_minutes.clamp(data::MIN_POMODORO_MINUTES, data::MAX_POMODORO_MINUTES);
                self.runtime_settings.pomodoro_rest_minutes =
                    rest_minutes.clamp(data::MIN_POMODORO_MINUTES, data::MAX_POMODORO_MINUTES);
                self.runtime_settings.pomodoro_warning_remaining_percent =
                    warning_remaining_percent.clamp(
                        data::MIN_POMODORO_WARNING_REMAINING_PERCENT,
                        data::MAX_POMODORO_WARNING_REMAINING_PERCENT,
                    );
                self.pending_runtime_settings_save = true;
                self.request_app_repaint();
            }
            TuiAppAction::UpdateNetworkSettings {
                proxy_enabled,
                proxy,
                no_proxy,
                mirror_proxy_protocol,
            } => {
                let settings = NetworkSettings {
                    proxy_enabled,
                    proxy,
                    no_proxy,
                    mirror_proxy_protocol,
                };
                if self.network_settings == settings {
                    return;
                }
                self.network_settings = settings;
                self.pending_network_settings_save = true;
                // 代理只在创建子进程时进入环境；重启已打开 host 才能立即生效。
                self.restart_open_terminal_hosts_for_network_settings(ctx);
                self.request_app_repaint();
            }
        }
    }

    /// 按终端字符差调整相邻可见 columns，并交给既有 store writer 持久化。
    ///
    /// 适用场景：TUI 竖向 splitter 拖动。示例：右拖 3 格 -> 左列增宽、右列缩窄。
    fn resize_tui_agent_columns(
        &mut self,
        index: usize,
        row_index: usize,
        left_index: usize,
        right_index: usize,
        delta: i16,
        content_width: u16,
    ) {
        let Some(row) = self
            .workspaces
            .get_mut(index)
            .and_then(|workspace| workspace.agent_rows.get_mut(row_index))
        else {
            return;
        };
        if left_index >= right_index || right_index >= row.columns.len() || content_width <= 1 {
            return;
        }
        let visible_weight = row
            .columns
            .iter()
            .filter(|column| !column.collapsed)
            .map(|column| column.width_weight.max(0.01))
            .sum::<f32>();
        let scale = visible_weight / f32::from(content_width);
        let delta_weight = f32::from(delta) * scale;
        let min_weight = f32::from(TUI_MIN_PANE_WIDTH.min(content_width / 2)) * scale;
        let left = row.columns[left_index].width_weight;
        let right = row.columns[right_index].width_weight;
        let applied = delta_weight.clamp(min_weight - left, right - min_weight);
        if applied.abs() <= f32::EPSILON {
            return;
        }
        row.columns[left_index].width_weight += applied;
        row.columns[right_index].width_weight -= applied;
        data::normalize_agent_column_widths(&mut row.columns);
        self.persist_workspaces();
        self.request_app_repaint();
    }

    /// 按终端字符差调整相邻可见 rows，并交给既有 store writer 持久化。
    ///
    /// 适用场景：TUI 横向 splitter 拖动。示例：下拖 2 行 -> 上行增高、下行缩短。
    fn resize_tui_agent_rows(
        &mut self,
        index: usize,
        top_index: usize,
        bottom_index: usize,
        delta: i16,
        content_height: u16,
    ) {
        let Some(workspace) = self.workspaces.get_mut(index) else {
            return;
        };
        if top_index >= bottom_index
            || bottom_index >= workspace.agent_rows.len()
            || content_height <= 1
        {
            return;
        }
        let visible_weight = workspace
            .agent_rows
            .iter()
            .filter(|row| !row.collapsed)
            .map(|row| row.height_weight.max(0.01))
            .sum::<f32>();
        let scale = visible_weight / f32::from(content_height);
        let delta_weight = f32::from(delta) * scale;
        let min_weight = f32::from(TUI_MIN_PANE_HEIGHT.min(content_height / 2)) * scale;
        let top = workspace.agent_rows[top_index].height_weight;
        let bottom = workspace.agent_rows[bottom_index].height_weight;
        let applied = delta_weight.clamp(min_weight - top, bottom - min_weight);
        if applied.abs() <= f32::EPSILON {
            return;
        }
        workspace.agent_rows[top_index].height_weight += applied;
        workspace.agent_rows[bottom_index].height_weight -= applied;
        data::normalize_agent_row_heights(&mut workspace.agent_rows);
        self.persist_workspaces();
        self.request_app_repaint();
    }

    /// 规范化 TUI 新 Agent 表单并复用现有新增逻辑。
    ///
    /// 适用场景：TUI 表单提交。示例：空 model -> None。
    fn add_tui_agent(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        column_id: String,
        value: TuiAgentFormValue,
    ) {
        let optional = |value: String| {
            let value = value.trim().to_string();
            (!value.is_empty()).then_some(value)
        };
        let work_dir = optional(value.work_dir)
            .and_then(|path| data::normalize_stored_agent_work_dir(Some(PathBuf::from(path))));
        let helix_work_dir = optional(value.helix_work_dir)
            .and_then(|path| data::normalize_stored_helix_work_dir(Some(PathBuf::from(path))));
        let effort =
            optional(value.effort).filter(|effort| value.kind.supports_effort(effort.as_str()));
        self.add_subagent(
            ctx,
            index,
            column_id,
            value.name.trim().to_string(),
            value.kind,
            optional(value.model),
            value
                .kind
                .supports_model_provider()
                .then(|| optional(value.model_provider))
                .flatten(),
            effort,
            value
                .kind
                .supports_fast_mode()
                .then_some(value.fast_mode)
                .flatten(),
            work_dir,
            helix_work_dir,
            optional(value.session_id),
        );
    }

    /// 规范化 TUI 表单并创建不进入普通布局的 Workflow Agent。
    ///
    /// 适用场景：Workflow 右键 Agent 后提交复用表单。例：Step -> task sidecar + terminal host。
    fn add_tui_workflow_agent(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        binding: WorkflowAgentBinding,
        value: TuiAgentFormValue,
    ) {
        let optional = |value: String| {
            let value = value.trim().to_string();
            (!value.is_empty()).then_some(value)
        };
        let work_dir = optional(value.work_dir)
            .and_then(|path| data::normalize_stored_agent_work_dir(Some(PathBuf::from(path))));
        let helix_work_dir = optional(value.helix_work_dir)
            .and_then(|path| data::normalize_stored_helix_work_dir(Some(PathBuf::from(path))));
        let effort =
            optional(value.effort).filter(|effort| value.kind.supports_effort(effort.as_str()));
        self.add_workflow_agent(
            ctx,
            index,
            binding,
            value.name.trim().to_string(),
            value.kind,
            optional(value.model),
            value
                .kind
                .supports_model_provider()
                .then(|| optional(value.model_provider))
                .flatten(),
            effort,
            value
                .kind
                .supports_fast_mode()
                .then_some(value.fast_mode)
                .flatten(),
            work_dir,
            helix_work_dir,
            optional(value.session_id),
        );
    }
}

/// 启动不创建 egui 窗口的原生终端工作台。
///
/// 适用场景：SSH 后运行 `gsdv tui --fps 4`。示例：4 -> 每秒最多四次输出。
pub fn run_tui(fps: u16) -> Result<()> {
    let mut agent_launch = resolved_agent_launch_config();
    if !agent_launch.kind_explicit {
        agent_launch.kind = data::load_default_agent_kind().unwrap_or_else(|| {
            let kind = default_agent_kind_from_available_commands();
            data::save_default_agent_kind(kind);
            kind
        });
    }
    let data = data::load_initial_gui_data(agent_launch.kind);
    let mut app = GsdvGuiApp::new_with_font_settings(
        data,
        agent_launch,
        data::load_font_settings(),
        Vec::new(),
    );
    let ctx = egui::Context::default();
    app.attach_frontend_runtime(&ctx);

    let mut terminal = TuiTerminalGuard::enter()?;
    let mut state = TuiState::new(fps, terminal::size()?);
    state.run(&mut terminal.stdout, &mut app, &ctx)
}

/// 解析 GUI/TUI 共用的 Agent 启动参数。
///
/// 适用场景：两种入口保持默认 Agent 一致。示例：`--agent claude -> Claude`。
fn resolved_agent_launch_config() -> AgentLaunchConfig {
    AgentLaunchConfig::from_env_args()
}

/// raw mode 和 alternate screen 的恢复守卫。
///
/// 适用场景：正常退出或错误返回。示例：Drop -> 恢复 shell 输入。
struct TuiTerminalGuard {
    /// 复用的标准输出句柄。
    stdout: Stdout,
    /// 当前终端是否启用了 kitty 键盘增强栈。
    keyboard_enhancement: bool,
}

impl TuiTerminalGuard {
    /// 进入 TUI 终端模式。
    ///
    /// 适用场景：主循环启动前。示例：普通 shell -> alternate screen。
    fn enter() -> Result<Self> {
        enable_raw_mode().context("failed to enable terminal raw mode")?;
        let mut stdout = stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture, Hide) {
            let _ = disable_raw_mode();
            return Err(error).context("failed to enter alternate screen");
        }
        let keyboard_enhancement = terminal::supports_keyboard_enhancement().unwrap_or(false);
        if keyboard_enhancement
            && let Err(error) = execute!(
                stdout,
                PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
                        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                )
            )
        {
            let _ = execute!(stdout, Show, DisableMouseCapture, LeaveAlternateScreen);
            let _ = disable_raw_mode();
            return Err(error).context("failed to enable enhanced keyboard input");
        }
        Ok(Self {
            stdout,
            keyboard_enhancement,
        })
    }
}

impl Drop for TuiTerminalGuard {
    /// 恢复鼠标、光标、主屏幕和 canonical input。
    fn drop(&mut self) {
        if self.keyboard_enhancement {
            let _ = execute!(self.stdout, PopKeyboardEnhancementFlags);
        }
        let _ = execute!(
            self.stdout,
            Show,
            DisableMouseCapture,
            LeaveAlternateScreen,
            Clear(ClearType::All)
        );
        let _ = disable_raw_mode();
    }
}

/// workspace 与 Agent 的稳定 TUI 路由键。
///
/// 适用场景：pane cache 和输入目标。示例：workspace 2 的 main。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct AgentTarget {
    /// workspace vector 索引。
    workspace_index: usize,
    /// main 或 subagent。
    slot: AgentSlotId,
}

/// 一个终端 pane 在屏幕上的字符矩形。
///
/// 适用场景：布局和鼠标 hit test。示例：`x=28,y=1,w=80,h=20`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TuiRect {
    /// 左边界。
    x: u16,
    /// 上边界。
    y: u16,
    /// 宽度。
    width: u16,
    /// 高度。
    height: u16,
}

impl TuiRect {
    /// 判断字符坐标是否在矩形内。
    ///
    /// 适用场景：鼠标点击。示例：边界外 -> false。
    fn contains(self, x: u16, y: u16) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x.saturating_add(self.width)
            && y < self.y.saturating_add(self.height)
    }
}

/// 一个可见 Agent pane 的布局描述。
///
/// 适用场景：普通 row/col 或专注单 pane。示例：column.active_slot -> target。
#[derive(Debug, Clone)]
struct PaneLayout {
    /// pane 外框。
    rect: TuiRect,
    /// 当前活动 Agent。
    target: AgentTarget,
    /// workspace Agent 行索引。
    row_index: usize,
    /// workspace Agent 列索引。
    column_index: usize,
    /// column 稳定 id。
    column_id: String,
    /// header 中的所有 tab。
    tabs: Vec<AgentSlotId>,
}

/// Agent grid 中一个可点击的折叠恢复区。
///
/// 适用场景：折叠 row/column 后保留返回入口。示例：点击 `[>]` -> 展开 column 2。
#[derive(Debug, Clone, PartialEq, Eq)]
struct LayoutRestoreRegion {
    /// 恢复区占用的字符矩形。
    rect: TuiRect,
    /// 点击后恢复的布局项。
    target: LayoutRestoreTarget,
}

/// 折叠恢复区对应的持久化布局项。
///
/// 适用场景：将 TUI 点击转成现有 `TuiAppAction`。示例：Row 1 -> `SetRowCollapsed(false)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayoutRestoreTarget {
    /// 恢复一个 Agent row。
    Row {
        /// workspace 索引。
        workspace_index: usize,
        /// row 索引。
        row_index: usize,
    },
    /// 恢复一个 Agent column。
    Column {
        /// workspace 索引。
        workspace_index: usize,
        /// row 索引。
        row_index: usize,
        /// column 索引。
        column_index: usize,
    },
}

/// Agent grid 的可见 pane 与折叠恢复区。
///
/// 适用场景：一次布局计算同时供 PTY resize 和恢复控件使用。示例：1 pane + 2 collapsed columns。
#[derive(Debug, Default)]
struct AgentPaneLayout {
    /// 需要渲染并启动 terminal 的 pane。
    panes: Vec<PaneLayout>,
    /// 不启动 terminal 的折叠恢复区。
    restore_regions: Vec<LayoutRestoreRegion>,
}

/// 可拖动 row/column 分隔线的业务目标。
///
/// 适用场景：绘制阶段生成命中区，鼠标按下后转为拖动状态。
#[derive(Debug, Clone, PartialEq, Eq)]
enum SplitterTarget {
    /// 同一 row 内相邻可见 columns。
    Columns {
        /// workspace 索引。
        workspace_index: usize,
        /// Agent row 索引。
        row_index: usize,
        /// 左侧 column 索引。
        left_index: usize,
        /// 右侧 column 索引。
        right_index: usize,
        /// 不含 splitter 的可用字符宽度。
        content_width: u16,
    },
    /// 相邻可见 rows。
    Rows {
        /// workspace 索引。
        workspace_index: usize,
        /// 上方 row 索引。
        top_index: usize,
        /// 下方 row 索引。
        bottom_index: usize,
        /// 不含 splitter 的可用字符高度。
        content_height: u16,
    },
}

/// 一个 splitter 的屏幕命中矩形和调整目标。
///
/// 适用场景：鼠标左键按下。示例：竖线命中 -> Columns。
#[derive(Debug, Clone)]
struct SplitterRegion {
    /// splitter 占用的字符矩形。
    rect: TuiRect,
    /// splitter 对应的布局目标。
    target: SplitterTarget,
}

/// 当前进行中的 splitter 拖动。
///
/// 适用场景：Mouse Down 到 Mouse Up。示例：column splitter 保存上次 x。
#[derive(Debug, Clone)]
struct SplitterDrag {
    /// splitter 对应的布局目标。
    target: SplitterTarget,
    /// 上一次鼠标字符坐标，column 使用 x、row 使用 y。
    last_position: u16,
}

/// 终端输出源及最后一份可绘制快照。
///
/// 适用场景：每个可见 pane 独立做行级更新。示例：Patch -> 只替换对应 rows。
struct PaneCache {
    /// 用于识别 stop/restart 后替换的 PTY host。
    host_id: u64,
    /// PTY 只读输出源。
    source: TerminalTuiOutputSource,
    /// emulator damage 消费状态。
    output_state: TerminalTuiOutputState,
    /// 当前可绘制 viewport。
    frame: TerminalTuiFrame,
    /// 最近同步给 PTY 的 cols/lines。
    grid_size: (u16, u16),
    /// 普通 terminal 模式下距底部的本地 scrollback 行数。
    scroll_offset: usize,
}

/// 当前鼠标悬停的 Agent 输出 token。
///
/// 适用场景：绘制 GUI 同款链接下划线。示例：pane 第 4 行路径列 8..20。
#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentOutputHover {
    /// token 所属 Agent。
    target: AgentTarget,
    /// pane terminal viewport 内行号。
    row: usize,
    /// token 起始字符列。
    start_column: usize,
    /// token 结束字符列。
    end_column: usize,
}

/// TUI 当前占据主区域的一级视图。
///
/// 适用场景：底栏在 Agent、Workflow 与 workspace terminal 间切换。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum TuiMainView {
    /// 多 Agent 主视图。
    #[default]
    Agents,
    /// 只读 Workflow 浏览器。
    Workflow,
    /// Finder 风格的 Markdown 目录浏览器。
    Directory,
    /// 当前 workspace 的交互式 terminal。
    Terminal,
    /// 当前 workspace 的嵌入式 Helix。
    Helix,
}

/// 当前 workspace terminal 的差量渲染缓存。
///
/// 适用场景：切换 workspace 或 PTY 重启时替换 source。示例：workspace 1 -> workspace 2。
struct WorkspaceTerminalCache {
    /// cache 所属 workspace。
    workspace_index: usize,
    /// 用于识别重启后的 PTY。
    host_id: u64,
    /// PTY 只读输出源。
    source: TerminalTuiOutputSource,
    /// emulator damage 消费状态。
    output_state: TerminalTuiOutputState,
    /// 当前可绘制 viewport。
    frame: TerminalTuiFrame,
    /// 最近同步给 PTY 的 cols/lines。
    grid_size: (u16, u16),
    /// 普通 terminal 模式下距底部的本地 scrollback 行数。
    scroll_offset: usize,
}

/// 一条屏幕行的基础文字和覆盖绘制。
///
/// 适用场景：ANSI 差量输出。示例：base chrome + terminal styled overlay。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RenderLine {
    /// 无样式整行底图。
    base: String,
    /// 指定字符列开始的 ANSI 内容。
    overlays: Vec<(u16, String)>,
}

/// 鼠标点击区域绑定的动作。
///
/// 适用场景：左键选择、右键菜单。示例：tab rect -> SelectAgent。
#[derive(Debug, Clone)]
struct HitRegion {
    /// 字符矩形。
    rect: TuiRect,
    /// 左键动作。
    left: Option<HitAction>,
    /// 右键动作。
    right: Option<HitAction>,
}

/// 鼠标区域动作。
///
/// 适用场景：render 产生、event loop 消费。示例：右键 workspace -> menu。
#[derive(Debug, Clone)]
enum HitAction {
    /// 选择 workspace。
    SelectWorkspace(usize),
    /// 选择 Agent pane/tab。
    SelectAgent {
        /// 目标 Agent。
        target: AgentTarget,
        /// 行索引。
        row_index: usize,
        /// 列索引。
        column_index: usize,
    },
    /// 打开 workspace 菜单。
    WorkspaceMenu(usize),
    /// 打开 Agent 菜单。
    AgentMenu(AgentMenuContext),
    /// 打开 pane 结构菜单。
    PaneMenu(PaneMenuContext),
    /// 展开折叠的 Agent row。
    ExpandAgentRow {
        /// workspace 索引。
        workspace_index: usize,
        /// row 索引。
        row_index: usize,
    },
    /// 展开折叠的 Agent column。
    ExpandAgentColumn {
        /// workspace 索引。
        workspace_index: usize,
        /// row 索引。
        row_index: usize,
        /// column 索引。
        column_index: usize,
    },
    /// 切换专注模式。
    ToggleSpotlight,
    /// 切换专注等级模式。
    ToggleSpotlightRankMode,
    /// 折叠或展开 workspace rail。
    ToggleSidebar,
    /// 打开新 workspace 表单。
    NewWorkspace,
    /// 打开番茄菜单。
    PomodoroMenu,
    /// 打开全局设置表单。
    OpenSettings,
    /// 打开只读 Workflow 浏览器。
    OpenWorkflow,
    /// 打开 Finder 风格目录浏览器。
    OpenDirectory,
    /// 打开当前 workspace terminal。
    OpenWorkspaceTerminal,
    /// 打开当前 workspace Helix。
    OpenWorkspaceHelix,
    /// 返回 Agent 主视图。
    ExitAuxiliaryView,
    /// 选择 Workflow 预览目标。
    SelectWorkflow(WorkflowSelectionTarget),
    /// 折叠或展开 Workflow project。
    ToggleWorkflowProject(String),
    /// 打开 Workflow 节点只读菜单。
    WorkflowMenu(WorkflowMenuContext),
    /// 选择目录列中的 root 或 folder。
    SelectDirectory { depth: usize, key: PathBuf },
    /// 打开目录列中的 Markdown 文件。
    OpenDirectoryMarkdown(PathBuf),
    /// 切换指定 Markdown 工作台到预览。
    MarkdownPreview(String),
    /// 切换指定 Markdown 工作台到编辑。
    MarkdownEdit(String),
    /// 把指定 Markdown 工作台的光标移动到鼠标位置。
    MarkdownPlaceCursor {
        /// 工作台稳定 key。
        key: String,
        /// 编辑正文左上角，用于把终端坐标换算为文本坐标。
        origin_x: u16,
        /// 编辑正文左上角，用于把终端坐标换算为文本坐标。
        origin_y: u16,
    },
    /// 保存指定 Markdown 工作台。
    MarkdownSave(String),
    /// 丢弃指定 Markdown 工作台草稿。
    MarkdownDiscard(String),
    /// 离开指定 Markdown 的编辑态。
    MarkdownExit(String),
    /// 图片画廊上一张。
    GalleryPrevious,
    /// 图片画廊下一张。
    GalleryNext,
    /// 关闭图片画廊。
    GalleryClose,
}

/// Workflow 节点右键菜单上下文。
///
/// 适用场景：只提供 Open 与 Copy path，不暴露编辑动作。
#[derive(Debug, Clone)]
struct WorkflowMenuContext {
    /// 要在右侧预览的节点。
    target: WorkflowSelectionTarget,
    /// 节点逻辑路径。
    copy_path: String,
}

/// Agent 菜单定位上下文。
///
/// 适用场景：tab 或专注列表右键。示例：subagent tab -> 可重命名/关闭。
#[derive(Debug, Clone)]
struct AgentMenuContext {
    /// Agent 目标。
    target: AgentTarget,
    /// 可选 row 索引。
    row_index: Option<usize>,
    /// 可选 column 索引。
    column_index: Option<usize>,
    /// 可选 column id。
    column_id: Option<String>,
}

/// pane 结构菜单定位上下文。
///
/// 适用场景：pane header 空白处右键。示例：row 1 col 0 -> Add column。
#[derive(Debug, Clone)]
struct PaneMenuContext {
    /// workspace 索引。
    workspace_index: usize,
    /// row 索引。
    row_index: usize,
    /// column 索引。
    column_index: usize,
    /// column id。
    column_id: String,
}

/// 菜单项与延迟执行动作。
///
/// 适用场景：右键浮层。示例：`Stop agent -> Queue(StopAgent)`。
#[derive(Debug, Clone)]
struct MenuItem {
    /// 屏幕文字。
    label: String,
    /// 点击动作；None 表示分隔提示。
    action: Option<MenuAction>,
}

/// 菜单选择后的动作。
///
/// 适用场景：直接投 AppEvent、打开表单或确认框。
#[derive(Debug, Clone)]
enum MenuAction {
    /// 直接投递业务动作。
    Queue(TuiAppAction),
    /// 打开一个表单。
    Form(FormDialog),
    /// 打开确认弹层。
    Confirm {
        message: String,
        action: TuiAppAction,
    },
    /// 打开保持父菜单可见的二级菜单。
    Submenu(Vec<MenuItem>),
    /// 在 Workflow 浏览器打开节点。
    SelectWorkflow(WorkflowSelectionTarget),
    /// 通过终端 OSC 52 复制文本。
    CopyText(String),
}

/// 当前右键菜单状态。
///
/// 适用场景：鼠标或键盘选择。示例：Up/Down 更新 selected。
#[derive(Debug, Clone)]
struct ContextMenu {
    /// 左上角字符列。
    x: u16,
    /// 左上角字符行。
    y: u16,
    /// 所有菜单项。
    items: Vec<MenuItem>,
    /// 当前选中项。
    selected: usize,
    /// 超长菜单的首个可见项。
    scroll: usize,
    /// 二级菜单返回时恢复的父菜单。
    parent: Option<Box<ContextMenu>>,
}

/// TUI 表单字段。
///
/// 适用场景：新增 Agent 和配置编辑。示例：Name 文本、Agent Type 选项。
#[derive(Debug, Clone)]
enum FormField {
    /// 单行文本字段。
    Text { label: String, value: String },
    /// 离散选择字段。
    Choice {
        label: String,
        options: Vec<String>,
        selected: usize,
    },
}

/// 表单提交目标。
///
/// 适用场景：把通用字段编辑器映射回具体业务动作。
#[derive(Debug, Clone)]
enum FormSubmit {
    /// 新增 Agent 到具体位置。
    AddAgent {
        /// workspace 索引。
        index: usize,
        /// 新 Agent 布局位置。
        placement: AddAgentPlacement,
    },
    /// 新增 workspace。
    AddWorkspace,
    /// 修改 workspace 根目录。
    ChangeWorkspace(usize),
    /// 修改 Agent 单个字符串配置。
    AgentText {
        /// Agent 目标。
        target: AgentTarget,
        /// 字段类型。
        kind: AgentTextField,
    },
    /// 保存番茄钟三个持久化参数。
    PomodoroSettings,
    /// 保存全局网络代理配置。
    NetworkSettings,
    /// 新增 Workflow Project。
    WorkflowAddProject {
        /// 所属 workspace。
        index: usize,
        /// 目标 gsdv-spec 相对路径。
        spec_path: PathBuf,
    },
    /// 新增 Workflow Task。
    WorkflowAddTask {
        /// 所属 workspace。
        index: usize,
        /// 目标 gsdv-spec 相对路径。
        spec_path: PathBuf,
        /// 所属 Project key。
        project_key: String,
    },
    /// 新增 Workflow Step。
    WorkflowAddStep {
        /// 所属 workspace。
        index: usize,
        /// 所属 Task 相对路径。
        task_path: PathBuf,
    },
}

/// 新 Agent 应插入的布局位置。
///
/// 适用场景：新增 tab/row/column。示例：ExistingColumn("col-1")。
#[derive(Debug, Clone)]
enum AddAgentPlacement {
    /// 既有 column。
    ExistingColumn(String),
    /// 新 row，值为插入位置前一行。
    NewRow(usize),
    /// 既有 row 的新 column。
    NewColumn(usize),
    /// Workflow 层级 sidecar，不进入普通 row/column。
    Workflow(WorkflowAgentBinding),
}

/// Agent 文本表单的业务字段。
///
/// 适用场景：模型、目录和名称。示例：Model -> SetAgentModel。
#[derive(Debug, Clone, Copy)]
enum AgentTextField {
    /// 模型。
    Model,
    /// 模型 provider。
    ModelProvider,
    /// Agent cwd。
    WorkDir,
    /// 代码 cwd。
    HelixWorkDir,
    /// subagent 名称。
    Name,
}

/// 居中的 TUI 表单弹层。
///
/// 适用场景：替代 egui modal。示例：Tab 切字段，Enter 提交。
#[derive(Debug, Clone)]
struct FormDialog {
    /// 标题。
    title: String,
    /// 字段列表。
    fields: Vec<FormField>,
    /// 当前字段索引。
    active: usize,
    /// 提交目标。
    submit: FormSubmit,
    /// 可选校验错误。
    error: Option<String>,
}

/// 当前 TUI 唯一覆盖层。
///
/// 适用场景：菜单、表单、番茄控制和休息屏统一独占输入与绘制。
#[derive(Debug, Clone)]
enum TuiLayer {
    /// 可包含父级的右键菜单树。
    Menu(ContextMenu),
    /// 编辑表单。
    Form(FormDialog),
    /// 危险动作确认。
    Confirm {
        message: String,
        action: TuiAppAction,
    },
    /// Alt 打开的番茄状态和操作层。
    Pomodoro,
    /// 工作到点后持续显示到 Esc 的全屏休息层。
    Rest,
    /// socket 图片画廊。
    Gallery,
    /// 切换 Markdown target 前的未保存确认。
    MarkdownUnsaved {
        /// 用户确认后继续执行的原点击动作。
        pending: Box<HitAction>,
        /// 原点击字符列。
        x: u16,
        /// 原点击字符行。
        y: u16,
    },
}

/// 原生 TUI 的渲染和交互状态。
///
/// 适用场景：单进程主循环。示例：idle 时不写 stdout。
struct TuiState {
    /// 最大输出 FPS。
    frame_interval: Duration,
    /// 当前终端尺寸。
    size: (u16, u16),
    /// 是否正在专注模式。
    spotlight_mode: bool,
    /// 当前一级主视图。
    main_view: TuiMainView,
    /// TUI 已观察到的等级模式 leader。
    spotlight_rank_leader_agent_id: Option<String>,
    /// workspace rail 是否折叠为最小宽度。
    sidebar_collapsed: bool,
    /// 可见 Busy 状态的旋转与颜色帧。
    busy_frame: usize,
    /// 上次推进 Busy 动画的时间；无可见 Busy 时为空。
    last_busy_frame_at: Option<Instant>,
    /// 上次绘制的番茄剩余分钟数，用于避免常驻 TUI 每秒输出。
    last_pomodoro_minute: Option<u64>,
    /// 当前键盘输入目标。
    focused_target: Option<AgentTarget>,
    /// 当前 Agent 输出 hover token。
    output_hover: Option<AgentOutputHover>,
    /// 每个已见 pane 的 terminal diff cache。
    panes: BTreeMap<AgentTarget, PaneCache>,
    /// 当前 workspace terminal 的差量 cache。
    workspace_terminal: Option<WorkspaceTerminalCache>,
    /// 当前 workspace Helix 的差量 cache。
    helix_terminal: Option<WorkspaceTerminalCache>,
    /// Helix terminal 是否已接收本轮主键按下，用于限定拖动和松开转发。
    helix_primary_mouse_down: bool,
    /// TUI Workflow 浏览状态。
    workflow_browser: WorkflowBrowser,
    /// TUI Column View 目录浏览状态。
    directory_browser: DirectoryBrowser,
    /// Dir 与 Workflow 共用的 Markdown 编辑工作台集合。
    markdown_workbenches: MarkdownWorkbenches,
    /// 保存成功后才允许执行的 Markdown 导航。
    pending_markdown_navigation: Option<PendingMarkdownNavigation>,
    /// 上一帧图片画廊摘要，用于已打开时的新图增量重绘。
    image_gallery_fingerprint: Option<(bool, usize, usize, usize)>,
    /// 最近一次 Workflow 数据摘要。
    workflow_fingerprint: String,
    /// 当前布局中的可见 panes。
    visible_panes: Vec<PaneLayout>,
    /// 当前布局中的 row/column 折叠恢复区。
    layout_restore_regions: Vec<LayoutRestoreRegion>,
    /// 上次实际写出的 frame。
    previous_frame: Vec<RenderLine>,
    /// 上次实际写出时间。
    last_frame_at: Option<Instant>,
    /// 强制下一帧输出。
    dirty: bool,
    /// 状态和布局变化摘要。
    state_fingerprint: String,
    /// 当前鼠标命中区。
    hit_regions: Vec<HitRegion>,
    /// 当前 frame 的可拖动 splitter 命中区。
    splitter_regions: Vec<SplitterRegion>,
    /// 当前进行中的 splitter 拖动。
    splitter_drag: Option<SplitterDrag>,
    /// 鼠标当前悬停的 splitter，用粗线提供终端内拖动反馈。
    splitter_hover: Option<SplitterTarget>,
    /// 鼠标悬停 splitter 时的 cell 坐标，用箭头标出可拖动方向。
    splitter_hover_position: Option<(u16, u16)>,
    /// 当前唯一菜单、表单或番茄覆盖层。
    layer: Option<TuiLayer>,
    /// 左侧列表滚动偏移。
    sidebar_scroll: usize,
    /// 当前聚焦 Agent 的宿主终端光标位置。
    cursor_position: Option<(u16, u16)>,
    /// 下一帧写入客户端剪贴板的文本。
    pending_clipboard_text: Option<String>,
}

impl TuiState {
    /// 创建 TUI 状态。
    ///
    /// 适用场景：进入 alternate screen 后。示例：fps=4 -> 250ms 闸门。
    fn new(fps: u16, size: (u16, u16)) -> Self {
        Self {
            frame_interval: Duration::from_secs_f64(1.0 / f64::from(fps.max(1))),
            size,
            spotlight_mode: false,
            main_view: TuiMainView::Agents,
            spotlight_rank_leader_agent_id: None,
            sidebar_collapsed: false,
            busy_frame: 0,
            last_busy_frame_at: None,
            last_pomodoro_minute: None,
            focused_target: None,
            output_hover: None,
            panes: BTreeMap::new(),
            workspace_terminal: None,
            helix_terminal: None,
            helix_primary_mouse_down: false,
            workflow_browser: WorkflowBrowser::default(),
            directory_browser: DirectoryBrowser::default(),
            markdown_workbenches: MarkdownWorkbenches::default(),
            pending_markdown_navigation: None,
            image_gallery_fingerprint: None,
            workflow_fingerprint: String::new(),
            visible_panes: Vec::new(),
            layout_restore_regions: Vec::new(),
            previous_frame: Vec::new(),
            last_frame_at: None,
            dirty: true,
            state_fingerprint: String::new(),
            hit_regions: Vec::new(),
            splitter_regions: Vec::new(),
            splitter_drag: None,
            splitter_hover: None,
            splitter_hover_position: None,
            layer: None,
            sidebar_scroll: 0,
            cursor_position: None,
            pending_clipboard_text: None,
        }
    }

    /// 运行输入、AppEvent drain、terminal diff 和限帧绘制循环。
    ///
    /// 适用场景：TUI 生命周期。示例：Ctrl+Q -> Ok。
    fn run(
        &mut self,
        stdout: &mut Stdout,
        app: &mut GsdvGuiApp,
        ctx: &egui::Context,
    ) -> Result<()> {
        loop {
            app.enqueue_update_events(ctx);
            app.drain_app_events(ctx, Instant::now(), Duration::from_millis(8));
            self.apply_pending_tui_navigation(app);
            self.sync_spotlight_rank_target(app);
            self.sync_pomodoro_layer(app);
            self.sync_image_gallery_layer(app);
            app.process_workspace_store_writer(ctx);
            self.sync_layout_and_terminals(app, ctx);
            self.sync_markdown_workbenches(app);
            self.finish_pending_markdown_navigation(app);
            self.request_markdown_images(app);
            self.refresh_state_fingerprint(app);
            self.refresh_pomodoro_countdown(app);
            self.refresh_terminal_updates();
            self.refresh_busy_animation(app);

            if self.dirty && self.frame_due() {
                self.draw(stdout, app)?;
            }

            if event::poll(self.poll_timeout())? {
                let event = event::read()?;
                if self.handle_event(event, app, ctx)? {
                    return Ok(());
                }
            }
        }
    }

    /// 返回距下一次允许绘制的等待时间，并保持后台事件低延迟。
    ///
    /// 适用场景：idle poll。示例：不 dirty -> 20ms。
    fn poll_timeout(&self) -> Duration {
        if !self.dirty {
            return TUI_POLL_INTERVAL;
        }
        let remaining = self
            .last_frame_at
            .map(|last| self.frame_interval.saturating_sub(last.elapsed()))
            .unwrap_or(Duration::ZERO);
        remaining.min(TUI_POLL_INTERVAL)
    }

    /// 判断全局 FPS 闸门是否允许输出。
    ///
    /// 适用场景：输入和多个 PTY 同时变化。示例：4 FPS 内合并为一帧。
    fn frame_due(&self) -> bool {
        self.last_frame_at
            .is_none_or(|last| last.elapsed() >= self.frame_interval)
    }

    /// 消费后台输出分类产生的 TUI 视图导航。
    ///
    /// 适用场景：点击 Agent 文件引用。示例：Markdown -> Dir，Rust -> Helix。
    fn apply_pending_tui_navigation(&mut self, app: &mut GsdvGuiApp) {
        let Some(request) = app.pending_tui_navigation.take() else {
            return;
        };
        match request {
            TuiNavigationRequest::Markdown {
                workspace_index,
                path,
            } => {
                if workspace_index != app.active_workspace {
                    app.switch_workspace(workspace_index);
                }
                self.main_view = TuiMainView::Directory;
                self.directory_browser.workspace_index = Some(workspace_index);
                self.directory_browser.selected_file = Some(path);
                self.directory_browser.content_scroll = 0;
            }
            TuiNavigationRequest::Helix { workspace_index } => {
                if workspace_index != app.active_workspace {
                    app.switch_workspace(workspace_index);
                }
                self.main_view = TuiMainView::Helix;
            }
        }
        self.dirty = true;
    }

    /// 等级模式只在最高优先级停止 Agent 变化时切换 TUI 目标。
    ///
    /// 适用场景：专注模式后台状态变化。示例：全部活跃 -> 保持当前 Agent。
    fn sync_spotlight_rank_target(&mut self, app: &GsdvGuiApp) {
        if self.main_view != TuiMainView::Agents || !self.spotlight_mode || !app.spotlight_rank_mode
        {
            self.spotlight_rank_leader_agent_id = None;
            return;
        }
        let leader = app.spotlight_agents().into_iter().find(|entry| {
            entry.manually_stopped
                || matches!(
                    entry.activity,
                    WorkspaceActivity::Idle | WorkspaceActivity::Error
                )
        });
        let Some(leader) = leader else {
            // 全部 Agent 活跃时不能抢走用户正在查看的目标。
            self.spotlight_rank_leader_agent_id = None;
            return;
        };
        if self.spotlight_rank_leader_agent_id.as_deref() == Some(&leader.agent_id) {
            return;
        }
        self.spotlight_rank_leader_agent_id = Some(leader.agent_id);
        self.focused_target = Some(AgentTarget {
            workspace_index: leader.workspace_index,
            slot: leader.slot,
        });
        self.dirty = true;
    }

    /// 根据现有 workspace row/col 或专注列表生成 pane，并只启动可见 Agent。
    ///
    /// 适用场景：每轮轻量同步。示例：切 tab -> 新 target 建 source。
    fn sync_layout_and_terminals(&mut self, app: &mut GsdvGuiApp, ctx: &egui::Context) {
        if self.main_view != TuiMainView::Agents && !self.layout_restore_regions.is_empty() {
            self.layout_restore_regions.clear();
            self.dirty = true;
        }
        match self.main_view {
            TuiMainView::Workflow => {
                let index = app.active_workspace;
                let should_load = app.workflow_states.get(index).is_some_and(|state| {
                    state.tree.is_none() && !state.loading && state.load_error.is_none()
                });
                if should_load {
                    app.request_workflow_tree_refresh(ctx, index);
                }
                let state = app.workflow_states.get(index);
                sync_browser(
                    &mut self.workflow_browser,
                    index,
                    state.and_then(|state| state.tree.as_ref()),
                );
                if let Some(selected) = state.and_then(|state| state.selected.clone()) {
                    self.workflow_browser.selected = Some(selected);
                }
                let workflow_agent = state
                    .and_then(|state| {
                        let tree = state.tree.as_ref()?;
                        let selected = self.workflow_browser.selected.as_ref()?;
                        let binding = crate::gui::workflow::workflow_agent_binding(tree, selected)?;
                        (state.active_agent_target.as_ref() == Some(&binding.target))
                            .then_some(binding.target)
                    })
                    .and_then(|target| {
                        app.workspaces
                            .get(index)?
                            .subagents
                            .iter()
                            .find_map(|agent| {
                                (agent.workflow_target.as_ref() == Some(&target))
                                    .then_some(agent.id.clone())
                            })
                    });
                let next = workflow_agent
                    .map(|id| {
                        vec![PaneLayout {
                            rect: column_browser::workflow_workbench_rect(self.size),
                            target: AgentTarget {
                                workspace_index: index,
                                slot: AgentSlotId::Subagent(id),
                            },
                            row_index: 0,
                            column_index: 0,
                            column_id: "workflow-agent".to_string(),
                            tabs: Vec::new(),
                        }]
                    })
                    .unwrap_or_default();
                if pane_layout_fingerprint(&next) != pane_layout_fingerprint(&self.visible_panes) {
                    self.visible_panes = next;
                    self.dirty = true;
                }
                let fingerprint = workflow::state_fingerprint(state);
                self.focused_target = self.visible_panes.first().map(|pane| pane.target.clone());
                self.sync_visible_agent_terminals(app, ctx);
                if fingerprint != self.workflow_fingerprint {
                    self.workflow_fingerprint = fingerprint;
                    self.dirty = true;
                }
                return;
            }
            TuiMainView::Directory => {
                if !self.visible_panes.is_empty() {
                    self.visible_panes.clear();
                    self.dirty = true;
                }
                return;
            }
            TuiMainView::Terminal => {
                if !self.visible_panes.is_empty() {
                    self.visible_panes.clear();
                    self.dirty = true;
                }
                self.sync_workspace_terminal(app, ctx);
                return;
            }
            TuiMainView::Helix => {
                if !self.visible_panes.is_empty() {
                    self.visible_panes.clear();
                    self.dirty = true;
                }
                self.sync_helix_terminal(app);
                return;
            }
            TuiMainView::Agents => {}
        }
        let next = self.compute_pane_layout(app);
        if pane_layout_fingerprint(&next.panes) != pane_layout_fingerprint(&self.visible_panes)
            || next.restore_regions != self.layout_restore_regions
        {
            self.visible_panes = next.panes;
            self.layout_restore_regions = next.restore_regions;
            self.dirty = true;
        }
        if self
            .focused_target
            .as_ref()
            .is_none_or(|target| !self.visible_panes.iter().any(|pane| &pane.target == target))
        {
            self.focused_target = self.visible_panes.first().map(|pane| pane.target.clone());
        }

        self.sync_visible_agent_terminals(app, ctx);
    }

    /// 同步当前可见 Agent pane 的 host、尺寸与远程输出缓存。
    ///
    /// 适用场景：普通 Agent 网格与 Workflow 工作台共用。例：切换目标 -> 创建对应缓存。
    fn sync_visible_agent_terminals(&mut self, app: &mut GsdvGuiApp, ctx: &egui::Context) {
        for pane in &self.visible_panes {
            app.ensure_agent_terminal_host(ctx, pane.target.workspace_index, &pane.target.slot);
            let body = pane_body_rect(pane);
            let terminal_lines = body.height.max(1);
            let terminal_cols = body.width.max(1);
            let Some(host) = app
                .terminal_hosts
                .get_mut(pane.target.workspace_index)
                .and_then(|hosts| hosts.agents.get_mut(&pane.target.slot))
                .and_then(|slot| slot.host.as_mut())
            else {
                continue;
            };
            let grid_size = (terminal_cols, terminal_lines);
            let host_id = host.id();
            if self
                .panes
                .get(&pane.target)
                .is_some_and(|cache| cache.host_id != host_id)
            {
                self.panes.remove(&pane.target);
            }
            if self
                .panes
                .get(&pane.target)
                .is_none_or(|cache| cache.grid_size != grid_size)
            {
                host.resize_remote_grid(terminal_cols, terminal_lines);
            }
            if !self.panes.contains_key(&pane.target) {
                let source = host.tui_output_source();
                let (frame, output_state) = source.frame(0);
                self.panes.insert(
                    pane.target.clone(),
                    PaneCache {
                        host_id,
                        source,
                        output_state,
                        frame,
                        grid_size,
                        scroll_offset: 0,
                    },
                );
                self.dirty = true;
            } else if let Some(cache) = self.panes.get_mut(&pane.target) {
                cache.grid_size = grid_size;
            }
        }
    }

    /// 判断 pane 是否是 workspace 记录的唯一输入焦点。
    ///
    /// 适用场景：同一 Agent 可出现在多个 pane，不能用 AgentTarget 猜焦点。示例：重复 target -> 仅 row/column 匹配项为 true。
    fn pane_is_focused(&self, app: &GsdvGuiApp, pane: &PaneLayout) -> bool {
        if self.spotlight_mode || self.main_view == TuiMainView::Workflow {
            return self.focused_target.as_ref() == Some(&pane.target);
        }
        app.workspaces
            .get(pane.target.workspace_index)
            .and_then(|workspace| workspace.agent_focus)
            .is_some_and(|focus| {
                focus.row_index == pane.row_index && focus.column_index == pane.column_index
            })
    }

    /// 返回当前 focused pane 的边界色，Busy 时随状态帧交替。
    ///
    /// 适用场景：顶部、外沿和共享 splitter 必须同步闪烁。示例：Busy -> 绿/青交替，Idle -> 青色。
    fn focused_pane_border_color(&self, app: &GsdvGuiApp) -> (u8, u8, u8) {
        let busy = self
            .visible_panes
            .iter()
            .find(|pane| self.pane_is_focused(app, pane))
            .is_some_and(|pane| {
                app.agent_slot_activity(pane.target.workspace_index, &pane.target.slot)
                    == WorkspaceActivity::Busy
            });
        if busy {
            busy_animation_color(self.busy_frame)
        } else {
            (34, 211, 238)
        }
    }

    /// 从共享 Document/Workflow 状态同步 TUI Markdown 草稿基线。
    ///
    /// 适用场景：异步加载、保存或 watcher 刷新到达。示例：保存成功 -> dirty 清除。
    fn sync_markdown_workbenches(&mut self, app: &GsdvGuiApp) {
        let changed = match self.main_view {
            TuiMainView::Workflow => column_browser::sync_workflow_workbenches(
                &self.workflow_browser,
                app.workflow_states.get(app.active_workspace),
                &mut self.markdown_workbenches,
                true,
                app.workspaces
                    .get(app.active_workspace)
                    .map(|workspace| workspace.path.as_path())
                    .unwrap_or_else(|| Path::new(".")),
            ),
            TuiMainView::Directory => column_browser::sync_directory_workbench(
                &self.directory_browser,
                app.documents.get(app.active_workspace),
                &mut self.markdown_workbenches,
            ),
            TuiMainView::Agents | TuiMainView::Terminal | TuiMainView::Helix => false,
        };
        if changed {
            self.dirty = true;
        }
    }

    /// 为当前 TUI Markdown 草稿派发去重后的本地图片加载。
    ///
    /// 适用场景：预览首次出现或编辑新增图片引用。示例：新 `a.png` -> 后台 decode。
    fn request_markdown_images(&self, app: &mut GsdvGuiApp) {
        let paths = self
            .markdown_workbenches
            .items
            .values()
            .flat_map(|item| item.image_paths.iter().cloned())
            .collect::<BTreeSet<_>>();
        for path in paths {
            app.request_local_image(path, false);
        }
    }

    /// 创建或同步当前 workspace terminal 的输出 cache 与字符尺寸。
    ///
    /// 适用场景：进入 Terminal、resize 或切换 workspace。示例：120x40 -> PTY 120x39。
    fn sync_workspace_terminal(&mut self, app: &mut GsdvGuiApp, ctx: &egui::Context) {
        let index = app.active_workspace;
        app.spawn_terminal_host_for_workspace(ctx, index, TerminalSurfaceKind::Workspace);
        let terminal_cols = self.size.0.max(1);
        let terminal_lines = self.size.1.saturating_sub(1).max(1);
        let Some(host) = app
            .terminal_hosts
            .get_mut(index)
            .and_then(|hosts| hosts.workspace.as_mut())
        else {
            return;
        };
        let host_id = host.id();
        let grid_size = (terminal_cols, terminal_lines);
        let replace = self
            .workspace_terminal
            .as_ref()
            .is_none_or(|cache| cache.workspace_index != index || cache.host_id != host_id);
        if replace {
            let source = host.tui_output_source();
            let (frame, output_state) = source.frame(0);
            self.workspace_terminal = Some(WorkspaceTerminalCache {
                workspace_index: index,
                host_id,
                source,
                output_state,
                frame,
                grid_size,
                scroll_offset: 0,
            });
            host.resize_remote_grid(terminal_cols, terminal_lines);
            self.dirty = true;
            return;
        }
        if let Some(cache) = self.workspace_terminal.as_mut()
            && cache.grid_size != grid_size
        {
            host.resize_remote_grid(terminal_cols, terminal_lines);
            cache.grid_size = grid_size;
            self.dirty = true;
        }
    }

    /// 创建或同步当前 workspace Helix 的输出 cache 与字符尺寸。
    ///
    /// 适用场景：底栏打开 Helix 或文件点击完成。示例：120x40 -> Helix 120x39。
    fn sync_helix_terminal(&mut self, app: &mut GsdvGuiApp) {
        let index = app.active_workspace;
        let terminal_cols = self.size.0.max(1);
        let terminal_lines = self.size.1.saturating_sub(1).max(1);
        let Some(host) = app
            .terminal_hosts
            .get_mut(index)
            .and_then(|hosts| hosts.helix.as_mut())
        else {
            return;
        };
        let host_id = host.id();
        let grid_size = (terminal_cols, terminal_lines);
        let replace = self
            .helix_terminal
            .as_ref()
            .is_none_or(|cache| cache.workspace_index != index || cache.host_id != host_id);
        if replace {
            let source = host.tui_output_source();
            let (frame, output_state) = source.frame(0);
            self.helix_terminal = Some(WorkspaceTerminalCache {
                workspace_index: index,
                host_id,
                source,
                output_state,
                frame,
                grid_size,
                scroll_offset: 0,
            });
            host.resize_remote_grid(terminal_cols, terminal_lines);
            self.dirty = true;
            return;
        }
        if let Some(cache) = self.helix_terminal.as_mut()
            && cache.grid_size != grid_size
        {
            host.resize_remote_grid(terminal_cols, terminal_lines);
            cache.grid_size = grid_size;
            self.dirty = true;
        }
    }

    /// 计算当前模式的可见 pane 字符矩形。
    ///
    /// 适用场景：resize、workspace 布局变化。示例：两列 -> 各占可用宽度一半。
    fn compute_pane_layout(&self, app: &GsdvGuiApp) -> AgentPaneLayout {
        let (width, height) = self.size;
        if width < TUI_MIN_WIDTH || height < TUI_MIN_HEIGHT {
            return AgentPaneLayout::default();
        }
        let sidebar = sidebar_width(width, self.sidebar_collapsed);
        let sidebar_separator = u16::from(sidebar > 0);
        let body = TuiRect {
            x: sidebar.saturating_add(sidebar_separator),
            y: 0,
            width: width.saturating_sub(sidebar.saturating_add(sidebar_separator)),
            height: height.saturating_sub(1),
        };
        if self.spotlight_mode {
            let spotlight_agents = app.spotlight_agents();
            let target = self
                .focused_target
                .clone()
                .filter(|target| {
                    spotlight_agents.iter().any(|entry| {
                        entry.workspace_index == target.workspace_index && entry.slot == target.slot
                    })
                })
                .or_else(|| {
                    spotlight_agents.first().map(|entry| AgentTarget {
                        workspace_index: entry.workspace_index,
                        slot: entry.slot.clone(),
                    })
                });
            return AgentPaneLayout {
                panes: target
                    .map(|target| PaneLayout {
                        rect: body,
                        target,
                        row_index: 0,
                        column_index: 0,
                        column_id: "spotlight".to_string(),
                        tabs: Vec::new(),
                    })
                    .into_iter()
                    .collect(),
                restore_regions: Vec::new(),
            };
        }
        let Some(workspace) = app.workspaces.get(app.active_workspace) else {
            return AgentPaneLayout::default();
        };
        let visible_rows = workspace
            .agent_rows
            .iter()
            .enumerate()
            .filter(|(_, row)| !row.collapsed)
            .collect::<Vec<_>>();
        let collapsed_row_height = u16::try_from(
            workspace
                .agent_rows
                .iter()
                .filter(|row| row.collapsed)
                .count(),
        )
        .unwrap_or(u16::MAX)
        .saturating_mul(TUI_COLLAPSED_ROW_HEIGHT);
        let row_splitters = u16::try_from(
            workspace
                .agent_rows
                .windows(2)
                .filter(|pair| !pair[0].collapsed && !pair[1].collapsed)
                .count(),
        )
        .unwrap_or(u16::MAX)
        .saturating_mul(TUI_SPLITTER_SIZE);
        let row_content_height = body
            .height
            .saturating_sub(collapsed_row_height)
            .saturating_sub(row_splitters);
        let row_heights = weighted_lengths(
            row_content_height,
            &visible_rows
                .iter()
                .map(|(_, row)| row.height_weight)
                .collect::<Vec<_>>(),
        );
        let mut panes = Vec::new();
        let mut restore_regions = Vec::new();
        let mut row_heights = row_heights.into_iter();
        let mut y = body.y;
        let body_bottom = body.y.saturating_add(body.height);
        for (row_index, row) in workspace.agent_rows.iter().enumerate() {
            if row.collapsed {
                let row_height = TUI_COLLAPSED_ROW_HEIGHT.min(body_bottom.saturating_sub(y));
                if row_height > 0 {
                    restore_regions.push(LayoutRestoreRegion {
                        rect: TuiRect {
                            x: body.x,
                            y,
                            width: body.width,
                            height: row_height,
                        },
                        target: LayoutRestoreTarget::Row {
                            workspace_index: app.active_workspace,
                            row_index,
                        },
                    });
                }
                y = y.saturating_add(row_height);
                continue;
            }
            let row_height = row_heights.next().unwrap_or_default();
            let visible_columns = row
                .columns
                .iter()
                .enumerate()
                .filter(|(_, column)| !column.collapsed)
                .collect::<Vec<_>>();
            let collapsed_column_width =
                u16::try_from(row.columns.iter().filter(|column| column.collapsed).count())
                    .unwrap_or(u16::MAX)
                    .saturating_mul(TUI_COLLAPSED_COLUMN_WIDTH);
            let column_splitters = u16::try_from(
                row.columns
                    .windows(2)
                    .filter(|pair| !pair[0].collapsed && !pair[1].collapsed)
                    .count(),
            )
            .unwrap_or(u16::MAX)
            .saturating_mul(TUI_SPLITTER_SIZE);
            let column_content_width = body
                .width
                .saturating_sub(collapsed_column_width)
                .saturating_sub(column_splitters);
            let widths = weighted_lengths(
                column_content_width,
                &visible_columns
                    .iter()
                    .map(|(_, column)| column.width_weight)
                    .collect::<Vec<_>>(),
            );
            let mut widths = widths.into_iter();
            let mut x = body.x;
            let body_right = body.x.saturating_add(body.width);
            for (column_index, column) in row.columns.iter().enumerate() {
                if column.collapsed {
                    let column_width = TUI_COLLAPSED_COLUMN_WIDTH.min(body_right.saturating_sub(x));
                    if column_width > 0 && row_height > 0 {
                        restore_regions.push(LayoutRestoreRegion {
                            rect: TuiRect {
                                x,
                                y,
                                width: column_width,
                                height: row_height,
                            },
                            target: LayoutRestoreTarget::Column {
                                workspace_index: app.active_workspace,
                                row_index,
                                column_index,
                            },
                        });
                    }
                    x = x.saturating_add(column_width);
                    continue;
                }
                let column_width = widths.next().unwrap_or_default();
                let slot = AgentSlotId::from_column_slot(&column.active_slot);
                let tabs = column
                    .tabs
                    .iter()
                    .map(AgentSlotId::from_column_slot)
                    .collect::<Vec<_>>();
                panes.push(PaneLayout {
                    rect: TuiRect {
                        x,
                        y,
                        width: column_width,
                        height: row_height,
                    },
                    target: AgentTarget {
                        workspace_index: app.active_workspace,
                        slot,
                    },
                    row_index,
                    column_index,
                    column_id: column.id.clone(),
                    tabs,
                });
                x = x.saturating_add(column_width);
                if row
                    .columns
                    .get(column_index + 1)
                    .is_some_and(|next| !next.collapsed)
                {
                    x = x.saturating_add(TUI_SPLITTER_SIZE);
                }
            }
            y = y.saturating_add(row_height);
            if workspace
                .agent_rows
                .get(row_index + 1)
                .is_some_and(|next| !next.collapsed)
            {
                y = y.saturating_add(TUI_SPLITTER_SIZE);
            }
        }
        AgentPaneLayout {
            panes,
            restore_regions,
        }
    }

    /// 消费所有可见 terminal source 的最小更新。
    ///
    /// 适用场景：PTY 输出变化。示例：Rows 只替换 cache 对应行。
    fn refresh_terminal_updates(&mut self) {
        if matches!(self.main_view, TuiMainView::Terminal | TuiMainView::Helix) {
            let cache = match self.main_view {
                TuiMainView::Terminal => self.workspace_terminal.as_mut(),
                TuiMainView::Helix => self.helix_terminal.as_mut(),
                TuiMainView::Agents | TuiMainView::Workflow | TuiMainView::Directory => None,
            };
            if let Some(cache) = cache {
                if apply_terminal_tui_update(
                    &mut cache.frame,
                    cache
                        .source
                        .update_since(&mut cache.output_state, cache.scroll_offset),
                ) {
                    self.dirty = true;
                }
            }
            return;
        }
        let mut changed = false;
        for pane in &self.visible_panes {
            let Some(cache) = self.panes.get_mut(&pane.target) else {
                continue;
            };
            if apply_terminal_tui_update(
                &mut cache.frame,
                cache
                    .source
                    .update_since(&mut cache.output_state, cache.scroll_offset),
            ) {
                changed = true;
            }
        }
        if changed {
            self.output_hover = None;
            self.dirty = true;
        }
    }

    /// 仅在屏幕存在 Busy Agent 时低频推进状态动画。
    ///
    /// 适用场景：后台 Busy tab 没有 PTY damage。示例：切到 Idle tab -> Busy tab 仍旋转变色。
    fn refresh_busy_animation(&mut self, app: &GsdvGuiApp) {
        if !self.has_visible_busy_agent(app) {
            self.last_busy_frame_at = None;
            return;
        }
        let now = Instant::now();
        if self
            .last_busy_frame_at
            .is_some_and(|last| now.saturating_duration_since(last) < TUI_BUSY_ANIMATION_INTERVAL)
        {
            return;
        }
        self.last_busy_frame_at = Some(now);
        self.busy_frame = self.busy_frame.wrapping_add(1);
        self.dirty = true;
    }

    /// 判断当前 TUI 画面是否包含需要动画的 Busy 状态。
    ///
    /// 适用场景：控制 Busy 动画生命周期。示例：同 column 的非活动 Busy tab -> true。
    fn has_visible_busy_agent(&self, app: &GsdvGuiApp) -> bool {
        let pane_busy = self.visible_panes.iter().any(|pane| {
            let slots = if pane.tabs.is_empty() {
                std::slice::from_ref(&pane.target.slot)
            } else {
                pane.tabs.as_slice()
            };
            slots.iter().any(|slot| {
                app.agent_slot_activity(pane.target.workspace_index, slot)
                    == WorkspaceActivity::Busy
            })
        });
        if pane_busy || self.main_view != TuiMainView::Agents || self.sidebar_collapsed {
            return pane_busy;
        }
        if self.spotlight_mode {
            return app
                .spotlight_agents()
                .iter()
                .any(|entry| entry.activity == WorkspaceActivity::Busy);
        }
        app.workspaces.iter().enumerate().any(|(index, workspace)| {
            (!app.focus_filter_enabled || workspace.focused)
                && app.agent_slot_activity(index, &AgentSlotId::Main) == WorkspaceActivity::Busy
        })
    }

    /// 比较 Agent 状态、workspace 和番茄钟摘要，发现变化后标脏。
    ///
    /// 适用场景：Hook 只改 metadata、没有 PTY 输出。示例：Busy -> Idle。
    fn refresh_state_fingerprint(&mut self, app: &GsdvGuiApp) {
        let mut next = format!("{}:{:?}", app.active_workspace, self.main_view);
        if self.main_view == TuiMainView::Agents {
            next.push_str(&format!(
                ":{}:{}:{:?}:{:?}:{}:{}:{}",
                self.spotlight_mode,
                app.spotlight_rank_mode,
                app.pomodoro.phase,
                app.runtime_settings.pomodoro_enabled,
                app.runtime_settings.pomodoro_work_minutes,
                app.runtime_settings.pomodoro_rest_minutes,
                app.runtime_settings.pomodoro_warning_remaining_percent,
            ));
            for (index, workspace) in app.workspaces.iter().enumerate() {
                next.push_str(&format!(
                    "|{index}:{}:{:?}:{}:{}",
                    workspace.name,
                    workspace.activity,
                    workspace.focused,
                    workspace
                        .subagents
                        .iter()
                        .filter(|agent| agent.workflow_target.is_none())
                        .count()
                ));
                for subagent in workspace
                    .subagents
                    .iter()
                    .filter(|agent| agent.workflow_target.is_none())
                {
                    next.push_str(&format!(":{}:{:?}", subagent.name, subagent.activity));
                }
            }
        } else if let Some(workspace) = app.workspaces.get(app.active_workspace) {
            next.push_str(&format!(":{}", workspace.name));
            if self.main_view == TuiMainView::Workflow {
                for agent in workspace
                    .subagents
                    .iter()
                    .filter(|agent| agent.workflow_target.is_some())
                {
                    next.push_str(&format!(
                        ":{:?}:{:?}",
                        agent.workflow_target, agent.activity
                    ));
                }
            }
        }
        if next != self.state_fingerprint {
            self.state_fingerprint = next;
            self.dirty = true;
        }
    }

    /// 工作到点后用唯一覆盖层抢占当前菜单或表单。
    ///
    /// 适用场景：Working 自动进入 Resting。示例：任意当前层 -> 全屏 R。
    fn sync_pomodoro_layer(&mut self, app: &GsdvGuiApp) {
        let rest_due = app.runtime_settings.pomodoro_enabled
            && matches!(
                app.pomodoro.phase,
                PomodoroPhase::WaitingForRestQuiet
                    | PomodoroPhase::Resting
                    | PomodoroPhase::ReadyToWork
            );
        if rest_due && !matches!(self.layer, Some(TuiLayer::Rest)) {
            self.layer = Some(TuiLayer::Rest);
            self.dirty = true;
        } else if !rest_due && matches!(self.layer, Some(TuiLayer::Rest)) {
            self.layer = None;
            self.dirty = true;
        }
    }

    /// 让 socket 图片请求自动打开 TUI 顶层画廊。
    ///
    /// 适用场景：`image.show` 在 TUI 运行期间到达。示例：首张图 -> Gallery layer。
    fn sync_image_gallery_layer(&mut self, app: &GsdvGuiApp) {
        let fingerprint = (
            app.image_gallery.open,
            app.image_gallery.paths.len(),
            app.image_gallery.current,
            app.image_gallery.unseen,
        );
        if self.image_gallery_fingerprint != Some(fingerprint) {
            self.image_gallery_fingerprint = Some(fingerprint);
            self.dirty = true;
        }
        if app.image_gallery.open && self.layer.is_none() {
            self.layer = Some(TuiLayer::Gallery);
            self.dirty = true;
        } else if !app.image_gallery.open && matches!(self.layer, Some(TuiLayer::Gallery)) {
            self.layer = None;
            self.dirty = true;
        }
    }

    /// 番茄倒计时只在显示分钟数变化时标记重绘。
    ///
    /// 适用场景：底栏、Alt 控制层和全屏 R。示例：25m -> 24m 重绘一次。
    fn refresh_pomodoro_countdown(&mut self, app: &GsdvGuiApp) {
        if self.main_view != TuiMainView::Agents
            && !matches!(self.layer, Some(TuiLayer::Pomodoro | TuiLayer::Rest))
        {
            return;
        }
        let minute = pomodoro_remaining_minutes(app);
        if minute != self.last_pomodoro_minute {
            self.last_pomodoro_minute = minute;
            self.dirty = true;
        }
    }

    /// 解析 crossterm 事件并返回是否退出。
    ///
    /// 适用场景：主循环。示例：Resize -> 重新布局；Ctrl+Q -> true。
    fn handle_event(
        &mut self,
        event: TerminalEvent,
        app: &mut GsdvGuiApp,
        ctx: &egui::Context,
    ) -> Result<bool> {
        match event {
            TerminalEvent::Resize(width, height) => {
                self.size = (width, height);
                self.previous_frame.clear();
                self.dirty = true;
            }
            TerminalEvent::Mouse(event) => self.handle_mouse(event, app),
            TerminalEvent::Paste(text) => self.handle_paste(text, app),
            TerminalEvent::Key(key) if key.kind != KeyEventKind::Release => {
                if matches!(self.layer, Some(TuiLayer::Gallery)) {
                    self.handle_gallery_key(key, app);
                } else if matches!(self.layer, Some(TuiLayer::Rest)) {
                    self.handle_rest_key(key, app);
                } else if matches!(self.layer, Some(TuiLayer::Pomodoro)) {
                    self.handle_pomodoro_key(key, app);
                } else if matches!(
                    self.layer,
                    Some(
                        TuiLayer::Form(_)
                            | TuiLayer::Confirm { .. }
                            | TuiLayer::MarkdownUnsaved { .. }
                    )
                ) {
                    self.handle_overlay_key(key, app);
                } else if matches!(self.layer, Some(TuiLayer::Menu(_))) {
                    self.handle_menu_key(key, app);
                } else if self.handle_global_key(key, app) {
                    return Ok(true);
                } else if self.markdown_workbenches.editing()
                    && self.markdown_workbenches.handle_key(key)
                {
                    self.dirty = true;
                } else if self.main_view == TuiMainView::Workflow && !self.visible_panes.is_empty()
                {
                    if key.code == KeyCode::Enter {
                        self.queue_terminal_key(app, TerminalRemoteKey::Enter);
                    } else if let Some(bytes) = terminal_bytes_for_key(key) {
                        self.queue_terminal_bytes(app, bytes);
                    }
                    app.queue_app_event(AppEvent::PomodoroInputDetected);
                } else if self.main_view == TuiMainView::Workflow {
                    self.handle_workflow_key(key, app);
                } else if self.main_view == TuiMainView::Directory {
                    if key.code == KeyCode::Esc {
                        self.main_view = TuiMainView::Agents;
                        self.dirty = true;
                    }
                } else if self.main_view == TuiMainView::Terminal {
                    if key.code == KeyCode::Enter {
                        self.queue_workspace_terminal_bytes(app, vec![b'\r']);
                    } else if let Some(bytes) = terminal_bytes_for_key(key) {
                        self.queue_workspace_terminal_bytes(app, bytes);
                    }
                } else if self.main_view == TuiMainView::Helix {
                    if key.code == KeyCode::Enter {
                        self.queue_helix_terminal_bytes(app, vec![b'\r']);
                    } else if let Some(bytes) = terminal_bytes_for_key(key) {
                        self.queue_helix_terminal_bytes(app, bytes);
                    }
                } else if key.code == KeyCode::Enter {
                    self.queue_terminal_key(app, TerminalRemoteKey::Enter);
                    app.queue_app_event(AppEvent::PomodoroInputDetected);
                } else if let Some(bytes) = terminal_bytes_for_key(key) {
                    self.queue_terminal_bytes(app, bytes);
                    app.queue_app_event(AppEvent::PomodoroInputDetected);
                }
            }
            _ => {}
        }
        app.drain_app_events(ctx, Instant::now(), Duration::from_millis(8));
        Ok(false)
    }

    /// 处理无浮层时的 TUI 全局按键。
    ///
    /// 适用场景：退出、专注切换、pane 导航。示例：F2 -> spotlight。
    fn handle_global_key(&mut self, key: KeyEvent, app: &GsdvGuiApp) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
            return true;
        }
        if self.main_view != TuiMainView::Agents {
            return false;
        }
        let bare_alt = matches!(
            key.code,
            KeyCode::Modifier(ModifierKeyCode::LeftAlt | ModifierKeyCode::RightAlt)
        );
        let alt_fallback = key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Char('p' | 'P'));
        if bare_alt || alt_fallback {
            self.layer = Some(TuiLayer::Pomodoro);
            self.dirty = true;
            return false;
        }
        if key.code == KeyCode::F(2) {
            self.spotlight_mode = !self.spotlight_mode;
            self.spotlight_rank_leader_agent_id = None;
            self.focused_target = if self.spotlight_mode {
                app.spotlight_agents().first().map(|entry| AgentTarget {
                    workspace_index: entry.workspace_index,
                    slot: entry.slot.clone(),
                })
            } else {
                None
            };
            self.dirty = true;
            return false;
        }
        if key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Up | KeyCode::Down)
        {
            self.cycle_agent(app, key.code == KeyCode::Down);
            return false;
        }
        false
    }

    /// 处理只读 Workflow 浏览器键盘导航。
    ///
    /// 适用场景：Esc 返回、方向键选节点、PageUp/PageDown 滚正文。
    fn handle_workflow_key(&mut self, key: KeyEvent, app: &GsdvGuiApp) {
        match key.code {
            KeyCode::Esc => {
                self.main_view = TuiMainView::Agents;
                self.focused_target = None;
            }
            KeyCode::Up | KeyCode::Down => {
                let Some(tree) = app
                    .workflow_states
                    .get(app.active_workspace)
                    .and_then(|state| state.tree.as_ref())
                else {
                    return;
                };
                let rows = workflow_rows(tree, &self.workflow_browser.collapsed_projects)
                    .into_iter()
                    .filter_map(|row| row.target)
                    .collect::<Vec<_>>();
                if rows.is_empty() {
                    return;
                }
                let current = self
                    .workflow_browser
                    .selected
                    .as_ref()
                    .and_then(|selected| rows.iter().position(|target| target == selected))
                    .unwrap_or(0);
                let next = if key.code == KeyCode::Down {
                    (current + 1) % rows.len()
                } else {
                    (current + rows.len() - 1) % rows.len()
                };
                self.select_workflow_target(app, rows[next].clone());
            }
            KeyCode::PageUp => match self.workflow_browser.selected.as_ref() {
                Some(WorkflowSelectionTarget::Step { .. }) => {
                    self.workflow_browser.step_content_scroll =
                        self.workflow_browser.step_content_scroll.saturating_sub(8);
                }
                Some(WorkflowSelectionTarget::Task { .. }) => {
                    self.workflow_browser.task_scroll =
                        self.workflow_browser.task_scroll.saturating_sub(8);
                }
                _ => {
                    self.workflow_browser.content_scroll =
                        self.workflow_browser.content_scroll.saturating_sub(8);
                }
            },
            KeyCode::PageDown => match self.workflow_browser.selected.as_ref() {
                Some(WorkflowSelectionTarget::Step { .. }) => {
                    self.workflow_browser.step_content_scroll =
                        self.workflow_browser.step_content_scroll.saturating_add(8);
                }
                Some(WorkflowSelectionTarget::Task { .. }) => {
                    self.workflow_browser.task_scroll =
                        self.workflow_browser.task_scroll.saturating_add(8);
                }
                _ => {
                    self.workflow_browser.content_scroll =
                        self.workflow_browser.content_scroll.saturating_add(8);
                }
            },
            _ => {}
        }
        self.dirty = true;
    }

    /// 处理 Alt 番茄控制层的直接操作。
    ///
    /// 适用场景：控制层打开。示例：`r` -> 立即休息并进入全屏 R。
    fn handle_pomodoro_key(&mut self, key: KeyEvent, app: &GsdvGuiApp) {
        match key.code {
            KeyCode::Esc => self.layer = None,
            KeyCode::Char(' ') => app.queue_app_event(AppEvent::TuiAction(
                TuiAppAction::SetPomodoroEnabled(!app.runtime_settings.pomodoro_enabled),
            )),
            KeyCode::Char('w' | 'W') => {
                app.queue_app_event(AppEvent::TuiAction(TuiAppAction::StartPomodoroWork))
            }
            KeyCode::Char('r' | 'R') => {
                app.queue_app_event(AppEvent::TuiAction(TuiAppAction::StartPomodoroRest))
            }
            KeyCode::Char('s' | 'S') => {
                self.layer = Some(TuiLayer::Form(pomodoro_settings_form(app)));
            }
            _ => {}
        }
        self.dirty = true;
    }

    /// 处理图片画廊键盘翻页和关闭。
    ///
    /// 适用场景：Gallery 独占输入。示例：Right -> 下一张，Esc -> 清空队列。
    fn handle_gallery_key(&mut self, key: KeyEvent, app: &GsdvGuiApp) {
        match key.code {
            KeyCode::Left => {
                app.queue_app_event(AppEvent::TuiAction(TuiAppAction::MoveImageGallery(-1)))
            }
            KeyCode::Right => {
                app.queue_app_event(AppEvent::TuiAction(TuiAppAction::MoveImageGallery(1)))
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                app.queue_app_event(AppEvent::TuiAction(TuiAppAction::CloseImageGallery))
            }
            _ => {}
        }
        self.dirty = true;
    }

    /// 处理全屏休息层；Esc 明确结束本轮休息并回到工作阶段。
    ///
    /// 适用场景：Resting 或 ReadyToWork。示例：Esc -> StartPomodoroWork。
    fn handle_rest_key(&mut self, key: KeyEvent, app: &GsdvGuiApp) {
        if key.code == KeyCode::Esc {
            app.queue_app_event(AppEvent::TuiAction(TuiAppAction::StartPomodoroWork));
            self.layer = None;
            self.dirty = true;
        }
    }

    /// 处理番茄控制层按钮点击。
    ///
    /// 适用场景：鼠标点击 Alt 面板。示例：Settings -> 参数表单。
    fn handle_pomodoro_mouse(&mut self, event: MouseEvent, app: &GsdvGuiApp) {
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let (x, y, width, _) = pomodoro_panel_rect(self.size);
        let row = y + 7;
        if event.row != row || event.column < x + 2 || event.column >= x + width - 2 {
            return;
        }
        let relative = event.column.saturating_sub(x + 2);
        if relative < 12 {
            app.queue_app_event(AppEvent::TuiAction(TuiAppAction::SetPomodoroEnabled(
                !app.runtime_settings.pomodoro_enabled,
            )));
        } else if relative < 27 {
            app.queue_app_event(AppEvent::TuiAction(TuiAppAction::StartPomodoroWork));
        } else if relative < 41 {
            app.queue_app_event(AppEvent::TuiAction(TuiAppAction::StartPomodoroRest));
        } else {
            self.layer = Some(TuiLayer::Form(pomodoro_settings_form(app)));
        }
        self.dirty = true;
    }

    /// 在当前左侧列表中前后切换 Agent。
    ///
    /// 适用场景：Alt+Up/Down。示例：当前第二项 + Down -> 第三项。
    fn cycle_agent(&mut self, app: &GsdvGuiApp, forward: bool) {
        if !self.spotlight_mode {
            if self.visible_panes.is_empty() {
                return;
            }
            let current = self
                .visible_panes
                .iter()
                .position(|pane| self.pane_is_focused(app, pane))
                .unwrap_or(0);
            let next = if forward {
                (current + 1) % self.visible_panes.len()
            } else {
                (current + self.visible_panes.len() - 1) % self.visible_panes.len()
            };
            let pane = self.visible_panes[next].clone();
            self.focused_target = Some(pane.target.clone());
            app.queue_app_event(AppEvent::TuiAction(TuiAppAction::SelectAgent {
                workspace_index: pane.target.workspace_index,
                row_index: pane.row_index,
                column_index: pane.column_index,
                slot: pane.target.slot,
            }));
            self.dirty = true;
            return;
        }
        let targets = if self.spotlight_mode {
            app.spotlight_agents()
                .into_iter()
                .map(|entry| AgentTarget {
                    workspace_index: entry.workspace_index,
                    slot: entry.slot,
                })
                .collect::<Vec<_>>()
        } else {
            self.visible_panes
                .iter()
                .map(|pane| pane.target.clone())
                .collect::<Vec<_>>()
        };
        if targets.is_empty() {
            return;
        }
        let current = self
            .focused_target
            .as_ref()
            .and_then(|target| targets.iter().position(|item| item == target))
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % targets.len()
        } else {
            (current + targets.len() - 1) % targets.len()
        };
        self.focused_target = Some(targets[next].clone());
        self.dirty = true;
    }

    /// 把普通按键字节投递到现有 terminal AppEvent 路径。
    ///
    /// 适用场景：TUI 无菜单输入。示例：`h` -> 当前 Agent PTY。
    fn queue_terminal_bytes(&mut self, app: &GsdvGuiApp, bytes: Vec<u8>) {
        let Some(target) = self.focused_target.clone() else {
            return;
        };
        if let Some(cache) = self.panes.get_mut(&target) {
            cache.scroll_offset = 0;
            cache.frame = cache.source.view(0);
        }
        app.queue_app_event(AppEvent::InputTerminalBytes {
            workspace_index: target.workspace_index,
            target: TerminalSurfaceKind::Agent,
            agent_slot: target.slot,
            bytes,
        });
    }

    /// 把普通按键字节投递给当前 workspace terminal。
    ///
    /// 适用场景：Terminal 主视图。示例：输入 ls -> Workspace PTY。
    fn queue_workspace_terminal_bytes(&mut self, app: &GsdvGuiApp, bytes: Vec<u8>) {
        if let Some(cache) = self.workspace_terminal.as_mut() {
            cache.scroll_offset = 0;
            cache.frame = cache.source.view(0);
        }
        app.queue_app_event(AppEvent::InputTerminalBytes {
            workspace_index: app.active_workspace,
            target: TerminalSurfaceKind::Workspace,
            agent_slot: AgentSlotId::Main,
            bytes,
        });
    }

    /// 把普通按键字节投递给当前 workspace 的 Helix。
    ///
    /// 适用场景：Helix 主视图。示例：输入 `i` -> 进入插入模式。
    fn queue_helix_terminal_bytes(&mut self, app: &GsdvGuiApp, bytes: Vec<u8>) {
        if let Some(cache) = self.helix_terminal.as_mut() {
            cache.scroll_offset = 0;
            cache.frame = cache.source.view(0);
        }
        app.queue_app_event(AppEvent::InputTerminalBytes {
            workspace_index: app.active_workspace,
            target: TerminalSurfaceKind::Helix,
            agent_slot: AgentSlotId::Main,
            bytes,
        });
    }

    /// 把 Enter 等特殊键投递到现有 terminal 协议编码路径。
    ///
    /// 适用场景：Codex 开启 kitty keyboard protocol。示例：Enter -> CSI 13u。
    fn queue_terminal_key(&mut self, app: &GsdvGuiApp, key: TerminalRemoteKey) {
        let Some(target) = self.focused_target.clone() else {
            return;
        };
        if let Some(cache) = self.panes.get_mut(&target) {
            cache.scroll_offset = 0;
            cache.frame = cache.source.view(0);
        }
        app.queue_app_event(AppEvent::TuiAction(TuiAppAction::TerminalKey {
            index: target.workspace_index,
            slot: target.slot,
            key,
        }));
    }

    /// 把 bracketed paste 写入表单或当前 Agent。
    ///
    /// 适用场景：终端粘贴。示例：表单打开时粘贴路径不会误发给 Agent。
    fn handle_paste(&mut self, text: String, app: &GsdvGuiApp) {
        if let Some(TuiLayer::Form(form)) = self.layer.as_mut()
            && let Some(FormField::Text { value, .. }) = form.fields.get_mut(form.active)
        {
            value.push_str(&text.replace(['\r', '\n'], ""));
            self.dirty = true;
            return;
        }
        if self.layer.is_some() {
            return;
        }
        if self.markdown_workbenches.paste(&text) {
            self.dirty = true;
            return;
        }
        if matches!(
            self.main_view,
            TuiMainView::Workflow | TuiMainView::Directory
        ) {
            return;
        }
        if self.main_view == TuiMainView::Terminal {
            self.queue_workspace_terminal_bytes(app, text.into_bytes());
            return;
        }
        if self.main_view == TuiMainView::Helix {
            self.queue_helix_terminal_bytes(app, text.into_bytes());
            return;
        }
        self.queue_terminal_bytes(app, text.into_bytes());
        app.queue_app_event(AppEvent::PomodoroInputDetected);
    }

    /// 处理鼠标选择、右键菜单和滚轮。
    ///
    /// 适用场景：EnableMouseCapture 后。示例：右键 tab -> Agent menu。
    fn handle_mouse(&mut self, event: MouseEvent, app: &GsdvGuiApp) {
        if event.kind == MouseEventKind::Moved {
            if self.layer.is_none()
                && matches!(self.main_view, TuiMainView::Agents | TuiMainView::Workflow)
            {
                self.update_agent_output_hover(event.column, event.row);
            } else if self.output_hover.take().is_some() {
                self.dirty = true;
            }
        }
        if matches!(
            self.layer,
            Some(TuiLayer::Form(_) | TuiLayer::Confirm { .. } | TuiLayer::MarkdownUnsaved { .. })
        ) {
            self.handle_overlay_mouse(event, app);
            return;
        }
        if matches!(self.layer, Some(TuiLayer::Gallery)) {
            if let MouseEventKind::Down(MouseButton::Left) = event.kind
                && let Some(action) = self.hit_regions.iter().rev().find_map(|region| {
                    region
                        .rect
                        .contains(event.column, event.row)
                        .then(|| region.left.clone())
                        .flatten()
                })
            {
                self.apply_hit_action(action, event.column, event.row, app);
            }
            return;
        }
        if matches!(self.layer, Some(TuiLayer::Pomodoro)) {
            self.handle_pomodoro_mouse(event, app);
            return;
        }
        if matches!(self.layer, Some(TuiLayer::Rest)) {
            return;
        }
        if let Some(TuiLayer::Menu(menu)) = self.layer.as_mut() {
            match event.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    let visible = menu_visible_range(menu, self.size.1);
                    if event.column >= menu.x
                        && event.row > menu.y
                        && usize::from(event.row - menu.y - 1) < visible.len()
                    {
                        menu.selected = visible.start + usize::from(event.row - menu.y - 1);
                        self.activate_selected_menu(app);
                    } else {
                        self.layer = None;
                        self.dirty = true;
                    }
                }
                MouseEventKind::ScrollUp => self.move_menu_selection(false),
                MouseEventKind::ScrollDown => self.move_menu_selection(true),
                _ => {}
            }
            return;
        }
        if matches!(
            self.main_view,
            TuiMainView::Workflow | TuiMainView::Directory
        ) {
            self.handle_column_view_mouse(event, app);
            return;
        }
        if self.main_view == TuiMainView::Terminal {
            self.handle_workspace_terminal_mouse(event, app);
            return;
        }
        if self.main_view == TuiMainView::Helix {
            self.handle_helix_terminal_mouse(event, app);
            return;
        }
        if matches!(event.kind, MouseEventKind::Moved) {
            let hovered = self
                .splitter_regions
                .iter()
                .rev()
                .find(|region| region.rect.contains(event.column, event.row))
                .map(|region| region.target.clone());
            let hover_position = hovered.as_ref().map(|_| (event.column, event.row));
            if self.splitter_hover != hovered || self.splitter_hover_position != hover_position {
                self.splitter_hover = hovered;
                self.splitter_hover_position = hover_position;
                self.dirty = true;
            }
        }
        if matches!(event.kind, MouseEventKind::Drag(MouseButton::Left))
            && self.splitter_drag.is_some()
        {
            self.handle_splitter_drag(event, app);
            return;
        }
        if matches!(event.kind, MouseEventKind::Up(MouseButton::Left))
            && self.splitter_drag.take().is_some()
        {
            self.dirty = true;
            return;
        }
        if event.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some(region) = self
                .splitter_regions
                .iter()
                .rev()
                .find(|region| region.rect.contains(event.column, event.row))
                .cloned()
        {
            let last_position = match region.target {
                SplitterTarget::Columns { .. } => event.column,
                SplitterTarget::Rows { .. } => event.row,
            };
            self.splitter_drag = Some(SplitterDrag {
                target: region.target,
                last_position,
            });
            self.dirty = true;
            return;
        }
        match event.kind {
            MouseEventKind::Down(button) => {
                if button == MouseButton::Left
                    && self.open_agent_output_at(event.column, event.row, app)
                {
                    return;
                }
                let action = self.hit_regions.iter().rev().find_map(|region| {
                    region
                        .rect
                        .contains(event.column, event.row)
                        .then(|| match button {
                            MouseButton::Left => region.left.clone(),
                            MouseButton::Right => region.right.clone(),
                            _ => None,
                        })?
                });
                if let Some(action) = action {
                    self.apply_hit_action(action, event.column, event.row, app);
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                if event.column < sidebar_width(self.size.0, self.sidebar_collapsed) {
                    if matches!(event.kind, MouseEventKind::ScrollUp) {
                        self.sidebar_scroll = self.sidebar_scroll.saturating_sub(1);
                    } else {
                        self.sidebar_scroll = self.sidebar_scroll.saturating_add(1);
                    }
                    self.dirty = true;
                } else if let Some(pane) = self
                    .visible_panes
                    .iter()
                    .find(|pane| pane_body_rect(pane).contains(event.column, event.row))
                    .cloned()
                {
                    let body = pane_body_rect(&pane);
                    let delta = if matches!(event.kind, MouseEventKind::ScrollUp) {
                        3
                    } else {
                        -3
                    };
                    let terminal_owns_scroll = self
                        .panes
                        .get(&pane.target)
                        .is_some_and(|cache| cache.frame.terminal_scroll);
                    if terminal_owns_scroll {
                        app.queue_app_event(AppEvent::TuiAction(TuiAppAction::ScrollAgent {
                            index: pane.target.workspace_index,
                            slot: pane.target.slot.clone(),
                            delta,
                            pointer_cell: Some((
                                event.column.saturating_sub(body.x),
                                event.row.saturating_sub(body.y),
                            )),
                        }));
                    } else if let Some(cache) = self.panes.get_mut(&pane.target) {
                        cache.scroll_offset = if delta > 0 {
                            cache.scroll_offset.saturating_add(delta as usize)
                        } else {
                            cache
                                .scroll_offset
                                .saturating_sub(delta.unsigned_abs() as usize)
                        }
                        .min(cache.frame.max_scroll_offset);
                        cache.frame = cache.source.view(cache.scroll_offset);
                        self.dirty = true;
                    }
                }
            }
            _ => {}
        }
    }

    /// 处理 Workflow tree 选择、菜单与分区滚轮。
    ///
    /// 适用场景：主 tree、task、step tree 和 step Markdown 分别滚动。
    fn handle_column_view_mouse(&mut self, event: MouseEvent, app: &GsdvGuiApp) {
        if let MouseEventKind::Down(button) = event.kind {
            if button == MouseButton::Left
                && self.open_agent_output_at(event.column, event.row, app)
            {
                return;
            }
            let action = self.hit_regions.iter().rev().find_map(|region| {
                region
                    .rect
                    .contains(event.column, event.row)
                    .then(|| match button {
                        MouseButton::Left => region.left.clone(),
                        MouseButton::Right => region.right.clone(),
                        _ => None,
                    })?
            });
            if let Some(action) = action {
                self.apply_hit_action(action, event.column, event.row, app);
            }
            return;
        }
        let direction = match event.kind {
            MouseEventKind::ScrollUp => -3_i32,
            MouseEventKind::ScrollDown => 3_i32,
            _ => return,
        };
        if let Some(pane) = self
            .visible_panes
            .iter()
            .find(|pane| pane_body_rect(pane).contains(event.column, event.row))
        {
            let body = pane_body_rect(pane);
            let delta = if matches!(event.kind, MouseEventKind::ScrollUp) {
                3
            } else {
                -3
            };
            let terminal_owns_scroll = self
                .panes
                .get(&pane.target)
                .is_some_and(|cache| cache.frame.terminal_scroll);
            if terminal_owns_scroll {
                app.queue_app_event(AppEvent::TuiAction(TuiAppAction::ScrollAgent {
                    index: pane.target.workspace_index,
                    slot: pane.target.slot.clone(),
                    delta,
                    pointer_cell: Some((
                        event.column.saturating_sub(body.x),
                        event.row.saturating_sub(body.y),
                    )),
                }));
            } else if let Some(cache) = self.panes.get_mut(&pane.target) {
                cache.scroll_offset = if delta > 0 {
                    cache.scroll_offset.saturating_add(delta as usize)
                } else {
                    cache
                        .scroll_offset
                        .saturating_sub(delta.unsigned_abs() as usize)
                }
                .min(cache.frame.max_scroll_offset);
                cache.frame = cache.source.view(cache.scroll_offset);
                self.dirty = true;
            }
            return;
        }
        let markdown_key = self.hit_regions.iter().rev().find_map(|region| {
            if !region.rect.contains(event.column, event.row) {
                return None;
            }
            match region.left.as_ref() {
                Some(HitAction::MarkdownPlaceCursor { key, .. }) => Some(key.clone()),
                _ => None,
            }
        });
        if let Some(key) = markdown_key
            && self.markdown_workbenches.scroll(&key, direction)
        {
            self.dirty = true;
            return;
        }
        let apply_scroll = |scroll: &mut usize| {
            *scroll = if direction.is_negative() {
                scroll.saturating_sub(direction.unsigned_abs() as usize)
            } else {
                scroll.saturating_add(direction as usize)
            };
        };
        if self.main_view == TuiMainView::Directory {
            apply_scroll(&mut self.directory_browser.content_scroll);
        } else if event.row < self.size.1.saturating_sub(1) {
            apply_scroll(&mut self.workflow_browser.content_scroll);
            apply_scroll(&mut self.workflow_browser.task_scroll);
            apply_scroll(&mut self.workflow_browser.step_content_scroll);
        }
        self.dirty = true;
    }

    /// 处理 workspace terminal 的退出按钮和滚轮。
    ///
    /// 适用场景：shell scrollback 或鼠标模式程序。示例：滚轮向上 -> ScrollWorkspace(+3)。
    fn handle_workspace_terminal_mouse(&mut self, event: MouseEvent, app: &GsdvGuiApp) {
        if let MouseEventKind::Down(button) = event.kind {
            let action = self.hit_regions.iter().rev().find_map(|region| {
                region
                    .rect
                    .contains(event.column, event.row)
                    .then(|| match button {
                        MouseButton::Left => region.left.clone(),
                        MouseButton::Right => region.right.clone(),
                        _ => None,
                    })?
            });
            if let Some(action) = action {
                self.apply_hit_action(action, event.column, event.row, app);
            }
            return;
        }
        let delta = match event.kind {
            MouseEventKind::ScrollUp => 3,
            MouseEventKind::ScrollDown => -3,
            _ => return,
        };
        let terminal_owns_scroll = self
            .workspace_terminal
            .as_ref()
            .is_some_and(|cache| cache.frame.terminal_scroll);
        if terminal_owns_scroll {
            app.queue_app_event(AppEvent::TuiAction(TuiAppAction::ScrollWorkspace {
                index: app.active_workspace,
                delta,
                pointer_cell: Some((event.column, event.row)),
            }));
        } else if let Some(cache) = self.workspace_terminal.as_mut() {
            cache.scroll_offset = if delta > 0 {
                cache.scroll_offset.saturating_add(delta as usize)
            } else {
                cache
                    .scroll_offset
                    .saturating_sub(delta.unsigned_abs() as usize)
            }
            .min(cache.frame.max_scroll_offset);
            cache.frame = cache.source.view(cache.scroll_offset);
            self.dirty = true;
        }
    }

    /// 处理 Helix 的底栏按钮、鼠标动作和滚轮。
    ///
    /// 适用场景：Helix 鼠标模式。示例：左键点击第 8 行 -> 光标定位到第 8 行。
    fn handle_helix_terminal_mouse(&mut self, event: MouseEvent, app: &GsdvGuiApp) {
        if let MouseEventKind::Down(button) = event.kind {
            let action = self.hit_regions.iter().rev().find_map(|region| {
                region
                    .rect
                    .contains(event.column, event.row)
                    .then(|| match button {
                        MouseButton::Left => region.left.clone(),
                        MouseButton::Right => region.right.clone(),
                        _ => None,
                    })?
            });
            if let Some(action) = action {
                self.apply_hit_action(action, event.column, event.row, app);
                self.helix_primary_mouse_down = false;
                return;
            }
            if button == MouseButton::Left && event.row < self.size.1.saturating_sub(1) {
                self.helix_primary_mouse_down = true;
                app.queue_app_event(AppEvent::TuiAction(TuiAppAction::HelixMouse {
                    index: app.active_workspace,
                    kind: TerminalTuiMouseKind::Press,
                    column: event.column,
                    row: event.row,
                }));
            }
            return;
        }
        let mouse_kind = match event.kind {
            MouseEventKind::Drag(MouseButton::Left) if self.helix_primary_mouse_down => {
                Some(TerminalTuiMouseKind::Drag)
            }
            MouseEventKind::Up(MouseButton::Left) if self.helix_primary_mouse_down => {
                self.helix_primary_mouse_down = false;
                Some(TerminalTuiMouseKind::Release)
            }
            _ => None,
        };
        if let Some(kind) = mouse_kind {
            // 触发条件：按下始于 Helix，但拖动或松开落到底栏。
            // 不能丢弃松开事件：Helix 会继续认为主键处于按下状态。
            // 防止回归：底栏仍由外层处理，同时 PTY 鼠标状态不会卡住。
            let row = event.row.min(self.size.1.saturating_sub(2));
            app.queue_app_event(AppEvent::TuiAction(TuiAppAction::HelixMouse {
                index: app.active_workspace,
                kind,
                column: event.column,
                row,
            }));
            return;
        }
        let delta = match event.kind {
            MouseEventKind::ScrollUp => 3,
            MouseEventKind::ScrollDown => -3,
            _ => return,
        };
        let terminal_owns_scroll = self
            .helix_terminal
            .as_ref()
            .is_some_and(|cache| cache.frame.terminal_scroll);
        if terminal_owns_scroll {
            app.queue_app_event(AppEvent::TuiAction(TuiAppAction::ScrollHelix {
                index: app.active_workspace,
                delta,
                pointer_cell: Some((event.column, event.row)),
            }));
        } else if let Some(cache) = self.helix_terminal.as_mut() {
            cache.scroll_offset = if delta > 0 {
                cache.scroll_offset.saturating_add(delta as usize)
            } else {
                cache
                    .scroll_offset
                    .saturating_sub(delta.unsigned_abs() as usize)
            }
            .min(cache.frame.max_scroll_offset);
            cache.frame = cache.source.view(cache.scroll_offset);
            self.dirty = true;
        }
    }

    /// 解析 Agent pane 字符坐标并投递输出点击动作。
    ///
    /// 适用场景：左键点击 URL 或路径。示例：`README.md:8` -> 后台分类。
    fn open_agent_output_at(&self, column: u16, row: u16, app: &GsdvGuiApp) -> bool {
        let Some((target, _, matched)) = self.agent_output_match_at(column, row) else {
            return false;
        };
        app.queue_app_event(AppEvent::TuiAction(TuiAppAction::OpenTerminalOutput {
            index: target.workspace_index,
            slot: target.slot,
            click: matched.click,
        }));
        true
    }

    /// 更新 Agent 输出 hover token；无命中时立即移除下划线。
    ///
    /// 适用场景：Mouse Moved。示例：路径 -> underline，空白 -> None。
    fn update_agent_output_hover(&mut self, column: u16, row: u16) {
        let next = self
            .agent_output_match_at(column, row)
            .map(|(target, row, matched)| AgentOutputHover {
                target,
                row,
                start_column: matched.start_column,
                end_column: matched.end_column,
            });
        if next != self.output_hover {
            self.output_hover = next;
            self.dirty = true;
        }
    }

    /// 返回屏幕字符位置对应的 Agent 输出目标及 token 范围。
    ///
    /// 适用场景：hover 与 click 共用。示例：pane 路径行 -> target + match。
    fn agent_output_match_at(
        &self,
        column: u16,
        row: u16,
    ) -> Option<(AgentTarget, usize, TerminalTuiOutputMatch)> {
        let Some(pane) = self
            .visible_panes
            .iter()
            .find(|pane| pane_body_rect(pane).contains(column, row))
        else {
            return None;
        };
        let body = pane_body_rect(pane);
        let Some(line) = self.previous_frame.get(usize::from(row)) else {
            return None;
        };
        let local_column = usize::from(column.saturating_sub(body.x));
        let local_row = usize::from(row.saturating_sub(body.y));
        let Some((_, ansi_row)) = line.overlays.iter().rev().find(|(x, _)| *x == body.x) else {
            return None;
        };
        let Some(cache) = self.panes.get(&pane.target) else {
            return None;
        };
        let matched = cache.source.output_match_in_row(ansi_row, local_column)?;
        Some((pane.target.clone(), local_row, matched))
    }

    /// 把一次 splitter 鼠标拖动转换成布局权重 AppEvent。
    ///
    /// 适用场景：左键保持期间的 Drag 事件。示例：column x 从 80 到 83 -> delta=3。
    fn handle_splitter_drag(&mut self, event: MouseEvent, app: &GsdvGuiApp) {
        let Some(drag) = self.splitter_drag.as_mut() else {
            return;
        };
        let position = match drag.target {
            SplitterTarget::Columns { .. } => event.column,
            SplitterTarget::Rows { .. } => event.row,
        };
        let delta = i32::from(position) - i32::from(drag.last_position);
        drag.last_position = position;
        let delta = i16::try_from(delta).unwrap_or(if delta.is_negative() {
            i16::MIN
        } else {
            i16::MAX
        });
        if delta == 0 {
            return;
        }
        let action = match drag.target.clone() {
            SplitterTarget::Columns {
                workspace_index,
                row_index,
                left_index,
                right_index,
                content_width,
            } => TuiAppAction::ResizeColumns {
                index: workspace_index,
                row_index,
                left_index,
                right_index,
                delta,
                content_width,
            },
            SplitterTarget::Rows {
                workspace_index,
                top_index,
                bottom_index,
                content_height,
            } => TuiAppAction::ResizeRows {
                index: workspace_index,
                top_index,
                bottom_index,
                delta,
                content_height,
            },
        };
        app.queue_app_event(AppEvent::TuiAction(action));
        self.dirty = true;
    }

    /// 处理表单字段、提交/取消与确认框鼠标点击。
    ///
    /// 适用场景：modal 已打开。示例：点击 Choice 字段 -> 选中并切到下一项。
    fn handle_overlay_mouse(&mut self, event: MouseEvent, app: &GsdvGuiApp) {
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let Some(mut layer) = self.layer.take() else {
            return;
        };
        let mut keep = true;
        let mut markdown_decision = None;
        match &mut layer {
            TuiLayer::Confirm { message, action } => {
                let width = (display_width(message) + 6).clamp(32, 70) as u16;
                let x = self.size.0.saturating_sub(width) / 2;
                let y = self.size.1.saturating_sub(5) / 2;
                if event.row == y + 3 && event.column >= x + 2 && event.column < x + 13 {
                    app.queue_app_event(AppEvent::TuiAction(action.clone()));
                    keep = false;
                } else if event.row == y + 3 && event.column >= x + 14 {
                    keep = false;
                }
            }
            TuiLayer::Form(form) => {
                let height = u16::try_from(form.fields.len())
                    .unwrap_or(1)
                    .saturating_add(6);
                let width = 72_u16.min(self.size.0.saturating_sub(4)).max(36);
                let x = self.size.0.saturating_sub(width) / 2;
                let y = self.size.1.saturating_sub(height) / 2;
                let field_start = y + 2;
                let field_end = field_start + form.fields.len() as u16;
                if event.row >= field_start && event.row < field_end {
                    form.active = usize::from(event.row - field_start);
                    adjust_form_choice(form, true);
                } else if event.row == y + height - 2 && event.column >= x + 2 {
                    if event.column < x + 13 {
                        match submit_form(form, app) {
                            Ok(action) => {
                                app.queue_app_event(AppEvent::TuiAction(action));
                                keep = false;
                            }
                            Err(error) => form.error = Some(error),
                        }
                    } else if event.column < x + 24 {
                        keep = false;
                    }
                }
            }
            TuiLayer::MarkdownUnsaved { pending, x, y } => {
                let width = 52_u16.min(self.size.0.saturating_sub(4));
                let dialog_x = self.size.0.saturating_sub(width) / 2;
                let dialog_y = self.size.1.saturating_sub(6) / 2;
                if event.row == dialog_y + 4 {
                    if event.column >= dialog_x + 2 && event.column < dialog_x + 8 {
                        markdown_decision = Some((true, (**pending).clone(), *x, *y));
                        keep = false;
                    } else if event.column >= dialog_x + 10 && event.column < dialog_x + 19 {
                        markdown_decision = Some((false, (**pending).clone(), *x, *y));
                        keep = false;
                    } else if event.column >= dialog_x + 21 {
                        keep = false;
                    }
                }
            }
            _ => keep = false,
        }
        if keep {
            self.layer = Some(layer);
        }
        if let Some((save, pending, x, y)) = markdown_decision {
            if save {
                self.save_dirty_workbenches(app);
                self.pending_markdown_navigation = Some(PendingMarkdownNavigation {
                    action: pending,
                    x,
                    y,
                });
            } else {
                self.discard_dirty_workbenches();
                self.apply_hit_action(pending, x, y, app);
            }
        }
        self.dirty = true;
    }

    /// 同步选择 Workflow 目标，并通过唯一事件队列更新共享 UI 状态。
    ///
    /// 适用场景：鼠标、键盘和右键菜单选择节点。示例：点击 task -> 共享状态打开 task。
    fn select_workflow_target(&mut self, app: &GsdvGuiApp, target: WorkflowSelectionTarget) {
        self.workflow_browser.selected = Some(target.clone());
        self.workflow_browser.content_scroll = 0;
        self.workflow_browser.task_scroll = 0;
        self.workflow_browser.step_content_scroll = 0;
        app.queue_app_event(AppEvent::TuiAction(TuiAppAction::OpenWorkflowTarget {
            index: app.active_workspace,
            target,
        }));
        self.dirty = true;
    }

    /// 执行点击区域动作或打开对应菜单。
    ///
    /// 适用场景：鼠标 Down。示例：左键 pane -> 聚焦输入目标。
    fn apply_hit_action(&mut self, action: HitAction, x: u16, y: u16, app: &GsdvGuiApp) {
        if matches!(
            action,
            HitAction::SelectWorkflow(_)
                | HitAction::SelectDirectory { .. }
                | HitAction::OpenDirectoryMarkdown(_)
                | HitAction::ExitAuxiliaryView
                | HitAction::OpenWorkflow
                | HitAction::OpenDirectory
                | HitAction::OpenWorkspaceTerminal
                | HitAction::OpenWorkspaceHelix
        ) && self
            .markdown_workbenches
            .items
            .values()
            .any(MarkdownWorkbench::dirty)
        {
            self.layer = Some(TuiLayer::MarkdownUnsaved {
                pending: Box::new(action),
                x,
                y,
            });
            self.dirty = true;
            return;
        }
        match action {
            HitAction::SelectWorkspace(index) => {
                app.queue_app_event(AppEvent::TuiAction(TuiAppAction::SelectWorkspace(index)));
                self.dirty = true;
            }
            HitAction::SelectAgent {
                target,
                row_index,
                column_index,
            } => {
                self.focused_target = Some(target.clone());
                let action = if self.spotlight_mode {
                    TuiAppAction::SelectSpotlightAgent {
                        workspace_index: target.workspace_index,
                        slot: target.slot,
                    }
                } else {
                    TuiAppAction::SelectAgent {
                        workspace_index: target.workspace_index,
                        row_index,
                        column_index,
                        slot: target.slot,
                    }
                };
                app.queue_app_event(AppEvent::TuiAction(action));
                self.dirty = true;
            }
            HitAction::WorkspaceMenu(index) => self.open_workspace_menu(x, y, app, index),
            HitAction::AgentMenu(context) => self.open_agent_menu(x, y, app, context),
            HitAction::PaneMenu(context) => self.open_pane_menu(x, y, app, context),
            HitAction::ExpandAgentRow {
                workspace_index,
                row_index,
            } => {
                app.queue_app_event(AppEvent::TuiAction(TuiAppAction::SetRowCollapsed {
                    index: workspace_index,
                    row_index,
                    collapsed: false,
                }));
                self.dirty = true;
            }
            HitAction::ExpandAgentColumn {
                workspace_index,
                row_index,
                column_index,
            } => {
                app.queue_app_event(AppEvent::TuiAction(TuiAppAction::SetColumnCollapsed {
                    index: workspace_index,
                    row_index,
                    column_index,
                    collapsed: false,
                }));
                self.dirty = true;
            }
            HitAction::ToggleSpotlight => {
                self.spotlight_mode = !self.spotlight_mode;
                self.spotlight_rank_leader_agent_id = None;
                self.focused_target = None;
                self.dirty = true;
            }
            HitAction::ToggleSpotlightRankMode => {
                app.queue_app_event(AppEvent::TuiAction(TuiAppAction::SetSpotlightRankMode(
                    !app.spotlight_rank_mode,
                )));
                self.spotlight_rank_leader_agent_id = None;
                self.dirty = true;
            }
            HitAction::ToggleSidebar => {
                self.sidebar_collapsed = !self.sidebar_collapsed;
                self.sidebar_scroll = 0;
                self.dirty = true;
            }
            HitAction::NewWorkspace => {
                self.layer = Some(TuiLayer::Form(workspace_path_form(false, None)));
                self.dirty = true;
            }
            HitAction::PomodoroMenu => self.open_pomodoro_menu(x, y, app),
            HitAction::OpenSettings => {
                self.layer = Some(TuiLayer::Form(network_settings_form(app)));
                self.dirty = true;
            }
            HitAction::OpenWorkflow => {
                self.main_view = TuiMainView::Workflow;
                self.workflow_browser.workspace_index = None;
                self.dirty = true;
            }
            HitAction::OpenDirectory => {
                self.main_view = TuiMainView::Directory;
                self.directory_browser.workspace_index = None;
                self.dirty = true;
            }
            HitAction::OpenWorkspaceTerminal => {
                self.main_view = TuiMainView::Terminal;
                self.dirty = true;
            }
            HitAction::OpenWorkspaceHelix => {
                self.main_view = TuiMainView::Helix;
                app.queue_app_event(AppEvent::TuiAction(TuiAppAction::OpenWorkspaceHelix {
                    index: app.active_workspace,
                }));
                self.dirty = true;
            }
            HitAction::ExitAuxiliaryView => {
                self.main_view = TuiMainView::Agents;
                self.focused_target = None;
                self.dirty = true;
            }
            HitAction::SelectWorkflow(target) => {
                self.select_workflow_target(app, target);
            }
            HitAction::ToggleWorkflowProject(key) => {
                if !self.workflow_browser.collapsed_projects.remove(&key) {
                    self.workflow_browser.collapsed_projects.insert(key);
                }
                self.dirty = true;
            }
            HitAction::WorkflowMenu(context) => self.open_workflow_menu(x, y, context, app),
            HitAction::SelectDirectory { depth, key } => {
                self.directory_browser.selected_dirs.truncate(depth);
                self.directory_browser.selected_dirs.push(key.clone());
                self.directory_browser.selected_file = None;
                self.directory_browser.content_scroll = 0;
                app.queue_app_event(AppEvent::TuiAction(TuiAppAction::ExpandOutlineDirectory {
                    index: app.active_workspace,
                    key,
                }));
                self.dirty = true;
            }
            HitAction::OpenDirectoryMarkdown(path) => {
                self.directory_browser.selected_file = Some(path.clone());
                self.directory_browser.content_scroll = 0;
                app.queue_app_event(AppEvent::TuiAction(TuiAppAction::OpenMarkdown {
                    index: app.active_workspace,
                    path,
                }));
                self.dirty = true;
            }
            HitAction::MarkdownPreview(key) => {
                if let Some(item) = self.markdown_workbenches.items.get_mut(&key) {
                    item.mode = MarkdownWorkbenchMode::Preview;
                    item.scroll = 0;
                }
                self.markdown_workbenches.focused = None;
                self.dirty = true;
            }
            HitAction::MarkdownEdit(key) => {
                let target = self
                    .markdown_workbenches
                    .items
                    .get(&key)
                    .filter(|item| item.editable)
                    .map(|item| item.target.clone());
                if let Some(item) = self.markdown_workbenches.items.get_mut(&key)
                    && item.editable
                {
                    item.mode = MarkdownWorkbenchMode::Edit;
                    item.cursor = item.cursor.min(item.text.len());
                    item.follow_cursor = true;
                    self.markdown_workbenches.focused = Some(key.clone());
                }
                match target {
                    Some(MarkdownWorkbenchTarget::Document(path)) => {
                        app.queue_app_event(AppEvent::TuiAction(TuiAppAction::OpenMarkdown {
                            index: app.active_workspace,
                            path,
                        }));
                    }
                    Some(MarkdownWorkbenchTarget::Workflow(target)) => {
                        app.queue_app_event(AppEvent::TuiAction(
                            TuiAppAction::OpenWorkflowTarget {
                                index: app.active_workspace,
                                target,
                            },
                        ));
                    }
                    None => {}
                }
                self.dirty = true;
            }
            HitAction::MarkdownPlaceCursor {
                key,
                origin_x,
                origin_y,
            } => {
                if self.markdown_workbenches.place_cursor(
                    &key,
                    x.saturating_sub(origin_x),
                    y.saturating_sub(origin_y),
                ) {
                    self.dirty = true;
                }
            }
            HitAction::MarkdownSave(key) => {
                let request = self.markdown_workbenches.items.get(&key).and_then(|item| {
                    (item.editable && item.dirty())
                        .then(|| (item.target.clone(), item.text.clone()))
                });
                if let Some((target, text)) = request {
                    let action = match target {
                        MarkdownWorkbenchTarget::Document(path) => TuiAppAction::SaveMarkdown {
                            index: app.active_workspace,
                            path,
                            text,
                        },
                        MarkdownWorkbenchTarget::Workflow(target) => {
                            TuiAppAction::SaveWorkflowMarkdown {
                                index: app.active_workspace,
                                target,
                                text,
                            }
                        }
                    };
                    if let Some(item) = self.markdown_workbenches.items.get_mut(&key) {
                        item.saving = true;
                    }
                    app.queue_app_event(AppEvent::TuiAction(action));
                }
                self.dirty = true;
            }
            HitAction::MarkdownDiscard(key) => {
                if let Some(item) = self.markdown_workbenches.items.get_mut(&key) {
                    item.text = item.saved_text.clone();
                    item.cursor = item.text.len();
                    item.scroll = 0;
                    item.saving = false;
                }
                self.dirty = true;
            }
            HitAction::MarkdownExit(key) => {
                if let Some(item) = self.markdown_workbenches.items.get_mut(&key) {
                    item.mode = MarkdownWorkbenchMode::Preview;
                }
                if self.markdown_workbenches.focused.as_deref() == Some(&key) {
                    self.markdown_workbenches.focused = None;
                }
                self.dirty = true;
            }
            HitAction::GalleryPrevious => {
                app.queue_app_event(AppEvent::TuiAction(TuiAppAction::MoveImageGallery(-1)));
                self.dirty = true;
            }
            HitAction::GalleryNext => {
                app.queue_app_event(AppEvent::TuiAction(TuiAppAction::MoveImageGallery(1)));
                self.dirty = true;
            }
            HitAction::GalleryClose => {
                app.queue_app_event(AppEvent::TuiAction(TuiAppAction::CloseImageGallery));
                self.dirty = true;
            }
        }
    }

    /// 打开 Workflow 节点菜单。
    ///
    /// 适用场景：右键 Repo/Project/Task/Step。示例：Task -> Add step/Delete task。
    fn open_workflow_menu(
        &mut self,
        x: u16,
        y: u16,
        context: WorkflowMenuContext,
        app: &GsdvGuiApp,
    ) {
        let mut items = vec![
            MenuItem {
                label: "Open".to_string(),
                action: Some(MenuAction::SelectWorkflow(context.target.clone())),
            },
            MenuItem {
                label: "Copy path".to_string(),
                action: Some(MenuAction::CopyText(context.copy_path)),
            },
        ];
        let index = app.active_workspace;
        let tree = app
            .workflow_states
            .get(index)
            .and_then(|state| state.tree.as_ref());
        if let Some(tree) = tree {
            if let Some(binding) =
                crate::gui::workflow::workflow_agent_binding(tree, &context.target)
            {
                let existing = app.workspaces.get(index).and_then(|workspace| {
                    workspace
                        .subagents
                        .iter()
                        .find(|agent| agent.workflow_target.as_ref() == Some(&binding.target))
                });
                items.push(menu_separator());
                if existing.is_some() {
                    items.push(menu_queue(
                        "Agent",
                        TuiAppAction::OpenWorkflowAgent {
                            index,
                            target: context.target.clone(),
                        },
                    ));
                    items.push(menu_queue(
                        "Destroy Agent",
                        TuiAppAction::DestroyWorkflowAgent {
                            index,
                            target: binding.target,
                        },
                    ));
                } else {
                    let kind = app
                        .workspaces
                        .get(index)
                        .map(|workspace| workspace.agent_kind)
                        .unwrap_or(app.default_agent_kind);
                    items.push(MenuItem {
                        label: "Agent...".to_string(),
                        action: Some(MenuAction::Form(workflow_agent_form(index, binding, kind))),
                    });
                }
            }
            match &context.target {
                WorkflowSelectionTarget::WorkspaceRoot { root_path } => {
                    if let Some(owner) =
                        workflow_trees(tree).find(|candidate| &candidate.root_path == root_path)
                    {
                        items.push(menu_separator());
                        items.push(MenuItem {
                            label: "Add project...".to_string(),
                            action: Some(MenuAction::Form(workflow_add_project_form(
                                index,
                                owner.spec_path.clone(),
                            ))),
                        });
                    }
                }
                WorkflowSelectionTarget::Project { root_path } => {
                    let owner = workflow_trees(tree).find_map(|candidate| {
                        candidate
                            .projects
                            .iter()
                            .find(|project| &project.root_path == root_path)
                            .map(|project| (candidate, project))
                    });
                    if let Some((owner, project)) = owner {
                        let project_path = owner.spec_path.join("ps").join(&project.key);
                        items.push(menu_separator());
                        items.push(MenuItem {
                            label: "Add task...".to_string(),
                            action: Some(MenuAction::Form(workflow_add_task_form(
                                index,
                                owner.spec_path.clone(),
                                project.key.clone(),
                            ))),
                        });
                        items.push(menu_confirm(
                            "Delete project",
                            &format!("Delete workflow project '{}' and all tasks?", project.label),
                            TuiAppAction::WorkflowMutation {
                                index,
                                request: WorkflowMutationRequest::DeleteProject { project_path },
                            },
                        ));
                    }
                }
                WorkflowSelectionTarget::Task { task_path } => {
                    if let Some(task) = workflow_tasks(tree).find(|task| &task.path == task_path) {
                        items.push(menu_separator());
                        items.push(MenuItem {
                            label: "Add step...".to_string(),
                            action: Some(MenuAction::Form(workflow_add_step_form(
                                index,
                                task.path.clone(),
                            ))),
                        });
                        items.push(menu_confirm(
                            "Delete task",
                            &format!("Delete workflow task '{}'?", task.label),
                            TuiAppAction::WorkflowMutation {
                                index,
                                request: WorkflowMutationRequest::DeleteTask {
                                    task_path: task.path.clone(),
                                },
                            },
                        ));
                    }
                }
                WorkflowSelectionTarget::Step {
                    task_path,
                    step_path,
                } => {
                    items.push(menu_separator());
                    items.push(menu_confirm(
                        "Delete step",
                        "Delete this workflow step and its children?",
                        TuiAppAction::WorkflowMutation {
                            index,
                            request: WorkflowMutationRequest::DeleteStep {
                                task_path: task_path.clone(),
                                step_path: step_path.clone(),
                            },
                        },
                    ));
                }
            }
        }
        self.open_menu(x, y, items);
    }

    /// 打开 workspace 右键菜单。
    ///
    /// 适用场景：rail 行右键。示例：本地 workspace -> Change directory。
    fn open_workspace_menu(&mut self, x: u16, y: u16, app: &GsdvGuiApp, index: usize) {
        let Some(workspace) = app.workspaces.get(index) else {
            return;
        };
        let mut items = vec![
            menu_queue("Open", TuiAppAction::SelectWorkspace(index)),
            menu_queue(
                if workspace.focused {
                    "Unfocus workspace"
                } else {
                    "Focus workspace"
                },
                TuiAppAction::ToggleWorkspaceFocus(index),
            ),
            menu_queue(
                "Expand all Agent panes",
                TuiAppAction::ExpandAllAgents { index },
            ),
        ];
        items.push(MenuItem {
            label: "Change directory...".to_string(),
            action: Some(MenuAction::Form(workspace_path_form(true, Some(index)))),
        });
        items.push(MenuItem {
            label: "Close workspace".to_string(),
            action: Some(MenuAction::Confirm {
                message: format!("Close workspace '{}'?", workspace.name),
                action: TuiAppAction::CloseWorkspace(index),
            }),
        });
        self.open_menu(x, y, items);
    }

    /// 打开 Agent tab 的完整常用动作菜单。
    ///
    /// 适用场景：pane tab 或专注列表右键。示例：Stop、等级、模型、目录。
    fn open_agent_menu(&mut self, x: u16, y: u16, app: &GsdvGuiApp, context: AgentMenuContext) {
        let target = &context.target;
        let workflow_target = match &target.slot {
            AgentSlotId::Subagent(id) => app
                .workspaces
                .get(target.workspace_index)
                .and_then(|workspace| workspace.subagents.iter().find(|agent| agent.id == *id))
                .and_then(|agent| agent.workflow_target.clone()),
            AgentSlotId::Main => None,
        };
        let stopped = app.agent_slot_manually_stopped(target.workspace_index, &target.slot);
        let mut items = vec![menu_queue(
            if stopped { "Start agent" } else { "Stop agent" },
            if stopped {
                TuiAppAction::StartAgent {
                    index: target.workspace_index,
                    slot: target.slot.clone(),
                }
            } else {
                TuiAppAction::StopAgent {
                    index: target.workspace_index,
                    slot: target.slot.clone(),
                }
            },
        )];
        if workflow_target.is_none() {
            let spotlight_level = app.spotlight_agent_level(target.workspace_index, &target.slot);
            let mut spotlight_items = Vec::new();
            if spotlight_level.is_some() {
                spotlight_items.push(menu_queue(
                    "Remove registration",
                    TuiAppAction::SetSpotlight {
                        index: target.workspace_index,
                        slot: target.slot.clone(),
                        level: None,
                    },
                ));
                spotlight_items.push(menu_separator());
            }
            for level in 1..=3 {
                spotlight_items.push(menu_queue(
                    &format!(
                        "{}Level {level}",
                        if spotlight_level == Some(level) {
                            "● "
                        } else {
                            ""
                        }
                    ),
                    TuiAppAction::SetSpotlight {
                        index: target.workspace_index,
                        slot: target.slot.clone(),
                        level: Some(level),
                    },
                ));
            }
            items.push(menu_submenu("Spotlight registration", spotlight_items));
        }
        items.push(menu_submenu(
            "Restart agent",
            vec![
                menu_confirm(
                    "Resume session",
                    "Restart this Agent and resume its session?",
                    TuiAppAction::RestartAgent {
                        index: target.workspace_index,
                        slot: target.slot.clone(),
                        resume: true,
                    },
                ),
                menu_confirm(
                    "New session",
                    "Restart this Agent with a new session?",
                    TuiAppAction::RestartAgent {
                        index: target.workspace_index,
                        slot: target.slot.clone(),
                        resume: false,
                    },
                ),
            ],
        ));
        items.push(menu_separator());

        let current_kind = app.agent_slot_kind(target.workspace_index, &target.slot);
        let mut switch_items = Vec::new();
        for kind in AgentKind::all()
            .into_iter()
            .filter(|kind| Some(*kind) != current_kind)
        {
            switch_items.push(menu_confirm(
                kind.title(),
                "Switching Agent type starts a new session.",
                TuiAppAction::SwitchAgentKind {
                    index: target.workspace_index,
                    slot: target.slot.clone(),
                    kind,
                },
            ));
        }
        items.push(agent_text_menu(
            "Set model...",
            target,
            AgentTextField::Model,
            app,
        ));
        if current_kind.is_some_and(AgentKind::supports_model_provider) {
            items.push(agent_text_menu(
                "Set model provider...",
                target,
                AgentTextField::ModelProvider,
                app,
            ));
        }
        items.push(agent_text_menu(
            "Set work-dir...",
            target,
            AgentTextField::WorkDir,
            app,
        ));
        items.push(agent_text_menu(
            "Set Code work-dir...",
            target,
            AgentTextField::HelixWorkDir,
            app,
        ));
        if let Some(kind) = current_kind {
            let current_effort = app.agent_slot_effort(target.workspace_index, &target.slot);
            let mut effort_items = vec![menu_queue(
                if current_effort.is_none() {
                    "● Default"
                } else {
                    "Default"
                },
                TuiAppAction::SetAgentEffort {
                    index: target.workspace_index,
                    slot: target.slot.clone(),
                    value: None,
                },
            )];
            for effort in kind.effort_levels() {
                effort_items.push(menu_queue(
                    &format!(
                        "{}{effort}",
                        if current_effort.as_deref() == Some(*effort) {
                            "● "
                        } else {
                            ""
                        }
                    ),
                    TuiAppAction::SetAgentEffort {
                        index: target.workspace_index,
                        slot: target.slot.clone(),
                        value: Some((*effort).to_string()),
                    },
                ));
            }
            items.push(menu_submenu("Effort", effort_items));
            if kind.supports_fast_mode() {
                let current_fast = app.agent_slot_fast_mode(target.workspace_index, &target.slot);
                let fast_items = [("Default", None), ("On", Some(true)), ("Off", Some(false))]
                    .into_iter()
                    .map(|(label, value)| {
                        menu_queue(
                            &format!("{}{label}", if current_fast == value { "● " } else { "" }),
                            TuiAppAction::SetAgentFastMode {
                                index: target.workspace_index,
                                slot: target.slot.clone(),
                                value,
                            },
                        )
                    })
                    .collect();
                items.push(menu_submenu("Fast mode", fast_items));
            }
        }
        if !switch_items.is_empty() {
            items.push(menu_submenu("Switch agent", switch_items));
        }
        if let AgentSlotId::Subagent(id) = &target.slot {
            items.push(agent_text_menu(
                "Rename subagent...",
                target,
                AgentTextField::Name,
                app,
            ));
            if let Some(workflow_target) = workflow_target.clone() {
                items.push(menu_confirm(
                    "Destroy Agent",
                    "Destroy this Workflow Agent association?",
                    TuiAppAction::DestroyWorkflowAgent {
                        index: target.workspace_index,
                        target: workflow_target,
                    },
                ));
            } else {
                items.push(menu_confirm(
                    "Close subagent",
                    "Close and delete this subagent?",
                    TuiAppAction::RemoveSubagent {
                        index: target.workspace_index,
                        id: id.clone(),
                    },
                ));
            }
        }
        if workflow_target.is_none()
            && let (Some(row_index), Some(column_index), Some(column_id)) = (
                context.row_index,
                context.column_index,
                context.column_id.as_deref(),
            )
        {
            items.push(menu_separator());
            let mut layout_items = Self::agent_layout_structure_items(
                app,
                target.workspace_index,
                row_index,
                column_index,
                column_id,
            );
            if let AgentSlotId::Subagent(id) = &target.slot {
                let move_items = Self::subagent_layout_move_items(app, &context, id);
                if !move_items.is_empty() {
                    layout_items.insert(3, menu_submenu("Move agent", move_items));
                }
            }
            items.push(menu_submenu("Layout", layout_items));
        }
        self.open_menu(x, y, items);
    }

    /// 构造与 GUI 对齐的 Agent row/column 结构菜单。
    ///
    /// 适用场景：Agent tab 和 pane 空白处右键。示例：两列 -> 折叠其他列、等分列。
    fn agent_layout_structure_items(
        app: &GsdvGuiApp,
        workspace_index: usize,
        row_index: usize,
        column_index: usize,
        column_id: &str,
    ) -> Vec<MenuItem> {
        let mut items = vec![
            MenuItem {
                label: "Add agent tab...".to_string(),
                action: Some(MenuAction::Form(add_agent_form(
                    workspace_index,
                    AddAgentPlacement::ExistingColumn(column_id.to_string()),
                    app.default_agent_kind,
                ))),
            },
            MenuItem {
                label: "Add row + agent...".to_string(),
                action: Some(MenuAction::Form(add_agent_form(
                    workspace_index,
                    AddAgentPlacement::NewRow(row_index),
                    app.default_agent_kind,
                ))),
            },
            MenuItem {
                label: "Add column + agent...".to_string(),
                action: Some(MenuAction::Form(add_agent_form(
                    workspace_index,
                    AddAgentPlacement::NewColumn(row_index),
                    app.default_agent_kind,
                ))),
            },
            menu_separator(),
            menu_queue(
                "Collapse column",
                TuiAppAction::SetColumnCollapsed {
                    index: workspace_index,
                    row_index,
                    column_index,
                    collapsed: true,
                },
            ),
        ];
        let Some(workspace) = app.workspaces.get(workspace_index) else {
            return items;
        };
        let column_count = workspace
            .agent_rows
            .get(row_index)
            .map(|row| row.columns.len())
            .unwrap_or_default();
        items.push(menu_queue(
            "Collapse other columns",
            TuiAppAction::CollapseOtherColumns {
                index: workspace_index,
                row_index,
                column_index,
            },
        ));
        items.push(menu_queue(
            "Collapse row",
            TuiAppAction::SetRowCollapsed {
                index: workspace_index,
                row_index,
                collapsed: true,
            },
        ));
        items.push(menu_queue(
            "Collapse other rows",
            TuiAppAction::CollapseOtherRows {
                index: workspace_index,
                row_index,
            },
        ));
        if column_count > 1 {
            items.push(menu_queue(
                "Equalize columns",
                TuiAppAction::EqualizeColumns {
                    index: workspace_index,
                    row_index,
                },
            ));
        }
        if workspace.agent_rows.len() > 1 {
            items.push(menu_queue(
                "Equalize rows",
                TuiAppAction::EqualizeRows {
                    index: workspace_index,
                },
            ));
        }
        if row_index > 0 || column_index > 0 {
            items.push(menu_separator());
        }
        if row_index > 0 || column_index > 0 {
            items.push(menu_confirm(
                "Close column",
                "Close this column and all subagents inside it?",
                TuiAppAction::RemoveColumn {
                    index: workspace_index,
                    column_id: column_id.to_string(),
                },
            ));
        }
        if row_index > 0 {
            items.push(menu_confirm(
                "Close row",
                "Close this row and all subagents inside it?",
                TuiAppAction::RemoveRow {
                    index: workspace_index,
                    row_index,
                },
            ));
        }
        items
    }

    /// 构造 subagent 在现有 Agent grid 中可执行的移动菜单。
    ///
    /// 适用场景：subagent tab 的 Layout 子菜单。示例：中间 tab -> 左右和首尾均可移动。
    fn subagent_layout_move_items(
        app: &GsdvGuiApp,
        context: &AgentMenuContext,
        id: &str,
    ) -> Vec<MenuItem> {
        let (Some(row_index), Some(column_index), Some(column_id)) = (
            context.row_index,
            context.column_index,
            context.column_id.as_deref(),
        ) else {
            return Vec::new();
        };
        let Some(workspace) = app.workspaces.get(context.target.workspace_index) else {
            return Vec::new();
        };
        let Some(column) = workspace
            .agent_rows
            .get(row_index)
            .and_then(|row| row.columns.get(column_index))
        else {
            return Vec::new();
        };
        let slot = data::AgentColumnSlot::Subagent(id.to_string());
        let Some(tab_index) = column.tabs.iter().position(|tab| tab == &slot) else {
            return Vec::new();
        };
        let index = context.target.workspace_index;
        let first_movable = usize::from(row_index == 0 && column_index == 0);
        let mut items = Vec::new();
        if tab_index > first_movable {
            items.push(menu_queue(
                "Move tab left",
                TuiAppAction::MoveSubagentLeft {
                    index,
                    column_id: column_id.to_string(),
                    id: id.to_string(),
                },
            ));
            items.push(menu_queue(
                "Move tab to head",
                TuiAppAction::MoveSubagentToHead {
                    index,
                    column_id: column_id.to_string(),
                    id: id.to_string(),
                },
            ));
        }
        if tab_index + 1 < column.tabs.len() {
            items.push(menu_queue(
                "Move tab right",
                TuiAppAction::MoveSubagentRight {
                    index,
                    column_id: column_id.to_string(),
                    id: id.to_string(),
                },
            ));
            items.push(menu_queue(
                "Move tab to tail",
                TuiAppAction::MoveSubagentToTail {
                    index,
                    column_id: column_id.to_string(),
                    id: id.to_string(),
                },
            ));
        }
        let column_count = workspace
            .agent_rows
            .get(row_index)
            .map(|row| row.columns.len())
            .unwrap_or_default();
        if column_index > 0 {
            items.push(menu_queue(
                "Move to left column",
                TuiAppAction::MoveSubagentToAdjacentColumn {
                    index,
                    row_index,
                    column_index,
                    id: id.to_string(),
                    direction: -1,
                },
            ));
        }
        if column_index + 1 < column_count {
            items.push(menu_queue(
                "Move to right column",
                TuiAppAction::MoveSubagentToAdjacentColumn {
                    index,
                    row_index,
                    column_index,
                    id: id.to_string(),
                    direction: 1,
                },
            ));
        }
        if row_index > 0 {
            items.push(menu_queue(
                "Move to upper row",
                TuiAppAction::MoveSubagentToAdjacentRow {
                    index,
                    row_index,
                    column_index,
                    id: id.to_string(),
                    direction: -1,
                },
            ));
        }
        if row_index + 1 < workspace.agent_rows.len() {
            items.push(menu_queue(
                "Move to lower row",
                TuiAppAction::MoveSubagentToAdjacentRow {
                    index,
                    row_index,
                    column_index,
                    id: id.to_string(),
                    direction: 1,
                },
            ));
        }
        let workspace_items = app
            .workspaces
            .iter()
            .enumerate()
            .filter(|(target_index, _)| *target_index != index)
            .map(|(target_index, target)| {
                menu_queue(
                    &target.name,
                    TuiAppAction::MoveSubagentToWorkspace {
                        index,
                        id: id.to_string(),
                        target_index,
                    },
                )
            })
            .collect::<Vec<_>>();
        if !workspace_items.is_empty() {
            items.push(menu_submenu("Move to workspace", workspace_items));
        }
        items
    }

    /// 打开 row/column 结构菜单。
    ///
    /// 适用场景：pane header 空白处右键。示例：新增 row/col、等分和关闭。
    fn open_pane_menu(&mut self, x: u16, y: u16, app: &GsdvGuiApp, context: PaneMenuContext) {
        let items = Self::agent_layout_structure_items(
            app,
            context.workspace_index,
            context.row_index,
            context.column_index,
            &context.column_id,
        );
        self.open_menu(x, y, items);
    }

    /// 打开与 Alt 相同的番茄控制层。
    ///
    /// 适用场景：点击底栏番茄状态。示例：Work 24:00 -> 状态与操作面板。
    fn open_pomodoro_menu(&mut self, _x: u16, _y: u16, _app: &GsdvGuiApp) {
        self.layer = Some(TuiLayer::Pomodoro);
        self.dirty = true;
    }

    /// 安装一个右键菜单并选中首个可执行项。
    ///
    /// 适用场景：所有 context menu。示例：超出右边界 -> 左移。
    fn open_menu(&mut self, x: u16, y: u16, items: Vec<MenuItem>) {
        let selected = items
            .iter()
            .position(|item| item.action.is_some())
            .unwrap_or(0);
        self.layer = Some(TuiLayer::Menu(ContextMenu {
            x,
            y,
            items,
            selected,
            scroll: 0,
            parent: None,
        }));
        self.dirty = true;
    }

    /// 处理右键菜单键盘输入。
    ///
    /// 适用场景：Up/Down/Left/Right/Enter/Esc。示例：Right -> 打开二级菜单。
    fn handle_menu_key(&mut self, key: KeyEvent, app: &GsdvGuiApp) {
        match key.code {
            KeyCode::Esc | KeyCode::Left => self.close_menu_level(),
            KeyCode::Up => self.move_menu_selection(false),
            KeyCode::Down => self.move_menu_selection(true),
            KeyCode::Right | KeyCode::Enter => self.activate_selected_menu(app),
            _ => {}
        }
    }

    /// 关闭当前菜单层级，二级菜单优先返回父菜单。
    ///
    /// 适用场景：Esc 或 Left。示例：子菜单 -> 根菜单；根菜单 -> 关闭。
    fn close_menu_level(&mut self) {
        self.layer = match self.layer.take() {
            Some(TuiLayer::Menu(mut menu)) => {
                menu.parent.take().map(|parent| TuiLayer::Menu(*parent))
            }
            _ => None,
        };
        self.dirty = true;
    }

    /// 跳到上一个或下一个可执行菜单项。
    ///
    /// 适用场景：菜单键盘和滚轮。示例：分隔项自动跳过。
    fn move_menu_selection(&mut self, forward: bool) {
        let Some(TuiLayer::Menu(menu)) = self.layer.as_mut() else {
            return;
        };
        if menu.items.is_empty() {
            return;
        }
        for _ in 0..menu.items.len() {
            menu.selected = if forward {
                (menu.selected + 1) % menu.items.len()
            } else {
                (menu.selected + menu.items.len() - 1) % menu.items.len()
            };
            if menu.items[menu.selected].action.is_some() {
                break;
            }
        }
        let max_visible = usize::from(self.size.1.saturating_sub(4).max(1));
        if menu.selected < menu.scroll {
            menu.scroll = menu.selected;
        } else if menu.selected >= menu.scroll.saturating_add(max_visible) {
            menu.scroll = menu.selected + 1 - max_visible;
        }
        self.dirty = true;
    }

    /// 激活选中菜单项。
    ///
    /// 适用场景：Enter 或鼠标点击。示例：Queue -> AppEvent::TuiAction。
    fn activate_selected_menu(&mut self, app: &GsdvGuiApp) {
        let Some(TuiLayer::Menu(menu)) = self.layer.take() else {
            return;
        };
        let action = menu
            .items
            .get(menu.selected)
            .and_then(|item| item.action.clone());
        match action {
            Some(MenuAction::Queue(action)) => {
                app.queue_app_event(AppEvent::TuiAction(action));
            }
            Some(MenuAction::Form(form)) => self.layer = Some(TuiLayer::Form(form)),
            Some(MenuAction::Confirm { message, action }) => {
                self.layer = Some(TuiLayer::Confirm { message, action });
            }
            Some(MenuAction::Submenu(items)) => {
                let child_width = menu_width_from_items(&items);
                let parent_width = menu_width_from_items(&menu.items);
                let selected_row = menu.y.saturating_add(1).saturating_add(
                    u16::try_from(menu.selected.saturating_sub(menu.scroll)).unwrap_or(u16::MAX),
                );
                let right = menu.x.saturating_add(parent_width.saturating_sub(1));
                let child_x = if right.saturating_add(child_width) <= self.size.0 {
                    right
                } else {
                    menu.x.saturating_sub(child_width.saturating_sub(1))
                };
                let selected = items
                    .iter()
                    .position(|item| item.action.is_some())
                    .unwrap_or(0);
                self.layer = Some(TuiLayer::Menu(ContextMenu {
                    x: child_x,
                    y: selected_row,
                    items,
                    selected,
                    scroll: 0,
                    parent: Some(Box::new(menu)),
                }));
            }
            Some(MenuAction::SelectWorkflow(target)) => {
                self.select_workflow_target(app, target);
            }
            Some(MenuAction::CopyText(text)) => {
                self.pending_clipboard_text = Some(text);
            }
            None => self.layer = Some(TuiLayer::Menu(menu)),
        }
        self.dirty = true;
    }

    /// 处理 modal 表单或确认框键盘输入。
    ///
    /// 适用场景：浮层独占输入。示例：Esc 取消，Tab 切字段，Enter 提交。
    fn handle_overlay_key(&mut self, key: KeyEvent, app: &GsdvGuiApp) {
        let Some(mut layer) = self.layer.take() else {
            return;
        };
        let mut markdown_decision = None;
        match &mut layer {
            TuiLayer::Confirm { action, .. } => match key.code {
                KeyCode::Esc | KeyCode::Char('n') => {}
                KeyCode::Enter | KeyCode::Char('y') => {
                    app.queue_app_event(AppEvent::TuiAction(action.clone()));
                }
                _ => {
                    self.layer = Some(layer);
                }
            },
            TuiLayer::Form(form) => {
                let mut keep = true;
                match key.code {
                    KeyCode::Esc => keep = false,
                    KeyCode::Tab => {
                        if !form.fields.is_empty() {
                            form.active = (form.active + 1) % form.fields.len();
                        }
                    }
                    KeyCode::BackTab => {
                        if !form.fields.is_empty() {
                            form.active = (form.active + form.fields.len() - 1) % form.fields.len();
                        }
                    }
                    KeyCode::Up | KeyCode::Left => adjust_form_choice(form, false),
                    KeyCode::Down | KeyCode::Right => adjust_form_choice(form, true),
                    KeyCode::Backspace => {
                        if let Some(FormField::Text { value, .. }) =
                            form.fields.get_mut(form.active)
                        {
                            value.pop();
                        }
                    }
                    KeyCode::Enter => match submit_form(form, app) {
                        Ok(action) => {
                            app.queue_app_event(AppEvent::TuiAction(action));
                            keep = false;
                        }
                        Err(error) => form.error = Some(error),
                    },
                    KeyCode::Char(ch)
                        if !key.modifiers.intersects(
                            KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                        ) =>
                    {
                        if let Some(FormField::Text { value, .. }) =
                            form.fields.get_mut(form.active)
                        {
                            value.push(ch);
                        }
                    }
                    _ => {}
                }
                if keep {
                    self.layer = Some(layer);
                }
            }
            TuiLayer::MarkdownUnsaved { pending, x, y } => match key.code {
                KeyCode::Char('s' | 'S') => {
                    markdown_decision = Some((true, (**pending).clone(), *x, *y));
                }
                KeyCode::Char('d' | 'D') => {
                    markdown_decision = Some((false, (**pending).clone(), *x, *y));
                }
                KeyCode::Esc | KeyCode::Char('c' | 'C') => {}
                _ => self.layer = Some(layer),
            },
            _ => self.layer = Some(layer),
        }
        if let Some((save, pending, x, y)) = markdown_decision {
            if save {
                self.save_dirty_workbenches(app);
                self.pending_markdown_navigation = Some(PendingMarkdownNavigation {
                    action: pending,
                    x,
                    y,
                });
            } else {
                self.discard_dirty_workbenches();
                self.apply_hit_action(pending, x, y, app);
            }
        }
        self.dirty = true;
    }

    /// 构建并差量写出完整字符界面。
    ///
    /// 适用场景：dirty 且到达 FPS 时间片。示例：只写变化行。
    fn draw(&mut self, stdout: &mut Stdout, app: &GsdvGuiApp) -> Result<()> {
        self.hit_regions.clear();
        self.splitter_regions.clear();
        self.cursor_position = None;
        let mut frame = vec![RenderLine::default(); usize::from(self.size.1)];
        if self.size.0 < TUI_MIN_WIDTH || self.size.1 < TUI_MIN_HEIGHT {
            set_base_text(
                &mut frame,
                0,
                0,
                &format!(
                    "Terminal too small: {}x{} (need at least {}x{})",
                    self.size.0, self.size.1, TUI_MIN_WIDTH, TUI_MIN_HEIGHT
                ),
                self.size.0,
            );
        } else {
            match self.main_view {
                TuiMainView::Agents => {
                    self.draw_chrome(&mut frame, app);
                    self.draw_panes(&mut frame, app);
                }
                TuiMainView::Workflow => {
                    column_browser::draw_workflow_columns(
                        &mut self.workflow_browser,
                        &mut self.markdown_workbenches,
                        &mut frame,
                        self.size,
                        app.workflow_states.get(app.active_workspace),
                        app.workspaces.get(app.active_workspace),
                        &mut self.hit_regions,
                        &app.local_images,
                    );
                    if !self.visible_panes.is_empty() {
                        self.draw_panes(&mut frame, app);
                    }
                    self.draw_auxiliary_bottom(&mut frame, app);
                }
                TuiMainView::Directory => {
                    column_browser::draw_directory_columns(
                        &mut self.directory_browser,
                        &mut self.markdown_workbenches,
                        &mut frame,
                        self.size,
                        app.active_workspace,
                        app.workspaces.get(app.active_workspace),
                        app.documents.get(app.active_workspace),
                        &mut self.hit_regions,
                        &app.local_images,
                    );
                    self.draw_auxiliary_bottom(&mut frame, app);
                }
                TuiMainView::Terminal => {
                    self.draw_workspace_terminal(&mut frame);
                    self.draw_auxiliary_bottom(&mut frame, app);
                }
                TuiMainView::Helix => {
                    self.draw_helix_terminal(&mut frame);
                    self.draw_auxiliary_bottom(&mut frame, app);
                }
            }
            self.draw_layer(&mut frame, app);
        }
        let cursor = self
            .layer
            .is_none()
            .then_some(self.cursor_position)
            .flatten();
        if let Some(text) = self.pending_clipboard_text.take() {
            queue!(stdout, Print(osc52_copy_sequence(&text)))?;
        }
        write_frame_diff(stdout, &self.previous_frame, &frame, self.size, cursor)?;
        self.previous_frame = frame;
        self.last_frame_at = Some(Instant::now());
        self.dirty = false;
        Ok(())
    }

    /// 绘制左侧列表、顶栏分隔和底部状态栏。
    ///
    /// 适用场景：每次实际 frame。示例：spotlight 列表按已有排序展示。
    fn draw_chrome(&mut self, frame: &mut [RenderLine], app: &GsdvGuiApp) {
        let (width, height) = self.size;
        let sidebar = sidebar_width(width, self.sidebar_collapsed);
        if !self.sidebar_collapsed && self.spotlight_mode {
            let exit_label = "[Exit]";
            let rank_label = if app.spotlight_rank_mode {
                "[x] Rank"
            } else {
                "[ ] Rank"
            };
            styled_overlay_text(frame, 0, 0, exit_label, (15, 23, 42), (250, 204, 21), true);
            styled_overlay_text(
                frame,
                0,
                7,
                rank_label,
                if app.spotlight_rank_mode {
                    (255, 255, 255)
                } else {
                    (148, 163, 184)
                },
                if app.spotlight_rank_mode {
                    (37, 99, 235)
                } else {
                    (11, 16, 22)
                },
                true,
            );
            self.hit_regions.push(HitRegion {
                rect: TuiRect {
                    x: 0,
                    y: 0,
                    width: 6,
                    height: 1,
                },
                left: Some(HitAction::ToggleSpotlight),
                right: None,
            });
            self.hit_regions.push(HitRegion {
                rect: TuiRect {
                    x: 7,
                    y: 0,
                    width: 8,
                    height: 1,
                },
                left: Some(HitAction::ToggleSpotlightRankMode),
                right: None,
            });
        }
        if sidebar > 0 {
            for y in 0..height.saturating_sub(1) {
                put_char(frame, y, sidebar, '│');
            }
        }

        let list_start = u16::from(self.spotlight_mode && !self.sidebar_collapsed);
        let available = usize::from(height.saturating_sub(1).saturating_sub(list_start));
        if self.sidebar_collapsed {
            self.sidebar_scroll = 0;
        } else if self.spotlight_mode {
            let entries = app.spotlight_agents();
            self.sidebar_scroll = self
                .sidebar_scroll
                .min(entries.len().saturating_sub(available.max(1)));
            for (screen_index, entry) in entries
                .iter()
                .skip(self.sidebar_scroll)
                .take(available)
                .enumerate()
            {
                let y = list_start + screen_index as u16;
                let target = AgentTarget {
                    workspace_index: entry.workspace_index,
                    slot: entry.slot.clone(),
                };
                let selected = self.focused_target.as_ref() == Some(&target);
                let status = status_marker(entry.activity, entry.manually_stopped, self.busy_frame);
                let prefix = if selected { "›" } else { " " };
                draw_navigation_row(
                    frame,
                    y,
                    sidebar,
                    &format!(
                        "{prefix}{status} {} · {}",
                        entry.title, entry.workspace_name
                    ),
                    entry.activity,
                    entry.manually_stopped,
                    selected,
                    self.busy_frame,
                );
                let context = AgentMenuContext {
                    target: target.clone(),
                    row_index: None,
                    column_index: None,
                    column_id: None,
                };
                self.hit_regions.push(HitRegion {
                    rect: TuiRect {
                        x: 0,
                        y,
                        width: sidebar,
                        height: 1,
                    },
                    left: Some(HitAction::SelectAgent {
                        target,
                        row_index: 0,
                        column_index: 0,
                    }),
                    right: Some(HitAction::AgentMenu(context)),
                });
            }
        } else {
            let indices = app
                .workspaces
                .iter()
                .enumerate()
                .filter_map(|(index, workspace)| {
                    (!app.focus_filter_enabled || workspace.focused).then_some(index)
                })
                .collect::<Vec<_>>();
            self.sidebar_scroll = self
                .sidebar_scroll
                .min(indices.len().saturating_sub(available.max(1)));
            for (screen_index, index) in indices
                .iter()
                .skip(self.sidebar_scroll)
                .take(available)
                .enumerate()
            {
                let workspace = &app.workspaces[*index];
                let y = list_start + screen_index as u16;
                let selected = *index == app.active_workspace;
                let prefix = if selected { "›" } else { " " };
                let activity = super::app_chrome::workspace_effective_activity(workspace);
                draw_navigation_row(
                    frame,
                    y,
                    sidebar,
                    &format!(
                        "{prefix}{} {}{}",
                        status_marker(activity, false, self.busy_frame),
                        workspace.name,
                        if workspace.focused { " ★" } else { "" }
                    ),
                    activity,
                    false,
                    selected,
                    self.busy_frame,
                );
                self.hit_regions.push(HitRegion {
                    rect: TuiRect {
                        x: 0,
                        y,
                        width: sidebar,
                        height: 1,
                    },
                    left: Some(HitAction::SelectWorkspace(*index)),
                    right: Some(HitAction::WorkspaceMenu(*index)),
                });
            }
        }

        let bottom = height.saturating_sub(1);
        let mut new_x = 0;
        let compact_bottom = width < 96;
        let sidebar_toggle = if self.sidebar_collapsed { "[>]" } else { "[<]" };
        styled_overlay_text(
            frame,
            bottom,
            new_x,
            sidebar_toggle,
            (34, 211, 238),
            (15, 23, 42),
            true,
        );
        self.hit_regions.push(HitRegion {
            rect: TuiRect {
                x: new_x,
                y: bottom,
                width: 3,
                height: 1,
            },
            left: Some(HitAction::ToggleSidebar),
            right: None,
        });
        new_x = 4;
        if !self.spotlight_mode {
            let focus_label = if compact_bottom { "[F2]" } else { "[F2 Focus]" };
            let focus_width = u16::try_from(focus_label.len()).unwrap_or(10);
            styled_overlay_text(
                frame,
                bottom,
                new_x,
                focus_label,
                (250, 204, 21),
                (11, 16, 22),
                true,
            );
            self.hit_regions.push(HitRegion {
                rect: TuiRect {
                    x: new_x,
                    y: bottom,
                    width: focus_width,
                    height: 1,
                },
                left: Some(HitAction::ToggleSpotlight),
                right: None,
            });
            new_x = new_x.saturating_add(focus_width).saturating_add(1);
        }
        let workspace_label = if compact_bottom {
            "[+]"
        } else {
            "[+ Workspace]"
        };
        let workspace_width = u16::try_from(workspace_label.len()).unwrap_or(13);
        set_base_text(frame, bottom, new_x, workspace_label, workspace_width);
        styled_overlay_text(
            frame,
            bottom,
            new_x,
            workspace_label,
            (74, 222, 128),
            (11, 16, 22),
            true,
        );
        self.hit_regions.push(HitRegion {
            rect: TuiRect {
                x: new_x,
                y: bottom,
                width: workspace_width,
                height: 1,
            },
            left: Some(HitAction::NewWorkspace),
            right: None,
        });
        new_x = new_x.saturating_add(workspace_width).saturating_add(1);
        let directory_label = if compact_bottom { "[D]" } else { "[Dir]" };
        let directory_width = u16::try_from(directory_label.len()).unwrap_or(5);
        styled_overlay_text(
            frame,
            bottom,
            new_x,
            directory_label,
            (244, 114, 182),
            (11, 16, 22),
            true,
        );
        self.hit_regions.push(HitRegion {
            rect: TuiRect {
                x: new_x,
                y: bottom,
                width: directory_width,
                height: 1,
            },
            left: Some(HitAction::OpenDirectory),
            right: None,
        });
        new_x = new_x.saturating_add(directory_width).saturating_add(1);
        let workflow_label = if compact_bottom { "[WF]" } else { "[Workflow]" };
        let workflow_width = u16::try_from(workflow_label.len()).unwrap_or(10);
        styled_overlay_text(
            frame,
            bottom,
            new_x,
            workflow_label,
            (96, 165, 250),
            (11, 16, 22),
            true,
        );
        self.hit_regions.push(HitRegion {
            rect: TuiRect {
                x: new_x,
                y: bottom,
                width: workflow_width,
                height: 1,
            },
            left: Some(HitAction::OpenWorkflow),
            right: None,
        });
        new_x = new_x.saturating_add(workflow_width).saturating_add(1);
        let terminal_label = if compact_bottom { "[T]" } else { "[Terminal]" };
        let terminal_width = u16::try_from(terminal_label.len()).unwrap_or(10);
        styled_overlay_text(
            frame,
            bottom,
            new_x,
            terminal_label,
            (45, 212, 191),
            (11, 16, 22),
            true,
        );
        self.hit_regions.push(HitRegion {
            rect: TuiRect {
                x: new_x,
                y: bottom,
                width: terminal_width,
                height: 1,
            },
            left: Some(HitAction::OpenWorkspaceTerminal),
            right: None,
        });
        new_x = new_x.saturating_add(terminal_width).saturating_add(1);
        let helix_label = if compact_bottom { "[H]" } else { "[Helix]" };
        let helix_width = u16::try_from(helix_label.len()).unwrap_or(7);
        styled_overlay_text(
            frame,
            bottom,
            new_x,
            helix_label,
            (192, 132, 252),
            (11, 16, 22),
            true,
        );
        self.hit_regions.push(HitRegion {
            rect: TuiRect {
                x: new_x,
                y: bottom,
                width: helix_width,
                height: 1,
            },
            left: Some(HitAction::OpenWorkspaceHelix),
            right: None,
        });
        new_x = new_x.saturating_add(helix_width).saturating_add(1);
        let settings_label = if compact_bottom { "[S]" } else { "[Settings]" };
        let settings_width = u16::try_from(settings_label.len()).unwrap_or(10);
        styled_overlay_text(
            frame,
            bottom,
            new_x,
            settings_label,
            (251, 146, 60),
            (11, 16, 22),
            true,
        );
        self.hit_regions.push(HitRegion {
            rect: TuiRect {
                x: new_x,
                y: bottom,
                width: settings_width,
                height: 1,
            },
            left: Some(HitAction::OpenSettings),
            right: None,
        });
        new_x = new_x.saturating_add(settings_width).saturating_add(1);
        let pomodoro = pomodoro_status(app);
        let pomodoro_width = display_width(&pomodoro).min(26) as u16;
        let pomodoro_x = width.saturating_sub(pomodoro_width.saturating_add(10));
        let pomodoro_warning = app.runtime_settings.pomodoro_enabled
            && app.pomodoro.phase == PomodoroPhase::Working
            && pomodoro_work_progress(&app.runtime_settings, &app.pomodoro)
                >= pomodoro_warning_progress(&app.runtime_settings);
        if new_x < pomodoro_x {
            set_base_text(
                frame,
                bottom,
                pomodoro_x,
                &" ".repeat(pomodoro_width as usize),
                pomodoro_width,
            );
            styled_overlay_text(
                frame,
                bottom,
                pomodoro_x,
                &pomodoro,
                if pomodoro_warning {
                    (250, 204, 21)
                } else if app.runtime_settings.pomodoro_enabled {
                    (45, 212, 191)
                } else {
                    (148, 163, 184)
                },
                (11, 16, 22),
                app.runtime_settings.pomodoro_enabled,
            );
            self.hit_regions.push(HitRegion {
                rect: TuiRect {
                    x: pomodoro_x,
                    y: bottom,
                    width: pomodoro_width,
                    height: 1,
                },
                left: Some(HitAction::PomodoroMenu),
                right: Some(HitAction::PomodoroMenu),
            });
        }
        set_base_text(frame, bottom, width.saturating_sub(9), "[Ctrl+Q]", 8);
    }

    /// 绘制 Workflow 与 Terminal 共用的无标题底栏。
    ///
    /// 适用场景：辅助视图返回或互相切换。示例：Workflow -> Terminal。
    fn draw_auxiliary_bottom(&mut self, frame: &mut [RenderLine], app: &GsdvGuiApp) {
        let bottom = self.size.1.saturating_sub(1);
        let compact_bottom = self.size.0 < 72;
        let buttons = [
            (
                "[Exit]",
                HitAction::ExitAuxiliaryView,
                (250, 204, 21),
                false,
            ),
            (
                if compact_bottom { "[D]" } else { "[Dir]" },
                HitAction::OpenDirectory,
                (244, 114, 182),
                self.main_view == TuiMainView::Directory,
            ),
            (
                if compact_bottom { "[WF]" } else { "[Workflow]" },
                HitAction::OpenWorkflow,
                (96, 165, 250),
                self.main_view == TuiMainView::Workflow,
            ),
            (
                if compact_bottom { "[T]" } else { "[Terminal]" },
                HitAction::OpenWorkspaceTerminal,
                (45, 212, 191),
                self.main_view == TuiMainView::Terminal,
            ),
            (
                if compact_bottom { "[H]" } else { "[Helix]" },
                HitAction::OpenWorkspaceHelix,
                (192, 132, 252),
                self.main_view == TuiMainView::Helix,
            ),
            (
                if compact_bottom { "[S]" } else { "[Settings]" },
                HitAction::OpenSettings,
                (251, 146, 60),
                false,
            ),
        ];
        let mut x = 0_u16;
        for (label, action, color, active) in buttons {
            let button_width = u16::try_from(label.len()).unwrap_or(10);
            styled_overlay_text(
                frame,
                bottom,
                x,
                label,
                if active { (255, 255, 255) } else { color },
                if active { (37, 99, 235) } else { (11, 16, 22) },
                true,
            );
            self.hit_regions.push(HitRegion {
                rect: TuiRect {
                    x,
                    y: bottom,
                    width: button_width,
                    height: 1,
                },
                left: Some(action),
                right: None,
            });
            x = x.saturating_add(button_width).saturating_add(1);
        }
        let workspace_name = app
            .workspaces
            .get(app.active_workspace)
            .map(|workspace| workspace.name.as_str())
            .unwrap_or("workspace");
        let available = self.size.0.saturating_sub(x).saturating_sub(10);
        styled_overlay_text(
            frame,
            bottom,
            x,
            &clip_text(workspace_name, available),
            (148, 163, 184),
            (11, 16, 22),
            false,
        );
        set_base_text(frame, bottom, self.size.0.saturating_sub(9), "[Ctrl+Q]", 8);
    }

    /// 绘制当前 workspace terminal 的完整字符视口。
    ///
    /// 适用场景：Terminal 一级视图，不添加额外标题行。示例：shell 占满底栏以上区域。
    fn draw_workspace_terminal(&mut self, frame: &mut [RenderLine]) {
        let rect = TuiRect {
            x: 0,
            y: 0,
            width: self.size.0,
            height: self.size.1.saturating_sub(1),
        };
        let Some(cache) = self.workspace_terminal.as_mut() else {
            set_base_text(frame, 0, 0, "Starting workspace terminal...", rect.width);
            return;
        };
        let scroll_offset = cache.scroll_offset.min(cache.frame.max_scroll_offset);
        if scroll_offset != cache.scroll_offset {
            cache.scroll_offset = scroll_offset;
            cache.frame = cache.source.view(scroll_offset);
        }
        draw_terminal_tui_frame(frame, rect, &cache.frame, None);
        self.cursor_position = terminal_tui_cursor_position(rect, &cache.frame);
    }

    /// 绘制当前 workspace Helix 的完整字符视口。
    ///
    /// 适用场景：Helix 一级视图。示例：编辑器占满底栏以上区域。
    fn draw_helix_terminal(&mut self, frame: &mut [RenderLine]) {
        let rect = TuiRect {
            x: 0,
            y: 0,
            width: self.size.0,
            height: self.size.1.saturating_sub(1),
        };
        let Some(cache) = self.helix_terminal.as_mut() else {
            set_base_text(frame, 0, 0, "Starting Helix...", rect.width);
            return;
        };
        let scroll_offset = cache.scroll_offset.min(cache.frame.max_scroll_offset);
        if scroll_offset != cache.scroll_offset {
            cache.scroll_offset = scroll_offset;
            cache.frame = cache.source.view(scroll_offset);
        }
        draw_terminal_tui_frame(frame, rect, &cache.frame, None);
        self.cursor_position = terminal_tui_cursor_position(rect, &cache.frame);
    }

    /// 绘制折叠 row/column 的固定恢复入口。
    ///
    /// 适用场景：Layout 菜单折叠布局项后仍可单独展开。示例：点击 `[v]` -> 展开对应 row。
    fn draw_layout_restore_regions(&mut self, frame: &mut [RenderLine]) {
        for region in self.layout_restore_regions.clone() {
            let label = match region.target {
                LayoutRestoreTarget::Row { .. } => "[v]",
                LayoutRestoreTarget::Column { .. } => "[>]",
            };
            for y in region.rect.y..region.rect.y.saturating_add(region.rect.height) {
                styled_overlay_text(
                    frame,
                    y,
                    region.rect.x,
                    &" ".repeat(usize::from(region.rect.width)),
                    (34, 211, 238),
                    (15, 23, 42),
                    false,
                );
            }
            styled_overlay_text(
                frame,
                region.rect.y,
                region.rect.x,
                &clip_text(label, region.rect.width),
                (34, 211, 238),
                (15, 23, 42),
                true,
            );
            let action = match region.target {
                LayoutRestoreTarget::Row {
                    workspace_index,
                    row_index,
                } => HitAction::ExpandAgentRow {
                    workspace_index,
                    row_index,
                },
                LayoutRestoreTarget::Column {
                    workspace_index,
                    row_index,
                    column_index,
                } => HitAction::ExpandAgentColumn {
                    workspace_index,
                    row_index,
                    column_index,
                },
            };
            self.hit_regions.push(HitRegion {
                rect: region.rect,
                left: Some(action),
                right: None,
            });
        }
    }

    /// 绘制所有可见 row/col pane、tab header 和 terminal 内容。
    ///
    /// 适用场景：普通多 Agent 或专注单 Agent。示例：两个 columns 同时显示。
    fn draw_panes(&mut self, frame: &mut [RenderLine], app: &GsdvGuiApp) {
        self.draw_layout_restore_regions(frame);
        let focused_color = self.focused_pane_border_color(app);
        let sidebar = sidebar_width(self.size.0, self.sidebar_collapsed);
        let content_x = sidebar.saturating_add(u16::from(sidebar > 0));
        for pane in self.visible_panes.clone() {
            if pane.rect.width == 0 || pane.rect.height == 0 {
                continue;
            }
            let focused = self.pane_is_focused(app, &pane);
            if focused && sidebar > 0 && pane.rect.x == content_x {
                for y in pane.rect.y..pane.rect.y.saturating_add(pane.rect.height) {
                    styled_overlay_text(frame, y, sidebar, "│", focused_color, (11, 16, 22), true);
                }
            }
            let header = pane.rect.y;
            let mut x = pane.rect.x;
            let tabs = if pane.tabs.is_empty() {
                vec![pane.target.slot.clone()]
            } else {
                pane.tabs.clone()
            };
            for slot in tabs {
                let target = AgentTarget {
                    workspace_index: pane.target.workspace_index,
                    slot: slot.clone(),
                };
                let label = agent_title(app, &target);
                let active = target == pane.target;
                let stopped = app.agent_slot_manually_stopped(target.workspace_index, &target.slot);
                let activity = app.agent_slot_activity(target.workspace_index, &target.slot);
                let star = app
                    .spotlight_agent_level(target.workspace_index, &target.slot)
                    .is_some()
                    .then_some("★")
                    .unwrap_or_default();
                let text = format!(
                    " {} {}{} ",
                    status_marker(activity, stopped, self.busy_frame),
                    label,
                    if star.is_empty() {
                        String::new()
                    } else {
                        format!(" {star}")
                    }
                );
                let tab_width = display_width(&text).min(24) as u16;
                if x.saturating_add(tab_width) > pane.rect.x.saturating_add(pane.rect.width) {
                    break;
                }
                draw_agent_tab(
                    frame,
                    header,
                    x,
                    tab_width,
                    &text,
                    activity,
                    stopped,
                    active,
                    self.busy_frame,
                );
                let context = AgentMenuContext {
                    target: target.clone(),
                    row_index: Some(pane.row_index),
                    column_index: Some(pane.column_index),
                    column_id: Some(pane.column_id.clone()),
                };
                self.hit_regions.push(HitRegion {
                    rect: TuiRect {
                        x,
                        y: header,
                        width: tab_width,
                        height: 1,
                    },
                    left: Some(HitAction::SelectAgent {
                        target,
                        row_index: pane.row_index,
                        column_index: pane.column_index,
                    }),
                    right: Some(HitAction::AgentMenu(context)),
                });
                x = x.saturating_add(tab_width);
            }
            let header_end = pane.rect.x.saturating_add(pane.rect.width);
            if x < header_end {
                self.hit_regions.push(HitRegion {
                    rect: TuiRect {
                        x,
                        y: header,
                        width: header_end.saturating_sub(x),
                        height: 1,
                    },
                    left: Some(HitAction::SelectAgent {
                        target: pane.target.clone(),
                        row_index: pane.row_index,
                        column_index: pane.column_index,
                    }),
                    right: Some(if pane.column_id == "workflow-agent" {
                        HitAction::AgentMenu(AgentMenuContext {
                            target: pane.target.clone(),
                            row_index: None,
                            column_index: None,
                            column_id: None,
                        })
                    } else {
                        HitAction::PaneMenu(PaneMenuContext {
                            workspace_index: pane.target.workspace_index,
                            row_index: pane.row_index,
                            column_index: pane.column_index,
                            column_id: pane.column_id.clone(),
                        })
                    }),
                });
            }
            let body = pane_body_rect(&pane);
            let underline = self
                .output_hover
                .as_ref()
                .filter(|hover| hover.target == pane.target)
                .map(|hover| (hover.row, hover.start_column, hover.end_column));
            self.hit_regions.push(HitRegion {
                rect: body,
                left: Some(HitAction::SelectAgent {
                    target: pane.target.clone(),
                    row_index: pane.row_index,
                    column_index: pane.column_index,
                }),
                right: Some(HitAction::AgentMenu(AgentMenuContext {
                    target: pane.target.clone(),
                    row_index: Some(pane.row_index),
                    column_index: Some(pane.column_index),
                    column_id: Some(pane.column_id.clone()),
                })),
            });
            if app.agent_slot_manually_stopped(pane.target.workspace_index, &pane.target.slot) {
                set_base_text(
                    frame,
                    body.y,
                    body.x,
                    "Agent stopped · right-click to start",
                    body.width,
                );
                continue;
            }
            if let Some(error) = app
                .terminal_hosts
                .get(pane.target.workspace_index)
                .and_then(|hosts| hosts.agents.get(&pane.target.slot))
                .and_then(|slot| slot.error.as_deref())
            {
                set_base_text(
                    frame,
                    body.y,
                    body.x,
                    &format!("Agent error: {error}"),
                    body.width,
                );
                continue;
            }
            let Some(cache) = self.panes.get_mut(&pane.target) else {
                set_base_text(frame, body.y, body.x, "Starting Agent...", body.width);
                continue;
            };
            let scroll_offset = cache.scroll_offset.min(cache.frame.max_scroll_offset);
            if scroll_offset != cache.scroll_offset {
                cache.scroll_offset = scroll_offset;
                cache.frame = cache.source.view(scroll_offset);
            }
            draw_terminal_tui_frame(frame, body, &cache.frame, underline);
            if focused {
                self.cursor_position = terminal_tui_cursor_position(body, &cache.frame);
            }
        }
        self.draw_splitters(frame, app);
    }

    /// 绘制 row/column splitter，并生成高优先级鼠标拖动命中区。
    ///
    /// 适用场景：普通多 pane 布局。示例：两列之间显示青色竖线并可左右拖动。
    fn draw_splitters(&mut self, frame: &mut [RenderLine], app: &GsdvGuiApp) {
        if self.spotlight_mode || self.visible_panes.len() < 2 {
            return;
        }
        let workspace_index = app.active_workspace;
        let focused_color = self.focused_pane_border_color(app);
        let mut rows = BTreeMap::<usize, Vec<PaneLayout>>::new();
        for pane in self.visible_panes.clone() {
            rows.entry(pane.row_index).or_default().push(pane);
        }

        for panes in rows.values_mut() {
            panes.sort_by_key(|pane| pane.rect.x);
            let content_width = panes.iter().map(|pane| pane.rect.width).sum::<u16>();
            for pair in panes.windows(2) {
                let left = &pair[0];
                let right = &pair[1];
                let x = left.rect.x.saturating_add(left.rect.width);
                if right.rect.x != x.saturating_add(TUI_SPLITTER_SIZE) {
                    continue;
                }
                let target = SplitterTarget::Columns {
                    workspace_index,
                    row_index: left.row_index,
                    left_index: left.column_index,
                    right_index: right.column_index,
                    content_width,
                };
                let highlighted = self.splitter_hover.as_ref() == Some(&target)
                    || self
                        .splitter_drag
                        .as_ref()
                        .is_some_and(|drag| drag.target == target);
                let focus_touches =
                    self.pane_is_focused(app, left) || self.pane_is_focused(app, right);
                for y in left.rect.y..left.rect.y.saturating_add(left.rect.height) {
                    let pointer = self.splitter_hover.as_ref() == Some(&target)
                        && self
                            .splitter_hover_position
                            .is_some_and(|(hover_x, hover_y)| hover_x == x && hover_y == y);
                    styled_overlay_text(
                        frame,
                        y,
                        x,
                        if pointer {
                            "↔"
                        } else if highlighted {
                            "┃"
                        } else {
                            "│"
                        },
                        if highlighted {
                            (147, 197, 253)
                        } else if focus_touches {
                            focused_color
                        } else {
                            (51, 65, 85)
                        },
                        if highlighted {
                            (23, 44, 74)
                        } else {
                            (11, 16, 22)
                        },
                        highlighted,
                    );
                }
                self.splitter_regions.push(SplitterRegion {
                    rect: TuiRect {
                        x,
                        y: left.rect.y,
                        width: TUI_SPLITTER_SIZE,
                        height: left.rect.height,
                    },
                    target,
                });
            }
        }

        let mut row_bounds = rows
            .iter()
            .filter_map(|(row_index, panes)| {
                let first = panes.first()?;
                Some((*row_index, first.rect.y, first.rect.height))
            })
            .collect::<Vec<_>>();
        row_bounds.sort_by_key(|(_, y, _)| *y);
        let content_height = row_bounds.iter().map(|(_, _, height)| *height).sum::<u16>();
        let body_x = self
            .visible_panes
            .iter()
            .map(|pane| pane.rect.x)
            .min()
            .unwrap_or_default();
        let body_right = self
            .visible_panes
            .iter()
            .map(|pane| pane.rect.x.saturating_add(pane.rect.width))
            .max()
            .unwrap_or(body_x);
        for pair in row_bounds.windows(2) {
            let (top_index, top_y, top_height) = pair[0];
            let (bottom_index, bottom_y, _) = pair[1];
            let y = top_y.saturating_add(top_height);
            if bottom_y != y.saturating_add(TUI_SPLITTER_SIZE) {
                continue;
            }
            let width = body_right.saturating_sub(body_x);
            let target = SplitterTarget::Rows {
                workspace_index,
                top_index,
                bottom_index,
                content_height,
            };
            let highlighted = self.splitter_hover.as_ref() == Some(&target)
                || self
                    .splitter_drag
                    .as_ref()
                    .is_some_and(|drag| drag.target == target);
            let top_panes = rows.get(&top_index).cloned().unwrap_or_default();
            let bottom_panes = rows.get(&bottom_index).cloned().unwrap_or_default();
            for x in body_x..body_right {
                let pointer = self.splitter_hover.as_ref() == Some(&target)
                    && self
                        .splitter_hover_position
                        .is_some_and(|(hover_x, hover_y)| hover_x == x && hover_y == y);
                let intersection =
                    top_panes
                        .windows(2)
                        .chain(bottom_panes.windows(2))
                        .any(|pair| {
                            pair[0].rect.x.saturating_add(pair[0].rect.width) == x
                                && pair[1].rect.x == x.saturating_add(TUI_SPLITTER_SIZE)
                        });
                // 横向共享边界必须按字符判断相邻 cell，避免不等宽 rows 把整条线误染成焦点色。
                let focus_touches = top_panes.iter().chain(&bottom_panes).any(|pane| {
                    self.pane_is_focused(app, pane)
                        && x >= pane.rect.x
                        && x < pane.rect.x.saturating_add(pane.rect.width)
                });
                styled_overlay_text(
                    frame,
                    y,
                    x,
                    if pointer {
                        "↕"
                    } else if highlighted {
                        "━"
                    } else if intersection {
                        "┼"
                    } else {
                        "─"
                    },
                    if highlighted {
                        (147, 197, 253)
                    } else if focus_touches {
                        focused_color
                    } else {
                        (51, 65, 85)
                    },
                    if highlighted {
                        (23, 44, 74)
                    } else {
                        (11, 16, 22)
                    },
                    highlighted || focus_touches,
                );
            }
            self.splitter_regions.push(SplitterRegion {
                rect: TuiRect {
                    x: body_x,
                    y,
                    width,
                    height: TUI_SPLITTER_SIZE,
                },
                target,
            });
        }
    }

    /// 绘制唯一覆盖层，避免菜单、表单和番茄状态互相叠加。
    ///
    /// 适用场景：每次 frame 的最后一步。示例：Rest 覆盖所有普通内容。
    fn draw_layer(&mut self, frame: &mut [RenderLine], app: &GsdvGuiApp) {
        let Some(layer) = self.layer.as_mut() else {
            return;
        };
        match layer {
            TuiLayer::Menu(menu) => {
                draw_context_menu_chain(frame, menu, self.size, true);
            }
            TuiLayer::Confirm { message, .. } => {
                let width = (display_width(message) + 6).clamp(32, 70) as u16;
                let x = self.size.0.saturating_sub(width) / 2;
                let y = self.size.1.saturating_sub(5) / 2;
                draw_box(frame, x, y, width, 5, "Confirm");
                overlay_text(frame, y + 2, x + 2, &clip_text(message, width - 4));
                overlay_text(frame, y + 3, x + 2, "[Confirm]    [Cancel]");
            }
            TuiLayer::Form(form) => {
                let height = u16::try_from(form.fields.len())
                    .unwrap_or(1)
                    .saturating_add(6);
                let width = 72_u16.min(self.size.0.saturating_sub(4)).max(36);
                let x = self.size.0.saturating_sub(width) / 2;
                let y = self.size.1.saturating_sub(height) / 2;
                draw_box(frame, x, y, width, height, &form.title);
                for (index, field) in form.fields.iter().enumerate() {
                    let marker = if index == form.active { "›" } else { " " };
                    let line = match field {
                        FormField::Text { label, value } => {
                            format!("{marker} {label:<18} {}", clip_text(value, width - 25))
                        }
                        FormField::Choice {
                            label,
                            options,
                            selected,
                        } => format!(
                            "{marker} {label:<18} < {} >",
                            options.get(*selected).map(String::as_str).unwrap_or("")
                        ),
                    };
                    overlay_text(
                        frame,
                        y + 2 + index as u16,
                        x + 2,
                        &clip_text(&line, width - 4),
                    );
                }
                if let Some(error) = form.error.as_deref() {
                    overlay_text(frame, y + height - 3, x + 2, &clip_text(error, width - 4));
                }
                overlay_text(
                    frame,
                    y + height - 2,
                    x + 2,
                    "[Submit]    [Cancel]   Tab: Next  ↑↓: Choose",
                );
            }
            TuiLayer::Pomodoro => draw_pomodoro_panel(frame, self.size, app),
            TuiLayer::Rest => draw_pomodoro_rest_screen(frame, self.size, app),
            TuiLayer::Gallery => self.draw_image_gallery(frame, app),
            TuiLayer::MarkdownUnsaved { .. } => {
                let width = 52_u16.min(self.size.0.saturating_sub(4));
                let x = self.size.0.saturating_sub(width) / 2;
                let y = self.size.1.saturating_sub(6) / 2;
                draw_box(frame, x, y, width, 6, "Unsaved Markdown");
                overlay_text(frame, y + 2, x + 2, "Save changes before switching?");
                overlay_text(frame, y + 4, x + 2, "[Save]  [Discard]  [Cancel]");
            }
        }
    }

    /// 绘制 TUI 顶层图片画廊。
    ///
    /// 适用场景：socket 连续推图。示例：保持当前图并显示新图计数。
    fn draw_image_gallery(&mut self, frame: &mut [RenderLine], app: &GsdvGuiApp) {
        let width = self.size.0.saturating_sub(4).max(24);
        let height = self.size.1.saturating_sub(4).max(10);
        let x = self.size.0.saturating_sub(width) / 2;
        let y = self.size.1.saturating_sub(height) / 2;
        draw_box(frame, x, y, width, height, "Image Gallery");
        let controls = format!(
            "[<]  {}/{}  [>]  [Close]{}",
            app.image_gallery.current.saturating_add(1),
            app.image_gallery.paths.len(),
            if app.image_gallery.unseen > 0 {
                format!("  +{} new", app.image_gallery.unseen)
            } else {
                String::new()
            }
        );
        overlay_text(frame, y + 1, x + 2, &clip_text(&controls, width - 4));
        self.hit_regions.push(HitRegion {
            rect: TuiRect {
                x: x + 2,
                y: y + 1,
                width: 3,
                height: 1,
            },
            left: Some(HitAction::GalleryPrevious),
            right: None,
        });
        self.hit_regions.push(HitRegion {
            rect: TuiRect {
                x: x + 12,
                y: y + 1,
                width: 3,
                height: 1,
            },
            left: Some(HitAction::GalleryNext),
            right: None,
        });
        self.hit_regions.push(HitRegion {
            rect: TuiRect {
                x: x + 17,
                y: y + 1,
                width: 7,
                height: 1,
            },
            left: Some(HitAction::GalleryClose),
            right: None,
        });
        let Some(path) = app.image_gallery.paths.get(app.image_gallery.current) else {
            return;
        };
        overlay_text(
            frame,
            y + 2,
            x + 2,
            &clip_text(&path.display().to_string(), width - 4),
        );
        if let Some(image) = app.local_images.ready(path) {
            markdown::draw_decoded_image(
                frame,
                TuiRect {
                    x: x + 2,
                    y: y + 3,
                    width: width - 4,
                    height: height - 5,
                },
                &image.decoded,
            );
        } else if let Some(error) = app.local_images.error(path) {
            overlay_text(frame, y + 4, x + 2, &clip_text(error, width - 4));
        }
    }

    /// 保存全部脏 Markdown 工作台并保留当前 Edit 模式。
    ///
    /// 适用场景：未保存确认选择 Save。示例：task+step 同时脏 -> 分别进入现有保存链。
    fn save_dirty_workbenches(&mut self, app: &GsdvGuiApp) {
        let requests = self
            .markdown_workbenches
            .items
            .values()
            .filter(|item| item.editable && item.dirty())
            .map(|item| (item.target.clone(), item.text.clone()))
            .collect::<Vec<_>>();
        for (target, text) in requests {
            let action = match target {
                MarkdownWorkbenchTarget::Document(path) => TuiAppAction::SaveMarkdown {
                    index: app.active_workspace,
                    path,
                    text,
                },
                MarkdownWorkbenchTarget::Workflow(target) => TuiAppAction::SaveWorkflowMarkdown {
                    index: app.active_workspace,
                    target,
                    text,
                },
            };
            app.queue_app_event(AppEvent::TuiAction(action));
        }
        for item in self.markdown_workbenches.items.values_mut() {
            if item.dirty() {
                item.saving = true;
                item.save_error = None;
            }
        }
    }

    /// 保存回显全部成功后执行被延迟的导航。
    ///
    /// 适用场景：未保存确认选择 Save。示例：保存失败 -> 保留原视图和草稿。
    fn finish_pending_markdown_navigation(&mut self, app: &GsdvGuiApp) {
        let Some(pending) = self.pending_markdown_navigation.as_ref() else {
            return;
        };
        if self
            .markdown_workbenches
            .items
            .values()
            .any(|item| item.saving)
        {
            return;
        }
        if self
            .markdown_workbenches
            .items
            .values()
            .any(MarkdownWorkbench::dirty)
        {
            self.pending_markdown_navigation = None;
            self.dirty = true;
            return;
        }
        let pending = self
            .pending_markdown_navigation
            .take()
            .expect("pending navigation checked above");
        self.apply_hit_action(pending.action, pending.x, pending.y, app);
    }

    /// 丢弃全部脏 Markdown 草稿。
    ///
    /// 适用场景：未保存确认选择 Discard。示例：恢复各自 saved_text。
    fn discard_dirty_workbenches(&mut self) {
        for item in self.markdown_workbenches.items.values_mut() {
            if item.dirty() {
                item.text = item.saved_text.clone();
                item.cursor = item.text.len();
                item.scroll = 0;
                item.saving = false;
            }
        }
    }
}

/// 等待 Markdown 保存成功后执行的点击动作。
///
/// 适用场景：Save/Discard/Cancel 切换保护。示例：Save 成功 -> SelectWorkflow。
struct PendingMarkdownNavigation {
    /// 保存成功后执行的原始动作。
    action: HitAction,
    /// 原始点击横坐标。
    x: u16,
    /// 原始点击纵坐标。
    y: u16,
}

/// 把 emulator damage 合并进本机字符界面 viewport。
///
/// 适用场景：Agent pane 与 workspace terminal 共用。示例：Rows -> 仅替换变化行。
fn apply_terminal_tui_update(frame: &mut TerminalTuiFrame, update: TerminalTuiUpdate) -> bool {
    match update {
        TerminalTuiUpdate::Unchanged => false,
        TerminalTuiUpdate::Rows {
            rows,
            cursor,
            terminal_scroll,
            max_scroll_offset,
        } => {
            let mut changed = false;
            for (index, row) in rows {
                if let Some(target) = frame.rows.get_mut(index)
                    && *target != row
                {
                    *target = row;
                    changed = true;
                }
            }
            let cursor = stable_terminal_tui_cursor(frame.cursor, cursor);
            if frame.cursor != cursor {
                frame.cursor = cursor;
                changed = true;
            }
            if frame.terminal_scroll != terminal_scroll {
                frame.terminal_scroll = terminal_scroll;
                changed = true;
            }
            if frame.max_scroll_offset != max_scroll_offset {
                frame.max_scroll_offset = max_scroll_offset;
                changed = true;
            }
            changed
        }
        TerminalTuiUpdate::Frame(mut next) => {
            next.cursor = stable_terminal_tui_cursor(frame.cursor, next.cursor);
            if *frame == next {
                return false;
            }
            *frame = next;
            true
        }
    }
}

/// 合并子 terminal cursor，并在隐藏期间保留最后一个可见输入锚点。
///
/// 适用场景：Agent Busy 时隐藏 cursor 并在屏幕内移动。示例：visible (4,8) + hidden (20,1) -> hidden (4,8)。
fn stable_terminal_tui_cursor(
    current: Option<TerminalTuiCursor>,
    next: Option<TerminalTuiCursor>,
) -> Option<TerminalTuiCursor> {
    let next = next?;
    if next.visible {
        return Some(next);
    }
    Some(TerminalTuiCursor {
        position: current
            .map(|cursor| cursor.position)
            .unwrap_or(next.position),
        visible: false,
    })
}

/// 生成终端客户端剪贴板使用的 OSC 52 序列。
///
/// 适用场景：SSH/Tmux 中复制 Workflow 逻辑路径。示例：text -> OSC 52 base64。
fn osc52_copy_sequence(text: &str) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    format!("\x1b]52;c;{encoded}\x07")
}

/// 把 terminal key 转成 PTY 字节，AppEvent 再负责真正写入。
///
/// 适用场景：无 TUI 全局动作时。示例：Left -> CSI D，Ctrl+C -> 0x03。
fn terminal_bytes_for_key(key: KeyEvent) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    if key.modifiers.contains(KeyModifiers::ALT) {
        bytes.push(0x1b);
    }
    match key.code {
        KeyCode::Char(ch) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let upper = ch.to_ascii_uppercase() as u8;
            if upper.is_ascii_uppercase() {
                bytes.push(upper - b'A' + 1);
            } else {
                return None;
            }
        }
        KeyCode::Char(ch) => {
            let mut buffer = [0_u8; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
        }
        KeyCode::Enter => bytes.push(b'\r'),
        KeyCode::Tab => bytes.push(b'\t'),
        KeyCode::BackTab => bytes.extend_from_slice(b"\x1b[Z"),
        KeyCode::Backspace => bytes.push(0x7f),
        KeyCode::Esc => bytes.push(0x1b),
        KeyCode::Up => bytes.extend_from_slice(b"\x1b[A"),
        KeyCode::Down => bytes.extend_from_slice(b"\x1b[B"),
        KeyCode::Right => bytes.extend_from_slice(b"\x1b[C"),
        KeyCode::Left => bytes.extend_from_slice(b"\x1b[D"),
        KeyCode::Home => bytes.extend_from_slice(b"\x1b[H"),
        KeyCode::End => bytes.extend_from_slice(b"\x1b[F"),
        KeyCode::PageUp => bytes.extend_from_slice(b"\x1b[5~"),
        KeyCode::PageDown => bytes.extend_from_slice(b"\x1b[6~"),
        KeyCode::Insert => bytes.extend_from_slice(b"\x1b[2~"),
        KeyCode::Delete => bytes.extend_from_slice(b"\x1b[3~"),
        _ => return None,
    }
    Some(bytes)
}

/// 返回字符串终端显示宽度。
///
/// 适用场景：布局。示例：中文按两列计算。
fn display_width(text: &str) -> usize {
    text.chars()
        .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(0))
        .sum()
}

/// 构造新增 Agent 的全字段表单。
///
/// 适用场景：Add tab/row/column。示例：默认 Agent 类型来自 settings。
fn add_agent_form(index: usize, placement: AddAgentPlacement, kind: AgentKind) -> FormDialog {
    let kinds = AgentKind::all();
    let selected = kinds.iter().position(|item| *item == kind).unwrap_or(0);
    FormDialog {
        title: "Add Agent".to_string(),
        fields: vec![
            FormField::Text {
                label: "Name".to_string(),
                value: String::new(),
            },
            FormField::Choice {
                label: "Agent type".to_string(),
                options: kinds.iter().map(|item| item.title().to_string()).collect(),
                selected,
            },
            FormField::Text {
                label: "Model".to_string(),
                value: String::new(),
            },
            FormField::Text {
                label: "Model provider".to_string(),
                value: String::new(),
            },
            FormField::Text {
                label: "Effort".to_string(),
                value: String::new(),
            },
            FormField::Choice {
                label: "Fast mode".to_string(),
                options: vec!["Default".to_string(), "On".to_string(), "Off".to_string()],
                selected: 0,
            },
            FormField::Text {
                label: "Work-dir".to_string(),
                value: String::new(),
            },
            FormField::Text {
                label: "Code work-dir".to_string(),
                value: String::new(),
            },
            FormField::Text {
                label: "Session ID".to_string(),
                value: String::new(),
            },
        ],
        active: 0,
        submit: FormSubmit::AddAgent { index, placement },
        error: None,
    }
}

/// 复用完整 Agent 表单创建 Workflow Agent，并填入目标默认名。
///
/// 适用场景：TUI Workflow 右键 Agent。例：Task build -> Name 默认 build。
fn workflow_agent_form(index: usize, binding: WorkflowAgentBinding, kind: AgentKind) -> FormDialog {
    let mut form = add_agent_form(index, AddAgentPlacement::Workflow(binding.clone()), kind);
    form.title = format!("Workflow Agent · {}", binding.label);
    if let Some(FormField::Text { value, .. }) = form.fields.first_mut() {
        *value = binding.label;
    }
    form
}

/// 构造新增或切换 workspace 路径表单。
///
/// 适用场景：底栏新增、workspace 菜单切目录。
fn workspace_path_form(change: bool, index: Option<usize>) -> FormDialog {
    FormDialog {
        title: if change {
            "Change Workspace Directory"
        } else {
            "New Workspace"
        }
        .to_string(),
        fields: vec![FormField::Text {
            label: "Path".to_string(),
            value: String::new(),
        }],
        active: 0,
        submit: index
            .map(FormSubmit::ChangeWorkspace)
            .unwrap_or(FormSubmit::AddWorkspace),
        error: None,
    }
}

/// 构造 Agent 单文本设置菜单项。
///
/// 适用场景：model/provider/work-dir/name。示例：当前值作为表单初值。
fn agent_text_menu(
    label: &str,
    target: &AgentTarget,
    kind: AgentTextField,
    app: &GsdvGuiApp,
) -> MenuItem {
    let metadata = app.agent_metadata_for_slot(target.workspace_index, &target.slot);
    let value = metadata
        .as_ref()
        .map(|metadata| match kind {
            AgentTextField::Model => metadata.agent_model.clone().unwrap_or_default(),
            AgentTextField::ModelProvider => {
                metadata.agent_model_provider.clone().unwrap_or_default()
            }
            AgentTextField::WorkDir => metadata
                .agent_work_dir
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            AgentTextField::HelixWorkDir => metadata
                .helix_work_dir
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            AgentTextField::Name => metadata.name.clone(),
        })
        .unwrap_or_default();
    MenuItem {
        label: label.to_string(),
        action: Some(MenuAction::Form(FormDialog {
            title: label.trim_end_matches("...").to_string(),
            fields: vec![FormField::Text {
                label: "Value".to_string(),
                value,
            }],
            active: 0,
            submit: FormSubmit::AgentText {
                target: target.clone(),
                kind,
            },
            error: None,
        })),
    }
}

/// 调整当前 choice 字段。
///
/// 适用场景：表单 Left/Right/Up/Down。示例：Default -> On。
fn adjust_form_choice(form: &mut FormDialog, forward: bool) {
    let Some(FormField::Choice {
        options, selected, ..
    }) = form.fields.get_mut(form.active)
    else {
        return;
    };
    if options.is_empty() {
        return;
    }
    *selected = if forward {
        (*selected + 1) % options.len()
    } else {
        (*selected + options.len() - 1) % options.len()
    };
}

/// 创建新增 Workflow Project 表单。
///
/// 适用场景：Repo 根节点右键菜单。示例：`demo` -> `gsdv-spec/ps/demo/root.md`。
fn workflow_add_project_form(index: usize, spec_path: PathBuf) -> FormDialog {
    FormDialog {
        title: "New Workflow Project".to_string(),
        fields: vec![FormField::Text {
            label: "Project key".to_string(),
            value: String::new(),
        }],
        active: 0,
        submit: FormSubmit::WorkflowAddProject { index, spec_path },
        error: None,
    }
}

/// 创建新增 Workflow Task 表单。
///
/// 适用场景：Project 节点右键菜单。示例：`login` -> `task-login.md`。
fn workflow_add_task_form(index: usize, spec_path: PathBuf, project_key: String) -> FormDialog {
    FormDialog {
        title: format!("New Task in {project_key}"),
        fields: vec![FormField::Text {
            label: "Task key".to_string(),
            value: String::new(),
        }],
        active: 0,
        submit: FormSubmit::WorkflowAddTask {
            index,
            spec_path,
            project_key,
        },
        error: None,
    }
}

/// 创建新增 Workflow Step 表单。
///
/// 适用场景：Task 节点右键菜单。示例：标题与可选单行说明 -> 新 step。
fn workflow_add_step_form(index: usize, task_path: PathBuf) -> FormDialog {
    FormDialog {
        title: "New Workflow Step".to_string(),
        fields: vec![
            FormField::Text {
                label: "Step title".to_string(),
                value: String::new(),
            },
            FormField::Text {
                label: "Description".to_string(),
                value: String::new(),
            },
        ],
        active: 0,
        submit: FormSubmit::WorkflowAddStep { index, task_path },
        error: None,
    }
}

/// 校验并把通用表单转换成业务动作。
///
/// 适用场景：Enter。示例：空 Agent 名 -> Err。
fn submit_form(form: &FormDialog, app: &GsdvGuiApp) -> std::result::Result<TuiAppAction, String> {
    match &form.submit {
        FormSubmit::AddAgent { index, placement } => {
            let name = form_text(form, 0);
            if name.trim().is_empty() {
                return Err("Agent name is required".to_string());
            }
            let kinds = AgentKind::all();
            let kind = kinds.get(form_choice(form, 1)).copied().unwrap_or_default();
            let value = TuiAgentFormValue {
                name,
                kind,
                model: form_text(form, 2),
                model_provider: form_text(form, 3),
                effort: form_text(form, 4),
                fast_mode: match form_choice(form, 5) {
                    1 => Some(true),
                    2 => Some(false),
                    _ => None,
                },
                work_dir: form_text(form, 6),
                helix_work_dir: form_text(form, 7),
                session_id: form_text(form, 8),
            };
            Ok(match placement {
                AddAgentPlacement::ExistingColumn(column_id) => TuiAppAction::AddAgent {
                    index: *index,
                    column_id: column_id.clone(),
                    value,
                },
                AddAgentPlacement::NewRow(after_row) => TuiAppAction::AddRowWithAgent {
                    index: *index,
                    after_row: *after_row,
                    value,
                },
                AddAgentPlacement::NewColumn(row_index) => TuiAppAction::AddColumnWithAgent {
                    index: *index,
                    row_index: *row_index,
                    value,
                },
                AddAgentPlacement::Workflow(binding) => TuiAppAction::AddWorkflowAgent {
                    index: *index,
                    binding: binding.clone(),
                    value,
                },
            })
        }
        FormSubmit::AddWorkspace => {
            let path = PathBuf::from(form_text(form, 0));
            if !path.is_dir() {
                return Err("Workspace path must be an existing directory".to_string());
            }
            Ok(TuiAppAction::AddWorkspace(path))
        }
        FormSubmit::ChangeWorkspace(index) => {
            let path = PathBuf::from(form_text(form, 0));
            if !path.is_dir() {
                return Err("Workspace path must be an existing directory".to_string());
            }
            Ok(TuiAppAction::ChangeWorkspaceDirectory {
                index: *index,
                path,
            })
        }
        FormSubmit::AgentText { target, kind } => {
            let value = form_text(form, 0);
            Ok(match kind {
                AgentTextField::Model => TuiAppAction::SetAgentModel {
                    index: target.workspace_index,
                    slot: target.slot.clone(),
                    value,
                },
                AgentTextField::ModelProvider => TuiAppAction::SetAgentModelProvider {
                    index: target.workspace_index,
                    slot: target.slot.clone(),
                    value,
                },
                AgentTextField::WorkDir => TuiAppAction::SetAgentWorkDir {
                    index: target.workspace_index,
                    slot: target.slot.clone(),
                    value,
                },
                AgentTextField::HelixWorkDir => TuiAppAction::SetAgentHelixWorkDir {
                    index: target.workspace_index,
                    slot: target.slot.clone(),
                    value,
                },
                AgentTextField::Name => {
                    let AgentSlotId::Subagent(id) = &target.slot else {
                        return Err("Main Agent cannot be renamed".to_string());
                    };
                    TuiAppAction::RenameSubagent {
                        index: target.workspace_index,
                        id: id.clone(),
                        name: value,
                    }
                }
            })
        }
        FormSubmit::PomodoroSettings => {
            let work_minutes = parse_form_number::<u16>(form, 0, "Work minutes")?;
            let rest_minutes = parse_form_number::<u16>(form, 1, "Rest minutes")?;
            let warning_remaining_percent = parse_form_number::<u8>(form, 2, "Warning %")?;
            if !(data::MIN_POMODORO_MINUTES..=data::MAX_POMODORO_MINUTES).contains(&work_minutes)
                || !(data::MIN_POMODORO_MINUTES..=data::MAX_POMODORO_MINUTES)
                    .contains(&rest_minutes)
            {
                return Err(format!(
                    "Minutes must be {}-{}",
                    data::MIN_POMODORO_MINUTES,
                    data::MAX_POMODORO_MINUTES
                ));
            }
            if !(data::MIN_POMODORO_WARNING_REMAINING_PERCENT
                ..=data::MAX_POMODORO_WARNING_REMAINING_PERCENT)
                .contains(&warning_remaining_percent)
            {
                return Err(format!(
                    "Warning must be {}-{}%",
                    data::MIN_POMODORO_WARNING_REMAINING_PERCENT,
                    data::MAX_POMODORO_WARNING_REMAINING_PERCENT
                ));
            }
            Ok(TuiAppAction::UpdatePomodoroSettings {
                work_minutes,
                rest_minutes,
                warning_remaining_percent,
            })
        }
        FormSubmit::NetworkSettings => Ok(TuiAppAction::UpdateNetworkSettings {
            proxy_enabled: form_choice(form, 0) == 1,
            proxy: form_text(form, 1),
            mirror_proxy_protocol: form_choice(form, 2) == 1,
            no_proxy: form_text(form, 3),
        }),
        FormSubmit::WorkflowAddProject { index, spec_path } => {
            let request = WorkflowMutationRequest::AddProject {
                spec_path: spec_path.clone(),
                project_key: form_text(form, 0),
            };
            if let Some(error) = app.workflow_mutation_key_error(&request) {
                return Err(error);
            }
            Ok(TuiAppAction::WorkflowMutation {
                index: *index,
                request,
            })
        }
        FormSubmit::WorkflowAddTask {
            index,
            spec_path,
            project_key,
        } => {
            let request = WorkflowMutationRequest::AddTask {
                spec_path: spec_path.clone(),
                project_key: project_key.clone(),
                task_key: form_text(form, 0),
            };
            if let Some(error) = app.workflow_mutation_key_error(&request) {
                return Err(error);
            }
            Ok(TuiAppAction::WorkflowMutation {
                index: *index,
                request,
            })
        }
        FormSubmit::WorkflowAddStep { index, task_path } => {
            let request = WorkflowMutationRequest::AddStep {
                task_path: task_path.clone(),
                key: form_text(form, 0),
                desc: form_text(form, 1),
            };
            if let Some(error) = app.workflow_mutation_key_error(&request) {
                return Err(error);
            }
            Ok(TuiAppAction::WorkflowMutation {
                index: *index,
                request,
            })
        }
    }
}

/// 在已加载 outline 中递归展开目标目录。
///
/// 适用场景：TUI Column View 请求下一列。示例：匹配 key -> expanded=true。
fn set_outline_directory_expanded(nodes: &mut [OutlineNode], target: &Path) -> bool {
    for node in nodes {
        match node {
            OutlineNode::Root {
                key,
                expanded,
                children,
                ..
            }
            | OutlineNode::Dir {
                key,
                expanded,
                children,
                ..
            } => {
                if key == target {
                    *expanded = true;
                    return true;
                }
                if set_outline_directory_expanded(children, target) {
                    *expanded = true;
                    return true;
                }
            }
            OutlineNode::File { .. } => {}
        }
    }
    false
}

/// 解析表单中的无符号数字并返回字段级错误。
///
/// 适用场景：番茄参数。示例：`abc` -> `Work minutes must be a number`。
fn parse_form_number<T>(
    form: &FormDialog,
    index: usize,
    label: &str,
) -> std::result::Result<T, String>
where
    T: std::str::FromStr,
{
    form_text(form, index)
        .trim()
        .parse()
        .map_err(|_| format!("{label} must be a number"))
}

/// 读取文本字段，类型不匹配时返回空字符串。
///
/// 适用场景：表单提交。示例：index 越界 -> empty。
fn form_text(form: &FormDialog, index: usize) -> String {
    match form.fields.get(index) {
        Some(FormField::Text { value, .. }) => value.clone(),
        _ => String::new(),
    }
}

/// 读取 choice 字段，类型不匹配时返回 0。
///
/// 适用场景：表单提交。示例：Fast mode -> selected index。
fn form_choice(form: &FormDialog, index: usize) -> usize {
    match form.fields.get(index) {
        Some(FormField::Choice { selected, .. }) => *selected,
        _ => 0,
    }
}

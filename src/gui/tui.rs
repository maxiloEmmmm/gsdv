//! 原生终端入口。
//!
//! TUI 只负责输入、布局和 ANSI 差量绘制；workspace、Agent、PTY、Hook、
//! 番茄钟和持久化继续由 `GsdvGuiApp` 与唯一 `AppEvent` 队列持有。

use super::*;
use crate::gui::terminal_host::TerminalRemoteRow;
use anyhow::{Context, Result};
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as TerminalEvent, KeyCode, KeyEvent,
    KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, ModifierKeyCode, MouseButton, MouseEvent,
    MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::style::Print;
use crossterm::terminal::{
    self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode,
};
use crossterm::{execute, queue};
use std::collections::BTreeMap;
use std::io::{Stdout, Write, stdout};
use std::path::PathBuf;
use unicode_width::UnicodeWidthChar;

#[path = "tui/menu.rs"]
mod menu;
#[path = "tui/pomodoro.rs"]
mod pomodoro;
#[path = "tui/render.rs"]
mod render;

use menu::*;
use pomodoro::*;
use render::*;

const TUI_POLL_INTERVAL: Duration = Duration::from_millis(20);
const TUI_MIN_WIDTH: u16 = 48;
const TUI_MIN_HEIGHT: u16 = 12;
const TUI_SIDEBAR_WIDTH: u16 = 27;
const TUI_COMPACT_SIDEBAR_WIDTH: u16 = 19;
const TUI_COLLAPSED_SIDEBAR_WIDTH: u16 = 3;
const TUI_SPLITTER_SIZE: u16 = 1;
const TUI_MIN_PANE_WIDTH: u16 = 12;
const TUI_MIN_PANE_HEIGHT: u16 = 4;

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
    /// 切换 Remote Workspace 启用状态。
    ToggleRemoteWorkspace(usize),
    /// 重启 Remote Workspace 的所有 Agent。
    RestartRemoteAgents(usize),
    /// 从输入路径新增本地 workspace。
    AddWorkspace(PathBuf),
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
    /// 等宽当前 row 的 columns。
    EqualizeColumns { index: usize, row_index: usize },
    /// 等高所有 rows。
    EqualizeRows { index: usize },
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
    /// 开关持久化番茄钟功能。
    SetPomodoroEnabled(bool),
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
            TuiAppAction::ToggleRemoteWorkspace(index) => self.toggle_remote_workspace(ctx, index),
            TuiAppAction::RestartRemoteAgents(index) => {
                self.restart_remote_workspace_agents(ctx, index)
            }
            TuiAppAction::AddWorkspace(path) => {
                let existing = self
                    .workspaces
                    .iter()
                    .map(|item| item.path.clone())
                    .collect();
                self.spawn_workspace_add_task(ctx, path, self.default_agent_kind, existing);
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
            TuiAppAction::EqualizeColumns { index, row_index } => {
                self.equalize_agent_columns(index, row_index)
            }
            TuiAppAction::EqualizeRows { index } => self.equalize_agent_rows(index),
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
            TuiAppAction::SetPomodoroEnabled(enabled) => {
                self.runtime_settings.pomodoro_enabled = enabled;
                self.pending_runtime_settings_save = true;
                self.pomodoro.start_working(Instant::now());
                self.request_app_repaint();
            }
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

/// 可拖动 row/column 分隔线的业务目标。
///
/// 适用场景：绘制阶段生成命中区，鼠标按下后转为拖动状态。
#[derive(Debug, Clone)]
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
    source: TerminalRemoteOutputSource,
    /// update_since 对比状态。
    output_state: TerminalRemoteOutputState,
    /// 当前完整可绘制快照。
    snapshot: TerminalRemoteSnapshot,
    /// 最近同步给 PTY 的 cols/lines。
    grid_size: (u16, u16),
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
    /// 切换专注模式。
    ToggleSpotlight,
    /// 折叠或展开 workspace rail。
    ToggleSidebar,
    /// 打开新 workspace 表单。
    NewWorkspace,
    /// 打开番茄菜单。
    PomodoroMenu,
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
    /// workspace rail 是否折叠为最小宽度。
    sidebar_collapsed: bool,
    /// Busy 状态动画当前帧。
    busy_frame: usize,
    /// 上次推进 Busy 动画的时间。
    last_busy_frame_at: Instant,
    /// 上次绘制的番茄剩余分钟数，用于避免常驻 TUI 每秒输出。
    last_pomodoro_minute: Option<u64>,
    /// 当前键盘输入目标。
    focused_target: Option<AgentTarget>,
    /// 每个已见 pane 的 terminal diff cache。
    panes: BTreeMap<AgentTarget, PaneCache>,
    /// 当前布局中的可见 panes。
    visible_panes: Vec<PaneLayout>,
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
    /// 当前唯一菜单、表单或番茄覆盖层。
    layer: Option<TuiLayer>,
    /// 左侧列表滚动偏移。
    sidebar_scroll: usize,
    /// 当前聚焦 Agent 的宿主终端光标位置。
    cursor_position: Option<(u16, u16)>,
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
            sidebar_collapsed: false,
            busy_frame: 0,
            last_busy_frame_at: Instant::now(),
            last_pomodoro_minute: None,
            focused_target: None,
            panes: BTreeMap::new(),
            visible_panes: Vec::new(),
            previous_frame: Vec::new(),
            last_frame_at: None,
            dirty: true,
            state_fingerprint: String::new(),
            hit_regions: Vec::new(),
            splitter_regions: Vec::new(),
            splitter_drag: None,
            layer: None,
            sidebar_scroll: 0,
            cursor_position: None,
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
            self.sync_pomodoro_layer(app);
            app.process_workspace_store_writer(ctx);
            self.sync_layout_and_terminals(app, ctx);
            self.refresh_state_fingerprint(app);
            self.refresh_busy_animation(app);
            self.refresh_pomodoro_countdown(app);
            self.refresh_terminal_updates();

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

    /// 根据现有 workspace row/col 或专注列表生成 pane，并只启动可见 Agent。
    ///
    /// 适用场景：每轮轻量同步。示例：切 tab -> 新 target 建 source。
    fn sync_layout_and_terminals(&mut self, app: &mut GsdvGuiApp, ctx: &egui::Context) {
        let next = self.compute_pane_layout(app);
        if pane_layout_fingerprint(&next) != pane_layout_fingerprint(&self.visible_panes) {
            self.visible_panes = next;
            self.dirty = true;
        }
        if self
            .focused_target
            .as_ref()
            .is_none_or(|target| !self.visible_panes.iter().any(|pane| &pane.target == target))
        {
            self.focused_target = self.visible_panes.first().map(|pane| pane.target.clone());
        }

        for pane in &self.visible_panes {
            app.ensure_agent_terminal_host(ctx, pane.target.workspace_index, &pane.target.slot);
            let terminal_lines = pane.rect.height.saturating_sub(1).max(1);
            let terminal_cols = pane.rect.width.max(1);
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
                let source = host.remote_output_source();
                let (snapshot, output_state) = source.snapshot();
                self.panes.insert(
                    pane.target.clone(),
                    PaneCache {
                        host_id,
                        source,
                        output_state,
                        snapshot,
                        grid_size,
                    },
                );
                self.dirty = true;
            } else if let Some(cache) = self.panes.get_mut(&pane.target) {
                cache.grid_size = grid_size;
            }
        }
    }

    /// 计算当前模式的可见 pane 字符矩形。
    ///
    /// 适用场景：resize、workspace 布局变化。示例：两列 -> 各占可用宽度一半。
    fn compute_pane_layout(&self, app: &GsdvGuiApp) -> Vec<PaneLayout> {
        let (width, height) = self.size;
        if width < TUI_MIN_WIDTH || height < TUI_MIN_HEIGHT {
            return Vec::new();
        }
        let sidebar = sidebar_width(width, self.sidebar_collapsed);
        let body = TuiRect {
            x: sidebar.saturating_add(1),
            y: 1,
            width: width.saturating_sub(sidebar.saturating_add(1)),
            height: height.saturating_sub(2),
        };
        if self.spotlight_mode {
            let target = self.focused_target.clone().or_else(|| {
                app.spotlight_agents().first().map(|entry| AgentTarget {
                    workspace_index: entry.workspace_index,
                    slot: entry.slot.clone(),
                })
            });
            return target
                .map(|target| PaneLayout {
                    rect: body,
                    target,
                    row_index: 0,
                    column_index: 0,
                    column_id: "spotlight".to_string(),
                    tabs: Vec::new(),
                })
                .into_iter()
                .collect();
        }
        let Some(workspace) = app.workspaces.get(app.active_workspace) else {
            return Vec::new();
        };
        let visible_rows = workspace
            .agent_rows
            .iter()
            .enumerate()
            .filter(|(_, row)| !row.collapsed)
            .collect::<Vec<_>>();
        if visible_rows.is_empty() {
            return Vec::new();
        }
        let row_splitters = u16::try_from(visible_rows.len().saturating_sub(1))
            .unwrap_or(u16::MAX)
            .saturating_mul(TUI_SPLITTER_SIZE);
        let row_content_height = body.height.saturating_sub(row_splitters);
        let row_heights = weighted_lengths(
            row_content_height,
            &visible_rows
                .iter()
                .map(|(_, row)| row.height_weight)
                .collect::<Vec<_>>(),
        );
        let mut panes = Vec::new();
        let mut y = body.y;
        for ((row_index, row), row_height) in visible_rows.into_iter().zip(row_heights) {
            let columns = row
                .columns
                .iter()
                .enumerate()
                .filter(|(_, column)| !column.collapsed)
                .collect::<Vec<_>>();
            if columns.is_empty() {
                y = y
                    .saturating_add(row_height)
                    .saturating_add(TUI_SPLITTER_SIZE);
                continue;
            }
            let column_splitters = u16::try_from(columns.len().saturating_sub(1))
                .unwrap_or(u16::MAX)
                .saturating_mul(TUI_SPLITTER_SIZE);
            let column_content_width = body.width.saturating_sub(column_splitters);
            let widths = weighted_lengths(
                column_content_width,
                &columns
                    .iter()
                    .map(|(_, column)| column.width_weight)
                    .collect::<Vec<_>>(),
            );
            let mut x = body.x;
            for ((column_index, column), column_width) in columns.into_iter().zip(widths) {
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
                x = x
                    .saturating_add(column_width)
                    .saturating_add(TUI_SPLITTER_SIZE);
            }
            y = y
                .saturating_add(row_height)
                .saturating_add(TUI_SPLITTER_SIZE);
        }
        panes
    }

    /// 消费所有可见 terminal source 的最小更新。
    ///
    /// 适用场景：PTY 输出变化。示例：Patch 只替换 cache 对应行。
    fn refresh_terminal_updates(&mut self) {
        for pane in &self.visible_panes {
            let Some(cache) = self.panes.get_mut(&pane.target) else {
                continue;
            };
            match cache.source.update_since(&mut cache.output_state) {
                TerminalRemoteUpdate::Unchanged => {}
                TerminalRemoteUpdate::Append(append) => {
                    cache.snapshot.rows.extend(append.rows);
                    let overflow = cache.snapshot.rows.len().saturating_sub(500);
                    if overflow > 0 {
                        cache.snapshot.rows.drain(..overflow);
                    }
                    self.dirty = true;
                }
                TerminalRemoteUpdate::Patch(patch) => {
                    for row in patch.rows {
                        if let Some(target) = cache.snapshot.rows.get_mut(row.index) {
                            *target = row.row;
                        }
                    }
                    cache.snapshot.cursor = patch.cursor;
                    self.dirty = true;
                }
                TerminalRemoteUpdate::Snapshot(snapshot) => {
                    cache.snapshot = snapshot;
                    self.dirty = true;
                }
            }
        }
    }

    /// 比较 Agent 状态、workspace 和番茄钟摘要，发现变化后标脏。
    ///
    /// 适用场景：Hook 只改 metadata、没有 PTY 输出。示例：Busy -> Idle。
    fn refresh_state_fingerprint(&mut self, app: &GsdvGuiApp) {
        let mut next = format!(
            "{}:{}:{:?}:{:?}:{}:{}:{}",
            app.active_workspace,
            self.spotlight_mode,
            app.pomodoro.phase,
            app.runtime_settings.pomodoro_enabled,
            app.runtime_settings.pomodoro_work_minutes,
            app.runtime_settings.pomodoro_rest_minutes,
            app.runtime_settings.pomodoro_warning_remaining_percent,
        );
        for (index, workspace) in app.workspaces.iter().enumerate() {
            next.push_str(&format!(
                "|{index}:{}:{:?}:{}:{}",
                workspace.name,
                workspace.activity,
                workspace.focused,
                workspace.subagents.len()
            ));
            for subagent in &workspace.subagents {
                next.push_str(&format!(":{}:{:?}", subagent.name, subagent.activity));
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

    /// Busy 期间按配置 FPS 推进同一套状态动画，Idle 时完全停止。
    ///
    /// 适用场景：workspace、专注列表和 tab。示例：Busy -> spinner 每帧变化。
    fn refresh_busy_animation(&mut self, app: &GsdvGuiApp) {
        let busy = app.workspaces.iter().any(|workspace| {
            super::app_chrome::workspace_effective_activity(workspace) == WorkspaceActivity::Busy
        });
        if busy && self.last_busy_frame_at.elapsed() >= self.frame_interval {
            self.busy_frame = self.busy_frame.wrapping_add(1);
            self.last_busy_frame_at = Instant::now();
            self.dirty = true;
        }
    }

    /// 番茄倒计时只在显示分钟数变化时标记重绘。
    ///
    /// 适用场景：底栏、Alt 控制层和全屏 R。示例：25m -> 24m 重绘一次。
    fn refresh_pomodoro_countdown(&mut self, app: &GsdvGuiApp) {
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
                if matches!(self.layer, Some(TuiLayer::Rest)) {
                    self.handle_rest_key(key, app);
                } else if matches!(self.layer, Some(TuiLayer::Pomodoro)) {
                    self.handle_pomodoro_key(key, app);
                } else if matches!(
                    self.layer,
                    Some(TuiLayer::Form(_) | TuiLayer::Confirm { .. })
                ) {
                    self.handle_overlay_key(key, app);
                } else if matches!(self.layer, Some(TuiLayer::Menu(_))) {
                    self.handle_menu_key(key, app);
                } else if self.handle_global_key(key, app) {
                    return Ok(true);
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
    fn queue_terminal_bytes(&self, app: &GsdvGuiApp, bytes: Vec<u8>) {
        let Some(target) = self.focused_target.as_ref() else {
            return;
        };
        app.queue_app_event(AppEvent::InputTerminalBytes {
            workspace_index: target.workspace_index,
            target: TerminalSurfaceKind::Agent,
            agent_slot: target.slot.clone(),
            bytes,
        });
    }

    /// 把 Enter 等特殊键投递到现有 terminal 协议编码路径。
    ///
    /// 适用场景：Codex 开启 kitty keyboard protocol。示例：Enter -> CSI 13u。
    fn queue_terminal_key(&self, app: &GsdvGuiApp, key: TerminalRemoteKey) {
        let Some(target) = self.focused_target.as_ref() else {
            return;
        };
        app.queue_app_event(AppEvent::TuiAction(TuiAppAction::TerminalKey {
            index: target.workspace_index,
            slot: target.slot.clone(),
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
        self.queue_terminal_bytes(app, text.into_bytes());
        app.queue_app_event(AppEvent::PomodoroInputDetected);
    }

    /// 处理鼠标选择、右键菜单和滚轮。
    ///
    /// 适用场景：EnableMouseCapture 后。示例：右键 tab -> Agent menu。
    fn handle_mouse(&mut self, event: MouseEvent, app: &GsdvGuiApp) {
        if matches!(
            self.layer,
            Some(TuiLayer::Form(_) | TuiLayer::Confirm { .. })
        ) {
            self.handle_overlay_mouse(event, app);
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
                    .find(|pane| pane.rect.contains(event.column, event.row))
                {
                    let delta = if matches!(event.kind, MouseEventKind::ScrollUp) {
                        3
                    } else {
                        -3
                    };
                    app.queue_app_event(AppEvent::TuiAction(TuiAppAction::ScrollAgent {
                        index: pane.target.workspace_index,
                        slot: pane.target.slot.clone(),
                        delta,
                        pointer_cell: Some((
                            event.column.saturating_sub(pane.rect.x),
                            event.row.saturating_sub(pane.rect.y.saturating_add(1)),
                        )),
                    }));
                }
            }
            _ => {}
        }
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
                        match submit_form(form) {
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
            _ => keep = false,
        }
        if keep {
            self.layer = Some(layer);
        }
        self.dirty = true;
    }

    /// 执行点击区域动作或打开对应菜单。
    ///
    /// 适用场景：鼠标 Down。示例：左键 pane -> 聚焦输入目标。
    fn apply_hit_action(&mut self, action: HitAction, x: u16, y: u16, app: &GsdvGuiApp) {
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
            HitAction::ToggleSpotlight => {
                self.spotlight_mode = !self.spotlight_mode;
                self.focused_target = None;
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
        }
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
        if workspace.remote.is_some() {
            items.push(menu_queue(
                if app.remote_enabled.get(index).copied().unwrap_or(true) {
                    "Disable remote"
                } else {
                    "Enable remote"
                },
                TuiAppAction::ToggleRemoteWorkspace(index),
            ));
            items.push(menu_queue(
                "Restart remote agents",
                TuiAppAction::RestartRemoteAgents(index),
            ));
        } else {
            items.push(MenuItem {
                label: "Change directory...".to_string(),
                action: Some(MenuAction::Form(workspace_path_form(true, Some(index)))),
            });
        }
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
            items.push(menu_confirm(
                "Close subagent",
                "Close and delete this subagent?",
                TuiAppAction::RemoveSubagent {
                    index: target.workspace_index,
                    id: id.clone(),
                },
            ));
        }
        if let (Some(row_index), Some(column_index), Some(column_id)) =
            (context.row_index, context.column_index, context.column_id)
        {
            items.push(menu_separator());
            items.push(menu_submenu(
                "Layout",
                vec![
                    MenuItem {
                        label: "Add agent tab...".to_string(),
                        action: Some(MenuAction::Form(add_agent_form(
                            target.workspace_index,
                            AddAgentPlacement::ExistingColumn(column_id.clone()),
                            app.default_agent_kind,
                        ))),
                    },
                    MenuItem {
                        label: "Add row + agent...".to_string(),
                        action: Some(MenuAction::Form(add_agent_form(
                            target.workspace_index,
                            AddAgentPlacement::NewRow(row_index),
                            app.default_agent_kind,
                        ))),
                    },
                    MenuItem {
                        label: "Add column + agent...".to_string(),
                        action: Some(MenuAction::Form(add_agent_form(
                            target.workspace_index,
                            AddAgentPlacement::NewColumn(row_index),
                            app.default_agent_kind,
                        ))),
                    },
                    menu_queue(
                        "Collapse column",
                        TuiAppAction::SetColumnCollapsed {
                            index: target.workspace_index,
                            row_index,
                            column_index,
                            collapsed: true,
                        },
                    ),
                ],
            ));
        }
        self.open_menu(x, y, items);
    }

    /// 打开 row/column 结构菜单。
    ///
    /// 适用场景：pane header 空白处右键。示例：新增 row/col、等分和关闭。
    fn open_pane_menu(&mut self, x: u16, y: u16, app: &GsdvGuiApp, context: PaneMenuContext) {
        let items = vec![
            MenuItem {
                label: "Add agent tab...".to_string(),
                action: Some(MenuAction::Form(add_agent_form(
                    context.workspace_index,
                    AddAgentPlacement::ExistingColumn(context.column_id.clone()),
                    app.default_agent_kind,
                ))),
            },
            MenuItem {
                label: "Add row + agent...".to_string(),
                action: Some(MenuAction::Form(add_agent_form(
                    context.workspace_index,
                    AddAgentPlacement::NewRow(context.row_index),
                    app.default_agent_kind,
                ))),
            },
            MenuItem {
                label: "Add column + agent...".to_string(),
                action: Some(MenuAction::Form(add_agent_form(
                    context.workspace_index,
                    AddAgentPlacement::NewColumn(context.row_index),
                    app.default_agent_kind,
                ))),
            },
            menu_queue(
                "Equalize columns",
                TuiAppAction::EqualizeColumns {
                    index: context.workspace_index,
                    row_index: context.row_index,
                },
            ),
            menu_queue(
                "Equalize rows",
                TuiAppAction::EqualizeRows {
                    index: context.workspace_index,
                },
            ),
            menu_queue(
                "Collapse column",
                TuiAppAction::SetColumnCollapsed {
                    index: context.workspace_index,
                    row_index: context.row_index,
                    column_index: context.column_index,
                    collapsed: true,
                },
            ),
            menu_queue(
                "Collapse row",
                TuiAppAction::SetRowCollapsed {
                    index: context.workspace_index,
                    row_index: context.row_index,
                    collapsed: true,
                },
            ),
            menu_confirm(
                "Close column",
                "Close this column and all subagents inside it?",
                TuiAppAction::RemoveColumn {
                    index: context.workspace_index,
                    column_id: context.column_id.clone(),
                },
            ),
            menu_confirm(
                "Close row",
                "Close this row and all subagents inside it?",
                TuiAppAction::RemoveRow {
                    index: context.workspace_index,
                    row_index: context.row_index,
                },
            ),
        ];
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
                    KeyCode::Enter => match submit_form(form) {
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
            _ => self.layer = Some(layer),
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
            self.draw_chrome(&mut frame, app);
            self.draw_panes(&mut frame, app);
            self.draw_layer(&mut frame, app);
        }
        let cursor = self
            .layer
            .is_none()
            .then_some(self.cursor_position)
            .flatten();
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
        if self.sidebar_collapsed {
            set_base_text(frame, 0, 0, "[>]", sidebar);
            styled_overlay_text(frame, 0, 0, "[>]", (34, 211, 238), (15, 23, 42), true);
        } else {
            set_base_text(frame, 0, 0, " GSDV TUI", sidebar);
            set_base_text(frame, 0, sidebar.saturating_sub(3), "[<]", 3);
            styled_overlay_text(frame, 0, 1, "GSDV TUI", (96, 165, 250), (11, 16, 22), true);
            styled_overlay_text(
                frame,
                0,
                sidebar.saturating_sub(3),
                "[<]",
                (34, 211, 238),
                (15, 23, 42),
                true,
            );
        }
        self.hit_regions.push(HitRegion {
            rect: TuiRect {
                x: if self.sidebar_collapsed {
                    0
                } else {
                    sidebar.saturating_sub(3)
                },
                y: 0,
                width: 3,
                height: 1,
            },
            left: Some(HitAction::ToggleSidebar),
            right: None,
        });
        put_char(frame, 0, sidebar, '│');
        set_base_text(
            frame,
            0,
            sidebar.saturating_add(2),
            if self.spotlight_mode {
                "SPOTLIGHT"
            } else {
                "AGENT WORKSPACE"
            },
            width.saturating_sub(sidebar.saturating_add(2)),
        );
        styled_overlay_text(
            frame,
            0,
            sidebar.saturating_add(2),
            if self.spotlight_mode {
                "SPOTLIGHT"
            } else {
                "AGENT WORKSPACE"
            },
            if self.spotlight_mode {
                (250, 204, 21)
            } else {
                (96, 165, 250)
            },
            (11, 16, 22),
            true,
        );
        for y in 1..height.saturating_sub(1) {
            put_char(frame, y, sidebar, '│');
        }

        let available = usize::from(height.saturating_sub(4));
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
                let y = 2 + screen_index as u16;
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
                let y = 2 + screen_index as u16;
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
        let focus_label = if self.spotlight_mode {
            "[F2 Exit focus]"
        } else {
            "[F2 Focus]"
        };
        set_base_text(
            frame,
            bottom,
            0,
            focus_label,
            u16::try_from(focus_label.len()).unwrap_or(12),
        );
        styled_overlay_text(
            frame,
            bottom,
            0,
            focus_label,
            if self.spotlight_mode {
                (15, 23, 42)
            } else {
                (250, 204, 21)
            },
            if self.spotlight_mode {
                (250, 204, 21)
            } else {
                (11, 16, 22)
            },
            true,
        );
        self.hit_regions.push(HitRegion {
            rect: TuiRect {
                x: 0,
                y: bottom,
                width: u16::try_from(focus_label.len()).unwrap_or(12),
                height: 1,
            },
            left: Some(HitAction::ToggleSpotlight),
            right: None,
        });
        let new_x = u16::try_from(focus_label.len())
            .unwrap_or(12)
            .saturating_add(1);
        set_base_text(frame, bottom, new_x, "[+ Workspace]", 13);
        styled_overlay_text(
            frame,
            bottom,
            new_x,
            "[+ Workspace]",
            (74, 222, 128),
            (11, 16, 22),
            true,
        );
        self.hit_regions.push(HitRegion {
            rect: TuiRect {
                x: new_x,
                y: bottom,
                width: 13,
                height: 1,
            },
            left: Some(HitAction::NewWorkspace),
            right: None,
        });
        let pomodoro = pomodoro_status(app);
        let pomodoro_width = display_width(&pomodoro).min(26) as u16;
        let pomodoro_x = width.saturating_sub(pomodoro_width.saturating_add(10));
        let pomodoro_warning = app.runtime_settings.pomodoro_enabled
            && app.pomodoro.phase == PomodoroPhase::Working
            && pomodoro_work_progress(&app.runtime_settings, &app.pomodoro)
                >= pomodoro_warning_progress(&app.runtime_settings);
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
        set_base_text(frame, bottom, width.saturating_sub(9), "[Ctrl+Q]", 8);
    }

    /// 绘制所有可见 row/col pane、tab header 和 terminal 内容。
    ///
    /// 适用场景：普通多 Agent 或专注单 Agent。示例：两个 columns 同时显示。
    fn draw_panes(&mut self, frame: &mut [RenderLine], app: &GsdvGuiApp) {
        for pane in self.visible_panes.clone() {
            if pane.rect.width == 0 || pane.rect.height == 0 {
                continue;
            }
            let focused = self.focused_target.as_ref() == Some(&pane.target);
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
                    .map(|level| format!("★{level}"))
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
                    frame, header, x, tab_width, &text, activity, stopped, active,
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
                let activity =
                    app.agent_slot_activity(pane.target.workspace_index, &pane.target.slot);
                let marker = if activity == WorkspaceActivity::Busy {
                    "─ BUSY ─"
                } else if focused {
                    "─ FOCUSED ─"
                } else {
                    "───────────"
                };
                set_base_text(frame, header, x, marker, header_end.saturating_sub(x));
                if activity == WorkspaceActivity::Busy {
                    styled_overlay_text(
                        frame,
                        header,
                        x,
                        marker,
                        (74, 222, 128),
                        (11, 16, 22),
                        true,
                    );
                }
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
                    right: Some(HitAction::PaneMenu(PaneMenuContext {
                        workspace_index: pane.target.workspace_index,
                        row_index: pane.row_index,
                        column_index: pane.column_index,
                        column_id: pane.column_id.clone(),
                    })),
                });
            }
            let body = pane_body_rect(&pane);
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
            let Some(cache) = self.panes.get(&pane.target) else {
                set_base_text(frame, body.y, body.x, "Starting Agent...", body.width);
                continue;
            };
            draw_terminal_snapshot(frame, body, &cache.snapshot);
            if focused {
                self.cursor_position = terminal_cursor_position(body, &cache.snapshot);
            }
        }
        self.draw_splitters(frame, app.active_workspace);
    }

    /// 绘制 row/column splitter，并生成高优先级鼠标拖动命中区。
    ///
    /// 适用场景：普通多 pane 布局。示例：两列之间显示青色竖线并可左右拖动。
    fn draw_splitters(&mut self, frame: &mut [RenderLine], workspace_index: usize) {
        if self.spotlight_mode || self.visible_panes.len() < 2 {
            return;
        }
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
                for y in left.rect.y..left.rect.y.saturating_add(left.rect.height) {
                    styled_overlay_text(frame, y, x, "│", (34, 211, 238), (11, 16, 22), false);
                }
                self.splitter_regions.push(SplitterRegion {
                    rect: TuiRect {
                        x,
                        y: left.rect.y,
                        width: TUI_SPLITTER_SIZE,
                        height: left.rect.height,
                    },
                    target: SplitterTarget::Columns {
                        workspace_index,
                        row_index: left.row_index,
                        left_index: left.column_index,
                        right_index: right.column_index,
                        content_width,
                    },
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
            let (bottom_index, _, _) = pair[1];
            let y = top_y.saturating_add(top_height);
            let width = body_right.saturating_sub(body_x);
            styled_overlay_text(
                frame,
                y,
                body_x,
                &"─".repeat(usize::from(width)),
                (34, 211, 238),
                (11, 16, 22),
                false,
            );
            self.splitter_regions.push(SplitterRegion {
                rect: TuiRect {
                    x: body_x,
                    y,
                    width,
                    height: TUI_SPLITTER_SIZE,
                },
                target: SplitterTarget::Rows {
                    workspace_index,
                    top_index,
                    bottom_index,
                    content_height,
                },
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
        }
    }
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

/// 校验并把通用表单转换成业务动作。
///
/// 适用场景：Enter。示例：空 Agent 名 -> Err。
fn submit_form(form: &FormDialog) -> std::result::Result<TuiAppAction, String> {
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
    }
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

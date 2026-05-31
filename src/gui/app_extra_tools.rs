//! Agent 主界面的外置脚本工具。
//!
//! 本模块只维护外置工具的扫描、刷新、执行和 modal 绘制。所有慢 IO 都
//! 通过后台 runtime 完成，结果再回到唯一 AppEvent 队列。

use super::*;
use tokio::io::AsyncReadExt;
use tokio::sync::oneshot;

const EXTRA_TOOL_CARD_WIDTH: f32 = 220.0;
const EXTRA_TOOL_CARD_BASE_HEIGHT: f32 = 126.0;
const EXTRA_TOOL_INPUT_ROW_HEIGHT: f32 = 24.0;
const EXTRA_TOOL_MAX_VISIBLE_INPUT_ROWS: usize = 4;
const EXTRA_TOOL_SWITCH_MIN_WIDTH: f32 = 68.0;
const EXTRA_TOOL_SWITCH_MAX_WIDTH: f32 = 220.0;
const EXTRA_TOOL_SWITCH_HEIGHT: f32 = 32.0;

/// 外置工具脚本所在区块。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum ExtraToolScope {
    /// 用户目录全局脚本。
    Global,
    /// 当前 workspace 根目录脚本。
    Workspace,
}

impl ExtraToolScope {
    /// 返回 UI 分区标题。
    fn title(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Workspace => "workspace",
        }
    }
}

/// 外置工具 card 的稳定标识。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ExtraToolKey {
    /// 脚本来自全局目录还是 workspace 根目录。
    pub(super) scope: ExtraToolScope,
    /// 脚本绝对路径，用于区分同名脚本。
    pub(super) path: PathBuf,
}

/// 脚本 metadata 子命令输出的协议结构。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ExtraToolMetadata {
    /// 工具展示类型，默认按 card 处理。
    pub(super) tool_type: ExtraToolType,
    /// Card 上要展示的 action 按钮，同时也是 `_gsdv_action` 值。
    pub(super) actions: Vec<String>,
    /// action 执行时是否需要把输入框内容传入 `__gsdv_input`。
    pub(super) input_need: bool,
    /// 首次加载 card 时填入 input 的默认值。
    pub(super) input_value: String,
    /// input 多行文本框的期望行数，默认一行。
    pub(super) input_rows: usize,
    /// value 自动刷新间隔，单位秒；小于等于 0 表示不自动刷新。
    pub(super) refresh: f64,
}

/// 外置工具在抽屉中的展示类型。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ExtraToolType {
    /// 原有卡片式工具。
    #[default]
    Card,
    /// 布尔开关工具。
    Switch,
}

impl ExtraToolType {
    /// 从 metadata 文本值解析工具类型。
    fn parse(value: &str) -> Result<Self, String> {
        match value.trim() {
            "" | "card" => Ok(Self::Card),
            "switch" => Ok(Self::Switch),
            other => Err(format!("metadata type parse failed: `{other}`")),
        }
    }
}

/// 扫描完成后可直接合并到 UI 状态的脚本信息。
#[derive(Debug)]
pub(super) struct ExtraToolsScanResult {
    /// 全局脚本 card 列表。
    pub(super) global: Vec<ExtraToolCard>,
    /// 当前 workspace 根目录脚本 card 列表。
    pub(super) workspace: Vec<ExtraToolCard>,
}

/// 单个脚本 card 的 UI 状态。
#[derive(Debug)]
pub(super) struct ExtraToolCard {
    /// Card 的稳定标识。
    pub(super) key: ExtraToolKey,
    /// UI 上显示的脚本文件名。
    pub(super) label: String,
    /// 脚本绝对路径。
    pub(super) path: PathBuf,
    /// 脚本执行时使用的工作目录。
    pub(super) workdir: PathBuf,
    /// 最近一次成功解析到的 metadata。
    pub(super) metadata: Option<ExtraToolMetadata>,
    /// Card 当前展示的 value stdout。
    pub(super) value: String,
    /// 用户在 card 输入框中编辑的文本。
    pub(super) input: String,
    /// 最近一次 metadata/value/action 错误。
    pub(super) error: Option<String>,
    /// value 刷新后台任务是否正在运行。
    pub(super) value_in_flight: bool,
    /// action 后台任务是否正在运行。
    pub(super) processing: bool,
    /// 当前正在执行的 action 名称。
    pub(super) running_action: Option<String>,
    /// 当前 action 的中断控制柄。
    pub(super) action_control: Option<ExtraToolActionControl>,
    /// 下一次 value 自动刷新时间。
    pub(super) next_value_refresh_at: Option<Instant>,
}

impl ExtraToolCard {
    /// 创建一个已加载 metadata 和 value 的 card。
    fn loaded(
        key: ExtraToolKey,
        label: String,
        path: PathBuf,
        workdir: PathBuf,
        metadata: Option<ExtraToolMetadata>,
        value: String,
        error: Option<String>,
        now: Instant,
    ) -> Self {
        let next_value_refresh_at = metadata
            .as_ref()
            .and_then(|metadata| extra_tool_next_refresh(metadata, now));
        let input = metadata
            .as_ref()
            .map(|metadata| metadata.input_value.clone())
            .unwrap_or_default();
        Self {
            key,
            label,
            path,
            workdir,
            metadata,
            value,
            input,
            error,
            value_in_flight: false,
            processing: false,
            running_action: None,
            action_control: None,
            next_value_refresh_at,
        }
    }
}

/// 单个 action 的中断控制柄。
pub(super) struct ExtraToolActionControl {
    /// 发给后台 action loop 的中断信号。
    cancel_tx: oneshot::Sender<()>,
}

impl ExtraToolActionControl {
    /// 请求中断当前 action，适用于用户点击 card 上的中断按钮。
    fn interrupt(self) {
        let _ = self.cancel_tx.send(());
    }
}

impl std::fmt::Debug for ExtraToolActionControl {
    /// 避免 Debug 输出暴露 oneshot 内部实现。
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ExtraToolActionControl")
    }
}

/// action 执行完成后回到 UI 线程的结果。
#[derive(Debug)]
pub(super) struct ExtraToolActionResult {
    /// action stdout 文本。
    pub(super) stdout: String,
    /// action stderr 文本。
    pub(super) stderr: String,
    /// 进程退出码；被信号终止时为空。
    pub(super) status_code: Option<i32>,
    /// 进程状态的人类可读描述。
    pub(super) status_text: String,
    /// action 是否被用户请求中断。
    pub(super) interrupted: bool,
    /// 启动、等待或读管道错误。
    pub(super) error: Option<String>,
}

impl ExtraToolActionResult {
    /// 返回 action 是否按协议成功结束。
    fn succeeded(&self) -> bool {
        self.error.is_none() && !self.interrupted && self.status_code == Some(0)
    }
}

/// 外置工具 modal 和后台任务的总体状态。
pub(super) struct ExtraToolsState {
    /// modal 是否打开。
    pub(super) open: bool,
    /// 全局脚本 cards。
    pub(super) global: Vec<ExtraToolCard>,
    /// 当前 workspace 根目录脚本 cards。
    pub(super) workspace: Vec<ExtraToolCard>,
    /// 目录扫描后台任务是否正在运行。
    pub(super) scan_in_flight: bool,
    /// 下一次全量扫描时间。
    pub(super) next_scan_at: Instant,
    /// 最近一次扫描对应的 workspace root。
    pub(super) scanned_workspace_path: Option<PathBuf>,
}

impl ExtraToolsState {
    /// 创建新 app 会话中的外置工具状态。
    pub(super) fn new(now: Instant) -> Self {
        Self {
            open: false,
            global: Vec::new(),
            workspace: Vec::new(),
            scan_in_flight: false,
            next_scan_at: now,
            scanned_workspace_path: None,
        }
    }
}

impl GsdvGuiApp {
    /// 切换外置工具 modal，只允许在 Agent 主界面打开。
    pub(super) fn toggle_extra_tools(&mut self, ctx: &egui::Context) {
        if self.extra_tools.open {
            self.close_extra_tools();
            return;
        }
        if !self.extra_tools_shortcut_context_allowed() {
            return;
        }
        self.extra_tools.open = true;
        self.mark_extra_tools_scan_due();
        self.request_app_repaint(ctx);
    }

    /// 关闭外置工具 modal。
    pub(super) fn close_extra_tools(&mut self) {
        self.extra_tools.open = false;
    }

    /// 标记下一帧需要重新扫描外置工具。
    pub(super) fn mark_extra_tools_scan_due(&mut self) {
        self.extra_tools.next_scan_at = Instant::now();
    }

    /// 返回外置工具是否有到期事件需要进入 AppEvent。
    pub(super) fn extra_tools_event_due(&self) -> bool {
        self.extra_tools_scan_due() || self.extra_tools_value_refresh_due()
    }

    /// 返回下一次外置工具后台工作需要唤醒 UI 的延迟。
    pub(super) fn next_extra_tools_delay(&self) -> Option<Duration> {
        let mut next = Some(
            self.extra_tools
                .next_scan_at
                .saturating_duration_since(Instant::now()),
        );
        for card in self.extra_tool_cards() {
            if card.processing || card.value_in_flight {
                continue;
            }
            if let Some(due_at) = card.next_value_refresh_at {
                next = min_optional_duration(
                    next,
                    Some(due_at.saturating_duration_since(Instant::now())),
                );
            }
        }
        next
    }

    /// 派发外置工具扫描和 value 刷新任务。
    pub(super) fn process_extra_tools(&mut self, ctx: &egui::Context) {
        if self.extra_tools_scan_due() {
            self.spawn_extra_tools_scan_task(ctx);
        }
        self.spawn_due_extra_tool_value_tasks(ctx);
    }

    /// 合并外置工具扫描结果。
    pub(super) fn apply_extra_tools_scan_result(
        &mut self,
        ctx: &egui::Context,
        workspace_path: Option<PathBuf>,
        result: Result<ExtraToolsScanResult, String>,
    ) {
        self.extra_tools.scan_in_flight = false;
        let now = Instant::now();
        self.extra_tools.next_scan_at = now + EXTRA_TOOLS_SCAN_INTERVAL;
        self.extra_tools.scanned_workspace_path = workspace_path;
        match result {
            Ok(result) => {
                let notifications = extra_tool_scan_error_lines(&result);
                Self::merge_extra_tool_cards(&mut self.extra_tools.global, result.global);
                Self::merge_extra_tool_cards(&mut self.extra_tools.workspace, result.workspace);
                for notification in notifications {
                    self.push_notification_line(notification);
                }
            }
            Err(error) => {
                self.push_notification_line(format!("[extra-tools] scan failed: {error}"));
                self.push_toast(
                    i18n::text(self.app_language, "Extra tools scan failed"),
                    theme::danger(),
                );
            }
        }
        self.request_app_repaint(ctx);
    }

    /// 合并单个 value 刷新结果。
    pub(super) fn apply_extra_tool_value_result(
        &mut self,
        ctx: &egui::Context,
        key: ExtraToolKey,
        result: Result<String, String>,
    ) {
        let mut notification = None;
        if let Some(card) = self.extra_tool_card_mut(&key) {
            card.value_in_flight = false;
            match result {
                Ok(value) => {
                    card.value = value;
                    card.error = None;
                    card.next_value_refresh_at = card
                        .metadata
                        .as_ref()
                        .and_then(|metadata| extra_tool_next_refresh(metadata, Instant::now()));
                }
                Err(error) => {
                    card.error = Some(error.clone());
                    notification = Some(format!(
                        "[extra-tools] value failed for {}: {error}",
                        card.label
                    ));
                    card.next_value_refresh_at = card
                        .metadata
                        .as_ref()
                        .and_then(|metadata| extra_tool_next_refresh(metadata, Instant::now()));
                }
            }
        }
        if let Some(notification) = notification {
            self.push_notification_line(notification);
        }
        self.request_app_repaint(ctx);
    }

    /// 合并 action 执行结果并触发一次 value 刷新。
    pub(super) fn apply_extra_tool_action_result(
        &mut self,
        ctx: &egui::Context,
        key: ExtraToolKey,
        action: String,
        result: ExtraToolActionResult,
    ) {
        let mut refresh_key = None;
        let mut label = None;
        if let Some(card) = self.extra_tool_card_mut(&key) {
            card.processing = false;
            card.running_action = None;
            card.action_control = None;
            card.error = result.error.clone();
            card.next_value_refresh_at = Some(Instant::now());
            refresh_key = Some(card.key.clone());
            label = Some(card.label.clone());
        }
        let label = label.unwrap_or_else(|| key.path.display().to_string());
        self.push_extra_tool_action_notifications(&label, &action, &result);
        if let Some(refresh_key) = refresh_key {
            self.spawn_extra_tool_value_task(ctx, refresh_key);
        }
        self.request_app_repaint(ctx);
    }

    /// 绘制外置工具右侧抽屉。
    pub(super) fn extra_tools_dialog(&mut self, ctx: &egui::Context) {
        if !self.extra_tools.open {
            return;
        }
        let mut action_request = None;
        let mut interrupt_request = None;
        let mut close_requested = false;
        let screen = ctx.screen_rect();
        let rail_width = if self.rail_collapsed {
            COMPACT_WORKSPACE_RAIL_WIDTH
        } else {
            WORKSPACE_RAIL_WIDTH
        }
        .min(screen.width());
        let width = (screen.width() - rail_width).max(1.0);
        let height = (screen.height() - BOTTOM_BAR_HEIGHT).max(1.0);
        let pos = egui::pos2(screen.right() - width, screen.top());
        let size = Vec2::new(width, height);

        egui::Area::new("extra_tools_overlay".into())
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(size, Sense::click_and_drag());
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    ui.set_min_size(size);
                    ui.set_max_size(size);
                    Frame::new()
                        .fill(theme::bg())
                        .stroke(Stroke::new(1.0, theme::border()))
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.set_min_size(Vec2::new(
                                (size.x - 20.0).max(1.0),
                                (size.y - 20.0).max(1.0),
                            ));
                            extra_tools_drawer_ui(
                                ui,
                                &mut self.extra_tools,
                                &mut action_request,
                                &mut interrupt_request,
                                &mut close_requested,
                                self.app_language,
                            );
                        });
                });
            });
        if close_requested {
            self.queue_app_event(AppEvent::InputUiCommand(UiCommand::ToggleExtraTools));
        }
        if let Some(key) = interrupt_request {
            self.queue_app_event(AppEvent::ExtraToolInterruptRequested { key });
        }
        if let Some((key, action)) = action_request {
            self.queue_app_event(AppEvent::ExtraToolActionRequested { key, action });
        }
    }

    /// 返回当前快捷键上下文是否允许打开外置工具。
    fn extra_tools_shortcut_context_allowed(&self) -> bool {
        self.active_app_dialog().is_none()
            && self.active_reviewer_dialog().is_none()
            && !self.notifications.open
            && !self.workspace_terminal_drawer_is_open()
            && !self.reviewer_helix_drawer_is_open()
            && self.current_workspace().is_some_and(|workspace| {
                workspace.route == Route::Workspace && workspace.center_mode == CenterMode::Agent
            })
    }

    /// 判断是否需要发起目录扫描。
    fn extra_tools_scan_due(&self) -> bool {
        if self.extra_tools.scan_in_flight {
            return false;
        }
        let workspace_path = self
            .current_workspace()
            .map(|workspace| workspace.path.clone());
        self.extra_tools.scanned_workspace_path != workspace_path
            || Instant::now() >= self.extra_tools.next_scan_at
    }

    /// 判断是否有 card 的 value 刷新已到期。
    fn extra_tools_value_refresh_due(&self) -> bool {
        let now = Instant::now();
        self.extra_tool_cards().any(|card| {
            !card.processing
                && !card.value_in_flight
                && card
                    .next_value_refresh_at
                    .is_some_and(|due_at| now >= due_at)
        })
    }

    /// 迭代当前所有外置工具 card。
    fn extra_tool_cards(&self) -> impl Iterator<Item = &ExtraToolCard> {
        self.extra_tools
            .global
            .iter()
            .chain(self.extra_tools.workspace.iter())
    }

    /// 查找可修改的外置工具 card。
    fn extra_tool_card_mut(&mut self, key: &ExtraToolKey) -> Option<&mut ExtraToolCard> {
        self.extra_tools
            .global
            .iter_mut()
            .chain(self.extra_tools.workspace.iter_mut())
            .find(|card| &card.key == key)
    }

    /// 启动目录扫描后台任务。
    fn spawn_extra_tools_scan_task(&mut self, ctx: &egui::Context) {
        if self.extra_tools.scan_in_flight {
            return;
        }
        self.extra_tools.scan_in_flight = true;
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_after = self.max_repaint_interval();
        let network_settings = self.network_settings.clone();
        let workspace_path = self
            .current_workspace()
            .map(|workspace| workspace.path.clone());
        let workspace_path_for_event = workspace_path.clone();
        self.background_runtime.spawn(async move {
            let result = scan_extra_tools(workspace_path, network_settings).await;
            let _ = tx.send(AppEvent::ExtraToolsScanned {
                workspace_path: workspace_path_for_event,
                result,
            });
            repaint_ctx.request_repaint_after(repaint_after);
        });
        self.request_app_repaint(ctx);
    }

    /// 启动所有到期 value 刷新任务。
    fn spawn_due_extra_tool_value_tasks(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let keys = self
            .extra_tool_cards()
            .filter(|card| {
                !card.processing
                    && !card.value_in_flight
                    && card
                        .next_value_refresh_at
                        .is_some_and(|due_at| now >= due_at)
            })
            .map(|card| card.key.clone())
            .collect::<Vec<_>>();
        for key in keys {
            self.spawn_extra_tool_value_task(ctx, key);
        }
    }

    /// 启动单个 value 刷新任务。
    fn spawn_extra_tool_value_task(&mut self, ctx: &egui::Context, key: ExtraToolKey) {
        let Some(card) = self.extra_tool_card_mut(&key) else {
            return;
        };
        if card.value_in_flight {
            return;
        }
        card.value_in_flight = true;
        card.next_value_refresh_at = None;
        let script = ExtraToolScriptRef::from_card(card);
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_after = self.max_repaint_interval();
        let network_settings = self.network_settings.clone();
        let workspace_path = self
            .current_workspace()
            .map(|workspace| workspace.path.clone());
        self.background_runtime.spawn(async move {
            let result =
                run_extra_tool_value(&script, workspace_path.as_deref(), &network_settings).await;
            let _ = tx.send(AppEvent::ExtraToolValueLoaded {
                key: script.key,
                result,
            });
            repaint_ctx.request_repaint_after(repaint_after);
        });
        self.request_app_repaint(ctx);
    }

    /// 启动单个 action 后台任务。
    pub(super) fn start_extra_tool_action(
        &mut self,
        ctx: &egui::Context,
        key: ExtraToolKey,
        action: String,
    ) {
        let workspace_path = self
            .current_workspace()
            .map(|workspace| workspace.path.clone());
        let Some(card) = self.extra_tool_card_mut(&key) else {
            return;
        };
        if card.processing {
            return;
        }
        let (cancel_tx, cancel_rx) = oneshot::channel();
        card.processing = true;
        card.running_action = Some(action.clone());
        card.action_control = Some(ExtraToolActionControl { cancel_tx });
        card.error = None;
        let input = if card
            .metadata
            .as_ref()
            .is_some_and(|metadata| metadata.input_need)
        {
            Some(card.input.clone())
        } else {
            None
        };
        let script = ExtraToolScriptRef::from_card(card);
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_after = self.max_repaint_interval();
        let network_settings = self.network_settings.clone();
        self.background_runtime.spawn(async move {
            let result = run_extra_tool_action(
                &script,
                &action,
                input.as_deref(),
                workspace_path.as_deref(),
                &network_settings,
                cancel_rx,
            )
            .await;
            let _ = tx.send(AppEvent::ExtraToolActionFinished {
                key: script.key,
                action,
                result,
            });
            repaint_ctx.request_repaint_after(repaint_after);
        });
        self.request_app_repaint(ctx);
    }

    /// 请求中断当前 action。
    pub(super) fn interrupt_extra_tool_action(&mut self, ctx: &egui::Context, key: ExtraToolKey) {
        let mut label = None;
        if let Some(card) = self.extra_tool_card_mut(&key) {
            label = Some(card.label.clone());
            if let Some(control) = card.action_control.take() {
                control.interrupt();
            }
        }
        if let Some(label) = label {
            self.push_notification_line(format!("[extra-tools] interrupt requested for {label}"));
        }
        self.request_app_repaint(ctx);
    }

    /// 保留用户输入和运行状态后合并扫描结果。
    fn merge_extra_tool_cards(current: &mut Vec<ExtraToolCard>, scanned: Vec<ExtraToolCard>) {
        let mut previous = current
            .drain(..)
            .map(|card| (card.key.clone(), card))
            .collect::<BTreeMap<_, _>>();
        *current = scanned
            .into_iter()
            .map(|mut card| {
                if let Some(mut old) = previous.remove(&card.key) {
                    card.input = old.input;
                    card.processing = old.processing;
                    card.running_action = old.running_action.take();
                    card.action_control = old.action_control.take();
                    if old.processing {
                        card.value = old.value;
                    }
                }
                card
            })
            .collect();
    }

    /// 将 action stdout/stderr/退出状态写入通知栏。
    fn push_extra_tool_action_notifications(
        &mut self,
        label: &str,
        action: &str,
        result: &ExtraToolActionResult,
    ) {
        if !result.stdout.trim().is_empty() {
            self.push_notification_block("[extra-tools stdout]", label, action, &result.stdout);
        }
        if !result.stderr.trim().is_empty() {
            self.push_notification_block("[extra-tools stderr]", label, action, &result.stderr);
        }
        let status = if result.succeeded() {
            "success"
        } else if result.interrupted {
            "interrupted"
        } else {
            "failed"
        };
        let detail = result
            .error
            .as_deref()
            .unwrap_or(result.status_text.as_str());
        self.push_notification_line(format!(
            "[extra-tools] {label} action {action}: {status} ({detail})"
        ));
        if result.succeeded() {
            self.push_toast(format!("{label}: {action} done"), theme::success());
        } else {
            self.push_toast(format!("{label}: {action} failed"), theme::danger());
        }
    }

    /// 按行写入多行通知，避免超长文本挤在一行。
    fn push_notification_block(&mut self, prefix: &str, label: &str, action: &str, text: &str) {
        for line in text.lines() {
            self.push_notification_line(format!("{prefix} {label} {action}: {line}"));
        }
    }
}

/// 脚本后台任务需要的不可变引用数据。
#[derive(Clone)]
struct ExtraToolScriptRef {
    /// Card 的稳定标识。
    key: ExtraToolKey,
    /// 脚本绝对路径。
    path: PathBuf,
    /// 执行脚本时使用的工作目录。
    workdir: PathBuf,
}

impl ExtraToolScriptRef {
    /// 从 UI card 拷贝后台任务所需字段。
    fn from_card(card: &ExtraToolCard) -> Self {
        Self {
            key: card.key.clone(),
            path: card.path.clone(),
            workdir: card.workdir.clone(),
        }
    }
}

/// 绘制右侧抽屉里的外置工具内容。
fn extra_tools_drawer_ui(
    ui: &mut Ui,
    state: &mut ExtraToolsState,
    action_request: &mut Option<(ExtraToolKey, String)>,
    interrupt_request: &mut Option<ExtraToolKey>,
    close_requested: &mut bool,
    language: AppLanguage,
) {
    let _ = close_requested;
    let layout = extra_tool_layout(state);
    ui.vertical(|ui| {
        ScrollArea::vertical()
            .id_salt("extra-tools-scroll")
            .show(ui, |ui| {
                extra_tool_section_ui(
                    ui,
                    ExtraToolScope::Global,
                    &mut state.global,
                    layout,
                    action_request,
                    interrupt_request,
                    language,
                );
                ui.add_space(18.0);
                extra_tool_section_ui(
                    ui,
                    ExtraToolScope::Workspace,
                    &mut state.workspace,
                    layout,
                    action_request,
                    interrupt_request,
                    language,
                );
            });
    });
}

/// 外置工具 card 网格的统一尺寸。
#[derive(Clone, Copy)]
struct ExtraToolLayout {
    /// 每个 card 的宽度。
    card_width: f32,
    /// 每个 card 的统一高度。
    card_height: f32,
    /// 多行 input 的可见高度。
    input_height: f32,
}

/// 计算所有分区共享的 card 尺寸。
fn extra_tool_layout(state: &ExtraToolsState) -> ExtraToolLayout {
    let max_rows = state
        .global
        .iter()
        .chain(state.workspace.iter())
        .filter_map(|card| {
            card.metadata
                .as_ref()
                .filter(|metadata| metadata.tool_type == ExtraToolType::Card && metadata.input_need)
                .map(|metadata| metadata.input_rows.max(1))
        })
        .max()
        .unwrap_or(1);
    let visible_rows = max_rows.min(EXTRA_TOOL_MAX_VISIBLE_INPUT_ROWS).max(1);
    let input_height = visible_rows as f32 * EXTRA_TOOL_INPUT_ROW_HEIGHT;
    ExtraToolLayout {
        card_width: EXTRA_TOOL_CARD_WIDTH,
        card_height: EXTRA_TOOL_CARD_BASE_HEIGHT + input_height,
        input_height,
    }
}

/// 生成扫描阶段 metadata/value 失败的通知行。
fn extra_tool_scan_error_lines(result: &ExtraToolsScanResult) -> Vec<String> {
    result
        .global
        .iter()
        .chain(result.workspace.iter())
        .filter_map(|card| {
            card.error.as_ref().map(|error| {
                format!(
                    "[extra-tools] load failed for {}: {error}",
                    card.path.display()
                )
            })
        })
        .collect()
}

/// 绘制一个外置工具分区。
fn extra_tool_section_ui(
    ui: &mut Ui,
    scope: ExtraToolScope,
    cards: &mut [ExtraToolCard],
    layout: ExtraToolLayout,
    action_request: &mut Option<(ExtraToolKey, String)>,
    interrupt_request: &mut Option<ExtraToolKey>,
    language: AppLanguage,
) {
    ui.label(section_label(i18n::text(language, scope.title())));
    ui.add_space(6.0);
    if cards.is_empty() {
        ui.label(muted(i18n::text(language, "No scripts")));
        return;
    }
    let gap = 10.0;
    extra_tool_switch_grid_ui(ui, cards, action_request, language);
    if cards.iter().any(extra_tool_is_switch) && cards.iter().any(extra_tool_is_card) {
        ui.add_space(10.0);
    }
    let card_count = cards.iter().filter(|card| extra_tool_is_card(card)).count();
    if card_count == 0 {
        return;
    }
    let columns = ((ui.available_width() + gap) / (layout.card_width + gap))
        .floor()
        .max(1.0) as usize;
    egui::Grid::new(format!("extra-tools-grid-{:?}", scope))
        .num_columns(columns)
        .spacing(Vec2::new(gap, gap))
        .show(ui, |ui| {
            let mut visible_index = 0;
            for card in cards.iter_mut().filter(|card| extra_tool_is_card(card)) {
                extra_tool_card_ui(
                    ui,
                    layout,
                    card,
                    action_request,
                    interrupt_request,
                    language,
                );
                visible_index += 1;
                if visible_index % columns == 0 {
                    ui.end_row();
                }
            }
        });
}

/// 绘制 switch 工具网格。
fn extra_tool_switch_grid_ui(
    ui: &mut Ui,
    cards: &mut [ExtraToolCard],
    action_request: &mut Option<(ExtraToolKey, String)>,
    language: AppLanguage,
) {
    let switches = cards
        .iter()
        .filter(|card| extra_tool_is_switch(card))
        .count();
    if switches == 0 {
        return;
    }
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
        ui.horizontal_wrapped(|ui| {
            for card in cards.iter_mut().filter(|card| extra_tool_is_switch(card)) {
                extra_tool_switch_ui(ui, card, action_request, language);
            }
        });
    });
}

/// 返回工具是否为 card 展示。
fn extra_tool_is_card(card: &ExtraToolCard) -> bool {
    card.metadata
        .as_ref()
        .map(|metadata| metadata.tool_type == ExtraToolType::Card)
        .unwrap_or(true)
}

/// 返回工具是否为 switch 展示。
fn extra_tool_is_switch(card: &ExtraToolCard) -> bool {
    card.metadata
        .as_ref()
        .is_some_and(|metadata| metadata.tool_type == ExtraToolType::Switch)
}

/// 绘制单个 switch 工具。
fn extra_tool_switch_ui(
    ui: &mut Ui,
    card: &mut ExtraToolCard,
    action_request: &mut Option<(ExtraToolKey, String)>,
    language: AppLanguage,
) {
    let current = extra_tool_switch_value(card);
    let enabled = current.is_some() && !card.processing && !card.value_in_flight;
    ui.push_id((&card.key.scope, &card.key.path), |ui| {
        let width = extra_tool_switch_width(ui, &card.label);
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, EXTRA_TOOL_SWITCH_HEIGHT), Sense::click());
        let mut response = if enabled {
            response.on_hover_cursor(egui::CursorIcon::PointingHand)
        } else {
            response
        };
        if card.processing {
            response = response.on_hover_text(i18n::text(language, "processing..."));
        } else if card.value_in_flight {
            response = response.on_hover_text(i18n::text(language, "loading..."));
        } else if current.is_none() {
            response = response.on_hover_text(i18n::text(language, "value must be true or false"));
        }
        if response.clicked() && enabled {
            let next = (!current.unwrap_or(false)).to_string();
            *action_request = Some((card.key.clone(), next));
        }

        let (mut fill, stroke, text) = extra_tool_switch_visuals(card, current);
        if response.hovered() && enabled && current != Some(true) {
            fill = theme::hover();
        }
        ui.painter().rect(
            rect,
            CornerRadius::same(theme::RADIUS_MD),
            fill,
            Stroke::new(1.0, stroke),
            egui::StrokeKind::Outside,
        );

        let text_rect = rect.shrink2(Vec2::new(12.0, 0.0));
        ui.painter().with_clip_rect(text_rect).text(
            egui::pos2(text_rect.left(), text_rect.center().y),
            Align2::LEFT_CENTER,
            &card.label,
            egui::TextStyle::Button.resolve(ui.style()),
            text,
        );
    });
}

/// 根据脚本名估算 switch chip 宽度，避免单个开关撑成大卡片。
fn extra_tool_switch_width(ui: &Ui, label: &str) -> f32 {
    let font_id = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), font_id, theme::text());
    (galley.rect.width() + 24.0).clamp(EXTRA_TOOL_SWITCH_MIN_WIDTH, EXTRA_TOOL_SWITCH_MAX_WIDTH)
}

/// 返回 switch 当前状态对应的颜色。
fn extra_tool_switch_visuals(
    card: &ExtraToolCard,
    current: Option<bool>,
) -> (Color32, Color32, Color32) {
    if card.processing || card.value_in_flight {
        return (
            theme::accent_soft(theme::warning()),
            theme::accent_border(theme::warning()),
            theme::warning(),
        );
    }
    if current.is_none() {
        return (
            theme::danger_soft(),
            theme::danger_border(),
            theme::danger(),
        );
    }
    if current == Some(true) {
        return (
            theme::accent_soft(theme::success()),
            theme::accent_border(theme::success()),
            theme::success(),
        );
    }
    (theme::surface_elevated(), theme::border(), theme::muted())
}

/// 从 value stdout 解析 switch 当前值。
fn extra_tool_switch_value(card: &ExtraToolCard) -> Option<bool> {
    match card.value.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// 绘制单个外置工具 card。
fn extra_tool_card_ui(
    ui: &mut Ui,
    layout: ExtraToolLayout,
    card: &mut ExtraToolCard,
    action_request: &mut Option<(ExtraToolKey, String)>,
    interrupt_request: &mut Option<ExtraToolKey>,
    language: AppLanguage,
) {
    let stroke_color = if card.error.is_some() {
        theme::danger_border()
    } else {
        theme::border()
    };
    let id_scope = card.key.scope;
    let id_path = card.key.path.clone();
    ui.push_id((id_scope, id_path), |ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(layout.card_width, layout.card_height),
            Layout::top_down(Align::LEFT),
            |ui| {
                Frame::new()
                    .fill(theme::surface_elevated())
                    .stroke(Stroke::new(1.0, stroke_color))
                    .corner_radius(CornerRadius::same(theme::RADIUS_MD))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_min_size(Vec2::new(
                            (layout.card_width - 20.0).max(1.0),
                            (layout.card_height - 20.0).max(1.0),
                        ));
                        ui.set_max_size(Vec2::new(
                            (layout.card_width - 20.0).max(1.0),
                            (layout.card_height - 20.0).max(1.0),
                        ));
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&card.label).strong().color(theme::text()));
                            ui.add_space(3.0);
                            extra_tool_value_ui(ui, card, language);
                            if card
                                .metadata
                                .as_ref()
                                .is_some_and(|metadata| metadata.input_need)
                            {
                                ui.add_space(3.0);
                                extra_tool_input_ui(ui, layout, card, language);
                            }
                            ui.add_space(8.0);
                            extra_tool_action_buttons(
                                ui,
                                card,
                                action_request,
                                interrupt_request,
                                language,
                            );
                        });
                    });
            },
        );
    });
}

/// 绘制 card 的 value 区域。
fn extra_tool_value_ui(ui: &mut Ui, card: &ExtraToolCard, language: AppLanguage) {
    if card.processing {
        ui.label(RichText::new(i18n::text(language, "processing...")).color(theme::warning()));
    } else if card.value_in_flight && card.value.is_empty() {
        ui.label(muted(i18n::text(language, "loading...")));
    } else {
        ui.add(
            egui::Label::new(
                RichText::new(if card.value.is_empty() {
                    "-"
                } else {
                    &card.value
                })
                .monospace()
                .color(theme::text()),
            )
            .wrap(),
        );
    }
    if let Some(error) = &card.error {
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(error).size(12.0).color(theme::danger())).wrap());
    }
}

/// 绘制可滚动的多行 input 区域。
fn extra_tool_input_ui(
    ui: &mut Ui,
    layout: ExtraToolLayout,
    card: &mut ExtraToolCard,
    language: AppLanguage,
) {
    let desired_rows = card
        .metadata
        .as_ref()
        .map(|metadata| metadata.input_rows.max(1))
        .unwrap_or(1);
    ScrollArea::vertical()
        .id_salt("extra-tool-input-scroll")
        .max_height(layout.input_height)
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut card.input)
                    .desired_rows(desired_rows)
                    .desired_width(layout.card_width - 20.0)
                    .hint_text(i18n::text(language, "input")),
            );
        });
}

/// 绘制单个 card 的 action 和中断按钮。
fn extra_tool_action_buttons(
    ui: &mut Ui,
    card: &mut ExtraToolCard,
    action_request: &mut Option<(ExtraToolKey, String)>,
    interrupt_request: &mut Option<ExtraToolKey>,
    language: AppLanguage,
) {
    ui.horizontal_wrapped(|ui| {
        let actions = card
            .metadata
            .as_ref()
            .map(|metadata| metadata.actions.clone())
            .unwrap_or_default();
        for action in actions {
            let response = ui.add_enabled(!card.processing, Button::new(action.as_str()));
            if response.clicked() {
                *action_request = Some((card.key.clone(), action));
            }
        }
        if card.processing
            && ui
                .add(Button::new(
                    RichText::new(i18n::text(language, "Interrupt")).color(theme::danger()),
                ))
                .clicked()
        {
            *interrupt_request = Some(card.key.clone());
        }
    });
}

/// 扫描全局和当前 workspace 外置工具。
async fn scan_extra_tools(
    workspace_path: Option<PathBuf>,
    network_settings: NetworkSettings,
) -> Result<ExtraToolsScanResult, String> {
    let now = Instant::now();
    let global_dir = extra_tools_global_dir();
    let global = scan_extra_tool_dir(
        ExtraToolScope::Global,
        &global_dir,
        workspace_path.as_deref(),
        &network_settings,
        now,
    )
    .await?;
    let workspace = if let Some(workspace_path) = workspace_path.as_deref() {
        scan_extra_tool_dir(
            ExtraToolScope::Workspace,
            workspace_path,
            Some(workspace_path),
            &network_settings,
            now,
        )
        .await?
    } else {
        Vec::new()
    };
    Ok(ExtraToolsScanResult { global, workspace })
}

/// 扫描一个目录下直属的 `.sh` 脚本。
async fn scan_extra_tool_dir(
    scope: ExtraToolScope,
    dir: &Path,
    workspace_path: Option<&Path>,
    network_settings: &NetworkSettings,
    now: Instant,
) -> Result<Vec<ExtraToolCard>, String> {
    let mut scripts = list_extra_tool_scripts(scope, dir)?;
    scripts.sort_by(|left, right| left.label.cmp(&right.label));
    let mut cards = Vec::new();
    for script in scripts {
        let loaded = load_extra_tool_card(script, workspace_path, network_settings, now).await;
        cards.push(loaded);
    }
    Ok(cards)
}

/// 从目录中列出直属 shell 脚本。
fn list_extra_tool_scripts(
    scope: ExtraToolScope,
    dir: &Path,
) -> Result<Vec<ExtraToolScriptEntry>, String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut scripts = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if !file_type.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("sh") {
            continue;
        }
        let Some(label) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        scripts.push(ExtraToolScriptEntry {
            key: ExtraToolKey {
                scope,
                path: path.clone(),
            },
            label: label.to_string(),
            path,
            workdir: dir.to_path_buf(),
        });
    }
    Ok(scripts)
}

/// 加载一个脚本的 metadata 和初始 value。
async fn load_extra_tool_card(
    script: ExtraToolScriptEntry,
    workspace_path: Option<&Path>,
    network_settings: &NetworkSettings,
    now: Instant,
) -> ExtraToolCard {
    match run_extra_tool_metadata(&script, workspace_path, network_settings).await {
        Ok(metadata) => {
            match run_extra_tool_value(&script.as_ref(), workspace_path, network_settings).await {
                Ok(value) => ExtraToolCard::loaded(
                    script.key,
                    script.label,
                    script.path,
                    script.workdir,
                    Some(metadata),
                    value,
                    None,
                    now,
                ),
                Err(error) => ExtraToolCard::loaded(
                    script.key,
                    script.label,
                    script.path,
                    script.workdir,
                    Some(metadata),
                    String::new(),
                    Some(error),
                    now,
                ),
            }
        }
        Err(error) => ExtraToolCard::loaded(
            script.key,
            script.label,
            script.path,
            script.workdir,
            None,
            String::new(),
            Some(error),
            now,
        ),
    }
}

/// 扫描阶段发现的脚本条目。
struct ExtraToolScriptEntry {
    /// Card 的稳定标识。
    key: ExtraToolKey,
    /// UI 上显示的脚本文件名。
    label: String,
    /// 脚本绝对路径。
    path: PathBuf,
    /// 脚本执行时使用的工作目录。
    workdir: PathBuf,
}

impl ExtraToolScriptEntry {
    /// 转成后台任务使用的不可变脚本引用。
    fn as_ref(&self) -> ExtraToolScriptRef {
        ExtraToolScriptRef {
            key: self.key.clone(),
            path: self.path.clone(),
            workdir: self.workdir.clone(),
        }
    }
}

/// 执行 metadata 子命令并解析 JSON。
async fn run_extra_tool_metadata(
    script: &ExtraToolScriptEntry,
    workspace_path: Option<&Path>,
    network_settings: &NetworkSettings,
) -> Result<ExtraToolMetadata, String> {
    let output = run_extra_tool_output(
        &script.path,
        &script.workdir,
        "metadata",
        workspace_path,
        network_settings,
        None,
    )
    .await?;
    parse_extra_tool_metadata(&output.stdout)
}

/// 解析外置工具 metadata 文本协议。
pub(super) fn parse_extra_tool_metadata(text: &str) -> Result<ExtraToolMetadata, String> {
    let mut actions = BTreeMap::new();
    let mut tool_type = ExtraToolType::Card;
    let mut input_need = false;
    let mut input_value = String::new();
    let mut input_rows = 1;
    let mut refresh = 0.0;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err(format!("metadata line missing ':' near `{line}`"));
        };
        let key = key.trim();
        let value = value.trim_start();
        if let Some(index) = extra_tool_action_key_index(key) {
            actions.insert(index, value.to_string());
        } else {
            match key {
                "type" => {
                    tool_type = ExtraToolType::parse(value)?;
                }
                "input_need" => {
                    input_need = parse_metadata_bool(value)?;
                }
                "input_value" => {
                    input_value = value.to_string();
                }
                "input_rows" => {
                    input_rows = parse_metadata_usize(value)?.max(1);
                }
                "refresh" => {
                    refresh = value
                        .trim()
                        .parse::<f64>()
                        .map_err(|error| format!("metadata refresh parse failed: {error}"))?;
                }
                _ => {}
            }
        }
    }
    Ok(ExtraToolMetadata {
        tool_type,
        actions: actions.into_values().collect(),
        input_need,
        input_value,
        input_rows,
        refresh,
    })
}

/// 解析 `action.N.key` 中的序号。
fn extra_tool_action_key_index(key: &str) -> Option<usize> {
    let rest = key.strip_prefix("action.")?;
    let index = rest.strip_suffix(".key")?;
    index.parse::<usize>().ok()
}

/// 解析 metadata 中的布尔值。
fn parse_metadata_bool(value: &str) -> Result<bool, String> {
    match value.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!("metadata bool parse failed: `{other}`")),
    }
}

/// 解析 metadata 中的正整数值。
fn parse_metadata_usize(value: &str) -> Result<usize, String> {
    value
        .trim()
        .parse::<usize>()
        .map_err(|error| format!("metadata usize parse failed: {error}"))
}

/// 执行 value 子命令并返回 stdout。
async fn run_extra_tool_value(
    script: &ExtraToolScriptRef,
    workspace_path: Option<&Path>,
    network_settings: &NetworkSettings,
) -> Result<String, String> {
    let output = run_extra_tool_output(
        &script.path,
        &script.workdir,
        "value",
        workspace_path,
        network_settings,
        None,
    )
    .await?;
    if output.status_code == Some(0) {
        Ok(output.stdout)
    } else {
        Err(format!(
            "value exited with {}; stderr: {}",
            output.status_text,
            output.stderr.trim()
        ))
    }
}

/// 执行 action 子命令并支持用户中断。
async fn run_extra_tool_action(
    script: &ExtraToolScriptRef,
    action: &str,
    input: Option<&str>,
    workspace_path: Option<&Path>,
    network_settings: &NetworkSettings,
    mut cancel_rx: oneshot::Receiver<()>,
) -> ExtraToolActionResult {
    let mut command = extra_tool_command(
        &script.path,
        &script.workdir,
        "action",
        workspace_path,
        network_settings,
    );
    command.env("_gsdv_action", action);
    if let Some(input) = input {
        command.env("__gsdv_input", input);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return ExtraToolActionResult {
                stdout: String::new(),
                stderr: String::new(),
                status_code: None,
                status_text: "spawn failed".to_string(),
                interrupted: false,
                error: Some(error.to_string()),
            };
        }
    };
    let stdout_task = child.stdout.take().map(read_process_pipe);
    let stderr_task = child.stderr.take().map(read_process_pipe);
    let mut interrupted = false;
    let wait_result = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(error) => break Err(error),
        }
        if !interrupted && cancel_rx.try_recv().is_ok() {
            interrupted = true;
            let _ = child.start_kill();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let stdout = read_joined_pipe(stdout_task).await;
    let stderr = read_joined_pipe(stderr_task).await;
    match wait_result {
        Ok(status) => ExtraToolActionResult {
            stdout: stdout.unwrap_or_default(),
            stderr: stderr.unwrap_or_default(),
            status_code: status.code(),
            status_text: status.to_string(),
            interrupted,
            error: None,
        },
        Err(error) => ExtraToolActionResult {
            stdout: stdout.unwrap_or_default(),
            stderr: stderr.unwrap_or_default(),
            status_code: None,
            status_text: "wait failed".to_string(),
            interrupted,
            error: Some(error.to_string()),
        },
    }
}

/// 执行 metadata/value 这类无需中断的子命令。
async fn run_extra_tool_output(
    path: &Path,
    workdir: &Path,
    command_name: &str,
    workspace_path: Option<&Path>,
    network_settings: &NetworkSettings,
    extra_env: Option<(&str, &str)>,
) -> Result<ExtraToolProcessOutput, String> {
    let mut command = extra_tool_command(
        path,
        workdir,
        command_name,
        workspace_path,
        network_settings,
    );
    if let Some((key, value)) = extra_env {
        command.env(key, value);
    }
    let output = command.output().await.map_err(|error| error.to_string())?;
    let result = ExtraToolProcessOutput {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        status_code: output.status.code(),
        status_text: output.status.to_string(),
    };
    if result.status_code == Some(0) {
        Ok(result)
    } else {
        Err(format!(
            "{command_name} exited with {}; stderr: {}",
            result.status_text,
            result.stderr.trim()
        ))
    }
}

/// 构造外置工具脚本命令。
fn extra_tool_command(
    path: &Path,
    workdir: &Path,
    command_name: &str,
    workspace_path: Option<&Path>,
    network_settings: &NetworkSettings,
) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("sh");
    command
        .arg(path)
        .arg(command_name)
        .current_dir(workdir)
        .envs(network_settings.env_vars())
        .env("GSDV_EXTRA_TOOL", path)
        .stdin(Stdio::null());
    if let Some(workspace_path) = workspace_path {
        command.env("GSDV_WORKSPACE_DIR", workspace_path);
    }
    command
}

/// 进程 stdout/stderr 的完整输出。
struct ExtraToolProcessOutput {
    /// stdout 文本。
    stdout: String,
    /// stderr 文本。
    stderr: String,
    /// 进程退出码；被信号终止时为空。
    status_code: Option<i32>,
    /// 进程状态的人类可读描述。
    status_text: String,
}

/// 异步读取一个进程管道。
fn read_process_pipe<R>(mut pipe: R) -> tokio::task::JoinHandle<Result<String, String>>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes)
            .await
            .map_err(|error| error.to_string())?;
        Ok(String::from_utf8_lossy(&bytes).to_string())
    })
}

/// 等待读管道任务结束并返回文本。
async fn read_joined_pipe(
    task: Option<tokio::task::JoinHandle<Result<String, String>>>,
) -> Option<String> {
    let task = task?;
    task.await.ok().and_then(Result::ok)
}

/// 返回全局外置工具目录。
fn extra_tools_global_dir() -> PathBuf {
    crate::home::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".gsdv/extra")
}

/// 根据 metadata 计算下一次 value 刷新时间。
fn extra_tool_next_refresh(metadata: &ExtraToolMetadata, now: Instant) -> Option<Instant> {
    if metadata.refresh <= 0.0 {
        return None;
    }
    Some(now + Duration::from_secs_f64(metadata.refresh))
}

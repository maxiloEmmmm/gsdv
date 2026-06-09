use crate::gui::agent::AgentLaunchConfig;
use crate::gui::data::{self, NetworkSettings, WorkspaceActivity, WorkspaceViewData};
use crate::gui::repaint_gate::RepaintController;
use crate::gui::theme as gui_theme;
use alacritty_terminal::event::{Event as PtyEvent, EventListener, Notify, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color as terminal_color_table;
use alacritty_terminal::term::{self, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{
    Color, CursorShape, NamedColor, Processor, Rgb, StdSyncHandler,
};
use anyhow::Result;
use eframe::egui::{
    self, Align2, Color32, CursorIcon, Event, FontFamily, FontId, Key, Modifiers, PointerButton,
    Pos2, Rect, Sense, Stroke, Ui, Vec2,
};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

static TERMINAL_ID: AtomicU64 = AtomicU64::new(1);
static TERMINAL_SPAWN_ENV_LOCK: Mutex<()> = Mutex::new(());
const TERMINAL_SCROLLBACK_HISTORY_LINES: usize = 2_000;
const TERMINAL_BELL_FLASH_DURATION: Duration = Duration::from_millis(180);
const TERMINAL_WAKEUP_FORWARD_INTERVAL: Duration = Duration::from_millis(100);
const TERMINAL_REPAINT_FORWARD_INTERVAL: Duration = Duration::from_millis(200);
const TERMINAL_TITLE_FORWARD_INTERVAL: Duration = Duration::from_millis(500);

// alacritty_terminal emits ColorRequest with its internal dynamic color indexes:
// 256 = default foreground, 257 = default background, 258 = cursor.
// The reply uses the xterm OSC dynamic color protocol: OSC 10/11/12 ; rgb:... ST.
const TERMINAL_FOREGROUND_COLOR_INDEX: usize = 256;
const TERMINAL_BACKGROUND_COLOR_INDEX: usize = 257;
const TERMINAL_CURSOR_COLOR_INDEX: usize = 258;
static UNHANDLED_PTY_EVENTS: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TerminalSurfaceKind {
    Agent,
    Workspace,
    Helix,
}

/// 返回 terminal host UI 计数标签，适用于区分当前可见 surface 类型。
fn terminal_host_ui_label(kind: TerminalSurfaceKind) -> &'static str {
    match kind {
        TerminalSurfaceKind::Agent => "terminal.host_ui.agent",
        TerminalSurfaceKind::Workspace => "terminal.host_ui.workspace",
        TerminalSurfaceKind::Helix => "terminal.host_ui.helix",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelixLaunchSpec {
    pub workdir: PathBuf,
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalFileLineClick {
    /// 终端输出里的文件路径，可能是绝对路径或相对 workspace 的路径。
    pub path: PathBuf,
    /// 一基索引目标行号。
    pub line: usize,
    /// 可选结束行号，来自 `file:start-end` 形式。
    pub end_line: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalOutputClick {
    /// 左键点中了尚未做文件系统分类的本地路径。
    PathCandidate(TerminalFileLineClick),
    /// 左键点中了 Agent 输出里的文件行引用。
    FileLine(TerminalFileLineClick),
    /// 左键点中了应交给文件管理器定位的本地路径。
    RevealPath(PathBuf),
    /// 左键点中了 Agent 输出里的 http/https URL。
    Url(String),
}

/// Terminal surface 一帧 UI 输出。
pub struct TerminalHostUiOutput {
    /// 实际 terminal widget 的交互响应。
    pub response: egui::Response,
    /// Agent 输入行位置，仅在调用方需要锚点时计算。
    pub input_rect: Option<Rect>,
    /// 本帧 terminal 输入字节是否提交了 Agent composer。
    pub input_submitted: bool,
    /// Agent terminal 输出里的可点击目标。
    pub output_click: Option<TerminalOutputClick>,
    /// Ctrl+点击只复制已解析目标，不执行打开动作。
    pub output_click_copy_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalInputShortcutScope {
    /// Agent 主 surface，需要保留全局 app 快捷键。
    AgentSurface,
    /// Workspace terminal 抽屉，只保留 terminal 自己拥有的快捷键。
    WorkspaceDrawerSurface,
    /// Helix 抽屉，需要额外保留 Helix 抽屉开关。
    HelixDrawerSurface,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TerminalFileLineMatch {
    /// 匹配到的文件路径。
    path: PathBuf,
    /// 一基索引目标行号。
    line: usize,
    /// 可选结束行号，Helix 打开时当前只定位起始行。
    end_line: Option<usize>,
    /// token 起始列。
    start_column: usize,
    /// token 结束列，开区间。
    end_column: usize,
    /// 可见终端行号。
    visible_line: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TerminalUrlMatch {
    /// 匹配到的完整 URL。
    url: String,
    /// URL 在当前可见行里的起始列。
    start_column: usize,
    /// URL 在当前可见行里的结束列。
    end_column: usize,
    /// URL 所在终端可见行。
    visible_line: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentProcessExit {
    pub command: String,
    pub status: String,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TerminalHostSummary {
    pub title: String,
    pub description: String,
    pub footer: String,
}

/// Terminal runtime 发给 AppEvent 的轻量通知。
///
/// 原始 PTY event 只能在 terminal runtime 内部消费；这里的事件只描述
/// UI 层需要知道的状态变化类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalRuntimeEvent {
    /// 产生事件的 terminal host id。
    pub id: u64,
    /// UI 层需要处理的粗粒度变化。
    pub kind: TerminalRuntimeEventKind,
}

/// Terminal runtime 对 UI 层暴露的粗粒度变化类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalRuntimeEventKind {
    /// 终端内容有新输出，用于刷新和 agent busy watchdog。
    Output,
    /// OSC title、退出状态等可见摘要状态发生变化。
    StateChanged,
    /// bell、光标闪烁、鼠标 cursor dirty 等只需要重绘的变化。
    Repaint,
}

/// Terminal runtime 向外投递轻量通知的函数。
pub type TerminalRuntimeEventSink = Arc<dyn Fn(TerminalRuntimeEvent) + Send + Sync + 'static>;

pub trait TerminalHost {
    fn kind(&self) -> TerminalSurfaceKind;
    fn workspace_root(&self) -> &Path;
    fn summary(&self) -> TerminalHostSummary;
}

pub struct GuiTerminalHost {
    kind: TerminalSurfaceKind,
    workspace_root: PathBuf,
    workspace_name: String,
    agent_title: String,
    session_id: Option<String>,
    activity: WorkspaceActivity,
    helix_spec: Option<HelixLaunchSpec>,
    launched_with_resume: bool,
    backend: TerminalBackend,
    /// terminal runtime 独立消费 PTY 后写入的轻量 UI 状态。
    runtime_state: Arc<Mutex<TerminalRuntimeState>>,
    /// 当前主题，供 terminal runtime 回复 OSC dynamic color query。
    runtime_theme_mode: Arc<Mutex<gui_theme::ThemeMode>>,
    font: FontId,
}

/// terminal runtime 写入、UI 绘制读取的共享状态。
#[derive(Debug, Default)]
struct TerminalRuntimeState {
    /// 子进程通过 OSC title 事件请求的最近标题。
    terminal_title: Option<String>,
    /// 终端 bell 闪烁效果持续到的时间。
    bell_flash_until: Option<Instant>,
    /// 子进程退出码。
    exited_status: Option<i32>,
    /// Agent 非零退出时留给 app 弹提示的摘要。
    abnormal_exit: Option<AgentProcessExit>,
    /// 隐藏终端收到但未能落到系统剪贴板的 OSC52 文本。
    pending_clipboard_text: Option<(term::ClipboardType, String)>,
}

/// 从 egui 布局和字体指标推导出的运行时终端尺寸。
#[derive(Debug, Clone, Copy, PartialEq)]
struct TerminalSize {
    /// 单个 cell 的 egui 物理点宽度。
    cell_width: f32,
    /// 单个 cell 的 egui 物理点高度。
    cell_height: f32,
    /// 当前字体是否允许把 ASCII 文本合并成一段绘制。
    ascii_text_runs: bool,
    /// 可见终端列数。
    cols: u16,
    /// 可见终端行数。
    lines: u16,
    /// 最近一次用于计算 grid 的 egui 布局尺寸。
    layout_size: Vec2,
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self {
            cell_width: 8.0,
            cell_height: 16.0,
            ascii_text_runs: false,
            cols: 80,
            lines: 24,
            layout_size: Vec2::ZERO,
        }
    }
}

impl From<TerminalSize> for WindowSize {
    fn from(size: TerminalSize) -> Self {
        Self {
            num_lines: size.lines,
            num_cols: size.cols,
            cell_width: size.cell_width.max(1.0).round() as u16,
            cell_height: size.cell_height.max(1.0).round() as u16,
        }
    }
}

/// 持有 PTY 事件循环，并向 egui 暴露借用渲染入口。
struct TerminalBackend {
    /// egui focus memory 使用的稳定 widget id。
    id: u64,
    /// PTY reader 线程更新的共享终端状态。
    term: Arc<FairMutex<Term<TerminalEventProxy>>>,
    /// 写入 alacritty event loop 的字节和 resize 消息发送端。
    notifier: Notifier,
    /// 最近一次发送给 PTY 和 emulator 的终端尺寸。
    size: TerminalSize,
    /// terminal runtime 回复 TextAreaSizeRequest 使用的最新尺寸。
    runtime_size: Arc<Mutex<TerminalSize>>,
    /// PTY 内容变化是否允许唤醒 egui repaint。
    repaint_enabled: Arc<AtomicBool>,
    /// 终端本地动画和 PTY 事件共用的 FPS 控制器。
    repaint_controller: RepaintController,
}

/// 将 alacritty 终端事件轻量转发到 UI 线程。
#[derive(Clone)]
struct TerminalEventProxy {
    /// 终端唯一 id，用于多终端路由。
    id: u64,
    /// 终端内部事件通道，只能由 terminal runtime 独立消费。
    tx: UnboundedSender<(u64, PtyEvent)>,
    /// egui context 只用于唤醒 immediate-mode render loop。
    egui_ctx: egui::Context,
    /// 终端输出唤醒 repaint 时使用的全局 FPS 控制器。
    repaint_controller: RepaintController,
    /// 当前终端是否允许 PTY 输出唤醒 repaint。
    ///
    /// repaint 只是唤醒 app update；状态变化由 terminal runtime 投 AppEvent。
    repaint_enabled: Arc<AtomicBool>,
    /// 高频 PTY 通知节流的毫秒起点。
    throttle_origin: Instant,
    /// 下一次允许转发 Wakeup 的毫秒刻度。
    next_wakeup_forward_ms: Arc<AtomicU64>,
    /// 下一次允许转发 repaint-only 事件的毫秒刻度。
    next_repaint_forward_ms: Arc<AtomicU64>,
    /// 下一次允许转发 title 状态事件的毫秒刻度。
    next_title_forward_ms: Arc<AtomicU64>,
}

impl EventListener for TerminalEventProxy {
    fn send_event(&self, event: PtyEvent) {
        if matches!(event, PtyEvent::Wakeup) {
            crate::gui::perf_log::count("terminal.proxy.wakeup");
            if !self.forward_due(
                &self.next_wakeup_forward_ms,
                TERMINAL_WAKEUP_FORWARD_INTERVAL,
            ) {
                crate::gui::perf_log::count("terminal.proxy.wakeup_coalesced");
                return;
            }
            // 触发条件：PTY 读线程解析到新的终端内容。
            // 不能只 repaint：watchdog 需要看到输出事件本身。
            // 防止后台 Agent 正常输出时被误判为卡死。
            let _ = self.tx.send((self.id, event));
            if self.repaint_enabled.load(Ordering::Relaxed) {
                self.repaint_controller.request_repaint(&self.egui_ctx);
            }
            return;
        }
        if matches!(
            event,
            PtyEvent::MouseCursorDirty | PtyEvent::CursorBlinkingChange
        ) {
            crate::gui::perf_log::count("terminal.proxy.repaint_only");
            if !self.forward_due(
                &self.next_repaint_forward_ms,
                TERMINAL_REPAINT_FORWARD_INTERVAL,
            ) {
                crate::gui::perf_log::count("terminal.proxy.repaint_only_coalesced");
                return;
            }
            let _ = self.tx.send((self.id, event));
            if self.repaint_enabled.load(Ordering::Relaxed) {
                self.repaint_controller.request_repaint(&self.egui_ctx);
            }
            return;
        }
        if matches!(event, PtyEvent::Title(_) | PtyEvent::ResetTitle) {
            crate::gui::perf_log::count("terminal.proxy.title");
            if !self.forward_due(&self.next_title_forward_ms, TERMINAL_TITLE_FORWARD_INTERVAL) {
                crate::gui::perf_log::count("terminal.proxy.title_coalesced");
                return;
            }
            let _ = self.tx.send((self.id, event));
            if self.repaint_enabled.load(Ordering::Relaxed) {
                self.repaint_controller.request_repaint(&self.egui_ctx);
            }
            return;
        }
        crate::gui::perf_log::count("terminal.proxy.other");
        let _ = self.tx.send((self.id, event));
        if self.repaint_enabled.load(Ordering::Relaxed) {
            self.repaint_controller.request_repaint(&self.egui_ctx);
        }
    }
}

impl TerminalEventProxy {
    /// 判断高频 PTY 通知是否到达转发时间片。
    fn forward_due(&self, next_forward_ms: &AtomicU64, interval: Duration) -> bool {
        let now_ms = terminal_elapsed_millis(self.throttle_origin);
        let current = next_forward_ms.load(Ordering::Acquire);
        if current > now_ms {
            return false;
        }
        next_forward_ms.store(
            now_ms.saturating_add(terminal_duration_millis(interval)),
            Ordering::Release,
        );
        true
    }
}

/// 计算 terminal 节流起点到现在经过的毫秒数。
fn terminal_elapsed_millis(origin: Instant) -> u64 {
    origin.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

/// 将 terminal 节流间隔转成至少 1ms 的毫秒数。
fn terminal_duration_millis(duration: Duration) -> u64 {
    (duration.as_millis().min(u128::from(u64::MAX)) as u64).max(1)
}

impl TerminalBackend {
    /// 创建 PTY-backed terminal 并启动 alacritty reader 线程。
    fn new(
        id: u64,
        egui_ctx: egui::Context,
        event_tx: UnboundedSender<(u64, PtyEvent)>,
        settings: BackendSettings,
    ) -> Result<Self> {
        let pty_config = tty::Options {
            shell: Some(tty::Shell::new(settings.shell, settings.args)),
            working_directory: settings.working_directory,
            ..tty::Options::default()
        };
        let size = TerminalSize::default();
        let runtime_size = Arc::new(Mutex::new(size));
        let pty = tty::new(&pty_config, size.into(), id)?;
        let repaint_enabled = Arc::new(AtomicBool::new(false));
        let throttle_origin = Instant::now();
        let proxy = TerminalEventProxy {
            id,
            tx: event_tx,
            egui_ctx,
            repaint_controller: settings.repaint_controller,
            repaint_enabled: repaint_enabled.clone(),
            throttle_origin,
            next_wakeup_forward_ms: Arc::new(AtomicU64::new(0)),
            next_repaint_forward_ms: Arc::new(AtomicU64::new(0)),
            next_title_forward_ms: Arc::new(AtomicU64::new(0)),
        };
        let repaint_controller = proxy.repaint_controller.clone();
        let term = Term::new(
            terminal_config(),
            &AlacrittyTermSize::from(size),
            proxy.clone(),
        );
        let term = Arc::new(FairMutex::new(term));
        if let Some(history) = settings.initial_history.as_deref() {
            restore_terminal_history(&term, history);
        }
        let event_loop = EventLoop::new(term.clone(), proxy, pty, false, false)?;
        let notifier = Notifier(event_loop.channel());
        let _pty_thread = event_loop.spawn();

        Ok(Self {
            id,
            term,
            notifier,
            size,
            runtime_size,
            repaint_enabled,
            repaint_controller,
        })
    }

    /// 仅在终端 surface 可见时启用 repaint 唤醒。
    fn set_repaint_enabled(&self, enabled: bool) {
        self.repaint_enabled.store(enabled, Ordering::Relaxed);
    }

    /// 通过 alacritty notifier 向子进程写入字节。
    fn write_bytes(&self, bytes: Vec<u8>) {
        // 触发条件：用户输入时 terminal viewport 仍在 scrollback。
        // 不能只 notify PTY：旧 BackendCommand::Write 会先滚到底部。
        // 防止回归：输入进了 Codex，但可见区域还停在历史输出。
        self.term.lock().scroll_display(Scroll::Bottom);
        self.notifier.notify(bytes);
    }

    /// host drop 时通知 PTY reader 退出。
    fn shutdown(&self) {
        let _ = self.notifier.0.send(Msg::Shutdown);
    }

    /// 同步 resize terminal emulator grid 和底层 PTY。
    fn resize(&mut self, layout_size: Vec2, font_measure: TerminalFontMeasure) {
        let cell_width = font_measure.cell_size.x.max(1.0);
        let cell_height = font_measure.cell_size.y.max(1.0);
        let cols = (layout_size.x / cell_width).floor().max(1.0) as u16;
        let lines = (layout_size.y / cell_height).floor().max(1.0) as u16;
        let next = TerminalSize {
            cell_width,
            cell_height,
            ascii_text_runs: font_measure.ascii_text_runs,
            cols,
            lines,
            layout_size,
        };
        if next == self.size {
            return;
        }

        self.size = next;
        if let Ok(mut runtime_size) = self.runtime_size.lock() {
            *runtime_size = next;
        }
        self.notifier.on_resize(next.into());
        self.term
            .lock()
            .resize(AlacrittyTermSize::new(cols as usize, lines as usize));
    }

    /// Converts egui drag coordinates into an alacritty grid point.
    fn selection_point(&self, rect: Rect, pos: Pos2) -> Point {
        let x = (pos.x - rect.left()).max(0.0);
        let y = (pos.y - rect.top()).max(0.0);
        let col = ((x / self.size.cell_width).floor() as usize)
            .min(self.size.cols.saturating_sub(1) as usize);
        let line = ((y / self.size.cell_height).floor() as usize)
            .min(self.size.lines.saturating_sub(1) as usize);
        let display_offset = self.term.lock().grid().display_offset();
        Point::new(Line(line as i32 - display_offset as i32), Column(col))
    }

    /// Starts a simple selection used by copy normalization.
    fn start_selection(&mut self, rect: Rect, pos: Pos2) {
        self.start_selection_with_type(rect, pos, SelectionType::Simple);
    }

    /// Starts a terminal selection with the requested alacritty expansion mode.
    fn start_selection_with_type(&mut self, rect: Rect, pos: Pos2, ty: SelectionType) {
        let point = self.selection_point(rect, pos);
        self.term.lock().selection = Some(Selection::new(ty, point, Side::Left));
    }

    /// Updates the active selection while the pointer is dragged.
    fn update_selection(&mut self, rect: Rect, pos: Pos2) {
        let point = self.selection_point(rect, pos);
        let mut term = self.term.lock();
        if let Some(selection) = &mut term.selection {
            selection.update(point, Side::Right);
        }
    }

    /// Scrolls the terminal display or sends alternate-screen cursor keys.
    fn scroll(&mut self, delta: i32) {
        if delta == 0 {
            return;
        }
        let mut term = self.term.lock();
        if term
            .mode()
            .contains(TermMode::ALTERNATE_SCROLL | TermMode::ALT_SCREEN)
        {
            let line_cmd = if delta > 0 { b'A' } else { b'B' };
            let mut content = Vec::new();
            for _ in 0..delta.abs() {
                content.push(0x1b);
                content.push(b'O');
                content.push(line_cmd);
            }
            drop(term);
            self.write_bytes(content);
        } else {
            term.grid_mut().scroll_display(Scroll::Delta(delta));
        }
    }
}

impl Drop for TerminalBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Drop for GuiTerminalHost {
    fn drop(&mut self) {
        if self.kind != TerminalSurfaceKind::Workspace {
            return;
        }
        let history = terminal_history_snapshot(&self.backend);
        if history.trim().is_empty() {
            return;
        }
        let _ = data::save_workspace_terminal_history(&self.workspace_root, &history);
    }
}

/// Backend settings needed to spawn a terminal child process.
struct BackendSettings {
    /// Program to execute inside the PTY.
    shell: String,
    /// Arguments passed to the program.
    args: Vec<String>,
    /// Optional working directory for the child process.
    working_directory: Option<PathBuf>,
    /// 终端驱动 repaint 时使用的全局 FPS 控制器。
    repaint_controller: RepaintController,
    /// 子进程输出前先写入模拟器的历史文本快照。
    initial_history: Option<String>,
}

/// Minimal terminal dimensions implementation accepted by alacritty.
struct AlacrittyTermSize {
    /// Number of terminal columns.
    cols: usize,
    /// Number of terminal rows.
    lines: usize,
}

impl AlacrittyTermSize {
    /// Creates emulator dimensions from visible columns and rows.
    fn new(cols: usize, lines: usize) -> Self {
        Self { cols, lines }
    }
}

impl From<TerminalSize> for AlacrittyTermSize {
    fn from(size: TerminalSize) -> Self {
        Self::new(size.cols as usize, size.lines as usize)
    }
}

impl Dimensions for AlacrittyTermSize {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

impl GuiTerminalHost {
    pub fn spawn(
        egui_ctx: &egui::Context,
        workspace: &WorkspaceViewData,
        kind: TerminalSurfaceKind,
        agent_launch: &AgentLaunchConfig,
        network_settings: &NetworkSettings,
        repaint_controller: RepaintController,
        theme_mode: gui_theme::ThemeMode,
        runtime_handle: Handle,
        event_sink: TerminalRuntimeEventSink,
    ) -> Result<Self> {
        Self::spawn_with_agent_session(
            egui_ctx,
            workspace,
            kind,
            agent_launch,
            network_settings,
            repaint_controller,
            theme_mode,
            runtime_handle,
            event_sink,
            workspace.session_id.as_deref(),
        )
    }

    pub fn spawn_without_resume(
        egui_ctx: &egui::Context,
        workspace: &WorkspaceViewData,
        agent_launch: &AgentLaunchConfig,
        network_settings: &NetworkSettings,
        repaint_controller: RepaintController,
        theme_mode: gui_theme::ThemeMode,
        runtime_handle: Handle,
        event_sink: TerminalRuntimeEventSink,
    ) -> Result<Self> {
        Self::spawn_with_agent_session(
            egui_ctx,
            workspace,
            TerminalSurfaceKind::Agent,
            agent_launch,
            network_settings,
            repaint_controller,
            theme_mode,
            runtime_handle,
            event_sink,
            None,
        )
    }

    fn spawn_with_agent_session(
        egui_ctx: &egui::Context,
        workspace: &WorkspaceViewData,
        kind: TerminalSurfaceKind,
        agent_launch: &AgentLaunchConfig,
        network_settings: &NetworkSettings,
        repaint_controller: RepaintController,
        theme_mode: gui_theme::ThemeMode,
        runtime_handle: Handle,
        event_sink: TerminalRuntimeEventSink,
        agent_session_id: Option<&str>,
    ) -> Result<Self> {
        let id = TERMINAL_ID.fetch_add(1, Ordering::Relaxed);
        let (event_tx, pty_events) = unbounded_channel();
        let runtime_state = Arc::new(Mutex::new(TerminalRuntimeState::default()));
        let runtime_theme_mode = Arc::new(Mutex::new(theme_mode));
        let shell = terminal_command(workspace, kind);
        let args = terminal_args(workspace, kind, id, agent_launch, agent_session_id);
        let env = terminal_env(workspace, kind, network_settings);
        let working_directory = terminal_working_directory(workspace, kind);
        let launch_command = command_display(&shell, &args);
        let initial_history = (kind == TerminalSurfaceKind::Workspace)
            .then(|| data::load_workspace_terminal_history(&workspace.path));
        let launched_with_resume = kind == TerminalSurfaceKind::Agent
            && args.iter().any(|arg| arg == "resume" || arg == "--resume");
        let launched_session_id = launched_with_resume
            .then(|| agent_session_id.map(str::to_string))
            .flatten();
        let backend = spawn_backend_with_env(
            id,
            egui_ctx.clone(),
            event_tx,
            BackendSettings {
                shell,
                args,
                working_directory: Some(working_directory),
                repaint_controller,
                initial_history,
            },
            &env,
        )?;
        spawn_terminal_runtime_drainer(
            runtime_handle,
            TerminalRuntimeDrainer {
                id,
                kind,
                command: launch_command.clone(),
                launched_session_id: launched_session_id.clone(),
                events: pty_events,
                term: backend.term.clone(),
                notifier: backend.notifier.0.clone(),
                runtime_size: backend.runtime_size.clone(),
                runtime_state: runtime_state.clone(),
                runtime_theme_mode: runtime_theme_mode.clone(),
                event_sink,
            },
        );

        Ok(Self {
            kind,
            workspace_root: workspace.path.clone(),
            workspace_name: workspace.name.clone(),
            agent_title: workspace.agent_kind.title().to_string(),
            session_id: workspace.session_id.clone(),
            activity: workspace.activity,
            helix_spec: None,
            launched_with_resume,
            backend,
            runtime_state,
            runtime_theme_mode,
            font: FontId::monospace(14.0),
        })
    }

    pub fn spawn_helix(
        egui_ctx: &egui::Context,
        workspace: &WorkspaceViewData,
        spec: HelixLaunchSpec,
        network_settings: &NetworkSettings,
        repaint_controller: RepaintController,
        theme_mode: gui_theme::ThemeMode,
        runtime_handle: Handle,
        event_sink: TerminalRuntimeEventSink,
    ) -> Result<Self> {
        let id = TERMINAL_ID.fetch_add(1, Ordering::Relaxed);
        let (event_tx, pty_events) = unbounded_channel();
        let runtime_state = Arc::new(Mutex::new(TerminalRuntimeState::default()));
        let runtime_theme_mode = Arc::new(Mutex::new(theme_mode));
        let args = helix_args(&spec);
        let launch_command = command_display("hx", &args);
        let backend = spawn_backend_with_env(
            id,
            egui_ctx.clone(),
            event_tx,
            BackendSettings {
                shell: "hx".to_string(),
                args,
                working_directory: Some(spec.workdir.clone()),
                repaint_controller,
                initial_history: None,
            },
            &terminal_env(workspace, TerminalSurfaceKind::Helix, network_settings),
        )?;
        spawn_terminal_runtime_drainer(
            runtime_handle,
            TerminalRuntimeDrainer {
                id,
                kind: TerminalSurfaceKind::Helix,
                command: launch_command.clone(),
                launched_session_id: None,
                events: pty_events,
                term: backend.term.clone(),
                notifier: backend.notifier.0.clone(),
                runtime_size: backend.runtime_size.clone(),
                runtime_state: runtime_state.clone(),
                runtime_theme_mode: runtime_theme_mode.clone(),
                event_sink,
            },
        );

        Ok(Self {
            kind: TerminalSurfaceKind::Helix,
            workspace_root: spec.workdir.clone(),
            workspace_name: workspace.name.clone(),
            agent_title: String::new(),
            session_id: None,
            activity: WorkspaceActivity::Unknown,
            helix_spec: Some(spec),
            launched_with_resume: false,
            backend,
            runtime_state,
            runtime_theme_mode,
            font: FontId::monospace(14.0),
        })
    }

    /// Updates cheap workspace metadata shown around an existing terminal.
    pub fn sync_workspace_metadata(&mut self, workspace: &WorkspaceViewData) {
        // Trigger: the UI calls this while an already-spawned terminal is drawn.
        // Why not always clone: draw paths run frequently and status rarely changes.
        // Prevents: per-frame String clones from dominating allocation profiles.
        if self.workspace_name != workspace.name {
            self.workspace_name = workspace.name.clone();
        }
        if self.session_id != workspace.session_id {
            self.session_id = workspace.session_id.clone();
        }
        self.activity = workspace.activity;
        if self.kind == TerminalSurfaceKind::Agent {
            let title = workspace.agent_kind.title();
            if self.agent_title != title {
                self.agent_title = title.to_string();
            }
        }
    }

    pub fn helix_spec(&self) -> Option<&HelixLaunchSpec> {
        self.helix_spec.as_ref()
    }

    /// 返回 terminal host 稳定 id，用于 AppEvent 轻量通知路由。
    pub fn id(&self) -> u64 {
        self.backend.id
    }

    pub fn take_abnormal_agent_exit(&mut self) -> Option<AgentProcessExit> {
        self.runtime_state
            .lock()
            .ok()
            .and_then(|mut state| state.abnormal_exit.take())
    }

    pub fn launched_with_resume(&self) -> bool {
        self.launched_with_resume
    }

    pub fn has_exited(&self) -> bool {
        self.runtime_state
            .lock()
            .is_ok_and(|state| state.exited_status.is_some())
    }

    /// 设置该终端是否允许 PTY 输出唤醒 repaint。
    pub fn set_event_repaint_enabled(&self, enabled: bool) {
        self.backend.set_repaint_enabled(enabled);
    }

    /// 更新 terminal runtime 回复 OSC dynamic color query 使用的主题。
    pub fn set_runtime_theme_mode(&self, theme_mode: gui_theme::ThemeMode) {
        if let Ok(mut current) = self.runtime_theme_mode.lock() {
            *current = theme_mode;
        }
    }

    pub fn exited_status_label(&self) -> Option<String> {
        self.runtime_state
            .lock()
            .ok()
            .and_then(|state| state.exited_status.map(exit_status_label))
    }

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        theme_mode: gui_theme::ThemeMode,
        font_size: f32,
        include_input_rect: bool,
        request_focus: bool,
        accept_input: bool,
        shortcut_scope: TerminalInputShortcutScope,
    ) -> Option<TerminalHostUiOutput> {
        let ui_started_at = Instant::now();
        crate::gui::perf_log::count(terminal_host_ui_label(self.kind));
        self.set_runtime_theme_mode(theme_mode);
        // 触发条件：隐藏终端收到 OSC52 后重新进入可见绘制。
        // 不能在这里 drain PTY：绘制路径只能补交 UI clipboard 副作用。
        // 防止回归：PTY 输出风暴借 render 路径卡住 UI。
        self.flush_pending_clipboard_text(ui.ctx());
        if self.has_exited() {
            return None;
        }
        self.font = terminal_font(font_size, self.kind);
        let size = Vec2::new(ui.available_width(), ui.available_height().max(1.0));
        let response = terminal_view(
            ui,
            &mut self.backend,
            size,
            theme_mode,
            self.font.clone(),
            request_focus,
            accept_input,
            &self.workspace_root,
        );
        let clicked_output = if self.kind == TerminalSurfaceKind::Agent && accept_input {
            terminal_output_click(
                &self.backend,
                response.rect,
                &response,
                &self.workspace_root,
            )
        } else {
            None
        };
        let output_click_copy_only =
            clicked_output.is_some() && ui.input(|input| input.modifiers.ctrl);
        self.paint_bell_flash(ui, response.rect);
        if request_focus {
            response.request_focus();
        }
        let copy_has_terminal_selection = self.override_selected_copy_text(ui, &response);
        let mut input_submitted = false;
        if self.kind == TerminalSurfaceKind::Agent
            && std::env::var_os("GSDV_AGENT_ESC_DEBUG").is_some()
            && ui.ctx().input(|input| input.key_pressed(egui::Key::Escape))
        {
            eprintln!(
                "[gsdv][agent-esc][render-surface] host={} request_focus={} has_focus={} accept_input={}",
                self.id(),
                request_focus,
                response.has_focus(),
                accept_input
            );
        }
        // 触发条件：terminal surface 已获得键盘焦点。
        // 不能交给 input runtime 写普通键：terminal focus 会让
        // wants_keyboard_input 为 true，导致英文和控制键被跳过。
        // 防止回归：普通按键不可输入，或 IME 能输入但英文不能输入。
        if accept_input && (request_focus || response.has_focus()) {
            let kitty_keyboard_protocol = self
                .terminal_mode()
                .intersects(TermMode::KITTY_KEYBOARD_PROTOCOL);
            let bytes = ui.ctx().input(|input| {
                agent_input_bytes_from_events_with_kitty_protocol(
                    &input.events,
                    input.modifiers,
                    !copy_has_terminal_selection,
                    kitty_keyboard_protocol,
                    Some(shortcut_scope),
                )
            });
            if self.kind == TerminalSurfaceKind::Agent
                && bytes.contains(&0x1b)
                && std::env::var_os("GSDV_AGENT_ESC_DEBUG").is_some()
            {
                eprintln!(
                    "[gsdv][agent-esc][render-write] host={} kind={:?} kitty={} request_focus={} has_focus={} accept_input={} bytes={:?}",
                    self.id(),
                    self.kind,
                    kitty_keyboard_protocol,
                    request_focus,
                    response.has_focus(),
                    accept_input,
                    bytes
                );
            }
            input_submitted = self.kind == TerminalSurfaceKind::Agent
                && terminal_agent_input_submit_bytes(&bytes);
            if input_submitted {
                crate::gui::perf_log::count("terminal.input_submitted");
            }
            self.write_bytes(&bytes);
            enable_terminal_ime(ui, &self.backend, response.rect);
            self.write_ime_commits(ui);
        }
        let input_rect = (include_input_rect && self.kind == TerminalSurfaceKind::Agent)
            .then(|| terminal_agent_input_rect(&self.backend, response.rect))
            .flatten();
        crate::gui::perf_log::duration_us("terminal.host_ui_us", ui_started_at.elapsed());
        Some(TerminalHostUiOutput {
            response,
            input_rect,
            input_submitted,
            output_click: clicked_output,
            output_click_copy_only,
        })
    }

    /// Paints a short visual flash for terminal bell events.
    fn paint_bell_flash(&mut self, ui: &Ui, rect: Rect) {
        let Some(until) = self
            .runtime_state
            .lock()
            .ok()
            .and_then(|state| state.bell_flash_until)
        else {
            return;
        };
        let now = Instant::now();
        if now >= until {
            if let Ok(mut state) = self.runtime_state.lock() {
                state.bell_flash_until = None;
            }
            return;
        }
        let remaining = until.saturating_duration_since(now);
        let alpha = terminal_bell_flash_alpha(remaining);
        ui.painter()
            .rect_filled(rect, 0.0, Color32::from_white_alpha(alpha));
        self.backend.repaint_controller.request_repaint(ui.ctx());
    }

    /// Copies selected text while skipping alacritty wide-cell spacers.
    fn override_selected_copy_text(&mut self, ui: &Ui, response: &egui::Response) -> bool {
        if !response.has_focus() || !terminal_selection_copy_requested(ui) {
            return false;
        }
        let term = self.backend.term.lock();
        let Some(range) = term
            .selection
            .as_ref()
            .and_then(|selection| selection.to_range(&term))
        else {
            return false;
        };

        let mut text = String::new();
        let mut current_line = None;
        for indexed in term.grid().display_iter() {
            if !range.contains(indexed.point) {
                continue;
            }
            if indexed
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            if current_line.is_some_and(|line| line != indexed.point.line) {
                trim_trailing_spaces(&mut text);
                text.push('\n');
            }
            current_line = Some(indexed.point.line);
            text.push(indexed.c);
        }

        trim_trailing_spaces(&mut text);
        if !text.is_empty() {
            ui.ctx().copy_text(text);
            return true;
        }
        false
    }

    pub fn paste_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.terminal_mode().contains(TermMode::BRACKETED_PASTE) {
            let mut bytes = Vec::with_capacity(text.len() + 12);
            bytes.extend_from_slice(b"\x1b[200~");
            bytes.extend_from_slice(text.as_bytes());
            bytes.extend_from_slice(b"\x1b[201~");
            self.write_bytes(&bytes);
        } else {
            self.write_bytes(text.as_bytes());
        }
    }

    /// 返回当前 Agent composer 里可见的输入草稿。
    pub fn agent_input_text_snapshot(&self) -> Option<String> {
        if self.kind != TerminalSurfaceKind::Agent {
            return None;
        }
        terminal_agent_input_text_snapshot(&self.backend)
    }

    /// Replaces the active Agent composer text with a prepared value.
    pub fn replace_agent_input_text(&mut self, text: &str, clear_existing: bool) {
        if self.kind != TerminalSurfaceKind::Agent || text.is_empty() {
            return;
        }
        if clear_existing {
            // 触发条件：把 AI 翻译结果应用回 Codex composer。
            // 不能用 Ctrl+A/K 组合猜多行编辑状态：Codex 有专门的
            // clear_for_ctrl_c 路径，会把当前草稿完整清掉并入历史。
            // 防止回归：多行草稿只删一行，导致翻译结果和原文混在一起。
            self.write_bytes(b"\x03");
        }
        self.paste_text(text);
    }

    /// Rewrites text around existing Codex image placeholders without deleting them.
    pub fn replace_agent_input_text_preserving_images(
        &mut self,
        source_text: &str,
        target_text: &str,
    ) -> bool {
        if self.kind != TerminalSurfaceKind::Agent || target_text.is_empty() {
            return false;
        }
        let Some(source) = image_placeholder_segments(source_text) else {
            return false;
        };
        let Some(target) = image_placeholder_segments(target_text) else {
            return false;
        };
        if source.placeholders != target.placeholders {
            return false;
        }
        let bytes = rewrite_text_preserving_placeholders_bytes(&source.texts, &target.texts);
        if bytes.is_empty() {
            return false;
        }
        self.write_bytes(&bytes);
        true
    }

    /// 提交当前 terminal 应用里的输入草稿。
    pub fn submit_current_input(&mut self) {
        crate::gui::perf_log::count("terminal.submit_current_input");
        // 触发条件：Codex TUI 启用 kitty 增强键盘协议后需要区分
        // Enter 和 Ctrl+M。
        // 不能直接写 \r：新版 keymap 会把 Ctrl+M 当成插入换行。
        // 防止右键快捷回复只变成“文字 + 换行”而没有发起请求。
        if self
            .terminal_mode()
            .intersects(TermMode::KITTY_KEYBOARD_PROTOCOL)
        {
            self.write_bytes(b"\x1b[13u");
        } else {
            self.write_bytes(b"\r");
        }
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.backend.write_bytes(bytes.to_vec());
    }

    /// 返回子进程是否已开启 kitty keyboard protocol。
    pub fn kitty_keyboard_protocol_enabled(&self) -> bool {
        self.terminal_mode()
            .intersects(TermMode::KITTY_KEYBOARD_PROTOCOL)
    }

    /// 返回当前子进程声明的 terminal 输入模式。
    fn terminal_mode(&self) -> TermMode {
        *self.backend.term.lock().mode()
    }

    pub fn request_graceful_exit(&mut self) {
        self.backend.write_bytes(vec![0x03]);
    }

    /// 终端重新可见时写入延迟的 OSC52 剪贴板文本。
    fn flush_pending_clipboard_text(&mut self, ctx: &egui::Context) {
        let pending = self
            .runtime_state
            .lock()
            .ok()
            .and_then(|mut state| state.pending_clipboard_text.take());
        if let Some((clipboard_type, text)) = pending {
            if set_terminal_clipboard_text(clipboard_type, &text).is_err()
                && clipboard_type == term::ClipboardType::Clipboard
            {
                ctx.copy_text(text);
            }
        }
    }

    fn write_ime_commits(&mut self, ui: &Ui) {
        let commits = ui
            .ctx()
            .input(|input| ime_commit_texts_without_text_duplicates(&input.events));
        for text in commits {
            self.backend.write_bytes(text.into_bytes());
        }
    }
}

/// terminal runtime drainer 需要持有的跨任务资源。
struct TerminalRuntimeDrainer {
    /// terminal host 稳定 id。
    id: u64,
    /// terminal surface 类型，用于判断 Agent 异常退出。
    kind: TerminalSurfaceKind,
    /// 启动命令摘要。
    command: String,
    /// resume 启动时绑定的 session id。
    launched_session_id: Option<String>,
    /// alacritty event proxy 写入的原始 PTY event 队列。
    events: UnboundedReceiver<(u64, PtyEvent)>,
    /// alacritty emulator 状态，用于 OSC color query。
    term: Arc<FairMutex<Term<TerminalEventProxy>>>,
    /// 写入 alacritty event loop 的消息发送端。
    notifier: alacritty_terminal::event_loop::EventLoopSender,
    /// 最新 terminal 尺寸。
    runtime_size: Arc<Mutex<TerminalSize>>,
    /// drainer 写入、UI 读取的共享状态。
    runtime_state: Arc<Mutex<TerminalRuntimeState>>,
    /// 最新 UI 主题。
    runtime_theme_mode: Arc<Mutex<gui_theme::ThemeMode>>,
    /// 向 AppEvent 队列投递轻量通知的出口。
    event_sink: TerminalRuntimeEventSink,
}

/// 启动 terminal 自有 runtime drainer 协程。
///
/// 触发条件：alacritty reader 线程产生原始 PTY event。
/// 不能交给 AppEvent drain：原始 PTY event 是 terminal 内部协议事件，
/// 可能高频且包含 clipboard/color/size 回复等业务细节。
/// 防止回归：AppEvent 里一个事件继续套一层全 host PTY 批处理。
fn spawn_terminal_runtime_drainer(runtime_handle: Handle, drainer: TerminalRuntimeDrainer) {
    runtime_handle.spawn(drain_terminal_runtime_events_owned(drainer));
}

/// 在 terminal runtime 协程内顺序消费原始 PTY event。
async fn drain_terminal_runtime_events_owned(mut drainer: TerminalRuntimeDrainer) {
    while let Some((_id, event)) = drainer.events.recv().await {
        handle_terminal_runtime_event(&drainer, event);
    }
}

/// 处理单个原始 PTY event，并只向 AppEvent 投递粗粒度 UI 通知。
fn handle_terminal_runtime_event(drainer: &TerminalRuntimeDrainer, event: PtyEvent) {
    if drainer.kind == TerminalSurfaceKind::Agent
        && std::env::var_os("GSDV_AGENT_ESC_DEBUG").is_some()
    {
        eprintln!(
            "[gsdv][agent-esc][runtime-drainer] id={} kind={:?} pty_event={}",
            drainer.id,
            drainer.kind,
            pty_event_kind(&event)
        );
    }
    match event {
        PtyEvent::Wakeup => {
            crate::gui::perf_log::count("terminal.drain.wakeup");
            emit_terminal_runtime_event(drainer, TerminalRuntimeEventKind::Output);
        }
        PtyEvent::ColorRequest(index, formatter) => {
            let mode = drainer
                .runtime_theme_mode
                .lock()
                .map(|mode| *mode)
                .unwrap_or(gui_theme::ThemeMode::Dark);
            if let Some(response) = terminal_color_response(&drainer.term, index, &*formatter, mode)
            {
                Notifier(drainer.notifier.clone()).notify(response.into_bytes());
            } else {
                record_unhandled_pty_event("ColorRequest");
            }
        }
        PtyEvent::PtyWrite(text) => {
            Notifier(drainer.notifier.clone()).notify(text.into_bytes());
        }
        PtyEvent::ClipboardStore(clipboard_type, text) => {
            // 触发条件：Agent 通过 OSC52 请求写系统剪贴板。
            // 不能忽略该事件：alacritty_terminal 只解析协议，
            // 真正写剪贴板必须由宿主完成。
            // 防止 Agent 内部复制操作看似成功但剪贴板为空。
            if set_terminal_clipboard_text(clipboard_type, &text).is_err() {
                if let Ok(mut state) = drainer.runtime_state.lock() {
                    state.pending_clipboard_text = Some((clipboard_type, text));
                }
                emit_terminal_runtime_event(drainer, TerminalRuntimeEventKind::Repaint);
            }
        }
        PtyEvent::ClipboardLoad(clipboard_type, formatter) => {
            let text = terminal_clipboard_text(clipboard_type).unwrap_or_default();
            Notifier(drainer.notifier.clone()).notify(formatter(&text).into_bytes());
        }
        PtyEvent::TextAreaSizeRequest(formatter) => {
            let size = drainer
                .runtime_size
                .lock()
                .map(|size| *size)
                .unwrap_or_default();
            Notifier(drainer.notifier.clone()).notify(formatter(size.into()).into_bytes());
        }
        PtyEvent::ChildExit(status) => {
            if let Ok(mut state) = drainer.runtime_state.lock() {
                state.exited_status = Some(status);
                if drainer.kind == TerminalSurfaceKind::Agent && status != 0 {
                    state.abnormal_exit = Some(AgentProcessExit {
                        command: drainer.command.clone(),
                        status: exit_status_label(status),
                        session_id: drainer.launched_session_id.clone(),
                    });
                }
            }
            emit_terminal_runtime_event(drainer, TerminalRuntimeEventKind::StateChanged);
        }
        PtyEvent::Title(title) => {
            if let Ok(mut state) = drainer.runtime_state.lock() {
                state.terminal_title = (!title.is_empty()).then_some(title);
            }
            emit_terminal_runtime_event(drainer, TerminalRuntimeEventKind::StateChanged);
        }
        PtyEvent::ResetTitle => {
            if let Ok(mut state) = drainer.runtime_state.lock() {
                state.terminal_title = None;
            }
            emit_terminal_runtime_event(drainer, TerminalRuntimeEventKind::StateChanged);
        }
        PtyEvent::Bell => {
            if let Ok(mut state) = drainer.runtime_state.lock() {
                state.bell_flash_until = Some(Instant::now() + TERMINAL_BELL_FLASH_DURATION);
            }
            emit_terminal_runtime_event(drainer, TerminalRuntimeEventKind::Repaint);
        }
        PtyEvent::MouseCursorDirty | PtyEvent::CursorBlinkingChange => {
            crate::gui::perf_log::count("terminal.drain.repaint_only");
            emit_terminal_runtime_event(drainer, TerminalRuntimeEventKind::Repaint);
        }
        other => {
            record_unhandled_pty_event(pty_event_kind(&other));
        }
    }
}

/// 投递 terminal 粗粒度 UI 通知。
fn emit_terminal_runtime_event(drainer: &TerminalRuntimeDrainer, kind: TerminalRuntimeEventKind) {
    (drainer.event_sink)(TerminalRuntimeEvent {
        id: drainer.id,
        kind,
    });
}

/// Builds an OSC color-query response using alacritty's formatter.
fn terminal_color_response(
    term: &Arc<FairMutex<Term<TerminalEventProxy>>>,
    index: usize,
    formatter: &(dyn Fn(Rgb) -> String + Sync + Send + 'static),
    mode: gui_theme::ThemeMode,
) -> Option<String> {
    let term = term.lock();
    let content = term.renderable_content();
    terminal_query_rgb(index, content.colors, mode).map(formatter)
}

/// 记录未知 PTY 事件类型，适用于 drain 中的轻量去重。
fn record_unhandled_pty_event(kind: &str) {
    if let Ok(mut events) = UNHANDLED_PTY_EVENTS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
    {
        let _ = events.insert(pty_event_kind_name(kind));
    }
}

/// 返回静态事件名，避免 drain 路径分配或写磁盘。
fn pty_event_kind_name(kind: &str) -> &'static str {
    match kind {
        "MouseCursorDirty" => "MouseCursorDirty",
        "Title" => "Title",
        "ResetTitle" => "ResetTitle",
        "ClipboardStore" => "ClipboardStore",
        "ClipboardLoad" => "ClipboardLoad",
        "ColorRequest" => "ColorRequest",
        "PtyWrite" => "PtyWrite",
        "TextAreaSizeRequest" => "TextAreaSizeRequest",
        "CursorBlinkingChange" => "CursorBlinkingChange",
        "Wakeup" => "Wakeup",
        "Bell" => "Bell",
        "Exit" => "Exit",
        "ChildExit" => "ChildExit",
        _ => "Unknown",
    }
}

/// 返回未知 PTY 日志使用的稳定事件名。
fn pty_event_kind(event: &PtyEvent) -> &'static str {
    match event {
        PtyEvent::MouseCursorDirty => "MouseCursorDirty",
        PtyEvent::Title(_) => "Title",
        PtyEvent::ResetTitle => "ResetTitle",
        PtyEvent::ClipboardStore(_, _) => "ClipboardStore",
        PtyEvent::ClipboardLoad(_, _) => "ClipboardLoad",
        PtyEvent::ColorRequest(_, _) => "ColorRequest",
        PtyEvent::PtyWrite(_) => "PtyWrite",
        PtyEvent::TextAreaSizeRequest(_) => "TextAreaSizeRequest",
        PtyEvent::CursorBlinkingChange => "CursorBlinkingChange",
        PtyEvent::Wakeup => "Wakeup",
        PtyEvent::Bell => "Bell",
        PtyEvent::Exit => "Exit",
        PtyEvent::ChildExit(_) => "ChildExit",
    }
}

/// Reads text for terminal clipboard query events.
fn terminal_clipboard_text(clipboard_type: term::ClipboardType) -> Result<String, arboard::Error> {
    let mut clipboard = arboard::Clipboard::new()?;
    terminal_clipboard_get_text(&mut clipboard, clipboard_type)
}

/// Reads the requested Linux clipboard selection.
#[cfg(target_os = "linux")]
fn terminal_clipboard_get_text(
    clipboard: &mut arboard::Clipboard,
    clipboard_type: term::ClipboardType,
) -> Result<String, arboard::Error> {
    use arboard::GetExtLinux;

    let selection = linux_clipboard_kind(clipboard_type);
    clipboard.get().clipboard(selection).text()
}

/// Reads the platform clipboard for non-Linux terminals.
#[cfg(not(target_os = "linux"))]
fn terminal_clipboard_get_text(
    clipboard: &mut arboard::Clipboard,
    _clipboard_type: term::ClipboardType,
) -> Result<String, arboard::Error> {
    clipboard.get_text()
}

/// Writes OSC52 clipboard-store text to the platform clipboard.
fn set_terminal_clipboard_text(
    clipboard_type: term::ClipboardType,
    text: &str,
) -> Result<(), arboard::Error> {
    let mut clipboard = arboard::Clipboard::new()?;
    terminal_clipboard_set_text(&mut clipboard, clipboard_type, text)
}

/// Writes text to the requested Linux clipboard selection.
#[cfg(target_os = "linux")]
fn terminal_clipboard_set_text(
    clipboard: &mut arboard::Clipboard,
    clipboard_type: term::ClipboardType,
    text: &str,
) -> Result<(), arboard::Error> {
    use arboard::SetExtLinux;

    let selection = linux_clipboard_kind(clipboard_type);
    clipboard.set().clipboard(selection).text(text.to_owned())
}

/// Writes text to the platform clipboard for non-Linux terminals.
#[cfg(not(target_os = "linux"))]
fn terminal_clipboard_set_text(
    clipboard: &mut arboard::Clipboard,
    _clipboard_type: term::ClipboardType,
    text: &str,
) -> Result<(), arboard::Error> {
    clipboard.set_text(text)
}

/// Maps alacritty clipboard types to Linux clipboard selections.
#[cfg(target_os = "linux")]
fn linux_clipboard_kind(clipboard_type: term::ClipboardType) -> arboard::LinuxClipboardKind {
    match clipboard_type {
        term::ClipboardType::Clipboard => arboard::LinuxClipboardKind::Clipboard,
        term::ClipboardType::Selection => arboard::LinuxClipboardKind::Primary,
    }
}

/// Converts egui keyboard events into bytes expected by terminal apps.
pub fn agent_input_bytes_from_events(
    events: &[Event],
    active_modifiers: Modifiers,
    copy_event_can_interrupt: bool,
) -> Vec<u8> {
    agent_input_bytes_from_events_with_kitty_protocol(
        events,
        active_modifiers,
        copy_event_can_interrupt,
        false,
        None,
    )
}

/// Converts egui keyboard events using negotiated kitty keyboard support.
pub(super) fn agent_input_bytes_from_events_with_kitty_protocol(
    events: &[Event],
    active_modifiers: Modifiers,
    copy_event_can_interrupt: bool,
    kitty_keyboard_protocol: bool,
    app_shortcut_scope: Option<TerminalInputShortcutScope>,
) -> Vec<u8> {
    let suppress_shortcut_text =
        active_modifiers.mac_cmd || (active_modifiers.alt && !active_modifiers.ctrl);
    let mut bytes = Vec::new();
    for event in events {
        match event {
            Event::Text(text) if !suppress_shortcut_text => {
                bytes.extend_from_slice(text.as_bytes());
            }
            Event::Paste(text) => {
                bytes.extend_from_slice(text.as_bytes());
            }
            Event::Copy
                if copy_event_should_interrupt(active_modifiers, copy_event_can_interrupt) =>
            {
                bytes.push(0x03);
            }
            Event::Copy => {}
            Event::Key {
                key,
                physical_key,
                pressed: true,
                modifiers,
                ..
            } => {
                if copy_key_event_should_interrupt(*key, *modifiers, copy_event_can_interrupt) {
                    bytes.push(0x03);
                } else if terminal_copy_key_event(*key, *modifiers) {
                    continue;
                } else if app_shortcut_scope.is_some_and(|scope| {
                    app_reserved_terminal_key_event(*key, *physical_key, *modifiers, scope)
                }) {
                    continue;
                } else if let Some(sequence) =
                    kitty_super_key_event_sequence(*key, *modifiers, kitty_keyboard_protocol)
                {
                    bytes.extend_from_slice(sequence.as_bytes());
                } else if let Some(sequence) =
                    kitty_plain_key_event_sequence(*key, *modifiers, kitty_keyboard_protocol)
                {
                    bytes.extend_from_slice(sequence.as_bytes());
                } else if let Some(sequence) = meta_key_event_sequence(*key, *modifiers) {
                    bytes.extend_from_slice(&sequence);
                } else if suppress_shortcut_text {
                    continue;
                } else if let Some(sequence) = key_event_sequence(*key, *modifiers) {
                    bytes.extend_from_slice(sequence.as_bytes());
                } else if let Some(byte) = control_key_byte(*key, *modifiers) {
                    bytes.push(byte);
                }
            }
            _ => {}
        }
    }
    bytes
}

/// Encodes supported Super/Cmd key chords with kitty's CSI-u keyboard protocol.
fn kitty_super_key_event_sequence(
    key: Key,
    modifiers: Modifiers,
    kitty_keyboard_protocol: bool,
) -> Option<&'static str> {
    if !kitty_keyboard_protocol || !modifiers.mac_cmd || modifiers.ctrl || modifiers.alt {
        return None;
    }
    // 触发条件：终端子进程已启用 kitty keyboard protocol，并收到
    // macOS Command + bracket 这类传统 PTY 无法表达的修饰键组合。
    // 不能走普通文本或 Alt 映射：Helix 等程序按协议读取 Super/Cmd。
    // 防止回归：老协议继续吞掉 Cmd 组合，不把快捷键文本漏给子进程。
    match key {
        Key::OpenBracket => Some("\x1b[91;9u"),
        Key::CloseBracket => Some("\x1b[93;9u"),
        _ => None,
    }
}

/// 编码 kitty keyboard protocol 下的基础控制键。
fn kitty_plain_key_event_sequence(
    key: Key,
    modifiers: Modifiers,
    kitty_keyboard_protocol: bool,
) -> Option<&'static str> {
    if !kitty_keyboard_protocol
        || modifiers.command
        || modifiers.mac_cmd
        || modifiers.ctrl
        || modifiers.alt
        || modifiers.shift
    {
        return None;
    }
    // 触发条件：Codex 通过 crossterm 开启 kitty keyboard protocol。
    // 不能继续写裸 \r/\x1b：增强模式下要区分控制字符和按键。
    // 防止回归：Esc 中断后，Enter 只换行或输入看起来卡死。
    match key {
        Key::Enter => Some("\x1b[13u"),
        Key::Escape => Some("\x1b[27u"),
        _ => None,
    }
}

/// Detects terminal copy key chords that must not reach the child process.
fn terminal_copy_key_event(key: Key, modifiers: Modifiers) -> bool {
    key == Key::C && modifiers.ctrl && modifiers.shift && !modifiers.mac_cmd && !modifiers.alt
}

/// Detects Copy events that should reach Codex as Ctrl+C.
fn copy_event_should_interrupt(modifiers: Modifiers, copy_event_can_interrupt: bool) -> bool {
    if !copy_event_can_interrupt {
        return false;
    }
    if modifiers.shift || modifiers.alt {
        return false;
    }
    // 触发条件：平台把 Ctrl/Cmd+C 上报为 Event::Copy。
    // 不能把无修饰键 Copy 当中断：Ctrl+R 搜索中普通 c
    // 可能被折成 Copy，误发 ETX 会直接取消搜索。
    // 防止回归：普通输入 c 被显示成 ^C。
    modifiers.ctrl || modifiers.mac_cmd || modifiers.command
}

/// Detects copy key events that should reach Codex as Ctrl+C.
fn copy_key_event_should_interrupt(
    key: Key,
    modifiers: Modifiers,
    copy_event_can_interrupt: bool,
) -> bool {
    key == Key::C && copy_event_should_interrupt(modifiers, copy_event_can_interrupt)
}

/// Detects app-owned shortcuts that must not be sent to terminal children.
fn app_reserved_terminal_key_event(
    key: Key,
    physical_key: Option<Key>,
    modifiers: Modifiers,
    scope: TerminalInputShortcutScope,
) -> bool {
    let command_or_alt = terminal_command_or_alt_shortcut_modifier(modifiers);
    match scope {
        TerminalInputShortcutScope::WorkspaceDrawerSurface => {
            workspace_terminal_drawer_reserved_key(key, physical_key, modifiers)
        }
        TerminalInputShortcutScope::HelixDrawerSurface => {
            workspace_terminal_drawer_reserved_key(key, physical_key, modifiers)
                || reviewer_helix_drawer_reserved_key(key, physical_key, modifiers)
        }
        TerminalInputShortcutScope::AgentSurface => {
            workspace_terminal_drawer_reserved_key(key, physical_key, modifiers)
                || (command_or_alt && matches_physical_or_logical_key(key, physical_key, Key::K))
                || (command_or_alt && matches_physical_or_logical_key(key, physical_key, Key::M))
                || (command_or_alt && matches_physical_or_logical_key(key, physical_key, Key::N))
                || (command_or_alt && matches_physical_or_logical_key(key, physical_key, Key::R))
                || (modifiers.command
                    && matches_physical_or_logical_key(key, physical_key, Key::Enter))
                || agent_slot_reserved_key(key, physical_key, modifiers)
                || workspace_cycle_reserved_key(key, physical_key, modifiers)
        }
    }
}

/// 检测 workspace terminal 抽屉归 app 处理的快捷键。
fn workspace_terminal_drawer_reserved_key(
    key: Key,
    physical_key: Option<Key>,
    modifiers: Modifiers,
) -> bool {
    let command_or_alt = terminal_command_or_alt_shortcut_modifier(modifiers);
    (command_or_alt && matches_physical_or_logical_key(key, physical_key, Key::T))
        || (command_or_alt && matches_physical_or_logical_key(key, physical_key, Key::W))
}

/// 检测 Helix 抽屉归 app 处理的快捷键。
fn reviewer_helix_drawer_reserved_key(
    key: Key,
    physical_key: Option<Key>,
    modifiers: Modifiers,
) -> bool {
    terminal_command_or_alt_shortcut_modifier(modifiers)
        && matches_physical_or_logical_key(key, physical_key, Key::X)
}

/// Detects Cmd/Alt number shortcuts reserved for agent slot selection.
fn agent_slot_reserved_key(key: Key, physical_key: Option<Key>, modifiers: Modifiers) -> bool {
    if modifiers.ctrl || !(modifiers.alt || modifiers.mac_cmd || modifiers.command) {
        return false;
    }
    [Key::Num1, Key::Num2, Key::Num3, Key::Num4]
        .into_iter()
        .any(|reserved| matches_physical_or_logical_key(key, physical_key, reserved))
}

/// Detects workspace cycle shortcuts that should stay in app routing.
fn workspace_cycle_reserved_key(key: Key, physical_key: Option<Key>, modifiers: Modifiers) -> bool {
    let ctrl_workspace = (modifiers.ctrl || modifiers.command)
        && !modifiers.shift
        && !modifiers.alt
        && !modifiers.mac_cmd;
    if ctrl_workspace
        && (matches_physical_or_logical_key(key, physical_key, Key::Backtick)
            || matches_physical_or_logical_key(key, physical_key, Key::Num1))
    {
        return true;
    }
    // 触发条件：不同键盘布局把 Cmd/Alt+数字行报成符号键。
    // 不能只看 logical key：app 快捷键解析会用 physical key 兜底。
    // 防止回归：终端先收到 ESC+符号，app 后续才切 workspace。
    terminal_command_or_alt_shortcut_modifier(modifiers)
        && physical_key == Some(Key::Num1)
        && key != Key::Num1
}

/// Checks app-owned Cmd/Alt terminal shortcuts without accepting Ctrl-as-command.
fn terminal_command_or_alt_shortcut_modifier(modifiers: Modifiers) -> bool {
    !modifiers.ctrl && (modifiers.alt || modifiers.mac_cmd || modifiers.command)
}

/// Checks a key using both egui logical and physical key data.
fn matches_physical_or_logical_key(key: Key, physical_key: Option<Key>, expected: Key) -> bool {
    key == expected || physical_key == Some(expected)
}

#[derive(Debug, Clone)]
struct TerminalPointerInput {
    modifiers: Modifiers,
    pointer_pos: Option<Pos2>,
    primary_down: bool,
    secondary_down: bool,
    middle_down: bool,
}

fn terminal_mouse_input_bytes(
    events: &[Event],
    input: &TerminalPointerInput,
    rect: Rect,
    size: TerminalSize,
    mode: TermMode,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    for event in events {
        match event {
            Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers,
            } if rect.contains(*pos) => {
                let Some(button_code) = terminal_mouse_button_code(*button) else {
                    continue;
                };
                let event_code = if *pressed { button_code } else { 3 };
                terminal_mouse_report(&mut bytes, event_code, *pos, *modifiers, rect, size, mode);
            }
            Event::PointerMoved(pos) if rect.contains(*pos) => {
                let Some(button_code) = terminal_mouse_motion_button_code(input, mode) else {
                    continue;
                };
                terminal_mouse_report(
                    &mut bytes,
                    button_code | 32,
                    *pos,
                    input.modifiers,
                    rect,
                    size,
                    mode,
                );
            }
            Event::MouseWheel {
                delta, modifiers, ..
            } if delta.y != 0.0 => {
                let Some(pos) = input.pointer_pos.filter(|pos| rect.contains(*pos)) else {
                    continue;
                };
                let event_code = if delta.y > 0.0 { 64 } else { 65 };
                terminal_mouse_report(&mut bytes, event_code, pos, *modifiers, rect, size, mode);
            }
            _ => {}
        }
    }
    bytes
}

fn terminal_mouse_motion_button_code(input: &TerminalPointerInput, mode: TermMode) -> Option<u8> {
    if mode.contains(TermMode::MOUSE_MOTION) {
        if input.primary_down {
            return Some(0);
        }
        if input.middle_down {
            return Some(1);
        }
        if input.secondary_down {
            return Some(2);
        }
        return Some(3);
    }
    if !mode.contains(TermMode::MOUSE_DRAG) {
        return None;
    }
    if input.primary_down {
        Some(0)
    } else if input.middle_down {
        Some(1)
    } else if input.secondary_down {
        Some(2)
    } else {
        None
    }
}

fn terminal_mouse_button_code(button: PointerButton) -> Option<u8> {
    match button {
        PointerButton::Primary => Some(0),
        PointerButton::Middle => Some(1),
        PointerButton::Secondary => Some(2),
        PointerButton::Extra1 | PointerButton::Extra2 => None,
    }
}

fn terminal_mouse_report(
    bytes: &mut Vec<u8>,
    event_code: u8,
    pos: Pos2,
    modifiers: Modifiers,
    rect: Rect,
    size: TerminalSize,
    mode: TermMode,
) {
    let event_code = event_code | terminal_mouse_modifier_bits(modifiers);
    let (col, line) = terminal_mouse_position(pos, rect, size);
    if mode.contains(TermMode::SGR_MOUSE) {
        let suffix = if event_code & 3 == 3 { 'm' } else { 'M' };
        bytes.extend_from_slice(
            format!("\x1b[<{};{};{}{}", event_code, col, line, suffix).as_bytes(),
        );
    } else {
        bytes.extend_from_slice(b"\x1b[M");
        bytes.push(event_code.saturating_add(32));
        bytes.push((col as u8).saturating_add(32));
        bytes.push((line as u8).saturating_add(32));
    }
}

fn terminal_mouse_modifier_bits(modifiers: Modifiers) -> u8 {
    let mut bits = 0;
    if modifiers.shift {
        bits |= 4;
    }
    if modifiers.alt {
        bits |= 8;
    }
    if modifiers.ctrl {
        bits |= 16;
    }
    bits
}

fn terminal_mouse_position(pos: Pos2, rect: Rect, size: TerminalSize) -> (usize, usize) {
    let x = (pos.x - rect.left()).max(0.0);
    let y = (pos.y - rect.top()).max(0.0);
    let col = ((x / size.cell_width).floor() as usize + 1).min(size.cols as usize);
    let line = ((y / size.cell_height).floor() as usize + 1).min(size.lines as usize);
    (col.max(1), line.max(1))
}

fn key_event_sequence(key: Key, modifiers: Modifiers) -> Option<&'static str> {
    if modifiers.mac_cmd {
        return None;
    }
    match key {
        Key::Enter => Some("\r"),
        Key::Tab => Some("\t"),
        Key::Backspace => Some("\x7f"),
        Key::Escape => Some("\x1b"),
        Key::ArrowUp => Some("\x1b[A"),
        Key::ArrowDown => Some("\x1b[B"),
        Key::ArrowRight => Some("\x1b[C"),
        Key::ArrowLeft => Some("\x1b[D"),
        Key::Home => Some("\x1b[H"),
        Key::End => Some("\x1b[F"),
        Key::Insert => Some("\x1b[2~"),
        Key::Delete => Some("\x1b[3~"),
        Key::PageUp => Some("\x1b[5~"),
        Key::PageDown => Some("\x1b[6~"),
        _ => None,
    }
}

fn meta_key_event_sequence(key: Key, modifiers: Modifiers) -> Option<Vec<u8>> {
    if !modifiers.alt || modifiers.ctrl || modifiers.mac_cmd {
        return None;
    }
    let byte = printable_key_byte(key, modifiers.shift)?;
    Some(vec![0x1b, byte])
}

fn printable_key_byte(key: Key, shift: bool) -> Option<u8> {
    let byte = match key {
        Key::Space => b' ',
        Key::A => letter_byte(b'a', shift),
        Key::B => letter_byte(b'b', shift),
        Key::C => letter_byte(b'c', shift),
        Key::D => letter_byte(b'd', shift),
        Key::E => letter_byte(b'e', shift),
        Key::F => letter_byte(b'f', shift),
        Key::G => letter_byte(b'g', shift),
        Key::H => letter_byte(b'h', shift),
        Key::I => letter_byte(b'i', shift),
        Key::J => letter_byte(b'j', shift),
        Key::K => letter_byte(b'k', shift),
        Key::L => letter_byte(b'l', shift),
        Key::M => letter_byte(b'm', shift),
        Key::N => letter_byte(b'n', shift),
        Key::O => letter_byte(b'o', shift),
        Key::P => letter_byte(b'p', shift),
        Key::Q => letter_byte(b'q', shift),
        Key::R => letter_byte(b'r', shift),
        Key::S => letter_byte(b's', shift),
        Key::T => letter_byte(b't', shift),
        Key::U => letter_byte(b'u', shift),
        Key::V => letter_byte(b'v', shift),
        Key::W => letter_byte(b'w', shift),
        Key::X => letter_byte(b'x', shift),
        Key::Y => letter_byte(b'y', shift),
        Key::Z => letter_byte(b'z', shift),
        Key::Num0 => b'0',
        Key::Num1 => b'1',
        Key::Num2 => b'2',
        Key::Num3 => b'3',
        Key::Num4 => b'4',
        Key::Num5 => b'5',
        Key::Num6 => b'6',
        Key::Num7 => b'7',
        Key::Num8 => b'8',
        Key::Num9 => b'9',
        Key::Colon => b':',
        Key::Comma => b',',
        Key::Backslash => b'\\',
        Key::Slash => b'/',
        Key::Pipe => b'|',
        Key::Questionmark => b'?',
        Key::Exclamationmark => b'!',
        Key::OpenBracket => b'[',
        Key::CloseBracket => b']',
        Key::OpenCurlyBracket => b'{',
        Key::CloseCurlyBracket => b'}',
        Key::Backtick => b'`',
        Key::Minus => b'-',
        Key::Period => b'.',
        Key::Plus => b'+',
        Key::Equals => b'=',
        Key::Semicolon => b';',
        Key::Quote => b'\'',
        _ => return None,
    };
    Some(byte)
}

fn letter_byte(lowercase: u8, shift: bool) -> u8 {
    if shift {
        lowercase.to_ascii_uppercase()
    } else {
        lowercase
    }
}

fn control_key_byte(key: Key, modifiers: Modifiers) -> Option<u8> {
    if !modifiers.ctrl || modifiers.mac_cmd {
        return None;
    }
    let offset = match key {
        Key::A => 1,
        Key::B => 2,
        Key::C => 3,
        Key::D => 4,
        Key::E => 5,
        Key::F => 6,
        Key::G => 7,
        Key::H => 8,
        Key::I => 9,
        Key::J => 10,
        Key::K => 11,
        Key::L => 12,
        Key::M => 13,
        Key::N => 14,
        Key::O => 15,
        Key::P => 16,
        Key::Q => 17,
        Key::R => 18,
        Key::S => 19,
        Key::T => 20,
        Key::U => 21,
        Key::V => 22,
        Key::W => 23,
        Key::X => 24,
        Key::Y => 25,
        Key::Z => 26,
        _ => return None,
    };
    Some(offset)
}

fn enable_terminal_ime(ui: &Ui, backend: &TerminalBackend, rect: egui::Rect) {
    let cursor_rect = terminal_ime_cursor_rect(backend, rect).unwrap_or_else(|| {
        egui::Rect::from_min_size(rect.left_top(), Vec2::new(1.0, backend.size.cell_height))
    });
    ui.ctx().output_mut(|output| {
        output.ime = Some(egui::output::IMEOutput {
            rect: cursor_rect,
            cursor_rect,
        });
    });
}

/// Detects copy requests that should copy terminal selection text.
fn terminal_selection_copy_requested(ui: &Ui) -> bool {
    ui.input(|input| terminal_selection_copy_requested_from_events(&input.events, input.modifiers))
}

/// Detects terminal-selection copy without stealing Ctrl+C interrupts.
fn terminal_selection_copy_requested_from_events(
    events: &[Event],
    active_modifiers: Modifiers,
) -> bool {
    events.iter().any(|event| match event {
        egui::Event::Copy => terminal_selection_copy_modifiers(active_modifiers),
        egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => terminal_selection_copy_key_event(*key, *modifiers),
        _ => false,
    })
}

/// Detects platform copy events that should copy terminal selection.
fn terminal_selection_copy_modifiers(modifiers: Modifiers) -> bool {
    // 触发条件：Linux/Windows 上 Ctrl+C 可同时带 command alias。
    // 不能把它按平台 Copy 处理：终端里 Ctrl+C 的主语义是 ETX。
    // 防止回归：有选区时 Ctrl+C 只复制、不再中断子进程。
    if modifiers.alt {
        return false;
    }
    if modifiers.ctrl {
        return modifiers.shift && !modifiers.mac_cmd;
    }
    modifiers.mac_cmd || modifiers.command
}

/// Detects key copy chords that should copy terminal selection.
fn terminal_selection_copy_key_event(key: Key, modifiers: Modifiers) -> bool {
    terminal_copy_key_event(key, modifiers)
        || (key == Key::C
            && !modifiers.ctrl
            && !modifiers.alt
            && (modifiers.mac_cmd || modifiers.command))
}

/// Removes padding cells from the end of a copied terminal line.
fn trim_trailing_spaces(text: &mut String) {
    let trimmed_len = text.trim_end_matches(' ').len();
    text.truncate(trimmed_len);
}

/// 返回当前 terminal 光标对应的 IME 候选窗锚点。
fn terminal_ime_cursor_rect(backend: &TerminalBackend, terminal_rect: egui::Rect) -> Option<Rect> {
    let term = backend.term.lock();
    let content = term.renderable_content();
    let display_offset = content.display_offset as i32;
    let line = content.cursor.point.line.0 + display_offset;
    if line < 0 || line >= backend.size.lines as i32 {
        return None;
    }
    let column = content
        .cursor
        .point
        .column
        .0
        .min(backend.size.cols as usize);
    let x = terminal_rect.left() + backend.size.cell_width * column as f32;
    let y = terminal_rect.top() + backend.size.cell_height * line as f32;
    Some(Rect::from_min_size(
        Pos2::new(x, y),
        Vec2::new(1.0, backend.size.cell_height.max(1.0)),
    ))
}

fn ime_commit_texts(events: &[egui::Event]) -> impl Iterator<Item = String> + '_ {
    events.iter().filter_map(|event| match event {
        egui::Event::Ime(egui::ImeEvent::Commit(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    })
}

fn ime_commit_texts_without_text_duplicates(events: &[egui::Event]) -> Vec<String> {
    ime_commit_texts(events)
        .filter(|commit| {
            !events
                .iter()
                .any(|event| terminal_text_event_matches(event, commit))
        })
        .collect()
}

fn terminal_text_event_matches(event: &egui::Event, value: &str) -> bool {
    matches!(event, egui::Event::Text(text) if text == value)
}

/// 构建嵌入式 terminal surface 使用的 alacritty 配置。
fn terminal_config() -> term::Config {
    // 触发条件：直接使用 alacritty_terminal 时需要显式控制配置。
    // 不能用 default()：alacritty 默认保留 10k scrollback，内存成本过高。
    // 防止空闲 agent/terminal surface 长期持有大 grid history。
    term::Config {
        scrolling_history: TERMINAL_SCROLLBACK_HISTORY_LINES,
        // 触发条件：Helix/现代 TUI 通过 kitty keyboard protocol 请求
        // Super/Cmd、Ctrl+Shift 等传统 PTY 表达不了的组合键。
        // 不能只看 TermMode：alacritty 默认会忽略协议请求，mode 不会生效。
        // 防止回归：Cmd+[ 这类配置好的快捷键在嵌入 terminal 中无响应。
        kitty_keyboard: true,
        ..term::Config::default()
    }
}

/// 只把保存的 workspace terminal 文本快照恢复到模拟器。
fn restore_terminal_history(term: &Arc<FairMutex<Term<TerminalEventProxy>>>, history: &str) {
    if history.is_empty() {
        return;
    }
    let mut parser: Processor<StdSyncHandler> = Processor::new();
    let mut term = term.lock();
    let restored = restored_terminal_history_bytes(history);
    parser.advance(&mut *term, restored.as_bytes());
    term.scroll_display(Scroll::Bottom);
}

/// 把保存的纯文本快照转换成终端回放可用的换行序列。
fn restored_terminal_history_bytes(history: &str) -> String {
    // 触发条件：从磁盘恢复 workspace terminal 的纯文本历史。
    // 不能直接写 \n：终端 LF 只下移，不会回到第 0 列。
    // 防止恢复后每一行继承上一行光标列，出现阶梯式错位。
    history
        .trim_end_matches('\n')
        .lines()
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n"
}

/// 把 workspace terminal 的 scrollback 和可见屏幕导出为纯文本。
fn terminal_history_snapshot(backend: &TerminalBackend) -> String {
    let term = backend.term.lock();
    let grid = term.grid();
    let mut lines = Vec::new();
    for line in grid.topmost_line().0..=grid.bottommost_line().0 {
        let mut text = String::new();
        for column in 0..grid.columns() {
            let cell = &grid[Line(line)][Column(column)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            text.push(cell.c);
        }
        trim_trailing_spaces(&mut text);
        lines.push(text);
    }
    while lines.first().is_some_and(String::is_empty) {
        lines.remove(0);
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

/// Renders a borrowed alacritty terminal view directly into egui.
fn terminal_view(
    ui: &mut Ui,
    backend: &mut TerminalBackend,
    size: Vec2,
    mode: gui_theme::ThemeMode,
    font_id: FontId,
    request_focus: bool,
    accept_input: bool,
    workspace_root: &Path,
) -> egui::Response {
    let sense = if accept_input {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if request_focus {
        response.request_focus();
    } else {
        response.surrender_focus();
    }

    let font_measure = terminal_font_measure(ui, &font_id);
    backend.resize(size, font_measure);
    if accept_input {
        handle_terminal_pointer(ui, backend, rect, &response);
        update_terminal_mouse_cursor(ui, backend, &response);
    }
    paint_terminal(ui, backend, rect, mode, &font_id);
    if accept_input {
        paint_terminal_click_hover(ui, backend, rect, &response, workspace_root);
    }
    response
}

/// Updates the platform cursor according to terminal mouse reporting mode.
fn update_terminal_mouse_cursor(ui: &Ui, backend: &TerminalBackend, response: &egui::Response) {
    if !response.hovered() {
        return;
    }
    let mouse_mode = backend.term.lock().mode().intersects(TermMode::MOUSE_MODE);
    if mouse_mode {
        ui.ctx().set_cursor_icon(CursorIcon::Default);
    } else {
        ui.ctx().set_cursor_icon(CursorIcon::Text);
    }
}

/// Converts remaining bell flash duration to overlay alpha.
fn terminal_bell_flash_alpha(remaining: Duration) -> u8 {
    let total = TERMINAL_BELL_FLASH_DURATION.as_secs_f32();
    let fraction = (remaining.as_secs_f32() / total).clamp(0.0, 1.0);
    (48.0 * fraction).round() as u8
}

/// 终端字体测量结果，适用于 PTY 尺寸和绘制策略共用。
struct TerminalFontMeasure {
    /// 单个终端 cell 的 egui 尺寸。
    cell_size: Vec2,
    /// 当前 ASCII 字体是否可以安全合并成 text run。
    ascii_text_runs: bool,
}

/// 测量终端字体 cell，适用于 PTY resize 和 ASCII 绘制合并判断。
fn terminal_font_measure(ui: &Ui, font_id: &FontId) -> TerminalFontMeasure {
    let ascii_galley = ui.painter().layout_no_wrap(
        "0123456789abcdefghijklmnopqrstuvwxyz".to_string(),
        font_id.clone(),
        egui::Color32::WHITE,
    );
    let primary = ui.fonts(|fonts| TerminalFontMetrics {
        ascii_sample_width: ascii_galley.rect.width(),
        ascii_sample_chars: 36,
        space_width: fonts.glyph_width(font_id, ' '),
        row_height: fonts.row_height(font_id),
    });
    let cjk_galley = ui.painter().layout_no_wrap(
        "中文，了".to_string(),
        font_id.clone(),
        egui::Color32::WHITE,
    );
    terminal_cell_measure_from_metrics(
        primary,
        CjkTerminalFontMetrics {
            sample_width: cjk_galley.rect.width(),
            sample_chars: 4,
            sample_height: cjk_galley.rect.height(),
        },
    )
}

/// egui 字体库里读取到的主终端字体指标。
struct TerminalFontMetrics {
    /// 当前终端字体中混合 ASCII 样本的总宽度。
    ascii_sample_width: f32,
    /// 混合 ASCII 样本包含的字符数。
    ascii_sample_chars: usize,
    /// 终端空白格使用的空格字形宽度。
    space_width: f32,
    /// 主终端字体族上报的行高。
    row_height: f32,
}

/// 通过真实布局样本测得的 CJK fallback 指标。
struct CjkTerminalFontMetrics {
    /// CJK 样本布局后的总宽度。
    sample_width: f32,
    /// CJK 样本包含的字符数。
    sample_chars: usize,
    /// CJK 样本布局后的高度。
    sample_height: f32,
}

/// 组合主字体和 CJK fallback 指标，适用于终端 cell 测量。
fn terminal_cell_measure_from_metrics(
    primary: TerminalFontMetrics,
    cjk: CjkTerminalFontMetrics,
) -> TerminalFontMeasure {
    // 触发条件：Agent 终端同时启用 ASCII 等宽字体和中文 fallback 字体。
    // 不能用 CJK fallback 宽度抬高列宽：PTY 会收到偏小的列数，
    // 子进程的原始输出会按错误宽度重排并插入大量对齐空格。
    // 也不能用 W 这种极宽字母：彩色 token 分段绘制时会产生假空格。
    // 防止回归：中文仍可用较高行高显示，但 ASCII 列宽贴近真实单列。
    let _ = cjk.sample_width;
    let _ = cjk.sample_chars;
    let ascii_cell_width = primary.ascii_sample_width / primary.ascii_sample_chars.max(1) as f32;
    let cell_width = ascii_cell_width.max(primary.space_width).max(1.0);
    TerminalFontMeasure {
        cell_size: Vec2::new(
            cell_width,
            primary.row_height.max(cjk.sample_height).ceil().max(1.0),
        ),
        // 触发条件：terminal cell 可以拥有独立前景色、反显和装饰。
        // 不能跨 cell 合并 text run：egui 单段文本只有一套绘制样式。
        // 防止回归：Agent 和 workspace terminal 的 ANSI 颜色被合并路径吃掉。
        ascii_text_runs: false,
    }
}

/// Handles pointer selection and scroll against the borrowed terminal backend.
fn handle_terminal_pointer(
    ui: &Ui,
    backend: &mut TerminalBackend,
    rect: Rect,
    response: &egui::Response,
) {
    let term_mode = *backend.term.lock().mode();
    // Trigger: pointer selection is implemented locally when the child app is
    // not responsible for selection.
    // Why not rely on drag_started: egui double/triple clicks are plain clicks,
    // so drag selection never runs and the terminal appears inert.
    // Prevents: semantic and line selection regressions for copied agent text.
    if response.triple_clicked_by(PointerButton::Primary) {
        if let Some(pos) = response.interact_pointer_pos() {
            backend.start_selection_with_type(rect, pos, SelectionType::Lines);
        }
        return;
    }
    if response.double_clicked_by(PointerButton::Primary) {
        if let Some(pos) = response.interact_pointer_pos() {
            backend.start_selection_with_type(rect, pos, SelectionType::Semantic);
        }
        return;
    }
    if term_mode.intersects(TermMode::MOUSE_MODE) {
        let bytes = ui.input(|state| {
            let input = TerminalPointerInput {
                modifiers: state.modifiers,
                pointer_pos: state.pointer.interact_pos(),
                primary_down: state.pointer.primary_down(),
                secondary_down: state.pointer.secondary_down(),
                middle_down: state.pointer.middle_down(),
            };
            terminal_mouse_input_bytes(&state.events, &input, rect, backend.size, term_mode)
        });
        if !bytes.is_empty() {
            backend.write_bytes(bytes);
        }
        return;
    }

    if response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            backend.start_selection(rect, pos);
        }
    }
    if response.dragged() {
        if let Some(pos) = response.interact_pointer_pos() {
            backend.update_selection(rect, pos);
        }
    }
    if response.hovered() {
        let scroll_y = ui.input(|input| input.smooth_scroll_delta.y);
        let lines = (scroll_y / backend.size.cell_height).round() as i32;
        backend.scroll(lines);
    }
}

/// Resolves a clicked Agent terminal URL or `path:line` token.
fn terminal_output_click(
    backend: &TerminalBackend,
    rect: Rect,
    response: &egui::Response,
    workspace_root: &Path,
) -> Option<TerminalOutputClick> {
    if !response.clicked_by(PointerButton::Primary)
        || response.double_clicked()
        || response.triple_clicked()
    {
        return None;
    }
    let pos = response.interact_pointer_pos()?;
    if let Some(matched) = terminal_url_match_at_pos(backend, rect, pos) {
        return Some(TerminalOutputClick::Url(matched.url));
    }
    let matched = terminal_file_match_at_pos(backend, rect, pos, workspace_root)?;
    let path = resolve_terminal_file_path(workspace_root, &matched.path);
    terminal_output_click_for_resolved_path(path, matched.line, matched.end_line)
}

/// 将已解析的终端路径转成纯语法点击候选。
fn terminal_output_click_for_resolved_path(
    path: PathBuf,
    line: usize,
    end_line: Option<usize>,
) -> Option<TerminalOutputClick> {
    Some(TerminalOutputClick::PathCandidate(TerminalFileLineClick {
        path,
        line,
        end_line,
    }))
}

/// 在后台完成终端路径 metadata 分类。
pub(super) fn classify_terminal_output_path_click(
    click: TerminalFileLineClick,
) -> TerminalOutputClick {
    let path = click.path.clone();
    if path.is_dir() || (path.is_file() && is_terminal_image_path(&path)) {
        return TerminalOutputClick::RevealPath(path);
    }
    TerminalOutputClick::FileLine(click)
}

/// Paints an underline over hovered clickable Agent output.
fn paint_terminal_click_hover(
    ui: &Ui,
    backend: &TerminalBackend,
    rect: Rect,
    response: &egui::Response,
    workspace_root: &Path,
) {
    if !response.hovered() {
        return;
    }
    let Some(pos) = ui.input(|input| input.pointer.hover_pos()) else {
        return;
    };
    if let Some(matched) = terminal_url_match_at_pos(backend, rect, pos) {
        paint_terminal_match_underline(
            ui,
            backend,
            rect,
            matched.visible_line,
            matched.start_column,
            matched.end_column,
        );
        return;
    }
    let Some(matched) = terminal_file_match_at_pos(backend, rect, pos, workspace_root) else {
        return;
    };
    paint_terminal_match_underline(
        ui,
        backend,
        rect,
        matched.visible_line,
        matched.start_column,
        matched.end_column,
    );
}

/// Paints the underline for one clickable terminal token.
fn paint_terminal_match_underline(
    ui: &Ui,
    backend: &TerminalBackend,
    rect: Rect,
    visible_line: i32,
    start_column: usize,
    end_column: usize,
) {
    let y = rect.top() + backend.size.cell_height * (visible_line as f32 + 1.0) - 2.0;
    let x1 = rect.left() + backend.size.cell_width * start_column as f32;
    let x2 = rect.left() + backend.size.cell_width * end_column as f32;
    ui.painter_at(rect).line_segment(
        [Pos2::new(x1, y), Pos2::new(x2, y)],
        Stroke::new(1.0, gui_theme::primary()),
    );
}

/// Resolves a terminal position to an http/https URL token match.
fn terminal_url_match_at_pos(
    backend: &TerminalBackend,
    rect: Rect,
    pos: Pos2,
) -> Option<TerminalUrlMatch> {
    if !rect.contains(pos) {
        return None;
    }
    let column = ((pos.x - rect.left()) / backend.size.cell_width).floor() as usize;
    let visible_line = ((pos.y - rect.top()) / backend.size.cell_height).floor() as i32;
    if visible_line < 0 {
        return None;
    }
    let line = terminal_visible_line_text(backend, visible_line)?;
    url_match_at_column(&line, column, visible_line)
}

/// Reads one visible terminal line as plain text.
fn terminal_visible_line_text(backend: &TerminalBackend, visible_line: i32) -> Option<String> {
    let term = backend.term.lock();
    let content = term.renderable_content();
    let display_offset = content.display_offset as i32;
    let mut cells = vec![' '; backend.size.cols as usize];
    let mut seen = false;
    for indexed in content.display_iter {
        let line = indexed.point.line.0 + display_offset;
        if line != visible_line {
            continue;
        }
        let cell = indexed.cell;
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        let column = indexed.point.column.0;
        if let Some(slot) = cells.get_mut(column) {
            *slot = cell.c;
            seen = true;
        }
    }
    if !seen {
        return None;
    }
    let mut text = cells.into_iter().collect::<String>();
    trim_trailing_spaces(&mut text);
    Some(text)
}

/// 一行可见 terminal 文本及对应 cell 样式。
#[derive(Debug, Clone)]
struct TerminalVisibleLine {
    /// 去掉右侧空白后的可见文本。
    text: String,
    /// 和 `text` 按 terminal column 对齐的字符。
    chars: Vec<char>,
    /// 和 `text` 按 terminal column 对齐的样式标记。
    flags: Vec<Flags>,
}

/// Reads visible terminal lines and cursor line for lightweight composer scraping.
fn terminal_visible_text_snapshot(
    backend: &TerminalBackend,
) -> (Vec<TerminalVisibleLine>, Option<usize>) {
    let term = backend.term.lock();
    let content = term.renderable_content();
    let display_offset = content.display_offset as i32;
    let line_count = backend.size.lines as usize;
    let col_count = backend.size.cols as usize;
    let mut cells = vec![vec![' '; col_count]; line_count];
    let mut flags = vec![vec![Flags::empty(); col_count]; line_count];
    let mut seen = vec![false; line_count];
    for indexed in content.display_iter {
        let visible_line = indexed.point.line.0 + display_offset;
        if visible_line < 0 || visible_line >= backend.size.lines as i32 {
            continue;
        }
        let cell = indexed.cell;
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        let line = visible_line as usize;
        let column = indexed.point.column.0;
        if let Some(slot) = cells
            .get_mut(line)
            .and_then(|line_cells| line_cells.get_mut(column))
        {
            *slot = cell.c;
            if let Some(flag_slot) = flags
                .get_mut(line)
                .and_then(|line_flags| line_flags.get_mut(column))
            {
                *flag_slot = cell.flags;
            }
            seen[line] = true;
        }
    }
    let lines = cells
        .into_iter()
        .zip(flags)
        .zip(seen)
        .map(|((mut line_cells, mut line_flags), line_seen)| {
            if !line_seen {
                TerminalVisibleLine {
                    text: String::new(),
                    chars: Vec::new(),
                    flags: Vec::new(),
                }
            } else {
                while line_cells.last().is_some_and(|ch| *ch == ' ') {
                    line_cells.pop();
                    line_flags.pop();
                }
                let text = line_cells.iter().collect::<String>();
                TerminalVisibleLine {
                    text,
                    chars: line_cells,
                    flags: line_flags,
                }
            }
        })
        .collect::<Vec<_>>();
    let cursor_line = content.cursor.point.line.0 + display_offset;
    let cursor_line = (cursor_line >= 0 && cursor_line < backend.size.lines as i32)
        .then_some(cursor_line as usize);
    (lines, cursor_line)
}

/// Extracts the active Agent composer draft from the terminal screen buffer.
fn terminal_agent_input_text_snapshot(backend: &TerminalBackend) -> Option<String> {
    let (lines, cursor_line) = terminal_visible_text_snapshot(backend);
    let cursor_line = cursor_line?;
    let prompt_line = terminal_find_agent_prompt_line(&lines, cursor_line)?;
    if terminal_prompt_belongs_to_transient_view(&lines, prompt_line) {
        return None;
    }
    let mut segments = Vec::new();
    let mut all_segments_dim = true;
    for line in lines.iter().skip(prompt_line).take(64) {
        if terminal_composer_stop_line(line, segments.is_empty()) {
            break;
        }
        if let Some((segment, segment_is_dim)) =
            terminal_composer_line_text(line, segments.is_empty())
        {
            if !segment.trim().is_empty() && !segment_is_dim {
                all_segments_dim = false;
            }
            segments.push(segment);
        }
    }
    let mut text = segments.join("\n");
    trim_trailing_blank_lines(&mut text);
    let trimmed = text.trim();
    if trimmed.is_empty() || all_segments_dim {
        None
    } else {
        Some(text)
    }
}

/// Split result for text with Codex image placeholders.
struct ImagePlaceholderSegments {
    /// Plain text pieces around image placeholders.
    texts: Vec<String>,
    /// Placeholder labels in encounter order.
    placeholders: Vec<String>,
}

/// Splits text into plain segments and `[Image #N]` placeholders.
fn image_placeholder_segments(text: &str) -> Option<ImagePlaceholderSegments> {
    let mut texts = Vec::new();
    let mut placeholders = Vec::new();
    let mut cursor = 0;
    while cursor < text.len() {
        let Some(relative_start) = text[cursor..].find("[Image #") else {
            texts.push(text[cursor..].to_string());
            return Some(ImagePlaceholderSegments {
                texts,
                placeholders,
            });
        };
        let start = cursor + relative_start;
        texts.push(text[cursor..start].to_string());
        let after_prefix = start + "[Image #".len();
        let end = image_placeholder_end(text, after_prefix)?;
        placeholders.push(text[start..end].to_string());
        cursor = end;
    }
    texts.push(String::new());
    Some(ImagePlaceholderSegments {
        texts,
        placeholders,
    })
}

/// Builds terminal edit bytes that keep placeholders and rewrite surrounding text.
fn rewrite_text_preserving_placeholders_bytes(source: &[String], target: &[String]) -> Vec<u8> {
    if source.len() != target.len() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    // Ctrl+E moves to current line end; repeated at EOL advances to next line.
    push_repeated(&mut bytes, b"\x05", text_line_count(source) + 1);
    for index in (0..source.len()).rev() {
        // 触发条件：Codex composer 里有 `[Image #N]` 原子 element。
        // 不能用 Backspace 贴着 element 删除：源码里 delete_backward 会把
        // prev_atomic_boundary 落到 element start，replace_range 随后扩展并删图。
        // 防止回归：Cmd/Alt+N 应用翻译时把真实图片 attachment 变成普通文本或删掉。
        push_repeated(&mut bytes, b"\x1b[D", source[index].chars().count());
        push_repeated(&mut bytes, b"\x1b[3~", source[index].chars().count());
        bytes.extend(bracketed_paste_bytes(&target[index]));
        if index > 0 {
            if !text_segments_need_rewrite_before(source, target, index) {
                break;
            }
            push_repeated(&mut bytes, b"\x1b[D", target[index].chars().count());
            bytes.extend_from_slice(b"\x1b[D");
        }
    }
    push_repeated(&mut bytes, b"\x05", text_line_count(target) + 1);
    bytes
}

/// Returns whether any earlier text segment still needs cursor movement/editing.
fn text_segments_need_rewrite_before(source: &[String], target: &[String], end: usize) -> bool {
    source[..end]
        .iter()
        .chain(target[..end].iter())
        .any(|segment| !segment.is_empty())
}

/// Counts logical lines across all non-placeholder text segments.
fn text_line_count(texts: &[String]) -> usize {
    texts
        .iter()
        .map(|text| text.chars().filter(|ch| *ch == '\n').count())
        .sum::<usize>()
        + 1
}

/// Appends `seq` to `bytes` `count` times.
fn push_repeated(bytes: &mut Vec<u8>, seq: &[u8], count: usize) {
    for _ in 0..count {
        bytes.extend_from_slice(seq);
    }
}

/// Returns bracketed paste bytes so Codex receives text as literal input.
fn bracketed_paste_bytes(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() + 12);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~");
    bytes
}

/// Returns the end byte index for `[Image #N]` starting after `#`.
fn image_placeholder_end(text: &str, after_prefix: usize) -> Option<usize> {
    let mut saw_digit = false;
    for (offset, ch) in text[after_prefix..].char_indices() {
        if ch.is_ascii_digit() {
            saw_digit = true;
            continue;
        }
        return (saw_digit && ch == ']').then_some(after_prefix + offset + ch.len_utf8());
    }
    None
}

/// Detects Agent composer submit bytes sent through normal or kitty keyboard mode.
pub(super) fn terminal_agent_input_submit_bytes(bytes: &[u8]) -> bool {
    bytes.contains(&b'\r')
        || bytes
            .windows(b"\x1b[13u".len())
            .any(|part| part == b"\x1b[13u")
}

/// Finds the Codex prompt row nearest to the active cursor line.
fn terminal_find_agent_prompt_line(
    lines: &[TerminalVisibleLine],
    cursor_line: usize,
) -> Option<usize> {
    let lower_bound = cursor_line.saturating_sub(32);
    (lower_bound..=cursor_line).rev().find(|index| {
        lines
            .get(*index)
            .is_some_and(terminal_line_has_agent_prompt)
    })
}

/// Detects the first visible composer line rendered by Codex.
fn terminal_line_has_agent_prompt(line: &TerminalVisibleLine) -> bool {
    let Some((leading_spaces, prompt)) = terminal_prompt_column(line) else {
        return false;
    };
    leading_spaces <= 3 && (prompt == '›' || prompt == '!')
}

/// Returns whether the prompt belongs to a non-composer Codex transient view.
fn terminal_prompt_belongs_to_transient_view(
    lines: &[TerminalVisibleLine],
    prompt_line: usize,
) -> bool {
    let before_start = prompt_line.saturating_sub(8);
    lines[before_start..=prompt_line]
        .iter()
        .any(terminal_line_is_codex_transient_view_marker)
        || lines
            .iter()
            .skip(prompt_line.saturating_add(1))
            .take(8)
            .any(terminal_line_is_codex_transient_view_marker)
}

/// Detects Codex modal/list views that also render a selected row with `›`.
fn terminal_line_is_codex_transient_view_marker(line: &TerminalVisibleLine) -> bool {
    let text = line.text.trim();
    text == "Press enter to continue"
        || text.contains("Update available!")
        || text.contains("Skip until next version")
        || text.contains("Update now")
}

/// Returns whether the line should end composer text collection.
fn terminal_composer_stop_line(line: &TerminalVisibleLine, first_line: bool) -> bool {
    if first_line {
        return false;
    }
    line.text.trim().is_empty()
}

/// Strips Codex composer chrome from one visible input line.
fn terminal_composer_line_text(
    line: &TerminalVisibleLine,
    first_line: bool,
) -> Option<(String, bool)> {
    if line.text.is_empty() {
        return None;
    }
    let (text_start, text) = if first_line {
        terminal_strip_composer_prompt(line)?
    } else {
        let text_start = if line.chars.starts_with(&[' ', ' ']) {
            2
        } else {
            0
        };
        (text_start, line.chars[text_start..].iter().collect())
    };
    Some((text, terminal_line_text_is_dim(line, text_start)))
}

/// Returns the visible Agent composer line rect for popup anchoring.
fn terminal_agent_input_rect(backend: &TerminalBackend, terminal_rect: Rect) -> Option<Rect> {
    let (lines, cursor_line) = terminal_visible_text_snapshot(backend);
    let cursor_line = cursor_line?;
    let prompt_line = terminal_find_agent_prompt_line(&lines, cursor_line)?;
    if terminal_prompt_belongs_to_transient_view(&lines, prompt_line) {
        return None;
    }
    let top = terminal_rect.top() + backend.size.cell_height * prompt_line as f32;
    Some(Rect::from_min_size(
        egui::pos2(terminal_rect.left(), top),
        Vec2::new(terminal_rect.width(), backend.size.cell_height),
    ))
}

/// Returns the visible Codex composer prompt column and glyph.
fn terminal_prompt_column(line: &TerminalVisibleLine) -> Option<(usize, char)> {
    line.chars
        .iter()
        .enumerate()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(column, ch)| (column, *ch))
}

/// Removes the live composer prefix used by Codex and shell mode prompts.
fn terminal_strip_composer_prompt(line: &TerminalVisibleLine) -> Option<(usize, String)> {
    let (prompt_column, prompt) = terminal_prompt_column(line)?;
    if prompt != '›' && prompt != '!' {
        return None;
    }
    let mut text_start = prompt_column + 1;
    while line
        .chars
        .get(text_start)
        .is_some_and(|ch| ch.is_whitespace())
    {
        text_start += 1;
    }
    let text = line.chars[text_start..].iter().collect();
    // 触发条件：Codex TUI 的 composer 只把 `›`/`!` 画在首行。
    // 不能直接复制整行：那会把 UI prompt 当作用户要翻译的内容。
    // 防止回归：快捷翻译结果混入终端 chrome 或空输入 placeholder。
    Some((text_start, text))
}

/// Detects Ratatui dimmed placeholder text rendered by Codex.
fn terminal_line_text_is_dim(line: &TerminalVisibleLine, text_start: usize) -> bool {
    let mut saw_text = false;
    for column in text_start..line.chars.len() {
        let Some(ch) = line.chars.get(column) else {
            continue;
        };
        if ch.is_whitespace() {
            continue;
        }
        saw_text = true;
        if !line
            .flags
            .get(column)
            .is_some_and(|flags| flags.intersects(Flags::DIM | Flags::DIM_BOLD))
        {
            return false;
        }
    }
    saw_text
}

/// Removes trailing blank lines while preserving intentional interior newlines.
fn trim_trailing_blank_lines(text: &mut String) {
    while text.ends_with('\n') || text.ends_with(' ') || text.ends_with('\t') {
        let Some(ch) = text.pop() else {
            break;
        };
        if ch == '\n' && !text.ends_with('\n') {
            break;
        }
    }
}

/// Finds an http/https URL token and returns its span.
fn url_match_at_column(line: &str, column: usize, visible_line: i32) -> Option<TerminalUrlMatch> {
    let mut search_start = 0usize;
    while search_start < line.len() {
        let http = line[search_start..].find("http://");
        let https = line[search_start..].find("https://");
        let relative_start = match (http, https) {
            (Some(http), Some(https)) => http.min(https),
            (Some(http), None) => http,
            (None, Some(https)) => https,
            (None, None) => return None,
        };
        let raw_start = search_start + relative_start;
        let raw_end = raw_start
            + line[raw_start..]
                .find(char::is_whitespace)
                .unwrap_or(line.len() - raw_start);
        let token = line[raw_start..raw_end].trim_end_matches(url_trim_char);
        let token_end = raw_start + token.len();
        let start_column = byte_to_terminal_column(line, raw_start);
        let end_column = byte_to_terminal_column(line, token_end);
        if !token.is_empty() && (start_column..end_column).contains(&column) {
            return Some(TerminalUrlMatch {
                url: token.to_string(),
                start_column,
                end_column,
                visible_line,
            });
        }
        search_start = raw_end.saturating_add(1);
    }
    None
}

/// Finds a `path:line` token that contains or sits near the clicked terminal column.
fn file_line_token_at_column(line: &str, column: usize) -> Option<TerminalFileLineClick> {
    file_line_match_at_column(line, column, 0).map(|matched| TerminalFileLineClick {
        path: matched.path,
        line: matched.line,
        end_line: matched.end_line,
    })
}

/// Finds a `path:line` or `path:start-end` token and returns its span.
fn file_line_match_at_column(
    line: &str,
    column: usize,
    visible_line: i32,
) -> Option<TerminalFileLineMatch> {
    let bytes = line.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        while index < bytes.len() && !file_line_path_byte(bytes[index]) {
            index += 1;
        }
        let raw_start = index;
        while index < bytes.len() && file_line_path_byte(bytes[index]) {
            index += 1;
        }
        if raw_start == index || index >= bytes.len() || bytes[index] != b':' {
            index = index.saturating_add(1);
            continue;
        }
        let line_start = index + 1;
        let mut line_end = line_start;
        while line_end < bytes.len() && bytes[line_end].is_ascii_digit() {
            line_end += 1;
        }
        if line_start == line_end {
            index = line_end;
            continue;
        }
        let mut token_end = line_end;
        let mut end_line = None;
        if token_end < bytes.len() && bytes[token_end] == b'-' {
            let range_start = token_end + 1;
            let mut range_end = range_start;
            while range_end < bytes.len() && bytes[range_end].is_ascii_digit() {
                range_end += 1;
            }
            if range_start < range_end {
                end_line = line[range_start..range_end]
                    .parse::<usize>()
                    .ok()
                    .map(|line| line.max(1));
                token_end = range_end;
            }
        }
        let path = line[raw_start..index].trim_matches(file_line_trim_char);
        let trim_left = line[raw_start..index].len().saturating_sub(path.len());
        let start = raw_start + trim_left;
        let start_column = byte_to_terminal_column(line, start);
        let end_column = byte_to_terminal_column(line, token_end);
        if !file_line_click_hits_token(start_column, end_column, column) {
            index = token_end;
            continue;
        }
        if path.is_empty() {
            index = token_end;
            continue;
        }
        let line_no = line[line_start..line_end].parse::<usize>().ok()?.max(1);
        return Some(TerminalFileLineMatch {
            path: PathBuf::from(path),
            line: line_no,
            end_line,
            start_column,
            end_column,
            visible_line,
        });
    }
    None
}

/// Resolves a terminal position to either `path:line` or a bare file path.
fn terminal_file_match_at_pos(
    backend: &TerminalBackend,
    rect: Rect,
    pos: Pos2,
    workspace_root: &Path,
) -> Option<TerminalFileLineMatch> {
    if !rect.contains(pos) {
        return None;
    }
    let column = ((pos.x - rect.left()) / backend.size.cell_width).floor() as usize;
    let visible_line = ((pos.y - rect.top()) / backend.size.cell_height).floor() as i32;
    if visible_line < 0 {
        return None;
    }
    let line = terminal_visible_line_text(backend, visible_line)?;
    if let Some(matched) = file_line_match_at_column(&line, column, visible_line) {
        return Some(matched);
    }
    file_path_match_at_column(&line, column, visible_line, workspace_root)
}

/// Finds a bare file or directory path token and returns it as a line-one target.
fn file_path_match_at_column(
    line: &str,
    column: usize,
    visible_line: i32,
    workspace_root: &Path,
) -> Option<TerminalFileLineMatch> {
    let bytes = line.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        while index < bytes.len() && !file_line_path_byte(bytes[index]) {
            index += 1;
        }
        let raw_start = index;
        while index < bytes.len() && file_line_path_byte(bytes[index]) {
            index += 1;
        }
        if raw_start == index {
            index = index.saturating_add(1);
            continue;
        }
        let raw_token = &line[raw_start..index];
        let path = raw_token.trim_matches(file_line_trim_char);
        let trim_left = raw_token.len().saturating_sub(path.len());
        let start = raw_start + trim_left;
        let end = start + path.len();
        let start_column = byte_to_terminal_column(line, start);
        let end_column = byte_to_terminal_column(line, end);
        if !file_line_click_hits_token(start_column, end_column, column) {
            continue;
        }
        if !path_may_be_file_reference(path) {
            continue;
        }
        let path_buf = PathBuf::from(path);
        return Some(TerminalFileLineMatch {
            path: path_buf,
            line: 1,
            end_line: None,
            start_column,
            end_column,
            visible_line,
        });
    }
    None
}

/// Allows clicks on list markers or tiny whitespace immediately before a token.
fn file_line_click_hits_token(start: usize, end: usize, column: usize) -> bool {
    let forgiving_start = start.saturating_sub(2);
    (forgiving_start..end).contains(&column)
}

/// Converts a UTF-8 byte index into the terminal column represented by chars.
fn byte_to_terminal_column(line: &str, byte_index: usize) -> usize {
    line[..byte_index.min(line.len())].chars().count()
}

/// Filters bare tokens so ordinary words do not become file candidates.
fn path_may_be_file_reference(path: &str) -> bool {
    path.contains('/') || path.starts_with('.') || path.starts_with('~')
}

/// 判断终端路径是否是图片，图片点击交给文件管理器定位。
fn is_terminal_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png"
                    | "jpg"
                    | "jpeg"
                    | "gif"
                    | "webp"
                    | "bmp"
                    | "ico"
                    | "tif"
                    | "tiff"
                    | "heic"
                    | "heif"
                    | "avif"
                    | "svg"
            )
        })
        .unwrap_or(false)
}

/// Resolves a terminal output path against its terminal workspace root.
fn resolve_terminal_file_path(workspace_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    }
}

/// Returns whether a byte can be part of a simple file path token.
fn file_line_path_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'/' | b'.' | b'_' | b'-' | b'~' | b'@' | b'+' | b'=' | b'\\'
        )
}

/// Trims punctuation that often surrounds terminal path references.
fn file_line_trim_char(value: char) -> bool {
    matches!(
        value,
        '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ','
    )
}

/// Trims punctuation that often follows terminal URL references.
fn url_trim_char(value: char) -> bool {
    matches!(
        value,
        '"' | '\'' | '`' | ')' | ']' | '}' | '>' | ',' | '.' | ';'
    )
}

/// 绘制可见终端 cell，适用于避免复制 alacritty grid。
fn paint_terminal(
    ui: &Ui,
    backend: &TerminalBackend,
    rect: Rect,
    mode: gui_theme::ThemeMode,
    font_id: &FontId,
) {
    let paint_started_at = Instant::now();
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, gui_theme::bg_for(mode));

    let term = backend.term.lock();
    let content = term.renderable_content();
    let display_offset = content.display_offset as i32;
    let selection = content.selection;
    let cursor = content.cursor;
    let colors = content.colors;
    let mut run_line: Option<i32> = None;
    let mut run_start_column = 0usize;
    let mut run_next_column = 0usize;
    let mut run_fg = gui_theme::text();
    let mut run_text = String::with_capacity(backend.size.cols as usize);
    let mut visible_cells = 0u64;
    let mut text_cells = 0u64;
    let mut bg_cells = 0u64;
    let mut text_runs = 0u64;
    let mut joined_text_cells = 0u64;
    let mut cell_text_calls = 0u64;

    for indexed in content.display_iter {
        let cell = indexed.cell;
        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        let line = indexed.point.line.0 + display_offset;
        if line < 0 {
            continue;
        }
        let x = rect.left() + backend.size.cell_width * indexed.point.column.0 as f32;
        let y = rect.top() + backend.size.cell_height * line as f32;
        if y > rect.bottom() {
            continue;
        }
        visible_cells += 1;
        let cell_rect = Rect::from_min_size(
            Pos2::new(x, y),
            Vec2::new(
                cell_width(cell, backend.size.cell_width),
                backend.size.cell_height,
            ),
        );
        let selected = selection.is_some_and(|range| range.contains(indexed.point));
        let (fg, bg) = terminal_cell_colors(cell, selected, colors, mode);
        if bg != gui_theme::bg_for(mode) {
            bg_cells += 1;
            painter.rect_filled(cell_rect, 0.0, bg);
        }
        if cell.c != ' ' && cell.c != '\t' && !cell.flags.contains(Flags::HIDDEN) {
            text_cells += 1;
            text_cells += cell.zerowidth().map_or(0, |marks| marks.len() as u64);
        }
        if cursor.point == indexed.point && cursor.shape != CursorShape::Hidden {
            paint_terminal_cursor(&painter, cell_rect, cursor.shape, fg);
        }
        let can_join_run = backend.size.ascii_text_runs
            && terminal_cell_can_join_text_run(cell, cursor.point == indexed.point);
        if can_join_run {
            let column = indexed.point.column.0;
            let same_run = run_line == Some(line) && run_next_column == column && run_fg == fg;
            if !same_run {
                if paint_terminal_text_run(
                    &painter,
                    rect,
                    backend.size.cell_width,
                    backend.size.cell_height,
                    font_id,
                    run_line,
                    run_start_column,
                    run_fg,
                    &mut run_text,
                ) {
                    text_runs += 1;
                }
                run_line = Some(line);
                run_start_column = column;
                run_fg = fg;
            }
            run_text.push(cell.c);
            for mark in cell.zerowidth().unwrap_or_default() {
                run_text.push(*mark);
            }
            run_next_column = column + 1;
            joined_text_cells += 1;
        } else {
            if paint_terminal_text_run(
                &painter,
                rect,
                backend.size.cell_width,
                backend.size.cell_height,
                font_id,
                run_line,
                run_start_column,
                run_fg,
                &mut run_text,
            ) {
                text_runs += 1;
            }
            run_line = None;
            cell_text_calls += paint_terminal_cell_text(&painter, cell, x, y, font_id, fg);
        }
        paint_terminal_decoration(&painter, cell, cell_rect, fg);
    }
    if paint_terminal_text_run(
        &painter,
        rect,
        backend.size.cell_width,
        backend.size.cell_height,
        font_id,
        run_line,
        run_start_column,
        run_fg,
        &mut run_text,
    ) {
        text_runs += 1;
    }
    crate::gui::perf_log::count("terminal.paint");
    crate::gui::perf_log::add("terminal.paint_cells", visible_cells);
    crate::gui::perf_log::add("terminal.paint_text_cells", text_cells);
    crate::gui::perf_log::add("terminal.paint_bg_cells", bg_cells);
    crate::gui::perf_log::add("terminal.paint_text_runs", text_runs);
    crate::gui::perf_log::add("terminal.paint_joined_text_cells", joined_text_cells);
    crate::gui::perf_log::add("terminal.paint_cell_text_calls", cell_text_calls);
    if backend.size.ascii_text_runs {
        crate::gui::perf_log::count("terminal.paint_ascii_runs_enabled");
    } else {
        crate::gui::perf_log::count("terminal.paint_ascii_runs_disabled");
    }
    crate::gui::perf_log::duration_us("terminal.paint_us", paint_started_at.elapsed());
}

/// 判断一个 cell 是否能合并到 egui 文本段里。
fn terminal_cell_can_join_text_run(cell: &Cell, is_cursor: bool) -> bool {
    let _ = cell;
    let _ = is_cursor;
    // 触发条件：终端输出包含 ANSI 前景色、反显、下划线或光标格。
    // 不能合并成整段 text run：egui 会按一套样式绘制整段文本。
    // 防止回归：Agent 输出和 workspace terminal 的颜色被相邻 cell 吃掉。
    false
}

/// 绘制并清空一个连续终端文本段，适用于减少 egui text layout 次数。
fn paint_terminal_text_run(
    painter: &egui::Painter,
    rect: Rect,
    cell_width: f32,
    cell_height: f32,
    font_id: &FontId,
    line: Option<i32>,
    start_column: usize,
    color: egui::Color32,
    text: &mut String,
) -> bool {
    if text.trim().is_empty() {
        text.clear();
        return false;
    }
    let Some(line) = line else {
        text.clear();
        return false;
    };
    // 触发条件：终端单帧有大量可见字符。
    // 不能逐 cell 画 ASCII：egui 每次 text 都会单独布局。
    // 防止回归：Agent 输出高频刷新时产生海量 LayoutJob。
    painter.text(
        Pos2::new(
            rect.left() + cell_width * start_column as f32,
            rect.top() + cell_height * line as f32,
        ),
        Align2::LEFT_TOP,
        text.as_str(),
        font_id.clone(),
        color,
    );
    text.clear();
    true
}

/// 绘制不能安全合并的单个终端 cell。
fn paint_terminal_cell_text(
    painter: &egui::Painter,
    cell: &Cell,
    x: f32,
    y: f32,
    font_id: &FontId,
    color: egui::Color32,
) -> u64 {
    if cell.c == ' ' || cell.c == '\t' || cell.flags.contains(Flags::HIDDEN) {
        return 0;
    }
    let mut calls = 1;
    painter.text(
        Pos2::new(x, y),
        Align2::LEFT_TOP,
        cell.c.to_string(),
        font_id.clone(),
        color,
    );
    for mark in cell.zerowidth().unwrap_or_default() {
        calls += 1;
        painter.text(
            Pos2::new(x, y),
            Align2::LEFT_TOP,
            mark.to_string(),
            font_id.clone(),
            color,
        );
    }
    calls
}

/// Returns the visual width for a terminal cell.
fn cell_width(cell: &Cell, base_width: f32) -> f32 {
    if cell.flags.contains(Flags::WIDE_CHAR) {
        base_width * 2.0
    } else {
        base_width
    }
}

/// Paints the cursor shape requested by alacritty.
fn paint_terminal_cursor(
    painter: &egui::Painter,
    rect: Rect,
    shape: CursorShape,
    color: egui::Color32,
) {
    match shape {
        CursorShape::Block => {
            painter.rect_filled(rect, 0.0, color.linear_multiply(0.55));
        }
        CursorShape::HollowBlock => {
            painter.rect_stroke(rect, 0.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
        }
        CursorShape::Underline => {
            painter.line_segment(
                [rect.left_bottom(), rect.right_bottom()],
                Stroke::new(2.0, color),
            );
        }
        CursorShape::Beam => {
            painter.line_segment(
                [rect.left_top(), rect.left_bottom()],
                Stroke::new(2.0, color),
            );
        }
        CursorShape::Hidden => {}
    }
}

/// Resolves foreground and background for a terminal cell.
fn terminal_cell_colors(
    cell: &Cell,
    selected: bool,
    colors: &alacritty_terminal::term::color::Colors,
    mode: gui_theme::ThemeMode,
) -> (egui::Color32, egui::Color32) {
    let mut fg = terminal_ansi_color(cell.fg, cell.flags, colors, mode);
    let mut bg = terminal_ansi_color(cell.bg, Flags::empty(), colors, mode);
    if cell.flags.intersects(Flags::DIM | Flags::DIM_BOLD) {
        fg = fg.linear_multiply(0.7);
    }
    if selected || cell.flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }
    (fg, bg)
}

/// Paints underline, strikeout, and other simple terminal text decorations.
fn paint_terminal_decoration(
    painter: &egui::Painter,
    cell: &Cell,
    rect: Rect,
    color: egui::Color32,
) {
    if cell.flags.intersects(Flags::ALL_UNDERLINES) {
        let y = rect.bottom() - 2.0;
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0, color),
        );
    }
    if cell.flags.contains(Flags::STRIKEOUT) {
        let y = rect.center().y;
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0, color),
        );
    }
}

/// Resolves alacritty ANSI colors to the current gsdv palette.
fn terminal_ansi_color(
    color: Color,
    _flags: Flags,
    _colors: &alacritty_terminal::term::color::Colors,
    mode: gui_theme::ThemeMode,
) -> egui::Color32 {
    match color {
        Color::Spec(rgb) => rgb_color(rgb),
        // Codex 会用 Cyan 样式渲染附件占位符。
        // 触发条件：直接 renderer 读取到的 alacritty 动态色表可能偏灰。
        // 不能走普通动态色表路径：产品主题需要稳定的附件强调色。
        // 防止 [Image #1] 这类 TUI span 在 renderer 改造后变得发白。
        Color::Indexed(index) => indexed_terminal_color(index, mode),
        Color::Named(named) => named_terminal_color(named, mode),
    }
}

/// Converts vte RGB colors to egui colors.
fn rgb_color(rgb: Rgb) -> egui::Color32 {
    egui::Color32::from_rgb(rgb.r, rgb.g, rgb.b)
}

/// Resolves named terminal colors from the gsdv theme.
fn named_terminal_color(named: NamedColor, mode: gui_theme::ThemeMode) -> egui::Color32 {
    match named {
        NamedColor::Foreground | NamedColor::BrightForeground => gui_theme::terminal_text_for(mode),
        NamedColor::Background => gui_theme::bg_for(mode),
        NamedColor::Cursor => gui_theme::terminal_text_for(mode),
        NamedColor::Black => gui_theme::bg_for(mode),
        NamedColor::Red | NamedColor::BrightRed | NamedColor::DimRed => gui_theme::danger_for(mode),
        NamedColor::Green | NamedColor::BrightGreen | NamedColor::DimGreen => {
            gui_theme::success_for(mode)
        }
        NamedColor::Yellow | NamedColor::BrightYellow | NamedColor::DimYellow => {
            gui_theme::warning_for(mode)
        }
        NamedColor::Blue | NamedColor::BrightBlue | NamedColor::DimBlue => {
            gui_theme::primary_for(mode)
        }
        NamedColor::White | NamedColor::DimWhite => gui_theme::terminal_white_for(mode),
        NamedColor::BrightWhite => gui_theme::terminal_bright_white_for(mode),
        NamedColor::BrightBlack | NamedColor::DimBlack | NamedColor::DimForeground => {
            gui_theme::muted_for(mode)
        }
        NamedColor::Magenta => terminal_magenta_for(mode),
        NamedColor::BrightMagenta => terminal_bright_magenta_for(mode),
        NamedColor::DimMagenta => terminal_dim_magenta_for(mode),
        NamedColor::Cyan => terminal_cyan_for(mode),
        NamedColor::BrightCyan => terminal_bright_cyan_for(mode),
        NamedColor::DimCyan => terminal_dim_cyan_for(mode),
    }
}

/// Resolves 256-color palette indexes for terminal applications.
fn indexed_terminal_color(index: u8, mode: gui_theme::ThemeMode) -> egui::Color32 {
    if let Some(color) = terminal_color(index as usize, mode) {
        return color;
    }
    match index {
        16..=231 => {
            let value = index - 16;
            let r = xterm_color_component(value / 36);
            let g = xterm_color_component((value / 6) % 6);
            let b = xterm_color_component(value % 6);
            egui::Color32::from_rgb(r, g, b)
        }
        232..=255 => {
            let value = 8 + (index - 232) * 10;
            egui::Color32::from_gray(value)
        }
        _ => gui_theme::terminal_text_for(mode),
    }
}

/// Converts one xterm 6x6x6 color cube component.
fn xterm_color_component(value: u8) -> u8 {
    if value == 0 { 0 } else { 55 + value * 40 }
}

/// 返回当前主题的 terminal magenta。
fn terminal_magenta_for(mode: gui_theme::ThemeMode) -> egui::Color32 {
    match mode {
        gui_theme::ThemeMode::Light | gui_theme::ThemeMode::Dark => {
            egui::Color32::from_rgb(0xAA, 0x75, 0x9F)
        }
    }
}

/// 返回兼容既有主题的 bright magenta。
fn terminal_bright_magenta_for(mode: gui_theme::ThemeMode) -> egui::Color32 {
    match mode {
        gui_theme::ThemeMode::Light | gui_theme::ThemeMode::Dark => {
            egui::Color32::from_rgb(0xC2, 0x8C, 0xB8)
        }
    }
}

/// 返回兼容既有主题的 dim magenta。
fn terminal_dim_magenta_for(mode: gui_theme::ThemeMode) -> egui::Color32 {
    match mode {
        gui_theme::ThemeMode::Light | gui_theme::ThemeMode::Dark => {
            egui::Color32::from_rgb(0x70, 0x4D, 0x68)
        }
    }
}

/// 返回当前主题的 terminal cyan。
fn terminal_cyan_for(mode: gui_theme::ThemeMode) -> egui::Color32 {
    match mode {
        gui_theme::ThemeMode::Light | gui_theme::ThemeMode::Dark => {
            egui::Color32::from_rgb(0x75, 0xB5, 0xAA)
        }
    }
}

/// 返回兼容既有主题的 bright cyan。
fn terminal_bright_cyan_for(mode: gui_theme::ThemeMode) -> egui::Color32 {
    match mode {
        gui_theme::ThemeMode::Light | gui_theme::ThemeMode::Dark => {
            egui::Color32::from_rgb(0x93, 0xD3, 0xC3)
        }
    }
}

/// 返回兼容既有主题的 dim cyan。
fn terminal_dim_cyan_for(mode: gui_theme::ThemeMode) -> egui::Color32 {
    match mode {
        gui_theme::ThemeMode::Light | gui_theme::ThemeMode::Dark => {
            egui::Color32::from_rgb(0x4D, 0x77, 0x70)
        }
    }
}

impl TerminalHost for GuiTerminalHost {
    fn kind(&self) -> TerminalSurfaceKind {
        self.kind
    }

    fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    fn summary(&self) -> TerminalHostSummary {
        let terminal_title = self
            .runtime_state
            .lock()
            .ok()
            .and_then(|state| state.terminal_title.clone());
        match self.kind {
            TerminalSurfaceKind::Agent => TerminalHostSummary {
                title: terminal_title
                    .as_ref()
                    .filter(|title| !title.trim().is_empty())
                    .map(|title| format!("Agent: {} · {title}", self.agent_title))
                    .unwrap_or_else(|| format!("Agent: {}", self.agent_title)),
                description: format!(
                    "Terminal-hosted {} session for {}.",
                    self.agent_title, self.workspace_name
                ),
                footer: match (&self.session_id, self.activity) {
                    (Some(session), WorkspaceActivity::Busy) => format!("session {session} · busy"),
                    (Some(session), WorkspaceActivity::Idle) => format!("session {session} · idle"),
                    (Some(session), WorkspaceActivity::Unknown) => {
                        format!("session {session} · status unknown")
                    }
                    (None, _) => "no known session yet".to_string(),
                },
            },
            TerminalSurfaceKind::Workspace => TerminalHostSummary {
                title: "Workspace Terminal".to_string(),
                description: format!("Persistent shell for {}.", self.workspace_name),
                footer: format!("cwd {}", self.workspace_root.display()),
            },
            TerminalSurfaceKind::Helix => TerminalHostSummary {
                title: "Helix".to_string(),
                description: format!("Helix editor for {}.", self.workspace_name),
                footer: format!("cwd {}", self.workspace_root.display()),
            },
        }
    }
}

fn terminal_command(workspace: &WorkspaceViewData, kind: TerminalSurfaceKind) -> String {
    match kind {
        TerminalSurfaceKind::Agent => workspace.agent_kind.command().to_string(),
        TerminalSurfaceKind::Workspace => workspace_terminal_command(),
        TerminalSurfaceKind::Helix => "hx".to_string(),
    }
}

/// 返回 workspace terminal 的启动命令，适用于跨平台 shell 兜底。
fn workspace_terminal_command() -> String {
    workspace_terminal_command_from_env(std::env::var_os("SHELL"), cfg!(target_os = "windows"))
}

/// 从环境变量解析 workspace terminal 命令，便于覆盖 Windows 缺 SHELL 的场景。
fn workspace_terminal_command_from_env(shell: Option<OsString>, is_windows: bool) -> String {
    if let Some(shell) = non_empty_os_string(shell) {
        return shell.to_string_lossy().to_string();
    }
    if is_windows {
        return "powershell".to_string();
    }
    "/bin/bash".to_string()
}

/// 过滤空环境变量，避免空 SHELL 被当成可执行程序。
fn non_empty_os_string(value: Option<OsString>) -> Option<OsString> {
    value.filter(|value| !value.as_os_str().is_empty())
}

fn terminal_args(
    workspace: &WorkspaceViewData,
    kind: TerminalSurfaceKind,
    _id: u64,
    agent_launch: &AgentLaunchConfig,
    agent_session_id: Option<&str>,
) -> Vec<String> {
    match kind {
        TerminalSurfaceKind::Agent => {
            let mut args = Vec::new();
            let resume_cwd = terminal_working_directory(workspace, kind);
            args.extend(agent_launch.args_for(
                workspace.agent_kind,
                agent_session_id,
                workspace.agent_model.as_deref(),
                workspace.agent_model_provider.as_deref(),
                workspace.agent_effort.as_deref(),
                workspace.agent_fast_mode,
                Some(resume_cwd.as_path()),
            ));
            args
        }
        TerminalSurfaceKind::Workspace => Vec::new(),
        TerminalSurfaceKind::Helix => Vec::new(),
    }
}

fn terminal_working_directory(workspace: &WorkspaceViewData, kind: TerminalSurfaceKind) -> PathBuf {
    if kind == TerminalSurfaceKind::Agent
        && let Some(work_dir) = workspace
            .agent_work_dir
            .as_ref()
            .filter(|path| path.is_dir())
    {
        return work_dir.clone();
    }
    workspace.path.clone()
}

fn terminal_env(
    workspace: &WorkspaceViewData,
    kind: TerminalSurfaceKind,
    network_settings: &NetworkSettings,
) -> HashMap<String, String> {
    let mut env = network_settings
        .env_vars()
        .into_iter()
        .collect::<HashMap<_, _>>();
    if let Some(gws) = data::ensure_workspace_store_dir(&workspace.path) {
        env.insert("GWS".to_string(), gws.display().to_string());
    }
    env.insert(
        "GSDV_WORKSPACE_DIR".to_string(),
        workspace.path.display().to_string(),
    );
    if kind == TerminalSurfaceKind::Agent {
        env.insert("GSDV_AGENT_ID".to_string(), workspace.agent_id.clone());
        env.insert(
            "GSDV_AGENT_KIND".to_string(),
            workspace.agent_kind.env_name().to_string(),
        );
    }
    env
}

fn spawn_backend_with_env(
    id: u64,
    egui_ctx: egui::Context,
    event_tx: UnboundedSender<(u64, PtyEvent)>,
    settings: BackendSettings,
    env: &HashMap<String, String>,
) -> Result<TerminalBackend> {
    if env.is_empty() {
        return Ok(TerminalBackend::new(id, egui_ctx, event_tx, settings)?);
    }
    let _guard = TERMINAL_SPAWN_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let previous = env
        .keys()
        .map(|key| (key.clone(), std::env::var_os(key)))
        .collect::<Vec<(String, Option<OsString>)>>();
    for (key, value) in env {
        unsafe {
            std::env::set_var(key, value);
        }
    }
    let result = TerminalBackend::new(id, egui_ctx, event_tx, settings);
    for (key, value) in previous {
        unsafe {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
    Ok(result?)
}

fn helix_args(spec: &HelixLaunchSpec) -> Vec<String> {
    let mut args = vec![
        "--working-dir".to_string(),
        spec.workdir.display().to_string(),
    ];
    let target = match (&spec.file, spec.line) {
        (Some(file), Some(line)) => format!("{}:{line}:1", file.display()),
        (Some(file), None) => file.display().to_string(),
        (None, _) => spec.workdir.display().to_string(),
    };
    args.push(target);
    args
}

fn command_display(shell: &str, args: &[String]) -> String {
    std::iter::once(shell)
        .chain(args.iter().map(String::as_str))
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    if value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(byte, b'_' | b'-' | b'.' | b'/' | b':' | b'=' | b'@')
    }) {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn exit_status_label(code: i32) -> String {
    format!("exit code {code}")
}

fn terminal_font(size: f32, kind: TerminalSurfaceKind) -> FontId {
    FontId::new(clamped_terminal_font_size(size), font_family(kind))
}

fn clamped_terminal_font_size(size: f32) -> f32 {
    size.clamp(9.0, 28.0)
}

fn font_family(kind: TerminalSurfaceKind) -> FontFamily {
    match kind {
        TerminalSurfaceKind::Agent => gui_theme::agent_system_font_family(),
        TerminalSurfaceKind::Workspace | TerminalSurfaceKind::Helix => {
            gui_theme::terminal_system_font_family()
        }
    }
}

fn terminal_query_rgb(
    index: usize,
    colors: &terminal_color_table::Colors,
    mode: gui_theme::ThemeMode,
) -> Option<Rgb> {
    if index < terminal_color_table::COUNT
        && let Some(rgb) = colors[index]
    {
        return Some(rgb);
    }
    let color = if index <= 255 {
        indexed_terminal_color(index as u8, mode)
    } else {
        terminal_color(index, mode)?
    };
    Some(Rgb {
        r: color.r(),
        g: color.g(),
        b: color.b(),
    })
}

fn terminal_color(index: usize, mode: gui_theme::ThemeMode) -> Option<egui::Color32> {
    match index {
        TERMINAL_FOREGROUND_COLOR_INDEX => Some(gui_theme::terminal_text_for(mode)),
        TERMINAL_BACKGROUND_COLOR_INDEX => Some(gui_theme::bg_for(mode)),
        TERMINAL_CURSOR_COLOR_INDEX => Some(gui_theme::terminal_text_for(mode)),
        0 => Some(gui_theme::bg_for(mode)),
        1 | 9 => Some(gui_theme::danger_for(mode)),
        2 | 10 => Some(gui_theme::success_for(mode)),
        3 | 11 => Some(gui_theme::warning_for(mode)),
        4 | 12 => Some(gui_theme::primary_for(mode)),
        5 | 13 => Some(terminal_magenta_for(mode)),
        6 | 14 => Some(terminal_cyan_for(mode)),
        7 => Some(gui_theme::terminal_white_for(mode)),
        8 => Some(gui_theme::muted_for(mode)),
        15 => Some(gui_theme::terminal_bright_white_for(mode)),
        _ => None,
    }
}

#[cfg(test)]
#[path = "terminal_host_test.rs"]
mod terminal_host_test;

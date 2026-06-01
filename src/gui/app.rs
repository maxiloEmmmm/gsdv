use crate::BranchInfo;
use crate::gui::agent::{AgentKind, AgentLaunchConfig};
use crate::gui::data::{
    self, AppLanguage, CenterMode, FontSettings, InitialGuiData, NetworkSettings, OutlineNode,
    ReviewerMode, Route, RuntimeSettings, WorkspaceActivity, WorkspaceViewData,
};
use crate::gui::i18n;
use crate::gui::markdown_preview;
#[cfg(test)]
use crate::gui::outline::tree_row_content_width_from_label_width;
use crate::gui::outline::{
    OutlineAction, TreeRowMarker, collapse_outline_to_first_level, compact_tree_row,
    outline_path_is_global, recent_markdown_outline_dialog_content, recent_markdown_outline_nodes,
    render_favorite_outline_node, render_outline_node, toggle_path_in_set,
};
use crate::gui::reviewer_adapter::{ReviewerAdapter, ReviewerBranchTarget};
use crate::gui::terminal_host::{
    AgentProcessExit, GuiTerminalHost, HelixLaunchSpec, TerminalFileLineClick, TerminalHost,
    TerminalInputShortcutScope, TerminalOutputClick, TerminalRuntimeEvent,
    TerminalRuntimeEventKind, TerminalRuntimeEventSink, TerminalSurfaceKind,
    agent_input_bytes_from_events_with_kitty_protocol, classify_terminal_output_path_click,
    terminal_agent_input_submit_bytes,
};
use crate::gui::theme;
use crate::gui::workflow::{
    WorkflowMutationRequest, WorkflowProjectNode, WorkflowSaveRequest, WorkflowSaveSuccess,
    WorkflowSelectionTarget, WorkflowStepEditor, WorkflowStepNode, WorkflowTaskEditor,
    WorkflowTaskNode, WorkflowTree, path_is_workflow_spec_path, workflow_root_missing_error,
    workflow_step_editor_from_node, workflow_task_editor_from_node,
};
use crate::reviewer::app::{GuiReviewerRowTone, ReviewerGitDataResult};
use eframe::egui::text_edit::{TextEditOutput, TextEditState};
use eframe::egui::{
    self, Align, Align2, Button, CentralPanel, Color32, CornerRadius, Frame, Layout, Margin, Rect,
    RichText, ScrollArea, Sense, SidePanel, Stroke, TopBottomPanel, Ui, Vec2,
};
use egui_extras::{Size, StripBuilder};
use notify::{RecursiveMode, Watcher};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender},
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs};

#[path = "app_dialogs.rs"]
mod app_dialogs;

#[path = "app_document.rs"]
mod app_document;

#[path = "app_events.rs"]
mod app_events;

#[path = "app_commands.rs"]
mod app_commands;

#[path = "app_extra_tools.rs"]
mod app_extra_tools;
use app_extra_tools::*;

#[path = "app_agent.rs"]
mod app_agent;

#[path = "app_chrome.rs"]
mod app_chrome;
use app_chrome::*;

#[path = "app_fonts.rs"]
mod app_fonts;
use app_fonts::*;

#[path = "app_fs.rs"]
mod app_fs;

#[path = "app_helpers.rs"]
mod app_helpers;
use app_helpers::*;

#[path = "app_input.rs"]
mod app_input;
use app_input::{process_input_runtime_request, read_base_route_command_for_input};
#[path = "app_markdown.rs"]
mod app_markdown;
use app_markdown::*;

#[path = "app_notifications.rs"]
mod app_notifications;

#[path = "app_pomodoro.rs"]
mod app_pomodoro;

#[path = "app_reviewer_ui.rs"]
mod app_reviewer_ui;
use app_reviewer_ui::{ReviewerDiffCopyKind, apply_reviewer_diff_selection_override};

#[path = "app_reviewer_state.rs"]
mod app_reviewer_state;

#[path = "app_shell_ui.rs"]
mod app_shell_ui;

#[path = "app_screenshot.rs"]
mod app_screenshot;

#[path = "app_terminal_ui.rs"]
mod app_terminal_ui;

#[path = "app_workspace.rs"]
mod app_workspace;

#[path = "app_store.rs"]
mod app_store;

#[path = "app_tasks.rs"]
mod app_tasks;

#[path = "app_workflow.rs"]
mod app_workflow;

#[cfg(test)]
use app_input::{agent_tab_own_shortcut_pressed, read_reviewer_command, read_ui_command};

const SCREENSHOT_REQUEST_FILE: &str = "capture.request";
const WORKSPACE_RAIL_WIDTH: f32 = 184.0;
const COMPACT_WORKSPACE_RAIL_WIDTH: f32 = 58.0;
const RAIL_EDGE_INSET: f32 = 2.0;
const BOTTOM_BAR_HEIGHT: f32 = 22.0;
const AGENT_THEME_RESTART_DELAY: Duration = Duration::from_millis(900);
const SCREENSHOT_REQUEST_POLL_INTERVAL: Duration = Duration::from_millis(250);
const FS_WATCH_DEBOUNCE: Duration = Duration::from_millis(250);
const EXTRA_TOOLS_SCAN_INTERVAL: Duration = Duration::from_secs(5);
const WORKSPACE_STORE_SAVE_DEBOUNCE: Duration = Duration::from_millis(120);
const AGENT_INPUT_TRANSLATION_IDLE_DEBOUNCE: Duration = Duration::from_millis(500);
const AGENT_BUSY_AUTO_GO_INPUT: &[u8] = b"go\r";
const AGENT_QUICK_REPLIES: [&str; 9] = [
    "ok",
    "next",
    "go",
    "hi?",
    "showMeCode",
    "showMeFiles-Lines",
    "WDYT?",
    "markItDone",
    "markItDoneAndNext",
];
const MARKDOWN_OUTLINE_WIDTH_FRACTION: f32 = 0.20;
const MARKDOWN_OUTLINE_SCROLL_TOP_PADDING: f32 = 48.0;
const NOTIFICATION_MAX_LINES: usize = 2000;
const RECENT_MARKDOWN_LIMIT: usize = 200;
const MARKDOWN_DIFF_HISTORY_LIMIT: usize = 2000;
const MARKDOWN_NAME_SUGGESTIONS: &[&str] = &["AGENTS.md", "README.md", "PLAN.md", "TODO.md"];
const APP_NAME: &str = "gsdv";
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const APP_COPYRIGHT: &str = "Copyright 2026 gsdv contributors.";
const APP_DESCRIPTION: &str = "Single-process egui workspace for Markdown projects, reviewer data, embedded agents, and terminal surfaces.";
const SCREENSHOT_REQUEST_POLL_ENV: &str = "GSDV_SCREENSHOT_REQUEST_POLL";
const POMODORO_CAT_SIZE: Vec2 = Vec2::new(220.0, 220.0);
const POMODORO_MEOW_INTERVAL: Duration = Duration::from_millis(1450);
const POMODORO_ANIMATION_FRAME: Duration = Duration::from_millis(33);
const POMODORO_RETURN_TO_WORK_DURATION: Duration = Duration::from_millis(3_000);
const POMODORO_RETURN_QUESTION_RAMP: Duration = Duration::from_millis(1_200);
const POMODORO_RETURN_QUESTION_COUNT: usize = 5;
const POMODORO_REST_QUIET_DURATION: Duration = Duration::from_secs(10);
const POMODORO_REST_QUIET_QUESTION_COUNT: usize = 5;
const POMODORO_PEEK_ORBIT_TEXT: &str = "差不多到时间咯";
const WORKSPACE_WATCH_IGNORED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".jj",
    ".gsdv",
    ".idea",
    ".vscode",
    ".direnv",
    ".cache",
    ".pnpm-store",
    ".yarn",
    ".npm",
    ".next",
    ".nx",
    ".nuxt",
    ".svelte-kit",
    ".parcel-cache",
    ".turbo",
    ".terraform",
    ".terraform.d",
    ".serverless",
    ".aws-sam",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    ".venv",
    "venv",
    "vendor",
    "dist",
    "build",
    "coverage",
    "target",
    "node_modules",
];

pub fn run() -> eframe::Result<()> {
    configure_platform_about_metadata();
    let mut agent_launch = AgentLaunchConfig::from_env_args();
    if !agent_launch.kind_explicit {
        if let Some(agent_kind) = data::load_default_agent_kind() {
            agent_launch.kind = agent_kind;
        } else {
            let agent_kind = prompt_default_agent_kind();
            data::save_default_agent_kind(agent_kind);
            agent_launch.kind = agent_kind;
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1560.0, 980.0])
            .with_min_inner_size([1280.0, 800.0])
            .with_title("gsdv")
            .with_icon(app_icon_data()),
        ..Default::default()
    };

    eframe::run_native(
        "gsdv",
        options,
        Box::new(|cc| {
            let system_fonts = scan_system_fonts();
            let mut font_settings = if data::has_saved_font_settings() {
                data::load_font_settings()
            } else {
                let font_settings = initial_font_settings(&system_fonts);
                data::save_font_settings(&font_settings);
                font_settings
            };
            if normalize_font_settings(&mut font_settings, &system_fonts) {
                data::save_font_settings(&font_settings);
            }
            theme::configure(&cc.egui_ctx);
            if let Some(theme_mode) = data::load_theme_mode() {
                theme::set_mode(&cc.egui_ctx, theme_mode);
            }
            let data = data::load_initial_gui_data(agent_launch.kind);
            apply_runtime_fonts(&cc.egui_ctx, &font_settings);
            let mut app =
                GsdvGuiApp::new_with_font_settings(data, agent_launch, font_settings, system_fonts);
            app.set_fs_watch_repaint_context(cc.egui_ctx.clone());
            Ok(Box::new(app))
        }),
    )
}

#[cfg(target_os = "macos")]
fn configure_platform_about_metadata() {
    macos_about::configure();
}

#[cfg(not(target_os = "macos"))]
fn configure_platform_about_metadata() {}

fn app_icon_data() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../../assets/gsdv-icon.png"))
        .expect("embedded gsdv app icon must be a valid PNG")
}

/// Builds the app-owned async runtime for non-render work.
fn build_background_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("gsdv-bg")
        .build()
        .expect("background runtime must start")
}

/// 启动 input runtime，独立消费 egui 原生输入快照。
fn spawn_input_runtime(
    runtime_handle: tokio::runtime::Handle,
    app_event_tx: Sender<AppEvent>,
) -> tokio::sync::mpsc::UnboundedSender<InputRuntimeRequest> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<InputRuntimeRequest>();
    runtime_handle.spawn(async move {
        while let Some(request) = rx.recv().await {
            let repaint_ctx = request.repaint_ctx.clone();
            let repaint_after = request.repaint_after;
            let events = process_input_runtime_request(request);
            if events.is_empty() {
                continue;
            }
            for event in events {
                let _ = app_event_tx.send(event);
            }
            repaint_ctx.request_repaint_after(repaint_after);
        }
    });
    tx
}

#[cfg(target_os = "macos")]
mod macos_about {
    use std::ffi::{CString, c_char, c_void};
    use std::ptr;

    use super::{APP_COPYRIGHT, APP_NAME, APP_VERSION};

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFBundleGetMainBundle() -> *const c_void;
        fn CFBundleGetInfoDictionary(bundle: *const c_void) -> *mut c_void;
        fn CFDictionarySetValue(dict: *mut c_void, key: *const c_void, value: *const c_void);
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            c_str: *const c_char,
            encoding: u32,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
    }

    pub fn configure() {
        // AppKit's standard About panel reads these keys from the main bundle.
        // When running outside a .app bundle, winit still creates the macOS menu,
        // so we patch the process bundle dictionary early enough for that panel.
        unsafe {
            let bundle = CFBundleGetMainBundle();
            if bundle.is_null() {
                return;
            }
            let info = CFBundleGetInfoDictionary(bundle);
            if info.is_null() {
                return;
            }

            set_bundle_string(info, "CFBundleName", APP_NAME);
            set_bundle_string(info, "CFBundleDisplayName", APP_NAME);
            set_bundle_string(info, "CFBundleShortVersionString", APP_VERSION);
            set_bundle_string(info, "CFBundleVersion", APP_VERSION);
            set_bundle_string(info, "NSHumanReadableCopyright", APP_COPYRIGHT);
        }
    }

    unsafe fn set_bundle_string(info: *mut c_void, key: &str, value: &str) {
        let Ok(key) = CString::new(key) else {
            return;
        };
        let Ok(value) = CString::new(value) else {
            return;
        };
        let key = unsafe {
            CFStringCreateWithCString(ptr::null(), key.as_ptr(), K_CF_STRING_ENCODING_UTF8)
        };
        let value = unsafe {
            CFStringCreateWithCString(ptr::null(), value.as_ptr(), K_CF_STRING_ENCODING_UTF8)
        };
        if !key.is_null() && !value.is_null() {
            unsafe { CFDictionarySetValue(info, key, value) };
        }
        if !key.is_null() {
            unsafe { CFRelease(key) };
        }
        if !value.is_null() {
            unsafe { CFRelease(value) };
        }
    }
}

fn prompt_default_agent_kind() -> AgentKind {
    loop {
        println!("Select the default agent type for gsdv:");
        for (index, agent_kind) in AgentKind::all().iter().enumerate() {
            println!("  {}. {}", index + 1, agent_kind.title());
        }
        print!("Enter a number: ");
        let _ = io::stdout().flush();

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            eprintln!("Could not read selection. Try again.");
            continue;
        }
        let Ok(choice) = input.trim().parse::<usize>() else {
            eprintln!("Invalid selection. Enter the number from the list.");
            continue;
        };
        if let Some(agent_kind) = AgentKind::all().get(choice.saturating_sub(1)) {
            return *agent_kind;
        }
        eprintln!("Invalid selection. Enter the number from the list.");
    }
}

struct GsdvGuiApp {
    active_workspace: usize,
    workspaces: Vec<WorkspaceViewData>,
    reviewer_adapters: Vec<Option<ReviewerAdapter>>,
    /// Reviewer adapter 暂时离开 UI 线程时保留的最后可绘制快照。
    reviewer_snapshots: Vec<Option<crate::reviewer::app::GuiReviewerSnapshot>>,
    reviewer_dialogs: Vec<Option<ReviewerDialog>>,
    reviewer_diff_scroll_targets: Vec<Option<usize>>,
    /// Reviewer diff 行的本地视觉选中覆盖。
    reviewer_diff_selected_rows: Vec<Option<usize>>,
    terminal_hosts: Vec<WorkspaceTerminalHosts>,
    /// Active agent terminal slot for each workspace.
    active_agent_slots: Vec<AgentSlotId>,
    /// 每个 workspace 的 Agent Busy 无输出守护状态。
    agent_busy_watchdogs: Vec<BTreeMap<AgentSlotId, AgentBusyWatchdogState>>,
    workspace_terminal_drawers: Vec<bool>,
    reviewer_helix_drawers: Vec<bool>,
    pending_agent_theme_restarts: Vec<Option<Instant>>,
    documents: Vec<DocumentState>,
    app_dialogs: Vec<Option<AppDialog>>,
    global_app_dialog: Option<AppDialog>,
    /// 每个 workspace 最近一次绘制出的 outline tree 区域。
    outline_tree_rects: Vec<Option<Rect>>,
    /// 每个 workspace 的 outline 是否只显示收藏项。
    outline_favorites_only: Vec<bool>,
    /// 每个 workspace 的左侧面板当前 tab。
    outline_panel_tabs: Vec<OutlinePanelTab>,
    /// 每个 workspace 的 workflow tree 和片段编辑状态。
    workflow_states: Vec<WorkflowUiState>,
    /// 每个 workspace 最近一次 memo 保存错误，用于底栏提示。
    memo_save_errors: Vec<Option<String>>,
    /// 上次把 Markdown diff 上下文粘贴给 Agent 的 Unix 毫秒时间。
    markdown_diff_paste_since_ms: u128,
    /// Last successful translation eligible for Cmd/Alt+N replacement.
    last_agent_input_translation: Option<AgentInputTranslation>,
    /// Non-modal translation popup shown above the Agent input.
    agent_input_translation_popup: Option<AgentInputTranslationPopup>,
    /// Last painted active Agent terminal rectangle.
    active_agent_terminal_rect: Option<Rect>,
    /// Active auto-translation debounce state for the Agent input.
    agent_input_translation_watch: Option<AgentInputTranslationWatch>,
    /// Agent translation requests currently running in the background.
    agent_input_translation_in_flight: BTreeSet<(usize, AgentSlotId)>,
    /// Cached Codex client for quick Agent input translation.
    agent_input_translation_client: Option<AgentInputTranslationClientCache>,
    toasts: Vec<Toast>,
    screenshot_sequence: u64,
    last_screenshot_path: Option<PathBuf>,
    theme_mode: theme::ThemeMode,
    rail_collapsed: bool,
    agent_launch: AgentLaunchConfig,
    fs_watcher: Arc<Mutex<FsWatcherService>>,
    fs_watch_dirty: FsWatchDirtyState,
    /// 上一次 UI pass 中 memo 文本变化的 workspace。
    pending_memo_saves: BTreeSet<usize>,
    /// 需要重建 Markdown 派生缓存的 workspace。
    pending_markdown_reparse: BTreeSet<usize>,
    /// 需要折叠 Markdown 本地 outline 的 workspace。
    pending_markdown_outline_collapse: BTreeSet<usize>,
    /// IME fallback 输入是否请求了下一次 UI pass。
    pending_input_repaint: bool,
    /// runtime settings 需要从事件阶段派发持久化。
    pending_runtime_settings_save: bool,
    /// 语言设置需要从事件阶段持久化。
    pending_language_settings_save: bool,
    /// font settings 需要从事件阶段应用并派发持久化。
    pending_font_settings_save: bool,
    /// network settings 需要从事件阶段派发持久化。
    pending_network_settings_save: bool,
    /// debug 截图请求文件轮询是否启用。
    screenshot_request_poll_enabled: bool,
    /// workspace store 最近一次被业务标记为需要持久化的时间。
    workspace_store_dirty_at: Option<Instant>,
    /// 独立 workspace store writer 是否正在写磁盘。
    workspace_store_save_in_flight: Arc<AtomicBool>,
    /// 最近一次检查可选截图请求文件的时间。
    last_screenshot_request_poll: Instant,
    /// 可选截图请求文件是否正在后台读取。
    screenshot_request_read_in_flight: bool,
    /// 最近一次完整 UI frame 完成绘制的时间。
    last_full_frame_at: Option<Instant>,
    /// 当前 frame 之后是否还需要再绘制一个完整 frame。
    render_dirty: bool,
    /// app 当前是否正在构建完整 UI frame。
    rendering_frame: bool,
    /// 后台任务完成后用于唤醒 app 的最近 egui context。
    app_repaint_ctx: Option<egui::Context>,
    /// 正在 UI 路径之外创建的 terminal host。
    pending_terminal_spawns: BTreeSet<TerminalSpawnKey>,
    /// 当前是否正在检查 `hx --version`。
    helix_binary_check_in_flight: bool,
    /// 等待 Helix binary 可用性结果的打开请求。
    pending_helix_open_request: Option<HelixOpenRequest>,
    /// render 或 input route 请求加载的 reviewer adapter。
    pending_reviewer_loads: BTreeSet<usize>,
    /// 当前正在后台构建的 reviewer adapter。
    reviewer_loads_in_flight: BTreeSet<usize>,
    /// 当前正在后台执行修改任务的 reviewer adapter。
    reviewer_adapter_tasks_in_flight: BTreeSet<usize>,
    /// 当前正在后台加载轻量 git 数据的 reviewer workspace。
    reviewer_git_data_in_flight: BTreeSet<usize>,
    /// git 数据加载中收到的后续请求，保存行数预算和是否追加历史。
    pending_reviewer_git_data_budget: BTreeMap<usize, (usize, bool)>,
    /// 等待当前修改任务完成的 reviewer adapter 后续任务。
    pending_reviewer_adapter_tasks:
        BTreeMap<usize, VecDeque<(ReviewerAdapterTask, ReviewerAdapterTaskEffect)>>,
    suppress_editor_input: bool,
    helix_binary_available: bool,
    reviewer_scripts: ReviewerScriptState,
    /// Agent 主界面的外置脚本工具状态。
    extra_tools: ExtraToolsState,
    notifications: NotificationCenter,
    /// 唯一渲染状态事件发送端，后台结果和外部信号都必须投递到这里。
    app_event_tx: Sender<AppEvent>,
    /// 执行不能跑在 egui update 中的后台工作。
    background_runtime: Arc<tokio::runtime::Runtime>,
    /// 唯一渲染状态事件接收端，只在 update 预算内消费。
    app_event_rx: Receiver<AppEvent>,
    /// 原生 egui 输入快照发送端，交给 input runtime 独立解析。
    input_runtime_tx: tokio::sync::mpsc::UnboundedSender<InputRuntimeRequest>,
    notification_return_context: Option<NotificationReturnContext>,
    app_language: AppLanguage,
    global_outline_favorites: BTreeSet<PathBuf>,
    network_settings: NetworkSettings,
    runtime_settings: RuntimeSettings,
    network_settings_dialog_baseline: Option<NetworkSettings>,
    /// settings 和 auth dialog 中显示的 Codex OAuth 状态。
    codex_auth: CodexAuthUiState,
    font_settings: FontSettings,
    system_fonts: Vec<SystemFontEntry>,
    /// Prepared filter for the default primary font picker.
    default_font_filter: FontPickerFilter,
    /// Prepared filter for the default fallback font picker.
    default_fallback_font_filter: FontPickerFilter,
    /// Prepared filter for the agent primary font picker.
    agent_font_filter: FontPickerFilter,
    /// Prepared filter for the agent fallback font picker.
    agent_fallback_font_filter: FontPickerFilter,
    /// Prepared filter for the terminal primary font picker.
    terminal_font_filter: FontPickerFilter,
    /// Prepared filter for the terminal fallback font picker.
    terminal_fallback_font_filter: FontPickerFilter,
    /// Prepared filter for the editor primary font picker.
    editor_font_filter: FontPickerFilter,
    /// Prepared filter for the editor fallback font picker.
    editor_fallback_font_filter: FontPickerFilter,
    /// Global in-memory pomodoro state for the current app session.
    pomodoro: PomodoroState,
    /// 哈基米右键菜单是否打开。
    pomodoro_cat_menu_open: bool,
    /// Cached texture for the user-provided Hajimi pixel sprite.
    hajimi_texture: Option<egui::TextureHandle>,
    suppress_default_agent_input: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SystemFontEntry {
    /// 设置里展示的字体名称。
    name: String,
    /// 字体文件绝对路径。
    path: PathBuf,
    /// 字体文件大小，用来优先选择更轻量的自动 CJK 主字体。
    size_bytes: u64,
    /// 给筛选输入使用的小写搜索键。
    search_key: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct FontPickerFilter {
    /// User-entered filter text.
    text: String,
    /// Trimmed lowercase filter updated only when text changes.
    normalized: String,
}

#[derive(Debug, Clone)]
struct CodexAuthUiState {
    /// 已保存凭证里的展示信息。
    info: Option<crate::ai::CodexAuthInfo>,
    /// OAuth 后台流程是否还在等待回调或换 token。
    in_flight: bool,
    /// 当前授权 URL，供弹窗展示。
    auth_url: Option<String>,
    /// 最近一次授权错误。
    error: Option<String>,
    /// 最近一次后台流程开始时间。
    started_at: Option<Instant>,
}

impl CodexAuthUiState {
    /// 创建 settings 使用的 Codex OAuth 展示状态。
    fn new() -> Self {
        Self {
            info: crate::ai::load_cached_auth_info(),
            in_flight: false,
            auth_url: None,
            error: None,
            started_at: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// 当前窗口会话里的番茄钟生命周期。
enum PomodoroPhase {
    /// 工作计时正在进行。
    Working,
    /// 休息前仍有输入，等待用户连续安静。
    WaitingForRestQuiet,
    /// 休息计时正在进行，哈基米可以移动。
    Resting,
    /// 休息完成，哈基米等待任意输入后退场。
    ReadyToWork,
    /// 已看到用户输入，哈基米播放短退场动画。
    ReturningToWork,
}

#[derive(Debug, Clone)]
/// 休息时哈基米飘出的提示文本。
struct PomodoroMeow {
    /// 提示文本在 app 窗口里的生成位置。
    origin: egui::Pos2,
    /// 提示文本创建时间。
    created_at: Instant,
}

#[derive(Debug, Clone)]
/// 全局番茄钟的非持久计时和动画状态。
struct PomodoroState {
    /// 当前番茄钟阶段。
    phase: PomodoroPhase,
    /// 当前阶段开始时间。
    phase_started_at: Instant,
    /// 哈基米在窗口里的左上角位置。
    cat_pos: egui::Pos2,
    /// 哈基米弹跳移动时使用的速度。
    cat_velocity: Vec2,
    /// 上次动画时间，用于按真实帧间隔移动。
    last_animation_at: Instant,
    /// 冷静期问号动画的起始时间。
    rest_quiet_animation_started_at: Instant,
    /// 休息模式里飘出的提示文本。
    meows: Vec<PomodoroMeow>,
    /// 下一次飘出提示文本的时间。
    next_meow_at: Instant,
}

impl PomodoroState {
    /// 为新的 app 会话创建非持久番茄钟状态。
    fn new(now: Instant) -> Self {
        Self {
            phase: PomodoroPhase::Working,
            phase_started_at: now,
            cat_pos: egui::pos2(520.0, 240.0),
            cat_velocity: Vec2::new(118.0, 86.0),
            last_animation_at: now,
            rest_quiet_animation_started_at: now,
            meows: Vec::new(),
            next_meow_at: now + POMODORO_MEOW_INTERVAL,
        }
    }

    /// 休息结束或菜单手动触发后进入工作阶段。
    fn start_working(&mut self, now: Instant) {
        self.phase = PomodoroPhase::Working;
        self.phase_started_at = now;
        self.last_animation_at = now;
        self.rest_quiet_animation_started_at = now;
        self.meows.clear();
    }

    /// 休息期间检测到输入后进入等待安静阶段。
    fn wait_for_rest_quiet(&mut self, now: Instant) {
        let previous_phase = self.phase;
        let already_resting = matches!(
            previous_phase,
            PomodoroPhase::WaitingForRestQuiet | PomodoroPhase::Resting
        );
        self.phase = PomodoroPhase::WaitingForRestQuiet;
        self.phase_started_at = now;
        if !already_resting {
            self.last_animation_at = now;
            self.rest_quiet_animation_started_at = now;
        } else if previous_phase == PomodoroPhase::Resting {
            self.rest_quiet_animation_started_at = now;
        }
        self.meows.clear();
    }

    /// 连续安静后正式开始休息倒计时。
    fn start_resting(&mut self, now: Instant) {
        self.phase = PomodoroPhase::Resting;
        self.phase_started_at = now;
        self.last_animation_at = now;
        self.rest_quiet_animation_started_at = now;
        self.next_meow_at = now;
    }

    /// 标记休息已完成并等待用户输入。
    fn wait_for_work_input(&mut self, now: Instant) {
        self.phase = PomodoroPhase::ReadyToWork;
        self.phase_started_at = now;
        self.last_animation_at = now;
        self.rest_quiet_animation_started_at = now;
    }

    /// 开始回到工作前的短退场动画。
    fn start_returning_to_work(&mut self, now: Instant) {
        self.phase = PomodoroPhase::ReturningToWork;
        self.phase_started_at = now;
        self.last_animation_at = now;
        self.rest_quiet_animation_started_at = now;
        self.meows.clear();
    }
}

/// Agent Busy 期间的无输出自动继续状态。
///
/// 适用场景：workspace 状态已经 Busy，但嵌入终端长时间没有新输出。
#[derive(Debug, Clone, Default)]
struct AgentBusyWatchdogState {
    /// 当前 Busy 周期被 UI 观察到的起点。
    busy_started_at: Option<Instant>,
    /// 最近一次终端内容输出的时间。
    last_output_at: Option<Instant>,
    /// 当前 Busy 周期是否已经发送过自动 go。
    auto_go_sent: bool,
}

impl AgentBusyWatchdogState {
    /// 记录 Busy 周期开始，适用于 Idle/Unknown 切到 Busy 的首帧。
    fn start_busy(&mut self, now: Instant) {
        self.busy_started_at = Some(now);
        self.last_output_at = Some(now);
        self.auto_go_sent = false;
    }

    /// 记录终端输出，适用于 PTY Wakeup 事件被 UI 线程清理时。
    fn record_output(&mut self, now: Instant) {
        self.last_output_at = Some(now);
    }

    /// 重置当前周期，适用于 Agent 回到非 Busy 状态后重新计时。
    fn reset_idle(&mut self) {
        self.busy_started_at = None;
        self.last_output_at = None;
        self.auto_go_sent = false;
    }

    /// 判断当前 Busy 周期是否已经满足自动继续条件。
    fn auto_go_due(&self, now: Instant, delay: Duration) -> bool {
        let Some(busy_started_at) = self.busy_started_at else {
            return false;
        };
        let quiet_started_at = self.last_output_at.unwrap_or(busy_started_at);
        now.duration_since(busy_started_at) >= delay
            && now.duration_since(quiet_started_at) >= delay
    }

    /// 返回下一次需要唤醒 watchdog 的延迟。
    fn next_due_delay(&self, now: Instant, delay: Duration) -> Option<Duration> {
        if self.auto_go_sent {
            return None;
        }
        let busy_started_at = self.busy_started_at?;
        let quiet_started_at = self.last_output_at.unwrap_or(busy_started_at);
        let due_at = busy_started_at
            .checked_add(delay)
            .unwrap_or(busy_started_at)
            .max(
                quiet_started_at
                    .checked_add(delay)
                    .unwrap_or(quiet_started_at),
            );
        Some(due_at.saturating_duration_since(now))
    }
}

impl FontPickerFilter {
    /// Clears both raw and normalized font filter text.
    fn clear(&mut self) {
        self.text.clear();
        self.normalized.clear();
    }

    /// Updates the prepared search key after input changes.
    fn sync_normalized(&mut self) {
        self.normalized = self.text.trim().to_lowercase();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NotificationReturnContext {
    workspace_index: usize,
    route: Route,
    center_mode: CenterMode,
    previous_center_mode: CenterMode,
    workspace_terminal_open: bool,
    reviewer_helix_open: bool,
}

impl From<InitialGuiData> for GsdvGuiApp {
    fn from(value: InitialGuiData) -> Self {
        Self::new(value, AgentLaunchConfig::default())
    }
}

impl GsdvGuiApp {
    fn new(value: InitialGuiData, agent_launch: AgentLaunchConfig) -> Self {
        Self::new_with_font_settings(
            value,
            agent_launch,
            data::load_font_settings(),
            scan_system_fonts(),
        )
    }

    fn new_with_font_settings(
        mut value: InitialGuiData,
        agent_launch: AgentLaunchConfig,
        mut font_settings: FontSettings,
        system_fonts: Vec<SystemFontEntry>,
    ) -> Self {
        normalize_font_settings(&mut font_settings, &system_fonts);
        for workspace in &mut value.workspaces {
            if workspace.center_mode == CenterMode::Terminal {
                workspace.center_mode = CenterMode::Agent;
            }
            if workspace.previous_center_mode == CenterMode::Terminal {
                workspace.previous_center_mode = CenterMode::Agent;
            }
        }
        let reviewer_adapters = (0..value.workspaces.len()).map(|_| None).collect();
        let reviewer_snapshots = (0..value.workspaces.len()).map(|_| None).collect();
        let reviewer_dialogs = (0..value.workspaces.len()).map(|_| None).collect();
        let reviewer_diff_scroll_targets = (0..value.workspaces.len()).map(|_| None).collect();
        let reviewer_diff_selected_rows = (0..value.workspaces.len()).map(|_| None).collect();
        let terminal_hosts = (0..value.workspaces.len())
            .map(|_| WorkspaceTerminalHosts::default())
            .collect();
        let active_agent_slots = (0..value.workspaces.len())
            .map(|_| AgentSlotId::Main)
            .collect();
        let agent_busy_watchdogs = (0..value.workspaces.len())
            .map(|_| BTreeMap::from([(AgentSlotId::Main, AgentBusyWatchdogState::default())]))
            .collect();
        let workspace_terminal_drawers = (0..value.workspaces.len()).map(|_| false).collect();
        let reviewer_helix_drawers = (0..value.workspaces.len()).map(|_| false).collect();
        let pending_agent_theme_restarts = (0..value.workspaces.len()).map(|_| None).collect();
        let documents = value
            .workspaces
            .iter()
            .map(|workspace| DocumentState {
                markdown_outline_collapsed: workspace.markdown_outline_collapsed,
                ..DocumentState::default()
            })
            .collect();
        let app_dialogs = (0..value.workspaces.len()).map(|_| None).collect();
        let outline_tree_rects = (0..value.workspaces.len()).map(|_| None).collect();
        let outline_favorites_only = (0..value.workspaces.len()).map(|_| false).collect();
        let outline_panel_tabs = (0..value.workspaces.len())
            .map(|_| OutlinePanelTab::Outline)
            .collect();
        let workflow_states = (0..value.workspaces.len())
            .map(|_| WorkflowUiState::default())
            .collect();
        let memo_save_errors = (0..value.workspaces.len()).map(|_| None).collect();
        let (app_event_tx, app_event_rx) = mpsc::channel();
        let background_runtime = Arc::new(build_background_runtime());
        let input_runtime_tx =
            spawn_input_runtime(background_runtime.handle().clone(), app_event_tx.clone());
        let mut app = Self {
            active_workspace: value.active_workspace,
            workspaces: value.workspaces,
            reviewer_adapters,
            reviewer_snapshots,
            reviewer_dialogs,
            reviewer_diff_scroll_targets,
            reviewer_diff_selected_rows,
            terminal_hosts,
            active_agent_slots,
            agent_busy_watchdogs,
            workspace_terminal_drawers,
            reviewer_helix_drawers,
            pending_agent_theme_restarts,
            documents,
            app_dialogs,
            global_app_dialog: None,
            outline_tree_rects,
            outline_favorites_only,
            outline_panel_tabs,
            workflow_states,
            memo_save_errors,
            markdown_diff_paste_since_ms: u128::from(current_unix_millis()),
            last_agent_input_translation: None,
            agent_input_translation_popup: None,
            active_agent_terminal_rect: None,
            agent_input_translation_watch: None,
            agent_input_translation_in_flight: BTreeSet::new(),
            agent_input_translation_client: None,
            toasts: Vec::new(),
            screenshot_sequence: 0,
            last_screenshot_path: None,
            theme_mode: theme::current_mode(),
            rail_collapsed: value.rail_collapsed,
            agent_launch,
            fs_watcher: Arc::new(Mutex::new(FsWatcherService::new(app_event_tx.clone()))),
            fs_watch_dirty: FsWatchDirtyState::new(),
            pending_memo_saves: BTreeSet::new(),
            pending_markdown_reparse: BTreeSet::new(),
            pending_markdown_outline_collapse: BTreeSet::new(),
            pending_input_repaint: false,
            pending_runtime_settings_save: false,
            pending_language_settings_save: false,
            pending_font_settings_save: false,
            pending_network_settings_save: false,
            screenshot_request_poll_enabled: screenshot_request_poll_enabled(),
            workspace_store_dirty_at: None,
            workspace_store_save_in_flight: Arc::new(AtomicBool::new(false)),
            last_screenshot_request_poll: Instant::now(),
            screenshot_request_read_in_flight: false,
            last_full_frame_at: None,
            render_dirty: true,
            rendering_frame: false,
            app_repaint_ctx: None,
            pending_terminal_spawns: BTreeSet::new(),
            helix_binary_check_in_flight: false,
            pending_helix_open_request: None,
            pending_reviewer_loads: BTreeSet::new(),
            reviewer_loads_in_flight: BTreeSet::new(),
            reviewer_adapter_tasks_in_flight: BTreeSet::new(),
            reviewer_git_data_in_flight: BTreeSet::new(),
            pending_reviewer_git_data_budget: BTreeMap::new(),
            pending_reviewer_adapter_tasks: BTreeMap::new(),
            suppress_editor_input: false,
            helix_binary_available: false,
            reviewer_scripts: ReviewerScriptState::default(),
            extra_tools: ExtraToolsState::new(Instant::now()),
            notifications: NotificationCenter::default(),
            app_event_tx,
            background_runtime,
            app_event_rx,
            input_runtime_tx,
            notification_return_context: None,
            app_language: data::load_app_language(),
            global_outline_favorites: data::load_global_outline_favorites(),
            network_settings: data::load_network_settings(),
            runtime_settings: data::load_runtime_settings(),
            network_settings_dialog_baseline: None,
            codex_auth: CodexAuthUiState::new(),
            font_settings,
            system_fonts,
            default_font_filter: FontPickerFilter::default(),
            default_fallback_font_filter: FontPickerFilter::default(),
            agent_font_filter: FontPickerFilter::default(),
            agent_fallback_font_filter: FontPickerFilter::default(),
            terminal_font_filter: FontPickerFilter::default(),
            terminal_fallback_font_filter: FontPickerFilter::default(),
            editor_font_filter: FontPickerFilter::default(),
            editor_fallback_font_filter: FontPickerFilter::default(),
            pomodoro: PomodoroState::new(Instant::now()),
            pomodoro_cat_menu_open: false,
            hajimi_texture: None,
            suppress_default_agent_input: false,
        };
        app.sync_fs_watches();
        app
    }
}

struct Toast {
    message: String,
    color: Color32,
    created_at: Instant,
}

/// Last successful Agent input translation that can be applied back to a composer.
#[derive(Clone)]
struct AgentInputTranslation {
    /// Workspace that owned the translated draft.
    workspace_index: usize,
    /// Agent slot that owned the translated draft.
    agent_slot: AgentSlotId,
    /// Original draft text that produced this translation.
    source_text: String,
    /// Whether the original draft contained Codex image attachments.
    source_has_images: bool,
    /// Natural English translation returned by the AI backend.
    text: String,
}

/// Floating Agent input translation helper shown above the Agent composer.
#[derive(Clone)]
struct AgentInputTranslationPopup {
    /// Popup body text.
    message: String,
}

/// Lightweight watch state for auto-triggering Agent input translation.
struct AgentInputTranslationWatch {
    /// Workspace that owns the observed draft.
    workspace_index: usize,
    /// Agent slot that owns the observed draft.
    agent_slot: AgentSlotId,
    /// Last visible draft text observed from the terminal.
    text: String,
    /// Time when the draft last changed.
    changed_at: Instant,
    /// Last draft text already submitted for translation.
    last_requested_text: Option<String>,
}

/// Cached Codex translation client keyed by runtime-relevant settings.
struct AgentInputTranslationClientCache {
    /// Network settings used to build the reqwest client.
    network_settings: NetworkSettings,
    /// Whether Codex HTTP fallback is allowed after repeated WS failures.
    allow_http_fallback: bool,
    /// Reusable Codex client with pooled HTTP and WebSocket transport state.
    client: crate::ai::CodexClient,
}

/// Terminal hosts owned by one workspace.
#[derive(Default)]
struct WorkspaceTerminalHosts {
    /// Main agent and subagent terminal hosts keyed by slot.
    agents: BTreeMap<AgentSlotId, AgentHostSlot>,
    /// Workspace shell drawer host.
    workspace: Option<GuiTerminalHost>,
    /// Reviewer Helix drawer host.
    helix: Option<GuiTerminalHost>,
    /// Last workspace shell spawn error.
    workspace_error: Option<String>,
    /// Last Reviewer Helix spawn error.
    helix_error: Option<String>,
}

/// Terminal host/error pair for one agent slot.
#[derive(Default)]
struct AgentHostSlot {
    /// Live embedded terminal process.
    host: Option<GuiTerminalHost>,
    /// Last spawn error for this slot.
    error: Option<String>,
}

#[derive(Default)]
struct DocumentState {
    path: Option<PathBuf>,
    /// 正在后台加载的目标 Markdown，用于避免点击时出现半切换状态。
    loading_path: Option<PathBuf>,
    text: String,
    saved_text: String,
    load_error: Option<String>,
    save_error: Option<String>,
    markdown_scroll_y: f32,
    markdown_outline_collapsed: bool,
    /// Current heading outline rebuilt when Markdown text changes.
    markdown_outline_entries: Vec<MarkdownOutlineEntry>,
    /// Parsed Markdown blocks rebuilt when Markdown text changes.
    markdown_preview_blocks: Vec<markdown_preview::MarkdownBlock>,
    /// Cached preview block positions for virtualized scroll rendering.
    markdown_preview_metrics: Option<markdown_preview::MarkdownRenderMetrics>,
    /// Width used to compute the cached preview block positions.
    markdown_preview_metrics_width: f32,
    markdown_preview_heading_offsets: Vec<f32>,
    markdown_preview_max_scroll_y: f32,
    markdown_editor_max_scroll_y: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
/// 左侧工作区面板的 tab。
enum OutlinePanelTab {
    /// 文件 outline。
    #[default]
    Outline,
    /// gsdv-spec workflow。
    Workflow,
}

#[derive(Debug, Clone, Default)]
/// 单个 workspace 的 workflow UI 状态。
struct WorkflowUiState {
    /// 最近加载出的 workflow tree。
    tree: Option<WorkflowTree>,
    /// workflow 中被折叠的 project key。
    collapsed_project_keys: BTreeSet<String>,
    /// 当前是否正在后台加载 workflow tree。
    loading: bool,
    /// 最近一次 workflow tree 加载错误。
    load_error: Option<String>,
    /// 当前选中的 workflow 目标。
    selected: Option<WorkflowSelectionTarget>,
    /// 当前 task 说明编辑器。
    task_editor: Option<WorkflowTaskEditor>,
    /// 当前叶子 step 片段编辑器。
    editor: Option<WorkflowStepEditor>,
    /// 片段保存成功后要继续打开的目标。
    pending_target_after_save: Option<WorkflowSelectionTarget>,
    /// task 工作台内两个片段 pane 是否显示 Markdown preview。
    preview_fragments: bool,
    /// 当前 task 工作台中多选的 step 路径。
    selected_step_paths: BTreeSet<Vec<usize>>,
    /// Shift 范围选择的锚点 step 路径。
    step_selection_anchor: Option<Vec<usize>>,
    /// 多选状态所属的 task 文档路径。
    step_selection_task_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MarkdownOutlineEntry {
    /// Heading depth from one to six, used for visual indentation.
    level: usize,
    /// Zero-based source line used as a stable scroll target.
    line: usize,
    /// Heading text shown in the Markdown-local outline.
    title: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct MarkdownOutlinePanelResult {
    /// Requested scroll target from a heading click.
    scroll_target: Option<f32>,
    /// Whether the collapse button was clicked.
    collapse_outline: bool,
}

#[derive(Default)]
struct ReviewerScriptState {
    scripts: Vec<ReviewerScript>,
    last_refresh: Option<Instant>,
    last_error: Option<String>,
}

struct FsWatchDirtyState {
    /// Workspaces whose outline should be rebuilt after debounce.
    outline_workspaces: BTreeSet<usize>,
    /// Workspaces whose workflow tree should be rebuilt after debounce.
    workflow_workspaces: BTreeSet<usize>,
    /// First workspace event timestamp used for debounce.
    outline_dirty_at: Option<Instant>,
    /// Workspaces whose loaded reviewer may need uncommitted diff refresh.
    reviewer_workspaces: BTreeSet<usize>,
    /// First reviewer workspace event timestamp used for debounce.
    reviewer_dirty_at: Option<Instant>,
    /// Whether reviewer scripts should be reloaded after debounce.
    reviewer_scripts: bool,
    /// First reviewer script event timestamp used for debounce.
    reviewer_scripts_dirty_at: Option<Instant>,
    /// Whether agent status should be refreshed after debounce.
    agent_status: bool,
    /// First agent status event timestamp used for debounce.
    agent_status_dirty_at: Option<Instant>,
}

impl FsWatchDirtyState {
    /// Creates dirty state with an initial reviewer script load request.
    fn new() -> Self {
        Self {
            outline_workspaces: BTreeSet::new(),
            workflow_workspaces: BTreeSet::new(),
            outline_dirty_at: None,
            reviewer_workspaces: BTreeSet::new(),
            reviewer_dirty_at: None,
            reviewer_scripts: true,
            reviewer_scripts_dirty_at: Some(Instant::now() - FS_WATCH_DEBOUNCE),
            agent_status: false,
            agent_status_dirty_at: None,
        }
    }

    /// Marks one workspace outline dirty from a filesystem event.
    fn mark_outline_dirty(&mut self, index: usize) {
        self.outline_workspaces.insert(index);
        self.outline_dirty_at.get_or_insert_with(Instant::now);
    }

    /// Marks one workspace workflow tree dirty from a spec file event.
    fn mark_workflow_dirty(&mut self, index: usize) {
        self.workflow_workspaces.insert(index);
        self.outline_dirty_at.get_or_insert_with(Instant::now);
    }

    /// Marks one workspace reviewer dirty from a filesystem event.
    fn mark_reviewer_dirty(&mut self, index: usize) {
        self.reviewer_workspaces.insert(index);
        self.reviewer_dirty_at.get_or_insert_with(Instant::now);
    }

    /// Marks reviewer scripts dirty from a filesystem event.
    fn mark_reviewer_scripts_dirty(&mut self) {
        self.reviewer_scripts = true;
        self.reviewer_scripts_dirty_at
            .get_or_insert_with(Instant::now);
    }

    /// Marks agent status dirty from a filesystem event.
    fn mark_agent_status_dirty(&mut self) {
        self.agent_status = true;
        self.agent_status_dirty_at.get_or_insert_with(Instant::now);
    }

    /// Keeps workspace dirty indexes valid after workspace list changes.
    fn clamp_workspace_indexes(&mut self, len: usize) {
        self.outline_workspaces.retain(|index| *index < len);
        self.workflow_workspaces.retain(|index| *index < len);
        if self.outline_workspaces.is_empty() && self.workflow_workspaces.is_empty() {
            self.outline_dirty_at = None;
        }
        self.reviewer_workspaces.retain(|index| *index < len);
        if self.reviewer_workspaces.is_empty() {
            self.reviewer_dirty_at = None;
        }
    }
}

struct FsWatcherService {
    /// 所有被观察 app 路径共享的 notify watcher。
    watcher: Option<notify::RecommendedWatcher>,
    /// notify callback 使用的唯一 AppEvent 队列。
    event_tx: Sender<AppEvent>,
    /// watcher callback 用来唤醒 UI 的可选 egui context。
    repaint_ctx: Arc<Mutex<Option<egui::Context>>>,
    /// 递归注册的 workspace root。
    workspace_roots: Vec<PathBuf>,
    /// 变化后需要触发重载的 reviewer script 目录。
    reviewer_script_dir: Option<PathBuf>,
    /// reviewer script 实际被 watch 的路径。
    reviewer_script_watch_path: Option<PathBuf>,
    /// 变化后需要触发刷新的 Agent status 文件。
    agent_status_path: Option<PathBuf>,
    /// Agent status 实际被 watch 的路径。
    agent_status_watch_path: Option<PathBuf>,
    /// 最近一次 watcher 配置错误。
    last_error: Option<String>,
}

impl FsWatcherService {
    /// 创建 app 级全局文件系统 watcher。
    fn new(event_tx: Sender<AppEvent>) -> Self {
        let repaint_ctx = Arc::new(Mutex::new(None));
        let mut service = Self {
            watcher: None,
            event_tx,
            repaint_ctx,
            workspace_roots: Vec::new(),
            reviewer_script_dir: None,
            reviewer_script_watch_path: None,
            agent_status_path: None,
            agent_status_watch_path: None,
            last_error: None,
        };
        service.ensure_watcher();
        service
    }

    /// Stores the egui context used by watcher callbacks to wake the app.
    fn set_repaint_context(&mut self, ctx: egui::Context) {
        if let Ok(mut repaint_ctx) = self.repaint_ctx.lock() {
            *repaint_ctx = Some(ctx);
        }
    }

    /// Creates the notify watcher once and keeps future registrations on it.
    fn ensure_watcher(&mut self) {
        if self.watcher.is_some() {
            return;
        }
        let event_tx = self.event_tx.clone();
        let repaint_ctx = self.repaint_ctx.clone();
        match notify::recommended_watcher(move |event| {
            let _ = event_tx.send(AppEvent::FsWatch(event));
            if let Ok(repaint_ctx) = repaint_ctx.lock()
                && let Some(ctx) = repaint_ctx.as_ref()
            {
                ctx.request_repaint();
            }
        }) {
            Ok(watcher) => {
                self.watcher = Some(watcher);
                self.last_error = None;
            }
            Err(error) => {
                self.last_error = Some(error.to_string());
            }
        }
    }

    /// Registers the current workspace roots on the shared watcher.
    fn sync_workspace_roots(&mut self, workspace_paths: &[PathBuf]) {
        let next_roots = workspace_paths
            .iter()
            .map(|path| comparable_watch_path(path))
            .collect::<Vec<_>>();
        if self.workspace_roots == next_roots {
            return;
        }
        for root in self.workspace_roots.clone() {
            self.unwatch_path(&root);
        }
        self.workspace_roots.clear();
        for root in next_roots {
            if self.watch_path(&root, RecursiveMode::Recursive) {
                self.workspace_roots.push(root);
            }
        }
    }

    /// 注册 reviewer script 和 Agent status 路径。
    fn sync_global_paths(&mut self) {
        self.sync_reviewer_script_path(reviewer_script_dir());
        self.sync_agent_status_path(data::agent_status_path());
    }

    /// 将单个 notify 事件映射到受影响的 app 资源。
    fn map_notify_event(&self, event: notify::Event, events: &mut Vec<FsWatchAppEvent>) {
        let mut status_changed = false;
        let mut scripts_changed = false;
        let mut workspace_indexes = BTreeSet::new();
        let mut workflow_indexes = BTreeSet::new();
        for path in event.paths {
            let path = comparable_watch_path(&path);
            if self
                .agent_status_path
                .as_ref()
                .is_some_and(|status_path| path == *status_path)
            {
                status_changed = true;
            }
            if self
                .reviewer_script_dir
                .as_ref()
                .is_some_and(|script_dir| path == *script_dir || path.starts_with(script_dir))
            {
                scripts_changed = true;
            }
            for (index, root) in self.workspace_roots.iter().enumerate() {
                if path == *root || path.starts_with(root) {
                    if path != *root && should_skip_workspace_watch_path(root, &path) {
                        continue;
                    }
                    workspace_indexes.insert(index);
                    if path_is_workflow_spec_path(root, &path) {
                        workflow_indexes.insert(index);
                    }
                }
            }
        }
        if status_changed {
            events.push(FsWatchAppEvent::AgentStatusChanged);
        }
        if scripts_changed {
            events.push(FsWatchAppEvent::ReviewerScriptsChanged);
        }
        for index in workspace_indexes {
            events.push(FsWatchAppEvent::WorkspaceChanged {
                index,
                workflow: workflow_indexes.contains(&index),
            });
        }
    }

    /// Updates the watched reviewer script anchor path.
    fn sync_reviewer_script_path(&mut self, next_dir: Option<PathBuf>) {
        let next_dir = next_dir.map(|path| comparable_watch_path(&path));
        if self.reviewer_script_dir == next_dir {
            return;
        }
        if let Some(path) = self.reviewer_script_watch_path.take() {
            self.unwatch_path(&path);
        }
        self.reviewer_script_dir = next_dir;
        if let Some(dir) = self.reviewer_script_dir.as_ref() {
            let watch_path = nearest_existing_watch_path(dir);
            if self.watch_path(&watch_path, RecursiveMode::NonRecursive) {
                self.reviewer_script_watch_path = Some(watch_path);
            }
        }
    }

    /// Updates the watched agent status anchor path.
    fn sync_agent_status_path(&mut self, next_path: Option<PathBuf>) {
        let next_path = next_path.map(|path| comparable_watch_path(&path));
        if self.agent_status_path == next_path {
            return;
        }
        if let Some(path) = self.agent_status_watch_path.take() {
            self.unwatch_path(&path);
        }
        self.agent_status_path = next_path;
        if let Some(status_path) = self.agent_status_path.as_ref() {
            let watch_path = status_path
                .parent()
                .map(nearest_existing_watch_path)
                .unwrap_or_else(|| nearest_existing_watch_path(status_path));
            if self.watch_path(&watch_path, RecursiveMode::NonRecursive) {
                self.agent_status_watch_path = Some(watch_path);
            }
        }
    }

    /// Registers one path and records setup errors without panicking.
    fn watch_path(&mut self, path: &Path, recursive_mode: RecursiveMode) -> bool {
        self.ensure_watcher();
        let Some(watcher) = self.watcher.as_mut() else {
            return false;
        };
        match watcher.watch(path, recursive_mode) {
            Ok(()) => {
                self.last_error = None;
                true
            }
            Err(error) => {
                self.last_error = Some(format!("{}: {error}", path.display()));
                false
            }
        }
    }

    /// Removes one path from the shared watcher.
    fn unwatch_path(&mut self, path: &Path) {
        if let Some(watcher) = self.watcher.as_mut() {
            let _ = watcher.unwatch(path);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FsWatchAppEvent {
    /// A workspace file tree changed.
    WorkspaceChanged {
        /// Workspace index affected by the event.
        index: usize,
        /// Whether the changed path is under the workflow spec directory.
        workflow: bool,
    },
    /// Reviewer script directory changed.
    ReviewerScriptsChanged,
    /// Shared agent status file changed.
    AgentStatusChanged,
    /// Watcher setup or runtime error.
    WatcherError(String),
}

#[derive(Default)]
struct NotificationCenter {
    open: bool,
    lines: VecDeque<String>,
    scroll_to_bottom: bool,
}

impl NotificationCenter {
    fn open(&mut self) {
        self.open = true;
        self.scroll_to_bottom = true;
    }

    fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.scroll_to_bottom = true;
        }
    }

    fn close(&mut self) {
        self.open = false;
    }

    fn clear(&mut self) {
        self.lines.clear();
        self.scroll_to_bottom = true;
    }

    fn push_line(&mut self, line: impl Into<String>) {
        self.lines.push_back(line.into());
        while self.lines.len() > NOTIFICATION_MAX_LINES {
            self.lines.pop_front();
        }
        self.scroll_to_bottom = true;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReviewerScript {
    label: String,
    path: PathBuf,
    tip: Option<String>,
}

/// 影响当前界面状态或触发界面状态更新任务的唯一 app 事件类型。
///
/// 只能表示“UI 状态如何变化”或“需要派发哪类后台工作”。
/// 非渲染业务副作用不要塞进这里；例如 store 持久化只标 dirty，
/// 由独立 writer 合并处理。
enum AppEvent {
    /// 单个 workspace 文档的 Markdown 解析完成。
    MarkdownParsed {
        index: usize,
        source_text: String,
        outline_entries: Vec<MarkdownOutlineEntry>,
        preview_blocks: Vec<markdown_preview::MarkdownBlock>,
    },
    /// 单个 workspace 的 memo 保存完成。
    MemoSaved { index: usize, error: Option<String> },
    /// 单个 workspace 的 outline 刷新完成。
    WorkspaceOutlineRefreshed {
        index: usize,
        workspace: WorkspaceViewData,
    },
    /// 单个 workspace 的 workflow tree 刷新完成。
    WorkflowTreeLoaded {
        index: usize,
        workspace_path: PathBuf,
        result: Result<WorkflowTree, String>,
    },
    /// 添加 workspace 的后台准备完成。
    WorkspaceAddPrepared {
        result: Result<WorkspaceAddTaskResult, String>,
    },
    /// 当前 workspace 列表的 Agent 状态刷新完成。
    AgentStatusesRefreshed {
        workspaces: Vec<WorkspaceViewData>,
        changed: bool,
    },
    /// Reviewer script 目录扫描完成。
    ReviewerScriptsLoaded {
        result: Result<Vec<ReviewerScript>, String>,
    },
    /// 外置工具目录扫描和初始 value 加载完成。
    ExtraToolsScanned {
        workspace_path: Option<PathBuf>,
        result: Result<ExtraToolsScanResult, String>,
    },
    /// 单个外置工具 value 刷新完成。
    ExtraToolValueLoaded {
        key: ExtraToolKey,
        result: Result<String, String>,
    },
    /// 单个外置工具 action 执行完成。
    ExtraToolActionFinished {
        key: ExtraToolKey,
        action: String,
        result: ExtraToolActionResult,
    },
    /// 用户请求执行单个外置工具 action。
    ExtraToolActionRequested { key: ExtraToolKey, action: String },
    /// 用户请求中断单个外置工具 action。
    ExtraToolInterruptRequested { key: ExtraToolKey },
    /// 单个 workspace 的 Markdown 文件加载和解析完成。
    DocumentLoaded {
        index: usize,
        path: PathBuf,
        absolute: PathBuf,
        markdown_outline_collapsed: bool,
        result: Result<LoadedDocument, String>,
    },
    /// 单个 workspace 的 Markdown 文件保存完成。
    DocumentSaved {
        index: usize,
        path: PathBuf,
        text: String,
        result: Result<(), String>,
        diff_history_error: Option<String>,
    },
    /// workflow step 左右片段保存完成。
    WorkflowStepSaved {
        index: usize,
        target: WorkflowSelectionTarget,
        result: Result<WorkflowSaveSuccess, String>,
    },
    /// workflow tree 右键菜单文件修改完成。
    WorkflowMutationFinished {
        index: usize,
        request: WorkflowMutationRequest,
        result: Result<(), String>,
    },
    /// Recent Markdown diff context prepared for the active Agent input.
    MarkdownDiffPromptBuilt { result: Result<String, String> },
    /// Current Agent input translated by the configured AI backend.
    AgentInputTranslationFinished {
        workspace_index: usize,
        agent_slot: AgentSlotId,
        source_text: String,
        source_has_images: bool,
        result: Result<String, String>,
    },
    /// 后台构建好的运行时字体定义。
    RuntimeFontsPrepared {
        settings: FontSettings,
        fonts: egui::FontDefinitions,
    },
    /// UI 显式命令触发的文件系统修改完成。
    FileMutationFinished(FileMutationResult),
    /// workspace close 前的 sidecar 删除完成。
    WorkspaceCloseSidecarsDeleted {
        index: usize,
        workspace_path: PathBuf,
        result: Result<(), String>,
    },
    /// 截图图片落盘完成。
    ScreenshotSaved {
        path: PathBuf,
        result: Result<(), String>,
    },
    /// input runtime 解析出的截图完成事件。
    ScreenshotCaptured {
        path: Option<PathBuf>,
        image: Arc<egui::ColorImage>,
    },
    /// 可选截图请求文件读取完成。
    ScreenshotRequestLoaded {
        result: Result<Option<String>, String>,
    },
    /// Terminal host 创建完成。
    TerminalHostSpawned {
        key: TerminalSpawnKey,
        result: Result<GuiTerminalHost, String>,
    },
    /// terminal runtime 投递的粗粒度 UI 状态通知。
    TerminalRuntime(TerminalRuntimeEvent),
    /// 终端文件点击的 Helix 启动参数构建完成。
    TerminalFileHelixSpecBuilt {
        workspace_index: usize,
        spec: HelixLaunchSpec,
    },
    /// 终端路径点击的文件系统分类完成。
    TerminalOutputPathClassified {
        workspace_index: usize,
        click: TerminalOutputClick,
    },
    /// Helix 可执行文件可用性检查完成。
    HelixBinaryChecked { available: bool },
    /// 文件管理器定位命令完成。
    RevealPathFinished { result: Result<(), String> },
    /// 单个 repo 的 reviewer 分支列表加载完成。
    ReviewerBranchChoicesLoaded {
        repo: ReviewerBranchTarget,
        result: Result<(String, Vec<BranchInfo>), String>,
    },
    /// Reviewer 分支切换和重载完成。
    ReviewerBranchSwitchFinished {
        repo: ReviewerBranchTarget,
        target: String,
        result: Result<ReviewerAdapter, String>,
    },
    /// Reviewer adapter 加载完成。
    ReviewerAdapterLoaded {
        index: usize,
        result: Result<ReviewerAdapter, String>,
    },
    /// Reviewer adapter 修改任务完成。
    ReviewerAdapterTaskFinished {
        index: usize,
        result: Result<ReviewerAdapter, String>,
        effect: ReviewerAdapterTaskEffect,
    },
    /// Reviewer git data loaded without taking the UI adapter.
    ReviewerGitDataLoaded {
        index: usize,
        result: Result<ReviewerGitDataResult, String>,
    },
    /// Codex OAuth 浏览器授权完成。
    CodexAuthFinished {
        result: Result<crate::ai::CodexAuthInfo, String>,
    },
    /// 文件系统 watcher 发出一个原始 notify 事件。
    FsWatch(notify::Result<notify::Event>),
    /// Reviewer script 产生一行通知。
    Notification(String),
    /// input runtime 解析出的 UI 命令。
    InputUiCommand(UiCommand),
    /// input runtime 解析出的 reviewer diff 动作。
    InputReviewerDiffAction(crate::gui::diff_viewer::DiffViewerAction),
    /// input runtime 解析出的 terminal 输入字节。
    InputTerminalBytes {
        workspace_index: usize,
        target: TerminalSurfaceKind,
        agent_slot: AgentSlotId,
        bytes: Vec<u8>,
    },
    /// input runtime 检测到番茄钟相关输入。
    PomodoroInputDetected,
    /// 派发 settings 保存副作用。
    ProcessPendingSettingsSideEffects,
    /// 派发 reviewer adapter 加载任务。
    ProcessPendingReviewerLoads,
    /// 派发 Markdown 重解析任务。
    ProcessPendingMarkdownReparse,
    /// 应用 Markdown outline 折叠请求。
    ProcessPendingMarkdownOutlineCollapse,
    /// 派发 memo 保存任务。
    ProcessPendingMemoSaves,
    /// 处理 IME fallback 需要的下一帧 repaint。
    ProcessPendingInputRepaint,
    /// 启用时轮询截图请求文件。
    HandleScreenshotRequestFile,
    /// 派发已防抖的文件系统工作。
    ProcessFsWatchDirty,
    /// 完成主题变化触发的延迟 Agent 重启。
    FinishPendingAgentThemeRestarts,
    /// 更新当前 route 的终端 repaint gate。
    SyncTerminalEventRepaintFlags,
    /// 处理 busy agent watchdog 截止时间。
    ProcessAgentBusyWatchdogs,
    /// 推进番茄钟状态。
    ProcessPomodoroState,
    /// 派发外置工具扫描、刷新和 action 后续工作。
    ProcessExtraTools,
}

/// input runtime 消费的 egui 原生输入快照。
struct InputRuntimeRequest {
    /// egui 本帧输入状态快照。
    input: egui::InputState,
    /// 当前是否有 egui 文本控件需要键盘输入。
    wants_keyboard_input: bool,
    /// 输入快照对应的 active workspace。
    active_workspace: usize,
    /// 输入快照对应的 active agent slot。
    active_agent_slot: AgentSlotId,
    /// 当前 active Agent 是否处于 Busy 状态。
    active_agent_busy: bool,
    /// 当前 workspace route。
    route: Route,
    /// 当前 workspace center mode。
    center_mode: CenterMode,
    /// app modal dialog 是否打开。
    active_app_dialog_open: bool,
    /// 外置工具 modal 是否打开。
    extra_tools_open: bool,
    /// 当前 app modal 是否为 Agent 输入翻译弹窗。
    agent_translation_dialog_open: bool,
    /// reviewer modal dialog 是否打开。
    active_reviewer_dialog_open: bool,
    /// notification drawer 是否打开。
    notifications_open: bool,
    /// workspace terminal drawer 是否打开。
    workspace_terminal_open: bool,
    /// reviewer helix drawer 是否打开。
    reviewer_helix_open: bool,
    /// 基础 outline 面板是否在当前 route 可见。
    outline_visible: bool,
    /// 最近访问 Markdown modal 当前是否打开。
    recent_markdown_dialog_open: bool,
    /// Escape 是否允许关闭当前键盘层。
    keyboard_layer_can_close_with_escape: bool,
    /// active outline tree 最近绘制区域。
    outline_tree_rect: Option<Rect>,
    /// 当前上下文是否允许 helix 快捷键。
    helix_shortcut_allowed: bool,
    /// reviewer diff 当前键盘选中行。
    selected_reviewer_diff_row: Option<usize>,
    /// 默认 terminal 输入目标。
    terminal_input_target: Option<TerminalSurfaceKind>,
    /// 可见 terminal surface 是否已经接管本帧键盘输入。
    terminal_surface_owns_input: bool,
    /// 默认 terminal 输入目标是否已启用 kitty keyboard protocol。
    terminal_kitty_keyboard_protocol: bool,
    /// input runtime 投回 AppEvent 后用于唤醒 UI。
    repaint_ctx: egui::Context,
    /// app FPS gate 对应的 repaint 间隔。
    repaint_after: Duration,
    /// 番茄钟是否启用。
    pomodoro_enabled: bool,
    /// 当前番茄钟阶段。
    pomodoro_phase: PomodoroPhase,
}

#[derive(Debug)]
struct LoadedDocument {
    /// File text read from disk.
    text: String,
    /// Heading outline parsed off the UI thread.
    outline_entries: Vec<MarkdownOutlineEntry>,
    /// Markdown preview blocks parsed off the UI thread.
    preview_blocks: Vec<markdown_preview::MarkdownBlock>,
}

/// Unique key for a pending terminal spawn request.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TerminalSpawnKey {
    /// Workspace slot that will receive the host.
    index: usize,
    /// Terminal surface to create for that workspace.
    kind: TerminalSurfaceKind,
    /// Agent slot when `kind` is Agent.
    agent_slot: AgentSlotId,
}

/// Identifies the main agent or one named subagent.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum AgentSlotId {
    /// The workspace's original single agent.
    Main,
    /// A named secondary agent stored in the workspace sidecar.
    Subagent(String),
}

impl AgentSlotId {
    /// Returns whether this slot is the workspace main agent.
    fn is_main(&self) -> bool {
        matches!(self, Self::Main)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HelixOpenRequest {
    /// Open Helix at the current reviewer selection.
    ReviewerSelection,
    /// Open Helix for the active workspace root.
    WorkspaceRoot,
    /// Open Helix for a terminal-detected file target.
    TerminalFile(HelixLaunchSpec),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReviewerAdapterTask {
    /// Reload reviewer data.
    Reload,
    /// Refresh uncommitted rows and selected working-tree diff.
    RefreshUncommitted,
    /// Toggle full diff mode.
    ToggleFullDiff,
    /// Refresh dirty state for a repo row.
    RefreshRepoDirty(usize),
    /// Load files and diff for the current lightweight selection.
    EnsureSelectedGitData { row_budget: usize },
    /// Load another page of commits for the selected repo.
    LoadMoreSelectedRepoCommits { row_budget: usize },
    /// Select a diff viewer row.
    SelectDiffRow(usize),
    /// Select a row from a reviewer column.
    ClickRow {
        column: usize,
        row: usize,
        commit_row_budget: usize,
    },
    /// Jump between full diff change blocks.
    JumpFullBlock { reverse: bool },
    /// Move to previous reviewer item.
    PreviousItem,
    /// Move to next reviewer item.
    NextItem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReviewerAdapterTaskEffect {
    /// No extra UI work after replacing the adapter.
    None,
    /// Show reviewer reload success.
    Reloaded,
    /// Queue diff scroll sync after replacing the adapter.
    SyncDiffScroll,
}

#[derive(Debug)]
enum WorkspaceAddTaskResult {
    /// 已存在的 workspace，只需要切换过去。
    Existing {
        /// 任务启动时匹配到的 workspace 下标。
        index: usize,
        /// 任务启动时匹配到的 workspace 路径。
        path: PathBuf,
    },
    /// 新 workspace 已完成磁盘侧数据加载。
    New {
        /// 需要插入 UI 状态树的 workspace。
        workspace: WorkspaceViewData,
    },
}

#[derive(Debug)]
enum FileMutationResult {
    /// Markdown file creation result.
    CreateMarkdown {
        index: usize,
        target: PathBuf,
        result: Result<(), String>,
    },
    /// Folder creation result.
    CreateFolder {
        index: usize,
        result: Result<(), String>,
    },
    /// File or folder rename result.
    Rename {
        index: usize,
        old_relative: PathBuf,
        new_relative: PathBuf,
        result: Result<(), String>,
    },
    /// Markdown deletion result.
    DeleteMarkdown {
        index: usize,
        path: PathBuf,
        result: Result<(), String>,
    },
}

#[derive(Debug)]
enum FileMutationTask {
    /// Create one Markdown file with prepared body text.
    CreateMarkdown {
        index: usize,
        absolute_dir: PathBuf,
        absolute_file: PathBuf,
        target: PathBuf,
        content: String,
    },
    /// Create one folder.
    CreateFolder { index: usize, absolute_dir: PathBuf },
    /// Rename one filesystem entry.
    Rename {
        index: usize,
        absolute: PathBuf,
        target: PathBuf,
        old_relative: PathBuf,
        new_relative: PathBuf,
    },
    /// Delete one Markdown file.
    DeleteMarkdown {
        index: usize,
        path: PathBuf,
        absolute: PathBuf,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReviewerScriptRunRequest {
    script: ReviewerScript,
    target: ReviewerBranchTarget,
}

impl DocumentState {
    fn is_dirty(&self) -> bool {
        self.path.is_some() && self.text != self.saved_text
    }
}

#[derive(Clone)]
enum AppDialog {
    RecentMarkdownOutline {
        nodes: Vec<OutlineNode>,
    },
    UnsavedSwitch {
        target: PathBuf,
    },
    WorkflowUnsavedSwitch {
        target: WorkflowSelectionTarget,
    },
    WorkflowAddTask {
        project_key: String,
        key: String,
    },
    WorkflowAddProject {
        key: String,
    },
    WorkflowAddStep {
        task_path: PathBuf,
        key: String,
        desc: String,
    },
    WorkflowRenameProject {
        project_key: String,
        key: String,
    },
    WorkflowRenameTask {
        task_path: PathBuf,
        key: String,
    },
    WorkflowRenameStep {
        task_path: PathBuf,
        step_path: Vec<usize>,
        key: String,
    },
    WorkflowMergeSteps {
        task_path: PathBuf,
        step_paths: Vec<Vec<usize>>,
        title: String,
    },
    WorkflowDeleteConfirm {
        target: WorkflowDeleteTarget,
    },
    CreateMarkdown {
        dir: PathBuf,
        name: String,
    },
    CreateFolder {
        dir: PathBuf,
        name: String,
    },
    RenamePath {
        path: PathBuf,
        name: String,
    },
    DeleteMarkdown {
        path: PathBuf,
    },
    CloseWorkspace {
        index: usize,
    },
    AddSubagent {
        index: usize,
        name: String,
        agent_kind: AgentKind,
        agent_model: String,
        agent_effort: String,
        agent_fast_mode: Option<bool>,
        session_id: String,
    },
    RestartAgent {
        index: usize,
    },
    SwitchAgent {
        index: usize,
        next_kind: AgentKind,
    },
    SetAgentModel {
        index: usize,
        slot: AgentSlotId,
        model: String,
    },
    ConfirmThemeSwitch {
        next_mode: theme::ThemeMode,
    },
    AgentExitedAbnormally {
        exit: AgentProcessExit,
    },
    Help,
    Settings,
    About,
    CodexAuth,
    Message {
        title: String,
        message: String,
    },
}

/// workflow 删除确认弹窗中的删除目标。
#[derive(Clone)]
enum WorkflowDeleteTarget {
    /// 删除整个 project 目录。
    Project {
        /// 项目目录名。
        project_key: String,
    },
    /// 删除一个 task 文件。
    Task {
        /// task 文档相对 workspace 的路径。
        task_path: PathBuf,
        /// 弹窗里展示的 task 名称。
        label: String,
    },
    /// 删除一个 step 子树。
    Step {
        /// step 所在 task 文档相对 workspace 的路径。
        task_path: PathBuf,
        /// step 在 task 内的路径。
        step_path: Vec<usize>,
        /// 弹窗里展示的 step key。
        title: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutlineFavoriteScope {
    Global,
    Workspace,
}

enum WorkspaceRailAction {
    Switch(usize),
    Close(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AgentTabAction {
    Restart(AgentSlotId),
    Switch {
        slot: AgentSlotId,
        next_kind: AgentKind,
    },
    SetModel {
        slot: AgentSlotId,
        model: String,
    },
    SetEffort {
        slot: AgentSlotId,
        effort: Option<String>,
    },
    SetFastMode {
        slot: AgentSlotId,
        fast_mode: Option<bool>,
    },
    CopySessionId(String),
    SetMarkdownOutlineCollapsed(bool),
    MoveSubagentLeft(String),
    MoveSubagentRight(String),
    MoveSubagentToHead(String),
    MoveSubagentToTail(String),
    MoveSubagentToWorkspace {
        id: String,
        target_index: usize,
    },
}

#[derive(Clone)]
enum ReviewerDialog {
    Message {
        title: String,
        message: String,
    },
    Dirty {
        repo: ReviewerBranchTarget,
        message: String,
    },
    BranchList {
        repo: ReviewerBranchTarget,
        current: String,
        branches: Vec<BranchInfo>,
        selected: usize,
        filter: String,
        visible: Vec<usize>,
    },
    BranchConfirm {
        repo: ReviewerBranchTarget,
        current: String,
        branch: BranchInfo,
    },
    ScriptConfirm {
        request: ReviewerScriptRunRequest,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiCommand {
    CloseTopLayer,
    OpenHelp,
    SaveDocument,
    CopyWorkflowPath,
    CaptureScreenshot,
    ToggleWorkspaceTerminal,
    ToggleNotifications,
    ToggleRecentMarkdownOutline,
    PasteRecentMarkdownDiffsToAgent,
    TranslateAgentInput,
    ApplyAgentInputTranslation,
    ToggleExtraTools,
    AgentMarkdownShortcut,
    ToggleMarkdownEditorPreview,
    SetCenterMode(CenterMode),
    ToggleReviewerHelix,
    OpenReviewerRoute,
    ExitReviewerRoute,
    AddWorkspace,
    OpenSettings,
    SwitchActiveWorkspace,
    SwitchInactiveWorkspace,
    SelectAgentSlot(usize),
    Reviewer(ReviewerCommand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReviewerCommand {
    PreviousColumn,
    NextColumn,
    PreviousItem,
    NextItem,
    JumpPreviousBlock,
    JumpNextBlock,
    CopySelectionToAgent,
    CopyDiffToAgent,
    Reload,
    ToggleFullDiff,
    OpenBranchDialog,
}

impl eframe::App for GsdvGuiApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::bg().to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.app_repaint_ctx = Some(ctx.clone());
        self.process_update_events(ctx);
        self.process_workspace_store_writer(ctx);
        self.paint_update_frame(ctx);
        self.process_agent_input_translation_auto_trigger(ctx);
        self.schedule_next_update(ctx);
    }
}

impl GsdvGuiApp {
    /// Runs egui layout and paint for the current immutable app state.
    fn paint_update_frame(&mut self, ctx: &egui::Context) {
        self.render_dirty = false;
        self.rendering_frame = true;

        TopBottomPanel::bottom("window_bottombar")
            .exact_height(BOTTOM_BAR_HEIGHT)
            .frame(bottom_bar_frame())
            .show(ctx, |ui| self.bottom_bar(ui));

        let workspace_rail_width = if self.rail_collapsed {
            COMPACT_WORKSPACE_RAIL_WIDTH
        } else {
            WORKSPACE_RAIL_WIDTH
        };
        SidePanel::left("workspace_rail")
            .exact_width(workspace_rail_width)
            .frame(panel_frame())
            .show(ctx, |ui| self.workspace_rail(ui));

        if should_show_outline_panel(self.current_workspace()) {
            SidePanel::left("outline_panel")
                .exact_width(272.0)
                .frame(panel_frame())
                .show(ctx, |ui| self.outline_panel(ui));
        }

        CentralPanel::default()
            .frame(panel_frame())
            .show(ctx, |ui| self.center_panel(ui));

        self.workspace_terminal_overlay(ctx);
        self.reviewer_helix_overlay(ctx);
        self.notification_overlay(ctx);
        self.extra_tools_dialog(ctx);
        self.app_dialog(ctx);
        self.reviewer_dialog(ctx);
        self.agent_input_translation_popup(ctx);
        self.pomodoro_work_peek_overlay(ctx);
        self.pomodoro_cat_overlay(ctx);
        self.toast_overlay(ctx);
        self.bottom_bar_overlay(ctx);

        self.rendering_frame = false;
        self.last_full_frame_at = Some(Instant::now());
        if self.render_dirty {
            self.schedule_dirty_render(ctx);
        }
    }

    fn current_workspace(&self) -> Option<&WorkspaceViewData> {
        self.workspaces.get(self.active_workspace)
    }

    fn current_workspace_mut(&mut self) -> Option<&mut WorkspaceViewData> {
        self.workspaces.get_mut(self.active_workspace)
    }

    /// Returns the configured lower bound between application-driven frames.
    fn max_repaint_interval(&self) -> Duration {
        max_frame_rate_interval(self.runtime_settings.max_frame_rate)
    }

    /// Requests a UI pass through the app repaint throttle.
    fn request_app_repaint(&mut self, ctx: &egui::Context) {
        self.mark_render_dirty(ctx);
    }

    /// Requests a delayed UI pass while preserving the app repaint throttle.
    fn request_app_repaint_after(&self, ctx: &egui::Context, duration: Duration) {
        ctx.request_repaint_after(duration.max(self.max_repaint_interval()));
    }

    /// Marks a frame dirty and schedules the next allowed full render.
    fn mark_render_dirty(&mut self, ctx: &egui::Context) {
        self.render_dirty = true;
        if !self.rendering_frame {
            self.schedule_dirty_render(ctx);
        }
    }

    /// Wakes the UI when the FPS gate allows a dirty frame to be consumed.
    fn schedule_dirty_render(&self, ctx: &egui::Context) {
        let delay = self
            .last_full_frame_at
            .map(|last_frame| {
                self.max_repaint_interval()
                    .saturating_sub(last_frame.elapsed())
            })
            .unwrap_or(Duration::ZERO);
        ctx.request_repaint_after(delay);
    }

    /// 调度没有原生事件唤醒的 route 和后台工作。
    fn schedule_next_update(&self, ctx: &egui::Context) {
        let mut next = self.screenshot_request_poll_enabled.then(|| {
            duration_until_due(
                self.last_screenshot_request_poll,
                SCREENSHOT_REQUEST_POLL_INTERVAL,
            )
        });
        next = min_optional_duration(next, self.next_fs_watch_dirty_delay());
        next = min_optional_duration(next, self.next_workspace_store_save_delay());
        next = min_optional_duration(next, self.next_agent_busy_watchdog_delay());
        next = min_optional_duration(next, self.next_pending_agent_theme_restart_delay());
        next = min_optional_duration(next, self.next_toast_expiration_delay());
        next = min_optional_duration(next, self.next_pomodoro_state_delay());
        next = min_optional_duration(next, self.next_pomodoro_delay());
        next = min_optional_duration(next, self.next_extra_tools_delay());
        if let Some(duration) = next {
            self.request_app_repaint_after(ctx, duration);
        }
    }

    /// 判断番茄钟状态机是否需要进入事件队列。
    fn pomodoro_state_event_due(&self) -> bool {
        if !self.runtime_settings.pomodoro_enabled {
            return false;
        }
        self.next_pomodoro_state_delay()
            .is_some_and(|delay| delay.is_zero())
    }

    /// 返回番茄钟状态机的最近截止时间，不包含纯动画帧。
    fn next_pomodoro_state_delay(&self) -> Option<Duration> {
        if !self.runtime_settings.pomodoro_enabled {
            return None;
        }
        let now = Instant::now();
        let elapsed = now.duration_since(self.pomodoro.phase_started_at);
        match self.pomodoro.phase {
            PomodoroPhase::Working => {
                Some(pomodoro_work_duration(&self.runtime_settings).saturating_sub(elapsed))
            }
            PomodoroPhase::WaitingForRestQuiet => {
                Some(POMODORO_REST_QUIET_DURATION.saturating_sub(elapsed))
            }
            PomodoroPhase::Resting => {
                Some(pomodoro_rest_duration(&self.runtime_settings).saturating_sub(elapsed))
            }
            PomodoroPhase::ReadyToWork => None,
            PomodoroPhase::ReturningToWork => {
                Some(POMODORO_RETURN_TO_WORK_DURATION.saturating_sub(elapsed))
            }
        }
    }

    /// 返回番茄钟计时器或动画需要的最近唤醒时间。
    fn next_pomodoro_delay(&self) -> Option<Duration> {
        if !self.runtime_settings.pomodoro_enabled {
            return None;
        }
        let now = Instant::now();
        match self.pomodoro.phase {
            PomodoroPhase::Working => {
                let total = pomodoro_work_duration(&self.runtime_settings);
                let elapsed = now.duration_since(self.pomodoro.phase_started_at);
                let peek_at = total.mul_f32(pomodoro_warning_progress(&self.runtime_settings));
                if elapsed >= peek_at {
                    Some(POMODORO_ANIMATION_FRAME)
                } else {
                    Some(peek_at.saturating_sub(elapsed))
                }
            }
            PomodoroPhase::WaitingForRestQuiet
            | PomodoroPhase::Resting
            | PomodoroPhase::ReadyToWork
            | PomodoroPhase::ReturningToWork => Some(POMODORO_ANIMATION_FRAME),
        }
    }

    /// 返回 Busy Agent 无输出自动继续的最近唤醒时间。
    fn next_agent_busy_watchdog_delay(&self) -> Option<Duration> {
        let now = Instant::now();
        let delay = agent_busy_auto_go_delay(&self.runtime_settings);
        self.workspaces
            .iter()
            .enumerate()
            .flat_map(|(index, workspace)| {
                agent_slots_for_workspace(workspace)
                    .into_iter()
                    .filter_map(move |slot| {
                        if self.agent_slot_activity(index, &slot) != WorkspaceActivity::Busy {
                            return None;
                        }
                        self.agent_busy_watchdogs
                            .get(index)
                            .and_then(|states| states.get(&slot))
                            .and_then(|state| state.next_due_delay(now, delay))
                    })
            })
            .min()
    }

    /// 返回 workspace store writer 的最近合并保存时间。
    fn next_workspace_store_save_delay(&self) -> Option<Duration> {
        self.workspace_store_dirty_at
            .map(|dirty_at| duration_until_due(dirty_at, WORKSPACE_STORE_SAVE_DEBOUNCE))
    }

    /// 返回主题变化触发的 Agent 延迟重启最近时间。
    fn next_pending_agent_theme_restart_delay(&self) -> Option<Duration> {
        let now = Instant::now();
        self.pending_agent_theme_restarts
            .iter()
            .filter_map(|pending| pending.map(|deadline| deadline.saturating_duration_since(now)))
            .min()
    }

    /// 返回 toast 过期的最近时间，避免过期 UI 残留。
    fn next_toast_expiration_delay(&self) -> Option<Duration> {
        let now = Instant::now();
        self.toasts
            .iter()
            .map(|toast| {
                Duration::from_secs(4).saturating_sub(now.duration_since(toast.created_at))
            })
            .min()
    }

    fn workspace_terminal_drawer_is_open(&self) -> bool {
        self.current_workspace().is_some()
            && self
                .workspace_terminal_drawers
                .get(self.active_workspace)
                .copied()
                .unwrap_or(false)
    }

    fn reviewer_helix_drawer_is_open(&self) -> bool {
        self.current_workspace().is_some()
            && self
                .reviewer_helix_drawers
                .get(self.active_workspace)
                .copied()
                .unwrap_or(false)
    }
}

#[cfg(test)]
#[path = "app_test.rs"]
mod app_test;

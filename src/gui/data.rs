use crate::gui::agent::AgentKind;
use crate::gui::theme::ThemeMode;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::env;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const HOME_OUTLINE_ROOT: &str = "~";
const HOME_OUTLINE_DIRS: [&str; 3] = [".codex", ".agents", ".claude"];
const TRANSCRIPT_ABORT_CACHE_LIMIT: usize = 256;
pub const BUILTIN_NO_PROXY: [&str; 2] = ["192.168.0.0/16", "127.0.0.1/8"];
pub const DEFAULT_MAX_FRAME_RATE: u16 = 30;
pub const MIN_MAX_FRAME_RATE: u16 = 5;
pub const MAX_MAX_FRAME_RATE: u16 = 120;
pub const DEFAULT_AGENT_BUSY_AUTO_GO_MINUTES: u16 = 5;
pub const MIN_AGENT_BUSY_AUTO_GO_MINUTES: u16 = 1;
pub const MAX_AGENT_BUSY_AUTO_GO_MINUTES: u16 = 120;
pub const DEFAULT_POMODORO_WORK_MINUTES: u16 = 25;
pub const DEFAULT_POMODORO_REST_MINUTES: u16 = 5;
pub const DEFAULT_POMODORO_WARNING_REMAINING_PERCENT: u8 = 20;
pub const MIN_POMODORO_MINUTES: u16 = 1;
pub const MAX_POMODORO_MINUTES: u16 = 180;
pub const MIN_POMODORO_WARNING_REMAINING_PERCENT: u8 = 1;
pub const MAX_POMODORO_WARNING_REMAINING_PERCENT: u8 = 99;

/// Cache key for Codex transcript aborted-turn checks.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TranscriptAbortCacheKey {
    /// Transcript file path from the status hook or Codex session lookup.
    path: PathBuf,
    /// Turn id whose aborted marker is being searched.
    turn_id: String,
    /// File length used to invalidate append-only transcript changes.
    len: u64,
    /// File modification time used to invalidate rewritten transcript changes.
    modified: Option<SystemTime>,
}

/// Shared cache for expensive transcript scans.
static TRANSCRIPT_ABORT_CACHE: OnceLock<Mutex<BTreeMap<TranscriptAbortCacheKey, bool>>> =
    OnceLock::new();
static STORE_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceActivity {
    Busy,
    Idle,
    Unknown,
}

fn default_workspace_activity() -> WorkspaceActivity {
    WorkspaceActivity::Unknown
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CenterMode {
    Agent,
    Terminal,
    Editor,
    Preview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Workspace,
    Reviewer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewerMode {
    Gsd,
    Git,
}

#[derive(Debug, Clone)]
pub struct WorkspaceViewData {
    pub name: String,
    pub path: PathBuf,
    pub agent_kind: AgentKind,
    /// Per-workspace main agent model override.
    pub agent_model: Option<String>,
    /// Per-workspace main agent effort override.
    pub agent_effort: Option<String>,
    /// Per-workspace Codex fast-mode override.
    pub agent_fast_mode: Option<bool>,
    /// Per-workspace main agent working directory override.
    pub agent_work_dir: Option<PathBuf>,
    pub agent_id: String,
    pub session_id: Option<String>,
    pub activity: WorkspaceActivity,
    /// Extra agent sessions that belong to this workspace.
    pub subagents: Vec<SubagentViewData>,
    pub center_mode: CenterMode,
    pub previous_center_mode: CenterMode,
    pub route: Route,
    pub reviewer_mode: ReviewerMode,
    pub selected_file: Option<PathBuf>,
    pub outline: Vec<OutlineNode>,
    /// Workspace-local Markdown favorites shown by the outline filter.
    pub outline_favorites: BTreeSet<PathBuf>,
    /// Extra absolute directories displayed beside workspace and home roots.
    pub attached_outline_dirs: Vec<PathBuf>,
    /// 当前 workspace 内按 app 访问时间排序的 Markdown 文件。
    pub recent_markdowns: Vec<RecentMarkdownEntry>,
    /// Whether the Markdown-local outline is hidden for this workspace.
    pub markdown_outline_collapsed: bool,
    /// Workspace-scoped freeform memo shown in the notification drawer.
    pub memo: String,
}

/// 单个 workspace 内部的 Markdown 最近访问记录。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RecentMarkdownEntry {
    /// outline tree 使用的路径。
    pub path: PathBuf,
    /// Unix epoch 毫秒级访问时间。
    pub edited_at_ms: u64,
}

/// Runtime and persisted metadata for one workspace subagent.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SubagentViewData {
    /// Stable id used for host routing and hook correlation.
    pub id: String,
    /// Human-readable tab label.
    pub name: String,
    /// Agent implementation used by this subagent.
    #[serde(default)]
    pub agent_kind: AgentKind,
    /// Per-subagent model override passed to the agent CLI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_model: Option<String>,
    /// Per-subagent effort override passed to the agent CLI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_effort: Option<String>,
    /// Per-subagent Codex fast-mode override passed as service_tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_fast_mode: Option<bool>,
    /// Per-subagent working directory override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_work_dir: Option<PathBuf>,
    /// Stable hook id used to route status/session events.
    pub agent_id: String,
    /// Runtime session id used for resume.
    pub session_id: Option<String>,
    /// Last known activity from the shared agent status file.
    #[serde(skip, default = "default_workspace_activity")]
    pub activity: WorkspaceActivity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentWorkspaceStatus {
    pub activity: WorkspaceActivity,
    pub session_id: Option<String>,
}

/// Status indexes loaded from the shared agent status file.
struct AgentStatusIndexes {
    /// Latest status keyed by agent kind and workspace path.
    by_path: BTreeMap<AgentKind, BTreeMap<PathBuf, AgentWorkspaceStatus>>,
    /// Latest status keyed by agent kind and persisted agent id.
    by_id: BTreeMap<AgentKind, BTreeMap<String, AgentWorkspaceStatus>>,
    /// Status keyed by agent kind and runtime session id.
    by_session: BTreeMap<AgentKind, BTreeMap<String, AgentWorkspaceStatus>>,
}

#[derive(Debug, Clone)]
pub enum OutlineRootKind {
    /// Current workspace root.
    Workspace,
    /// Built-in home config root.
    Home,
    /// User-attached external directory.
    Attached,
}

#[derive(Debug, Clone)]
pub enum OutlineNode {
    Root {
        root_kind: OutlineRootKind,
        key: PathBuf,
        label: String,
        expanded: bool,
        children: Vec<OutlineNode>,
    },
    Dir {
        key: PathBuf,
        label: String,
        expanded: bool,
        children: Vec<OutlineNode>,
    },
    File {
        path: PathBuf,
        label: String,
    },
}

#[derive(Debug, Clone)]
pub struct InitialGuiData {
    pub active_workspace: usize,
    pub workspaces: Vec<WorkspaceViewData>,
    /// Whether the workspace rail should start in compact mode.
    pub rail_collapsed: bool,
}

/// 应用界面语言，适用于全局 GUI 文案切换。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppLanguage {
    /// 英文界面，也是首次启动和旧配置的默认值。
    #[default]
    English,
    /// 简体中文界面。
    Chinese,
    /// 日文界面。
    Japanese,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct NetworkSettings {
    /// Primary proxy URL used for child processes.
    #[serde(default)]
    pub proxy: String,
    /// Additional no_proxy entries appended after gsdv built-ins.
    #[serde(default)]
    pub no_proxy: String,
    /// Whether to emit both HTTP(S) and SOCKS proxy env variants.
    #[serde(default)]
    pub mirror_proxy_protocol: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RuntimeSettings {
    /// Maximum application-scheduled repaint rate in frames per second.
    #[serde(default = "default_max_frame_rate")]
    pub max_frame_rate: u16,
    /// Agent Busy 且无输出时自动发送 go 的等待分钟数。
    #[serde(default = "default_agent_busy_auto_go_minutes")]
    pub agent_busy_auto_go_minutes: u16,
    /// Whether the global pomodoro timer is enabled.
    #[serde(default = "default_pomodoro_enabled")]
    pub pomodoro_enabled: bool,
    /// Pomodoro focused work duration in minutes.
    #[serde(default = "default_pomodoro_work_minutes")]
    pub pomodoro_work_minutes: u16,
    /// Pomodoro rest duration in minutes.
    #[serde(default = "default_pomodoro_rest_minutes")]
    pub pomodoro_rest_minutes: u16,
    /// 工作剩余低于该百分比时显示哈基米预警。
    #[serde(default = "default_pomodoro_warning_remaining_percent")]
    pub pomodoro_warning_remaining_percent: u8,
    /// User-defined Agent quick replies, one command per line.
    #[serde(default)]
    pub agent_custom_quick_replies: String,
    /// Whether Agent input translation should start after the draft is idle.
    #[serde(default)]
    pub agent_input_translation_auto_trigger: bool,
    /// Whether Codex Responses may fall back to HTTP after repeated WebSocket failures.
    #[serde(default)]
    pub codex_responses_http_fallback_enabled: bool,
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            max_frame_rate: default_max_frame_rate(),
            agent_busy_auto_go_minutes: default_agent_busy_auto_go_minutes(),
            pomodoro_enabled: default_pomodoro_enabled(),
            pomodoro_work_minutes: default_pomodoro_work_minutes(),
            pomodoro_rest_minutes: default_pomodoro_rest_minutes(),
            pomodoro_warning_remaining_percent: default_pomodoro_warning_remaining_percent(),
            agent_custom_quick_replies: String::new(),
            agent_input_translation_auto_trigger: false,
            codex_responses_http_fallback_enabled: false,
        }
    }
}

/// Returns the default upper bound for application-driven repaint requests.
fn default_max_frame_rate() -> u16 {
    DEFAULT_MAX_FRAME_RATE
}

/// 返回 Agent Busy 无输出自动继续的默认等待分钟数。
fn default_agent_busy_auto_go_minutes() -> u16 {
    DEFAULT_AGENT_BUSY_AUTO_GO_MINUTES
}

/// Returns whether the pomodoro timer is enabled for first-run settings.
fn default_pomodoro_enabled() -> bool {
    true
}

/// Returns the default focused work length for a pomodoro cycle.
fn default_pomodoro_work_minutes() -> u16 {
    DEFAULT_POMODORO_WORK_MINUTES
}

/// Returns the default rest length for a pomodoro cycle.
fn default_pomodoro_rest_minutes() -> u16 {
    DEFAULT_POMODORO_REST_MINUTES
}

/// 返回哈基米工作末段预警的默认剩余百分比。
fn default_pomodoro_warning_remaining_percent() -> u8 {
    DEFAULT_POMODORO_WARNING_REMAINING_PERCENT
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FontFamilySetting {
    /// Inherits the global default font chain for a surface.
    Default,
    Monospace,
    Proportional,
    System,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct FontSurfaceSettings {
    #[serde(default = "default_font_family_setting")]
    pub family: FontFamilySetting,
    #[serde(default = "default_font_size")]
    pub size: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_path: Option<String>,
    /// 主字体缺字时使用的第二系统字体。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_system_name: Option<String>,
    /// 第二系统字体路径，注册在主字体之后。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_system_path: Option<String>,
}

impl Default for FontSurfaceSettings {
    fn default() -> Self {
        Self {
            family: default_font_family_setting(),
            size: default_font_size(),
            system_name: None,
            system_path: None,
            fallback_system_name: None,
            fallback_system_path: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct FontSettings {
    /// Global font chain used when a surface chooses the default.
    #[serde(default = "default_global_font_surface_settings")]
    pub default_fonts: FontSurfaceSettings,
    /// Agent terminal font selection, or inherited default.
    #[serde(default = "default_inherited_font_surface_settings")]
    pub agent: FontSurfaceSettings,
    /// Workspace terminal font selection, or inherited default.
    #[serde(default = "default_inherited_font_surface_settings")]
    pub terminal: FontSurfaceSettings,
    /// Markdown editor font selection, or inherited default.
    #[serde(default = "default_inherited_font_surface_settings")]
    pub editor: FontSurfaceSettings,
}

impl Default for FontSettings {
    fn default() -> Self {
        Self {
            default_fonts: default_global_font_surface_settings(),
            agent: default_inherited_font_surface_settings(),
            terminal: default_inherited_font_surface_settings(),
            editor: default_inherited_font_surface_settings(),
        }
    }
}

/// Builds the global concrete font selection used by inherited surfaces.
fn default_global_font_surface_settings() -> FontSurfaceSettings {
    FontSurfaceSettings {
        family: FontFamilySetting::Monospace,
        ..FontSurfaceSettings::default()
    }
}

/// Builds a surface selection that inherits the global font chain.
fn default_inherited_font_surface_settings() -> FontSurfaceSettings {
    FontSurfaceSettings {
        family: FontFamilySetting::Default,
        ..FontSurfaceSettings::default()
    }
}

fn default_font_family_setting() -> FontFamilySetting {
    FontFamilySetting::Monospace
}

fn default_font_size() -> f32 {
    14.0
}

impl NetworkSettings {
    /// Returns no_proxy entries with gsdv built-ins first.
    pub fn effective_no_proxy_entries(&self) -> Vec<String> {
        let mut entries = BUILTIN_NO_PROXY
            .iter()
            .map(|entry| (*entry).to_string())
            .collect::<Vec<_>>();
        for entry in split_no_proxy_entries(&self.no_proxy) {
            if !entries.iter().any(|existing| existing == &entry) {
                entries.push(entry);
            }
        }
        entries
    }

    /// Returns the comma-joined no_proxy value for child processes.
    pub fn effective_no_proxy(&self) -> String {
        self.effective_no_proxy_entries().join(",")
    }

    /// Builds proxy-related environment variables for terminal-backed processes.
    pub fn env_vars(&self) -> Vec<(String, String)> {
        let mut env = Vec::new();
        let proxy = self.proxy.trim();
        if !proxy.is_empty() {
            if proxy_is_socks5(proxy) {
                env.push(("all_proxy".to_string(), proxy.to_string()));
                if self.mirror_proxy_protocol {
                    if let Some(http_proxy) = mirrored_http_proxy(proxy) {
                        env.push(("http_proxy".to_string(), http_proxy.clone()));
                        env.push(("https_proxy".to_string(), http_proxy));
                    }
                }
            } else {
                env.push(("http_proxy".to_string(), proxy.to_string()));
                env.push(("https_proxy".to_string(), proxy.to_string()));
                if self.mirror_proxy_protocol {
                    if let Some(socks_proxy) = mirrored_socks5_proxy(proxy) {
                        env.push(("all_proxy".to_string(), socks_proxy));
                    }
                }
            }
        }
        env.push(("no_proxy".to_string(), self.effective_no_proxy()));
        env
    }
}

/// Detects socks5 proxy URLs before building process environment.
fn proxy_is_socks5(proxy: &str) -> bool {
    proxy.to_ascii_lowercase().starts_with("socks5")
}

/// Converts a SOCKS proxy URL into its HTTP env sibling.
fn mirrored_http_proxy(proxy: &str) -> Option<String> {
    replace_proxy_scheme(proxy, &["socks5h://", "socks5://"], "http://")
}

/// Converts an HTTP(S) proxy URL into its SOCKS env sibling.
fn mirrored_socks5_proxy(proxy: &str) -> Option<String> {
    replace_proxy_scheme(proxy, &["https://", "http://"], "socks5://")
}

/// Replaces a known URL scheme without guessing bare proxy values.
fn replace_proxy_scheme(proxy: &str, schemes: &[&str], replacement: &str) -> Option<String> {
    let lower = proxy.to_ascii_lowercase();
    for scheme in schemes {
        if lower.starts_with(scheme) {
            return Some(format!("{replacement}{}", &proxy[scheme.len()..]));
        }
    }
    None
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct StoreFile {
    active: Option<usize>,
    #[serde(default)]
    rail_collapsed: bool,
    #[serde(default)]
    default_agent_kind: Option<AgentKind>,
    #[serde(default)]
    theme_mode: Option<ThemeMode>,
    #[serde(default)]
    language: AppLanguage,
    #[serde(default)]
    outline_global_favorites: BTreeSet<PathBuf>,
    #[serde(default)]
    network_settings: NetworkSettings,
    #[serde(default)]
    runtime_settings: RuntimeSettings,
    #[serde(default)]
    font_settings: FontSettings,
    #[serde(default)]
    workspaces: Vec<StoredWorkspace>,
}

#[derive(Debug, Deserialize, Serialize)]
struct StoredWorkspace {
    path: String,
    #[serde(default)]
    agent_kind: Option<AgentKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_fast_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_work_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(default, skip_serializing)]
    selected_file: Option<String>,
    #[serde(
        default,
        skip_serializing,
        deserialize_with = "deserialize_center_mode_option"
    )]
    center_mode: Option<CenterMode>,
    #[serde(default, skip_serializing)]
    reviewer_mode: Option<ReviewerMode>,
    #[serde(default = "default_true")]
    markdown_outline_collapsed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    attached_outline_dirs: Vec<PathBuf>,
}

/// 读取历史 center mode，适用于旧 store 里还残留 recent 的场景。
fn deserialize_center_mode_option<'de, D>(deserializer: D) -> Result<Option<CenterMode>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };
    match value.as_str() {
        "agent" => Ok(Some(CenterMode::Agent)),
        "terminal" => Ok(Some(CenterMode::Terminal)),
        "editor" => Ok(Some(CenterMode::Editor)),
        "preview" => Ok(Some(CenterMode::Preview)),
        "recent" => Ok(Some(CenterMode::Agent)),
        _ => Err(serde::de::Error::unknown_variant(
            &value,
            &["agent", "terminal", "editor", "preview"],
        )),
    }
}

/// Default-on boolean used for UI panels that should start collapsed.
fn default_true() -> bool {
    true
}

#[derive(Debug, Default, Deserialize)]
struct AgentStatusFile {
    #[serde(default)]
    agents: BTreeMap<String, AgentStatusEntry>,
    #[serde(default)]
    sessions: BTreeMap<String, AgentStatusEntry>,
}

#[derive(Debug, Default, Deserialize)]
struct AgentStatusEntry {
    #[serde(default)]
    status: String,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    workspace: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    turn_id: Option<String>,
    #[serde(default)]
    transcript_path: Option<String>,
    #[serde(default)]
    started_at_ms: Option<u64>,
    #[serde(default)]
    updated_at_ms: Option<u64>,
}

/// 与主 workspace store 分离保存的 subagent sidecar 数据。
#[derive(Debug, Default, Deserialize, Serialize)]
struct SubagentFile {
    /// Subagents that belong to one workspace.
    #[serde(default)]
    subagents: Vec<SubagentViewData>,
}

/// Sidecar payload for workspace-local outline favorites.
#[derive(Debug, Default, Deserialize, Serialize)]
struct OutlineFavoritesFile {
    /// Workspace-relative Markdown paths favorited in the outline.
    #[serde(default)]
    favorites: BTreeSet<PathBuf>,
}

pub fn load_initial_gui_data(default_agent_kind: AgentKind) -> InitialGuiData {
    let mut stored = load_store();
    prune_agent_statuses_to_store_sessions(&stored);
    prune_store_sessions_to_status_file(&mut stored);
    let statuses = load_agent_status_indexes();
    let workspaces = stored
        .workspaces
        .into_iter()
        .filter_map(|workspace| {
            let path = PathBuf::from(&workspace.path);
            if path.is_dir() {
                Some(build_workspace(
                    path,
                    workspace,
                    &statuses.by_session,
                    &statuses.by_id,
                    &statuses.by_path,
                    default_agent_kind,
                ))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let active_workspace = stored
        .active
        .filter(|index| *index < workspaces.len())
        .unwrap_or(0);

    InitialGuiData {
        active_workspace,
        workspaces,
        rail_collapsed: stored.rail_collapsed,
    }
}

pub fn load_default_agent_kind() -> Option<AgentKind> {
    load_store().default_agent_kind
}

pub fn load_theme_mode() -> Option<ThemeMode> {
    load_store().theme_mode
}

pub fn load_network_settings() -> NetworkSettings {
    load_store().network_settings
}

/// 读取持久化界面语言，缺省使用英文。
pub fn load_app_language() -> AppLanguage {
    load_store().language
}

/// 读取全局 outline 收藏，适用于 Home Root 下的 Markdown。
pub fn load_global_outline_favorites() -> BTreeSet<PathBuf> {
    load_store().outline_global_favorites
}

/// Loads runtime behavior knobs that affect the app shell itself.
pub fn load_runtime_settings() -> RuntimeSettings {
    sanitize_runtime_settings(load_store().runtime_settings)
}

pub fn load_font_settings() -> FontSettings {
    load_store().font_settings
}

/// Returns whether the store already contains an explicit font configuration.
pub fn has_saved_font_settings() -> bool {
    let Some(path) = gsdv_store_path() else {
        return false;
    };
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(&content)
        .ok()
        .and_then(|value| value.get("font_settings").cloned())
        .is_some()
}

/// Loads the workspace memo from its standalone Markdown file.
pub fn load_workspace_memo(workspace_path: &Path) -> String {
    workspace_memo_path(workspace_path)
        .and_then(|path| fs::read_to_string(path).ok())
        .unwrap_or_default()
}

/// Saves the workspace memo outside the JSON store so body text stays file-like.
pub fn save_workspace_memo(workspace_path: &Path, memo: &str) -> Result<(), String> {
    let Some(path) = workspace_memo_path(workspace_path) else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(path, memo).map_err(|error| error.to_string())
}

/// Deletes the standalone workspace memo file when a workspace is removed.
pub fn delete_workspace_memo(workspace_path: &Path) -> Result<(), String> {
    let Some(path) = workspace_memo_path(workspace_path) else {
        return Ok(());
    };
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

/// 读取单个 workspace 的 app 内部最近访问 Markdown 列表。
pub fn load_workspace_recent_markdowns(workspace_path: &Path) -> Vec<RecentMarkdownEntry> {
    let Some(path) = workspace_recent_markdowns_path(workspace_path) else {
        return Vec::new();
    };
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<RecentMarkdownEntry>>(&content).unwrap_or_default()
}

/// 保存单个 workspace 的 app 内部最近访问 Markdown 列表。
pub fn save_workspace_recent_markdowns(
    workspace_path: &Path,
    recent: &[RecentMarkdownEntry],
) -> Result<(), String> {
    let Some(path) = workspace_recent_markdowns_path(workspace_path) else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let content = serde_json::to_string_pretty(recent).map_err(|error| error.to_string())?;
    fs::write(path, content).map_err(|error| error.to_string())
}

/// 删除 workspace 时同步删除最近访问 Markdown sidecar。
pub fn delete_workspace_recent_markdowns(workspace_path: &Path) -> Result<(), String> {
    let Some(path) = workspace_recent_markdowns_path(workspace_path) else {
        return Ok(());
    };
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

/// Deletes saved Markdown diff history when a workspace is removed.
pub fn delete_workspace_markdown_diffs(workspace_path: &Path) -> Result<(), String> {
    let Some(path) = workspace_markdown_diffs_dir(workspace_path) else {
        return Ok(());
    };
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

/// Loads the workspace terminal history snapshot.
pub fn load_workspace_terminal_history(workspace_path: &Path) -> String {
    workspace_terminal_history_path(workspace_path)
        .and_then(|path| fs::read_to_string(path).ok())
        .unwrap_or_default()
}

/// Saves the workspace terminal history snapshot outside the JSON store.
pub fn save_workspace_terminal_history(workspace_path: &Path, history: &str) -> Result<(), String> {
    let Some(path) = workspace_terminal_history_path(workspace_path) else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(path, history).map_err(|error| error.to_string())
}

/// Deletes the workspace terminal history snapshot when a workspace is removed.
pub fn delete_workspace_terminal_history(workspace_path: &Path) -> Result<(), String> {
    let Some(path) = workspace_terminal_history_path(workspace_path) else {
        return Ok(());
    };
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

/// Deletes the workspace subagent sidecar when a workspace is removed.
pub fn delete_workspace_subagents(workspace_path: &Path) -> Result<(), String> {
    let Some(path) = workspace_subagents_path(workspace_path) else {
        return Ok(());
    };
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

/// 返回所有 workspace 粒度数据的根目录。
pub fn workspaces_store_root() -> Option<PathBuf> {
    let store_path = gsdv_store_path()?;
    let root = store_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Some(root.join("workspaces"))
}

/// 返回 workspace 路径对应的稳定存储目录名。
pub fn workspace_store_key(workspace_path: &Path) -> String {
    stable_path_hash(workspace_path)
}

/// 返回单个 workspace 的粒度数据目录。
pub fn workspace_store_dir(workspace_path: &Path) -> Option<PathBuf> {
    Some(workspaces_store_root()?.join(workspace_store_key(workspace_path)))
}

/// 确保单个 workspace 粒度数据目录存在并返回路径。
pub fn ensure_workspace_store_dir(workspace_path: &Path) -> Option<PathBuf> {
    let path = workspace_store_dir(workspace_path)?;
    let _ = fs::create_dir_all(&path);
    Some(path)
}

/// Returns the sidecar file used to persist workspace subagents.
pub fn workspace_subagents_path(workspace_path: &Path) -> Option<PathBuf> {
    Some(workspace_store_dir(workspace_path)?.join("subagents.json"))
}

/// Returns the sidecar file used to persist workspace outline favorites.
pub fn workspace_outline_favorites_path(workspace_path: &Path) -> Option<PathBuf> {
    Some(workspace_store_dir(workspace_path)?.join("outline-favorites.json"))
}

/// 返回 workspace 最近访问 Markdown sidecar 文件。
pub fn workspace_recent_markdowns_path(workspace_path: &Path) -> Option<PathBuf> {
    Some(workspace_store_dir(workspace_path)?.join("recent-markdowns.json"))
}

/// Returns the workspace-local directory for saved Markdown diffs.
pub fn workspace_markdown_diffs_dir(workspace_path: &Path) -> Option<PathBuf> {
    Some(workspace_store_dir(workspace_path)?.join("markdown-diffs"))
}

/// Returns a stable hash for path-keyed workspace sidecar filenames.
pub fn stable_path_hash(path: &Path) -> String {
    format!("{:x}", stable_state_key(path))
}

pub fn save_default_agent_kind(agent_kind: AgentKind) {
    let _guard = store_write_guard();
    let Some(path) = gsdv_store_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut store = load_store();
    store.default_agent_kind = Some(agent_kind);
    if let Ok(content) = serde_json::to_string_pretty(&store) {
        let _ = fs::write(path, content);
    }
}

pub fn save_theme_mode(theme_mode: ThemeMode) {
    let _guard = store_write_guard();
    let Some(path) = gsdv_store_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut store = load_store();
    store.theme_mode = Some(theme_mode);
    if let Ok(content) = serde_json::to_string_pretty(&store) {
        let _ = fs::write(path, content);
    }
}

/// 保存所有 workspace 共享的界面语言。
pub fn save_app_language(language: AppLanguage) {
    let _guard = store_write_guard();
    let Some(path) = gsdv_store_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut store = load_store();
    store.language = language;
    if let Ok(content) = serde_json::to_string_pretty(&store) {
        let _ = fs::write(path, content);
    }
}

/// 保存全局 outline 收藏，适用于 Home Root 下的 Markdown。
pub fn save_global_outline_favorites(favorites: &BTreeSet<PathBuf>) {
    let _guard = store_write_guard();
    let Some(path) = gsdv_store_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut store = load_store();
    store.outline_global_favorites = favorites.clone();
    if let Ok(content) = serde_json::to_string_pretty(&store) {
        let _ = fs::write(path, content);
    }
}

pub fn save_network_settings(network_settings: &NetworkSettings) {
    let _guard = store_write_guard();
    let Some(path) = gsdv_store_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut store = load_store();
    store.network_settings = network_settings.clone();
    if let Ok(content) = serde_json::to_string_pretty(&store) {
        let _ = fs::write(path, content);
    }
}

/// Saves runtime behavior knobs shared across all workspaces.
pub fn save_runtime_settings(runtime_settings: &RuntimeSettings) {
    let _guard = store_write_guard();
    let Some(path) = gsdv_store_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut store = load_store();
    store.runtime_settings = sanitize_runtime_settings(runtime_settings.clone());
    if let Ok(content) = serde_json::to_string_pretty(&store) {
        let _ = fs::write(path, content);
    }
}

pub fn save_font_settings(font_settings: &FontSettings) {
    let _guard = store_write_guard();
    let Some(path) = gsdv_store_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut store = load_store();
    store.font_settings = font_settings.clone();
    if let Ok(content) = serde_json::to_string_pretty(&store) {
        let _ = fs::write(path, content);
    }
}

/// Clamps loaded runtime settings so old or edited stores cannot disable UI wakeups.
fn sanitize_runtime_settings(mut settings: RuntimeSettings) -> RuntimeSettings {
    settings.max_frame_rate = settings
        .max_frame_rate
        .clamp(MIN_MAX_FRAME_RATE, MAX_MAX_FRAME_RATE);
    settings.agent_busy_auto_go_minutes = settings.agent_busy_auto_go_minutes.clamp(
        MIN_AGENT_BUSY_AUTO_GO_MINUTES,
        MAX_AGENT_BUSY_AUTO_GO_MINUTES,
    );
    settings.pomodoro_work_minutes = settings
        .pomodoro_work_minutes
        .clamp(MIN_POMODORO_MINUTES, MAX_POMODORO_MINUTES);
    settings.pomodoro_rest_minutes = settings
        .pomodoro_rest_minutes
        .clamp(MIN_POMODORO_MINUTES, MAX_POMODORO_MINUTES);
    settings.pomodoro_warning_remaining_percent =
        settings.pomodoro_warning_remaining_percent.clamp(
            MIN_POMODORO_WARNING_REMAINING_PERCENT,
            MAX_POMODORO_WARNING_REMAINING_PERCENT,
        );
    settings.agent_custom_quick_replies =
        sanitize_agent_custom_quick_replies(&settings.agent_custom_quick_replies);
    settings
}

/// Trims custom quick replies while preserving line-based editing semantics.
fn sanitize_agent_custom_quick_replies(value: &str) -> String {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn refresh_workspace_outline(workspace: &mut WorkspaceViewData) {
    let selected = workspace.selected_file.clone();
    let expanded = expanded_outline_dirs(&workspace.outline);
    let collapsed = collapsed_outline_dirs(&workspace.outline);
    workspace.outline = build_outline_with_state(
        &workspace.path,
        &workspace.attached_outline_dirs,
        selected.as_deref(),
        expanded,
        collapsed,
    );
    if workspace.selected_file.is_none() {
        workspace.selected_file = first_markdown_file(&workspace.outline);
    }
}

pub fn refresh_workspace_agent_statuses(workspaces: &mut [WorkspaceViewData]) -> bool {
    let statuses = load_agent_status_indexes();
    let mut changed = false;
    for workspace in workspaces {
        let status_by_session = workspace.session_id.as_deref().and_then(|session_id| {
            statuses
                .by_session
                .get(&workspace.agent_kind)
                .and_then(|statuses| statuses.get(session_id))
        });
        let status_by_id = statuses
            .by_id
            .get(&workspace.agent_kind)
            .and_then(|statuses| statuses.get(&workspace.agent_id));
        let status_by_path = statuses
            .by_path
            .get(&workspace.agent_kind)
            .and_then(|statuses| statuses.get(&normalize_path(&workspace.path)));
        let Some(status) = status_by_id.or(status_by_session).or(status_by_path) else {
            continue;
        };
        if workspace.activity != status.activity {
            workspace.activity = status.activity;
            changed = true;
        }
        let status_session_id = status.session_id.as_ref().cloned();
        if status_session_id.is_some() && workspace.session_id != status_session_id {
            workspace.session_id = status_session_id;
            changed = true;
        }
        if refresh_subagent_statuses(
            &mut workspace.subagents,
            &statuses.by_id,
            &statuses.by_session,
        ) {
            changed = true;
        }
    }
    changed
}

/// Refreshes subagent activity/session data from hook status indexes.
fn refresh_subagent_statuses(
    subagents: &mut [SubagentViewData],
    statuses_by_id: &BTreeMap<AgentKind, BTreeMap<String, AgentWorkspaceStatus>>,
    statuses_by_session: &BTreeMap<AgentKind, BTreeMap<String, AgentWorkspaceStatus>>,
) -> bool {
    let mut changed = false;
    for subagent in subagents {
        let status_by_id = statuses_by_id
            .get(&subagent.agent_kind)
            .and_then(|statuses| statuses.get(&subagent.agent_id));
        let status_by_session = subagent.session_id.as_deref().and_then(|session_id| {
            statuses_by_session
                .get(&subagent.agent_kind)
                .and_then(|statuses| statuses.get(session_id))
        });
        let Some(status) = status_by_id.or(status_by_session) else {
            continue;
        };
        if subagent.activity != status.activity {
            subagent.activity = status.activity;
            changed = true;
        }
        if status.session_id.is_some() && subagent.session_id != status.session_id {
            subagent.session_id = status.session_id.clone();
            changed = true;
        }
    }
    changed
}

pub fn new_workspace(path: PathBuf, agent_kind: AgentKind) -> WorkspaceViewData {
    let statuses = load_agent_status_indexes();
    build_workspace(
        path.clone(),
        StoredWorkspace {
            path: path.to_string_lossy().to_string(),
            agent_kind: Some(agent_kind),
            agent_model: None,
            agent_effort: None,
            agent_fast_mode: None,
            agent_work_dir: None,
            agent_id: None,
            session_id: None,
            selected_file: None,
            center_mode: None,
            reviewer_mode: None,
            markdown_outline_collapsed: true,
            attached_outline_dirs: Vec::new(),
        },
        &statuses.by_session,
        &statuses.by_id,
        &statuses.by_path,
        agent_kind,
    )
}

pub fn save_workspace_store(workspaces: &[WorkspaceViewData], active: usize, rail_collapsed: bool) {
    let _guard = store_write_guard();
    let Some(path) = gsdv_store_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let existing_store = load_store();
    let store = StoreFile {
        active: (!workspaces.is_empty()).then_some(active.min(workspaces.len().saturating_sub(1))),
        rail_collapsed,
        default_agent_kind: existing_store.default_agent_kind,
        theme_mode: existing_store.theme_mode,
        language: existing_store.language,
        outline_global_favorites: existing_store.outline_global_favorites,
        network_settings: existing_store.network_settings,
        runtime_settings: existing_store.runtime_settings,
        font_settings: existing_store.font_settings,
        workspaces: workspaces
            .iter()
            .map(|workspace| StoredWorkspace {
                path: workspace.path.to_string_lossy().to_string(),
                agent_kind: Some(workspace.agent_kind),
                agent_model: workspace.agent_model.clone(),
                agent_effort: workspace.agent_effort.clone(),
                agent_fast_mode: normalize_stored_agent_fast_mode(
                    workspace.agent_kind,
                    workspace.agent_fast_mode,
                ),
                agent_work_dir: normalize_stored_agent_work_dir(workspace.agent_work_dir.clone()),
                agent_id: Some(workspace.agent_id.clone()),
                session_id: workspace.session_id.clone(),
                selected_file: workspace
                    .selected_file
                    .as_ref()
                    .map(|path| path.to_string_lossy().to_string()),
                center_mode: Some(workspace.center_mode),
                reviewer_mode: Some(workspace.reviewer_mode),
                markdown_outline_collapsed: workspace.markdown_outline_collapsed,
                attached_outline_dirs: workspace.attached_outline_dirs.clone(),
            })
            .collect(),
    };
    if let Ok(content) = serde_json::to_string_pretty(&store) {
        let _ = fs::write(path, content);
    }
    for workspace in workspaces {
        save_workspace_subagents(workspace);
        save_workspace_outline_favorites(&workspace.path, &workspace.outline_favorites);
        let _ = save_workspace_recent_markdowns(&workspace.path, &workspace.recent_markdowns);
    }
}

/// 将 subagent 数据保存到 workspace 独立 sidecar 文件。
pub fn save_workspace_subagents(workspace: &WorkspaceViewData) {
    let Some(path) = workspace_subagents_path(&workspace.path) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let file = SubagentFile {
        subagents: workspace.subagents.clone(),
    };
    if let Ok(content) = serde_json::to_string_pretty(&file) {
        let _ = fs::write(path, content);
    }
}

/// 返回清理后的 agent model 覆盖值。
fn normalize_stored_agent_model(model: Option<String>) -> Option<String> {
    model
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// 返回清理后的 agent effort 覆盖值。
fn normalize_stored_agent_effort(kind: AgentKind, effort: Option<String>) -> Option<String> {
    effort
        .map(|value| value.trim().to_string())
        .filter(|value| kind.supports_effort(value))
}

/// 返回当前 agent 类型支持的 fast-mode 覆盖值。
fn normalize_stored_agent_fast_mode(kind: AgentKind, fast_mode: Option<bool>) -> Option<bool> {
    kind.supports_fast_mode().then_some(fast_mode).flatten()
}

pub fn normalize_stored_agent_work_dir(work_dir: Option<PathBuf>) -> Option<PathBuf> {
    work_dir
        .map(|path| normalize_path(&path))
        .filter(|path| path.is_dir())
}

/// 保存 workspace 级 outline 收藏。
pub fn save_workspace_outline_favorites(workspace_path: &Path, favorites: &BTreeSet<PathBuf>) {
    let Some(path) = workspace_outline_favorites_path(workspace_path) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let file = OutlineFavoritesFile {
        favorites: favorites.clone(),
    };
    if let Ok(content) = serde_json::to_string_pretty(&file) {
        let _ = fs::write(path, content);
    }
}

/// Deletes the workspace outline favorites sidecar when a workspace is removed.
pub fn delete_workspace_outline_favorites(workspace_path: &Path) -> Result<(), String> {
    let Some(path) = workspace_outline_favorites_path(workspace_path) else {
        return Ok(());
    };
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

/// Creates a new subagent using a stable id under the workspace.
pub fn new_subagent(
    workspace_path: &Path,
    name: String,
    agent_kind: AgentKind,
    agent_model: Option<String>,
    agent_effort: Option<String>,
    agent_fast_mode: Option<bool>,
    agent_work_dir: Option<PathBuf>,
    session_id: Option<String>,
) -> SubagentViewData {
    let id = format!("sub-{}-{:x}", now_ms(), stable_state_key(Path::new(&name)));
    SubagentViewData {
        agent_id: subagent_agent_id(workspace_path, &id),
        id,
        name,
        agent_kind,
        agent_model,
        agent_effort,
        agent_fast_mode: normalize_stored_agent_fast_mode(agent_kind, agent_fast_mode),
        agent_work_dir: normalize_stored_agent_work_dir(agent_work_dir),
        session_id,
        activity: WorkspaceActivity::Unknown,
    }
}

/// Loads subagents persisted in the workspace sidecar.
fn load_workspace_subagents(
    workspace_path: &Path,
    default_agent_kind: AgentKind,
) -> Vec<SubagentViewData> {
    let Some(path) = workspace_subagents_path(workspace_path) else {
        return Vec::new();
    };
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(file) = serde_json::from_str::<SubagentFile>(&content) else {
        return Vec::new();
    };
    file.subagents
        .into_iter()
        .filter(|subagent| !subagent.id.trim().is_empty() && !subagent.name.trim().is_empty())
        .map(|mut subagent| {
            if subagent.agent_id.trim().is_empty() {
                subagent.agent_id = subagent_agent_id(workspace_path, &subagent.id);
            }
            if !AgentKind::all().contains(&subagent.agent_kind) {
                subagent.agent_kind = default_agent_kind;
            }
            subagent.agent_model = normalize_stored_agent_model(subagent.agent_model);
            subagent.agent_effort =
                normalize_stored_agent_effort(subagent.agent_kind, subagent.agent_effort);
            subagent.agent_fast_mode =
                normalize_stored_agent_fast_mode(subagent.agent_kind, subagent.agent_fast_mode);
            subagent.agent_work_dir = normalize_stored_agent_work_dir(subagent.agent_work_dir);
            subagent.activity = WorkspaceActivity::Unknown;
            subagent
        })
        .collect()
}

/// 读取 workspace 级 outline 收藏。
fn load_workspace_outline_favorites(workspace_path: &Path) -> BTreeSet<PathBuf> {
    let Some(path) = workspace_outline_favorites_path(workspace_path) else {
        return BTreeSet::new();
    };
    let Ok(content) = fs::read_to_string(path) else {
        return BTreeSet::new();
    };
    serde_json::from_str::<OutlineFavoritesFile>(&content)
        .map(|file| file.favorites)
        .unwrap_or_default()
}

fn split_no_proxy_entries(value: &str) -> impl Iterator<Item = String> + '_ {
    value
        .split(|ch: char| ch == ',' || ch.is_whitespace())
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .filter(|entry| !BUILTIN_NO_PROXY.contains(entry))
        .map(str::to_string)
}

fn build_workspace(
    path: PathBuf,
    stored: StoredWorkspace,
    statuses_by_session: &BTreeMap<AgentKind, BTreeMap<String, AgentWorkspaceStatus>>,
    statuses_by_id: &BTreeMap<AgentKind, BTreeMap<String, AgentWorkspaceStatus>>,
    statuses: &BTreeMap<AgentKind, BTreeMap<PathBuf, AgentWorkspaceStatus>>,
    default_agent_kind: AgentKind,
) -> WorkspaceViewData {
    let agent_kind = stored.agent_kind.unwrap_or(default_agent_kind);
    let agent_id = stored.agent_id.unwrap_or_else(|| new_agent_id(&path));
    let stored_selected_file = stored
        .selected_file
        .as_deref()
        .map(PathBuf::from)
        .filter(|selected| selected_file_exists(&path, selected));
    let attached_outline_dirs = normalize_attached_outline_dirs(stored.attached_outline_dirs);
    let mut outline = build_outline(
        &path,
        &attached_outline_dirs,
        stored_selected_file.as_deref(),
    );
    let selected_file = stored_selected_file.or_else(|| first_markdown_file(&outline));
    outline = build_outline(&path, &attached_outline_dirs, selected_file.as_deref());
    let status_by_id = statuses_by_id
        .get(&agent_kind)
        .and_then(|statuses| statuses.get(&agent_id));
    let status_by_session = stored.session_id.as_deref().and_then(|session_id| {
        statuses_by_session
            .get(&agent_kind)
            .and_then(|statuses| statuses.get(session_id))
    });
    let status_by_path = statuses
        .get(&agent_kind)
        .and_then(|statuses| statuses.get(&normalize_path(&path)));
    let status = status_by_id.or(status_by_session).or(status_by_path);
    let activity = status
        .map(|status| status.activity)
        .unwrap_or(WorkspaceActivity::Unknown);
    let session_id = status.and_then(|status| status.session_id.clone());
    let memo = load_workspace_memo(&path);
    let outline_favorites = load_workspace_outline_favorites(&path);
    let recent_markdowns = load_workspace_recent_markdowns(&path);
    let mut subagents = load_workspace_subagents(&path, agent_kind);
    refresh_subagent_statuses(&mut subagents, statuses_by_id, statuses_by_session);

    WorkspaceViewData {
        name: workspace_name(&path),
        path,
        agent_kind,
        agent_model: normalize_stored_agent_model(stored.agent_model),
        agent_effort: normalize_stored_agent_effort(agent_kind, stored.agent_effort),
        agent_fast_mode: normalize_stored_agent_fast_mode(agent_kind, stored.agent_fast_mode),
        agent_work_dir: normalize_stored_agent_work_dir(stored.agent_work_dir),
        agent_id,
        session_id,
        activity,
        subagents,
        center_mode: stored.center_mode.unwrap_or(CenterMode::Agent),
        previous_center_mode: stored.center_mode.unwrap_or(CenterMode::Agent),
        route: Route::Workspace,
        reviewer_mode: stored.reviewer_mode.unwrap_or(ReviewerMode::Git),
        selected_file,
        outline,
        outline_favorites,
        attached_outline_dirs,
        recent_markdowns,
        markdown_outline_collapsed: stored.markdown_outline_collapsed,
        memo,
    }
}

fn selected_file_exists(project_root: &Path, selected: &Path) -> bool {
    if selected.is_absolute() {
        return selected.is_file();
    }
    if let Some(rest) = selected.strip_prefix(HOME_OUTLINE_ROOT).ok()
        && let Some(home) = home_dir()
    {
        return home.join(rest).is_file();
    }
    project_root.join(selected).is_file()
}

fn build_outline(
    project_root: &Path,
    attached_outline_dirs: &[PathBuf],
    selected_file: Option<&Path>,
) -> Vec<OutlineNode> {
    build_outline_with_state(
        project_root,
        attached_outline_dirs,
        selected_file,
        default_expanded_dirs(project_root, attached_outline_dirs),
        HashSet::new(),
    )
}

/// Builds an outline while preserving explicit user tree state.
fn build_outline_with_state(
    project_root: &Path,
    attached_outline_dirs: &[PathBuf],
    selected_file: Option<&Path>,
    expanded: HashSet<PathBuf>,
    collapsed: HashSet<PathBuf>,
) -> Vec<OutlineNode> {
    let mut nodes = Vec::new();
    let workspace_children = build_workspace_children(
        project_root,
        project_root,
        PathBuf::new(),
        &expanded,
        &collapsed,
        selected_file,
    );
    nodes.push(OutlineNode::Root {
        root_kind: OutlineRootKind::Workspace,
        key: PathBuf::new(),
        label: format!("{} (Workspace Root)", workspace_name(project_root)),
        expanded: !collapsed.contains(Path::new("")),
        children: workspace_children,
    });

    if let Some(home) = home_dir() {
        let home_children = build_home_children(&home, selected_file, &expanded, &collapsed);
        if !home_children.is_empty() {
            let home_key = PathBuf::from(HOME_OUTLINE_ROOT);
            nodes.push(OutlineNode::Root {
                root_kind: OutlineRootKind::Home,
                expanded: !collapsed.contains(&home_key),
                key: home_key,
                label: "~ (Home Root)".to_string(),
                children: home_children,
            });
        }
    }

    for dir in attached_outline_dirs {
        if !dir.is_dir() {
            continue;
        }
        let root_expanded = !collapsed.contains(dir);
        let children = if root_expanded {
            build_attached_dir_children(dir, &expanded, &collapsed, selected_file)
        } else {
            Vec::new()
        };
        nodes.push(OutlineNode::Root {
            root_kind: OutlineRootKind::Attached,
            expanded: root_expanded,
            key: dir.clone(),
            label: format!("{} (Attached Root)", attached_outline_label(dir)),
            children,
        });
    }

    nodes
}

/// Returns directory keys that are currently open in the outline tree.
fn expanded_outline_dirs(nodes: &[OutlineNode]) -> HashSet<PathBuf> {
    let mut expanded = HashSet::new();
    collect_expanded_outline_dirs(nodes, &mut expanded);
    expanded
}

/// Recursively collects expanded directory keys for refresh preservation.
fn collect_expanded_outline_dirs(nodes: &[OutlineNode], expanded: &mut HashSet<PathBuf>) {
    for node in nodes {
        match node {
            OutlineNode::Root {
                key,
                expanded: is_expanded,
                children,
                ..
            }
            | OutlineNode::Dir {
                key,
                expanded: is_expanded,
                children,
                ..
            } => {
                if *is_expanded {
                    expanded.insert(key.clone());
                }
                collect_expanded_outline_dirs(children, expanded);
            }
            OutlineNode::File { .. } => {}
        }
    }
}

/// Returns directory keys that are currently closed in the outline tree.
fn collapsed_outline_dirs(nodes: &[OutlineNode]) -> HashSet<PathBuf> {
    let mut collapsed = HashSet::new();
    collect_collapsed_outline_dirs(nodes, &mut collapsed);
    collapsed
}

/// Recursively collects collapsed directory keys for refresh preservation.
fn collect_collapsed_outline_dirs(nodes: &[OutlineNode], collapsed: &mut HashSet<PathBuf>) {
    for node in nodes {
        match node {
            OutlineNode::Root {
                key,
                expanded: is_expanded,
                children,
                ..
            }
            | OutlineNode::Dir {
                key,
                expanded: is_expanded,
                children,
                ..
            } => {
                if !*is_expanded {
                    collapsed.insert(key.clone());
                }
                collect_collapsed_outline_dirs(children, collapsed);
            }
            OutlineNode::File { .. } => {}
        }
    }
}

fn build_workspace_children(
    project_root: &Path,
    current_dir: &Path,
    relative_dir: PathBuf,
    expanded: &HashSet<PathBuf>,
    collapsed: &HashSet<PathBuf>,
    selected_file: Option<&Path>,
) -> Vec<OutlineNode> {
    let Ok(entries) = fs::read_dir(current_dir) else {
        return Vec::new();
    };
    let mut entries = entries.filter_map(|entry| entry.ok()).collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());

    let mut nodes = Vec::new();
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if should_skip_outline_dir(current_dir, &name) {
                continue;
            }
            let child_relative = relative_dir.join(&name);
            // 触发条件：当前选中文件位于被用户手动折叠的目录下。
            // 不能只按 selected_file 自动展开：文件监听刷新会重建树。
            // 防止用户刚折叠的目录在下一帧或下一次刷新中弹开。
            let is_expanded = expanded.contains(&child_relative)
                || outline_workspace_dir_contains_selected(
                    project_root,
                    &child_relative,
                    selected_file,
                ) && !collapsed.contains(&child_relative);
            let children = build_workspace_children(
                project_root,
                &path,
                child_relative.clone(),
                expanded,
                collapsed,
                selected_file,
            );
            nodes.push(OutlineNode::Dir {
                key: child_relative,
                label: name,
                expanded: is_expanded,
                children,
            });
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        {
            let relative = path
                .strip_prefix(project_root)
                .map(Path::to_path_buf)
                .unwrap_or(path.clone());
            let _selected = selected_file.is_some_and(|selected| selected == path);
            nodes.push(OutlineNode::File {
                path: relative,
                label: name,
            });
        }
    }
    nodes
}

fn build_home_children(
    home: &Path,
    selected_file: Option<&Path>,
    expanded: &HashSet<PathBuf>,
    collapsed: &HashSet<PathBuf>,
) -> Vec<OutlineNode> {
    let mut children = Vec::new();
    let mut root_markdowns = fs::read_dir(home)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(|entry| entry.ok()))
        .filter_map(|entry| {
            let path = entry.path();
            entry
                .file_type()
                .ok()
                .filter(|file_type| file_type.is_file())?;
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
                .then_some(path)
        })
        .collect::<Vec<_>>();
    root_markdowns.sort();
    for path in root_markdowns {
        let Some(label) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let label = label.to_string();
        let _selected = selected_file.is_some_and(|selected| selected == path);
        children.push(OutlineNode::File { path, label });
    }

    for dir_name in HOME_OUTLINE_DIRS {
        let dir = home.join(dir_name);
        if !dir.is_dir() {
            continue;
        }
        let key = PathBuf::from(HOME_OUTLINE_ROOT).join(dir_name);
        let is_expanded = expanded.contains(&key) && !collapsed.contains(&key);
        // Trigger: status/outline refresh keeps home roots visible.
        // Why not probe recursively: ~/.codex and peers can be very large.
        // Prevents: a closed home outline section from scanning all sessions.
        let dir_children = if is_expanded {
            build_home_dir_children(&dir, key.clone(), expanded)
        } else {
            Vec::new()
        };
        children.push(OutlineNode::Dir {
            key,
            label: dir_name.to_string(),
            expanded: is_expanded,
            children: dir_children,
        });
    }

    children
}

fn build_home_dir_children(
    current_dir: &Path,
    key_dir: PathBuf,
    expanded: &HashSet<PathBuf>,
) -> Vec<OutlineNode> {
    let Ok(entries) = fs::read_dir(current_dir) else {
        return Vec::new();
    };
    let mut entries = entries.filter_map(|entry| entry.ok()).collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());

    let mut nodes = Vec::new();
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if should_skip_outline_dir(current_dir, &name) {
                continue;
            }
            let child_key = key_dir.join(&name);
            let is_expanded = expanded.contains(&child_key);
            // Trigger: home outline directories are refreshed periodically.
            // Why not recurse collapsed dirs: agent session folders fan out.
            // Prevents: closed outline branches from dominating allocations.
            let children = if is_expanded {
                build_home_dir_children(&path, child_key.clone(), expanded)
            } else {
                Vec::new()
            };
            nodes.push(OutlineNode::Dir {
                key: child_key,
                label: name,
                expanded: is_expanded,
                children,
            });
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        {
            nodes.push(OutlineNode::File { path, label: name });
        }
    }
    nodes
}

fn build_attached_dir_children(
    current_dir: &Path,
    expanded: &HashSet<PathBuf>,
    collapsed: &HashSet<PathBuf>,
    selected_file: Option<&Path>,
) -> Vec<OutlineNode> {
    let Ok(entries) = fs::read_dir(current_dir) else {
        return Vec::new();
    };
    let mut entries = entries.filter_map(|entry| entry.ok()).collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());

    let mut nodes = Vec::new();
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if should_skip_outline_dir(current_dir, &name) {
                continue;
            }
            let is_expanded = (expanded.contains(&path)
                || outline_attached_dir_contains_selected(&path, selected_file))
                && !collapsed.contains(&path);
            let children = if is_expanded {
                build_attached_dir_children(&path, expanded, collapsed, selected_file)
            } else {
                Vec::new()
            };
            nodes.push(OutlineNode::Dir {
                key: path.clone(),
                label: name,
                expanded: is_expanded,
                children,
            });
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        {
            nodes.push(OutlineNode::File { path, label: name });
        }
    }
    nodes
}

fn first_markdown_file(nodes: &[OutlineNode]) -> Option<PathBuf> {
    for node in nodes {
        match node {
            OutlineNode::Root { children, .. } | OutlineNode::Dir { children, .. } => {
                if let Some(path) = first_markdown_file(children) {
                    return Some(path);
                }
            }
            OutlineNode::File { path, .. } => return Some(path.clone()),
        }
    }
    None
}

fn default_expanded_dirs(root: &Path, attached_outline_dirs: &[PathBuf]) -> HashSet<PathBuf> {
    let mut expanded = HashSet::new();
    expanded.insert(PathBuf::new());
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.filter_map(|entry| entry.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            if entry
                .file_type()
                .map(|file_type| file_type.is_dir())
                .unwrap_or(false)
                && !should_skip_outline_dir(root, &name)
            {
                expanded.insert(PathBuf::from(name));
            }
        }
    }
    expanded.insert(PathBuf::from(HOME_OUTLINE_ROOT));
    for dir_name in HOME_OUTLINE_DIRS {
        expanded.insert(PathBuf::from(HOME_OUTLINE_ROOT).join(dir_name));
    }
    expanded.extend(attached_outline_dirs.iter().cloned());
    expanded
}

/// Returns whether a workspace directory contains the selected file.
fn outline_workspace_dir_contains_selected(
    project_root: &Path,
    dir_relative: &Path,
    selected_file: Option<&Path>,
) -> bool {
    let Some(selected_file) = selected_file else {
        return false;
    };
    let selected_relative = selected_file
        .strip_prefix(project_root)
        .unwrap_or(selected_file);
    selected_relative.starts_with(dir_relative)
}

fn outline_attached_dir_contains_selected(dir: &Path, selected_file: Option<&Path>) -> bool {
    selected_file.is_some_and(|selected| selected.is_absolute() && selected.starts_with(dir))
}

fn attached_outline_label(dir: &Path) -> String {
    dir.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| dir.to_string_lossy().to_string())
}

pub fn normalize_attached_outline_dir(path: PathBuf) -> Option<PathBuf> {
    let path = normalize_path(&path);
    path.is_dir().then_some(path)
}

fn normalize_attached_outline_dirs(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for path in paths {
        let Some(path) = normalize_attached_outline_dir(path) else {
            continue;
        };
        if seen.insert(path.clone()) {
            normalized.push(path);
        }
    }
    normalized
}

fn should_skip_outline_dir(parent: &Path, dir_name: &str) -> bool {
    if matches!(
        dir_name,
        ".git"
            | ".hg"
            | ".svn"
            | ".jj"
            | ".idea"
            | ".vscode"
            | ".direnv"
            | ".cache"
            | ".pnpm-store"
            | ".yarn"
            | ".npm"
            | ".next"
            | ".nx"
            | ".nuxt"
            | ".svelte-kit"
            | ".parcel-cache"
            | ".turbo"
            | ".terraform"
            | ".terraform.d"
            | ".serverless"
            | ".aws-sam"
            | "__pycache__"
            | ".pytest_cache"
            | ".mypy_cache"
            | ".ruff_cache"
            | ".tox"
            | ".venv"
            | "venv"
            | "vendor"
            | "dist"
            | "build"
            | "coverage"
    ) {
        return true;
    }
    match dir_name {
        "target" => parent.join("Cargo.toml").is_file(),
        "node_modules" => parent.join("package.json").is_file(),
        _ => false,
    }
}

fn workspace_name(project_dir: &Path) -> String {
    let mut segments = project_dir
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    match segments.len() {
        0 => "workspace".to_string(),
        1 => segments[0].to_string(),
        _ => {
            let tail = segments.split_off(segments.len() - 2);
            tail.join("/")
        }
    }
}

fn new_agent_id(project_dir: &Path) -> String {
    format!("gsdv-{:x}", stable_state_key(project_dir))
}

/// Builds a stable hook id for a workspace subagent.
fn subagent_agent_id(project_dir: &Path, subagent_id: &str) -> String {
    format!("{}-{subagent_id}", new_agent_id(project_dir))
}

fn stable_state_key(path: &Path) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    normalize_path(path).to_string_lossy().hash(&mut hasher);
    hasher.finish()
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn load_store() -> StoreFile {
    let Some(path) = gsdv_store_path() else {
        return StoreFile::default();
    };
    let Ok(content) = fs::read_to_string(path) else {
        return StoreFile::default();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

fn prune_agent_statuses_to_store_sessions(store: &StoreFile) {
    let Some(path) = agent_status_path() else {
        return;
    };
    let Ok(content) = fs::read_to_string(&path) else {
        return;
    };
    let Ok(mut root) = serde_json::from_str::<serde_json::Value>(&content) else {
        return;
    };
    let store_sessions = store
        .workspaces
        .iter()
        .filter_map(|workspace| workspace.session_id.as_deref())
        .filter(|session_id| !session_id.trim().is_empty())
        .map(str::to_string)
        .chain(store.workspaces.iter().flat_map(|workspace| {
            let path = PathBuf::from(&workspace.path);
            load_workspace_subagents(&path, workspace.agent_kind.unwrap_or_default())
                .into_iter()
                .filter_map(|subagent| subagent.session_id)
        }))
        .collect::<HashSet<_>>();
    let mut changed = false;

    if let Some(sessions) = root
        .get_mut("sessions")
        .and_then(serde_json::Value::as_object_mut)
    {
        let before = sessions.len();
        sessions.retain(|session_id, _| store_sessions.contains(session_id.as_str()));
        changed |= sessions.len() != before;
    }
    if let Some(agents) = root
        .get_mut("agents")
        .and_then(serde_json::Value::as_object_mut)
    {
        let before = agents.len();
        agents.retain(|_, entry| {
            entry
                .get("session_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|session_id| store_sessions.contains(session_id))
        });
        changed |= agents.len() != before;
    }
    if changed {
        root["version"] = serde_json::Value::from(2);
        if let Ok(content) = serde_json::to_string_pretty(&root) {
            let _ = fs::write(path, format!("{content}\n"));
        }
    }
}

fn prune_store_sessions_to_status_file(store: &mut StoreFile) {
    let Some(store_path) = gsdv_store_path() else {
        return;
    };
    let Some(status_path) = agent_status_path() else {
        clear_store_sessions(store, &store_path);
        return;
    };
    let Ok(content) = fs::read_to_string(status_path) else {
        clear_store_sessions(store, &store_path);
        return;
    };
    let Ok(file) = serde_json::from_str::<AgentStatusFile>(&content) else {
        clear_store_sessions(store, &store_path);
        return;
    };
    let status_sessions = agent_status_entries(file)
        .into_iter()
        .filter_map(|entry| entry.session_id)
        .filter(|session_id| !session_id.trim().is_empty())
        .collect::<HashSet<_>>();
    let mut changed = false;
    for workspace in &mut store.workspaces {
        let stale = workspace
            .session_id
            .as_deref()
            .filter(|session_id| !session_id.trim().is_empty())
            .is_some_and(|session_id| !status_sessions.contains(session_id));
        if stale {
            workspace.session_id = None;
            changed = true;
        }
    }
    if changed {
        write_store_file(&store_path, store);
    }
}

fn clear_store_sessions(store: &mut StoreFile, store_path: &Path) {
    let mut changed = false;
    for workspace in &mut store.workspaces {
        if workspace.session_id.take().is_some() {
            changed = true;
        }
    }
    if changed {
        write_store_file(store_path, store);
    }
}

fn write_store_file(path: &Path, store: &StoreFile) {
    let _guard = store_write_guard();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(content) = serde_json::to_string_pretty(store) {
        let _ = fs::write(path, format!("{content}\n"));
    }
}

/// Serializes whole-store read-modify-write updates inside the app process.
///
/// 触发条件：settings 保存和 workspace 保存都在后台线程里执行。
/// 不能只靠各保存函数保留未知字段：两个线程会基于旧 store 交叉写回。
/// 防止回归：后完成的 workspace 保存不再覆盖刚写入的设置。
fn store_write_guard() -> std::sync::MutexGuard<'static, ()> {
    STORE_WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("store write lock should not be poisoned")
}

/// Loads agent status once and builds all lookup indexes used by workspaces.
fn load_agent_status_indexes() -> AgentStatusIndexes {
    let Some(path) = agent_status_path() else {
        return empty_agent_status_indexes();
    };
    let Ok(content) = fs::read_to_string(path) else {
        return empty_agent_status_indexes();
    };
    let Ok(file) = serde_json::from_str::<AgentStatusFile>(&content) else {
        return empty_agent_status_indexes();
    };

    let mut by_session = empty_agent_status_session_maps();
    let mut by_id = empty_agent_status_id_maps();
    let mut by_path = empty_agent_status_path_maps();
    let mut id_order: BTreeMap<AgentKind, BTreeMap<String, u64>> = empty_agent_status_id_orders();
    let mut path_order: BTreeMap<AgentKind, BTreeMap<PathBuf, u64>> =
        empty_agent_status_path_orders();

    for entry in agent_status_entries(file) {
        let agent_kinds = agent_kinds_for_status_entry(&entry);
        let session_id = entry
            .session_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string);
        let agent_id = entry
            .agent_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string);
        let workspace_path = (!entry.workspace.trim().is_empty())
            .then(|| normalize_path(Path::new(&entry.workspace)));
        let updated_at_ms = entry_status_order(&entry);
        let status = agent_workspace_status_from_entry(entry);

        for agent_kind in agent_kinds {
            if let Some(session_id) = session_id.as_ref() {
                by_session
                    .entry(agent_kind)
                    .or_default()
                    .insert(session_id.clone(), status.clone());
            }
            if let Some(agent_id) = agent_id.as_ref() {
                let replace = id_order
                    .entry(agent_kind)
                    .or_default()
                    .get(agent_id)
                    .is_none_or(|existing| updated_at_ms >= *existing);
                if replace {
                    id_order
                        .entry(agent_kind)
                        .or_default()
                        .insert(agent_id.clone(), updated_at_ms);
                    by_id
                        .entry(agent_kind)
                        .or_default()
                        .insert(agent_id.clone(), status.clone());
                }
            }
            if let Some(path) = workspace_path.as_ref() {
                let replace = path_order
                    .entry(agent_kind)
                    .or_default()
                    .get(path)
                    .is_none_or(|existing| updated_at_ms >= *existing);
                if replace {
                    path_order
                        .entry(agent_kind)
                        .or_default()
                        .insert(path.clone(), updated_at_ms);
                    by_path
                        .entry(agent_kind)
                        .or_default()
                        .insert(path.clone(), status.clone());
                }
            }
        }
    }

    AgentStatusIndexes {
        by_path,
        by_id,
        by_session,
    }
}

/// Returns empty status indexes for missing or unreadable status files.
fn empty_agent_status_indexes() -> AgentStatusIndexes {
    AgentStatusIndexes {
        by_path: empty_agent_status_path_maps(),
        by_id: empty_agent_status_id_maps(),
        by_session: empty_agent_status_session_maps(),
    }
}

/// Creates one workspace-path status map per supported agent kind.
fn empty_agent_status_path_maps() -> BTreeMap<AgentKind, BTreeMap<PathBuf, AgentWorkspaceStatus>> {
    AgentKind::all()
        .into_iter()
        .map(|agent_kind| (agent_kind, BTreeMap::new()))
        .collect()
}

/// Creates one agent-id status map per supported agent kind.
fn empty_agent_status_id_maps() -> BTreeMap<AgentKind, BTreeMap<String, AgentWorkspaceStatus>> {
    AgentKind::all()
        .into_iter()
        .map(|agent_kind| (agent_kind, BTreeMap::new()))
        .collect()
}

/// Creates one session-id status map per supported agent kind.
fn empty_agent_status_session_maps() -> BTreeMap<AgentKind, BTreeMap<String, AgentWorkspaceStatus>>
{
    AgentKind::all()
        .into_iter()
        .map(|agent_kind| (agent_kind, BTreeMap::new()))
        .collect()
}

/// Creates timestamp order maps for agent-id status replacement.
fn empty_agent_status_id_orders() -> BTreeMap<AgentKind, BTreeMap<String, u64>> {
    AgentKind::all()
        .into_iter()
        .map(|agent_kind| (agent_kind, BTreeMap::new()))
        .collect()
}

/// Creates timestamp order maps for workspace-path status replacement.
fn empty_agent_status_path_orders() -> BTreeMap<AgentKind, BTreeMap<PathBuf, u64>> {
    AgentKind::all()
        .into_iter()
        .map(|agent_kind| (agent_kind, BTreeMap::new()))
        .collect()
}

/// Resolves which agent kinds should consume a status entry.
fn agent_kinds_for_status_entry(entry: &AgentStatusEntry) -> Vec<AgentKind> {
    let Some(entry_agent) = entry
        .agent
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return AgentKind::all().to_vec();
    };
    AgentKind::all()
        .into_iter()
        .filter(|agent_kind| entry_agent.eq_ignore_ascii_case(agent_kind.env_name()))
        .collect()
}

fn agent_status_entries(file: AgentStatusFile) -> Vec<AgentStatusEntry> {
    let mut entries = Vec::new();
    for (session_id, mut entry) in file.sessions {
        if entry.session_id.as_deref().is_none_or(str::is_empty) {
            entry.session_id = Some(session_id);
        }
        entries.push(entry);
    }
    for (agent_id, mut entry) in file.agents {
        if entry.agent_id.as_deref().is_none_or(str::is_empty) {
            entry.agent_id = Some(agent_id);
        }
        entries.push(entry);
    }
    entries
}

fn entry_status_order(entry: &AgentStatusEntry) -> u64 {
    entry.started_at_ms.or(entry.updated_at_ms).unwrap_or(0)
}

fn agent_workspace_status_from_entry(entry: AgentStatusEntry) -> AgentWorkspaceStatus {
    let session_id = entry
        .session_id
        .filter(|session_id| !session_id.trim().is_empty());
    let turn_id = entry.turn_id.filter(|turn_id| !turn_id.trim().is_empty());
    let transcript_path = entry
        .transcript_path
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from);
    let mut activity = workspace_activity_from_status(&entry.status);
    if activity == WorkspaceActivity::Busy
        && codex_transcript_has_aborted_turn(
            transcript_path.as_deref(),
            session_id.as_deref(),
            turn_id.as_deref(),
        )
    {
        activity = WorkspaceActivity::Idle;
    }
    AgentWorkspaceStatus {
        activity,
        session_id,
    }
}

fn workspace_activity_from_status(status: &str) -> WorkspaceActivity {
    match status.trim().to_ascii_lowercase().as_str() {
        "busy" => WorkspaceActivity::Busy,
        "idle" => WorkspaceActivity::Idle,
        _ => WorkspaceActivity::Unknown,
    }
}

/// Checks Codex transcript data for an aborted marker for the active turn.
fn codex_transcript_has_aborted_turn(
    transcript_path: Option<&Path>,
    session_id: Option<&str>,
    turn_id: Option<&str>,
) -> bool {
    let Some(turn_id) = turn_id else {
        return false;
    };
    if transcript_path.is_some_and(|path| transcript_contains_aborted_turn(path, turn_id)) {
        return true;
    }
    session_id
        .and_then(find_codex_transcript_path)
        .is_some_and(|path| transcript_contains_aborted_turn(&path, turn_id))
}

/// Checks whether a transcript contains a Codex turn_aborted entry.
fn transcript_contains_aborted_turn(path: &Path, turn_id: &str) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    let cache_key = TranscriptAbortCacheKey {
        path: normalize_path(path),
        turn_id: turn_id.to_string(),
        len: metadata.len(),
        modified: metadata.modified().ok(),
    };
    if let Some(cached) = transcript_abort_cache_lookup(&cache_key) {
        return cached;
    }

    let found = transcript_file_contains_aborted_turn(path, turn_id);
    transcript_abort_cache_store(cache_key, found);
    found
}

/// Reads transcript lines incrementally while looking for an aborted turn.
fn transcript_file_contains_aborted_turn(path: &Path, turn_id: &str) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    loop {
        line.clear();
        let Ok(read) = reader.read_line(&mut line) else {
            return false;
        };
        if read == 0 {
            return false;
        }
        // Trigger: agent status is still busy while Codex may have aborted.
        // Why not the normal status path: the hook can lag behind transcript.
        // Prevents: reading a large jsonl transcript or allocating every line.
        if !line.contains("turn_aborted") || !line.contains(turn_id) {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let payload = value.get("payload").unwrap_or(&serde_json::Value::Null);
        if payload.get("type").and_then(serde_json::Value::as_str) == Some("turn_aborted")
            && payload.get("turn_id").and_then(serde_json::Value::as_str) == Some(turn_id)
        {
            return true;
        }
    }
}

/// Returns a cached transcript aborted-turn result when the file is unchanged.
fn transcript_abort_cache_lookup(cache_key: &TranscriptAbortCacheKey) -> Option<bool> {
    let cache = TRANSCRIPT_ABORT_CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    cache.lock().ok()?.get(cache_key).copied()
}

/// Stores a transcript aborted-turn result with a bounded cache size.
fn transcript_abort_cache_store(cache_key: TranscriptAbortCacheKey, found: bool) {
    let cache = TRANSCRIPT_ABORT_CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    let Ok(mut cache) = cache.lock() else {
        return;
    };
    if cache.len() >= TRANSCRIPT_ABORT_CACHE_LIMIT {
        cache.clear();
    }
    cache.insert(cache_key, found);
}

fn find_codex_transcript_path(session_id: &str) -> Option<PathBuf> {
    let root = home_dir()?.join(".codex").join("sessions");
    find_codex_transcript_path_in(&root, session_id)
}

fn find_codex_transcript_path_in(root: &Path, session_id: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_codex_transcript_path_in(&path, session_id) {
                return Some(found);
            }
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains(session_id) && name.ends_with(".jsonl"))
        {
            return Some(path);
        }
    }
    None
}

fn normalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn home_dir() -> Option<PathBuf> {
    crate::home::home_dir()
}

fn gsdv_store_path() -> Option<PathBuf> {
    env::var_os("GSDV_STORE_PATH")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".gsdv").join("store")))
}

/// Returns the standalone Markdown file used for a workspace memo.
pub(crate) fn workspace_memo_path(workspace_path: &Path) -> Option<PathBuf> {
    Some(workspace_store_dir(workspace_path)?.join("memo.md"))
}

/// Returns the standalone text file used for workspace terminal history.
pub(crate) fn workspace_terminal_history_path(workspace_path: &Path) -> Option<PathBuf> {
    Some(workspace_store_dir(workspace_path)?.join("terminal.txt"))
}

/// Returns the shared agent status file path used by hooks and the GUI.
pub fn agent_status_path() -> Option<PathBuf> {
    env::var_os("GSDV_AGENT_STATUS_PATH")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".gsdv").join("agent-status.json")))
}

#[cfg(test)]
#[path = "data_test.rs"]
mod data_test;

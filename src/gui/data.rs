use crate::gui::agent::AgentKind;
use crate::gui::remote_workspace::RemoteWorkspaceConfig;
use crate::gui::theme::ThemeMode;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const HOME_OUTLINE_ROOT: &str = "~";
const HOME_OUTLINE_DIRS: [&str; 3] = [".codex", ".agents", ".claude"];
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
pub const DEFAULT_REMOTE_SERVER_ENABLED: bool = true;
pub const DEFAULT_REMOTE_SERVER_HOST: &str = "0.0.0.0";
pub const DEFAULT_REMOTE_SERVER_PORT: u16 = 20050;
pub const MIN_REMOTE_SERVER_PORT: u16 = 1;
pub const MAX_REMOTE_SERVER_PORT: u16 = u16::MAX;
/// Fixed Codex model provider option requested by the UI.
pub const CODEX_MODEL_PROVIDER_OPENAI: &str = "openai";

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
    /// Remote 连接配置；None 表示普通本地 workspace。
    pub remote: Option<RemoteWorkspaceConfig>,
    /// Remote 项目 Outline 是否折叠成 28px 恢复条。
    pub remote_outline_collapsed: bool,
    /// 本地项目 Outline 面板是否折叠成 28px 恢复条。
    pub outline_panel_collapsed: bool,
    /// Whether this workspace is included when focus filtering is enabled.
    pub focused: bool,
    pub name: String,
    pub path: PathBuf,
    pub agent_kind: AgentKind,
    /// Per-workspace main agent model override.
    pub agent_model: Option<String>,
    /// Per-workspace Codex model provider override.
    pub agent_model_provider: Option<String>,
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
    /// Visible Agent rows owned by this workspace.
    pub agent_rows: Vec<AgentRowViewData>,
    /// Focused Agent row and column used for keyboard routing.
    pub agent_focus: Option<AgentFocusViewData>,
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

/// Persisted and runtime metadata for one visible Agent row.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AgentRowViewData {
    /// Agent columns displayed inside this row.
    #[serde(default)]
    pub columns: Vec<AgentColumnViewData>,
    /// Relative height used by the draggable multi-row layout.
    #[serde(default = "default_agent_row_height_weight")]
    pub height_weight: f32,
    /// Whether this row is collapsed into a restore strip.
    #[serde(default)]
    pub collapsed: bool,
}

/// Persisted and runtime metadata for one visible Agent column.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct AgentColumnViewData {
    /// Stable id used by UI state, splitters, and dialog routing.
    pub id: String,
    /// Agent tabs displayed inside this column.
    #[serde(default)]
    pub tabs: Vec<AgentColumnSlot>,
    /// The currently selected tab in this column.
    #[serde(default)]
    pub active_slot: AgentColumnSlot,
    /// Relative width used by the draggable multi-column layout.
    #[serde(default = "default_agent_column_width_weight")]
    pub width_weight: f32,
    /// Whether this column is collapsed into a restore strip.
    #[serde(default)]
    pub collapsed: bool,
}

/// Focused Agent grid cell for keyboard and active-agent routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct AgentFocusViewData {
    /// Zero-based row index.
    pub row_index: usize,
    /// Zero-based column index inside the row.
    pub column_index: usize,
}

/// Stored identifier for a main-agent or subagent tab in one column.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentColumnSlot {
    /// The workspace's built-in primary Agent.
    Main,
    /// A named subagent stored in the workspace sidecar.
    Subagent(String),
}

impl Default for AgentColumnSlot {
    /// Uses the built-in Agent when older sidecar data has no active tab.
    fn default() -> Self {
        Self::Main
    }
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
    /// Per-subagent Codex model provider override passed to the agent CLI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_model_provider: Option<String>,
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
    /// Last known runtime activity from direct Agent hook events.
    #[serde(skip, default = "default_workspace_activity")]
    pub activity: WorkspaceActivity,
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
    /// Whether the workspace rail only shows focused workspaces.
    pub focus_filter_enabled: bool,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct NetworkSettings {
    /// Whether proxy env vars should be injected into child processes.
    pub proxy_enabled: bool,
    /// Primary proxy URL used for child processes.
    pub proxy: String,
    /// Additional no_proxy entries appended after gsdv built-ins.
    pub no_proxy: String,
    /// Whether to emit both HTTP(S) and SOCKS proxy env variants.
    pub mirror_proxy_protocol: bool,
}

/// Serialized network settings shape, used to migrate older proxy configs.
#[derive(Debug, Default, Deserialize)]
struct NetworkSettingsFile {
    /// Stored proxy switch; missing in old settings files.
    #[serde(default)]
    proxy_enabled: Option<bool>,
    /// Stored primary proxy URL.
    #[serde(default)]
    proxy: String,
    /// Stored no_proxy additions.
    #[serde(default)]
    no_proxy: String,
    /// Stored protocol mirroring switch.
    #[serde(default)]
    mirror_proxy_protocol: bool,
}

impl<'de> Deserialize<'de> for NetworkSettings {
    /// Loads network settings and keeps old non-empty proxy configs enabled.
    ///
    /// Example: `{ "proxy": "http://127.0.0.1:7890" }` -> enabled proxy.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let file = NetworkSettingsFile::deserialize(deserializer)?;
        let proxy_enabled = file
            .proxy_enabled
            .unwrap_or_else(|| !file.proxy.trim().is_empty());
        Ok(Self {
            proxy_enabled,
            proxy: file.proxy,
            no_proxy: file.no_proxy,
            mirror_proxy_protocol: file.mirror_proxy_protocol,
        })
    }
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
    /// Whether the embedded remote HTTP/WebSocket server should listen.
    #[serde(default = "default_remote_server_enabled")]
    pub remote_server_enabled: bool,
    /// Listen host for the embedded remote HTTP/WebSocket server.
    #[serde(default = "default_remote_server_host")]
    pub remote_server_host: String,
    /// Listen port for the embedded remote HTTP/WebSocket server.
    #[serde(default = "default_remote_server_port")]
    pub remote_server_port: u16,
}

impl Default for RuntimeSettings {
    /// Builds first-run runtime defaults used when the store has no settings.
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
            remote_server_enabled: default_remote_server_enabled(),
            remote_server_host: default_remote_server_host(),
            remote_server_port: default_remote_server_port(),
        }
    }
}

impl RuntimeSettings {
    /// Returns the remote listener subset used by the server lifecycle owner.
    pub fn remote_server_settings(&self) -> RemoteServerSettings {
        RemoteServerSettings {
            enabled: self.remote_server_enabled,
            host: self.remote_server_host.clone(),
            port: self.remote_server_port,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteServerSettings {
    /// Whether the embedded remote HTTP/WebSocket server should listen.
    pub enabled: bool,
    /// Listen host accepted by Tokio's socket address resolver.
    pub host: String,
    /// TCP listen port.
    pub port: u16,
}

impl RemoteServerSettings {
    /// Formats the configured listener address for bind calls and UI messages.
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Returns the default upper bound for application-driven repaint requests.
fn default_max_frame_rate() -> u16 {
    DEFAULT_MAX_FRAME_RATE
}

/// Returns the default relative width for one Agent column.
fn default_agent_column_width_weight() -> f32 {
    1.0
}

/// Returns the default relative height for one Agent row.
fn default_agent_row_height_weight() -> f32 {
    1.0
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

/// Returns whether the remote server listens on first-run settings.
fn default_remote_server_enabled() -> bool {
    DEFAULT_REMOTE_SERVER_ENABLED
}

/// Returns the first-run remote server listen host.
fn default_remote_server_host() -> String {
    DEFAULT_REMOTE_SERVER_HOST.to_string()
}

/// Returns the first-run remote server listen port.
fn default_remote_server_port() -> u16 {
    DEFAULT_REMOTE_SERVER_PORT
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
        if !self.proxy_enabled || proxy.is_empty() {
            return env;
        }
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
    focus_filter_enabled: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote: Option<RemoteWorkspaceConfig>,
    /// Remote Outline 的 workspace 级持久折叠状态。
    #[serde(default)]
    remote_outline_collapsed: bool,
    /// 本地 Outline 面板的 workspace 级持久折叠状态。
    #[serde(default)]
    outline_panel_collapsed: bool,
    /// Whether this workspace is marked as focused in the rail.
    #[serde(default)]
    focused: bool,
    #[serde(default)]
    agent_kind: Option<AgentKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_model_provider: Option<String>,
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

/// 与主 workspace store 分离保存的 subagent sidecar 数据。
#[derive(Debug, Default, Deserialize, Serialize)]
struct SubagentFile {
    /// Subagents that belong to one workspace.
    #[serde(default)]
    subagents: Vec<SubagentViewData>,
    /// Legacy visible Agent columns for this workspace.
    #[serde(default)]
    columns: Vec<AgentColumnViewData>,
    /// Visible Agent rows for this workspace.
    #[serde(default)]
    rows: Vec<AgentRowViewData>,
    /// Focused Agent row and column.
    #[serde(default)]
    focus: Option<AgentFocusViewData>,
}

/// Sidecar payload for workspace-local outline favorites.
#[derive(Debug, Default, Deserialize, Serialize)]
struct OutlineFavoritesFile {
    /// Workspace-relative Markdown paths favorited in the outline.
    #[serde(default)]
    favorites: BTreeSet<PathBuf>,
}

pub fn load_initial_gui_data(default_agent_kind: AgentKind) -> InitialGuiData {
    let stored = load_store();
    let workspaces = stored
        .workspaces
        .into_iter()
        .filter_map(|workspace| {
            if let Some(config) = workspace.remote.clone() {
                return Some(new_remote_workspace_with_outline_state(
                    config,
                    default_agent_kind,
                    workspace.remote_outline_collapsed,
                    workspace.focused,
                ));
            }
            let path = PathBuf::from(&workspace.path);
            if path.is_dir() {
                Some(build_workspace(path, workspace, default_agent_kind))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let active_workspace = stored
        .active
        .filter(|index| *index < workspaces.len())
        .unwrap_or(0);
    let active_workspace = if stored.focus_filter_enabled
        && !workspaces
            .get(active_workspace)
            .is_some_and(|workspace| workspace.focused)
    {
        workspaces
            .iter()
            .position(|workspace| workspace.focused)
            .unwrap_or(active_workspace)
    } else {
        active_workspace
    };

    InitialGuiData {
        active_workspace,
        workspaces,
        rail_collapsed: stored.rail_collapsed,
        focus_filter_enabled: stored.focus_filter_enabled,
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
    settings.remote_server_host = sanitize_remote_server_host(&settings.remote_server_host);
    settings.remote_server_port = settings
        .remote_server_port
        .clamp(MIN_REMOTE_SERVER_PORT, MAX_REMOTE_SERVER_PORT);
    settings.agent_custom_quick_replies =
        sanitize_agent_custom_quick_replies(&settings.agent_custom_quick_replies);
    settings
}

/// Normalizes the remote listen host without inventing DNS behavior.
fn sanitize_remote_server_host(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        DEFAULT_REMOTE_SERVER_HOST.to_string()
    } else {
        value.to_string()
    }
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
    let dir_states = outline_dir_states(&workspace.outline);
    workspace.outline = build_outline_with_state(
        &workspace.path,
        &workspace.attached_outline_dirs,
        dir_states,
    );
    if workspace.selected_file.is_none() {
        workspace.selected_file = first_markdown_file(&workspace.outline);
    }
}

/// Rebuilds outline data without cloning unrelated workspace UI state.
///
/// Example: large Agent rows plus outline -> returns only path, outline, selected file.
pub fn refresh_workspace_outline_parts(
    workspace_path: &Path,
    attached_outline_dirs: &[PathBuf],
    previous_outline: &[OutlineNode],
    selected_file: Option<PathBuf>,
) -> (PathBuf, Vec<OutlineNode>, Option<PathBuf>) {
    let dir_states = outline_dir_states(previous_outline);
    let outline = build_outline_with_state(workspace_path, attached_outline_dirs, dir_states);
    let selected_file = selected_file.or_else(|| first_markdown_file(&outline));
    (workspace_path.to_path_buf(), outline, selected_file)
}

pub fn new_workspace(path: PathBuf, agent_kind: AgentKind) -> WorkspaceViewData {
    build_workspace(
        path.clone(),
        StoredWorkspace {
            path: path.to_string_lossy().to_string(),
            remote: None,
            remote_outline_collapsed: false,
            outline_panel_collapsed: false,
            focused: false,
            agent_kind: Some(agent_kind),
            agent_model: None,
            agent_model_provider: None,
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
        agent_kind,
    )
}

/// 构造尚未连接的 Remote Workspace 占位渲染状态。
///
/// 适用场景：新增 Remote Workspace。例：连接成功 -> 后台快照替换 Agent 数据。
pub fn new_remote_workspace(
    config: RemoteWorkspaceConfig,
    agent_kind: AgentKind,
) -> WorkspaceViewData {
    new_remote_workspace_with_outline_state(config, agent_kind, false, false)
}

/// 按持久状态构造尚未连接的 Remote Workspace 占位渲染状态。
///
/// 适用场景：从 store 恢复 Remote Outline 折叠状态。
/// 例：`collapsed=true -> 28px Remote Outline`。
fn new_remote_workspace_with_outline_state(
    mut config: RemoteWorkspaceConfig,
    agent_kind: AgentKind,
    remote_outline_collapsed: bool,
    focused: bool,
) -> WorkspaceViewData {
    if config.workspace_key.trim().is_empty() {
        config.workspace_key = crate::gui::remote_workspace::new_workspace_key();
    }
    let identity = PathBuf::from(format!("remote-{}", config.workspace_key));
    WorkspaceViewData {
        remote: Some(config.clone()),
        remote_outline_collapsed,
        outline_panel_collapsed: false,
        focused,
        name: config.name,
        path: identity.clone(),
        agent_kind,
        agent_model: None,
        agent_model_provider: None,
        agent_effort: None,
        agent_fast_mode: None,
        agent_work_dir: None,
        agent_id: new_agent_id(&identity),
        session_id: None,
        activity: WorkspaceActivity::Unknown,
        subagents: Vec::new(),
        agent_rows: default_agent_rows(&[]),
        agent_focus: Some(default_agent_focus()),
        center_mode: CenterMode::Agent,
        previous_center_mode: CenterMode::Agent,
        route: Route::Workspace,
        reviewer_mode: ReviewerMode::Git,
        selected_file: None,
        outline: Vec::new(),
        outline_favorites: BTreeSet::new(),
        attached_outline_dirs: Vec::new(),
        recent_markdowns: Vec::new(),
        markdown_outline_collapsed: true,
        memo: String::new(),
    }
}

pub fn save_workspace_store(
    workspaces: &[WorkspaceViewData],
    active: usize,
    rail_collapsed: bool,
    focus_filter_enabled: bool,
) {
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
        focus_filter_enabled,
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
                remote: workspace.remote.clone(),
                remote_outline_collapsed: workspace.remote_outline_collapsed,
                outline_panel_collapsed: workspace.outline_panel_collapsed,
                focused: workspace.focused,
                agent_kind: Some(workspace.agent_kind),
                agent_model: workspace.agent_model.clone(),
                agent_model_provider: normalize_stored_agent_model_provider(
                    workspace.agent_kind,
                    workspace.agent_model_provider.clone(),
                ),
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
    for workspace in workspaces
        .iter()
        .filter(|workspace| workspace.remote.is_none())
    {
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
        columns: Vec::new(),
        rows: workspace.agent_rows.clone(),
        focus: workspace.agent_focus,
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

/// 返回清理后的 Codex model provider 覆盖值。
fn normalize_stored_agent_model_provider(
    kind: AgentKind,
    provider: Option<String>,
) -> Option<String> {
    if kind != AgentKind::Codex {
        return None;
    }
    provider
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

/// Reads model provider names from the user's Codex config.
pub fn load_codex_model_provider_names() -> Vec<String> {
    let Some(path) = home_dir().map(|home| home.join(".codex").join("config.toml")) else {
        return Vec::new();
    };
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    codex_model_provider_names_from_config(&content)
}

/// Extracts `[model_providers.<name>]` table names from Codex TOML.
pub fn codex_model_provider_names_from_config(content: &str) -> Vec<String> {
    let mut names = BTreeSet::new();
    for line in content.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed
            .strip_prefix("[model_providers.")
            .and_then(|value| value.strip_suffix(']'))
        else {
            continue;
        };
        if rest.contains('.') || rest.trim().is_empty() {
            continue;
        }
        let name = rest.trim_matches('"').trim();
        if !name.is_empty() {
            names.insert(name.to_string());
        }
    }
    names.into_iter().collect()
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
    agent_model_provider: Option<String>,
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
        agent_model_provider: normalize_stored_agent_model_provider(
            agent_kind,
            agent_model_provider,
        ),
        agent_effort,
        agent_fast_mode: normalize_stored_agent_fast_mode(agent_kind, agent_fast_mode),
        agent_work_dir: normalize_stored_agent_work_dir(agent_work_dir),
        session_id,
        activity: WorkspaceActivity::Unknown,
    }
}

/// Loads subagents persisted in the workspace sidecar.
fn load_workspace_agent_layout(
    workspace_path: &Path,
    default_agent_kind: AgentKind,
) -> (
    Vec<SubagentViewData>,
    Vec<AgentRowViewData>,
    Option<AgentFocusViewData>,
) {
    let Some(path) = workspace_subagents_path(workspace_path) else {
        return (
            Vec::new(),
            default_agent_rows(&[]),
            Some(default_agent_focus()),
        );
    };
    let Ok(content) = fs::read_to_string(path) else {
        return (
            Vec::new(),
            default_agent_rows(&[]),
            Some(default_agent_focus()),
        );
    };
    let Ok(file) = serde_json::from_str::<SubagentFile>(&content) else {
        return (
            Vec::new(),
            default_agent_rows(&[]),
            Some(default_agent_focus()),
        );
    };
    let subagents = file
        .subagents
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
            subagent.agent_model_provider = normalize_stored_agent_model_provider(
                subagent.agent_kind,
                subagent.agent_model_provider,
            );
            subagent.agent_effort =
                normalize_stored_agent_effort(subagent.agent_kind, subagent.agent_effort);
            subagent.agent_fast_mode =
                normalize_stored_agent_fast_mode(subagent.agent_kind, subagent.agent_fast_mode);
            subagent.agent_work_dir = normalize_stored_agent_work_dir(subagent.agent_work_dir);
            subagent.activity = WorkspaceActivity::Unknown;
            subagent
        })
        .collect::<Vec<_>>();
    let rows = if file.rows.is_empty() {
        vec![AgentRowViewData {
            columns: file.columns,
            height_weight: default_agent_row_height_weight(),
            collapsed: false,
        }]
    } else {
        file.rows
    };
    let (rows, focus) = normalize_agent_rows(rows, file.focus, &subagents);
    (subagents, rows, focus)
}

/// Builds the compatibility single-row Agent layout for legacy sidecars.
fn default_agent_rows(subagents: &[SubagentViewData]) -> Vec<AgentRowViewData> {
    let mut tabs = Vec::with_capacity(subagents.len() + 1);
    tabs.push(AgentColumnSlot::Main);
    tabs.extend(
        subagents
            .iter()
            .map(|subagent| AgentColumnSlot::Subagent(subagent.id.clone())),
    );
    vec![AgentRowViewData {
        columns: vec![AgentColumnViewData {
            id: "main".to_string(),
            tabs,
            active_slot: AgentColumnSlot::Main,
            width_weight: default_agent_column_width_weight(),
            collapsed: false,
        }],
        height_weight: default_agent_row_height_weight(),
        collapsed: false,
    }]
}

/// Returns the default focused Agent cell.
fn default_agent_focus() -> AgentFocusViewData {
    AgentFocusViewData {
        row_index: 0,
        column_index: 0,
    }
}

/// Normalizes Agent rows after loading or before saving workspace state.
pub fn normalize_agent_rows(
    rows: Vec<AgentRowViewData>,
    focus: Option<AgentFocusViewData>,
    subagents: &[SubagentViewData],
) -> (Vec<AgentRowViewData>, Option<AgentFocusViewData>) {
    let valid_subagents = subagents
        .iter()
        .map(|subagent| subagent.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut seen_column_ids = BTreeSet::new();
    let mut seen_slots = HashSet::new();
    let mut normalized_rows = Vec::new();
    for (row_index, mut row) in rows.into_iter().enumerate() {
        if !row.height_weight.is_finite() || row.height_weight <= 0.0 {
            row.height_weight = default_agent_row_height_weight();
        }
        let mut columns = Vec::new();
        for (column_index, mut column) in row.columns.into_iter().enumerate() {
            if column.id.trim().is_empty() || !seen_column_ids.insert(column.id.clone()) {
                column.id = format!("col-{}-{:x}-{:x}", now_ms(), row_index, column_index);
            }
            column.tabs.retain(|slot| match slot {
                AgentColumnSlot::Main => true,
                AgentColumnSlot::Subagent(id) => valid_subagents.contains(id.as_str()),
            });
            column.tabs.retain(|slot| seen_slots.insert(slot.clone()));
            if !column.tabs.contains(&column.active_slot) {
                column.active_slot = column
                    .tabs
                    .first()
                    .cloned()
                    .unwrap_or(AgentColumnSlot::Main);
            }
            if !column.width_weight.is_finite() || column.width_weight <= 0.0 {
                column.width_weight = default_agent_column_width_weight();
            }
            columns.push(column);
        }
        if row_index == 0 && columns.is_empty() {
            columns.push(default_agent_rows(&[]).remove(0).columns.remove(0));
        }
        normalize_agent_column_widths(&mut columns);
        row.columns = columns;
        if !row.columns.is_empty() || row_index == 0 {
            normalized_rows.push(row);
        }
    }
    if normalized_rows.is_empty() {
        normalized_rows = default_agent_rows(subagents);
        seen_slots = normalized_rows
            .iter()
            .flat_map(|row| row.columns.iter())
            .flat_map(|column| column.tabs.iter().cloned())
            .collect();
    }
    for subagent in subagents {
        let slot = AgentColumnSlot::Subagent(subagent.id.clone());
        if seen_slots.insert(slot.clone()) {
            normalized_rows[0].columns[0].tabs.push(slot);
        }
    }
    if seen_slots.insert(AgentColumnSlot::Main) {
        normalized_rows[0].columns[0]
            .tabs
            .insert(0, AgentColumnSlot::Main);
    }
    normalize_agent_column_widths(&mut normalized_rows[0].columns);
    normalize_agent_row_heights(&mut normalized_rows);
    let focus = normalize_agent_focus_for_rows(focus, &normalized_rows);
    (normalized_rows, focus)
}

/// Keeps Agent column width weights positive and normalized.
pub fn normalize_agent_column_widths(columns: &mut [AgentColumnViewData]) {
    if columns.is_empty() {
        return;
    }
    for column in columns.iter_mut() {
        if !column.width_weight.is_finite() || column.width_weight <= 0.0 {
            column.width_weight = default_agent_column_width_weight();
        }
    }
    let total = columns
        .iter()
        .map(|column| column.width_weight)
        .sum::<f32>();
    if total <= f32::EPSILON {
        let weight = default_agent_column_width_weight();
        for column in columns {
            column.width_weight = weight;
        }
        return;
    }
    for column in columns {
        column.width_weight /= total;
    }
}

/// Keeps Agent row height weights positive and normalized.
pub fn normalize_agent_row_heights(rows: &mut [AgentRowViewData]) {
    if rows.is_empty() {
        return;
    }
    for row in rows.iter_mut() {
        if !row.height_weight.is_finite() || row.height_weight <= 0.0 {
            row.height_weight = default_agent_row_height_weight();
        }
    }
    let total = rows.iter().map(|row| row.height_weight).sum::<f32>();
    if total <= f32::EPSILON {
        for row in rows {
            row.height_weight = default_agent_row_height_weight();
        }
        return;
    }
    for row in rows {
        row.height_weight /= total;
    }
}

/// Clamps focus to a visible Agent cell, returning none when all cells are folded.
pub fn normalize_agent_focus_for_rows(
    focus: Option<AgentFocusViewData>,
    rows: &[AgentRowViewData],
) -> Option<AgentFocusViewData> {
    if let Some(focus) = focus
        && rows.get(focus.row_index).is_some_and(|row| !row.collapsed)
        && rows
            .get(focus.row_index)
            .and_then(|row| row.columns.get(focus.column_index))
            .is_some_and(|column| !column.collapsed)
    {
        return Some(focus);
    }
    first_visible_agent_focus(rows)
}

/// Finds the first expanded Agent cell in row-major order.
pub fn first_visible_agent_focus(rows: &[AgentRowViewData]) -> Option<AgentFocusViewData> {
    for (row_index, row) in rows.iter().enumerate() {
        if row.collapsed {
            continue;
        }
        for (column_index, column) in row.columns.iter().enumerate() {
            if !column.collapsed {
                return Some(AgentFocusViewData {
                    row_index,
                    column_index,
                });
            }
        }
    }
    None
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
    let outline = build_outline(&path, &attached_outline_dirs);
    let selected_file = stored_selected_file.or_else(|| first_markdown_file(&outline));
    let session_id = stored.session_id.clone();
    let memo = load_workspace_memo(&path);
    let outline_favorites = load_workspace_outline_favorites(&path);
    let recent_markdowns = load_workspace_recent_markdowns(&path);
    let (subagents, mut agent_rows, mut agent_focus) =
        load_workspace_agent_layout(&path, agent_kind);
    (agent_rows, agent_focus) = normalize_agent_rows(agent_rows, agent_focus, &subagents);

    WorkspaceViewData {
        remote: None,
        remote_outline_collapsed: false,
        outline_panel_collapsed: stored.outline_panel_collapsed,
        focused: stored.focused,
        name: workspace_name(&path),
        path,
        agent_kind,
        agent_model: normalize_stored_agent_model(stored.agent_model),
        agent_model_provider: normalize_stored_agent_model_provider(
            agent_kind,
            stored.agent_model_provider,
        ),
        agent_effort: normalize_stored_agent_effort(agent_kind, stored.agent_effort),
        agent_fast_mode: normalize_stored_agent_fast_mode(agent_kind, stored.agent_fast_mode),
        agent_work_dir: normalize_stored_agent_work_dir(stored.agent_work_dir),
        agent_id,
        session_id,
        activity: WorkspaceActivity::Unknown,
        subagents,
        agent_rows,
        agent_focus,
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

/// Builds the initial visible outline tree for one workspace.
///
/// Example: root with `README.md` -> workspace root containing that file.
fn build_outline(project_root: &Path, attached_outline_dirs: &[PathBuf]) -> Vec<OutlineNode> {
    build_outline_with_state(
        project_root,
        attached_outline_dirs,
        default_outline_dir_states(project_root, attached_outline_dirs),
    )
}

/// Builds an outline while preserving visible directory expansion state.
///
/// Example: `{ "src": false }` -> `src` is kept visible with empty children.
fn build_outline_with_state(
    project_root: &Path,
    attached_outline_dirs: &[PathBuf],
    dir_states: BTreeMap<PathBuf, bool>,
) -> Vec<OutlineNode> {
    let mut nodes = Vec::new();
    let workspace_expanded = outline_dir_expanded(&dir_states, Path::new(""));
    let workspace_children = if workspace_expanded {
        build_workspace_children(project_root, project_root, PathBuf::new(), &dir_states)
    } else {
        Vec::new()
    };
    nodes.push(OutlineNode::Root {
        root_kind: OutlineRootKind::Workspace,
        key: PathBuf::new(),
        label: format!("{} (Workspace Root)", workspace_name(project_root)),
        expanded: workspace_expanded,
        children: workspace_children,
    });

    if let Some(home) = home_dir() {
        let home_key = PathBuf::from(HOME_OUTLINE_ROOT);
        let home_expanded = outline_dir_expanded(&dir_states, &home_key);
        let home_children = if home_expanded {
            build_home_children(&home, &dir_states)
        } else {
            Vec::new()
        };
        if dir_states.contains_key(&home_key) || !home_children.is_empty() {
            nodes.push(OutlineNode::Root {
                root_kind: OutlineRootKind::Home,
                expanded: home_expanded,
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
        let root_expanded = outline_dir_expanded(&dir_states, dir);
        let children = if root_expanded {
            build_attached_dir_children(dir, &dir_states)
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

/// Returns visible directory keys and their current expanded state.
///
/// Example: expanded `src` -> map entry `src: true`.
fn outline_dir_states(nodes: &[OutlineNode]) -> BTreeMap<PathBuf, bool> {
    let mut states = BTreeMap::new();
    collect_outline_dir_states(nodes, &mut states);
    states
}

/// Recursively collects only nodes already visible in the outline tree.
///
/// Example: hidden child nodes -> no entries are inserted for them.
fn collect_outline_dir_states(nodes: &[OutlineNode], states: &mut BTreeMap<PathBuf, bool>) {
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
                states.insert(key.clone(), *is_expanded);
                collect_outline_dir_states(children, states);
            }
            OutlineNode::File { .. } => {}
        }
    }
}

/// Returns a visible directory's stored expansion flag.
///
/// Example: missing key -> `false`.
fn outline_dir_expanded(states: &BTreeMap<PathBuf, bool>, key: &Path) -> bool {
    states.get(key).copied().unwrap_or(false)
}

/// Builds visible Markdown nodes directly under one workspace directory.
///
/// Example: collapsed `src` -> `src` node with no scanned descendants.
fn build_workspace_children(
    project_root: &Path,
    current_dir: &Path,
    relative_dir: PathBuf,
    dir_states: &BTreeMap<PathBuf, bool>,
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
            let is_expanded = outline_dir_expanded(dir_states, &child_relative);
            // 触发条件：文件监听刷新 outline 时遇到折叠目录。
            // 不能递归探测：大仓库会把不可见路径也装进 tree。
            // 防止刷新后残留途径目录和不可见文件节点。
            let children = if is_expanded {
                build_workspace_children(project_root, &path, child_relative.clone(), dir_states)
            } else {
                Vec::new()
            };
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
            nodes.push(OutlineNode::File {
                path: relative,
                label: name,
            });
        }
    }
    nodes
}

/// Builds visible Markdown nodes under the configured home outline root.
///
/// Example: collapsed `~/.codex` -> `~/.codex` node with empty children.
fn build_home_children(home: &Path, dir_states: &BTreeMap<PathBuf, bool>) -> Vec<OutlineNode> {
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
        children.push(OutlineNode::File { path, label });
    }

    for dir_name in HOME_OUTLINE_DIRS {
        let dir = home.join(dir_name);
        if !dir.is_dir() {
            continue;
        }
        let key = PathBuf::from(HOME_OUTLINE_ROOT).join(dir_name);
        let is_expanded = outline_dir_expanded(dir_states, &key);
        // Trigger: status/outline refresh keeps home roots visible.
        // Why not probe recursively: ~/.codex and peers can be very large.
        // Prevents: a closed home outline section from scanning all sessions.
        let dir_children = if is_expanded {
            build_home_dir_children(&dir, key.clone(), dir_states)
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

/// Builds visible Markdown nodes inside an expanded home outline directory.
///
/// Example: collapsed nested dir -> nested dir is listed without descendants.
fn build_home_dir_children(
    current_dir: &Path,
    key_dir: PathBuf,
    dir_states: &BTreeMap<PathBuf, bool>,
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
            let is_expanded = outline_dir_expanded(dir_states, &child_key);
            // Trigger: home outline directories are refreshed periodically.
            // Why not recurse collapsed dirs: agent session folders fan out.
            // Prevents: closed outline branches from dominating allocations.
            let children = if is_expanded {
                build_home_dir_children(&path, child_key.clone(), dir_states)
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

/// Builds visible Markdown nodes under an attached absolute directory.
///
/// Example: collapsed attached child -> child node with no scanned descendants.
fn build_attached_dir_children(
    current_dir: &Path,
    dir_states: &BTreeMap<PathBuf, bool>,
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
            let is_expanded = outline_dir_expanded(dir_states, &path);
            let children = if is_expanded {
                build_attached_dir_children(&path, dir_states)
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

/// Returns the startup expansion map used before the user changes outline state.
///
/// Example: workspace first-level dirs -> inserted as expanded.
fn default_outline_dir_states(
    root: &Path,
    attached_outline_dirs: &[PathBuf],
) -> BTreeMap<PathBuf, bool> {
    let mut states = BTreeMap::new();
    states.insert(PathBuf::new(), true);
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.filter_map(|entry| entry.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            if entry
                .file_type()
                .map(|file_type| file_type.is_dir())
                .unwrap_or(false)
                && !should_skip_outline_dir(root, &name)
            {
                states.insert(PathBuf::from(name), true);
            }
        }
    }
    states.insert(PathBuf::from(HOME_OUTLINE_ROOT), true);
    for dir_name in HOME_OUTLINE_DIRS {
        states.insert(PathBuf::from(HOME_OUTLINE_ROOT).join(dir_name), true);
    }
    for dir in attached_outline_dirs {
        states.insert(dir.clone(), true);
    }
    states
}

/// Returns a compact label for an attached outline root.
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

/// 返回 workspace 路径末尾最多两段作为紧凑名称。
///
/// 适用场景：Local Rail 与 Remote Outline 共享命名。例：`/a/b/c` -> `b/c`。
pub fn workspace_name(project_dir: &Path) -> String {
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

#[cfg(test)]
#[path = "data_test.rs"]
mod data_test;

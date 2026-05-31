#[allow(dead_code, unused_imports, unused_variables)]
#[path = "../ai/mod.rs"]
mod ai;
#[allow(dead_code, unused_imports, unused_variables)]
#[path = "../gui/mod.rs"]
mod gui;
#[allow(dead_code, unused_imports, unused_variables)]
#[path = "../home.rs"]
mod home;
#[allow(dead_code, unused_imports, unused_variables)]
#[path = "../reviewer/mod.rs"]
mod reviewer;
#[allow(dead_code, unused_imports, unused_variables)]
#[path = "../scrolling.rs"]
mod scrolling;

use anyhow::{Context, Result};
use serde_json::Map;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

const GSDV_WF_SKILL: &str = include_str!("../../assets/skills/gsdv-wf/SKILL.md");
const CODEX_AGENT_STATUS_HOOK_EVENTS: [&str; 3] = ["SessionStart", "UserPromptSubmit", "Stop"];
const CODEX_AGENT_STATUS_HOOK_TIMEOUT: u64 = 5;
const CODEX_AGENT_STATUS_MESSAGE: &str = "updating gsdv agent status";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchInfo {
    pub label: String,
    pub checkout: BranchCheckout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchCheckout {
    Local(String),
    Remote { remote: String, local: String },
}

#[allow(dead_code)]
pub(crate) fn debug_log(_args: std::fmt::Arguments<'_>) {}

fn main() -> eframe::Result<()> {
    let first_arg = std::env::args().nth(1);
    if first_arg.as_deref() == Some("agent-status-hook") {
        if let Err(error) = gui::agent_status_hook::run_from_stdin() {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Err(error) = install_self() {
        eprintln!("failed to install gsdv integration: {error:#}");
        std::process::exit(1);
    }
    gui::run()
}

fn install_self() -> Result<()> {
    let current = std::env::current_exe().context("failed to resolve current executable")?;
    install_agent_status_hook(&current)?;
    install_codex_hooks()?;
    install_claude_hooks()?;
    install_agent_skills()?;
    Ok(())
}

fn install_agent_status_hook(gsdv_exe: &Path) -> Result<()> {
    let hook_dir = gsdv_home_dir()?.join("hooks");
    fs::create_dir_all(&hook_dir)
        .with_context(|| format!("failed to create {}", hook_dir.display()))?;
    let hook_path = hook_dir.join("agent-status-hook");
    fs::write(&hook_path, agent_status_hook_script(gsdv_exe))
        .with_context(|| format!("failed to write {}", hook_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&hook_path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&hook_path, permissions)?;
    }
    Ok(())
}

fn agent_status_hook_script(gsdv_exe: &Path) -> String {
    let gsdv_exe = shell_quote(&gsdv_exe.to_string_lossy());
    format!("#!/bin/sh\nexec {gsdv_exe} agent-status-hook\n")
}

/// 安装 Codex hooks，并把 gsdv 自己写入的 hook 标记为已信任。
fn install_codex_hooks() -> Result<()> {
    let codex_dir = home_dir().context("HOME is not set")?.join(".codex");
    fs::create_dir_all(&codex_dir)
        .with_context(|| format!("failed to create {}", codex_dir.display()))?;
    let config_path = codex_dir.join("config.toml");
    let hooks_path = codex_dir.join("hooks.json");
    enable_codex_hooks_feature(&config_path)?;
    let trust_entries = install_codex_hooks_json(&hooks_path)?;
    trust_installed_codex_hooks(&config_path, &trust_entries)?;
    Ok(())
}

/// 开启 Codex hooks feature，兼容并清理旧版 codex_hooks 别名。
fn enable_codex_hooks_feature(config_path: &Path) -> Result<()> {
    let content = fs::read_to_string(config_path).unwrap_or_default();
    let updated = set_toml_feature_bool_removing_aliases(&content, "hooks", true, &["codex_hooks"]);
    if updated != content {
        fs::write(config_path, updated)
            .with_context(|| format!("failed to write {}", config_path.display()))?;
    }
    Ok(())
}

/// 写入 Codex feature 开关，并移除已经废弃的同义 key。
fn set_toml_feature_bool_removing_aliases(
    content: &str,
    key: &str,
    value: bool,
    removed_keys: &[&str],
) -> String {
    let target = format!("{key} = {value}");
    let mut lines = content.lines().map(str::to_string).collect::<Vec<_>>();
    let mut in_features = false;
    let mut saw_features = false;
    let mut wrote = false;
    let mut insert_at = None;

    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed == "[features]" {
            in_features = true;
            saw_features = true;
            index += 1;
            continue;
        }
        if in_features && trimmed.starts_with('[') && trimmed.ends_with(']') {
            insert_at = Some(index);
            break;
        }
        if in_features && toml_line_assigns_key(trimmed, key) {
            lines[index] = target.clone();
            wrote = true;
            index += 1;
            continue;
        }
        if in_features
            && removed_keys
                .iter()
                .any(|removed_key| toml_line_assigns_key(trimmed, removed_key))
        {
            // 触发条件：Codex CLI 0.129.0 起把 codex_hooks 改名为 hooks。
            // 不能保留旧 key：0.130.0 会持续输出 deprecated 警告。
            // 防止回归：启动 gsdv 后 Codex 仍提示旧 feature 配置。
            lines.remove(index);
            continue;
        }
        index += 1;
    }

    if !wrote {
        if saw_features {
            lines.insert(insert_at.unwrap_or(lines.len()), target);
        } else {
            if !lines.is_empty() && lines.last().is_some_and(|line| !line.is_empty()) {
                lines.push(String::new());
            }
            lines.push("[features]".to_string());
            lines.push(target);
        }
    }

    let mut updated = lines.join("\n");
    updated.push('\n');
    updated
}

/// 判断一行 TOML 是否是指定 key 的赋值。
fn toml_line_assigns_key(trimmed: &str, key: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix(key) else {
        return false;
    };
    rest.trim_start().starts_with('=')
}

/// 写入 Codex hooks.json，并返回这些 hook 对应的信任状态条目。
fn install_codex_hooks_json(hooks_path: &Path) -> Result<Vec<CodexHookTrustEntry>> {
    let hook_path = gsdv_home_dir()?.join("hooks").join("agent-status-hook");
    let command = shell_quote(&hook_path.to_string_lossy());
    let content = fs::read_to_string(hooks_path).unwrap_or_default();
    let mut root = serde_json::from_str::<Value>(&content).unwrap_or_else(|_| {
        serde_json::json!({
            "hooks": {}
        })
    });
    if !root.is_object() {
        root = serde_json::json!({
            "hooks": {}
        });
    }
    if !root.get("hooks").is_some_and(Value::is_object) {
        root["hooks"] = serde_json::json!({});
    }
    let mut trust_entries = Vec::new();
    for event in CODEX_AGENT_STATUS_HOOK_EVENTS {
        let group_index = install_codex_hook_event(&mut root, event, None, &command);
        trust_entries.push(CodexHookTrustEntry::new(
            hooks_path,
            event,
            group_index,
            0,
            &command,
        ));
    }
    for event in ["PreToolUse", "PostToolUse"] {
        remove_codex_hook_event(&mut root, event);
    }
    let updated = serde_json::to_string_pretty(&root)? + "\n";
    if updated != content {
        fs::write(hooks_path, updated)
            .with_context(|| format!("failed to write {}", hooks_path.display()))?;
    }
    Ok(trust_entries)
}

/// 把启动安装的 Codex hook 当前定义写入 hooks.state。
fn trust_installed_codex_hooks(config_path: &Path, entries: &[CodexHookTrustEntry]) -> Result<()> {
    let content = fs::read_to_string(config_path).unwrap_or_default();
    let updated = set_toml_hook_trusted_hashes(&content, entries);
    if updated != content {
        fs::write(config_path, updated)
            .with_context(|| format!("failed to write {}", config_path.display()))?;
    }
    Ok(())
}

/// 更新 TOML 中 gsdv 自己维护的 Codex hook 信任 hash。
fn set_toml_hook_trusted_hashes(content: &str, entries: &[CodexHookTrustEntry]) -> String {
    if entries.is_empty() {
        return ensure_trailing_newline(content);
    }

    let mut lines = content.lines().map(str::to_string).collect::<Vec<_>>();
    let mut in_hooks_state = false;
    let mut saw_hooks_state = false;
    let mut insert_at = None;
    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed == "[hooks.state]" {
            in_hooks_state = true;
            saw_hooks_state = true;
            index += 1;
            continue;
        }
        if in_hooks_state && trimmed.starts_with('[') && trimmed.ends_with(']') {
            insert_at = Some(index);
            break;
        }
        if in_hooks_state
            && entries
                .iter()
                .any(|entry| toml_line_assigns_quoted_key(trimmed, &entry.escaped_key()))
        {
            lines.remove(index);
            continue;
        }
        index += 1;
    }

    let trust_lines = entries
        .iter()
        .map(|entry| {
            format!(
                "\"{}\" = {{ trusted_hash = \"{}\" }}",
                entry.escaped_key(),
                entry.trusted_hash
            )
        })
        .collect::<Vec<_>>();
    if saw_hooks_state {
        let at = insert_at.unwrap_or(lines.len());
        for (offset, line) in trust_lines.into_iter().enumerate() {
            lines.insert(at + offset, line);
        }
    } else {
        if !lines.is_empty() && lines.last().is_some_and(|line| !line.is_empty()) {
            lines.push(String::new());
        }
        lines.push("[hooks.state]".to_string());
        lines.extend(trust_lines);
    }

    let mut updated = lines.join("\n");
    updated.push('\n');
    updated
}

/// 判断一行 TOML 是否是指定双引号 key 的赋值。
fn toml_line_assigns_quoted_key(trimmed: &str, escaped_key: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix('"') else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(escaped_key) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix('"') else {
        return false;
    };
    rest.trim_start().starts_with('=')
}

/// Codex 单个 hook handler 的信任状态写入项。
#[derive(Debug, Clone, PartialEq, Eq)]
struct CodexHookTrustEntry {
    /// Codex 持久化到 hooks.state 的 handler key。
    key: String,
    /// 当前 handler 规范化定义对应的 sha256 hash。
    trusted_hash: String,
}

impl CodexHookTrustEntry {
    /// 基于 Codex 持久化 key 和当前 hook 定义创建信任条目。
    fn new(
        hooks_path: &Path,
        event: &str,
        group_index: usize,
        handler_index: usize,
        command: &str,
    ) -> Self {
        let event_key = codex_hook_event_key(event).expect("known codex hook event");
        Self {
            key: format!(
                "{}:{event_key}:{group_index}:{handler_index}",
                hooks_path.display()
            ),
            trusted_hash: codex_command_hook_hash(
                event_key,
                None,
                command,
                CODEX_AGENT_STATUS_HOOK_TIMEOUT,
                Some(CODEX_AGENT_STATUS_MESSAGE),
            ),
        }
    }

    /// 返回可放进 TOML 双引号 key 的转义文本。
    fn escaped_key(&self) -> String {
        toml_basic_string_escape(&self.key)
    }
}

/// 返回 Codex 用于 hooks.state key 和 hash identity 的事件名。
fn codex_hook_event_key(event: &str) -> Option<&'static str> {
    match event {
        "PreToolUse" => Some("pre_tool_use"),
        "PermissionRequest" => Some("permission_request"),
        "PostToolUse" => Some("post_tool_use"),
        "PreCompact" => Some("pre_compact"),
        "PostCompact" => Some("post_compact"),
        "SessionStart" => Some("session_start"),
        "UserPromptSubmit" => Some("user_prompt_submit"),
        "Stop" => Some("stop"),
        _ => None,
    }
}

/// 按 Codex 的 normalized hook identity 规则计算 command hook hash。
fn codex_command_hook_hash(
    event_key: &str,
    matcher: Option<&str>,
    command: &str,
    timeout: u64,
    status_message: Option<&str>,
) -> String {
    let mut hook = Map::new();
    hook.insert("async".to_string(), Value::Bool(false));
    hook.insert("command".to_string(), Value::String(command.to_string()));
    if let Some(status_message) = status_message {
        hook.insert(
            "statusMessage".to_string(),
            Value::String(status_message.to_string()),
        );
    }
    hook.insert("timeout".to_string(), Value::from(timeout));
    hook.insert("type".to_string(), Value::String("command".to_string()));

    let mut identity = Map::new();
    identity.insert(
        "event_name".to_string(),
        Value::String(event_key.to_string()),
    );
    identity.insert("hooks".to_string(), Value::Array(vec![Value::Object(hook)]));
    if let Some(matcher) = matcher {
        identity.insert("matcher".to_string(), Value::String(matcher.to_string()));
    }
    version_for_json(&Value::Object(identity))
}

/// 复刻 Codex config::version_for_toml 的 JSON 排序和 sha256 输出格式。
fn version_for_json(value: &Value) -> String {
    let canonical = canonical_json(value);
    let serialized = serde_json::to_vec(&canonical).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(serialized);
    let hash = hasher.finalize();
    let hex = hash
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{hex}")
}

/// 递归排序 JSON object key，保证 hash 与 Codex 稳定规则一致。
fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = Map::new();
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                if let Some(value) = map.get(&key) {
                    sorted.insert(key, canonical_json(value));
                }
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_json).collect()),
        other => other.clone(),
    }
}

/// 转义 TOML basic string 中不能直接出现的字符。
fn toml_basic_string_escape(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character => escaped.push(character),
        }
    }
    escaped
}

/// Installs user-level Claude Code hooks for gsdv agent status tracking.
fn install_claude_hooks() -> Result<()> {
    let claude_dir = home_dir().context("HOME is not set")?.join(".claude");
    fs::create_dir_all(&claude_dir)
        .with_context(|| format!("failed to create {}", claude_dir.display()))?;
    install_claude_settings_hooks(&claude_dir.join("settings.json"))
}

/// 安装 gsdv 内置 skill 到支持的 agent 目录。
fn install_agent_skills() -> Result<()> {
    let home = home_dir().context("HOME is not set")?;
    install_skill(
        &home.join(".codex").join("skills").join("gsdv-wf"),
        GSDV_WF_SKILL,
    )?;
    install_skill(
        &home.join(".claude").join("skills").join("gsdv-wf"),
        GSDV_WF_SKILL,
    )?;
    Ok(())
}

/// 写入单个 agent skill 目录。
fn install_skill(skill_dir: &Path, skill_content: &str) -> Result<()> {
    fs::create_dir_all(skill_dir)
        .with_context(|| format!("failed to create {}", skill_dir.display()))?;
    let skill_path = skill_dir.join("SKILL.md");
    let content = ensure_trailing_newline(skill_content);
    // 触发条件：应用启动时安装内置 skill。
    // 不能按内容相同跳过：用户可能手改了目标文件或权限状态。
    // 防止启动后 agent 继续读到旧版/魔改版内置 skill。
    fs::write(&skill_path, content)
        .with_context(|| format!("failed to write {}", skill_path.display()))?;
    Ok(())
}

/// 确保嵌入文本写入后以换行结尾。
fn ensure_trailing_newline(content: &str) -> String {
    if content.ends_with('\n') {
        content.to_string()
    } else {
        format!("{content}\n")
    }
}

/// Updates ~/.claude/settings.json without disturbing unrelated settings.
fn install_claude_settings_hooks(settings_path: &Path) -> Result<()> {
    let hook_path = gsdv_home_dir()?.join("hooks").join("agent-status-hook");
    let command = shell_quote(&hook_path.to_string_lossy());
    let content = fs::read_to_string(settings_path).unwrap_or_default();
    let mut root =
        serde_json::from_str::<Value>(&content).unwrap_or_else(|_| serde_json::json!({}));
    if !root.is_object() {
        root = serde_json::json!({});
    }
    if !root.get("hooks").is_some_and(Value::is_object) {
        root["hooks"] = serde_json::json!({});
    }
    for event in ["SessionStart", "UserPromptSubmit", "Stop", "SessionEnd"] {
        install_claude_hook_event(&mut root, event, None, &command);
    }
    for event in ["PreToolUse", "PostToolUse"] {
        install_claude_hook_event(
            &mut root,
            event,
            Some("Write|Edit|MultiEdit|NotebookEdit"),
            &command,
        );
    }
    let updated = serde_json::to_string_pretty(&root)? + "\n";
    if updated != content {
        fs::write(settings_path, updated)
            .with_context(|| format!("failed to write {}", settings_path.display()))?;
    }
    Ok(())
}

/// Adds one Claude Code command hook group for an event.
fn install_claude_hook_event(root: &mut Value, event: &str, matcher: Option<&str>, command: &str) {
    let hooks = root
        .get_mut("hooks")
        .and_then(Value::as_object_mut)
        .expect("hooks object exists");
    let entry = hooks
        .entry(event.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !entry.is_array() {
        *entry = Value::Array(Vec::new());
    }
    let groups = entry.as_array_mut().expect("event hooks are an array");
    for group in groups.iter_mut() {
        if let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) {
            hooks.retain(|hook| !is_gsdv_agent_status_hook(hook));
        }
    }
    groups.retain(|group| {
        group
            .get("hooks")
            .and_then(Value::as_array)
            .is_some_and(|hooks| !hooks.is_empty())
    });
    let mut group = serde_json::json!({
        "hooks": [{
            "type": "command",
            "command": command,
            "timeout": 5
        }]
    });
    if let Some(matcher) = matcher {
        group["matcher"] = Value::from(matcher);
    }
    groups.push(group);
}

/// 写入单个 Codex command hook，并返回新 group 的下标。
fn install_codex_hook_event(
    root: &mut Value,
    event: &str,
    matcher: Option<&str>,
    command: &str,
) -> usize {
    let hooks = root
        .get_mut("hooks")
        .and_then(Value::as_object_mut)
        .expect("hooks object exists");
    let entry = hooks
        .entry(event.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !entry.is_array() {
        *entry = Value::Array(Vec::new());
    }
    let groups = entry.as_array_mut().expect("event hooks are an array");
    for group in groups.iter_mut() {
        if let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) {
            hooks.retain(|hook| !is_gsdv_agent_status_hook(hook));
        }
    }
    groups.retain(|group| {
        group
            .get("hooks")
            .and_then(Value::as_array)
            .is_some_and(|hooks| !hooks.is_empty())
    });
    let mut group = serde_json::json!({
    "hooks": [{
        "type": "command",
        "command": command,
        "timeout": 5,
        "statusMessage": "updating gsdv agent status"
    }]
    });
    if let Some(matcher) = matcher {
        group["matcher"] = Value::from(matcher);
    }
    let group_index = groups.len();
    groups.push(group);
    group_index
}

/// 移除 Codex 指定事件里的 gsdv 状态 hook。
fn remove_codex_hook_event(root: &mut Value, event: &str) {
    let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) else {
        return;
    };
    let should_remove_event = {
        let Some(groups) = hooks.get_mut(event).and_then(Value::as_array_mut) else {
            return;
        };
        for group in groups.iter_mut() {
            if let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                hooks.retain(|hook| !is_gsdv_agent_status_hook(hook));
            }
        }
        groups.retain(|group| {
            group
                .get("hooks")
                .and_then(Value::as_array)
                .is_some_and(|hooks| !hooks.is_empty())
        });
        groups.is_empty()
    };
    if should_remove_event {
        hooks.remove(event);
    }
}

fn is_gsdv_agent_status_hook(hook: &Value) -> bool {
    hook.get("type").and_then(Value::as_str) == Some("command")
        && hook
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(|command| command.contains("agent-status-hook"))
}

fn gsdv_home_dir() -> Result<PathBuf> {
    Ok(home_dir().context("HOME is not set")?.join(".gsdv"))
}

fn home_dir() -> Option<PathBuf> {
    home::home_dir()
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
#[path = "gsdv_gui_test.rs"]
mod gsdv_gui_test;

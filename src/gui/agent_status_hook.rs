use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs::{self, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookStatus {
    Idle,
    Busy,
}

pub fn run_from_stdin() -> Result<()> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("failed to read hook stdin")?;
    run_with_input(&input)
}

fn run_with_input(input: &str) -> Result<()> {
    let payload = serde_json::from_str::<Value>(input).unwrap_or(Value::Null);
    let Ok(agent_id) = env::var("GSDV_AGENT_ID") else {
        return Ok(());
    };
    let agent_id = agent_id.trim();
    if agent_id.is_empty() {
        return Ok(());
    }
    let agent_kind = env::var("GSDV_AGENT_KIND").unwrap_or_else(|_| "unknown".to_string());
    let workspace_dir = env::var("GSDV_WORKSPACE_DIR").unwrap_or_default();
    let event = payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let status = match event {
        "SessionStart" | "Stop" | "SessionEnd" => HookStatus::Idle,
        "UserPromptSubmit" => HookStatus::Busy,
        _ => return Ok(()),
    };
    write_agent_status(agent_id, &agent_kind, &workspace_dir, status, &payload)?;
    notify_agent_status(agent_id, &agent_kind, &workspace_dir, status, &payload);
    sync_store_session_from_start(agent_id, &agent_kind, &workspace_dir, &payload)
}

/// 通过 app hook socket/pipe 直接通知 GUI 状态变化。
fn notify_agent_status(
    agent_id: &str,
    agent_kind: &str,
    workspace_dir: &str,
    status: HookStatus,
    payload: &Value,
) {
    let status = match status {
        HookStatus::Idle => "idle",
        HookStatus::Busy => "busy",
    };
    let data = json!({
        "agent_id": agent_id,
        "agent": agent_kind,
        "workspace": workspace_dir,
        "status": status,
        "session_id": payload.get("session_id").and_then(Value::as_str).unwrap_or(""),
        "turn_id": payload.get("turn_id").and_then(Value::as_str).unwrap_or(""),
        "transcript_path": payload.get("transcript_path").and_then(Value::as_str).unwrap_or(""),
        "hook_event_name": payload.get("hook_event_name").and_then(Value::as_str).unwrap_or(""),
    });
    let endpoint =
        env::var("GSDV_HOOK_ENDPOINT").unwrap_or_else(|_| crate::gui::hook::app_hook_endpoint());
    let _ = crate::gui::hook::send_hook_event(
        &endpoint,
        crate::gui::hook::AGENT_STATUS_KEY,
        &data.to_string(),
    );
}

fn write_agent_status(
    agent_id: &str,
    agent_kind: &str,
    workspace_dir: &str,
    status: HookStatus,
    payload: &Value,
) -> Result<()> {
    let Some(path) = agent_status_path() else {
        return Ok(());
    };
    write_agent_status_at(&path, agent_id, agent_kind, workspace_dir, status, payload)
}

fn write_agent_status_at(
    path: &Path,
    agent_id: &str,
    agent_kind: &str,
    workspace_dir: &str,
    status: HookStatus,
    payload: &Value,
) -> Result<()> {
    let Some(session_id) = payload
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let mut root = serde_json::from_str::<Value>(&content).unwrap_or_else(|_| {
        json!({
            "version": 2,
            "sessions": {}
        })
    });
    if !root.is_object() {
        root = json!({
            "version": 2,
            "sessions": {}
        });
    }
    root["version"] = Value::from(2);
    if !root.get("sessions").is_some_and(Value::is_object) {
        root["sessions"] = json!({});
    }
    let event = payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let status_value = match status {
        HookStatus::Idle => "idle",
        HookStatus::Busy => "busy",
    };
    let timestamp = now_ms();
    let started_at_ms = if event == "SessionStart" {
        timestamp
    } else {
        root.get("sessions")
            .and_then(|sessions| sessions.get(session_id))
            .and_then(|entry| entry.get("started_at_ms"))
            .and_then(Value::as_u64)
            .map(u128::from)
            .unwrap_or(timestamp)
    };
    root["sessions"][session_id] = json!({
        "status": status_value,
        "agent": agent_kind,
        "agent_id": agent_id,
        "workspace": workspace_dir,
        "session_id": session_id,
        "turn_id": payload.get("turn_id").and_then(Value::as_str).unwrap_or(""),
        "transcript_path": payload.get("transcript_path").and_then(Value::as_str).unwrap_or(""),
        "last_event": event,
        "last_source": payload.get("source").and_then(Value::as_str).unwrap_or(""),
        "started_at_ms": started_at_ms,
        "updated_at_ms": timestamp,
    });
    let serialized = serde_json::to_vec_pretty(&root)?;
    file.seek(SeekFrom::Start(0))?;
    file.set_len(0)?;
    file.write_all(&serialized)?;
    file.write_all(b"\n")?;
    file.sync_data()?;
    Ok(())
}

fn sync_store_session_from_start(
    agent_id: &str,
    agent_kind: &str,
    workspace_dir: &str,
    payload: &Value,
) -> Result<()> {
    if payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        != "SessionStart"
    {
        return Ok(());
    }
    let Some(session_id) = payload
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(());
    };
    if workspace_dir.trim().is_empty() {
        return Ok(());
    }
    let Some(path) = gsdv_store_path() else {
        return Ok(());
    };
    sync_store_session_at(&path, agent_id, agent_kind, workspace_dir, session_id)
}

fn sync_store_session_at(
    path: &Path,
    agent_id: &str,
    agent_kind: &str,
    workspace_dir: &str,
    session_id: &str,
) -> Result<()> {
    let content = fs::read_to_string(path).unwrap_or_default();
    let mut root = serde_json::from_str::<Value>(&content).unwrap_or_else(|_| json!({}));
    let Some(workspaces) = root.get_mut("workspaces").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    let workspace_path = normalize_path(Path::new(workspace_dir));
    let mut changed = false;
    for workspace in workspaces {
        let Some(workspace_object) = workspace.as_object_mut() else {
            continue;
        };
        let Some(stored_path) = workspace_object
            .get("path")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .map(|path| normalize_path(&path))
        else {
            continue;
        };
        if stored_path != workspace_path {
            continue;
        }
        if sync_subagent_session_at(path, agent_id, agent_kind, &stored_path, session_id)? {
            continue;
        }
        if workspace_object
            .get("agent_kind")
            .and_then(Value::as_str)
            .is_some_and(|stored| !stored.eq_ignore_ascii_case(agent_kind))
        {
            continue;
        }
        if workspace_object
            .get("agent_id")
            .and_then(Value::as_str)
            .is_some_and(|stored| !stored.is_empty() && stored != agent_id)
        {
            continue;
        }
        if workspace_object.get("agent_id").and_then(Value::as_str) != Some(agent_id) {
            workspace_object.insert("agent_id".to_string(), Value::from(agent_id));
            changed = true;
        }
        if workspace_object.get("session_id").and_then(Value::as_str) != Some(session_id) {
            workspace_object.insert("session_id".to_string(), Value::from(session_id));
            changed = true;
        }
    }
    if changed {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let content = serde_json::to_string_pretty(&root)?;
        fs::write(path, format!("{content}\n"))
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    Ok(())
}

/// Mirrors a SessionStart id into the matching subagent sidecar entry.
fn sync_subagent_session_at(
    store_path: &Path,
    agent_id: &str,
    agent_kind: &str,
    workspace_path: &Path,
    session_id: &str,
) -> Result<bool> {
    let Some(parent) = store_path.parent() else {
        return Ok(false);
    };
    let path = parent
        .join("workspaces")
        .join(format!("{:x}", stable_state_key(workspace_path)))
        .join("subagents.json");
    let content = fs::read_to_string(&path).unwrap_or_default();
    let mut root = serde_json::from_str::<Value>(&content).unwrap_or_else(|_| json!({}));
    let Some(subagents) = root.get_mut("subagents").and_then(Value::as_array_mut) else {
        return Ok(false);
    };
    let mut matched = false;
    let mut changed = false;
    for subagent in subagents {
        let Some(subagent) = subagent.as_object_mut() else {
            continue;
        };
        if subagent.get("agent_id").and_then(Value::as_str) != Some(agent_id) {
            continue;
        }
        matched = true;
        if subagent
            .get("agent_kind")
            .and_then(Value::as_str)
            .is_some_and(|stored| !stored.eq_ignore_ascii_case(agent_kind))
        {
            continue;
        }
        if subagent.get("session_id").and_then(Value::as_str) != Some(session_id) {
            subagent.insert("session_id".to_string(), Value::from(session_id));
            changed = true;
        }
    }
    if changed {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let content = serde_json::to_string_pretty(&root)?;
        fs::write(&path, format!("{content}\n"))
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    Ok(matched)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn normalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn stable_state_key(path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    normalize_path(path).to_string_lossy().hash(&mut hasher);
    hasher.finish()
}

fn home_dir() -> Option<PathBuf> {
    crate::home::home_dir()
}

fn gsdv_store_path() -> Option<PathBuf> {
    env::var_os("GSDV_STORE_PATH")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".gsdv").join("store")))
}

fn agent_status_path() -> Option<PathBuf> {
    env::var_os("GSDV_AGENT_STATUS_PATH")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|home| home.join(".gsdv").join("agent-status.json")))
}

#[cfg(test)]
#[path = "agent_status_hook_test.rs"]
mod agent_status_hook_test;

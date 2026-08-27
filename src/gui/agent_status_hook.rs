use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::env;
use std::io::Read;

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
    notify_agent_status(agent_id, &workspace_dir, status, &payload);
    Ok(())
}

/// 通过 app hook socket/pipe 直接通知 GUI 状态变化。
fn notify_agent_status(agent_id: &str, workspace_dir: &str, status: HookStatus, payload: &Value) {
    let status = match status {
        HookStatus::Idle => "idle",
        HookStatus::Busy => "busy",
    };
    let data = json!({
        "agent_id": agent_id,
        "workspace": workspace_dir,
        "status": status,
        "session_id": payload.get("session_id").and_then(Value::as_str).unwrap_or(""),
    });
    let endpoint =
        env::var("GSDV_HOOK_ENDPOINT").unwrap_or_else(|_| crate::gui::hook::app_hook_endpoint());
    let _ = crate::gui::hook::send_hook_event(
        &endpoint,
        crate::gui::hook::AGENT_STATUS_KEY,
        &data.to_string(),
    );
}

#[cfg(test)]
#[path = "agent_status_hook_test.rs"]
mod agent_status_hook_test;

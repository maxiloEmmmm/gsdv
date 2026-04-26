use super::*;
#[test]
fn session_start_updates_matching_store_workspace() {
    let root = test_root("session_start_store");
    let workspace = root.join("repo");
    fs::create_dir_all(&workspace).unwrap();
    let store = root.join("store.json");
    let status = root.join("status.json");
    fs::write(
        &store,
        format!(
            r#"{{
                    "active": 0,
                    "workspaces": [{{
                        "path": "{}",
                        "agent_kind": "codex",
                        "agent_id": "agent",
                        "session_id": "old-session"
                    }}]
                }}"#,
            workspace.display()
        ),
    )
    .unwrap();
    let payload = serde_json::from_str::<Value>(
        r#"{"hook_event_name":"SessionStart","source":"clear","session_id":"new-session"}"#,
    )
    .unwrap();

    write_agent_status_at(
        &status,
        "agent",
        "codex",
        &workspace.to_string_lossy(),
        HookStatus::Idle,
        &payload,
    )
    .unwrap();
    sync_store_session_at(
        &store,
        "agent",
        "codex",
        &workspace.to_string_lossy(),
        "new-session",
    )
    .unwrap();

    let store_content = fs::read_to_string(&store).unwrap();
    assert!(store_content.contains(r#""session_id": "new-session""#));
    let status_content = fs::read_to_string(&status).unwrap();
    assert!(status_content.contains(r#""new-session""#));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn non_session_start_event_does_not_roll_back_session_after_clear() {
    let root = test_root("non_start_no_store_rollback");
    let workspace = root.join("repo");
    fs::create_dir_all(&workspace).unwrap();
    let store = root.join("store.json");
    let status = root.join("status.json");
    fs::write(
        &store,
        format!(
            r#"{{
                    "active": 0,
                    "workspaces": [{{
                        "path": "{}",
                        "agent_kind": "codex",
                        "agent_id": "agent",
                        "session_id": "new-session"
                    }}]
                }}"#,
            workspace.display()
        ),
    )
    .unwrap();
    let payload = serde_json::from_str::<Value>(
        r#"{"hook_event_name":"Stop","session_id":"old-session","turn_id":"turn"}"#,
    )
    .unwrap();

    write_agent_status_at(
        &status,
        "agent",
        "codex",
        &workspace.to_string_lossy(),
        HookStatus::Idle,
        &payload,
    )
    .unwrap();

    let store_content = fs::read_to_string(&store).unwrap();
    assert!(store_content.contains(r#""session_id": "new-session""#));
    let status_content = fs::read_to_string(&status).unwrap();
    assert!(status_content.contains(r#""old-session""#));

    let _ = fs::remove_dir_all(root);
}

fn test_root(name: &str) -> PathBuf {
    let root = env::temp_dir().join(format!("gsdv-agent-hook-{name}-{}", now_ms()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

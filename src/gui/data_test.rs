use super::*;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn load_initial_gui_data_allows_zero_workspaces() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("zero_workspaces");
    let store = root.join("store.json");
    fs::write(&store, r#"{"active":0,"workspaces":[]}"#).unwrap();
    unsafe {
        env::set_var("GSDV_STORE_PATH", &store);
    }

    let data = load_initial_gui_data(AgentKind::Codex);

    assert!(data.workspaces.is_empty());
    assert_eq!(data.active_workspace, 0);

    unsafe {
        env::remove_var("GSDV_STORE_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn default_agent_kind_round_trips_through_store() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("default_agent_kind");
    let store = root.join("store.json");
    unsafe {
        env::set_var("GSDV_STORE_PATH", &store);
    }

    assert_eq!(load_default_agent_kind(), None);

    save_default_agent_kind(AgentKind::Claude);

    assert_eq!(load_default_agent_kind(), Some(AgentKind::Claude));

    unsafe {
        env::remove_var("GSDV_STORE_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

/// 验证 workspace terminal 历史按 workspace 粒度独立读写。
#[test]
fn workspace_terminal_history_round_trips_by_workspace() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("workspace_terminal_history");
    let store = root.join("store.json");
    let workspace = root.join("repo");
    unsafe {
        env::set_var("GSDV_STORE_PATH", &store);
    }

    save_workspace_terminal_history(&workspace, "pwd\n/tmp/repo").unwrap();

    assert_eq!(
        load_workspace_terminal_history(&workspace),
        "pwd\n/tmp/repo"
    );

    delete_workspace_terminal_history(&workspace).unwrap();
    assert_eq!(load_workspace_terminal_history(&workspace), "");

    unsafe {
        env::remove_var("GSDV_STORE_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

/// 验证 workspace 粒度文件统一落到同一个 hash 目录。
#[test]
fn workspace_scoped_files_share_workspace_store_dir() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("workspace_store_dir");
    let store = root.join("store.json");
    let workspace = root.join("repo");
    unsafe {
        env::set_var("GSDV_STORE_PATH", &store);
    }

    let gws = workspace_store_dir(&workspace).unwrap();

    assert_eq!(
        workspace_memo_path(&workspace).unwrap(),
        gws.join("memo.md")
    );
    assert_eq!(
        workspace_terminal_history_path(&workspace).unwrap(),
        gws.join("terminal.txt")
    );
    assert_eq!(
        workspace_recent_markdowns_path(&workspace).unwrap(),
        gws.join("recent-markdowns.json")
    );
    assert_eq!(gws.parent().unwrap(), root.join("workspaces"));

    unsafe {
        env::remove_var("GSDV_STORE_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

/// 验证 workspace 最近编辑 Markdown 记录走独立 sidecar。
#[test]
fn workspace_recent_markdowns_round_trips_by_workspace() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("workspace_recent_markdowns");
    let store = root.join("store.json");
    let workspace = root.join("repo");
    unsafe {
        env::set_var("GSDV_STORE_PATH", &store);
    }
    let recent = vec![RecentMarkdownEntry {
        path: PathBuf::from("docs/a.md"),
        edited_at_ms: 42,
    }];

    save_workspace_recent_markdowns(&workspace, &recent).unwrap();

    assert_eq!(load_workspace_recent_markdowns(&workspace), recent);

    unsafe {
        env::remove_var("GSDV_STORE_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn theme_mode_round_trips_through_store() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("theme_mode");
    let store = root.join("store.json");
    unsafe {
        env::set_var("GSDV_STORE_PATH", &store);
    }

    assert_eq!(load_theme_mode(), None);

    save_theme_mode(ThemeMode::Light);

    assert_eq!(load_theme_mode(), Some(ThemeMode::Light));

    unsafe {
        env::remove_var("GSDV_STORE_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

/// 验证界面语言会写入全局 store 并可重新读取。
#[test]
fn app_language_round_trips_through_store() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("app_language");
    let store = root.join("store.json");
    unsafe {
        env::set_var("GSDV_STORE_PATH", &store);
    }

    assert_eq!(load_app_language(), AppLanguage::English);

    save_app_language(AppLanguage::Japanese);

    assert_eq!(load_app_language(), AppLanguage::Japanese);

    unsafe {
        env::remove_var("GSDV_STORE_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn font_settings_round_trips_through_store() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("font_settings");
    let store = root.join("store.json");
    unsafe {
        env::set_var("GSDV_STORE_PATH", &store);
    }

    let settings = FontSettings {
        default_fonts: FontSurfaceSettings::default(),
        agent: FontSurfaceSettings {
            family: FontFamilySetting::System,
            size: 14.5,
            system_name: Some("Agent Font".to_string()),
            system_path: Some("/tmp/agent-font.ttf".to_string()),
            fallback_system_name: Some("Agent Fallback".to_string()),
            fallback_system_path: Some("/tmp/agent-fallback.ttf".to_string()),
        },
        terminal: FontSurfaceSettings {
            family: FontFamilySetting::Monospace,
            size: 13.5,
            system_name: None,
            system_path: None,
            fallback_system_name: None,
            fallback_system_path: None,
        },
        editor: FontSurfaceSettings {
            family: FontFamilySetting::System,
            size: 16.0,
            system_name: Some("Demo Font".to_string()),
            system_path: Some("/tmp/demo-font.ttf".to_string()),
            fallback_system_name: None,
            fallback_system_path: None,
        },
    };
    save_font_settings(&settings);

    assert_eq!(load_font_settings(), settings);

    unsafe {
        env::remove_var("GSDV_STORE_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn home_codex_markdown_is_visible_by_default() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("home_codex_outline");
    let workspace = root.join("workspace");
    let codex = root.join(".codex");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&codex).unwrap();
    fs::write(codex.join("AGENTS.md"), "# Agents\n").unwrap();
    let previous_home = env::var_os("HOME");
    unsafe {
        env::set_var("HOME", &root);
    }

    let outline = build_outline(&workspace, None);

    assert!(outline_contains_file(&outline, &codex.join("AGENTS.md")));

    unsafe {
        match previous_home {
            Some(home) => env::set_var("HOME", home),
            None => env::remove_var("HOME"),
        }
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn workspace_outline_preloads_children_for_later_expansion() {
    let root = test_root("outline_preloads_children");
    let workspace = root.join("workspace");
    fs::create_dir_all(workspace.join("repos/product/skills/nested")).unwrap();
    fs::write(
        workspace.join("repos/product/skills/nested/README.md"),
        "# Nested\n",
    )
    .unwrap();

    let outline = build_outline(&workspace, None);
    let skills = find_outline_dir(&outline, Path::new("repos/product/skills")).unwrap();

    match skills {
        OutlineNode::Dir {
            expanded, children, ..
        } => {
            assert!(!expanded);
            assert!(!children.is_empty());
        }
        _ => panic!("expected skills to be a directory"),
    }

    let _ = fs::remove_dir_all(root);
}

/// 验证 outline 刷新不会重新展开用户刚折叠的选中文件父级。
#[test]
fn refresh_workspace_outline_preserves_collapsed_selected_parent() {
    let root = test_root("outline_preserves_collapsed_selected_parent");
    let workspace = root.join("workspace");
    fs::create_dir_all(workspace.join("src/nested")).unwrap();
    fs::write(workspace.join("src/nested/README.md"), "# Nested\n").unwrap();
    let selected = PathBuf::from("src/nested/README.md");
    let mut outline = build_outline(&workspace, Some(&selected));
    assert!(set_outline_dir_expanded(
        &mut outline,
        Path::new("src"),
        false
    ));
    let mut workspace_data = test_workspace_data(workspace.clone(), Some(selected), outline);

    refresh_workspace_outline(&mut workspace_data);

    let src = find_outline_dir(&workspace_data.outline, Path::new("src")).unwrap();
    match src {
        OutlineNode::Dir { expanded, .. } => assert!(!expanded),
        _ => panic!("expected src dir"),
    }

    let _ = fs::remove_dir_all(root);
}

/// 验证 outline 刷新不会把用户折叠的 workspace root 写回展开。
#[test]
fn refresh_workspace_outline_preserves_collapsed_root() {
    let root = test_root("outline_preserves_collapsed_root");
    let workspace = root.join("workspace");
    fs::create_dir_all(workspace.join("src")).unwrap();
    fs::write(workspace.join("src/README.md"), "# Src\n").unwrap();
    let mut outline = build_outline(&workspace, Some(Path::new("src/README.md")));
    if let Some(OutlineNode::Root { expanded, .. }) = outline.get_mut(0) {
        *expanded = false;
    }
    let mut workspace_data = test_workspace_data(
        workspace.clone(),
        Some(PathBuf::from("src/README.md")),
        outline,
    );

    refresh_workspace_outline(&mut workspace_data);

    match &workspace_data.outline[0] {
        OutlineNode::Root { expanded, .. } => assert!(!expanded),
        _ => panic!("expected workspace root"),
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn network_settings_round_trip_through_store() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("network_settings");
    let store = root.join("store.json");
    unsafe {
        env::set_var("GSDV_STORE_PATH", &store);
    }

    let settings = NetworkSettings {
        proxy: "http://proxy.local:8080".to_string(),
        no_proxy: "example.com 127.0.0.1/8,.internal".to_string(),
        mirror_proxy_protocol: false,
    };
    save_network_settings(&settings);

    let loaded = load_network_settings();
    assert_eq!(loaded, settings);
    assert_eq!(
        loaded.env_vars(),
        vec![
            (
                "http_proxy".to_string(),
                "http://proxy.local:8080".to_string()
            ),
            (
                "https_proxy".to_string(),
                "http://proxy.local:8080".to_string()
            ),
            (
                "no_proxy".to_string(),
                "192.168.0.0/16,127.0.0.1/8,example.com,.internal".to_string()
            ),
        ]
    );

    unsafe {
        env::remove_var("GSDV_STORE_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn default_network_settings_leave_proxy_empty() {
    let settings = NetworkSettings::default();

    assert!(settings.proxy.is_empty());
    assert!(settings.no_proxy.is_empty());
    assert_eq!(
        settings.env_vars(),
        vec![(
            "no_proxy".to_string(),
            "192.168.0.0/16,127.0.0.1/8".to_string()
        )]
    );
}

#[test]
fn network_settings_preserve_local_proxy() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("local_proxy_settings");
    let store = root.join("store.json");
    unsafe {
        env::set_var("GSDV_STORE_PATH", &store);
    }

    let settings = NetworkSettings {
        proxy: "http://127.0.0.1:7890".to_string(),
        no_proxy: String::new(),
        mirror_proxy_protocol: false,
    };
    save_network_settings(&settings);

    let loaded = load_network_settings();

    assert_eq!(loaded, settings);
    assert_eq!(
        loaded.env_vars(),
        vec![
            (
                "http_proxy".to_string(),
                "http://127.0.0.1:7890".to_string()
            ),
            (
                "https_proxy".to_string(),
                "http://127.0.0.1:7890".to_string()
            ),
            (
                "no_proxy".to_string(),
                "192.168.0.0/16,127.0.0.1/8".to_string()
            ),
        ]
    );

    unsafe {
        env::remove_var("GSDV_STORE_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

/// 验证开启互补协议后 HTTP 代理会额外生成 SOCKS env。
#[test]
fn network_settings_mirror_http_proxy_to_socks_env() {
    let settings = NetworkSettings {
        proxy: "http://127.0.0.1:7890".to_string(),
        no_proxy: String::new(),
        mirror_proxy_protocol: true,
    };

    assert_eq!(
        settings.env_vars(),
        vec![
            (
                "http_proxy".to_string(),
                "http://127.0.0.1:7890".to_string()
            ),
            (
                "https_proxy".to_string(),
                "http://127.0.0.1:7890".to_string()
            ),
            (
                "all_proxy".to_string(),
                "socks5://127.0.0.1:7890".to_string()
            ),
            (
                "no_proxy".to_string(),
                "192.168.0.0/16,127.0.0.1/8".to_string()
            ),
        ]
    );
}

/// 验证开启互补协议后 SOCKS 代理会额外生成 HTTP env。
#[test]
fn network_settings_mirror_socks_proxy_to_http_env() {
    let settings = NetworkSettings {
        proxy: "socks5://127.0.0.1:7891".to_string(),
        no_proxy: String::new(),
        mirror_proxy_protocol: true,
    };

    assert_eq!(
        settings.env_vars(),
        vec![
            (
                "all_proxy".to_string(),
                "socks5://127.0.0.1:7891".to_string()
            ),
            (
                "http_proxy".to_string(),
                "http://127.0.0.1:7891".to_string()
            ),
            (
                "https_proxy".to_string(),
                "http://127.0.0.1:7891".to_string()
            ),
            (
                "no_proxy".to_string(),
                "192.168.0.0/16,127.0.0.1/8".to_string()
            ),
        ]
    );
}

#[test]
fn save_workspace_store_preserves_default_agent_kind() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("preserve_default_agent_kind");
    let store = root.join("store.json");
    fs::write(
            &store,
            r#"{"default_agent_kind":"claude","theme_mode":"light","network_settings":{"proxy":"http://proxy.local:8080","no_proxy":"example.com","mirror_proxy_protocol":true},"runtime_settings":{"max_frame_rate":45,"agent_busy_auto_go_minutes":7,"pomodoro_enabled":false,"pomodoro_work_minutes":30,"pomodoro_rest_minutes":9},"font_settings":{"terminal":{"family":"monospace","size":13.0},"editor":{"family":"proportional","size":15.0}},"workspaces":[]}"#,
        )
        .unwrap();
    unsafe {
        env::set_var("GSDV_STORE_PATH", &store);
    }
    let workspace = WorkspaceViewData {
        name: "repo".to_string(),
        path: root.join("repo"),
        agent_kind: AgentKind::Codex,
        agent_id: "agent".to_string(),
        session_id: None,
        subagents: Vec::new(),
        activity: WorkspaceActivity::Unknown,
        center_mode: CenterMode::Agent,
        previous_center_mode: CenterMode::Agent,
        route: Route::Workspace,
        reviewer_mode: ReviewerMode::Git,
        selected_file: None,
        outline: Vec::new(),
        outline_favorites: BTreeSet::new(),
        recent_markdowns: Vec::new(),
        markdown_outline_collapsed: false,
        memo: String::new(),
    };

    save_workspace_store(&[workspace], 0, false);

    let content = fs::read_to_string(&store).unwrap();
    assert!(content.contains(r#""default_agent_kind": "claude""#));
    assert!(content.contains(r#""theme_mode": "light""#));
    assert!(content.contains(r#""proxy": "http://proxy.local:8080""#));
    assert!(content.contains(r#""no_proxy": "example.com""#));
    assert!(content.contains(r#""mirror_proxy_protocol": true"#));
    assert!(content.contains(r#""max_frame_rate": 45"#));
    assert!(content.contains(r#""agent_busy_auto_go_minutes": 7"#));
    assert!(content.contains(r#""pomodoro_enabled": false"#));
    assert!(content.contains(r#""pomodoro_work_minutes": 30"#));
    assert!(content.contains(r#""pomodoro_rest_minutes": 9"#));
    assert!(content.contains(r#""pomodoro_warning_remaining_percent": 20"#));
    assert!(content.contains(r#""font_settings""#));
    assert!(content.contains(r#""family": "proportional""#));

    unsafe {
        env::remove_var("GSDV_STORE_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn load_initial_gui_data_restores_persisted_workspace_state() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("restore_workspace_state");
    let workspace = root.join("repo");
    fs::create_dir_all(workspace.join("docs")).unwrap();
    fs::write(workspace.join("docs").join("plan.md"), "# Plan\n").unwrap();
    let store = root.join("store.json");
    let content = format!(
        r#"{{
                "active": 0,
                "workspaces": [{{
                    "path": "{}",
                    "session_id": "session-123",
                    "selected_file": "docs/plan.md",
                    "center_mode": "preview",
                    "reviewer_mode": "gsd",
                    "markdown_outline_collapsed": true
                }}]
            }}"#,
        workspace.display()
    );
    fs::write(&store, content).unwrap();
    unsafe {
        env::set_var("GSDV_STORE_PATH", &store);
    }

    let data = load_initial_gui_data(AgentKind::Codex);

    assert_eq!(data.workspaces.len(), 1);
    let workspace = &data.workspaces[0];
    assert_eq!(workspace.session_id, None);
    assert_eq!(
        workspace.selected_file.as_deref(),
        Some(Path::new("docs/plan.md"))
    );
    assert_eq!(workspace.center_mode, CenterMode::Preview);
    assert_eq!(workspace.reviewer_mode, ReviewerMode::Gsd);
    assert_eq!(workspace.agent_kind, AgentKind::Codex);
    assert!(workspace.markdown_outline_collapsed);

    unsafe {
        env::remove_var("GSDV_STORE_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn load_initial_gui_data_prunes_status_sessions_missing_from_store() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("prune_status_sessions_on_load");
    let workspace = root.join("repo");
    fs::create_dir_all(&workspace).unwrap();
    let store = root.join("store.json");
    let status_file = root.join("agent-status.json");
    fs::write(
        &store,
        format!(
            r#"{{
                    "active": 0,
                    "workspaces": [{{
                        "path": "{}",
                        "agent_kind": "codex",
                        "agent_id": "agent",
                        "session_id": "kept-session"
                    }}]
                }}"#,
            workspace.display()
        ),
    )
    .unwrap();
    fs::write(
        &status_file,
        format!(
            r#"{{
                    "version": 2,
                    "sessions": {{
                        "kept-session": {{
                            "workspace": "{}",
                            "agent": "codex",
                            "agent_id": "agent",
                            "status": "idle",
                            "updated_at_ms": 1
                        }},
                        "stale-session": {{
                            "workspace": "{}",
                            "agent": "codex",
                            "agent_id": "agent",
                            "status": "busy",
                            "updated_at_ms": 2
                        }}
                    }},
                    "agents": {{
                        "kept-agent": {{
                            "workspace": "{}",
                            "agent": "codex",
                            "session_id": "kept-session",
                            "status": "idle"
                        }},
                        "stale-agent": {{
                            "workspace": "{}",
                            "agent": "codex",
                            "session_id": "stale-session",
                            "status": "busy"
                        }}
                    }}
                }}"#,
            workspace.display(),
            workspace.display(),
            workspace.display(),
            workspace.display()
        ),
    )
    .unwrap();
    unsafe {
        env::set_var("GSDV_STORE_PATH", &store);
        env::set_var("GSDV_AGENT_STATUS_PATH", &status_file);
    }

    let data = load_initial_gui_data(AgentKind::Codex);

    assert_eq!(data.workspaces.len(), 1);
    let content = fs::read_to_string(&status_file).unwrap();
    let status = serde_json::from_str::<serde_json::Value>(&content).unwrap();
    assert!(status["sessions"].get("kept-session").is_some());
    assert!(status["sessions"].get("stale-session").is_none());
    assert!(status["agents"].get("kept-agent").is_some());
    assert!(status["agents"].get("stale-agent").is_none());
    let store_content = fs::read_to_string(&store).unwrap();
    assert!(store_content.contains(r#""session_id": "kept-session""#));

    unsafe {
        env::remove_var("GSDV_STORE_PATH");
        env::remove_var("GSDV_AGENT_STATUS_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn load_initial_gui_data_clears_store_session_missing_from_status_file() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("clear_store_session_missing_status");
    let workspace = root.join("repo");
    fs::create_dir_all(&workspace).unwrap();
    let store = root.join("store.json");
    let status_file = root.join("agent-status.json");
    fs::write(
        &store,
        format!(
            r#"{{
                    "active": 0,
                    "workspaces": [{{
                        "path": "{}",
                        "agent_kind": "codex",
                        "agent_id": "agent",
                        "session_id": "missing-session"
                    }}]
                }}"#,
            workspace.display()
        ),
    )
    .unwrap();
    fs::write(
        &status_file,
        format!(
            r#"{{
                    "version": 2,
                    "sessions": {{
                        "other-session": {{
                            "workspace": "{}",
                            "agent": "codex",
                            "agent_id": "agent",
                            "status": "idle"
                        }}
                    }}
                }}"#,
            workspace.display()
        ),
    )
    .unwrap();
    unsafe {
        env::set_var("GSDV_STORE_PATH", &store);
        env::set_var("GSDV_AGENT_STATUS_PATH", &status_file);
    }

    let data = load_initial_gui_data(AgentKind::Codex);

    assert_eq!(data.workspaces.len(), 1);
    assert_eq!(data.workspaces[0].session_id, None);
    let store_content = fs::read_to_string(&store).unwrap();
    assert!(!store_content.contains("missing-session"));

    unsafe {
        env::remove_var("GSDV_STORE_PATH");
        env::remove_var("GSDV_AGENT_STATUS_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn load_initial_gui_data_does_not_reuse_codex_session_for_claude() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("restore_workspace_agent_kind");
    let workspace = root.join("repo");
    fs::create_dir_all(&workspace).unwrap();
    let store = root.join("store.json");
    let content = format!(
        r#"{{
                "active": 0,
                "workspaces": [{{
                    "path": "{}",
                    "session_id": "codex-session"
                }}]
            }}"#,
        workspace.display()
    );
    fs::write(&store, content).unwrap();
    unsafe {
        env::set_var("GSDV_STORE_PATH", &store);
        env::remove_var("GSDV_AGENT_STATUS_PATH");
    }

    let data = load_initial_gui_data(AgentKind::Claude);

    assert_eq!(data.workspaces.len(), 1);
    assert_eq!(data.workspaces[0].session_id, None);

    unsafe {
        env::remove_var("GSDV_STORE_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn load_initial_gui_data_restores_persisted_agent_kind() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("restore_persisted_agent_kind");
    let workspace = root.join("repo");
    fs::create_dir_all(&workspace).unwrap();
    let store = root.join("store.json");
    let content = format!(
        r#"{{
                "active": 0,
                "workspaces": [{{
                    "path": "{}",
                    "agent_kind": "codex",
                    "session_id": "codex-session"
                }}]
            }}"#,
        workspace.display()
    );
    fs::write(&store, content).unwrap();
    unsafe {
        env::set_var("GSDV_STORE_PATH", &store);
        env::remove_var("GSDV_AGENT_STATUS_PATH");
    }

    let data = load_initial_gui_data(AgentKind::Claude);

    assert_eq!(data.workspaces.len(), 1);
    assert_eq!(data.workspaces[0].agent_kind, AgentKind::Codex);
    assert_eq!(data.workspaces[0].session_id, None);

    unsafe {
        env::remove_var("GSDV_STORE_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn save_workspace_store_persists_agent_kind() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("save_agent_kind");
    let store = root.join("store.json");
    unsafe {
        env::set_var("GSDV_STORE_PATH", &store);
    }
    let workspace = WorkspaceViewData {
        name: "repo".to_string(),
        path: root.join("repo"),
        agent_kind: AgentKind::Claude,
        agent_id: "agent".to_string(),
        session_id: None,
        subagents: Vec::new(),
        activity: WorkspaceActivity::Unknown,
        center_mode: CenterMode::Agent,
        previous_center_mode: CenterMode::Agent,
        route: Route::Workspace,
        reviewer_mode: ReviewerMode::Git,
        selected_file: None,
        outline: Vec::new(),
        outline_favorites: BTreeSet::new(),
        recent_markdowns: Vec::new(),
        markdown_outline_collapsed: true,
        memo: String::new(),
    };

    save_workspace_store(&[workspace], 0, false);

    let content = fs::read_to_string(&store).unwrap();
    assert!(content.contains(r#""agent_kind": "claude""#));
    assert!(!content.contains("session_id"));
    assert!(!content.contains("selected_file"));
    assert!(!content.contains("center_mode"));
    assert!(!content.contains("reviewer_mode"));
    assert!(content.contains(r#""markdown_outline_collapsed": true"#));

    unsafe {
        env::remove_var("GSDV_STORE_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn save_workspace_store_persists_non_empty_session_id() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("save_session_id");
    let store = root.join("store.json");
    unsafe {
        env::set_var("GSDV_STORE_PATH", &store);
    }
    let workspace = WorkspaceViewData {
        name: "repo".to_string(),
        path: root.join("repo"),
        agent_kind: AgentKind::Codex,
        agent_id: "agent".to_string(),
        session_id: Some("session-abc".to_string()),
        subagents: Vec::new(),
        activity: WorkspaceActivity::Unknown,
        center_mode: CenterMode::Agent,
        previous_center_mode: CenterMode::Agent,
        route: Route::Workspace,
        reviewer_mode: ReviewerMode::Git,
        selected_file: None,
        outline: Vec::new(),
        outline_favorites: BTreeSet::new(),
        recent_markdowns: Vec::new(),
        markdown_outline_collapsed: false,
        memo: String::new(),
    };

    save_workspace_store(&[workspace], 0, false);

    let content = fs::read_to_string(&store).unwrap();
    assert!(content.contains(r#""session_id": "session-abc""#));

    unsafe {
        env::remove_var("GSDV_STORE_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn refresh_workspace_agent_statuses_uses_latest_status_file_entry() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("refresh_agent_status");
    let workspace = root.join("repo");
    fs::create_dir_all(&workspace).unwrap();
    let status_file = root.join("agent-status.json");
    let content = format!(
        r#"{{
                "agents": {{
                    "old": {{
                        "workspace": "{}",
                        "status": "idle",
                        "session_id": "old-session",
                        "updated_at_ms": 1
                    }},
                    "new": {{
                        "workspace": "{}",
                        "status": "busy",
                        "session_id": "new-session",
                        "updated_at_ms": 2
                    }}
                }}
            }}"#,
        workspace.display(),
        workspace.display()
    );
    fs::write(&status_file, content).unwrap();
    unsafe {
        env::set_var("GSDV_AGENT_STATUS_PATH", &status_file);
    }
    let mut workspaces = vec![WorkspaceViewData {
        name: "repo".to_string(),
        path: workspace,
        agent_kind: AgentKind::Codex,
        agent_id: "agent".to_string(),
        session_id: None,
        subagents: Vec::new(),
        activity: WorkspaceActivity::Idle,
        center_mode: CenterMode::Agent,
        previous_center_mode: CenterMode::Agent,
        route: Route::Workspace,
        reviewer_mode: ReviewerMode::Git,
        selected_file: None,
        outline: Vec::new(),
        outline_favorites: BTreeSet::new(),
        recent_markdowns: Vec::new(),
        markdown_outline_collapsed: false,
        memo: String::new(),
    }];

    assert!(refresh_workspace_agent_statuses(&mut workspaces));

    assert_eq!(workspaces[0].activity, WorkspaceActivity::Busy);
    assert_eq!(workspaces[0].session_id.as_deref(), Some("new-session"));

    unsafe {
        env::remove_var("GSDV_AGENT_STATUS_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn refresh_workspace_agent_statuses_prefers_matching_agent_id() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("refresh_agent_status_id");
    let workspace = root.join("repo");
    fs::create_dir_all(&workspace).unwrap();
    let status_file = root.join("agent-status.json");
    let content = format!(
        r#"{{
                "agents": {{
                    "other-agent": {{
                        "workspace": "{}",
                        "agent": "codex",
                        "status": "idle",
                        "session_id": "other-session",
                        "updated_at_ms": 20
                    }},
                    "workspace-agent": {{
                        "workspace": "{}",
                        "agent": "codex",
                        "status": "busy",
                        "session_id": "active-session",
                        "updated_at_ms": 10
                    }}
                }}
            }}"#,
        workspace.display(),
        workspace.display()
    );
    fs::write(&status_file, content).unwrap();
    unsafe {
        env::set_var("GSDV_AGENT_STATUS_PATH", &status_file);
    }
    let mut workspaces = vec![WorkspaceViewData {
        name: "repo".to_string(),
        path: workspace,
        agent_kind: AgentKind::Codex,
        agent_id: "workspace-agent".to_string(),
        session_id: None,
        subagents: Vec::new(),
        activity: WorkspaceActivity::Idle,
        center_mode: CenterMode::Agent,
        previous_center_mode: CenterMode::Agent,
        route: Route::Workspace,
        reviewer_mode: ReviewerMode::Git,
        selected_file: None,
        outline: Vec::new(),
        outline_favorites: BTreeSet::new(),
        recent_markdowns: Vec::new(),
        markdown_outline_collapsed: false,
        memo: String::new(),
    }];

    assert!(refresh_workspace_agent_statuses(&mut workspaces));

    assert_eq!(workspaces[0].activity, WorkspaceActivity::Busy);
    assert_eq!(workspaces[0].session_id.as_deref(), Some("active-session"));

    unsafe {
        env::remove_var("GSDV_AGENT_STATUS_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn refresh_workspace_agent_statuses_reads_sessions_keyed_by_session_id() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("refresh_session_keyed_status");
    let workspace = root.join("repo");
    fs::create_dir_all(&workspace).unwrap();
    let status_file = root.join("agent-status.json");
    let content = format!(
        r#"{{
                "version": 2,
                "sessions": {{
                    "old-session": {{
                        "workspace": "{}",
                        "agent": "codex",
                        "agent_id": "workspace-agent",
                        "status": "idle",
                        "started_at_ms": 10,
                        "updated_at_ms": 30
                    }},
                    "new-session": {{
                        "workspace": "{}",
                        "agent": "codex",
                        "agent_id": "workspace-agent",
                        "status": "busy",
                        "started_at_ms": 20,
                        "updated_at_ms": 20
                    }}
                }}
            }}"#,
        workspace.display(),
        workspace.display()
    );
    fs::write(&status_file, content).unwrap();
    unsafe {
        env::set_var("GSDV_AGENT_STATUS_PATH", &status_file);
    }
    let mut workspaces = vec![WorkspaceViewData {
        name: "repo".to_string(),
        path: workspace,
        agent_kind: AgentKind::Codex,
        agent_id: "workspace-agent".to_string(),
        session_id: Some("old-session".to_string()),
        subagents: Vec::new(),
        activity: WorkspaceActivity::Idle,
        center_mode: CenterMode::Agent,
        previous_center_mode: CenterMode::Agent,
        route: Route::Workspace,
        reviewer_mode: ReviewerMode::Git,
        selected_file: None,
        outline: Vec::new(),
        outline_favorites: BTreeSet::new(),
        recent_markdowns: Vec::new(),
        markdown_outline_collapsed: false,
        memo: String::new(),
    }];

    assert!(refresh_workspace_agent_statuses(&mut workspaces));

    assert_eq!(workspaces[0].activity, WorkspaceActivity::Busy);
    assert_eq!(workspaces[0].session_id.as_deref(), Some("new-session"));

    unsafe {
        env::remove_var("GSDV_AGENT_STATUS_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn refresh_workspace_agent_statuses_recovers_busy_aborted_turn() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("refresh_agent_status_aborted");
    let workspace = root.join("repo");
    fs::create_dir_all(&workspace).unwrap();
    let transcript = root.join("rollout-session.jsonl");
    fs::write(
            &transcript,
            r#"{"type":"event_msg","payload":{"type":"turn_aborted","turn_id":"turn-one","reason":"interrupted"}}"#,
        )
        .unwrap();
    let status_file = root.join("agent-status.json");
    let content = format!(
        r#"{{
                "agents": {{
                    "workspace-agent": {{
                        "workspace": "{}",
                        "agent": "codex",
                        "status": "busy",
                        "session_id": "session-one",
                        "turn_id": "turn-one",
                        "transcript_path": "{}",
                        "updated_at_ms": 10
                    }}
                }}
            }}"#,
        workspace.display(),
        transcript.display()
    );
    fs::write(&status_file, content).unwrap();
    unsafe {
        env::set_var("GSDV_AGENT_STATUS_PATH", &status_file);
    }
    let mut workspaces = vec![WorkspaceViewData {
        name: "repo".to_string(),
        path: workspace,
        agent_kind: AgentKind::Codex,
        agent_id: "workspace-agent".to_string(),
        session_id: None,
        subagents: Vec::new(),
        activity: WorkspaceActivity::Busy,
        center_mode: CenterMode::Agent,
        previous_center_mode: CenterMode::Agent,
        route: Route::Workspace,
        reviewer_mode: ReviewerMode::Git,
        selected_file: None,
        outline: Vec::new(),
        outline_favorites: BTreeSet::new(),
        recent_markdowns: Vec::new(),
        markdown_outline_collapsed: false,
        memo: String::new(),
    }];

    assert!(refresh_workspace_agent_statuses(&mut workspaces));

    assert_eq!(workspaces[0].activity, WorkspaceActivity::Idle);
    assert_eq!(workspaces[0].session_id.as_deref(), Some("session-one"));

    unsafe {
        env::remove_var("GSDV_AGENT_STATUS_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn refresh_workspace_agent_statuses_keeps_busy_without_matching_abort() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("refresh_agent_status_still_busy");
    let workspace = root.join("repo");
    fs::create_dir_all(&workspace).unwrap();
    let transcript = root.join("rollout-session.jsonl");
    fs::write(
            &transcript,
            r#"{"type":"event_msg","payload":{"type":"turn_aborted","turn_id":"other-turn","reason":"interrupted"}}"#,
        )
        .unwrap();
    let status_file = root.join("agent-status.json");
    let content = format!(
        r#"{{
                "agents": {{
                    "workspace-agent": {{
                        "workspace": "{}",
                        "agent": "codex",
                        "status": "busy",
                        "session_id": "session-one",
                        "turn_id": "turn-one",
                        "transcript_path": "{}",
                        "updated_at_ms": 10
                    }}
                }}
            }}"#,
        workspace.display(),
        transcript.display()
    );
    fs::write(&status_file, content).unwrap();
    unsafe {
        env::set_var("GSDV_AGENT_STATUS_PATH", &status_file);
    }
    let mut workspaces = vec![WorkspaceViewData {
        name: "repo".to_string(),
        path: workspace,
        agent_kind: AgentKind::Codex,
        agent_id: "workspace-agent".to_string(),
        session_id: None,
        subagents: Vec::new(),
        activity: WorkspaceActivity::Idle,
        center_mode: CenterMode::Agent,
        previous_center_mode: CenterMode::Agent,
        route: Route::Workspace,
        reviewer_mode: ReviewerMode::Git,
        selected_file: None,
        outline: Vec::new(),
        outline_favorites: BTreeSet::new(),
        recent_markdowns: Vec::new(),
        markdown_outline_collapsed: false,
        memo: String::new(),
    }];

    assert!(refresh_workspace_agent_statuses(&mut workspaces));

    assert_eq!(workspaces[0].activity, WorkspaceActivity::Busy);

    unsafe {
        env::remove_var("GSDV_AGENT_STATUS_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn refresh_workspace_agent_statuses_filters_other_agent_kinds() {
    let _guard = ENV_LOCK.lock().unwrap();
    let root = test_root("refresh_agent_status_kind");
    let workspace = root.join("repo");
    fs::create_dir_all(&workspace).unwrap();
    let status_file = root.join("agent-status.json");
    let content = format!(
        r#"{{
                "agents": {{
                    "claude": {{
                        "workspace": "{}",
                        "agent": "claude",
                        "status": "busy",
                        "session_id": "claude-session",
                        "updated_at_ms": 2
                    }}
                }}
            }}"#,
        workspace.display()
    );
    fs::write(&status_file, content).unwrap();
    unsafe {
        env::set_var("GSDV_AGENT_STATUS_PATH", &status_file);
    }
    let mut workspaces = vec![WorkspaceViewData {
        name: "repo".to_string(),
        path: workspace,
        agent_kind: AgentKind::Codex,
        agent_id: "agent".to_string(),
        session_id: None,
        subagents: Vec::new(),
        activity: WorkspaceActivity::Idle,
        center_mode: CenterMode::Agent,
        previous_center_mode: CenterMode::Agent,
        route: Route::Workspace,
        reviewer_mode: ReviewerMode::Git,
        selected_file: None,
        outline: Vec::new(),
        outline_favorites: BTreeSet::new(),
        recent_markdowns: Vec::new(),
        markdown_outline_collapsed: false,
        memo: String::new(),
    }];

    assert!(!refresh_workspace_agent_statuses(&mut workspaces));
    assert_eq!(workspaces[0].activity, WorkspaceActivity::Idle);

    unsafe {
        env::remove_var("GSDV_AGENT_STATUS_PATH");
    }
    let _ = fs::remove_dir_all(root);
}

/// Verifies that transcript scanning finds the matching aborted turn.
#[test]
fn transcript_contains_aborted_turn_finds_matching_turn() {
    let root = test_root("transcript_aborted_turn");
    let transcript = root.join("session.jsonl");
    fs::write(
        &transcript,
        concat!(
            r#"{"payload":{"type":"turn_started","turn_id":"turn-1"}}"#,
            "\n",
            r#"{"payload":{"type":"turn_aborted","turn_id":"turn-2"}}"#,
            "\n"
        ),
    )
    .unwrap();

    assert!(transcript_contains_aborted_turn(&transcript, "turn-2"));
    assert!(!transcript_contains_aborted_turn(&transcript, "turn-1"));

    let _ = fs::remove_dir_all(root);
}

/// Verifies that transcript scanning ignores malformed and unrelated lines.
#[test]
fn transcript_contains_aborted_turn_ignores_unrelated_lines() {
    let root = test_root("transcript_unrelated_lines");
    let transcript = root.join("session.jsonl");
    fs::write(
        &transcript,
        concat!(
            "not json\n",
            r#"{"payload":{"type":"turn_aborted","turn_id":"other-turn"}}"#,
            "\n",
            r#"{"payload":{"type":"turn_completed","turn_id":"target-turn"}}"#,
            "\n"
        ),
    )
    .unwrap();

    assert!(!transcript_contains_aborted_turn(
        &transcript,
        "target-turn"
    ));

    let _ = fs::remove_dir_all(root);
}

fn test_root(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!("gsdv-data-{name}-{unique}"));
    fs::create_dir_all(&root).unwrap();
    root
}

fn outline_contains_file(nodes: &[OutlineNode], expected: &Path) -> bool {
    nodes.iter().any(|node| match node {
        OutlineNode::Root { children, .. } | OutlineNode::Dir { children, .. } => {
            outline_contains_file(children, expected)
        }
        OutlineNode::File { path, .. } => path == expected,
    })
}

fn find_outline_dir<'a>(nodes: &'a [OutlineNode], expected: &Path) -> Option<&'a OutlineNode> {
    for node in nodes {
        match node {
            OutlineNode::Root { children, .. } => {
                if let Some(found) = find_outline_dir(children, expected) {
                    return Some(found);
                }
            }
            OutlineNode::Dir { key, children, .. } => {
                if key == expected {
                    return Some(node);
                }
                if let Some(found) = find_outline_dir(children, expected) {
                    return Some(found);
                }
            }
            OutlineNode::File { .. } => {}
        }
    }
    None
}

/// Sets a directory expansion flag by its stable outline key.
fn set_outline_dir_expanded(nodes: &mut [OutlineNode], expected: &Path, value: bool) -> bool {
    for node in nodes {
        match node {
            OutlineNode::Root { children, .. } => {
                if set_outline_dir_expanded(children, expected, value) {
                    return true;
                }
            }
            OutlineNode::Dir {
                key,
                expanded,
                children,
                ..
            } => {
                if key == expected {
                    *expanded = value;
                    return true;
                }
                if set_outline_dir_expanded(children, expected, value) {
                    return true;
                }
            }
            OutlineNode::File { .. } => continue,
        }
    }
    false
}

/// Builds a minimal workspace record for outline refresh tests.
fn test_workspace_data(
    path: PathBuf,
    selected_file: Option<PathBuf>,
    outline: Vec<OutlineNode>,
) -> WorkspaceViewData {
    WorkspaceViewData {
        name: "workspace".to_string(),
        path,
        agent_kind: AgentKind::Codex,
        agent_id: "agent".to_string(),
        session_id: None,
        subagents: Vec::new(),
        activity: WorkspaceActivity::Unknown,
        center_mode: CenterMode::Agent,
        previous_center_mode: CenterMode::Agent,
        route: Route::Workspace,
        reviewer_mode: ReviewerMode::Git,
        selected_file,
        outline,
        outline_favorites: BTreeSet::new(),
        recent_markdowns: Vec::new(),
        markdown_outline_collapsed: false,
        memo: String::new(),
    }
}

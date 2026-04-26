use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

/// 验证 agent status hook 脚本会执行给定 gsdv 二进制。
#[test]
fn agent_status_hook_script_execs_given_binary_path() {
    let script = agent_status_hook_script(Path::new("/tmp/gsdv current/bin/gsdv"));

    assert_eq!(
        script,
        "#!/bin/sh\nexec '/tmp/gsdv current/bin/gsdv' agent-status-hook\n"
    );
}

/// 验证 Claude hook 安装会替换旧 gsdv hook。
#[test]
fn claude_hook_event_replaces_existing_gsdv_hook() {
    let mut root = serde_json::json!({
        "hooks": {
            "SessionEnd": [{
                "hooks": [
                    {
                        "type": "command",
                        "command": "'/old/agent-status-hook'",
                        "timeout": 5
                    },
                    {
                        "type": "command",
                        "command": "other-hook",
                        "timeout": 5
                    }
                ]
            }]
        }
    });

    install_claude_hook_event(&mut root, "SessionEnd", None, "'/new/agent-status-hook'");

    let groups = root["hooks"]["SessionEnd"].as_array().unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0]["hooks"].as_array().unwrap().len(), 1);
    assert_eq!(groups[0]["hooks"][0]["command"], "other-hook");
    assert_eq!(groups[1]["hooks"][0]["command"], "'/new/agent-status-hook'");
}

/// 验证 Codex feature 会迁移到新版 hooks key。
#[test]
fn codex_feature_migrates_deprecated_codex_hooks_key() {
    let updated = set_toml_feature_bool_removing_aliases(
        "[model]\nname = \"gpt-5\"\n\n[features]\ncodex_hooks = true\n",
        "hooks",
        true,
        &["codex_hooks"],
    );

    assert!(updated.contains("[features]\nhooks = true\n"));
    assert!(!updated.contains("codex_hooks"));
}

/// 验证 Codex hook 清理会移除 recent 遗留的工具事件。
#[test]
fn codex_hook_cleanup_removes_obsolete_tool_events() {
    let mut root = serde_json::json!({
        "hooks": {
            "PreToolUse": [{
                "matcher": "apply_patch|Write|Edit",
                "hooks": [{
                    "type": "command",
                    "command": "'/old/agent-status-hook'",
                    "timeout": 5
                }]
            }],
            "PostToolUse": [{
                "matcher": "apply_patch|Write|Edit",
                "hooks": [
                    {
                        "type": "command",
                        "command": "'/old/agent-status-hook'",
                        "timeout": 5
                    },
                    {
                        "type": "command",
                        "command": "other-hook",
                        "timeout": 5
                    }
                ]
            }]
        }
    });

    remove_codex_hook_event(&mut root, "PreToolUse");
    remove_codex_hook_event(&mut root, "PostToolUse");

    assert!(root["hooks"].get("PreToolUse").is_none());
    let groups = root["hooks"]["PostToolUse"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["hooks"].as_array().unwrap().len(), 1);
    assert_eq!(groups[0]["hooks"][0]["command"], "other-hook");
}

/// 验证 gsdv 计算出的信任 hash 与 Codex normalized identity 一致。
#[test]
fn codex_hook_trusted_hash_matches_normalized_identity() {
    let hash = codex_command_hook_hash(
        "session_start",
        None,
        "'/Users/test/.gsdv/hooks/agent-status-hook'",
        CODEX_AGENT_STATUS_HOOK_TIMEOUT,
        Some(CODEX_AGENT_STATUS_MESSAGE),
    );

    assert_eq!(
        hash,
        "sha256:a44aef407be1d22870fb19677d4381978bb4d50711a4700f111173b872e77a07"
    );
}

/// 验证安装 Codex hook 后会同步写入 hooks.state 信任状态。
#[test]
fn codex_hook_trust_state_is_written_after_install() {
    let root = unique_temp_dir("codex-hook-trust");
    fs::create_dir_all(&root).unwrap();
    let hooks_path = root.join("hooks.json");
    let config_path = root.join("config.toml");
    fs::write(&config_path, "[features]\nhooks = true\n").unwrap();

    let entries = install_codex_hooks_json(&hooks_path).unwrap();
    trust_installed_codex_hooks(&config_path, &entries).unwrap();

    let content = fs::read_to_string(&config_path).unwrap();
    assert!(content.contains("[hooks.state]"));
    assert!(content.contains("session_start:0:0"));
    assert!(content.contains("user_prompt_submit:0:0"));
    assert!(content.contains("stop:0:0"));
    assert!(
        content.contains("sha256:334d2a87410e9fe76f92092b3500f9da8d8debfa0b1866738c2e0c0a267e46a6")
    );

    let _ = fs::remove_dir_all(root);
}

/// 验证内置 skill 会写入目标目录。
#[test]
fn install_skill_writes_gsdv_wf_skill() {
    let root = unique_temp_dir("install-skill");
    let skill_dir = root.join("skills/gsdv-wf");

    install_skill(&skill_dir, GSDV_WF_SKILL).unwrap();

    let content = fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
    assert!(content.contains("name: gsdv-wf"));
    assert!(content.contains("description: \"轻量级任务管理\""));

    let _ = fs::remove_dir_all(root);
}

/// 验证内置 skill 会覆盖已有目标文件。
#[test]
fn install_skill_overwrites_existing_skill_file() {
    let root = unique_temp_dir("install-skill-overwrite");
    let skill_dir = root.join("skills/gsdv-wf");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), "old skill\n").unwrap();

    install_skill(&skill_dir, GSDV_WF_SKILL).unwrap();

    let content = fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
    assert!(content.contains("name: gsdv-wf"));
    assert!(!content.contains("old skill"));

    let _ = fs::remove_dir_all(root);
}

/// 生成不会和并发测试互相覆盖的临时目录。
fn unique_temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("gsdv-{name}-{nanos}"))
}

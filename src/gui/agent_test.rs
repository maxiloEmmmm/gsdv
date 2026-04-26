use super::*;

#[test]
fn codex_args_resume_non_empty_session() {
    assert_eq!(
        AgentKind::Codex.args(Some("abc123")),
        vec![
            "resume".to_string(),
            "--dangerously-bypass-approvals-and-sandbox".to_string(),
            "abc123".to_string(),
        ]
    );
    assert_eq!(
        AgentKind::Codex.args(None),
        vec!["--dangerously-bypass-approvals-and-sandbox".to_string()]
    );
}

#[test]
fn claude_args_use_skip_permissions_and_resume_session() {
    assert_eq!(
        AgentKind::Claude.args(Some("abc123")),
        vec![
            "--dangerously-skip-permissions".to_string(),
            "--resume".to_string(),
            "abc123".to_string(),
        ]
    );
    assert_eq!(
        AgentKind::Claude.args(None),
        vec!["--dangerously-skip-permissions".to_string()]
    );
}

#[test]
fn launch_config_parses_agent_and_coder_args() {
    let config = AgentLaunchConfig::from_args([
        "--agent".to_string(),
        "claude".to_string(),
        "--coder-arg=--verbose".to_string(),
        "--coder-arg".to_string(),
        "--model".to_string(),
    ]);

    assert_eq!(config.kind, AgentKind::Claude);
    assert!(config.kind_explicit);
    assert_eq!(config.coder_args, vec!["--verbose", "--model"]);
}

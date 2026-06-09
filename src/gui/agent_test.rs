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

#[test]
fn codex_resume_args_include_explicit_cwd_before_subcommand() {
    let root = std::env::temp_dir();
    let config = AgentLaunchConfig::default();

    let args = config.args_for(
        AgentKind::Codex,
        Some("abc123"),
        None,
        None,
        None,
        None,
        Some(root.as_path()),
    );

    assert_eq!(args[0], "-C");
    assert_eq!(args[1], root.display().to_string());
    assert_eq!(args[2], "resume");
}

#[test]
fn claude_args_do_not_include_work_dir_cli_arg() {
    let root = std::env::temp_dir();
    let root_display = root.display().to_string();
    let config = AgentLaunchConfig::default();

    let claude_args = config.args_for(
        AgentKind::Claude,
        Some("abc123"),
        None,
        None,
        None,
        None,
        Some(root.as_path()),
    );

    assert!(!claude_args.iter().any(|arg| arg == "-C"));
    assert!(!claude_args.iter().any(|arg| arg == "--cd"));
    assert!(!claude_args.iter().any(|arg| arg == &root_display));
}

/// 验证 Codex model_provider 覆盖会按 -c 参数传给 CLI。
#[test]
fn codex_args_include_model_provider_override() {
    let config = AgentLaunchConfig::default();

    let args = config.args_for(
        AgentKind::Codex,
        None,
        None,
        Some("frank"),
        None,
        None,
        None,
    );

    assert!(
        args.windows(2)
            .any(|pair| pair == ["-c", "model_provider=\"frank\""])
    );
}

/// 验证 Codex model_provider 覆盖中的字符串特殊字符会被转义。
#[test]
fn codex_args_escape_model_provider_override() {
    let config = AgentLaunchConfig::default();

    let args = config.args_for(
        AgentKind::Codex,
        None,
        None,
        Some("quote\"slash\\tab\t"),
        None,
        None,
        None,
    );

    assert!(
        args.windows(2)
            .any(|pair| pair == ["-c", "model_provider=\"quote\\\"slash\\\\tab\\t\""])
    );
}

/// 验证 Claude 不接收 Codex 专属 model_provider 覆盖。
#[test]
fn claude_args_ignore_model_provider_override() {
    let config = AgentLaunchConfig::default();

    let args = config.args_for(
        AgentKind::Claude,
        None,
        None,
        Some("frank"),
        None,
        None,
        None,
    );

    assert!(!args.iter().any(|arg| arg.contains("model_provider")));
}

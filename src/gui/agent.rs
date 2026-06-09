use std::env;
use std::fmt;
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Codex,
    Claude,
}

impl AgentKind {
    pub fn integration(self) -> &'static dyn AgentIntegration {
        match self {
            Self::Codex => &CODEX_INTEGRATION,
            Self::Claude => &CLAUDE_INTEGRATION,
        }
    }

    pub fn command(self) -> &'static str {
        self.integration().command()
    }

    pub fn title(self) -> &'static str {
        self.integration().title()
    }

    pub fn env_name(self) -> &'static str {
        self.integration().env_name()
    }

    pub fn args(self, session_id: Option<&str>) -> Vec<String> {
        self.integration().args(session_id)
    }

    /// Returns effort levels supported by this agent CLI.
    pub fn effort_levels(self) -> &'static [&'static str] {
        match self {
            Self::Codex => &["low", "medium", "high", "xhigh"],
            Self::Claude => &["low", "medium", "high", "xhigh", "max"],
        }
    }

    /// Returns whether an effort value is valid for this agent CLI.
    pub fn supports_effort(self, effort: &str) -> bool {
        self.effort_levels().contains(&effort.trim())
    }

    /// Returns whether this agent CLI supports Codex service tier fast mode.
    pub fn supports_fast_mode(self) -> bool {
        matches!(self, Self::Codex)
    }

    /// Returns whether this agent CLI supports model provider overrides.
    pub fn supports_model_provider(self) -> bool {
        matches!(self, Self::Codex)
    }

    pub fn all() -> [Self; 2] {
        [Self::Codex, Self::Claude]
    }
}

pub trait AgentIntegration: Sync {
    fn command(&self) -> &'static str;
    fn title(&self) -> &'static str;
    fn env_name(&self) -> &'static str;
    fn args(&self, session_id: Option<&str>) -> Vec<String>;
}

struct CodexIntegration;
struct ClaudeIntegration;

static CODEX_INTEGRATION: CodexIntegration = CodexIntegration;
static CLAUDE_INTEGRATION: ClaudeIntegration = ClaudeIntegration;

impl AgentIntegration for CodexIntegration {
    fn command(&self) -> &'static str {
        "codex"
    }

    fn title(&self) -> &'static str {
        "codex"
    }

    fn env_name(&self) -> &'static str {
        "codex"
    }

    fn args(&self, session_id: Option<&str>) -> Vec<String> {
        if let Some(session_id) = session_id.filter(|value| !value.is_empty()) {
            vec![
                "resume".to_string(),
                "--dangerously-bypass-approvals-and-sandbox".to_string(),
                session_id.to_string(),
            ]
        } else {
            vec!["--dangerously-bypass-approvals-and-sandbox".to_string()]
        }
    }
}

impl AgentIntegration for ClaudeIntegration {
    fn command(&self) -> &'static str {
        "claude"
    }

    fn title(&self) -> &'static str {
        "claude"
    }

    fn env_name(&self) -> &'static str {
        "claude"
    }

    fn args(&self, session_id: Option<&str>) -> Vec<String> {
        let mut args = vec!["--dangerously-skip-permissions".to_string()];
        if let Some(session_id) = session_id.filter(|value| !value.is_empty()) {
            args.push("--resume".to_string());
            args.push(session_id.to_string());
        }
        args
    }
}

impl Default for AgentKind {
    fn default() -> Self {
        Self::Codex
    }
}

impl FromStr for AgentKind {
    type Err = AgentKindParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "codex" => Ok(Self::Codex),
            "claude" => Ok(Self::Claude),
            _ => Err(AgentKindParseError {
                value: value.to_string(),
            }),
        }
    }
}

impl fmt::Display for AgentKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.env_name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentKindParseError {
    value: String,
}

impl fmt::Display for AgentKindParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unsupported agent kind {}", self.value)
    }
}

impl std::error::Error for AgentKindParseError {}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentLaunchConfig {
    pub kind: AgentKind,
    pub kind_explicit: bool,
    pub coder_args: Vec<String>,
}

impl AgentLaunchConfig {
    pub fn from_env_args() -> Self {
        Self::from_args(env::args().skip(1))
    }

    pub fn from_args(args: impl IntoIterator<Item = String>) -> Self {
        let mut config = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            if let Some(value) = arg.strip_prefix("--agent=") {
                if let Ok(kind) = value.parse() {
                    config.kind = kind;
                    config.kind_explicit = true;
                }
                continue;
            }
            if arg == "--agent" {
                if let Some(value) = args.next()
                    && let Ok(kind) = value.parse()
                {
                    config.kind = kind;
                    config.kind_explicit = true;
                }
                continue;
            }
            if let Some(value) = arg.strip_prefix("--coder-arg=") {
                config.coder_args.push(value.to_string());
                continue;
            }
            if arg == "--coder-arg" {
                if let Some(value) = args.next() {
                    config.coder_args.push(value);
                }
            }
        }
        config
    }

    /// Builds CLI args for the configured default agent.
    pub fn args(&self, session_id: Option<&str>) -> Vec<String> {
        self.args_for(self.kind, session_id, None, None, None, None, None)
    }

    /// Builds CLI args and applies per-agent overrides when present.
    pub fn args_for(
        &self,
        kind: AgentKind,
        session_id: Option<&str>,
        model: Option<&str>,
        model_provider: Option<&str>,
        effort: Option<&str>,
        fast_mode: Option<bool>,
        resume_cwd: Option<&Path>,
    ) -> Vec<String> {
        let mut args = kind.args(session_id);
        if let Some(cwd) = normalized_codex_resume_cwd_arg(kind, session_id, resume_cwd) {
            args.splice(0..0, ["-C".to_string(), cwd.display().to_string()]);
        }
        if let Some(model) = normalized_agent_model_arg(model) {
            args.push("--model".to_string());
            args.push(model.to_string());
        }
        if let Some(provider) = normalized_agent_model_provider_arg(kind, model_provider) {
            args.push("-c".to_string());
            args.push(format!(
                "model_provider={}",
                codex_config_basic_string(provider)
            ));
        }
        if let Some(effort) = normalized_agent_effort_arg(kind, effort) {
            match kind {
                AgentKind::Codex => {
                    args.push("-c".to_string());
                    args.push(format!("model_reasoning_effort=\"{effort}\""));
                }
                AgentKind::Claude => {
                    args.push("--effort".to_string());
                    args.push(effort.to_string());
                }
            }
        }
        if let Some(service_tier) = normalized_agent_fast_mode_arg(kind, fast_mode) {
            args.push("-c".to_string());
            args.push(format!("service_tier=\"{service_tier}\""));
        }
        args.extend(self.coder_args.iter().cloned());
        args
    }
}

/// Returns a non-empty model override suitable for CLI args.
fn normalized_agent_model_arg(model: Option<&str>) -> Option<&str> {
    model.map(str::trim).filter(|value| !value.is_empty())
}

/// Returns a non-empty Codex model provider override suitable for CLI args.
fn normalized_agent_model_provider_arg(kind: AgentKind, provider: Option<&str>) -> Option<&str> {
    if !kind.supports_model_provider() {
        return None;
    }
    provider.map(str::trim).filter(|value| !value.is_empty())
}

/// Escapes a value for Codex `-c key=<basic string>` CLI overrides.
fn codex_config_basic_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

/// Returns Codex's explicit cwd arg for resume sessions.
fn normalized_codex_resume_cwd_arg<'a>(
    kind: AgentKind,
    session_id: Option<&str>,
    cwd: Option<&'a Path>,
) -> Option<&'a Path> {
    if kind != AgentKind::Codex || session_id.is_none_or(|value| value.trim().is_empty()) {
        return None;
    }
    cwd.filter(|path| path.is_dir())
}

/// Returns a valid effort override suitable for the selected agent CLI.
fn normalized_agent_effort_arg(kind: AgentKind, effort: Option<&str>) -> Option<&str> {
    effort
        .map(str::trim)
        .filter(|value| kind.supports_effort(value))
}

/// Returns Codex's CLI service tier override for one fast mode state.
fn normalized_agent_fast_mode_arg(
    kind: AgentKind,
    fast_mode: Option<bool>,
) -> Option<&'static str> {
    if !kind.supports_fast_mode() {
        return None;
    }
    match fast_mode {
        Some(true) => Some("fast"),
        Some(false) => Some("default"),
        None => None,
    }
}

#[cfg(test)]
#[path = "agent_test.rs"]
mod agent_test;

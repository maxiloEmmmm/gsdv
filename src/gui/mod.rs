pub(crate) mod agent;
pub(crate) mod agent_status_hook;
mod app;
mod data;
mod diff_viewer;
mod i18n;
mod markdown_preview;
mod outline;
mod perf_log;
mod repaint_gate;
mod reviewer_adapter;
mod terminal_host;
mod theme;
mod workflow;

#[cfg(test)]
mod workflow_test;

pub use agent::{AgentKind, AgentLaunchConfig};
pub use app::run;

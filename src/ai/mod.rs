//! AI 调用模块，当前只承载 Codex 后端调用。

mod codex;
mod codex_auth;
pub mod error;
pub mod types;

pub use codex::{CodexClient, CodexRemote};
pub use codex_auth::{
    CodexAuthFlow, CodexAuthInfo, complete_browser_auth_flow, load_cached_auth_info,
    open_auth_browser, start_browser_auth_flow,
};
pub use error::{AiError, Result};
pub use types::{
    ChatRequest, ChatResponse, ChatTool, Message, Role, ThinkingMode, ToolCallObservation,
    ToolChoice, ToolHandler, ToolObserver, ToolResult, Usage,
};

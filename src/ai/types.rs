use serde::{Deserialize, Serialize};
use std::sync::Arc;

use futures::{FutureExt, future::BoxFuture};

use crate::ai::error::Result;

/// 对话消息角色，适用于 Codex Responses 输入转换。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// 系统消息，用于顶层 instructions。
    System,
    /// 用户消息，用于 user input item。
    User,
    /// 助手消息，用于续写历史 assistant item。
    Assistant,
}

/// 一条聊天消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// 消息角色。
    pub role: Role,
    /// 消息文本内容。
    pub content: String,
}

impl Message {
    /// 创建系统消息，适用于注入全局指令。
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    /// 创建用户消息，适用于提交本轮任务。
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    /// 创建助手消息，适用于携带历史回复。
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// Codex 聊天请求。
#[derive(Clone)]
pub struct ChatRequest {
    /// 本轮对话消息。
    pub messages: Vec<Message>,
    /// 采样温度。
    pub temperature: Option<f32>,
    /// 旧版兼容模型的输出 token 上限。
    pub max_tokens: Option<u32>,
    /// 新规范的 completion token 上限。
    pub max_completion_tokens: Option<u32>,
    /// 推理思考模式。
    pub thinking: ThinkingMode,
    /// 是否启用流式输出。
    pub stream: bool,
    /// 可供模型调用的工具列表。
    pub tools: Vec<ChatTool>,
    /// 模型工具选择策略。
    pub tool_choice: Option<ToolChoice>,
    /// 工具调用最大轮数。
    pub max_tool_rounds: usize,
    /// 工具调用观测回调。
    pub tool_observer: Option<ToolObserver>,
}

impl std::fmt::Debug for ChatRequest {
    /// 输出请求元数据，避免打印不可调试的 observer 闭包。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatRequest")
            .field("messages", &self.messages)
            .field("temperature", &self.temperature)
            .field("max_tokens", &self.max_tokens)
            .field("max_completion_tokens", &self.max_completion_tokens)
            .field("thinking", &self.thinking)
            .field("stream", &self.stream)
            .field("tools", &self.tools)
            .field("tool_choice", &self.tool_choice)
            .field("max_tool_rounds", &self.max_tool_rounds)
            .field(
                "tool_observer",
                &self.tool_observer.as_ref().map(|_| "<observer>"),
            )
            .finish()
    }
}

impl ChatRequest {
    /// 创建聊天请求，适用于单轮或多轮消息发送。
    pub fn new(messages: Vec<Message>) -> Self {
        Self {
            messages,
            temperature: None,
            max_tokens: None,
            max_completion_tokens: None,
            thinking: ThinkingMode::Disabled,
            stream: false,
            tools: Vec::new(),
            tool_choice: None,
            max_tool_rounds: 8,
            tool_observer: None,
        }
    }
}

/// Codex reasoning 参数模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThinkingMode {
    /// 显式关闭思考。
    Disabled,
    /// 显式开启中等强度思考。
    Enabled,
    /// 交给模型或服务端自动决定。
    Auto,
}

/// token 用量信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    /// 输入 token 数。
    pub prompt_tokens: u32,
    /// 输出 token 数。
    pub completion_tokens: u32,
}

/// Codex 聊天响应。
#[derive(Debug, Clone)]
pub struct ChatResponse {
    /// 模型输出文本。
    pub content: String,
    /// 实际使用的模型名。
    pub model: String,
    /// token 用量，服务端未返回时为空。
    pub usage: Option<Usage>,
}

/// 可提供给模型调用的本地 function tool。
#[derive(Clone)]
pub struct ChatTool {
    /// 工具名称。
    pub name: String,
    /// 工具说明。
    pub description: String,
    /// 工具入参 JSON Schema。
    pub parameters: serde_json::Value,
    /// 是否启用严格 schema。
    pub strict: bool,
    /// 本地工具处理函数。
    pub handler: ToolHandler,
}

impl std::fmt::Debug for ChatTool {
    /// 输出工具元数据，避免打印不可调试的闭包处理器。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatTool")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("parameters", &self.parameters)
            .field("strict", &self.strict)
            .finish_non_exhaustive()
    }
}

impl ChatTool {
    /// 创建一个普通 function tool，适用于只返回文本的工具。
    pub fn function<F>(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
        handler: F,
    ) -> Self
    where
        F: Fn(serde_json::Value) -> BoxFuture<'static, Result<String>> + Send + Sync + 'static,
    {
        let handler = Arc::new(move |args| {
            let output = handler(args);
            async move { output.await.map(ToolResult::continue_with_content) }.boxed()
        });
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
            strict: true,
            handler,
        }
    }

    /// 创建一个可控制工具循环的 function tool。
    pub fn controlled_function<F>(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
        handler: F,
    ) -> Self
    where
        F: Fn(serde_json::Value) -> BoxFuture<'static, Result<ToolResult>> + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
            strict: true,
            handler: Arc::new(handler),
        }
    }
}

/// 本地工具处理函数。
pub type ToolHandler =
    Arc<dyn Fn(serde_json::Value) -> BoxFuture<'static, Result<ToolResult>> + Send + Sync>;

/// 本地工具执行结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    /// 返回给模型的工具输出文本。
    pub content: String,
    /// 是否允许继续发起下一轮模型请求。
    pub continue_next_round: bool,
}

impl ToolResult {
    /// 返回文本并允许模型继续下一轮。
    pub fn continue_with_content(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            continue_next_round: true,
        }
    }

    /// 返回文本并停止模型继续下一轮。
    pub fn stop_with_content(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            continue_next_round: false,
        }
    }
}

/// 工具调用观测回调。
pub type ToolObserver =
    Arc<dyn Fn(ToolCallObservation) -> BoxFuture<'static, Result<()>> + Send + Sync>;

/// 一次模型工具调用观测事件。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallObservation {
    /// OpenAI tool call ID。
    pub id: String,
    /// 当前工具调用轮次。
    pub round: usize,
    /// 当前轮中的工具序号。
    pub call_index: usize,
    /// 工具名称。
    pub name: String,
    /// 解析后的工具参数。
    pub arguments: serde_json::Value,
}

/// Codex tool_choice 策略。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolChoice {
    /// 由模型自动决定是否调用工具。
    Auto,
    /// 禁止调用工具。
    None,
    /// 要求至少调用一个工具。
    Required,
    /// 强制调用指定 function tool。
    Function(String),
}

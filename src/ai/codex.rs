use base64::Engine as _;
use futures::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::pin::Pin;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::Mutex as AsyncMutex;
use tokio_socks::tcp::Socks5Stream;
use tokio_tungstenite::tungstenite::{
    Error as WsError, Message as WsMessage, client::IntoClientRequest,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, client_async_tls_with_config};

use crate::ai::codex_auth::{CodexCredentials, resolve_credentials};
use crate::ai::error::{
    AiError, Result, header_value, read_response_text, request_send_error, stream_body_error,
};
use crate::ai::types::{
    ChatRequest, ChatResponse, ChatTool, Message, Role, ThinkingMode, ToolCallObservation,
    ToolChoice, ToolResult, Usage,
};

/// Codex 瞬时错误最大重试次数。
const MAX_RETRIES: usize = 3;
/// Codex 瞬时错误基础退避时间。
const BASE_RETRY_DELAY_MS: u64 = 1_000;
/// Codex WebSocket 连续失败后才允许 HTTP fallback 的次数。
const WS_FAILURES_BEFORE_HTTP_FALLBACK: usize = 5;
/// Codex Responses WebSocket 握手头。
const RESPONSES_WEBSOCKETS_V2_BETA_HEADER_VALUE: &str = "responses_websockets=2026-02-06";
/// Codex WebSocket 空闲等待上限。
const WEBSOCKET_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
/// Codex 默认后端地址。
const DEFAULT_CODEX_ENDPOINT: &str = "https://chatgpt.com/backend-api";
/// Codex 默认模型。
const DEFAULT_CODEX_MODEL: &str = "gpt-5.5";

/// Codex 远端配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexRemote {
    /// 远端名称，也会作为 session_id 和 prompt cache key。
    pub name: String,
    /// Codex 后端 base URL。
    pub endpoint: String,
    /// 手动 access token；为空时使用本地 OAuth 凭证。
    pub api_key: String,
    /// Codex 模型 ID。
    pub model: String,
    /// Settings 中的代理 URL，空字符串表示直连。
    pub proxy: String,
    /// Settings 中已展开的 no_proxy 规则。
    pub no_proxy: String,
    /// 是否允许 WebSocket 连续失败后退回 HTTP/SSE。
    pub allow_http_fallback: bool,
}

impl CodexRemote {
    /// 创建默认 Codex 远端，适用于尚未接入配置 UI 的调用方。
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            endpoint: DEFAULT_CODEX_ENDPOINT.to_string(),
            api_key: String::new(),
            model: DEFAULT_CODEX_MODEL.to_string(),
            proxy: String::new(),
            no_proxy: String::new(),
            allow_http_fallback: false,
        }
    }
}

/// Codex API 客户端。
#[derive(Debug, Clone)]
pub struct CodexClient {
    /// Codex 远端配置。
    remote: CodexRemote,
    /// 复用的 HTTP 客户端。
    http: Client,
    /// 复用的 Codex transport 状态。
    transport: Arc<CodexTransportState>,
}

impl CodexClient {
    /// 创建 Codex 客户端，适用于调用方提供完整远端配置的场景。
    pub fn new(remote: CodexRemote) -> Self {
        Self {
            remote,
            http: Client::new(),
            transport: Arc::new(CodexTransportState::default()),
        }
    }

    /// 创建带自定义 HTTP 客户端的 Codex 客户端，适用于复用 UI 代理设置。
    pub fn with_http(remote: CodexRemote, http: Client) -> Self {
        Self {
            remote,
            http,
            transport: Arc::new(CodexTransportState::default()),
        }
    }

    /// 返回当前远端配置。
    pub fn remote(&self) -> &CodexRemote {
        &self.remote
    }

    /// 调用 Codex 并返回完整文本。
    pub async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse> {
        chat(&self.http, &self.remote, &self.transport, req).await
    }

    /// 调用 Codex 并返回流式文本块。
    pub async fn chat_stream(
        &self,
        req: &ChatRequest,
    ) -> Result<impl futures::Stream<Item = Result<String>>> {
        chat_stream(&self.http, &self.remote, &self.transport, req).await
    }
}

/// Codex transport 复用状态，适用于同一个 GUI 会话内的多次短请求。
#[derive(Default)]
struct CodexTransportState {
    /// WebSocket 连接；同一时间只允许一个 Responses stream 占用。
    websocket: AsyncMutex<Option<CodexWebSocketConnection>>,
    /// HTTP fallback 是否已被当前 session 激活。
    http_fallback_active: AtomicBool,
    /// 当前连续 WebSocket 失败次数。
    websocket_failure_streak: AtomicUsize,
}

impl fmt::Debug for CodexTransportState {
    /// Formats transport state without exposing live socket internals.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CodexTransportState")
            .field(
                "http_fallback_active",
                &self.http_fallback_active.load(Ordering::Relaxed),
            )
            .field(
                "websocket_failure_streak",
                &self.websocket_failure_streak.load(Ordering::Relaxed),
            )
            .finish_non_exhaustive()
    }
}

/// Codex Responses WebSocket 连接。
struct CodexWebSocketConnection {
    /// 底层 WebSocket stream。
    stream: WebSocketStream<MaybeTlsStream<CodexProxyStream>>,
}

/// Codex WebSocket 代理前置 TCP stream。
#[derive(Debug)]
enum CodexProxyStream {
    /// Direct TCP stream.
    Direct(TcpStream),
    /// SOCKS5-proxied TCP stream.
    Socks5(Socks5Stream<TcpStream>),
}

impl AsyncRead for CodexProxyStream {
    /// Delegates read polling to the active stream variant.
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            CodexProxyStream::Direct(stream) => Pin::new(stream).poll_read(cx, buf),
            CodexProxyStream::Socks5(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for CodexProxyStream {
    /// Delegates write polling to the active stream variant.
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            CodexProxyStream::Direct(stream) => Pin::new(stream).poll_write(cx, buf),
            CodexProxyStream::Socks5(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    /// Delegates flush polling to the active stream variant.
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            CodexProxyStream::Direct(stream) => Pin::new(stream).poll_flush(cx),
            CodexProxyStream::Socks5(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    /// Delegates shutdown polling to the active stream variant.
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            CodexProxyStream::Direct(stream) => Pin::new(stream).poll_shutdown(cx),
            CodexProxyStream::Socks5(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

/// Codex Responses 请求体，适用于 ChatGPT Codex 后端。
#[derive(Clone, serde::Serialize)]
struct CodexRequest {
    /// 模型 ID。
    model: String,
    /// 是否保存服务端响应。
    store: bool,
    /// 是否开启 SSE 流式响应。
    stream: bool,
    /// 顶层系统指令。
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    /// Responses API 输入项。
    input: Vec<Value>,
    /// 可用 function tools。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<Value>,
    /// 工具选择策略。
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<Value>,
    /// 是否允许并行工具调用。
    parallel_tool_calls: bool,
    /// 采样温度。
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    /// Responses 输出 token 上限，适用于短任务控制延迟和费用。
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    /// 思考配置。
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<Value>,
    /// 文本输出配置。
    text: Value,
    /// 附加返回字段。
    include: Vec<&'static str>,
    /// prompt 缓存 key。
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<String>,
}

/// Codex 单轮结果。
#[derive(Debug)]
enum CodexTurn {
    /// 最终文本响应。
    Final(ChatResponse),
    /// 模型请求调用工具。
    ToolCalls {
        /// 当前轮已输出文本。
        content: String,
        /// 当前轮工具调用。
        tool_calls: Vec<CodexToolCall>,
        /// 当前轮 Responses 输入项。
        response_items: Vec<Value>,
    },
}

/// Codex function tool 调用。
#[derive(Clone, Debug)]
struct CodexToolCall {
    /// Responses function_call item id。
    item_id: Option<String>,
    /// function call id。
    call_id: String,
    /// 工具名称。
    name: String,
    /// 工具参数 JSON 字符串。
    arguments: String,
}

/// 流式解析中的部分工具调用。
#[derive(Default)]
struct PartialCodexToolCall {
    /// Responses function_call item id。
    item_id: Option<String>,
    /// function call id。
    call_id: Option<String>,
    /// 工具名称。
    name: String,
    /// 工具参数 JSON 字符串。
    arguments: String,
}

/// Codex 流式单轮累积状态。
struct CodexStreamState {
    /// 已输出文本。
    content: String,
    /// 当前正在解析的文本块索引。
    current_text_index: Option<usize>,
    /// 当前正在解析的工具块索引。
    current_tool_index: Option<usize>,
    /// 已解析的工具调用。
    tool_calls: Vec<PartialCodexToolCall>,
    /// 本轮响应 item。
    response_items: Vec<Value>,
    /// 本轮 usage。
    usage: Option<Usage>,
    /// 是否收到完成事件。
    completed: bool,
}

impl CodexStreamState {
    /// 创建空的 Codex 流状态。
    fn new() -> Self {
        Self {
            content: String::new(),
            current_text_index: None,
            current_tool_index: None,
            tool_calls: Vec::new(),
            response_items: Vec::new(),
            usage: None,
            completed: false,
        }
    }
}

/// 调用 ChatGPT Codex 后端并返回完整文本。
async fn chat(
    client: &Client,
    remote: &CodexRemote,
    transport: &Arc<CodexTransportState>,
    req: &ChatRequest,
) -> Result<ChatResponse> {
    let credentials = resolve_remote_credentials(client, remote).await?;
    let instructions = split_instructions(&req.messages);
    let mut input = build_input(&req.messages);
    let tools = build_tools(&req.tools);
    let handlers = build_tool_handlers(&req.tools);

    for round in 0..=req.max_tool_rounds {
        let body = build_request(
            remote,
            req,
            instructions.clone(),
            input.clone(),
            tools.clone(),
        );
        let turn = send_stream_turn(client, remote, transport, &credentials, body).await?;
        match turn {
            CodexTurn::Final(response) => return Ok(response),
            CodexTurn::ToolCalls {
                content: _,
                tool_calls,
                response_items,
            } => {
                if round == req.max_tool_rounds {
                    return Err(AiError::Parse(format!(
                        "codex tool call loop exceeded max_tool_rounds={}",
                        req.max_tool_rounds
                    )));
                }
                input.extend(response_items);
                let mut continue_next_round = true;
                append_tool_results(
                    &mut input,
                    &handlers,
                    req.tool_observer.as_ref(),
                    round,
                    tool_calls,
                    &mut continue_next_round,
                )
                .await?;
                if !continue_next_round {
                    return Ok(ChatResponse {
                        content: String::new(),
                        model: remote.model.clone(),
                        usage: None,
                    });
                }
            }
        }
    }

    Err(AiError::Parse(
        "unreachable codex tool call loop state".into(),
    ))
}

/// 调用 ChatGPT Codex 后端并以文本块形式返回。
async fn chat_stream(
    client: &Client,
    remote: &CodexRemote,
    transport: &Arc<CodexTransportState>,
    req: &ChatRequest,
) -> Result<impl futures::Stream<Item = Result<String>>> {
    let client = client.clone();
    let remote = remote.clone();
    let transport = Arc::clone(transport);
    let req = req.clone();

    Ok(async_stream::try_stream! {
        let credentials = resolve_remote_credentials(&client, &remote).await?;
        let instructions = split_instructions(&req.messages);
        let mut input = build_input(&req.messages);
        let tools = build_tools(&req.tools);
        let handlers = build_tool_handlers(&req.tools);

        for round in 0..=req.max_tool_rounds {
            let body = build_request(&remote, &req, instructions.clone(), input.clone(), tools.clone());
            let turn = send_stream_turn(&client, &remote, &transport, &credentials, body).await?;
            match turn {
                CodexTurn::Final(response) => {
                    if !response.content.is_empty() {
                        yield response.content;
                    }
                    break;
                }
                CodexTurn::ToolCalls {
                    content,
                    tool_calls,
                    response_items,
                } => {
                    if !content.is_empty() {
                        yield content;
                    }
                    if round == req.max_tool_rounds {
                        Err(AiError::Parse(format!(
                            "codex streaming tool call loop exceeded max_tool_rounds={}",
                            req.max_tool_rounds
                        )))?;
                    }
                    input.extend(response_items);
                    let mut continue_next_round = true;
                    append_tool_results(
                        &mut input,
                        &handlers,
                        req.tool_observer.as_ref(),
                        round,
                        tool_calls,
                        &mut continue_next_round,
                    )
                    .await?;
                    if !continue_next_round {
                        break;
                    }
                }
            }
        }
    })
}

/// 构造 Codex 请求体。
fn build_request(
    remote: &CodexRemote,
    req: &ChatRequest,
    instructions: Option<String>,
    mut input: Vec<Value>,
    tools: Vec<Value>,
) -> CodexRequest {
    if input.is_empty() {
        // 触发条件：调用方把完整任务放在 Codex instructions 中。
        // 不能直接用空 input 的原因：Codex Responses API 会拒绝
        // 没有 input / previous_response_id / prompt / conversation_id 的请求。
        // 防止什么副作用或回归：不重复注入业务问题，只提供传输占位。
        input.push(serde_json::json!({
            "role": "user",
            "content": [{ "type": "input_text", "text": "执行 instructions 中的任务。" }],
        }));
    }
    CodexRequest {
        model: remote.model.clone(),
        store: false,
        stream: true,
        instructions: Some(instructions.unwrap_or_else(|| {
            "You are a helpful assistant. Follow the user's request exactly.".to_string()
        })),
        input,
        tools,
        tool_choice: Some(
            build_tool_choice(req.tool_choice.as_ref())
                .unwrap_or_else(|| serde_json::json!("auto")),
        ),
        parallel_tool_calls: true,
        temperature: req.temperature,
        max_output_tokens: req.max_completion_tokens.or(req.max_tokens),
        reasoning: thinking_value(req.thinking),
        text: serde_json::json!({ "verbosity": "medium" }),
        include: vec!["reasoning.encrypted_content"],
        prompt_cache_key: Some(remote.name.clone()),
    }
}

/// 把 system 消息合并为 Codex 顶层 instructions。
fn split_instructions(messages: &[Message]) -> Option<String> {
    let instructions = messages
        .iter()
        .filter(|message| message.role == Role::System)
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    (!instructions.is_empty()).then_some(instructions)
}

/// 把普通消息转换为 Responses 输入项。
fn build_input(messages: &[Message]) -> Vec<Value> {
    messages
        .iter()
        .filter_map(|message| match message.role {
            Role::System => None,
            Role::User => Some(serde_json::json!({
                "role": "user",
                "content": [{ "type": "input_text", "text": message.content }],
            })),
            Role::Assistant => Some(serde_json::json!({
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": message.content, "annotations": [] }],
                "status": "completed",
            })),
        })
        .collect()
}

/// 把本地工具转换为 Responses function tool。
fn build_tools(tools: &[ChatTool]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            serde_json::json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            })
        })
        .collect()
}

/// 把工具选择策略转换为 Responses tool_choice。
fn build_tool_choice(choice: Option<&ToolChoice>) -> Option<Value> {
    choice.map(|choice| match choice {
        ToolChoice::Auto => serde_json::json!("auto"),
        ToolChoice::None => serde_json::json!("none"),
        ToolChoice::Required => serde_json::json!("required"),
        ToolChoice::Function(name) => serde_json::json!({ "type": "function", "name": name }),
    })
}

/// 建立工具名称到处理函数的映射。
fn build_tool_handlers(tools: &[ChatTool]) -> BTreeMap<String, crate::ai::types::ToolHandler> {
    tools
        .iter()
        .map(|tool| (tool.name.clone(), tool.handler.clone()))
        .collect()
}

/// 把 read_ai thinking 模式转换为 Codex reasoning 字段。
fn thinking_value(mode: ThinkingMode) -> Option<Value> {
    match mode {
        ThinkingMode::Disabled => None,
        ThinkingMode::Enabled | ThinkingMode::Auto => {
            Some(serde_json::json!({ "effort": "medium", "summary": "auto" }))
        }
    }
}

/// 发送一轮 Codex 请求并收集结果。
async fn send_stream_turn(
    client: &Client,
    remote: &CodexRemote,
    transport: &Arc<CodexTransportState>,
    credentials: &CodexCredentials,
    body: CodexRequest,
) -> Result<CodexTurn> {
    if remote.allow_http_fallback && transport.http_fallback_active.load(Ordering::Relaxed) {
        return send_stream_turn_http(client, remote, credentials, body).await;
    }

    let mut last_error: Option<AiError> = None;
    for _ in 0..WS_FAILURES_BEFORE_HTTP_FALLBACK {
        match send_stream_turn_websocket(remote, transport, credentials, &body).await {
            Ok(turn) => {
                transport
                    .websocket_failure_streak
                    .store(0, Ordering::Relaxed);
                return Ok(turn);
            }
            Err(error) => {
                *transport.websocket.lock().await = None;
                let failures = transport
                    .websocket_failure_streak
                    .fetch_add(1, Ordering::Relaxed)
                    + 1;
                last_error = Some(error);
                if remote.allow_http_fallback && failures >= WS_FAILURES_BEFORE_HTTP_FALLBACK {
                    transport
                        .http_fallback_active
                        .store(true, Ordering::Relaxed);
                    return send_stream_turn_http(client, remote, credentials, body).await;
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| AiError::Stream("codex websocket request failed".into())))
}

/// 通过 Responses WebSocket 发送一轮 Codex 请求。
async fn send_stream_turn_websocket(
    remote: &CodexRemote,
    transport: &Arc<CodexTransportState>,
    credentials: &CodexCredentials,
    body: &CodexRequest,
) -> Result<CodexTurn> {
    let mut guard = transport.websocket.lock().await;
    if guard.is_none() {
        *guard = Some(connect_responses_websocket(remote, credentials).await?);
    }
    let connection = guard
        .as_mut()
        .ok_or_else(|| AiError::Stream("codex websocket connection unavailable".into()))?;
    connection.send_turn(remote, body).await
}

/// 发送一轮 Codex SSE 请求并收集结果。
async fn send_stream_turn_http(
    client: &Client,
    remote: &CodexRemote,
    credentials: &CodexCredentials,
    body: CodexRequest,
) -> Result<CodexTurn> {
    let headers = build_headers(remote, credentials)?;
    let url = build_url(remote);
    let body_json = serde_json::to_string(&body)?;
    let resp = send_with_retry(client, &url, headers, body_json).await?;

    let status = resp.status();
    if !status.is_success() {
        let text = read_response_text("codex", &url, status, resp).await?;
        return Err(AiError::Api {
            status: status.as_u16(),
            message: text,
        });
    }

    let mut state = CodexStreamState::new();
    let mut sse_buffer = String::new();
    let content_type = header_value(resp.headers(), reqwest::header::CONTENT_TYPE);
    let content_encoding = header_value(resp.headers(), reqwest::header::CONTENT_ENCODING);
    let mut byte_stream = resp.bytes_stream();

    while let Some(chunk) = byte_stream.next().await {
        let chunk = chunk.map_err(|error| {
            stream_body_error(
                "codex",
                &url,
                status,
                &content_type,
                &content_encoding,
                error,
            )
        })?;
        sse_buffer.push_str(&String::from_utf8_lossy(&chunk));
        let events = drain_sse_events(&mut sse_buffer)?;
        for event in events {
            let _ = absorb_event(&mut state, event)?;
        }
    }

    finish_turn(remote, state)
}

impl CodexWebSocketConnection {
    /// 发送一轮 Responses WebSocket 请求并等待完成事件。
    async fn send_turn(&mut self, remote: &CodexRemote, body: &CodexRequest) -> Result<CodexTurn> {
        let request_text = websocket_request_text(body)?;
        tokio::time::timeout(
            WEBSOCKET_IDLE_TIMEOUT,
            self.stream.send(WsMessage::Text(request_text.into())),
        )
        .await
        .map_err(|_| AiError::Stream("codex websocket send timed out".into()))?
        .map_err(map_websocket_error)?;

        let mut state = CodexStreamState::new();
        loop {
            let message = tokio::time::timeout(WEBSOCKET_IDLE_TIMEOUT, self.stream.next())
                .await
                .map_err(|_| AiError::Stream("codex websocket idle timeout".into()))?
                .ok_or_else(|| {
                    AiError::Stream("codex websocket closed before response.completed".into())
                })?
                .map_err(map_websocket_error)?;
            match message {
                WsMessage::Text(text) => {
                    let event = serde_json::from_str::<Value>(&text).map_err(|error| {
                        AiError::Parse(format!("invalid codex websocket event: {error}"))
                    })?;
                    let _ = absorb_event(&mut state, event)?;
                    if state.completed {
                        break;
                    }
                }
                WsMessage::Ping(payload) => {
                    self.stream
                        .send(WsMessage::Pong(payload))
                        .await
                        .map_err(map_websocket_error)?;
                }
                WsMessage::Pong(_) | WsMessage::Frame(_) => {}
                WsMessage::Binary(_) => {
                    return Err(AiError::Stream(
                        "unexpected binary codex websocket event".into(),
                    ));
                }
                WsMessage::Close(_) => {
                    return Err(AiError::Stream(
                        "codex websocket closed before response.completed".into(),
                    ));
                }
            }
        }
        finish_turn(remote, state)
    }
}

/// 建立 Codex Responses WebSocket 连接。
async fn connect_responses_websocket(
    remote: &CodexRemote,
    credentials: &CodexCredentials,
) -> Result<CodexWebSocketConnection> {
    let url = build_websocket_url(remote);
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|error| AiError::Stream(format!("failed to build websocket request: {error}")))?;
    request
        .headers_mut()
        .extend(build_websocket_headers(remote, credentials)?);
    let stream = connect_websocket_tcp_stream(remote, &url).await?;
    let (stream, _response) = client_async_tls_with_config(request, stream, None, None)
        .await
        .map_err(map_websocket_error)?;
    Ok(CodexWebSocketConnection { stream })
}

/// Opens the TCP layer for WebSocket, honoring Settings proxy and no_proxy.
async fn connect_websocket_tcp_stream(
    remote: &CodexRemote,
    websocket_url: &str,
) -> Result<CodexProxyStream> {
    let target_url = reqwest::Url::parse(websocket_url)
        .map_err(|error| AiError::Stream(format!("invalid websocket URL: {error}")))?;
    let target_host = target_url
        .host_str()
        .ok_or_else(|| AiError::Stream("websocket URL missing host".into()))?;
    let target_port = target_url
        .port_or_known_default()
        .ok_or_else(|| AiError::Stream("websocket URL missing port and known default".into()))?;
    let target = format!("{target_host}:{target_port}");

    let proxy = remote.proxy.trim();
    if proxy.is_empty() || no_proxy_matches(target_host, &remote.no_proxy) {
        let stream = TcpStream::connect(&target).await.map_err(|error| {
            AiError::Stream(format!(
                "failed to connect websocket target {target}: {error}"
            ))
        })?;
        return Ok(CodexProxyStream::Direct(stream));
    }

    let proxy_url = reqwest::Url::parse(proxy)
        .map_err(|error| AiError::Stream(format!("invalid websocket proxy URL: {error}")))?;
    let scheme = proxy_url.scheme().to_ascii_lowercase();
    if scheme.starts_with("socks5") {
        return connect_socks5_websocket_stream(&proxy_url, target_host, target_port).await;
    }
    connect_http_proxy_websocket_stream(&proxy_url, target_host, target_port).await
}

/// Opens a SOCKS5 proxied stream to the WebSocket target.
async fn connect_socks5_websocket_stream(
    proxy_url: &reqwest::Url,
    target_host: &str,
    target_port: u16,
) -> Result<CodexProxyStream> {
    let proxy_addr = proxy_socket_addr(proxy_url, 1080)?;
    let target = (target_host, target_port);
    let username = proxy_url.username();
    let stream = if username.is_empty() {
        Socks5Stream::connect(proxy_addr.as_str(), target).await
    } else {
        Socks5Stream::connect_with_password(
            proxy_addr.as_str(),
            target,
            username,
            proxy_url.password().unwrap_or_default(),
        )
        .await
    }
    .map_err(|error| {
        AiError::Stream(format!("failed to connect websocket SOCKS5 proxy: {error}"))
    })?;
    Ok(CodexProxyStream::Socks5(stream))
}

/// Opens an HTTP CONNECT tunnel to the WebSocket target.
async fn connect_http_proxy_websocket_stream(
    proxy_url: &reqwest::Url,
    target_host: &str,
    target_port: u16,
) -> Result<CodexProxyStream> {
    let proxy_addr = proxy_socket_addr(proxy_url, 80)?;
    let mut stream = TcpStream::connect(&proxy_addr).await.map_err(|error| {
        AiError::Stream(format!(
            "failed to connect websocket HTTP proxy {proxy_addr}: {error}"
        ))
    })?;
    let target = format!("{target_host}:{target_port}");
    let mut request = format!(
        "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\nProxy-Connection: Keep-Alive\r\nUser-Agent: {}\r\n",
        codex_user_agent()
    );
    if !proxy_url.username().is_empty() {
        let auth = format!(
            "{}:{}",
            proxy_url.username(),
            proxy_url.password().unwrap_or_default()
        );
        let encoded = base64::engine::general_purpose::STANDARD.encode(auth);
        request.push_str(&format!("Proxy-Authorization: Basic {encoded}\r\n"));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|error| {
            AiError::Stream(format!("failed to send websocket proxy CONNECT: {error}"))
        })?;

    let mut response = Vec::new();
    let mut buffer = [0_u8; 1024];
    while response.len() < 8192 && !response.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut buffer).await.map_err(|error| {
            AiError::Stream(format!("failed to read websocket proxy CONNECT: {error}"))
        })?;
        if read == 0 {
            break;
        }
        response.extend_from_slice(&buffer[..read]);
    }
    let response_text = String::from_utf8_lossy(&response);
    if !response_text.starts_with("HTTP/1.1 200") && !response_text.starts_with("HTTP/1.0 200") {
        return Err(AiError::Stream(format!(
            "websocket proxy CONNECT failed: {}",
            response_text.lines().next().unwrap_or("<empty response>")
        )));
    }
    Ok(CodexProxyStream::Direct(stream))
}

/// Returns proxy host:port with a scheme-specific default port.
fn proxy_socket_addr(proxy_url: &reqwest::Url, default_port: u16) -> Result<String> {
    let host = proxy_url
        .host_str()
        .ok_or_else(|| AiError::Stream("proxy URL missing host".into()))?;
    let port = proxy_url.port().unwrap_or(default_port);
    Ok(format!("{host}:{port}"))
}

/// Checks a small no_proxy subset used by Settings proxy bypass.
fn no_proxy_matches(host: &str, no_proxy: &str) -> bool {
    no_proxy.split(',').map(str::trim).any(|entry| {
        if entry.is_empty() {
            return false;
        }
        if entry == "*" || entry.eq_ignore_ascii_case(host) {
            return true;
        }
        let normalized = entry.trim_start_matches('.');
        host.ends_with(&format!(".{normalized}"))
    })
}

/// 构造 Responses WebSocket 请求 JSON。
fn websocket_request_text(body: &CodexRequest) -> Result<String> {
    let mut value = serde_json::to_value(body)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| AiError::Parse("codex websocket request is not an object".into()))?;
    object.insert(
        "type".to_string(),
        serde_json::Value::String("response.create".to_string()),
    );
    Ok(serde_json::to_string(&value)?)
}

/// 将 HTTP Responses URL 转成 WebSocket URL。
fn build_websocket_url(remote: &CodexRemote) -> String {
    let url = build_url(remote);
    if let Some(rest) = url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        url
    }
}

/// 构造 Codex Responses WebSocket 握手头。
fn build_websocket_headers(
    remote: &CodexRemote,
    credentials: &CodexCredentials,
) -> Result<reqwest::header::HeaderMap> {
    let mut headers = reqwest::header::HeaderMap::new();
    insert_header(
        &mut headers,
        "Authorization",
        &format!("Bearer {}", credentials.access),
    )?;
    insert_header(&mut headers, "chatgpt-account-id", &credentials.account_id)?;
    insert_header(
        &mut headers,
        "OpenAI-Beta",
        RESPONSES_WEBSOCKETS_V2_BETA_HEADER_VALUE,
    )?;
    insert_header(&mut headers, "originator", "pi")?;
    insert_header(&mut headers, "User-Agent", &codex_user_agent())?;
    insert_header(&mut headers, "session_id", &remote.name)?;
    insert_header(&mut headers, "session-id", &remote.name)?;
    insert_header(&mut headers, "thread-id", &remote.name)?;
    insert_header(&mut headers, "x-client-request-id", &remote.name)?;
    Ok(headers)
}

/// 将 WebSocket 错误转成现有 AI 错误类型。
fn map_websocket_error(error: WsError) -> AiError {
    match error {
        WsError::Http(response) => AiError::Api {
            status: response.status().as_u16(),
            message: format!("codex websocket upgrade failed: {}", response.status()),
        },
        WsError::ConnectionClosed | WsError::AlreadyClosed => {
            AiError::Stream("codex websocket closed".into())
        }
        WsError::Io(error) => AiError::Stream(format!("codex websocket io error: {error}")),
        other => AiError::Stream(format!("codex websocket error: {other}")),
    }
}

/// 按 Pi 的策略发送 Codex 请求并处理瞬时错误重试。
async fn send_with_retry(
    client: &Client,
    url: &str,
    headers: reqwest::header::HeaderMap,
    body: String,
) -> Result<reqwest::Response> {
    let mut last_error: Option<AiError> = None;
    for attempt in 0..=MAX_RETRIES {
        let resp = client
            .post(url)
            .headers(headers.clone())
            .body(body.clone())
            .send()
            .await
            .map_err(|error| {
                request_send_error("codex", url, "send responses request", attempt, error)
            })?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let text = read_response_text("codex", url, status, resp).await?;
        if attempt < MAX_RETRIES && is_retryable_error(status.as_u16(), &text) {
            tokio::time::sleep(Duration::from_millis(
                BASE_RETRY_DELAY_MS * 2_u64.pow(attempt as u32),
            ))
            .await;
            continue;
        }
        last_error = Some(AiError::Api {
            status: status.as_u16(),
            message: text,
        });
        break;
    }
    Err(last_error.unwrap_or_else(|| AiError::Stream("codex request failed after retries".into())))
}

/// 判断 Codex 错误是否适合重试。
fn is_retryable_error(status: u16, text: &str) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
        || text.to_ascii_lowercase().contains("rate limit")
        || text.contains("overloaded")
        || text.contains("service unavailable")
        || text.contains("upstream connect")
        || text.contains("connection refused")
}

/// 消费 SSE 缓冲区中的完整事件。
fn drain_sse_events(buffer: &mut String) -> Result<Vec<Value>> {
    let mut events = Vec::new();
    while let Some(pos) = buffer.find("\n\n") {
        let chunk = buffer[..pos].to_string();
        *buffer = buffer[pos + 2..].to_string();
        let data = chunk
            .lines()
            .filter_map(|line| line.trim().strip_prefix("data:"))
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let event = serde_json::from_str::<Value>(&data)
            .map_err(|error| AiError::Parse(format!("invalid codex stream chunk: {error}")))?;
        events.push(event);
    }
    Ok(events)
}

/// 把一个 Responses 事件吸收到当前轮状态。
fn absorb_event(state: &mut CodexStreamState, event: Value) -> Result<Option<String>> {
    let event_type = event["type"].as_str().unwrap_or_default();
    match event_type {
        "response.output_item.added" => absorb_output_item_added(state, &event),
        "response.output_text.delta" => {
            let delta = event["delta"].as_str().unwrap_or_default().to_string();
            state.content.push_str(&delta);
            Ok(Some(delta))
        }
        "response.function_call_arguments.delta" => {
            if let Some(index) = state.current_tool_index
                && let Some(delta) = event["delta"].as_str()
            {
                state.tool_calls[index].arguments.push_str(delta);
            }
            Ok(None)
        }
        "response.function_call_arguments.done" => {
            if let Some(index) = state.current_tool_index
                && let Some(arguments) = event["arguments"].as_str()
            {
                state.tool_calls[index].arguments = arguments.to_string();
            }
            Ok(None)
        }
        "response.output_item.done" => absorb_output_item_done(state, &event),
        "response.completed" | "response.done" => {
            state.completed = true;
            state.usage = parse_usage(&event["response"]);
            Ok(None)
        }
        "error" => Err(AiError::Api {
            status: 200,
            message: parse_codex_error_message(&event),
        }),
        "response.failed" => Err(AiError::Api {
            status: 200,
            message: parse_codex_error_message(&event),
        }),
        _ => Ok(None),
    }
}

/// 提取 Codex SSE 错误消息。
fn parse_codex_error_message(event: &Value) -> String {
    event["error"]["message"]
        .as_str()
        .or_else(|| event["response"]["error"]["message"].as_str())
        .or_else(|| event["message"].as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| event.to_string())
}

/// 处理 output item 开始事件。
fn absorb_output_item_added(state: &mut CodexStreamState, event: &Value) -> Result<Option<String>> {
    let item = &event["item"];
    match item["type"].as_str() {
        Some("message") => {
            state.current_text_index = Some(state.response_items.len());
            state.response_items.push(serde_json::json!({
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": "", "annotations": [] }],
                "status": "completed",
            }));
        }
        Some("function_call") => {
            let call = PartialCodexToolCall {
                item_id: item["id"].as_str().map(ToString::to_string),
                call_id: item["call_id"].as_str().map(ToString::to_string),
                name: item["name"].as_str().unwrap_or_default().to_string(),
                arguments: item["arguments"].as_str().unwrap_or_default().to_string(),
            };
            state.current_tool_index = Some(state.tool_calls.len());
            state.tool_calls.push(call);
        }
        Some("reasoning") => {
            state.current_text_index = None;
        }
        _ => {}
    }
    Ok(None)
}

/// 处理 output item 结束事件。
fn absorb_output_item_done(state: &mut CodexStreamState, event: &Value) -> Result<Option<String>> {
    let item = &event["item"];
    match item["type"].as_str() {
        Some("message") => {
            if let Some(index) = state.current_text_index {
                state.response_items[index] = item.clone();
            }
        }
        Some("function_call") => {
            let item_id = normalized_function_item_id(item);
            let call_id = normalized_function_call_id(item);
            let name = item["name"].as_str().unwrap_or_default().to_string();
            let arguments = item["arguments"].as_str().unwrap_or_default().to_string();
            if let Some(index) = state.current_tool_index {
                let call = &mut state.tool_calls[index];
                if item_id.is_some() {
                    call.item_id = item_id.clone();
                }
                if call_id.is_some() {
                    call.call_id = call_id.clone();
                }
                if !name.is_empty() {
                    call.name = name.clone();
                }
                if !arguments.is_empty() {
                    call.arguments = arguments.clone();
                }
            }
            let mut response_item = item.clone();
            if let Some(object) = response_item.as_object_mut() {
                if let Some(item_id) = item_id.as_ref() {
                    object.insert("id".to_string(), serde_json::json!(item_id));
                }
                if let Some(call_id) = call_id.as_ref() {
                    object.insert("call_id".to_string(), serde_json::json!(call_id));
                }
            }
            state.response_items.push(response_item);
        }
        Some("reasoning") => {}
        _ => {}
    }
    Ok(None)
}

/// 结束一轮 Codex 解析。
fn finish_turn(remote: &CodexRemote, state: CodexStreamState) -> Result<CodexTurn> {
    if !state.completed {
        return Err(AiError::Stream(
            "codex stream closed before response.completed".into(),
        ));
    }
    let tool_calls = finish_tool_calls(state.tool_calls)?;
    if tool_calls.is_empty() {
        return Ok(CodexTurn::Final(ChatResponse {
            content: state.content,
            model: remote.model.clone(),
            usage: state.usage,
        }));
    }
    Ok(CodexTurn::ToolCalls {
        content: state.content,
        tool_calls,
        response_items: state.response_items,
    })
}

/// 完成部分工具调用的必填字段校验。
fn finish_tool_calls(calls: Vec<PartialCodexToolCall>) -> Result<Vec<CodexToolCall>> {
    calls
        .into_iter()
        .map(|call| {
            Ok(CodexToolCall {
                item_id: call.item_id,
                call_id: call
                    .call_id
                    .ok_or_else(|| AiError::Parse("codex function_call missing call_id".into()))?,
                name: call.name,
                arguments: call.arguments,
            })
        })
        .collect()
}

/// 标准化 Codex function_call item id。
fn normalized_function_item_id(item: &Value) -> Option<String> {
    item["id"]
        .as_str()
        .map(sanitize_responses_id)
        .map(|id| ensure_prefix(id, "fc_"))
}

/// 标准化 Codex function_call call_id。
fn normalized_function_call_id(item: &Value) -> Option<String> {
    item["call_id"].as_str().map(sanitize_responses_id)
}

/// Responses id 只保留 OpenAI 接受的字符，并限制长度。
fn sanitize_responses_id(value: &str) -> String {
    let mut output = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if output.len() > 64 {
        output.truncate(64);
    }
    while output.ends_with('_') {
        output.pop();
    }
    output
}

/// 确保 Responses item id 带指定前缀。
fn ensure_prefix(value: String, prefix: &str) -> String {
    if value.starts_with(prefix) {
        value
    } else {
        format!("{prefix}{value}")
    }
}

/// 执行工具调用并追加 function_call_output。
async fn append_tool_results(
    input: &mut Vec<Value>,
    handlers: &BTreeMap<String, crate::ai::types::ToolHandler>,
    observer: Option<&crate::ai::types::ToolObserver>,
    round: usize,
    tool_calls: Vec<CodexToolCall>,
    continue_next_round: &mut bool,
) -> Result<()> {
    for (call_index, tool_call) in tool_calls.into_iter().enumerate() {
        let output = execute_tool_call(handlers, observer, round, call_index, &tool_call).await?;
        *continue_next_round &= output.continue_next_round;
        input.push(serde_json::json!({
            "type": "function_call_output",
            "call_id": tool_call.call_id,
            "output": output.content,
        }));
    }
    Ok(())
}

/// 执行单个 Codex function tool。
async fn execute_tool_call(
    handlers: &BTreeMap<String, crate::ai::types::ToolHandler>,
    observer: Option<&crate::ai::types::ToolObserver>,
    round: usize,
    call_index: usize,
    tool_call: &CodexToolCall,
) -> Result<ToolResult> {
    let Some(handler) = handlers.get(&tool_call.name) else {
        return Err(AiError::Parse(format!(
            "tool `{}` was requested but no handler is registered",
            tool_call.name
        )));
    };
    let arguments = serde_json::from_str::<Value>(&tool_call.arguments).map_err(|error| {
        AiError::Parse(format!(
            "invalid arguments for tool `{}`: {error}",
            tool_call.name
        ))
    })?;
    if let Some(observer) = observer {
        observer(ToolCallObservation {
            id: tool_call
                .item_id
                .clone()
                .unwrap_or_else(|| tool_call.call_id.clone()),
            round,
            call_index,
            name: tool_call.name.clone(),
            arguments: arguments.clone(),
        })
        .await?;
    }
    handler(arguments).await
}

/// 解析 Responses usage。
fn parse_usage(response: &Value) -> Option<Usage> {
    let input = response["usage"]["input_tokens"].as_u64()?;
    let output = response["usage"]["output_tokens"].as_u64()?;
    Some(Usage {
        prompt_tokens: input as u32,
        completion_tokens: output as u32,
    })
}

/// 构造 Codex 请求 URL。
pub(crate) fn build_url(remote: &CodexRemote) -> String {
    let base = remote.endpoint.trim_end_matches('/');
    if base.ends_with("/codex/responses") {
        base.to_string()
    } else if base.ends_with("/codex") {
        format!("{base}/responses")
    } else {
        format!("{base}/codex/responses")
    }
}

/// 构造 Codex 专用请求头。
fn build_headers(
    remote: &CodexRemote,
    credentials: &CodexCredentials,
) -> Result<reqwest::header::HeaderMap> {
    let mut headers = reqwest::header::HeaderMap::new();
    insert_header(
        &mut headers,
        "Authorization",
        &format!("Bearer {}", credentials.access),
    )?;
    insert_header(&mut headers, "chatgpt-account-id", &credentials.account_id)?;
    insert_header(&mut headers, "OpenAI-Beta", "responses=experimental")?;
    insert_header(&mut headers, "originator", "pi")?;
    insert_header(&mut headers, "accept", "text/event-stream")?;
    insert_header(&mut headers, "content-type", "application/json")?;
    insert_header(&mut headers, "User-Agent", &codex_user_agent())?;
    insert_header(&mut headers, "session_id", &remote.name)?;
    Ok(headers)
}

/// 构造与 Pi 形态一致的 Codex User-Agent。
fn codex_user_agent() -> String {
    format!(
        "pi ({} {}; {})",
        std::env::consts::OS,
        std::env::consts::FAMILY,
        std::env::consts::ARCH
    )
}

/// 解析 remote 手动 token，或走本地 Codex OAuth 凭证。
async fn resolve_remote_credentials(
    client: &Client,
    remote: &CodexRemote,
) -> Result<CodexCredentials> {
    if remote.api_key.trim().is_empty() {
        return resolve_credentials(client).await;
    }
    Ok(CodexCredentials {
        access: remote.api_key.clone(),
        refresh: String::new(),
        expires: u64::MAX,
        account_id: extract_account_id(&remote.api_key)?,
    })
}

/// 插入 HTTP 头并统一错误格式。
fn insert_header(
    headers: &mut reqwest::header::HeaderMap,
    name: &'static str,
    value: &str,
) -> Result<()> {
    let value = reqwest::header::HeaderValue::from_str(value)
        .map_err(|error| AiError::Parse(format!("invalid codex header `{name}`: {error}")))?;
    headers.insert(name, value);
    Ok(())
}

/// 从 ChatGPT OAuth JWT 中提取 account id。
pub(crate) fn extract_account_id(token: &str) -> Result<String> {
    let json = decode_jwt_payload(token)?;
    json["https://api.openai.com/auth"]["chatgpt_account_id"]
        .as_str()
        .map(ToString::to_string)
        .ok_or_else(|| AiError::Parse("codex JWT missing chatgpt_account_id".into()))
}

/// 从 ChatGPT OAuth JWT 中提取 profile email。
pub(crate) fn extract_profile_email(token: &str) -> Result<Option<String>> {
    let json = decode_jwt_payload(token)?;
    Ok(json["https://api.openai.com/profile"]["email"]
        .as_str()
        .map(ToString::to_string))
}

/// 解码 JWT payload 为 JSON。
fn decode_jwt_payload(token: &str) -> Result<Value> {
    let mut parts = token.split('.');
    let _header = parts.next();
    let Some(payload) = parts.next() else {
        return Err(AiError::Parse("codex token is not a JWT".into()));
    };
    if parts.next().is_none() {
        return Err(AiError::Parse("codex token is not a JWT".into()));
    }
    let bytes = decode_base64_url(payload)?;
    serde_json::from_slice::<Value>(&bytes)
        .map_err(|error| AiError::Parse(format!("invalid codex JWT payload: {error}")))
}

/// 解码 JWT 使用的 base64url 片段。
fn decode_base64_url(input: &str) -> Result<Vec<u8>> {
    let mut bits = 0u32;
    let mut bit_count = 0u8;
    let mut output = Vec::new();
    for byte in input.bytes() {
        if byte == b'=' {
            break;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => {
                return Err(AiError::Parse(format!(
                    "invalid base64url byte in codex token: {byte}"
                )));
            }
        } as u32;
        bits = (bits << 6) | value;
        bit_count += 6;
        while bit_count >= 8 {
            bit_count -= 8;
            output.push((bits >> bit_count) as u8);
            bits &= (1 << bit_count) - 1;
        }
    }
    Ok(output)
}

use thiserror::Error;

use std::error::Error as _;

/// AI 调用错误，适用于 Codex HTTP、SSE、OAuth 和解析路径。
#[derive(Debug, Error)]
pub enum AiError {
    /// 未配置可用远端。
    #[error("no active remote configured")]
    NoActiveRemote,

    /// HTTP 请求阶段失败。
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),

    /// 服务端返回非成功状态或 SSE error。
    #[error("API error ({status}): {message}")]
    Api {
        /// HTTP 状态码，SSE 内部错误用 200 表示。
        status: u16,
        /// 错误正文或事件消息。
        message: String,
    },

    /// 响应解析失败。
    #[error("failed to parse response: {0}")]
    Parse(String),

    /// 流式响应读取失败。
    #[error("stream error: {0}")]
    Stream(String),

    /// 本地配置或凭证 I/O 失败。
    #[error("config I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// 本地配置或凭证 JSON 失败。
    #[error("config JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// AI 调用结果类型。
pub type Result<T> = std::result::Result<T, AiError>;

/// 读取响应 body，并在解码失败时保留响应上下文。
pub(crate) async fn read_response_text(
    provider: &str,
    url: &str,
    status: reqwest::StatusCode,
    resp: reqwest::Response,
) -> Result<String> {
    let content_type = header_value(resp.headers(), reqwest::header::CONTENT_TYPE);
    let content_encoding = header_value(resp.headers(), reqwest::header::CONTENT_ENCODING);
    match resp.bytes().await {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Err(error) => Err(response_body_error(
            provider,
            url,
            status,
            &content_type,
            &content_encoding,
            error,
        )),
    }
}

/// 构造流式响应 body 读取错误。
pub(crate) fn stream_body_error(
    provider: &str,
    url: &str,
    status: reqwest::StatusCode,
    content_type: &str,
    content_encoding: &str,
    error: reqwest::Error,
) -> AiError {
    response_body_error(provider, url, status, content_type, content_encoding, error)
}

/// 构造 HTTP 发送阶段错误，保留 reqwest 分类和 source 链。
pub(crate) fn request_send_error(
    provider: &str,
    url: &str,
    phase: &str,
    attempt: usize,
    error: reqwest::Error,
) -> AiError {
    let source_chain = error_source_chain(&error);
    AiError::Stream(format!(
        "{provider} HTTP request failed: phase={phase} attempt={attempt} url={url} is_timeout={} is_connect={} is_request={} is_body={} is_decode={} error={} source_chain={}",
        error.is_timeout(),
        error.is_connect(),
        error.is_request(),
        error.is_body(),
        error.is_decode(),
        error,
        source_chain
    ))
}

/// 提取用于错误诊断的响应头。
pub(crate) fn header_value(
    headers: &reqwest::header::HeaderMap,
    name: reqwest::header::HeaderName,
) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("<missing>")
        .to_string()
}

/// 统一格式化响应 body 读取错误。
fn response_body_error(
    provider: &str,
    url: &str,
    status: reqwest::StatusCode,
    content_type: &str,
    content_encoding: &str,
    error: reqwest::Error,
) -> AiError {
    let source = error
        .source()
        .map(|source| source.to_string())
        .unwrap_or_else(|| "<none>".to_string());
    AiError::Stream(format!(
        "{provider} response body read failed: url={url} status={} content-type={} content-encoding={} is_decode={} is_timeout={} error={} source={}",
        status.as_u16(),
        content_type,
        content_encoding,
        error.is_decode(),
        error.is_timeout(),
        error,
        source
    ))
}

/// 展开错误 source 链，方便定位 TLS、DNS、连接和代理问题。
fn error_source_chain(error: &dyn std::error::Error) -> String {
    let mut sources = Vec::new();
    let mut current = error.source();
    while let Some(source) = current {
        sources.push(source.to_string());
        current = source.source();
    }
    if sources.is_empty() {
        "<none>".to_string()
    } else {
        sources.join(" -> ")
    }
}

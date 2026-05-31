use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};

use crate::ai::codex::{extract_account_id, extract_profile_email};
use crate::ai::error::{AiError, Result, request_send_error};

/// ChatGPT Codex OAuth client id，来自 Codex/Pi 公共 OAuth 流。
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// ChatGPT OAuth 授权地址。
const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
/// ChatGPT OAuth token 地址。
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
/// 本地 OAuth 回调地址。
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
/// OAuth scope。
const SCOPE: &str = "openid profile email offline_access";
/// token endpoint 请求超时时间。
const TOKEN_REQUEST_TIMEOUT_SECONDS: u64 = 30;

/// Codex OAuth 凭证。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CodexCredentials {
    /// access token。
    pub(crate) access: String,
    /// refresh token。
    pub(crate) refresh: String,
    /// 过期时间，Unix 毫秒。
    pub(crate) expires: u64,
    /// ChatGPT account id。
    pub(crate) account_id: String,
}

/// Codex 登录展示信息。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexAuthInfo {
    /// ChatGPT account id。
    pub account_id: String,
    /// access token 中的邮箱。
    pub email: Option<String>,
    /// 过期时间，Unix 毫秒。
    pub expires: u64,
}

/// OAuth 授权流的临时参数。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexAuthFlow {
    /// PKCE verifier。
    pub(crate) verifier: String,
    /// CSRF state。
    pub(crate) state: String,
    /// 用户需要打开的授权 URL。
    pub url: String,
}

/// token 接口返回体。
#[derive(Deserialize)]
struct TokenResponse {
    /// access token。
    access_token: String,
    /// refresh token。
    refresh_token: String,
    /// 有效期秒数。
    expires_in: u64,
}

/// 获取可用 Codex OAuth 凭证，必要时自动刷新或登录。
pub(crate) async fn resolve_credentials(client: &Client) -> Result<CodexCredentials> {
    if let Some(credentials) = load_credentials() {
        if credentials.expires > now_ms() {
            return Ok(credentials);
        }
        match refresh_credentials(client, &credentials.refresh).await {
            Ok(refreshed) => {
                save_credentials(&refreshed)?;
                return Ok(refreshed);
            }
            Err(error) => {
                log::warn!("failed to refresh codex credentials: {error}");
            }
        }
    }

    let credentials = login(client).await?;
    save_credentials(&credentials)?;
    Ok(credentials)
}

/// 读取已保存的 Codex 登录展示信息。
pub fn load_cached_auth_info() -> Option<CodexAuthInfo> {
    load_credentials().and_then(|credentials| auth_info_from_credentials(&credentials).ok())
}

/// 创建浏览器 OAuth 流，适用于 GUI 弹窗展示授权 URL。
pub fn start_browser_auth_flow() -> CodexAuthFlow {
    create_authorization_flow()
}

/// 打开 OAuth 授权 URL。
pub fn open_auth_browser(flow: &CodexAuthFlow) -> Result<()> {
    webbrowser::open(&flow.url)
        .map(|_| ())
        .map_err(|error| AiError::Stream(format!("failed to open auth browser: {error}")))
}

/// 等待浏览器 OAuth 回调并保存凭证。
pub async fn complete_browser_auth_flow(
    client: &Client,
    flow: CodexAuthFlow,
) -> Result<CodexAuthInfo> {
    let wait = wait_for_browser_code_after_open(&flow).await?;
    let Some(code) = wait else {
        return Err(AiError::Stream(
            "codex authorization callback timed out".into(),
        ));
    };
    let credentials = exchange_authorization_code(client, &code, &flow.verifier).await?;
    save_credentials(&credentials)?;
    auth_info_from_credentials(&credentials)
}

/// 先绑定回调端口，再打开浏览器并等待授权码。
async fn wait_for_browser_code_after_open(flow: &CodexAuthFlow) -> Result<Option<String>> {
    let listener = match TcpListener::bind("127.0.0.1:1455").await {
        Ok(listener) => listener,
        Err(error) => {
            log::warn!("failed to bind OAuth callback server: {error}");
            return Ok(None);
        }
    };
    let open_flow = flow.clone();
    let _ = tokio::task::spawn_blocking(move || open_auth_browser(&open_flow)).await;
    wait_for_browser_code_with_listener(listener, &flow.state).await
}

/// 发起完整 Codex OAuth 登录流程。
async fn login(client: &Client) -> Result<CodexCredentials> {
    let flow = create_authorization_flow();
    eprintln!("Open this URL to authorize Codex:\n{}\n", flow.url);
    if let Err(error) = webbrowser::open(&flow.url) {
        eprintln!("failed to open browser automatically: {error}");
    }
    eprintln!("Waiting for browser authorization callback on {REDIRECT_URI} ...");

    let code = match wait_for_browser_code(&flow.state).await? {
        Some(code) => {
            eprintln!("Codex authorization callback received.");
            code
        }
        None => {
            eprintln!("Paste the authorization code or full redirect URL:");
            read_manual_authorization_code(&flow.state).await?
        }
    };
    exchange_authorization_code(client, &code, &flow.verifier).await
}

/// 生成 OAuth 授权 URL 和 PKCE 参数。
fn create_authorization_flow() -> CodexAuthFlow {
    let verifier = random_base64url(32);
    let challenge = sha256_base64url(verifier.as_bytes());
    let state = random_hex(16);
    let mut query = Vec::new();
    push_query(&mut query, "response_type", "code");
    push_query(&mut query, "client_id", CLIENT_ID);
    push_query(&mut query, "redirect_uri", REDIRECT_URI);
    push_query(&mut query, "scope", SCOPE);
    push_query(&mut query, "code_challenge", &challenge);
    push_query(&mut query, "code_challenge_method", "S256");
    push_query(&mut query, "state", &state);
    push_query(&mut query, "id_token_add_organizations", "true");
    push_query(&mut query, "codex_cli_simplified_flow", "true");
    push_query(&mut query, "originator", "pi");
    CodexAuthFlow {
        verifier,
        state,
        url: format!("{AUTHORIZE_URL}?{}", query.join("&")),
    }
}

/// 等待浏览器回调里的 authorization code。
async fn wait_for_browser_code(state: &str) -> Result<Option<String>> {
    let listener = match TcpListener::bind("127.0.0.1:1455").await {
        Ok(listener) => listener,
        Err(error) => {
            log::warn!("failed to bind OAuth callback server: {error}");
            return Ok(None);
        }
    };
    wait_for_browser_code_with_listener(listener, state).await
}

/// 使用已绑定的 listener 等待浏览器回调里的 authorization code。
async fn wait_for_browser_code_with_listener(
    listener: TcpListener,
    state: &str,
) -> Result<Option<String>> {
    let state = state.to_string();
    let wait = async move {
        loop {
            let (mut stream, _) = listener.accept().await?;
            let mut buffer = vec![0; 8192];
            let n = stream.read(&mut buffer).await?;
            let request = String::from_utf8_lossy(&buffer[..n]);
            let Some(first_line) = request.lines().next() else {
                write_http_response(&mut stream, 400, "Bad Request").await?;
                continue;
            };
            let Some(path) = first_line.split_whitespace().nth(1) else {
                write_http_response(&mut stream, 400, "Bad Request").await?;
                continue;
            };
            match parse_callback_path(path, &state) {
                Ok(code) => {
                    write_http_response(
                        &mut stream,
                        200,
                        "Authentication successful. Return to your terminal.",
                    )
                    .await?;
                    return Ok(Some(code));
                }
                Err(message) => {
                    write_http_response(&mut stream, 400, &message.to_string()).await?;
                }
            }
        }
    };

    match timeout(Duration::from_secs(60), wait).await {
        Ok(result) => result,
        Err(_) => Ok(None),
    }
}

/// 写入最小 HTTP 响应。
async fn write_http_response(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    body: &str,
) -> Result<()> {
    let reason = if status == 200 { "OK" } else { "Bad Request" };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: text/plain; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

/// 从回调 path 中解析 code 并校验 state。
pub(crate) fn parse_callback_path(path: &str, expected_state: &str) -> Result<String> {
    let Some((route, query)) = path.split_once('?') else {
        return Err(AiError::Parse("missing OAuth callback query".into()));
    };
    if route != "/auth/callback" {
        return Err(AiError::Parse("unexpected OAuth callback path".into()));
    }
    let code = query_param(query, "code");
    let state = query_param(query, "state");
    if state.as_deref() != Some(expected_state) {
        return Err(AiError::Parse("OAuth state mismatch".into()));
    }
    code.ok_or_else(|| AiError::Parse("missing OAuth authorization code".into()))
}

/// 从用户粘贴的 code 或 redirect URL 中解析授权码。
async fn read_manual_authorization_code(expected_state: &str) -> Result<String> {
    let input = tokio::task::spawn_blocking(|| {
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        Ok::<_, std::io::Error>(input)
    })
    .await
    .map_err(|error| AiError::Parse(format!("failed to read OAuth code: {error}")))??;

    parse_authorization_input(input.trim(), expected_state)
}

/// 解析手动输入的授权码。
pub(crate) fn parse_authorization_input(input: &str, expected_state: &str) -> Result<String> {
    if let Some((_, query)) = input.split_once('?') {
        let query = query.split('#').next().unwrap_or(query);
        if let Some(state) = query_param(query, "state")
            && state != expected_state
        {
            return Err(AiError::Parse("OAuth state mismatch".into()));
        }
        return query_param(query, "code")
            .ok_or_else(|| AiError::Parse("missing OAuth authorization code".into()));
    }
    if let Some((code, state)) = input.split_once('#') {
        if state != expected_state {
            return Err(AiError::Parse("OAuth state mismatch".into()));
        }
        return Ok(code.to_string());
    }
    if input.contains("code=") {
        if let Some(state) = query_param(input, "state")
            && state != expected_state
        {
            return Err(AiError::Parse("OAuth state mismatch".into()));
        }
        return query_param(input, "code")
            .ok_or_else(|| AiError::Parse("missing OAuth authorization code".into()));
    }
    Ok(input.to_string())
}

/// 使用 authorization code 换取 token。
async fn exchange_authorization_code(
    client: &Client,
    code: &str,
    verifier: &str,
) -> Result<CodexCredentials> {
    let mut body = Vec::new();
    push_query(&mut body, "grant_type", "authorization_code");
    push_query(&mut body, "client_id", CLIENT_ID);
    push_query(&mut body, "code", code);
    push_query(&mut body, "code_verifier", verifier);
    push_query(&mut body, "redirect_uri", REDIRECT_URI);
    request_token(client, body.join("&")).await
}

/// 使用 refresh token 刷新 access token。
async fn refresh_credentials(client: &Client, refresh_token: &str) -> Result<CodexCredentials> {
    let mut body = Vec::new();
    push_query(&mut body, "grant_type", "refresh_token");
    push_query(&mut body, "refresh_token", refresh_token);
    push_query(&mut body, "client_id", CLIENT_ID);
    request_token(client, body.join("&")).await
}

/// 请求 OAuth token endpoint。
async fn request_token(client: &Client, body: String) -> Result<CodexCredentials> {
    let send = client
        .post(TOKEN_URL)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send();
    let resp = timeout(Duration::from_secs(TOKEN_REQUEST_TIMEOUT_SECONDS), send)
        .await
        .map_err(|_| AiError::Stream(format!("codex token request timed out: url={TOKEN_URL}")))?
        .map_err(|error| request_send_error("codex", TOKEN_URL, "oauth token request", 0, error))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(AiError::Api {
            status: status.as_u16(),
            message: text,
        });
    }
    let token = resp.json::<TokenResponse>().await?;
    let account_id = extract_account_id(&token.access_token)?;
    Ok(CodexCredentials {
        access: token.access_token,
        refresh: token.refresh_token,
        expires: now_ms() + token.expires_in * 1000,
        account_id,
    })
}

/// 读取本地 Codex OAuth 凭证，坏文件会触发重新登录。
fn load_credentials() -> Option<CodexCredentials> {
    let path = credentials_path()?;
    match std::fs::read_to_string(path) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(credentials) => Some(credentials),
            Err(error) => {
                log::warn!("failed to parse codex credentials, login again: {error}");
                None
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            log::warn!("failed to read codex credentials, login again: {error}");
            None
        }
    }
}

/// 保存本地 Codex OAuth 凭证。
fn save_credentials(credentials: &CodexCredentials) -> Result<()> {
    let path = credentials_path()
        .ok_or_else(|| AiError::Parse("failed to determine home directory".into()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(credentials)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// 从凭证中解析 settings 展示信息。
fn auth_info_from_credentials(credentials: &CodexCredentials) -> Result<CodexAuthInfo> {
    Ok(CodexAuthInfo {
        account_id: credentials.account_id.clone(),
        email: extract_profile_email(&credentials.access)?,
        expires: credentials.expires,
    })
}

/// 返回 gsdv 的 Codex OAuth 凭证文件路径。
fn credentials_path() -> Option<PathBuf> {
    let home = home_dir()?;
    Some(home.join(".gsdv").join("auth").join("codex.json"))
}

/// 返回当前用户 home 目录。
fn home_dir() -> Option<PathBuf> {
    crate::home::home_dir()
}

/// 当前 Unix 毫秒。
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// 生成随机 base64url 字符串。
fn random_base64url(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    rand::rng().fill(&mut bytes[..]);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// 生成随机十六进制字符串。
fn random_hex(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    rand::rng().fill(&mut bytes[..]);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 计算 SHA-256 并返回 base64url。
fn sha256_base64url(input: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input);
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

/// 追加 x-www-form-urlencoded 参数。
fn push_query(out: &mut Vec<String>, key: &str, value: &str) {
    out.push(format!("{}={}", percent_encode(key), percent_encode(value)));
}

/// 查询字符串中取单个参数。
fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (raw_key, raw_value) = pair.split_once('=')?;
        (percent_decode(raw_key).ok()?.as_str() == key).then(|| percent_decode(raw_value).ok())?
    })
}

/// 百分号编码表单参数。
fn percent_encode(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                output.push(byte as char)
            }
            b' ' => output.push('+'),
            _ => output.push_str(&format!("%{byte:02X}")),
        }
    }
    output
}

/// 百分号解码表单参数。
fn percent_decode(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut output = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                output.push(b' ');
                index += 1;
            }
            b'%' => {
                if index + 2 >= bytes.len() {
                    return Err(AiError::Parse("invalid percent encoding".into()));
                }
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).map_err(|error| {
                    AiError::Parse(format!("invalid percent encoding: {error}"))
                })?;
                let byte = u8::from_str_radix(hex, 16).map_err(|error| {
                    AiError::Parse(format!("invalid percent encoding: {error}"))
                })?;
                output.push(byte);
                index += 3;
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(output)
        .map_err(|error| AiError::Parse(format!("invalid utf8 in percent decoding: {error}")))
}

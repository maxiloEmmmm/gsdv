//! remote server listener 和 HTTP API 接入。
//!
//! HTTP handler 只能解析协议、投递 `AppEvent` 并等待结果；所有影响 GUI
//! 状态或 terminal host 的动作都必须回到 app event drain。

use super::*;
use axum::body::Body;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{StatusCode, Uri, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::MissedTickBehavior;

/// Remote 图片粘贴发送给 terminal TUI 的 Ctrl+V 控制字节。
const REMOTE_AGENT_IMAGE_PASTE_BYTES: &[u8] = &[0x16];
/// Remote agent 输出 WebSocket 心跳间隔。
const REMOTE_AGENT_OUTPUT_PING_INTERVAL: Duration = Duration::from_secs(30);
/// Remote agent 输出 WebSocket 兜底检查间隔。
const REMOTE_AGENT_OUTPUT_POLL_INTERVAL: Duration = Duration::from_millis(250);
/// Remote agent 输出 WebSocket 允许连续丢失的 pong 次数。
const REMOTE_AGENT_OUTPUT_MAX_MISSED_PONGS: u8 = 2;
/// Remote 请求等待后台 agent terminal 创建的重试次数。
const REMOTE_AGENT_START_RETRY_ATTEMPTS: usize = 80;
/// Remote 请求等待后台 agent terminal 创建的重试间隔。
const REMOTE_AGENT_START_RETRY_INTERVAL: Duration = Duration::from_millis(50);
/// Remote resize 允许的最大 terminal 列数。
const REMOTE_AGENT_RESIZE_MAX_COLS: u16 = 500;
/// Remote resize 允许的最大 terminal 行数。
const REMOTE_AGENT_RESIZE_MAX_ROWS: u16 = 200;

/// 内嵌 remote 前端静态资源。
struct RemoteWebAsset {
    /// HTTP 请求路径。
    path: &'static str,
    /// HTTP Content-Type。
    mime: &'static str,
    /// 编译进二进制的文件内容。
    bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/remote_web_assets.rs"));

/// HTTP handler 共享的 remote API 状态。
#[derive(Clone)]
struct RemoteApiState {
    /// 唯一 AppEvent 队列发送端。
    event_tx: Sender<AppEvent>,
    /// 唤醒 egui update 的上下文。
    repaint_ctx: egui::Context,
    /// 合并 remote API 唤醒频率的 repaint 控制器。
    repaint_controller: repaint_gate::RepaintController,
}

/// 从 HTTP handler 发往 GUI AppEvent drain 的请求信封。
pub(super) struct RemoteApiEnvelope {
    /// 已解析并完成重 CPU 准备的 remote API 请求。
    request: RemoteApiRequest,
    /// GUI drain 处理完成后回复 HTTP handler 的通道。
    reply: oneshot::Sender<RemoteApiResult>,
}

/// 已解析的 remote API 业务请求。
#[derive(Clone)]
enum RemoteApiRequest {
    /// 读取所有 workspace/agent 元数据。
    Workspace,
    /// 给目标 agent 提交文本输入。
    AgentInput(RemoteAgentInput),
    /// 给目标 agent 发送中断输入。
    AgentEsc(RemoteAgentTarget),
    /// 给目标 agent 发送剪贴板图片。
    AgentImage(RemoteAgentImage),
    /// 订阅目标 agent terminal 输出。
    AgentOutputSubscribe(RemoteAgentTarget),
    /// 按浏览器视口调整目标 agent terminal grid。
    AgentResize(RemoteAgentResize),
}

/// remote API 调用处理结果。
enum RemoteApiResult {
    /// 请求成功并携带响应数据。
    Ok(RemoteApiPayload),
    /// 请求失败并携带稳定错误。
    Err(RemoteApiError),
}

/// remote API 成功响应数据。
enum RemoteApiPayload {
    /// workspace 元数据响应。
    Workspace(RemoteWorkspaceResponse),
    /// agent 操作成功响应。
    AgentOk,
    /// agent 输出订阅响应。
    AgentOutputSubscription(RemoteAgentOutputSubscription),
}

/// remote API 错误响应。
struct RemoteApiError {
    /// HTTP 状态码。
    status: StatusCode,
    /// 稳定机器可读错误码。
    code: &'static str,
    /// 人类可读错误信息。
    message: String,
}

/// Agent 操作目标位置。
#[derive(Debug, Clone, Deserialize)]
struct RemoteAgentTarget {
    /// 对外稳定 workspace id。
    workspace_id: String,
    /// Agent row 数组下标。
    row_index: usize,
    /// Agent column 数组下标。
    col_index: usize,
    /// Agent 运行实例稳定 id。
    agent_id: String,
}

/// Agent 文本输入请求。
#[derive(Debug, Clone, Deserialize)]
struct RemoteAgentInput {
    /// 对外稳定 workspace id。
    workspace_id: String,
    /// Agent row 数组下标。
    row_index: usize,
    /// Agent column 数组下标。
    col_index: usize,
    /// Agent 运行实例稳定 id。
    agent_id: String,
    /// 要提交给 agent 的文本。
    text: String,
}

/// Agent 图片输入请求。
#[derive(Clone)]
struct RemoteAgentImage {
    /// Agent 操作目标。
    target: RemoteAgentTarget,
    /// 已解码成 egui 可复制形态的图片。
    image: egui::ColorImage,
}

/// Agent terminal resize 请求。
#[derive(Debug, Clone)]
struct RemoteAgentResize {
    /// Agent 操作目标。
    target: RemoteAgentTarget,
    /// 目标 terminal 列数。
    cols: u16,
    /// 目标 terminal 行数。
    rows: u16,
}

/// Agent 输出 WebSocket query 参数。
#[derive(Debug, Clone, Deserialize)]
struct RemoteAgentOutputQuery {
    /// 对外稳定 workspace id。
    workspace_id: String,
    /// Agent row 数组下标。
    row_index: usize,
    /// Agent column 数组下标。
    col_index: usize,
    /// Agent 运行实例稳定 id。
    agent_id: String,
}

/// Agent 输出 WebSocket 订阅数据。
struct RemoteAgentOutputSubscription {
    /// 可跨任务读取的 terminal 输出源。
    source: TerminalRemoteOutputSource,
    /// 连接成功后立即发送的完整快照。
    snapshot: TerminalRemoteSnapshot,
    /// 后续 append-only 对比状态。
    state: TerminalRemoteOutputState,
}

/// Agent 输出 WebSocket snapshot 消息。
#[derive(Debug, Serialize)]
struct RemoteAgentOutputSnapshotMessage {
    /// 消息类型。
    #[serde(rename = "type")]
    message_type: &'static str,
    /// 连接内递增序号。
    sequence: u64,
    /// 当前 terminal 列数。
    cols: usize,
    /// 当前 terminal buffer 行。
    rows: Vec<crate::gui::terminal_host::TerminalRemoteRow>,
    /// 当前 terminal 光标。
    cursor: crate::gui::terminal_host::TerminalRemoteCursor,
}

/// Agent 输出 WebSocket append 消息。
#[derive(Debug, Serialize)]
struct RemoteAgentOutputAppendMessage {
    /// 消息类型。
    #[serde(rename = "type")]
    message_type: &'static str,
    /// 连接内递增序号。
    sequence: u64,
    /// 新增底部行。
    rows: Vec<crate::gui::terminal_host::TerminalRemoteRow>,
}

/// Agent 输出 WebSocket ping 消息。
#[derive(Debug, Serialize)]
struct RemoteAgentOutputPingMessage {
    /// 消息类型。
    #[serde(rename = "type")]
    message_type: &'static str,
    /// 连接内递增序号。
    sequence: u64,
}

/// Agent 输出 WebSocket 客户端消息。
#[derive(Debug, Deserialize)]
struct RemoteAgentOutputClientMessage {
    /// 消息类型。
    #[serde(rename = "type")]
    message_type: String,
    /// 客户端回传的服务端序号。
    sequence: Option<u64>,
    /// resize 消息携带的 terminal 列数。
    cols: Option<u16>,
    /// resize 消息携带的 terminal 行数。
    rows: Option<u16>,
}

/// HTTP JSON 图片输入请求。
#[derive(Debug, Clone, Deserialize)]
struct RemoteAgentImageHttpRequest {
    /// 对外稳定 workspace id。
    workspace_id: String,
    /// Agent row 数组下标。
    row_index: usize,
    /// Agent column 数组下标。
    col_index: usize,
    /// Agent 运行实例稳定 id。
    agent_id: String,
    /// 标准 base64 编码图片内容。
    image_base64: String,
    /// 图片 MIME 类型，目前只支持 image/png。
    mime_type: String,
}

/// workspace 列表响应。
#[derive(Debug, Clone, Serialize)]
struct RemoteWorkspaceResponse {
    /// 所有已打开 workspace。
    workspaces: Vec<RemoteWorkspace>,
}

/// 单个 workspace 元数据。
#[derive(Debug, Clone, Serialize)]
struct RemoteWorkspace {
    /// 对外稳定 workspace id。
    workspace_id: String,
    /// UI 显示名称。
    name: String,
    /// workspace 绝对或用户打开时记录的路径。
    path: String,
    /// workspace 内 Agent 行。
    rows: Vec<RemoteAgentRow>,
}

/// 单个 Agent 行元数据。
#[derive(Debug, Clone, Serialize)]
struct RemoteAgentRow {
    /// Agent row 数组下标。
    row_index: usize,
    /// 当前行内所有列。
    cols: Vec<RemoteAgentColumn>,
}

/// 单个 Agent 列元数据。
#[derive(Debug, Clone, Serialize)]
struct RemoteAgentColumn {
    /// Agent column 数组下标。
    col_index: usize,
    /// 当前列内所有 agent tab。
    agents: Vec<RemoteAgent>,
}

/// 单个 Agent tab 元数据。
#[derive(Debug, Clone, Serialize)]
struct RemoteAgent {
    /// Agent 运行实例稳定 id。
    agent_id: String,
    /// UI 展示标题。
    title: String,
}

/// agent 操作成功响应。
#[derive(Debug, Clone, Serialize)]
struct RemoteAgentOkResponse {
    /// 操作是否成功。
    ok: bool,
}

/// remote API 错误响应外层。
#[derive(Debug, Clone, Serialize)]
struct RemoteErrorResponse {
    /// 稳定错误对象。
    error: RemoteErrorBody,
}

/// remote API 错误响应内容。
#[derive(Debug, Clone, Serialize)]
struct RemoteErrorBody {
    /// 稳定机器可读错误码。
    code: &'static str,
    /// 人类可读错误信息。
    message: String,
}

impl GsdvGuiApp {
    /// Restarts the embedded remote server listener from current runtime settings.
    pub(super) fn restart_remote_server(&mut self, ctx: &egui::Context, announce_stopped: bool) {
        self.remote_server_generation = self.remote_server_generation.wrapping_add(1);
        let generation = self.remote_server_generation;
        let settings = self.runtime_settings.remote_server_settings();
        let stopped_task = self.remote_server_task.take();
        let had_task = stopped_task.is_some();
        if let Some(task) = stopped_task {
            task.abort();
        }
        if !settings.enabled {
            if announce_stopped && had_task {
                self.push_toast(
                    i18n::text(self.app_language, "Remote server stopped"),
                    theme::warning(),
                );
            }
            return;
        }
        self.remote_server_task = Some(spawn_remote_server_listener(
            Arc::clone(&self.background_runtime),
            self.app_event_tx.clone(),
            self.repaint_controller.clone(),
            ctx.clone(),
            generation,
            settings,
        ));
    }

    /// Applies a remote server listener success event to visible UI state.
    pub(super) fn apply_remote_server_started(&mut self, generation: u64, address: String) {
        if generation != self.remote_server_generation {
            return;
        }
        self.push_toast(
            i18n::text_with_arg(
                self.app_language,
                "Remote server listening on {address}",
                "{address}",
                address,
            ),
            theme::success(),
        );
    }

    /// Applies a remote server listener failure event to visible UI state.
    pub(super) fn apply_remote_server_failed(&mut self, generation: u64, error: String) {
        if generation != self.remote_server_generation {
            return;
        }
        self.remote_server_task = None;
        self.push_toast(
            i18n::text_with_arg(
                self.app_language,
                "Remote server failed: {error}",
                "{error}",
                error,
            ),
            theme::danger(),
        );
    }

    /// 处理 remote HTTP API 通过 AppEvent 投递的请求。
    pub(super) fn handle_remote_api_envelope(
        &mut self,
        ctx: &egui::Context,
        envelope: RemoteApiEnvelope,
    ) {
        let result = self.handle_remote_api_request(ctx, envelope.request);
        let _ = envelope.reply.send(result);
    }

    /// 在 GUI 主线程处理一个 remote API 请求。
    fn handle_remote_api_request(
        &mut self,
        ctx: &egui::Context,
        request: RemoteApiRequest,
    ) -> RemoteApiResult {
        match request {
            RemoteApiRequest::Workspace => RemoteApiResult::Ok(RemoteApiPayload::Workspace(
                self.remote_workspace_response(),
            )),
            RemoteApiRequest::AgentInput(input) => self.handle_remote_agent_input(ctx, input),
            RemoteApiRequest::AgentEsc(target) => {
                self.handle_remote_agent_bytes(ctx, target, &[0x03], false)
            }
            RemoteApiRequest::AgentImage(input) => self.handle_remote_agent_image(ctx, input),
            RemoteApiRequest::AgentOutputSubscribe(target) => {
                self.handle_remote_agent_output_subscribe(ctx, target)
            }
            RemoteApiRequest::AgentResize(input) => self.handle_remote_agent_resize(ctx, input),
        }
    }

    /// 构造 remote workspace 元数据响应。
    fn remote_workspace_response(&self) -> RemoteWorkspaceResponse {
        RemoteWorkspaceResponse {
            workspaces: self
                .workspaces
                .iter()
                .map(remote_workspace_from_view_data)
                .collect(),
        }
    }

    /// 处理 remote agent 文本输入。
    fn handle_remote_agent_input(
        &mut self,
        ctx: &egui::Context,
        input: RemoteAgentInput,
    ) -> RemoteApiResult {
        if input.text.trim().is_empty() {
            return RemoteApiResult::Err(remote_bad_request(
                "invalid_request",
                "text must not be empty",
            ));
        }
        let target = RemoteAgentTarget {
            workspace_id: input.workspace_id,
            row_index: input.row_index,
            col_index: input.col_index,
            agent_id: input.agent_id,
        };
        let Some((workspace_index, slot_id)) = self.remote_agent_slot(&target) else {
            return RemoteApiResult::Err(remote_not_found());
        };
        self.ensure_agent_terminal_host(ctx, workspace_index, &slot_id);
        let Some(host) = self.remote_agent_host_mut(workspace_index, &slot_id) else {
            if self.remote_agent_spawn_pending(workspace_index, &slot_id) {
                return RemoteApiResult::Err(remote_agent_starting());
            }
            return RemoteApiResult::Err(remote_not_found());
        };
        host.paste_text(&input.text);
        host.submit_current_input();
        RemoteApiResult::Ok(RemoteApiPayload::AgentOk)
    }

    /// 处理 remote agent 图片输入。
    fn handle_remote_agent_image(
        &mut self,
        ctx: &egui::Context,
        input: RemoteAgentImage,
    ) -> RemoteApiResult {
        let Some((workspace_index, slot_id)) = self.remote_agent_slot(&input.target) else {
            return RemoteApiResult::Err(remote_not_found());
        };
        self.ensure_agent_terminal_host(ctx, workspace_index, &slot_id);
        ctx.copy_image(input.image);
        let Some(host) = self.remote_agent_host_mut(workspace_index, &slot_id) else {
            if self.remote_agent_spawn_pending(workspace_index, &slot_id) {
                return RemoteApiResult::Err(remote_agent_starting());
            }
            return RemoteApiResult::Err(remote_not_found());
        };
        // 触发条件：remote API 已把图片写入系统剪贴板，需要通知 Agent TUI。
        // 不能走文本 bracketed paste：图片 attachment 不是 PTY 文本。
        // 防止回归：HTTP 返回成功但 Agent composer 没有收到图片。
        host.write_bytes(REMOTE_AGENT_IMAGE_PASTE_BYTES);
        host.submit_current_input();
        RemoteApiResult::Ok(RemoteApiPayload::AgentOk)
    }

    /// 处理 remote agent 原始控制字节输入。
    fn handle_remote_agent_bytes(
        &mut self,
        ctx: &egui::Context,
        target: RemoteAgentTarget,
        bytes: &[u8],
        submit: bool,
    ) -> RemoteApiResult {
        let Some((workspace_index, slot_id)) = self.remote_agent_slot(&target) else {
            return RemoteApiResult::Err(remote_not_found());
        };
        self.ensure_agent_terminal_host(ctx, workspace_index, &slot_id);
        let Some(host) = self.remote_agent_host_mut(workspace_index, &slot_id) else {
            if self.remote_agent_spawn_pending(workspace_index, &slot_id) {
                return RemoteApiResult::Err(remote_agent_starting());
            }
            return RemoteApiResult::Err(remote_not_found());
        };
        host.write_bytes(bytes);
        if submit {
            host.submit_current_input();
        }
        RemoteApiResult::Ok(RemoteApiPayload::AgentOk)
    }

    /// 处理 remote agent 输出订阅请求。
    fn handle_remote_agent_output_subscribe(
        &mut self,
        ctx: &egui::Context,
        target: RemoteAgentTarget,
    ) -> RemoteApiResult {
        let Some((workspace_index, slot_id)) = self.remote_agent_slot(&target) else {
            return RemoteApiResult::Err(remote_not_found());
        };
        self.ensure_agent_terminal_host(ctx, workspace_index, &slot_id);
        let Some(host) = self.remote_agent_host_mut(workspace_index, &slot_id) else {
            if self.remote_agent_spawn_pending(workspace_index, &slot_id) {
                return RemoteApiResult::Err(remote_agent_starting());
            }
            return RemoteApiResult::Err(remote_not_found());
        };
        let source = host.remote_output_source();
        let (snapshot, state) = source.snapshot();
        RemoteApiResult::Ok(RemoteApiPayload::AgentOutputSubscription(
            RemoteAgentOutputSubscription {
                source,
                snapshot,
                state,
            },
        ))
    }

    /// 处理 remote agent terminal resize。
    fn handle_remote_agent_resize(
        &mut self,
        ctx: &egui::Context,
        input: RemoteAgentResize,
    ) -> RemoteApiResult {
        if input.cols == 0
            || input.rows == 0
            || input.cols > REMOTE_AGENT_RESIZE_MAX_COLS
            || input.rows > REMOTE_AGENT_RESIZE_MAX_ROWS
        {
            return RemoteApiResult::Err(remote_bad_request(
                "invalid_terminal_size",
                "terminal size is outside supported remote bounds",
            ));
        }

        let Some((workspace_index, slot_id)) = self.remote_agent_slot(&input.target) else {
            return RemoteApiResult::Err(remote_not_found());
        };
        self.ensure_agent_terminal_host(ctx, workspace_index, &slot_id);
        let Some(host) = self.remote_agent_host_mut(workspace_index, &slot_id) else {
            if self.remote_agent_spawn_pending(workspace_index, &slot_id) {
                return RemoteApiResult::Err(remote_agent_starting());
            }
            return RemoteApiResult::Err(remote_not_found());
        };
        host.resize_remote_grid(input.cols, input.rows);
        RemoteApiResult::Ok(RemoteApiPayload::AgentOk)
    }

    /// 查找 remote agent 目标对应的 workspace index 和 agent slot。
    fn remote_agent_slot(&self, target: &RemoteAgentTarget) -> Option<(usize, AgentSlotId)> {
        if target.workspace_id.trim().is_empty() || target.agent_id.trim().is_empty() {
            return None;
        }
        self.workspaces
            .iter()
            .enumerate()
            .find_map(|(workspace_index, workspace)| {
                let workspace_id = data::workspace_store_key(&workspace.path);
                if workspace_id != target.workspace_id {
                    return None;
                }
                let column = workspace
                    .agent_rows
                    .get(target.row_index)?
                    .columns
                    .get(target.col_index)?;
                remote_column_agent_slot(workspace, column, &target.agent_id)
                    .map(|slot| (workspace_index, slot))
            })
    }

    /// 返回指定 Agent slot 已就绪的 terminal host。
    fn remote_agent_host_mut(
        &mut self,
        workspace_index: usize,
        slot_id: &AgentSlotId,
    ) -> Option<&mut GuiTerminalHost> {
        self.terminal_hosts
            .get_mut(workspace_index)?
            .agents
            .get_mut(slot_id)?
            .host
            .as_mut()
    }

    /// 判断 remote 请求目标 agent 是否正在后台创建 terminal host。
    fn remote_agent_spawn_pending(&self, workspace_index: usize, slot_id: &AgentSlotId) -> bool {
        self.pending_terminal_spawns.contains(&TerminalSpawnKey {
            index: workspace_index,
            kind: TerminalSurfaceKind::Agent,
            agent_slot: slot_id.clone(),
        })
    }
}

/// Spawns a listener task that owns the configured remote server port.
fn spawn_remote_server_listener(
    runtime: Arc<tokio::runtime::Runtime>,
    event_tx: Sender<AppEvent>,
    repaint_controller: repaint_gate::RepaintController,
    repaint_ctx: egui::Context,
    generation: u64,
    settings: RemoteServerSettings,
) -> tokio::task::JoinHandle<()> {
    runtime.spawn(async move {
        let address = settings.bind_address();
        match TcpListener::bind(&address).await {
            Ok(listener) => {
                run_remote_server_listener(
                    event_tx,
                    repaint_controller,
                    repaint_ctx,
                    generation,
                    address,
                    listener,
                )
                .await
            }
            Err(error) => {
                let _ = event_tx.send(AppEvent::RemoteServerFailed {
                    generation,
                    error: format!("{} ({error})", settings.bind_address()),
                });
            }
        }
    })
}

/// Runs the axum listener until the task is aborted or serving fails.
async fn run_remote_server_listener(
    event_tx: Sender<AppEvent>,
    repaint_controller: repaint_gate::RepaintController,
    repaint_ctx: egui::Context,
    generation: u64,
    configured_address: String,
    listener: TcpListener,
) {
    let address = listener
        .local_addr()
        .map(|address| address.to_string())
        .unwrap_or(configured_address);
    let _ = event_tx.send(AppEvent::RemoteServerStarted {
        generation,
        address,
    });
    let app = remote_router(RemoteApiState {
        event_tx: event_tx.clone(),
        repaint_ctx,
        repaint_controller,
    });
    if let Err(error) = axum::serve(listener, app).await {
        let _ = event_tx.send(AppEvent::RemoteServerFailed {
            generation,
            error: error.to_string(),
        });
    }
}

/// Builds the remote HTTP API router.
fn remote_router(state: RemoteApiState) -> Router {
    Router::new()
        .route("/api/workspace", get(remote_workspace_handler))
        .route("/api/agent/input", post(remote_agent_input_handler))
        .route("/api/agent/esc", post(remote_agent_esc_handler))
        .route("/api/agent/image", post(remote_agent_image_handler))
        .route("/api/agent/output/ws", get(remote_agent_output_ws_handler))
        .fallback(remote_web_handler)
        .with_state(state)
}

/// Handles embedded remote web frontend requests.
async fn remote_web_handler(uri: Uri) -> axum::response::Response {
    let path = uri.path();
    if path != "/" && !path.starts_with("/assets/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(asset) = remote_web_asset(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = axum::response::Response::new(Body::from(asset.bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(asset.mime),
    );
    response
}

/// Looks up an embedded remote web asset by request path.
fn remote_web_asset(path: &str) -> Option<&'static RemoteWebAsset> {
    REMOTE_WEB_ASSETS.iter().find(|asset| asset.path == path)
}

/// Handles workspace metadata requests.
async fn remote_workspace_handler(State(state): State<RemoteApiState>) -> impl IntoResponse {
    dispatch_remote_api_request(state, RemoteApiRequest::Workspace).await
}

/// Handles agent text input requests.
async fn remote_agent_input_handler(
    State(state): State<RemoteApiState>,
    payload: Result<Json<RemoteAgentInput>, JsonRejection>,
) -> impl IntoResponse {
    let input = match json_payload(payload) {
        Ok(input) => input,
        Err(response) => return response,
    };
    dispatch_remote_api_request(state, RemoteApiRequest::AgentInput(input)).await
}

/// Handles agent interrupt requests.
async fn remote_agent_esc_handler(
    State(state): State<RemoteApiState>,
    payload: Result<Json<RemoteAgentTarget>, JsonRejection>,
) -> impl IntoResponse {
    let target = match json_payload(payload) {
        Ok(target) => target,
        Err(response) => return response,
    };
    dispatch_remote_api_request(state, RemoteApiRequest::AgentEsc(target)).await
}

/// Handles agent image input requests.
async fn remote_agent_image_handler(
    State(state): State<RemoteApiState>,
    payload: Result<Json<RemoteAgentImageHttpRequest>, JsonRejection>,
) -> impl IntoResponse {
    let input = match json_payload(payload) {
        Ok(input) => input,
        Err(response) => return response,
    };
    let image = match decode_remote_agent_image(&input) {
        Ok(image) => image,
        Err(error) => return remote_error_response(error),
    };
    let target = RemoteAgentTarget {
        workspace_id: input.workspace_id,
        row_index: input.row_index,
        col_index: input.col_index,
        agent_id: input.agent_id,
    };
    dispatch_remote_api_request(
        state,
        RemoteApiRequest::AgentImage(RemoteAgentImage { target, image }),
    )
    .await
}

/// Handles agent terminal output WebSocket subscriptions.
async fn remote_agent_output_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<RemoteApiState>,
    query: Result<Query<RemoteAgentOutputQuery>, QueryRejection>,
) -> axum::response::Response {
    let query = match query_payload(query) {
        Ok(query) => query,
        Err(response) => return response,
    };
    let target = RemoteAgentTarget {
        workspace_id: query.workspace_id,
        row_index: query.row_index,
        col_index: query.col_index,
        agent_id: query.agent_id,
    };
    let subscription =
        match dispatch_remote_agent_output_subscription(state.clone(), target.clone()).await {
            Ok(subscription) => subscription,
            Err(error) => return remote_error_response(error),
        };
    ws.on_upgrade(|socket| handle_remote_agent_output_socket(socket, subscription, state, target))
}

/// Dispatches an API request through the unique AppEvent queue.
async fn dispatch_remote_api_request(
    state: RemoteApiState,
    request: RemoteApiRequest,
) -> axum::response::Response {
    let mut attempts = 0_usize;
    loop {
        let result = dispatch_remote_api_request_once(state.clone(), request.clone()).await;
        if !remote_api_result_is_agent_starting(&result)
            || attempts >= REMOTE_AGENT_START_RETRY_ATTEMPTS
        {
            return remote_api_result_response(result);
        }
        attempts += 1;
        tokio::time::sleep(REMOTE_AGENT_START_RETRY_INTERVAL).await;
    }
}

/// Dispatches one API request attempt through the unique AppEvent queue.
async fn dispatch_remote_api_request_once(
    state: RemoteApiState,
    request: RemoteApiRequest,
) -> RemoteApiResult {
    let (reply, response) = oneshot::channel();
    let envelope = RemoteApiEnvelope { request, reply };
    if state
        .event_tx
        .send(AppEvent::RemoteApiRequest(envelope))
        .is_err()
    {
        return RemoteApiResult::Err(remote_internal_error("app event queue is closed"));
    }
    state.repaint_controller.request_repaint(&state.repaint_ctx);
    match response.await {
        Ok(result) => result,
        Err(_) => RemoteApiResult::Err(remote_internal_error("remote api response was dropped")),
    }
}

/// Dispatches an agent output subscription request through AppEvent.
async fn dispatch_remote_agent_output_subscription(
    state: RemoteApiState,
    target: RemoteAgentTarget,
) -> Result<RemoteAgentOutputSubscription, RemoteApiError> {
    let mut attempts = 0_usize;
    loop {
        match dispatch_remote_api_request_once(
            state.clone(),
            RemoteApiRequest::AgentOutputSubscribe(target.clone()),
        )
        .await
        {
            RemoteApiResult::Ok(RemoteApiPayload::AgentOutputSubscription(subscription)) => {
                return Ok(subscription);
            }
            RemoteApiResult::Ok(_) => {
                return Err(remote_internal_error(
                    "remote api returned unexpected output subscription payload",
                ));
            }
            RemoteApiResult::Err(error)
                if remote_error_is_agent_starting(&error)
                    && attempts < REMOTE_AGENT_START_RETRY_ATTEMPTS =>
            {
                attempts += 1;
                tokio::time::sleep(REMOTE_AGENT_START_RETRY_INTERVAL).await;
            }
            RemoteApiResult::Err(error) => return Err(error),
        }
    }
}

/// Dispatches a remote terminal resize request through AppEvent.
async fn dispatch_remote_agent_resize(
    state: &RemoteApiState,
    target: RemoteAgentTarget,
    cols: u16,
    rows: u16,
) {
    let (reply, response) = oneshot::channel();
    let envelope = RemoteApiEnvelope {
        request: RemoteApiRequest::AgentResize(RemoteAgentResize { target, cols, rows }),
        reply,
    };
    if state
        .event_tx
        .send(AppEvent::RemoteApiRequest(envelope))
        .is_err()
    {
        return;
    }
    state.repaint_controller.request_repaint(&state.repaint_ctx);
    let _ = response.await;
}

/// Extracts a JSON payload or returns the stable remote error shape.
fn json_payload<T>(payload: Result<Json<T>, JsonRejection>) -> Result<T, axum::response::Response> {
    payload.map(|Json(value)| value).map_err(|error| {
        remote_error_response(remote_bad_request("invalid_json", error.body_text()))
    })
}

/// Extracts a query payload or returns the stable remote error shape.
fn query_payload<T>(
    payload: Result<Query<T>, QueryRejection>,
) -> Result<T, axum::response::Response> {
    payload.map(|Query(value)| value).map_err(|error| {
        remote_error_response(remote_bad_request("invalid_query", error.to_string()))
    })
}

/// Converts a remote API result into an HTTP response.
fn remote_api_result_response(result: RemoteApiResult) -> axum::response::Response {
    match result {
        RemoteApiResult::Ok(RemoteApiPayload::Workspace(payload)) => {
            (StatusCode::OK, Json(json!(payload))).into_response()
        }
        RemoteApiResult::Ok(RemoteApiPayload::AgentOk) => (
            StatusCode::OK,
            Json(json!(RemoteAgentOkResponse { ok: true })),
        )
            .into_response(),
        RemoteApiResult::Ok(RemoteApiPayload::AgentOutputSubscription(_)) => remote_error_response(
            remote_internal_error("agent output subscription is websocket-only"),
        ),
        RemoteApiResult::Err(error) => remote_error_response(error),
    }
}

/// Runs one upgraded agent output WebSocket connection.
async fn handle_remote_agent_output_socket(
    mut socket: WebSocket,
    mut subscription: RemoteAgentOutputSubscription,
    state: RemoteApiState,
    target: RemoteAgentTarget,
) {
    let mut sequence = 1_u64;
    if !send_remote_agent_output_snapshot(&mut socket, sequence, subscription.snapshot.clone())
        .await
    {
        return;
    }
    let mut poll_interval = tokio::time::interval(REMOTE_AGENT_OUTPUT_POLL_INTERVAL);
    poll_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut ping_interval = tokio::time::interval(REMOTE_AGENT_OUTPUT_PING_INTERVAL);
    ping_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut missed_pongs = 0_u8;
    loop {
        tokio::select! {
            changed = subscription.source.changed() => {
                if !changed {
                    return;
                }
                if !send_remote_agent_output_update(&mut socket, &mut subscription, &mut sequence).await {
                    return;
                }
            }
            _ = poll_interval.tick() => {
                if !send_remote_agent_output_update(&mut socket, &mut subscription, &mut sequence).await {
                    return;
                }
            }
            _ = ping_interval.tick() => {
                missed_pongs = missed_pongs.saturating_add(1);
                if missed_pongs > REMOTE_AGENT_OUTPUT_MAX_MISSED_PONGS {
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
                sequence = sequence.wrapping_add(1);
                if !send_remote_agent_output_ping(&mut socket, sequence).await {
                    return;
                }
            }
            message = socket.recv() => {
                let Some(Ok(message)) = message else {
                    return;
                };
                if remote_agent_output_message_is_pong(&message) {
                    missed_pongs = 0;
                    continue;
                }
                if let Some((cols, rows)) = remote_agent_output_resize_message(&message) {
                    dispatch_remote_agent_resize(&state, target.clone(), cols, rows).await;
                    continue;
                }
                if matches!(message, Message::Close(_)) {
                    return;
                }
            }
        }
    }
}

/// Sends a terminal output update when the subscribed grid changed.
async fn send_remote_agent_output_update(
    socket: &mut WebSocket,
    subscription: &mut RemoteAgentOutputSubscription,
    sequence: &mut u64,
) -> bool {
    match subscription.source.update_since(&mut subscription.state) {
        TerminalRemoteUpdate::Unchanged => true,
        TerminalRemoteUpdate::Append(append) => {
            *sequence = sequence.wrapping_add(1);
            send_remote_agent_output_append(socket, *sequence, append).await
        }
        TerminalRemoteUpdate::Snapshot(snapshot) => {
            *sequence = sequence.wrapping_add(1);
            send_remote_agent_output_snapshot(socket, *sequence, snapshot).await
        }
    }
}

/// Sends the initial terminal snapshot over WebSocket.
async fn send_remote_agent_output_snapshot(
    socket: &mut WebSocket,
    sequence: u64,
    snapshot: TerminalRemoteSnapshot,
) -> bool {
    let message = RemoteAgentOutputSnapshotMessage {
        message_type: "snapshot",
        sequence,
        cols: snapshot.cols,
        rows: snapshot.rows,
        cursor: snapshot.cursor,
    };
    send_remote_agent_output_json(socket, &message).await
}

/// Sends append-only terminal rows over WebSocket.
async fn send_remote_agent_output_append(
    socket: &mut WebSocket,
    sequence: u64,
    append: crate::gui::terminal_host::TerminalRemoteAppend,
) -> bool {
    let message = RemoteAgentOutputAppendMessage {
        message_type: "append",
        sequence,
        rows: append.rows,
    };
    send_remote_agent_output_json(socket, &message).await
}

/// Sends a JSON heartbeat ping over WebSocket.
async fn send_remote_agent_output_ping(socket: &mut WebSocket, sequence: u64) -> bool {
    let message = RemoteAgentOutputPingMessage {
        message_type: "ping",
        sequence,
    };
    send_remote_agent_output_json(socket, &message).await
}

/// Serializes and sends one remote output WebSocket message.
async fn send_remote_agent_output_json<T: Serialize>(socket: &mut WebSocket, message: &T) -> bool {
    let Ok(text) = serde_json::to_string(message) else {
        return false;
    };
    socket.send(Message::Text(text.into())).await.is_ok()
}

/// Detects JSON pong messages and WebSocket protocol pong frames.
fn remote_agent_output_message_is_pong(message: &Message) -> bool {
    match message {
        Message::Pong(_) => true,
        Message::Text(text) => serde_json::from_str::<RemoteAgentOutputClientMessage>(
            text.as_str(),
        )
        .is_ok_and(|message| {
            let _ = message.sequence;
            message.message_type == "pong"
        }),
        _ => false,
    }
}

/// Extracts a browser terminal resize request from a WebSocket message.
fn remote_agent_output_resize_message(message: &Message) -> Option<(u16, u16)> {
    let Message::Text(text) = message else {
        return None;
    };
    let message = serde_json::from_str::<RemoteAgentOutputClientMessage>(text.as_str()).ok()?;
    if message.message_type != "resize" {
        return None;
    }
    let cols = message.cols?;
    let rows = message.rows?;
    if cols == 0 || rows == 0 {
        return None;
    }
    Some((cols, rows))
}

/// Converts a remote API error into an HTTP response.
fn remote_error_response(error: RemoteApiError) -> axum::response::Response {
    (
        error.status,
        Json(json!(RemoteErrorResponse {
            error: RemoteErrorBody {
                code: error.code,
                message: error.message,
            },
        })),
    )
        .into_response()
}

/// Checks whether an API result means the target agent host is still starting.
fn remote_api_result_is_agent_starting(result: &RemoteApiResult) -> bool {
    matches!(result, RemoteApiResult::Err(error) if remote_error_is_agent_starting(error))
}

/// Checks whether an API error is the internal remote agent startup sentinel.
fn remote_error_is_agent_starting(error: &RemoteApiError) -> bool {
    error.code == "agent_starting"
}

/// Builds a 400 remote API error.
fn remote_bad_request(code: &'static str, message: impl Into<String>) -> RemoteApiError {
    RemoteApiError {
        status: StatusCode::BAD_REQUEST,
        code,
        message: message.into(),
    }
}

/// Builds an internal retry sentinel for agents that are valid but still spawning.
fn remote_agent_starting() -> RemoteApiError {
    RemoteApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "agent_starting",
        message: "agent terminal is still starting".to_string(),
    }
}

/// Builds the shared 404 remote API error.
fn remote_not_found() -> RemoteApiError {
    RemoteApiError {
        status: StatusCode::NOT_FOUND,
        code: "agent_not_found",
        message: "agent not found".to_string(),
    }
}

/// Builds a 500 remote API error.
fn remote_internal_error(message: impl Into<String>) -> RemoteApiError {
    RemoteApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "internal_error",
        message: message.into(),
    }
}

/// Decodes remote image JSON into egui image data.
fn decode_remote_agent_image(
    input: &RemoteAgentImageHttpRequest,
) -> Result<egui::ColorImage, RemoteApiError> {
    if input.mime_type.trim() != "image/png" {
        return Err(remote_bad_request(
            "unsupported_image_type",
            "only image/png is supported",
        ));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(input.image_base64.trim())
        .map_err(|error| remote_bad_request("invalid_image_base64", error.to_string()))?;
    let image = image::load_from_memory(&bytes)
        .map_err(|error| remote_bad_request("invalid_image", error.to_string()))?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [width as usize, height as usize],
        rgba.as_raw(),
    ))
}

/// Converts workspace view data into remote API metadata.
fn remote_workspace_from_view_data(workspace: &WorkspaceViewData) -> RemoteWorkspace {
    RemoteWorkspace {
        workspace_id: data::workspace_store_key(&workspace.path),
        name: workspace.name.clone(),
        path: workspace.path.display().to_string(),
        rows: workspace
            .agent_rows
            .iter()
            .enumerate()
            .map(|(row_index, row)| remote_agent_row_from_view_data(workspace, row_index, row))
            .collect(),
    }
}

/// Converts an Agent row into remote API metadata.
fn remote_agent_row_from_view_data(
    workspace: &WorkspaceViewData,
    row_index: usize,
    row: &data::AgentRowViewData,
) -> RemoteAgentRow {
    RemoteAgentRow {
        row_index,
        cols: row
            .columns
            .iter()
            .enumerate()
            .map(|(col_index, column)| {
                remote_agent_column_from_view_data(workspace, col_index, column)
            })
            .collect(),
    }
}

/// Converts an Agent column into remote API metadata.
fn remote_agent_column_from_view_data(
    workspace: &WorkspaceViewData,
    col_index: usize,
    column: &data::AgentColumnViewData,
) -> RemoteAgentColumn {
    RemoteAgentColumn {
        col_index,
        agents: column
            .tabs
            .iter()
            .filter_map(|slot| remote_agent_from_column_slot(workspace, slot))
            .collect(),
    }
}

/// Converts one column slot into remote Agent metadata.
fn remote_agent_from_column_slot(
    workspace: &WorkspaceViewData,
    slot: &data::AgentColumnSlot,
) -> Option<RemoteAgent> {
    match slot {
        data::AgentColumnSlot::Main => Some(RemoteAgent {
            agent_id: workspace.agent_id.clone(),
            title: "main".to_string(),
        }),
        data::AgentColumnSlot::Subagent(id) => workspace
            .subagents
            .iter()
            .find(|subagent| subagent.id == *id)
            .map(|subagent| RemoteAgent {
                agent_id: subagent.agent_id.clone(),
                title: subagent.name.clone(),
            }),
    }
}

/// Resolves a remote column target into an Agent slot id.
fn remote_column_agent_slot(
    workspace: &WorkspaceViewData,
    column: &data::AgentColumnViewData,
    agent_id: &str,
) -> Option<AgentSlotId> {
    column.tabs.iter().find_map(|slot| match slot {
        data::AgentColumnSlot::Main if workspace.agent_id == agent_id => Some(AgentSlotId::Main),
        data::AgentColumnSlot::Main => None,
        data::AgentColumnSlot::Subagent(id) => workspace
            .subagents
            .iter()
            .find(|subagent| subagent.id == *id && subagent.agent_id == agent_id)
            .map(|subagent| AgentSlotId::Subagent(subagent.id.clone())),
    })
}

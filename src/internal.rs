use axum::Router;
use axum::body::Body;
use axum::extract::Query;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

const INTERNAL_SOCKET_FILE: &str = ".socket";
const DEFAULT_CPU_PROFILE_SECONDS: u64 = 30;
const DEFAULT_CPU_PROFILE_FREQUENCY: i32 = 100;

/// pprof HTTP query 参数。
///
/// 适用于 `/debug/pprof/heap` 和 `/debug/pprof/profile`。
/// Example: `?debug=1&seconds=5 -> 可读文本，CPU 采样 5 秒`。
#[derive(Debug, Deserialize)]
struct PprofQuery {
    /// Go pprof 兼容开关，1 表示输出可读文本。
    debug: Option<u8>,
    /// CPU profile 采集秒数，仅 `/debug/pprof/profile` 使用。
    seconds: Option<u64>,
    /// CPU profile 采样频率，仅 `/debug/pprof/profile` 使用。
    frequency: Option<i32>,
}

impl PprofQuery {
    /// 判断请求是否要求可读文本。
    ///
    /// 适用于兼容 Go pprof 的 `debug=1` 参数。
    /// Example: `debug=Some(1) -> true`。
    fn debug_enabled(&self) -> bool {
        self.debug == Some(1)
    }
}

/// 启动 internal pprof HTTP server。
///
/// 适用于 `pprof-run` 排查版启动时挂载本地 domain socket。
/// Example: `~/.gsdv/.socket 不存在 -> 后台监听 HTTP`。
#[cfg(all(feature = "pprof-run", unix))]
pub(crate) fn spawn_internal_server() {
    let Some(socket_path) = internal_socket_path() else {
        eprintln!("failed to resolve HOME for internal pprof socket");
        return;
    };

    thread::Builder::new()
        .name("gsdv-internal".to_string())
        .spawn(move || run_internal_server_thread(socket_path))
        .ok();
}

/// 启动 internal pprof HTTP server。
///
/// 适用于暂未提供当前平台 domain socket HTTP 的排查版。
/// Example: `windows -> no-op`。
#[cfg(not(all(feature = "pprof-run", unix)))]
pub(crate) fn spawn_internal_server() {}

/// 运行 internal server 专用线程。
///
/// 适用于避免依赖 GUI runtime 生命周期的 debug HTTP 入口。
/// Example: `bind 成功 -> current-thread tokio runtime serve axum`。
#[cfg(all(feature = "pprof-run", unix))]
fn run_internal_server_thread(socket_path: PathBuf) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("failed to start internal pprof runtime: {error}");
            return;
        }
    };

    let _guard = runtime.enter();
    let listener = match bind_internal_listener(&socket_path) {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => return,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return,
        Err(error) => {
            eprintln!(
                "failed to bind internal pprof socket {}: {error}",
                socket_path.display()
            );
            return;
        }
    };
    drop(_guard);

    runtime.block_on(async move {
        if let Err(error) = axum::serve(listener, internal_router()).await {
            eprintln!("internal pprof server failed: {error}");
        }
    });
}

/// 绑定 internal domain socket。
///
/// 适用于 Linux/macOS 的 pathname Unix domain socket。
/// Example: `~/.gsdv/.socket -> tokio UnixListener`。
#[cfg(all(feature = "pprof-run", unix))]
fn bind_internal_listener(socket_path: &PathBuf) -> std::io::Result<tokio::net::UnixListener> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // 为什么需要它:
    // - 触发条件: axum 需要 tokio listener，但 std 支持直接绑定 pathname UDS。
    // - 不能直接用常规路径的原因: tokio::net::UnixListener::bind 只能收 path，
    //   但这里要先设置 nonblocking，再交给 tokio runtime 接管。
    // - 防止什么副作用或回归: 不删除已有 socket，避免抢占已经存在的 gsdv。
    let listener = std::os::unix::net::UnixListener::bind(socket_path)?;
    listener.set_nonblocking(true)?;
    tokio::net::UnixListener::from_std(listener)
}

/// 返回 internal socket 路径。
///
/// 适用于统一 Linux/macOS 的 pathname UDS 入口。
/// Example: `HOME=/home/a -> /home/a/.gsdv/.socket`。
#[cfg(all(feature = "pprof-run", unix))]
fn internal_socket_path() -> Option<PathBuf> {
    Some(
        PathBuf::from(std::env::var_os("HOME")?)
            .join(".gsdv")
            .join(INTERNAL_SOCKET_FILE),
    )
}

/// 构建 internal HTTP router。
///
/// 适用于只暴露 debug pprof 路径的 domain socket server。
/// Example: `/debug/pprof/heap -> heap handler`。
fn internal_router() -> Router {
    Router::new()
        .route("/debug/pprof/heap", get(heap_handler))
        .route("/debug/pprof/profile", get(profile_handler))
}

/// 处理 heap pprof 请求。
///
/// 适用于读取当前 sampled heap profile 或可读 memory snapshot。
/// Example: `/debug/pprof/heap?debug=1 -> text/plain`。
async fn heap_handler(Query(query): Query<PprofQuery>) -> Response {
    let mut body = Vec::new();
    let options = crate::common::pprof::HeapProfileOptions {
        debug: query.debug_enabled(),
    };
    match crate::common::pprof::write_heap_profile(&mut body, options) {
        Ok(()) => pprof_response(body, query.debug_enabled()),
        Err(error) => error_response(error),
    }
}

/// 处理 CPU pprof 请求。
///
/// 适用于按请求临时采样 CPU profile。
/// Example: `/debug/pprof/profile?seconds=5 -> 5 秒 CPU pprof`。
async fn profile_handler(Query(query): Query<PprofQuery>) -> Response {
    let seconds = query.seconds.unwrap_or(DEFAULT_CPU_PROFILE_SECONDS).max(1);
    let frequency = query
        .frequency
        .unwrap_or(DEFAULT_CPU_PROFILE_FREQUENCY)
        .max(1);
    let mut body = Vec::new();
    let options = crate::common::pprof::CpuProfileOptions {
        duration: Duration::from_secs(seconds),
        frequency,
        debug: query.debug_enabled(),
    };
    match crate::common::pprof::write_cpu_profile(&mut body, options).await {
        Ok(()) => pprof_response(body, query.debug_enabled()),
        Err(error) => error_response(error),
    }
}

/// 构建 pprof HTTP response。
///
/// 适用于二进制 pprof 和 debug 文本两种返回。
/// Example: `debug=true -> text/plain`。
fn pprof_response(body: Vec<u8>, debug: bool) -> Response {
    let content_type = if debug {
        "text/plain; charset=utf-8"
    } else {
        "application/octet-stream"
    };
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(content_type),
    );
    response
}

/// 构建 internal error response。
///
/// 适用于 profile 生成失败或 feature 未启用的请求。
/// Example: `anyhow error -> 500 text/plain`。
fn error_response(error: anyhow::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        error.to_string(),
    )
        .into_response()
}

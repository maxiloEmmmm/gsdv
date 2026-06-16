use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::{Path, PathBuf};

/// 外部 hook 帧最大 payload，避免异常客户端撑爆内存。
pub(crate) const MAX_HOOK_PAYLOAD_LEN: usize = 64 * 1024;
/// Helix 当前文件位置 hook key。
pub(crate) const HELIX_CURRENT_KEY: &str = "helix.current";
/// Agent 状态变化 hook key。
pub(crate) const AGENT_STATUS_KEY: &str = "agent.status";

/// 外部 hook 事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExternalHookEvent {
    /// hook key，用于区分不同来源和语义。
    pub key: String,
    /// key 后面的原始数据。
    pub data: String,
}

/// 返回 app 级 hook socket/pipe 地址。
pub(crate) fn app_hook_endpoint() -> String {
    if cfg!(windows) {
        format!(r"\\.\pipe\gsdv-hook-{}", std::process::id())
    } else {
        app_hook_socket_path().display().to_string()
    }
}

/// 返回固定 Helix 临时配置路径。
pub(crate) fn helix_hook_config_path() -> PathBuf {
    std::env::temp_dir().join("gsdv-helix-hook-config.toml")
}

/// 构建 Helix hook 配置内容。
pub(crate) fn helix_hook_config(endpoint: &str, gsdv_exe: &Path) -> String {
    let exe = shell_arg(&gsdv_exe.to_string_lossy());
    let endpoint = shell_arg(endpoint);
    let command = format!(
        "\":sh {exe} hook-client --endpoint {endpoint} --key {HELIX_CURRENT_KEY} --data %{{buffer_name}}:%{{cursor_line}}\""
    );
    let bindings = vec![format!("A-d = {command}"), format!("Cmd-d = {command}")];
    merge_helix_key_bindings(&read_user_helix_config().unwrap_or_default(), &bindings)
}

/// 合并 Helix keymap，避免重复 key 表头。
pub(crate) fn merge_helix_key_binding(config: &str, binding: &str) -> String {
    merge_helix_key_bindings(config, &[binding.to_string()])
}

/// 合并多条 Helix keymap，避免重复 key 表头。
pub(crate) fn merge_helix_key_bindings(config: &str, bindings: &[String]) -> String {
    let config = merge_helix_key_binding_table(config, "[keys.normal]", bindings);
    let config = merge_helix_key_binding_table(&config, "[keys.insert]", bindings);
    merge_helix_key_binding_table(&config, "[keys.select]", bindings)
}

/// 合并单个 Helix keymap 表。
fn merge_helix_key_binding_table(config: &str, table: &str, bindings: &[String]) -> String {
    let mut lines = config.lines().map(str::to_string).collect::<Vec<_>>();
    let mut table_start = None;
    let mut table_end = lines.len();

    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed == table {
            table_start = Some(index);
            continue;
        }
        if table_start.is_some()
            && index > table_start.unwrap()
            && trimmed.starts_with('[')
            && trimmed.ends_with(']')
        {
            table_end = index;
            break;
        }
    }

    if let Some(start) = table_start {
        let mut index = start + 1;
        while index < table_end {
            if toml_line_assigns_any_key(lines[index].trim(), &["A-d", "Cmd-d"]) {
                lines.remove(index);
                table_end -= 1;
                continue;
            }
            index += 1;
        }
        for binding in bindings.iter().rev() {
            lines.insert(start + 1, binding.to_string());
        }
    } else {
        if !lines.is_empty() && lines.last().is_some_and(|line| !line.is_empty()) {
            lines.push(String::new());
        }
        lines.push("# gsdv 临时 hook keymap，固定文件名会在每次启动 Helix 前重写。".to_string());
        lines.push(table.to_string());
        lines.extend(bindings.iter().cloned());
    }

    let mut merged = lines.join("\n");
    merged.push('\n');
    merged
}

/// 判断 TOML 行是否给任一 bare key 赋值。
fn toml_line_assigns_any_key(trimmed: &str, keys: &[&str]) -> bool {
    keys.iter().any(|key| toml_line_assigns_key(trimmed, key))
}

/// 读取用户 Helix 配置内容，用于生成临时合并配置。
fn read_user_helix_config() -> Option<String> {
    for path in user_helix_config_candidates() {
        if let Ok(content) = std::fs::read_to_string(path) {
            return Some(content);
        }
    }
    None
}

/// 返回 Helix 用户配置候选路径。
fn user_helix_config_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(config_home) = std::env::var("XDG_CONFIG_HOME") {
        paths.push(PathBuf::from(config_home).join("helix/config.toml"));
    }
    if let Some(home) = crate::home::home_dir() {
        paths.push(home.join(".config/helix/config.toml"));
        paths.push(home.join(".helix/config.toml"));
    }
    paths
}

/// 判断 TOML 行是否给指定 key 赋值。
fn toml_line_assigns_key(trimmed: &str, key: &str) -> bool {
    let bare_key = trimmed.strip_prefix(key);
    let double_quoted = trimmed
        .strip_prefix('"')
        .and_then(|rest| rest.strip_prefix(key))
        .and_then(|rest| rest.strip_prefix('"'));
    let single_quoted = trimmed
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_prefix(key))
        .and_then(|rest| rest.strip_prefix('\''));
    [bare_key, double_quoted, single_quoted]
        .into_iter()
        .flatten()
        .any(|rest| rest.trim_start().starts_with('='))
}

/// 写入固定路径的 Helix hook 配置。
pub(crate) fn write_helix_hook_config(endpoint: &str, gsdv_exe: &Path) -> Result<PathBuf> {
    let path = helix_hook_config_path();
    std::fs::write(&path, helix_hook_config(endpoint, gsdv_exe))
        .with_context(|| format!("failed to write {}", path.display()))?;
    hook_info(format_args!(
        "wrote helix config path={} endpoint={} exe={}",
        path.display(),
        endpoint,
        gsdv_exe.display()
    ));
    Ok(path)
}

/// 构建 big-endian 长度前缀 hook 帧。
pub(crate) fn encode_hook_frame(key: &str, data: &str) -> Result<Vec<u8>> {
    let payload = format!("{key}:{data}");
    let len = payload.len();
    if len > MAX_HOOK_PAYLOAD_LEN {
        bail!("hook payload too large: {len}");
    }
    let len = u32::try_from(len).context("hook payload length exceeds u32")?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(payload.as_bytes());
    Ok(frame)
}

/// 解析 hook payload。
pub(crate) fn parse_hook_payload(payload: &[u8]) -> Result<ExternalHookEvent> {
    let payload = std::str::from_utf8(payload).context("hook payload is not utf-8")?;
    let Some((key, data)) = payload.split_once(':') else {
        bail!("hook payload missing key separator");
    };
    if key.is_empty() {
        bail!("hook payload key is empty");
    }
    Ok(ExternalHookEvent {
        key: key.to_string(),
        data: data.to_string(),
    })
}

/// hook-client 子命令入口。
pub(crate) fn run_hook_client_from_args<I>(args: I) -> Result<()>
where
    I: IntoIterator<Item = String>,
{
    let mut endpoint = None;
    let mut key = None;
    let mut data = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--endpoint" => endpoint = args.next(),
            "--key" => key = args.next(),
            "--data" => data = args.next(),
            other => bail!("unknown hook-client argument: {other}"),
        }
    }
    let endpoint = endpoint.context("missing --endpoint")?;
    let key = key.context("missing --key")?;
    let data = data.context("missing --data")?;
    let data = normalize_client_data(&key, &data);
    send_hook_event(&endpoint, &key, &data)
}

/// 向指定 endpoint 发送 hook 事件。
pub(crate) fn send_hook_event(endpoint: &str, key: &str, data: &str) -> Result<()> {
    let frame = encode_hook_frame(key, data)?;
    write_hook_frame(endpoint, &frame)
}

/// 规范化 hook-client 数据，保证 Helix current 发送绝对路径。
pub(crate) fn normalize_client_data(key: &str, data: &str) -> String {
    if key != HELIX_CURRENT_KEY {
        return data.to_string();
    }
    let Some((file, line)) = data.rsplit_once(':') else {
        return data.to_string();
    };
    let path = PathBuf::from(file);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|dir| dir.join(path))
            .unwrap_or_else(|_| PathBuf::from(file))
    };
    format!("{}:{line}", absolute.display())
}

/// 判断 hook 诊断日志是否开启。
pub(crate) fn hook_info_enabled() -> bool {
    std::env::var_os("GSDV_HOOK_INFO").is_some() || std::env::var_os("GSDV_HOOK_DEBUG").is_some()
}

/// 输出 hook 诊断日志，用于确认外部数据是否进入 app。
pub(crate) fn hook_info(args: std::fmt::Arguments<'_>) {
    if hook_info_enabled() {
        eprintln!("[gsdv][hook] {args}");
    }
}

/// 向 app hook endpoint 写入完整帧。
fn write_hook_frame(endpoint: &str, frame: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;

        let mut stream = UnixStream::connect(endpoint)
            .with_context(|| format!("failed to connect hook socket {endpoint}"))?;
        stream
            .write_all(frame)
            .context("failed to write hook frame")?;
        return Ok(());
    }

    #[cfg(windows)]
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(endpoint)
            .with_context(|| format!("failed to open hook pipe {endpoint}"))?;
        file.write_all(frame)
            .context("failed to write hook frame")?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    {
        bail!("hook endpoint is unsupported on this platform")
    }
}

#[cfg(unix)]
fn app_hook_socket_path() -> PathBuf {
    std::env::temp_dir().join(format!("gsdv-hook-{}.sock", std::process::id()))
}

/// 用单引号包裹 shell 参数。
fn shell_arg(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

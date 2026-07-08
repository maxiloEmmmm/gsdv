use anyhow::Result;
#[cfg(feature = "pprof-run")]
use pprof::protos::Message;
use std::io::Write;
use std::time::Duration;

/// CPU profile 的采集参数。
///
/// 适用于 `/debug/pprof/profile` 这种按请求临时采样的场景。
/// Example: `seconds=30, frequency=100 -> 采集 30 秒 100Hz profile`。
pub(crate) struct CpuProfileOptions {
    /// CPU 采样持续时间。
    pub duration: Duration,
    /// 每秒采样频率。
    pub frequency: i32,
    /// 是否输出可读文本而不是 pprof protobuf。
    pub debug: bool,
}

/// Heap profile 的输出参数。
///
/// 适用于 `/debug/pprof/heap` 这种读取当前 allocator 样本的场景。
/// Example: `debug=true -> 输出可读 snapshot`。
pub(crate) struct HeapProfileOptions {
    /// 是否输出可读文本而不是 pprof protobuf。
    pub debug: bool,
}

/// 写出当前 heap profile。
///
/// 适用于已经启用 `pprof-run` feature 的排查版进程。
/// Example: `debug=false -> writer 收到 pprof heap protobuf bytes`。
#[cfg(feature = "pprof-run")]
pub(crate) fn write_heap_profile<W: Write>(
    mut writer: W,
    options: HeapProfileOptions,
) -> Result<()> {
    if options.debug {
        let snapshot = pprof_alloc::snapshot();
        serde_json::to_writer_pretty(&mut writer, &snapshot)?;
        writer.write_all(b"\n")?;
        return Ok(());
    }

    let profile = pprof_alloc::generate_pprof()?;
    writer.write_all(&profile)?;
    Ok(())
}

/// 写出当前 heap profile。
///
/// 适用于未启用 `pprof-run` feature 的普通进程。
/// Example: `debug=false -> 返回 feature 未启用错误`。
#[cfg(not(feature = "pprof-run"))]
pub(crate) fn write_heap_profile<W: Write>(_writer: W, _options: HeapProfileOptions) -> Result<()> {
    anyhow::bail!("pprof-run feature is not enabled")
}

/// 写出一段 CPU profile。
///
/// 适用于按 HTTP 请求临时启动 CPU 采样，然后写入 response/file。
/// Example: `seconds=30 -> writer 收到 30 秒 CPU pprof`。
#[cfg(feature = "pprof-run")]
pub(crate) async fn write_cpu_profile<W: Write>(
    mut writer: W,
    options: CpuProfileOptions,
) -> Result<()> {
    let guard = pprof::ProfilerGuard::new(options.frequency)?;
    tokio::time::sleep(options.duration).await;
    let report = guard.report().build()?;

    if options.debug {
        write!(writer, "{report:?}")?;
        return Ok(());
    }

    let profile = report.pprof()?;
    let mut bytes = Vec::new();
    profile.write_to_vec(&mut bytes)?;
    writer.write_all(&bytes)?;
    Ok(())
}

/// 写出一段 CPU profile。
///
/// 适用于未启用 `pprof-run` feature 的普通进程。
/// Example: `seconds=30 -> 返回 feature 未启用错误`。
#[cfg(not(feature = "pprof-run"))]
pub(crate) async fn write_cpu_profile<W: Write>(
    _writer: W,
    _options: CpuProfileOptions,
) -> Result<()> {
    anyhow::bail!("pprof-run feature is not enabled")
}

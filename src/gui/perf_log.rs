//! 短时性能计数日志。
//!
//! 本模块只在 `GSDV_PERF_LOG=1` 时聚合写入 `/tmp/gsdv-perf.log`，
//! 用于定位 repaint、AppEvent 和 terminal runtime 的真实频率。

use std::collections::BTreeMap;
use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PERF_LOG_ENV: &str = "GSDV_PERF_LOG";
const PERF_LOG_PATH: &str = "/tmp/gsdv-perf.log";
const PERF_NOTE_LOG_PATH: &str = "/tmp/gsdv-repaint-causes.log";
const PERF_LOG_WINDOW: Duration = Duration::from_secs(1);

static PERF_COUNTERS: OnceLock<Mutex<PerfCounters>> = OnceLock::new();
static PERF_ENABLED: OnceLock<bool> = OnceLock::new();
static PERF_NOTES: OnceLock<Mutex<BTreeMap<&'static str, Instant>>> = OnceLock::new();

/// 性能计数窗口状态。
struct PerfCounters {
    /// 当前聚合窗口的开始时间。
    window_started_at: Instant,
    /// 当前窗口内每个标签的触发次数。
    counts: BTreeMap<&'static str, u64>,
}

impl PerfCounters {
    /// 创建新的聚合窗口，适用于首次记录性能计数。
    fn new(now: Instant) -> Self {
        Self {
            window_started_at: now,
            counts: BTreeMap::new(),
        }
    }
}

/// 判断性能计数日志是否启用。
pub(crate) fn enabled() -> bool {
    *PERF_ENABLED.get_or_init(|| {
        env::var_os(PERF_LOG_ENV)
            .and_then(|value| value.into_string().ok())
            .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
    })
}

/// 记录一个性能事件计数，适用于高频路径的低成本聚合。
pub(crate) fn count(label: &'static str) {
    add(label, 1);
}

/// 累加一个性能数值，适用于 cell 数、字节数等窗口内总量。
pub(crate) fn add(label: &'static str, amount: u64) {
    if amount == 0 {
        return;
    }
    if !enabled() {
        return;
    }
    let now = Instant::now();
    let counters = PERF_COUNTERS.get_or_init(|| Mutex::new(PerfCounters::new(now)));
    let mut pending_counts = None;
    let Ok(mut guard) = counters.lock() else {
        return;
    };
    if now.duration_since(guard.window_started_at) >= PERF_LOG_WINDOW {
        pending_counts = Some(std::mem::take(&mut guard.counts));
        guard.window_started_at = now;
    }
    *guard.counts.entry(label).or_default() += amount;
    drop(guard);
    if let Some(counts) = pending_counts {
        write_counts(now, counts);
    }
}

/// 累加耗时微秒，适用于每秒总耗时和平均耗时分析。
pub(crate) fn duration_us(label: &'static str, duration: Duration) {
    add(label, duration_micros(duration));
}

/// 限频写入诊断文本，适用于记录 repaint cause 这类动态内容。
pub(crate) fn note_throttled(label: &'static str, interval: Duration, message: &str) {
    if !enabled() {
        return;
    }
    let now = Instant::now();
    let notes = PERF_NOTES.get_or_init(|| Mutex::new(BTreeMap::new()));
    let Ok(mut guard) = notes.lock() else {
        return;
    };
    if guard
        .get(label)
        .is_some_and(|last| now.duration_since(*last) < interval)
    {
        return;
    }
    guard.insert(label, now);
    drop(guard);
    write_note(label, message);
}

/// 把 Duration 转成饱和微秒，避免极端长耗时溢出 u64。
fn duration_micros(duration: Duration) -> u64 {
    duration.as_micros().min(u64::MAX as u128) as u64
}

/// 写入一个聚合窗口，适用于把内存计数转换成日志行。
fn write_counts(_now: Instant, counts: BTreeMap<&'static str, u64>) {
    if counts.is_empty() {
        return;
    }
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let mut line = format!("{timestamp_ms}");
    for (label, count) in counts {
        line.push(' ');
        line.push_str(label);
        line.push('=');
        line.push_str(&count.to_string());
    }
    let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(PERF_LOG_PATH)
    else {
        return;
    };
    let _ = writeln!(file, "{line}");
}

/// 追加一行动态诊断文本，适用于人工阅读而不是 awk 聚合。
fn write_note(label: &'static str, message: &str) {
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(PERF_NOTE_LOG_PATH)
    else {
        return;
    };
    let _ = writeln!(file, "{timestamp_ms} {label} {message}");
}

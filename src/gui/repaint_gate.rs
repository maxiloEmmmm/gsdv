//! App repaint 的无锁 FPS 闸门。
//!
//! 本模块只处理 egui 唤醒频率，不表达业务状态。后台任务仍然只能投
//! `AppEvent`，这里负责把多路唤醒合并到同一个 FPS 节拍上。

use eframe::egui;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

/// 跨线程共享的 repaint 唤醒控制器。
#[derive(Clone)]
pub(crate) struct RepaintController {
    /// 原子毫秒刻度的起点，所有 clone 必须共用同一个起点。
    origin: Instant,
    /// 最近一次 egui update 开始的毫秒刻度，0 表示尚未进入过 update。
    last_update_ms: Arc<AtomicU64>,
    /// 当前 FPS 配置对应的最小帧间隔毫秒。
    frame_interval_ms: Arc<AtomicU64>,
    /// 已排队的 app repaint deadline 毫秒刻度，0 表示没有待触发 repaint。
    next_repaint_ms: Arc<AtomicU64>,
    /// 已排队的业务 deadline 毫秒刻度，0 表示没有待触发 deadline。
    next_timed_update_ms: Arc<AtomicU64>,
}

impl RepaintController {
    /// 创建 repaint 控制器，适用于单个 app 实例的所有唤醒入口。
    pub(crate) fn new() -> Self {
        Self {
            origin: Instant::now(),
            last_update_ms: Arc::new(AtomicU64::new(0)),
            frame_interval_ms: Arc::new(AtomicU64::new(1)),
            next_repaint_ms: Arc::new(AtomicU64::new(0)),
            next_timed_update_ms: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 记录一次 update 已开始，并清理真正到期的 repaint deadline。
    pub(crate) fn frame_started(&self, frame_interval: Duration) {
        let now_ms = self.elapsed_millis();
        let frame_interval_ms = duration_millis(frame_interval);
        count_frame_interval(frame_interval_ms);
        self.frame_interval_ms
            .store(frame_interval_ms, Ordering::Release);
        self.last_update_ms.store(now_ms, Ordering::Release);
        self.clear_consumed_repaint(now_ms);
        self.clear_consumed_timed_update(now_ms);
    }

    /// 请求一次 repaint，适用于后台任务完成或 UI 状态变脏后的唤醒。
    pub(crate) fn request_repaint(&self, ctx: &egui::Context) {
        crate::gui::perf_log::count("repaint_gate.request");
        let last_update_ms = self.last_update_ms.load(Ordering::Acquire);
        if last_update_ms == 0 {
            crate::gui::perf_log::count("repaint_gate.ignored_no_update");
            return;
        }
        let now_ms = self.elapsed_millis();
        let frame_interval_ms = self.frame_interval_ms.load(Ordering::Acquire).max(1);
        let deadline_base_ms = last_update_ms.max(now_ms);
        let deadline_ms = deadline_base_ms.saturating_add(frame_interval_ms);
        if !self.schedule_repaint_deadline(now_ms, deadline_ms) {
            crate::gui::perf_log::count("repaint_gate.coalesced");
            return;
        }
        let delay_ms = deadline_ms.saturating_sub(now_ms);
        count_repaint_delay(delay_ms);
        crate::gui::perf_log::count("repaint_gate.scheduled");
        ctx.request_repaint_after(Duration::from_millis(delay_ms));
    }

    /// 请求一次业务 deadline 唤醒，适用于 store debounce 等低频定时。
    pub(crate) fn request_timed_update(&self, ctx: &egui::Context, duration: Duration) {
        crate::gui::perf_log::count("repaint_gate.timed_update");
        let now_ms = self.elapsed_millis();
        let deadline_ms = now_ms.saturating_add(duration_millis(duration));
        if !schedule_deadline(
            &self.next_timed_update_ms,
            now_ms,
            deadline_ms,
            "repaint_gate.timed_stale",
        ) {
            crate::gui::perf_log::count("repaint_gate.timed_coalesced");
            return;
        }
        crate::gui::perf_log::count("repaint_gate.timed_scheduled");
        ctx.request_repaint_after(duration);
    }

    /// 返回从 controller 创建以来经过的毫秒数。
    fn elapsed_millis(&self) -> u64 {
        duration_millis(self.origin.elapsed())
    }

    /// 合并 app repaint deadline，适用于避免旧 future 回调制造滚动队列。
    fn schedule_repaint_deadline(&self, now_ms: u64, deadline_ms: u64) -> bool {
        schedule_deadline(
            &self.next_repaint_ms,
            now_ms,
            deadline_ms,
            "repaint_gate.stale",
        )
    }

    /// 清除已经真正到期的 app repaint deadline。
    fn clear_consumed_repaint(&self, now_ms: u64) {
        clear_consumed_deadline(&self.next_repaint_ms, now_ms);
    }

    /// 清除已经触发的业务 deadline，适用于 update 已被唤醒之后。
    fn clear_consumed_timed_update(&self, now_ms: u64) {
        clear_consumed_deadline(&self.next_timed_update_ms, now_ms);
    }
}

/// 合并 deadline，已过期但未消费的 deadline 允许被重新调度。
fn schedule_deadline(
    deadline: &AtomicU64,
    now_ms: u64,
    deadline_ms: u64,
    stale_label: &'static str,
) -> bool {
    let mut current = deadline.load(Ordering::Acquire);
    loop {
        if current != 0 && current <= now_ms {
            crate::gui::perf_log::count(stale_label);
        } else if current != 0 && current <= deadline_ms {
            return false;
        }
        match deadline.compare_exchange(current, deadline_ms, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return true,
            Err(next) => current = next,
        }
    }
}

/// 清除已经到期的 deadline，适用于不可取消的 egui future wakeup。
fn clear_consumed_deadline(deadline: &AtomicU64, now_ms: u64) {
    let mut current = deadline.load(Ordering::Acquire);
    loop {
        if current == 0 || current > now_ms {
            return;
        }
        match deadline.compare_exchange(current, 0, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return,
            Err(next) => current = next,
        }
    }
}

/// 将 Duration 转成至少 1ms 的原子计数值。
fn duration_millis(duration: Duration) -> u64 {
    let millis = duration.as_millis().max(1);
    millis.min(u128::from(u64::MAX)) as u64
}

/// 记录 FPS 间隔桶，适用于确认运行时配置是否按 store 生效。
fn count_frame_interval(frame_interval_ms: u64) {
    let label = match frame_interval_ms {
        0..=16 => "repaint_gate.interval_le_16",
        17..=20 => "repaint_gate.interval_17_20",
        21..=25 => "repaint_gate.interval_21_25",
        26..=40 => "repaint_gate.interval_26_40",
        _ => "repaint_gate.interval_gt_40",
    };
    crate::gui::perf_log::count(label);
}

/// 记录 repaint delay 桶，适用于确认下一帧是否真的被延后。
fn count_repaint_delay(delay_ms: u64) {
    let label = match delay_ms {
        0 => "repaint_gate.delay_0",
        1..=8 => "repaint_gate.delay_1_8",
        9..=20 => "repaint_gate.delay_9_20",
        21..=40 => "repaint_gate.delay_21_40",
        _ => "repaint_gate.delay_gt_40",
    };
    crate::gui::perf_log::count(label);
}

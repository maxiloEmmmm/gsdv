//! TUI 番茄控制层、设置表单和全屏休息提示。

use super::*;

/// 构造现有 RuntimeSettings 三个番茄参数的编辑表单。
///
/// 适用场景：Alt 面板 Settings。示例：25/5/20 -> 保存同一份全局设置。
pub(super) fn pomodoro_settings_form(app: &GsdvGuiApp) -> FormDialog {
    FormDialog {
        title: "Pomodoro Settings".to_string(),
        fields: vec![
            FormField::Text {
                label: "Work minutes".to_string(),
                value: app.runtime_settings.pomodoro_work_minutes.to_string(),
            },
            FormField::Text {
                label: "Rest minutes".to_string(),
                value: app.runtime_settings.pomodoro_rest_minutes.to_string(),
            },
            FormField::Text {
                label: "Warning %".to_string(),
                value: app
                    .runtime_settings
                    .pomodoro_warning_remaining_percent
                    .to_string(),
            },
        ],
        active: 0,
        submit: FormSubmit::PomodoroSettings,
        error: None,
    }
}

/// 返回居中的番茄控制面板矩形。
///
/// 适用场景：绘制和鼠标命中共用。示例：157x57 -> 居中 68x11。
pub(super) fn pomodoro_panel_rect(size: (u16, u16)) -> (u16, u16, u16, u16) {
    let width = 68_u16.min(size.0.saturating_sub(4)).max(44);
    let height = 11_u16.min(size.1.saturating_sub(2)).max(9);
    (
        size.0.saturating_sub(width) / 2,
        size.1.saturating_sub(height) / 2,
        width,
        height,
    )
}

/// 绘制 Alt 打开的彩色番茄状态和操作层。
///
/// 适用场景：TuiLayer::Pomodoro。示例：Working -> 蓝色状态、黄色倒计时。
pub(super) fn draw_pomodoro_panel(frame: &mut [RenderLine], size: (u16, u16), app: &GsdvGuiApp) {
    let (x, y, width, height) = pomodoro_panel_rect(size);
    draw_box(frame, x, y, width, height, "POMODORO");
    let enabled = app.runtime_settings.pomodoro_enabled;
    let phase = pomodoro_phase_label(app.pomodoro.phase);
    let remaining_minutes = pomodoro_remaining_minutes(app).unwrap_or_default();
    let warning = app.pomodoro.phase == PomodoroPhase::Working
        && pomodoro_work_progress(&app.runtime_settings, &app.pomodoro)
            >= pomodoro_warning_progress(&app.runtime_settings);
    styled_overlay_text(
        frame,
        y + 2,
        x + 2,
        &format!("STATUS  {}", if enabled { phase } else { "DISABLED" }),
        if enabled {
            (96, 165, 250)
        } else {
            (148, 163, 184)
        },
        (24, 33, 43),
        true,
    );
    styled_overlay_text(
        frame,
        y + 3,
        x + 2,
        &format!("REMAIN  {remaining_minutes}m"),
        if warning {
            (250, 204, 21)
        } else {
            (45, 212, 191)
        },
        (24, 33, 43),
        true,
    );
    overlay_text(
        frame,
        y + 5,
        x + 2,
        &format!(
            "Work {}m   Rest {}m   Warn at {}% remaining",
            app.runtime_settings.pomodoro_work_minutes,
            app.runtime_settings.pomodoro_rest_minutes,
            app.runtime_settings.pomodoro_warning_remaining_percent
        ),
    );
    styled_overlay_text(
        frame,
        y + 7,
        x + 2,
        if enabled { "[Disable]" } else { "[Enable] " },
        (255, 255, 255),
        if enabled {
            (71, 85, 105)
        } else {
            (22, 163, 74)
        },
        true,
    );
    styled_overlay_text(
        frame,
        y + 7,
        x + 15,
        "[Work W]",
        (255, 255, 255),
        (37, 99, 235),
        true,
    );
    styled_overlay_text(
        frame,
        y + 7,
        x + 30,
        "[Rest R]",
        (255, 255, 255),
        (220, 38, 38),
        true,
    );
    styled_overlay_text(
        frame,
        y + 7,
        x + 43,
        "[Settings S]",
        (15, 23, 42),
        (250, 204, 21),
        true,
    );
    overlay_text(frame, y + height - 2, x + 2, "Esc close · Alt+P fallback");
}

/// 绘制工作到点后持续到 Esc 的全屏休息提示。
///
/// 适用场景：Resting、WaitingForRestQuiet、ReadyToWork。示例：休息结束仍显示 R。
pub(super) fn draw_pomodoro_rest_screen(
    frame: &mut [RenderLine],
    size: (u16, u16),
    app: &GsdvGuiApp,
) {
    for y in 0..size.1 {
        set_base_text(frame, y, 0, &" ".repeat(size.0 as usize), size.0);
        styled_overlay_text(
            frame,
            y,
            0,
            &" ".repeat(size.0 as usize),
            (248, 113, 113),
            (28, 8, 14),
            false,
        );
    }
    let art = [
        "██████╗ ",
        "██╔══██╗",
        "██████╔╝",
        "██╔══██╗",
        "██║  ██║",
        "╚═╝  ╚═╝",
    ];
    let art_width = art
        .iter()
        .map(|line| display_width(line))
        .max()
        .unwrap_or(8) as u16;
    let art_x = size.0.saturating_sub(art_width) / 2;
    let art_y = size.1.saturating_sub(art.len() as u16) / 2;
    for (offset, line) in art.iter().enumerate() {
        styled_overlay_text(
            frame,
            art_y + offset as u16,
            art_x,
            line,
            (248, 113, 113),
            (28, 8, 14),
            true,
        );
    }
    let remaining_minutes = pomodoro_remaining_minutes(app).unwrap_or_default();
    let status = if app.pomodoro.phase == PomodoroPhase::ReadyToWork {
        "REST COMPLETE".to_string()
    } else {
        format!("REST {remaining_minutes}m")
    };
    let status_x = size.0.saturating_sub(display_width(&status) as u16) / 2;
    styled_overlay_text(
        frame,
        art_y.saturating_add(art.len() as u16 + 2),
        status_x,
        &status,
        (250, 204, 21),
        (28, 8, 14),
        true,
    );
    let hint = "Esc · return to work";
    overlay_text(
        frame,
        size.1.saturating_sub(2),
        size.0.saturating_sub(display_width(hint) as u16) / 2,
        hint,
    );
}

/// 返回当前番茄阶段的人类可读标签。
///
/// 适用场景：底栏和 Alt 面板。示例：ReadyToWork -> READY。
pub(super) fn pomodoro_phase_label(phase: PomodoroPhase) -> &'static str {
    match phase {
        PomodoroPhase::Working => "WORK",
        PomodoroPhase::WaitingForRestQuiet => "QUIET",
        PomodoroPhase::Resting => "REST",
        PomodoroPhase::ReadyToWork => "READY",
        PomodoroPhase::ReturningToWork => "RETURN",
    }
}

/// 返回当前阶段剩余时间；禁用时不产生倒计时更新。
///
/// 适用场景：底栏、控制层和 Rest 层。示例：Working 1m elapsed -> work duration - 1m。
pub(super) fn pomodoro_remaining(app: &GsdvGuiApp) -> Option<Duration> {
    if !app.runtime_settings.pomodoro_enabled {
        return None;
    }
    let elapsed = Instant::now().saturating_duration_since(app.pomodoro.phase_started_at);
    Some(match app.pomodoro.phase {
        PomodoroPhase::Working => {
            pomodoro_work_duration(&app.runtime_settings).saturating_sub(elapsed)
        }
        PomodoroPhase::WaitingForRestQuiet => POMODORO_REST_QUIET_DURATION.saturating_sub(elapsed),
        PomodoroPhase::Resting => {
            pomodoro_rest_duration(&app.runtime_settings).saturating_sub(elapsed)
        }
        PomodoroPhase::ReadyToWork => Duration::ZERO,
        PomodoroPhase::ReturningToWork => POMODORO_RETURN_TO_WORK_DURATION.saturating_sub(elapsed),
    })
}

/// 返回向上取整的剩余分钟数，避免常驻 TUI 每秒产生 SSH 输出。
///
/// 适用场景：底栏、Alt 控制层和全屏 R。示例：24:01 -> 25m。
pub(super) fn pomodoro_remaining_minutes(app: &GsdvGuiApp) -> Option<u64> {
    pomodoro_remaining(app).map(|remaining| remaining.as_secs().div_ceil(60))
}

/// 返回番茄钟阶段与剩余时间底栏文字。
///
/// 适用场景：每个实际 frame。示例：Working -> `WORK 25m`。
pub(super) fn pomodoro_status(app: &GsdvGuiApp) -> String {
    if !app.runtime_settings.pomodoro_enabled {
        return "[Pomodoro off]".to_string();
    }
    let label = pomodoro_phase_label(app.pomodoro.phase);
    let remaining_minutes = pomodoro_remaining_minutes(app).unwrap_or_default();
    format!("[{label} {remaining_minutes}m]")
}

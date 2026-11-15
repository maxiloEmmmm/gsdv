//! TUI 字符布局、终端快照和 ANSI 差量渲染。

use super::*;

/// 按权重把总字符数分配给可见 row/column。
///
/// 适用场景：TUI 复用 GUI 布局权重。示例：100 与 [1,1] -> [50,50]。
pub(super) fn weighted_lengths(total: u16, weights: &[f32]) -> Vec<u16> {
    if weights.is_empty() {
        return Vec::new();
    }
    let sum = weights
        .iter()
        .copied()
        .map(|value| value.max(0.01))
        .sum::<f32>();
    let mut used = 0_u16;
    weights
        .iter()
        .enumerate()
        .map(|(index, weight)| {
            if index + 1 == weights.len() {
                return total.saturating_sub(used);
            }
            let value = ((f32::from(total) * weight.max(0.01) / sum).round() as u16).max(1);
            let value = value.min(total.saturating_sub(used));
            used = used.saturating_add(value);
            value
        })
        .collect()
}

/// 生成 pane 布局的低成本比较摘要。
///
/// 适用场景：避免每轮无意义重绘。示例：tab 变化 -> 不同字符串。
pub(super) fn pane_layout_fingerprint(panes: &[PaneLayout]) -> String {
    panes
        .iter()
        .map(|pane| {
            format!(
                "{}:{:?}:{}:{}:{}:{}:{}:{}",
                pane.target.workspace_index,
                pane.target.slot,
                pane.rect.x,
                pane.rect.y,
                pane.rect.width,
                pane.rect.height,
                pane.row_index,
                pane.column_index
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

/// 返回响应终端宽度的 sidebar 宽度。
///
/// 适用场景：窄 SSH 终端。示例：80 cols -> 19。
pub(super) fn sidebar_width(width: u16, collapsed: bool) -> u16 {
    if collapsed {
        TUI_COLLAPSED_SIDEBAR_WIDTH
    } else if width < 100 {
        TUI_COMPACT_SIDEBAR_WIDTH
    } else {
        TUI_SIDEBAR_WIDTH
    }
}

/// 生成 Agent 状态单字符。
///
/// 适用场景：rail 与 tab。示例：Busy -> `●`，Stopped -> `■`。
pub(super) fn status_marker(
    activity: WorkspaceActivity,
    stopped: bool,
    busy_frame: usize,
) -> &'static str {
    if stopped {
        return "■";
    }
    match activity {
        WorkspaceActivity::Busy => ["◐", "◓", "◑", "◒"][busy_frame % 4],
        WorkspaceActivity::Idle => "○",
        WorkspaceActivity::Error => "!",
        WorkspaceActivity::Unknown => "·",
    }
}

/// 返回 activity 在深色 xterm 表面上的语义前景色。
///
/// 适用场景：workspace、专注列表和 tab。示例：Busy -> bright green。
pub(super) fn activity_color(activity: WorkspaceActivity, stopped: bool) -> (u8, u8, u8) {
    if stopped {
        return (250, 204, 21);
    }
    match activity {
        WorkspaceActivity::Busy => (74, 222, 128),
        WorkspaceActivity::Idle => (34, 211, 238),
        WorkspaceActivity::Error => (248, 113, 113),
        WorkspaceActivity::Unknown => (148, 163, 184),
    }
}

/// 裁剪并补齐固定显示宽度，保证 ANSI 背景完整覆盖 hit region。
///
/// 适用场景：彩色 rail 行和 tab。示例：`abc`, width 5 -> `abc  `。
pub(super) fn pad_display_text(text: &str, width: u16) -> String {
    let clipped = clip_text(text, width);
    let padding = usize::from(width).saturating_sub(display_width(&clipped));
    format!("{clipped}{}", " ".repeat(padding))
}

/// 绘制一个具有选择背景和 activity 语义色的左侧导航行。
///
/// 适用场景：普通 workspace rail 与专注列表。示例：selected -> 蓝底白字。
pub(super) fn draw_navigation_row(
    frame: &mut [RenderLine],
    y: u16,
    width: u16,
    text: &str,
    activity: WorkspaceActivity,
    stopped: bool,
    selected: bool,
) {
    set_base_text(frame, y, 0, &" ".repeat(width as usize), width);
    styled_overlay_text(
        frame,
        y,
        0,
        &pad_display_text(text, width),
        if selected {
            (255, 255, 255)
        } else {
            activity_color(activity, stopped)
        },
        if selected {
            (37, 99, 235)
        } else {
            (11, 16, 22)
        },
        selected || activity == WorkspaceActivity::Busy,
    );
}

/// 绘制具有稳定宽度、active 背景和 activity 颜色的 Agent tab。
///
/// 适用场景：多 tab pane header。示例：active -> 蓝底；Busy inactive -> 绿字。
pub(super) fn draw_agent_tab(
    frame: &mut [RenderLine],
    y: u16,
    x: u16,
    width: u16,
    text: &str,
    activity: WorkspaceActivity,
    stopped: bool,
    active: bool,
) {
    set_base_text(frame, y, x, &" ".repeat(width as usize), width);
    styled_overlay_text(
        frame,
        y,
        x,
        &pad_display_text(text, width),
        if active {
            (255, 255, 255)
        } else {
            activity_color(activity, stopped)
        },
        if active {
            (37, 99, 235)
        } else if stopped {
            (66, 49, 12)
        } else {
            (30, 41, 59)
        },
        active || activity == WorkspaceActivity::Busy,
    );
}

/// 返回 Agent 显示名。
///
/// 适用场景：pane tab。示例：Main -> workspace name，subagent -> 自定义名称。
pub(super) fn agent_title(app: &GsdvGuiApp, target: &AgentTarget) -> String {
    let Some(workspace) = app.workspaces.get(target.workspace_index) else {
        return "missing".to_string();
    };
    match &target.slot {
        AgentSlotId::Main => workspace.name.clone(),
        AgentSlotId::Subagent(id) => workspace
            .subagents
            .iter()
            .find(|item| &item.id == id)
            .map(|item| item.name.clone())
            .unwrap_or_else(|| id.clone()),
    }
}

/// 把本机 terminal viewport 的 ANSI 行写入完整 TUI frame。
///
/// 适用场景：Agent pane 与 workspace terminal。示例：viewport 第 0 行 -> rect 第 0 行。
pub(super) fn draw_terminal_tui_frame(
    frame: &mut [RenderLine],
    rect: TuiRect,
    terminal: &TerminalTuiFrame,
) {
    for (offset, ansi) in terminal
        .rows
        .iter()
        .take(usize::from(rect.height))
        .enumerate()
    {
        let y = rect.y.saturating_add(offset as u16);
        if let Some(line) = frame.get_mut(usize::from(y)) {
            line.overlays.push((rect.x, ansi.clone()));
        }
    }
}

/// 把本机 terminal viewport 光标映射到完整 TUI 坐标。
///
/// 适用场景：当前 pane 获得输入焦点。示例：cell=(2,3) -> rect 偏移后坐标。
pub(super) fn terminal_tui_cursor_position(
    rect: TuiRect,
    terminal: &TerminalTuiFrame,
) -> Option<(u16, u16)> {
    let (col, row) = terminal.cursor?;
    Some((
        rect.x.saturating_add(
            u16::try_from(col)
                .unwrap_or(u16::MAX)
                .min(rect.width.saturating_sub(1)),
        ),
        rect.y.saturating_add(
            u16::try_from(row)
                .unwrap_or(u16::MAX)
                .min(rect.height.saturating_sub(1)),
        ),
    ))
}

/// 写出与上一帧不同的行，避免 SSH 反复传完整屏幕。
///
/// 适用场景：最多 fps 次每秒。示例：等宽 ANSI 动画 -> 原位覆盖 overlay。
pub(super) fn write_frame_diff(
    stdout: &mut Stdout,
    previous: &[RenderLine],
    next: &[RenderLine],
    size: (u16, u16),
    cursor: Option<(u16, u16)>,
) -> Result<()> {
    if previous.is_empty() {
        queue!(stdout, Hide, Clear(ClearType::All))?;
    }
    for (index, line) in next.iter().enumerate() {
        let previous_line = previous.get(index);
        if previous_line == Some(line) {
            continue;
        }
        if let Some(previous_line) = previous_line
            && render_line_overlays_can_overwrite(previous_line, line)
        {
            if let Some(first_changed) = previous_line
                .overlays
                .iter()
                .zip(&line.overlays)
                .position(|((_, previous_overlay), (_, overlay))| previous_overlay != overlay)
            {
                // 触发条件：底层 terminal overlay 变化，上层 Modal 保持不变。
                // 不能只重画变化项：整行 terminal 会覆盖后画的 Modal。
                // 从首个变化层恢复后续层，防止 Modal 视觉消失但仍截获输入。
                for (x, overlay) in line.overlays.iter().skip(first_changed) {
                    queue!(stdout, MoveTo(*x, index as u16), Print(overlay))?;
                }
            }
            continue;
        }
        queue!(
            stdout,
            MoveTo(0, index as u16),
            Clear(ClearType::CurrentLine),
            Print(&line.base)
        )?;
        for (x, overlay) in &line.overlays {
            queue!(stdout, MoveTo(*x, index as u16), Print(overlay))?;
        }
    }
    match cursor {
        Some((x, y)) => queue!(stdout, MoveTo(x, y), Show)?,
        None => queue!(
            stdout,
            MoveTo(size.0.saturating_sub(1), size.1.saturating_sub(1)),
            Hide
        )?,
    }
    stdout.flush()?;
    Ok(())
}

/// 判断新 overlay 能否不清行地完整覆盖旧 overlay。
///
/// 适用场景：Codex `Working` 仅更新 ANSI 颜色。示例：同坐标同宽文本 -> true。
fn render_line_overlays_can_overwrite(previous: &RenderLine, next: &RenderLine) -> bool {
    previous.base == next.base
        && previous.overlays.len() == next.overlays.len()
        && previous.overlays.iter().zip(&next.overlays).all(
            |((previous_x, previous), (next_x, next))| {
                previous_x == next_x && ansi_display_width(previous) == ansi_display_width(next)
            },
        )
}

/// 计算忽略 CSI ANSI 控制序列后的显示宽度。
///
/// 适用场景：判断新 overlay 是否会留下旧字符。示例：红色 `abc` -> 3。
fn ansi_display_width(text: &str) -> usize {
    /// ANSI 宽度扫描器的当前解析状态。
    #[derive(Clone, Copy)]
    enum EscapeState {
        /// 普通可见文本。
        Text,
        /// 已读取 ESC，等待控制序列类型。
        Escape,
        /// 正在读取 CSI 参数和终止字符。
        Csi,
    }

    let mut state = EscapeState::Text;
    let mut width = 0;
    for ch in text.chars() {
        match state {
            EscapeState::Text if ch == '\x1b' => state = EscapeState::Escape,
            EscapeState::Text => {
                if !ch.is_control() {
                    width += UnicodeWidthChar::width(ch).unwrap_or(0);
                }
            }
            EscapeState::Escape if ch == '[' => state = EscapeState::Csi,
            EscapeState::Escape => state = EscapeState::Text,
            EscapeState::Csi if ('@'..='~').contains(&ch) => state = EscapeState::Text,
            EscapeState::Csi => {}
        }
    }
    width
}

/// 在基础行写入限定宽度文本。
///
/// 适用场景：chrome。示例：长 workspace 名被截断。
pub(super) fn set_base_text(frame: &mut [RenderLine], y: u16, x: u16, text: &str, width: u16) {
    let Some(line) = frame.get_mut(usize::from(y)) else {
        return;
    };
    let target_len = usize::from(x.saturating_add(width));
    let mut chars = line.base.chars().collect::<Vec<_>>();
    if chars.len() < target_len {
        chars.resize(target_len, ' ');
    }
    let clipped = clip_text(text, width);
    let mut cursor = usize::from(x);
    for ch in clipped.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0).max(1);
        if cursor + ch_width > target_len {
            break;
        }
        chars[cursor] = ch;
        for fill in 1..ch_width {
            chars[cursor + fill] = ' ';
        }
        cursor += ch_width;
    }
    line.base = chars.into_iter().collect();
}

/// 写一个 chrome 分隔字符。
///
/// 适用场景：sidebar 边界。示例：x=27 -> `│`。
pub(super) fn put_char(frame: &mut [RenderLine], y: u16, x: u16, value: char) {
    set_base_text(frame, y, x, &value.to_string(), 1);
}

/// 添加覆盖文字，不改变基础字符坐标。
///
/// 适用场景：菜单和弹框。示例：box 覆盖 terminal。
pub(super) fn overlay_text(frame: &mut [RenderLine], y: u16, x: u16, text: &str) {
    if let Some(line) = frame.get_mut(usize::from(y)) {
        line.overlays.push((
            x,
            format!("\x1b[0;38;2;230;237;243;48;2;24;33;43m{text}\x1b[0m"),
        ));
    }
}

/// 添加指定前景、背景和粗体状态的 ANSI 覆盖文字。
///
/// 适用场景：tab、状态和层级菜单。示例：active tab -> 蓝底白字。
pub(super) fn styled_overlay_text(
    frame: &mut [RenderLine],
    y: u16,
    x: u16,
    text: &str,
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
    bold: bool,
) {
    if let Some(line) = frame.get_mut(usize::from(y)) {
        let weight = if bold { "1;" } else { "" };
        line.overlays.push((
            x,
            format!(
                "\x1b[0;{weight}38;2;{};{};{};48;2;{};{};{}m{text}\x1b[0m",
                fg.0, fg.1, fg.2, bg.0, bg.1, bg.2
            ),
        ));
    }
}

/// 绘制简单 modal 边框。
///
/// 适用场景：表单与确认。示例：title 放在顶边。
pub(super) fn draw_box(
    frame: &mut [RenderLine],
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    title: &str,
) {
    let title = clip_text(title, width.saturating_sub(4));
    let top_fill = width
        .saturating_sub(3)
        .saturating_sub(display_width(&title) as u16);
    overlay_text(
        frame,
        y,
        x,
        &format!("┌ {title}{}┐", "─".repeat(top_fill as usize)),
    );
    for row in 1..height.saturating_sub(1) {
        overlay_text(
            frame,
            y + row,
            x,
            &format!("│{}│", " ".repeat(width.saturating_sub(2) as usize)),
        );
    }
    overlay_text(
        frame,
        y + height.saturating_sub(1),
        x,
        &format!("└{}┘", "─".repeat(width.saturating_sub(2) as usize)),
    );
}

/// 按终端显示宽度裁剪文本。
///
/// 适用场景：所有固定布局标签。示例：CJK 宽字符不切半。
pub(super) fn clip_text(text: &str, width: u16) -> String {
    clip_text_from_width(text, usize::from(width))
}

/// 按 usize 显示宽度裁剪文本。
///
/// 适用场景：terminal run。示例：width=3 保留一个中文和一个 ASCII。
pub(super) fn clip_text_from_width(text: &str, width: usize) -> String {
    let mut used = 0;
    text.chars()
        .take_while(|ch| {
            let next = UnicodeWidthChar::width(*ch).unwrap_or(0);
            if used + next > width {
                false
            } else {
                used += next;
                true
            }
        })
        .collect()
}

/// 返回 pane header 下方的 terminal 正文矩形。
///
/// 适用场景：绘制、命中和文本选择共用。示例：pane y=1,h=10 -> body y=2,h=9。
pub(super) fn pane_body_rect(pane: &PaneLayout) -> TuiRect {
    TuiRect {
        x: pane.rect.x,
        y: pane.rect.y.saturating_add(1),
        width: pane.rect.width,
        height: pane.rect.height.saturating_sub(1),
    }
}

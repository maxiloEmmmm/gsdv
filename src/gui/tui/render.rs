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

/// 绘制 terminal snapshot 最后一个 viewport，保留 run 样式。
///
/// 适用场景：pane body。示例：500 行 snapshot + 20 行 pane -> 只画末 20 行。
pub(super) fn draw_terminal_snapshot(
    frame: &mut [RenderLine],
    rect: TuiRect,
    snapshot: &TerminalRemoteSnapshot,
) {
    let take = usize::from(rect.height);
    let start = snapshot.rows.len().saturating_sub(take);
    let pad = take.saturating_sub(snapshot.rows.len().saturating_sub(start));
    for offset in 0..take {
        let y = rect.y.saturating_add(offset as u16);
        if offset < pad {
            continue;
        }
        if let Some(row) = snapshot.rows.get(start + offset - pad) {
            let ansi = render_terminal_row(row, rect.width);
            if let Some(line) = frame.get_mut(usize::from(y)) {
                line.overlays.push((rect.x, ansi));
            }
        }
    }
}

/// 把 terminal 逻辑光标映射到当前 pane 的可见字符坐标。
///
/// 适用场景：聚焦 Agent 且没有菜单或弹窗。示例：最后一行第 3 列 -> pane 内光标。
pub(super) fn terminal_cursor_position(
    rect: TuiRect,
    snapshot: &TerminalRemoteSnapshot,
) -> Option<(u16, u16)> {
    let take = usize::from(rect.height);
    let start = snapshot.rows.len().saturating_sub(take);
    let visible = &snapshot.rows[start..];
    let pad = take.saturating_sub(visible.len());
    let row = visible
        .iter()
        .position(|row| row.line_index == snapshot.cursor.line_index)?;
    Some((
        rect.x.saturating_add(
            u16::try_from(snapshot.cursor.col_index)
                .unwrap_or(u16::MAX)
                .min(rect.width.saturating_sub(1)),
        ),
        rect.y
            .saturating_add(u16::try_from(pad + row).unwrap_or(u16::MAX)),
    ))
}

/// 把结构化 terminal row 编码成限定宽度的 true-color ANSI 文本。
///
/// 适用场景：SSH 输出。示例：bold RGB run -> SGR + clipped text。
pub(super) fn render_terminal_row(row: &TerminalRemoteRow, width: u16) -> String {
    let mut output = String::new();
    let mut used = 0_usize;
    let limit = usize::from(width);
    for run in &row.runs {
        if used >= limit {
            break;
        }
        let text = clip_text_from_width(&run.text, limit - used);
        let text_width = display_width(&text);
        if text_width == 0 && text.is_empty() {
            continue;
        }
        let (fg, bg) = if run.inverse {
            (&run.bg, &run.fg)
        } else {
            (&run.fg, &run.bg)
        };
        output.push_str("\x1b[0m");
        if let Some((r, g, b)) = parse_hex_color(fg) {
            output.push_str(&format!("\x1b[38;2;{r};{g};{b}m"));
        }
        if let Some((r, g, b)) = parse_hex_color(bg) {
            output.push_str(&format!("\x1b[48;2;{r};{g};{b}m"));
        }
        if run.bold {
            output.push_str("\x1b[1m");
        }
        if run.dim {
            output.push_str("\x1b[2m");
        }
        if run.italic {
            output.push_str("\x1b[3m");
        }
        if run.underline != "none" {
            output.push_str("\x1b[4m");
        }
        if run.strikeout {
            output.push_str("\x1b[9m");
        }
        if run.hidden {
            output.push_str(&" ".repeat(text_width));
        } else {
            output.push_str(&sanitize_terminal_text(&text));
        }
        used += text_width;
    }
    output.push_str("\x1b[0m");
    if used < limit {
        output.push_str(&" ".repeat(limit - used));
    }
    output
}

/// 移除 emulator cell 中不应再次解释的控制字符。
///
/// 适用场景：terminal row 输出到宿主终端。示例：ESC -> 空格。
pub(super) fn sanitize_terminal_text(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

/// 解析 `#RRGGBB`。
///
/// 适用场景：TerminalRemoteRun 颜色。示例：`#ff0000 -> (255,0,0)`。
pub(super) fn parse_hex_color(value: &str) -> Option<(u8, u8, u8)> {
    let value = value.strip_prefix('#')?;
    (value.len() == 6).then_some((
        u8::from_str_radix(&value[0..2], 16).ok()?,
        u8::from_str_radix(&value[2..4], 16).ok()?,
        u8::from_str_radix(&value[4..6], 16).ok()?,
    ))
}

/// 写出与上一帧不同的行，避免 SSH 反复传完整屏幕。
///
/// 适用场景：最多 fps 次每秒。示例：一行变化 -> 一次 MoveTo + Clear + Print。
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
        if previous.get(index) == Some(line) {
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

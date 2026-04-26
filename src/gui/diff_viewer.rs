use crate::gui::theme;
use eframe::egui::{
    self, Align2, Button, Color32, CornerRadius, Frame, Margin, RichText, ScrollArea, Sense,
    Stroke, Ui, Vec2,
};

/// 共享 diff 视图的显示模式。
///
/// 适用场景：reviewer 需要 diff/full 两种显示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiffViewerMode {
    /// 只显示变更块。
    Diff,
    /// 显示全文件上下文。
    Full,
}

/// 共享 diff 行类型。
///
/// 适用场景：渲染层只关心颜色和行文本，不关心数据来自 git 还是 hook。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiffViewerLineKind {
    /// 上下文行。
    Context,
    /// 新增行。
    Insert,
    /// 删除行。
    Delete,
    /// hunk 元数据行。
    Hunk,
    /// 普通元数据行。
    Metadata,
}

/// 共享 diff 行。
///
/// 适用场景：调用方提前准备复制文本，组件只负责选择和渲染。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiffViewerLine {
    /// 行类型。
    pub(crate) kind: DiffViewerLineKind,
    /// 已经格式化好的显示文本。
    pub(crate) text: String,
    /// 当前行是否选中。
    pub(crate) selected: bool,
    /// 按 c 时复制给 agent 的单行定位信息。
    pub(crate) copy_line: Option<String>,
    /// 按 d 时复制给 agent 的当前 diff 块信息。
    pub(crate) copy_diff: Option<String>,
}

/// 共享 diff 视图快照。
///
/// 适用场景：render path 只读该结构，不做 IO、不做 diff 计算。
pub(crate) struct DiffViewerSnapshot<'a> {
    /// 标题。
    pub(crate) title: &'a str,
    /// 行列表。
    pub(crate) lines: &'a [DiffViewerLine],
    /// 最长行字符数。
    pub(crate) content_chars: usize,
    /// 当前顶部滚动行。
    pub(crate) scroll_row: usize,
    /// 显示模式。
    pub(crate) mode: DiffViewerMode,
    /// 是否能跳到上一个变更块。
    pub(crate) can_jump_previous_block: bool,
    /// 是否能跳到下一个变更块。
    pub(crate) can_jump_next_block: bool,
    /// 当前变更块序号，一基索引。
    pub(crate) current_block: usize,
    /// 总变更块数。
    pub(crate) block_count: usize,
    /// 是否由该 diff 视图处理键盘交互。
    pub(crate) keyboard_active: bool,
    /// 是否允许该 diff 视图处理复制快捷键。
    pub(crate) copy_active: bool,
}

/// 共享 diff 视图动作。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DiffViewerAction {
    /// 无动作。
    None,
    /// 切换 diff/full 模式。
    ToggleMode,
    /// 跳到上一个变更块。
    PreviousBlock,
    /// 跳到下一个变更块。
    NextBlock,
    /// 复制当前选中行。
    CopyLine { row: usize },
    /// 复制当前选中行所属 diff 信息。
    CopyDiff { row: usize },
}

/// 共享 diff 视图渲染结果。
pub(crate) struct DiffViewerResult {
    /// 组件产生的动作。
    pub(crate) action: DiffViewerAction,
    /// 当前顶部滚动行。
    pub(crate) scroll_row: usize,
    /// 点击选中的行。
    pub(crate) selected_row: Option<usize>,
}

/// 绘制可复用 diff viewer。
pub(crate) fn diff_viewer_column(
    ui: &mut Ui,
    snapshot: DiffViewerSnapshot<'_>,
    scroll_to_row: Option<usize>,
    id_salt: impl std::hash::Hash,
) -> DiffViewerResult {
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace).max(14.0) + 4.0;
    let frame_height = ui.available_height();
    let mut action = DiffViewerAction::None;
    let mut scroll_row = snapshot.scroll_row;
    let mut selected_row = None;
    Frame::new()
        .fill(theme::bg())
        .stroke(Stroke::new(1.0, theme::border()))
        .corner_radius(CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(Margin::same(0))
        .show(ui, |ui| {
            ui.set_min_height((frame_height - 2.0).max(120.0));
            action = diff_viewer_header(ui, &snapshot);
            let scroll_height = (ui.available_height() - 2.0).max(100.0);
            let content_width = diff_viewer_content_width(ui, &snapshot);
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut scroll_area = ScrollArea::both()
                .id_salt(id_salt)
                .max_height(scroll_height)
                .auto_shrink([false, false]);
            if let Some(row) = scroll_to_row {
                scroll_area = scroll_area.vertical_scroll_offset(row as f32 * row_height);
            }
            let output =
                scroll_area.show_rows(ui, row_height, snapshot.lines.len(), |ui, row_range| {
                    for line_index in row_range {
                        let response = diff_viewer_row(
                            ui,
                            row_height,
                            content_width,
                            &snapshot.lines[line_index],
                        );
                        if response.clicked()
                            || response.double_clicked()
                            || response_contains_primary_press(ui, &response)
                        {
                            selected_row = Some(line_index);
                        }
                    }
                });
            scroll_row = (output.state.offset.y / row_height).floor().max(0.0) as usize;
        });
    DiffViewerResult {
        action,
        scroll_row,
        selected_row,
    }
}

/// 判断当前帧是否在指定行内按下了左键。
fn response_contains_primary_press(ui: &Ui, response: &egui::Response) -> bool {
    response.hovered()
        && ui.input(|input| {
            input.pointer.button_pressed(egui::PointerButton::Primary)
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|pos| response.rect.contains(pos))
        })
}

/// 读取 diff viewer 自己负责的键盘动作。
pub(crate) fn read_diff_viewer_keyboard_action(
    input: &egui::InputState,
    selected_row: Option<usize>,
    keyboard_active: bool,
    copy_active: bool,
) -> DiffViewerAction {
    if !keyboard_active {
        return DiffViewerAction::None;
    }
    if diff_viewer_plain_key(input, egui::Key::F) {
        return DiffViewerAction::ToggleMode;
    }
    if diff_viewer_plain_key(input, egui::Key::N) {
        if input.modifiers.shift {
            return DiffViewerAction::PreviousBlock;
        }
        return DiffViewerAction::NextBlock;
    }
    if !copy_active {
        return DiffViewerAction::None;
    }
    if diff_viewer_plain_key(input, egui::Key::C)
        && let Some(row) = selected_row
    {
        return DiffViewerAction::CopyLine { row };
    }
    if diff_viewer_plain_key(input, egui::Key::D)
        && let Some(row) = selected_row
    {
        return DiffViewerAction::CopyDiff { row };
    }
    DiffViewerAction::None
}

/// 判断没有平台修饰键的 diff viewer 快捷键。
fn diff_viewer_plain_key(input: &egui::InputState, key: egui::Key) -> bool {
    input.key_pressed(key)
        && !input.modifiers.command
        && !input.modifiers.mac_cmd
        && !input.modifiers.ctrl
        && !input.modifiers.alt
}

/// 绘制 diff viewer 头部。
fn diff_viewer_header(ui: &mut Ui, snapshot: &DiffViewerSnapshot<'_>) -> DiffViewerAction {
    let height = 30.0;
    let width = ui.available_width().max(120.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::ZERO, theme::surface_elevated());
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, theme::border()),
    );
    let is_full = snapshot.mode == DiffViewerMode::Full;
    let count_right = rect.right() - 10.0;
    let button_width = 62.0;
    let toggle_width = 58.0;
    let block_count_width = 48.0;
    let button_height = 22.0;
    let button_gap = 6.0;
    let toggle_rect = egui::Rect::from_min_size(
        egui::pos2(
            count_right - 76.0 - toggle_width,
            rect.center().y - button_height / 2.0,
        ),
        Vec2::new(toggle_width, button_height),
    );
    let next_rect = egui::Rect::from_min_size(
        egui::pos2(
            toggle_rect.left() - button_gap - button_width,
            rect.center().y - button_height / 2.0,
        ),
        Vec2::new(button_width, button_height),
    );
    let block_count_rect = next_rect.translate(Vec2::new(-(block_count_width + button_gap), 0.0));
    let prev_rect = block_count_rect.translate(Vec2::new(-(button_width + button_gap), 0.0));
    let title_right = if is_full {
        prev_rect.left() - 8.0
    } else {
        toggle_rect.left() - 8.0
    };
    let title_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 10.0, rect.top()),
        egui::pos2(title_right.max(rect.left() + 40.0), rect.bottom()),
    );
    ui.painter().with_clip_rect(title_rect).text(
        egui::pos2(title_rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        snapshot.title,
        egui::TextStyle::Small.resolve(ui.style()),
        theme::muted(),
    );

    let mut action = DiffViewerAction::None;
    let toggle_label = if is_full { "Diff" } else { "Full" };
    if diff_viewer_jump_button(
        ui,
        toggle_rect,
        toggle_label,
        true,
        "Toggle diff/full mode (F)",
    )
    .clicked()
    {
        action = DiffViewerAction::ToggleMode;
    }
    if is_full {
        if diff_viewer_jump_button(
            ui,
            prev_rect,
            "N Prev",
            snapshot.can_jump_previous_block,
            "Previous diff block (Shift+N)",
        )
        .clicked()
            && snapshot.can_jump_previous_block
        {
            action = DiffViewerAction::PreviousBlock;
        }
        diff_viewer_block_count(ui, block_count_rect, snapshot);
        if diff_viewer_jump_button(
            ui,
            next_rect,
            "n Next",
            snapshot.can_jump_next_block,
            "Next diff block (N)",
        )
        .clicked()
            && snapshot.can_jump_next_block
        {
            action = DiffViewerAction::NextBlock;
        }
    }

    ui.painter().text(
        egui::pos2(rect.right() - 10.0, rect.center().y),
        Align2::RIGHT_CENTER,
        format!("{} lines", snapshot.lines.len()),
        egui::TextStyle::Small.resolve(ui.style()),
        theme::muted(),
    );
    action
}

/// 绘制当前和总 diff 块数量。
fn diff_viewer_block_count(ui: &mut Ui, rect: egui::Rect, snapshot: &DiffViewerSnapshot<'_>) {
    let label = if snapshot.block_count == 0 {
        "-/-".to_string()
    } else {
        format!("{}/{}", snapshot.current_block, snapshot.block_count)
    };
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(theme::RADIUS_SM),
        theme::diff_metadata_bg(),
    );
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(theme::RADIUS_SM),
        Stroke::new(1.0, theme::border()),
        egui::StrokeKind::Outside,
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        egui::TextStyle::Small.resolve(ui.style()),
        theme::muted(),
    );
}

/// 绘制 diff 头部里的跳转按钮。
fn diff_viewer_jump_button(
    ui: &mut Ui,
    rect: egui::Rect,
    label: &str,
    enabled: bool,
    hover: &str,
) -> egui::Response {
    let text_color = if enabled {
        theme::primary()
    } else {
        theme::muted()
    };
    let fill = if enabled {
        theme::primary_soft()
    } else {
        theme::surface()
    };
    let stroke = if enabled {
        Stroke::new(1.0, theme::primary_border())
    } else {
        Stroke::new(1.0, theme::border())
    };
    ui.put(
        rect,
        Button::new(RichText::new(label).size(11.0).color(text_color))
            .small()
            .min_size(rect.size())
            .fill(fill)
            .stroke(stroke)
            .corner_radius(CornerRadius::same(theme::RADIUS_SM)),
    )
    .on_hover_text(hover)
}

/// 计算等宽 diff 文本需要的横向宽度。
fn diff_viewer_content_width(ui: &Ui, snapshot: &DiffViewerSnapshot<'_>) -> f32 {
    let char_width = ui
        .fonts(|fonts| {
            let font_id = egui::TextStyle::Monospace.resolve(ui.style());
            fonts
                .glyph_width(&font_id, 'W')
                .max(fonts.glyph_width(&font_id, ' '))
        })
        .max(8.0);
    let max_columns = snapshot
        .lines
        .iter()
        .map(|line| diff_text_columns(&line.text))
        .max()
        .unwrap_or(snapshot.content_chars);
    (max_columns as f32 * char_width + 28.0).max(ui.available_width())
}

/// 估算 diff 文本列宽，避免中文等宽度字符被按半宽裁剪。
fn diff_text_columns(text: &str) -> usize {
    text.chars()
        .map(|ch| if ch.is_ascii() { 1 } else { 2 })
        .sum()
}

/// 绘制一行 diff。
fn diff_viewer_row(
    ui: &mut Ui,
    row_height: f32,
    content_width: f32,
    line: &DiffViewerLine,
) -> egui::Response {
    let width = content_width.max(ui.available_width()).max(160.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, row_height), Sense::click());
    let pressed = response_contains_primary_press(ui, &response);
    let (fill, text_color) = match line.kind {
        DiffViewerLineKind::Context => (theme::bg(), theme::list_text()),
        DiffViewerLineKind::Insert => (theme::diff_insert_bg(), theme::success()),
        DiffViewerLineKind::Delete => (theme::diff_delete_bg(), theme::danger()),
        DiffViewerLineKind::Hunk => (theme::diff_hunk_bg(), theme::primary()),
        DiffViewerLineKind::Metadata => (theme::diff_metadata_bg(), theme::muted()),
    };
    ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
    if line.selected || pressed {
        ui.painter().rect_filled(
            rect.shrink2(Vec2::new(2.0, 1.0)),
            CornerRadius::same(theme::RADIUS_SM),
            theme::primary_soft(),
        );
        ui.painter().rect_stroke(
            rect.shrink2(Vec2::new(2.0, 1.0)),
            CornerRadius::same(theme::RADIUS_SM),
            Stroke::new(1.0, theme::primary_border()),
            egui::StrokeKind::Outside,
        );
    }
    let text_rect = rect.shrink2(Vec2::new(10.0, 0.0));
    ui.painter().with_clip_rect(text_rect).text(
        egui::pos2(text_rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        &line.text,
        egui::TextStyle::Monospace.resolve(ui.style()),
        text_color,
    );
    response
}

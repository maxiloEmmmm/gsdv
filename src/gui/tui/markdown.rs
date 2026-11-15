//! TUI Markdown 工作台。
//!
//! 该模块只持有终端编辑草稿；真正保存仍通过 `TuiAppAction -> AppEvent` 进入 app 保存链。

use super::*;
use pulldown_cmark::{Event, Options, Parser, Tag};

/// Markdown 工作台对应的业务目标。
///
/// 适用场景：Dir 文件、Workflow task/step 共用编辑器。示例：Step -> 片段保存链。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MarkdownWorkbenchTarget {
    /// 普通 Markdown 文件路径。
    Document(PathBuf),
    /// Workflow task 或 step 片段。
    Workflow(WorkflowSelectionTarget),
}

/// Markdown 工作台模式。
///
/// 适用场景：保存后保持 Edit，Preview 只能显式切换。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum MarkdownWorkbenchMode {
    /// 渲染 Markdown。
    #[default]
    Preview,
    /// 编辑原始 Markdown。
    Edit,
}

/// 单个 Markdown 工作台的草稿状态。
///
/// 适用场景：task 与 step 同时显示时各自保持独立草稿。
#[derive(Debug, Clone)]
pub(super) struct MarkdownWorkbench {
    /// 业务保存目标。
    pub target: MarkdownWorkbenchTarget,
    /// 当前编辑文本。
    pub text: String,
    /// 最近确认已保存文本。
    pub saved_text: String,
    /// Preview 或 Edit。
    pub mode: MarkdownWorkbenchMode,
    /// UTF-8 字节光标，始终位于字符边界。
    pub cursor: usize,
    /// 预览或编辑首个可见行。
    pub scroll: usize,
    /// 下一帧是否需要把编辑光标重新纳入可视区。
    pub follow_cursor: bool,
    /// 保存已发出、等待共享状态回显。
    pub saving: bool,
    /// 最近一次共享保存链返回的错误。
    pub save_error: Option<String>,
    /// 当前目标是否允许编辑；Remote Workflow 为 false。
    pub editable: bool,
    /// 当前 Markdown 文件绝对路径，用于解析相对图片。
    pub markdown_file: Option<PathBuf>,
    /// 已从当前文本解析出的本地图片绝对路径。
    pub image_paths: Vec<PathBuf>,
}

impl MarkdownWorkbench {
    /// 判断草稿是否有未保存修改。
    ///
    /// 适用场景：切换节点前提示。示例：text != saved -> true。
    pub(super) fn dirty(&self) -> bool {
        self.text != self.saved_text
    }
}

/// 全部 Markdown 工作台，key 由视图和 target 共同确定。
///
/// 适用场景：Workflow 上下两个编辑器与 Dir 编辑器并存。
#[derive(Debug, Default)]
pub(super) struct MarkdownWorkbenches {
    /// 稳定 key 到工作台状态。
    pub items: BTreeMap<String, MarkdownWorkbench>,
    /// 当前接收键盘输入的工作台。
    pub focused: Option<String>,
}

impl MarkdownWorkbenches {
    /// 同步一个工作台，脏草稿不会被后台刷新覆盖。
    ///
    /// 适用场景：Workflow tree 或 DocumentState 更新。示例：保存回显 -> 清 dirty。
    pub(super) fn sync(
        &mut self,
        key: String,
        target: MarkdownWorkbenchTarget,
        text: &str,
        saved_text: &str,
        editable: bool,
        markdown_file: Option<PathBuf>,
        save_error: Option<&str>,
    ) {
        let item = self.items.entry(key).or_insert_with(|| MarkdownWorkbench {
            target: target.clone(),
            text: text.to_string(),
            saved_text: saved_text.to_string(),
            mode: MarkdownWorkbenchMode::Preview,
            cursor: text.len(),
            scroll: 0,
            follow_cursor: true,
            saving: false,
            save_error: save_error.map(str::to_string),
            editable,
            markdown_file: markdown_file.clone(),
            image_paths: collect_markdown_image_paths(text, markdown_file.as_deref()),
        });
        item.editable = editable;
        item.save_error = save_error.map(str::to_string);
        if item.markdown_file != markdown_file {
            item.markdown_file = markdown_file.clone();
            item.image_paths = collect_markdown_image_paths(&item.text, markdown_file.as_deref());
        }
        if item.target != target {
            if item.dirty() {
                return;
            }
            item.target = target;
            item.text = text.to_string();
            item.saved_text = saved_text.to_string();
            item.cursor = item.text.len();
            item.scroll = 0;
            item.follow_cursor = true;
            item.saving = false;
            item.save_error = save_error.map(str::to_string);
            item.image_paths =
                collect_markdown_image_paths(&item.text, item.markdown_file.as_deref());
            return;
        }
        if item.saving && save_error.is_some() {
            item.saving = false;
            return;
        }
        if !item.dirty() || (item.saving && item.text == saved_text) {
            item.text = text.to_string();
            item.saved_text = saved_text.to_string();
            item.cursor = item.cursor.min(item.text.len());
            item.saving = false;
            item.save_error = save_error.map(str::to_string);
            item.image_paths =
                collect_markdown_image_paths(&item.text, item.markdown_file.as_deref());
        }
    }

    /// 判断当前键盘目标是否处于编辑态。
    ///
    /// 适用场景：路由字符输入。示例：Preview -> false。
    pub(super) fn editing(&self) -> bool {
        self.focused
            .as_ref()
            .and_then(|key| self.items.get(key))
            .is_some_and(|item| item.mode == MarkdownWorkbenchMode::Edit)
    }

    /// 编辑当前工作台的文本与光标。
    ///
    /// 适用场景：无 modal 时的键盘输入。示例：Enter -> 插入换行。
    pub(super) fn handle_key(&mut self, key: KeyEvent) -> bool {
        let Some(item) = self
            .focused
            .as_ref()
            .and_then(|focused| self.items.get_mut(focused))
        else {
            return false;
        };
        if item.mode != MarkdownWorkbenchMode::Edit || !item.editable {
            return false;
        }
        match key.code {
            KeyCode::Char(character)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                item.text.insert(item.cursor, character);
                item.cursor += character.len_utf8();
            }
            KeyCode::Enter => {
                item.text.insert(item.cursor, '\n');
                item.cursor += 1;
            }
            KeyCode::Backspace => {
                if item.cursor > 0 {
                    let previous = item.text[..item.cursor]
                        .char_indices()
                        .next_back()
                        .map(|(index, _)| index)
                        .unwrap_or(0);
                    item.text.replace_range(previous..item.cursor, "");
                    item.cursor = previous;
                }
            }
            KeyCode::Delete => {
                if item.cursor < item.text.len() {
                    let next = item.text[item.cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(index, _)| item.cursor + index)
                        .unwrap_or(item.text.len());
                    item.text.replace_range(item.cursor..next, "");
                }
            }
            KeyCode::Left => {
                item.cursor = item.text[..item.cursor]
                    .char_indices()
                    .next_back()
                    .map(|(index, _)| index)
                    .unwrap_or(0);
            }
            KeyCode::Right => {
                item.cursor = item.text[item.cursor..]
                    .char_indices()
                    .nth(1)
                    .map(|(index, _)| item.cursor + index)
                    .unwrap_or(item.text.len());
            }
            KeyCode::Home => {
                item.cursor = item.text[..item.cursor]
                    .rfind('\n')
                    .map_or(0, |index| index + 1);
            }
            KeyCode::End => {
                item.cursor = item.text[item.cursor..]
                    .find('\n')
                    .map_or(item.text.len(), |index| item.cursor + index);
            }
            KeyCode::Esc => return true,
            _ => return false,
        }
        item.image_paths = collect_markdown_image_paths(&item.text, item.markdown_file.as_deref());
        item.follow_cursor = true;
        true
    }

    /// 粘贴到当前编辑工作台。
    ///
    /// 适用场景：bracketed paste。示例：多行文本保持换行。
    pub(super) fn paste(&mut self, text: &str) -> bool {
        let Some(item) = self
            .focused
            .as_ref()
            .and_then(|focused| self.items.get_mut(focused))
        else {
            return false;
        };
        if item.mode != MarkdownWorkbenchMode::Edit || !item.editable {
            return false;
        }
        item.text.insert_str(item.cursor, text);
        item.cursor += text.len();
        item.image_paths = collect_markdown_image_paths(&item.text, item.markdown_file.as_deref());
        item.follow_cursor = true;
        true
    }

    /// 把鼠标所在终端单元格换算为 UTF-8 文本光标。
    ///
    /// 适用场景：编辑态点击正文。示例：第 2 个可见字符 -> 对应字节边界。
    pub(super) fn place_cursor(&mut self, key: &str, column: u16, row: u16) -> bool {
        let Some(item) = self.items.get_mut(key) else {
            return false;
        };
        if item.mode != MarkdownWorkbenchMode::Edit || !item.editable {
            return false;
        }
        let target_line = item.scroll.saturating_add(usize::from(row));
        let mut line_start = 0_usize;
        let mut selected = None;
        for (line_index, line) in item.text.split('\n').enumerate() {
            if line_index == target_line {
                selected = Some(line_start + byte_index_at_display_column(line, column));
                break;
            }
            line_start = line_start.saturating_add(line.len()).saturating_add(1);
        }
        item.cursor = selected.unwrap_or(item.text.len());
        item.follow_cursor = true;
        self.focused = Some(key.to_string());
        true
    }

    /// 滚动鼠标所在的 Markdown 工作台。
    ///
    /// 适用场景：Preview 与 Edit 共用滚轮。示例：向下 3 行 -> scroll + 3。
    pub(super) fn scroll(&mut self, key: &str, delta: i32) -> bool {
        let Some(item) = self.items.get_mut(key) else {
            return false;
        };
        item.scroll = if delta.is_negative() {
            item.scroll.saturating_sub(delta.unsigned_abs() as usize)
        } else {
            item.scroll.saturating_add(delta as usize)
        };
        // 鼠标滚动用于浏览光标之外的内容，下一帧不能立刻弹回光标。
        item.follow_cursor = false;
        true
    }
}

/// 返回显示列对应的 UTF-8 字节边界。
///
/// 适用场景：终端宽字符不能按字节定位。示例：点击中文字符单元格 -> 字符起点。
fn byte_index_at_display_column(line: &str, target_column: u16) -> usize {
    let target_column = usize::from(target_column);
    let mut column = 0_usize;
    for (byte_index, character) in line.char_indices() {
        let width = UnicodeWidthChar::width(character).unwrap_or(0);
        if target_column < column.saturating_add(width.max(1)) {
            return byte_index;
        }
        column = column.saturating_add(width);
    }
    line.len()
}

/// 绘制一个带工具栏的 Markdown 工作台。
///
/// 适用场景：Dir 右侧、Workflow task/step 工作台。示例：Edit 时绘制原文和光标。
pub(super) fn draw_markdown_workbench(
    workbenches: &mut MarkdownWorkbenches,
    key: &str,
    frame: &mut [RenderLine],
    rect: TuiRect,
    hit_regions: &mut Vec<HitRegion>,
    images: &local_image::LocalImageStore,
) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let Some(item) = workbenches.items.get_mut(key) else {
        return;
    };
    let toolbar = [
        (
            "Preview",
            HitAction::MarkdownPreview(key.to_string()),
            item.mode == MarkdownWorkbenchMode::Preview,
        ),
        (
            "Edit",
            HitAction::MarkdownEdit(key.to_string()),
            item.mode == MarkdownWorkbenchMode::Edit,
        ),
        ("Save", HitAction::MarkdownSave(key.to_string()), false),
        (
            "Discard",
            HitAction::MarkdownDiscard(key.to_string()),
            false,
        ),
        (
            "Exit Markdown",
            HitAction::MarkdownExit(key.to_string()),
            false,
        ),
    ];
    let mut x = rect.x;
    for (label, action, active) in toolbar {
        let text = format!("[{label}]");
        let width = display_width(&text) as u16;
        styled_overlay_text(
            frame,
            rect.y,
            x,
            &text,
            if active {
                (255, 255, 255)
            } else {
                (148, 163, 184)
            },
            if active { (37, 99, 235) } else { (15, 23, 42) },
            active,
        );
        hit_regions.push(HitRegion {
            rect: TuiRect {
                x,
                y: rect.y,
                width,
                height: 1,
            },
            left: if label == "Edit" && !item.editable {
                None
            } else {
                Some(action)
            },
            right: None,
        });
        x = x.saturating_add(width).saturating_add(1);
    }
    if item.dirty() && rect.width > 2 {
        styled_overlay_text(
            frame,
            rect.y,
            rect.x + rect.width - 2,
            "*",
            (250, 204, 21),
            (11, 16, 22),
            true,
        );
    }
    if let Some(error) = item.save_error.as_deref() {
        let status = clip_text(error, rect.width.saturating_sub(2));
        let status_width = display_width(&status) as u16;
        styled_overlay_text(
            frame,
            rect.y,
            rect.x + rect.width.saturating_sub(status_width),
            &status,
            (254, 202, 202),
            (127, 29, 29),
            true,
        );
    }
    let content = TuiRect {
        x: rect.x,
        y: rect.y.saturating_add(1),
        width: rect.width,
        height: rect.height.saturating_sub(1),
    };
    match item.mode {
        MarkdownWorkbenchMode::Preview => {
            draw_markdown_preview(item, frame, content, images);
        }
        MarkdownWorkbenchMode::Edit => {
            draw_markdown_editor(item, frame, content);
        }
    }
    // Preview 也需要同一命中区承接滚轮；点击定位仅在 Edit 内生效。
    hit_regions.push(HitRegion {
        rect: content,
        left: Some(HitAction::MarkdownPlaceCursor {
            key: key.to_string(),
            origin_x: content.x,
            origin_y: content.y,
        }),
        right: None,
    });
}

/// 绘制带本地图片的 Markdown 预览。
///
/// 适用场景：TUI 没有 Kitty/Sixel 时使用 TrueColor 半块字符。示例：PNG -> ANSI 预览。
fn draw_markdown_preview(
    item: &mut MarkdownWorkbench,
    frame: &mut [RenderLine],
    rect: TuiRect,
    images: &local_image::LocalImageStore,
) {
    let mut y = rect.y;
    let mut remaining = rect.height;
    for path in &item.image_paths {
        let Some(image) = images.ready(path) else {
            continue;
        };
        if remaining < 4 {
            break;
        }
        let image_height = remaining.min(10);
        let used = draw_decoded_image(
            frame,
            TuiRect {
                x: rect.x,
                y,
                width: rect.width,
                height: image_height,
            },
            &image.decoded,
        );
        y = y.saturating_add(used).saturating_add(1);
        remaining = rect.y.saturating_add(rect.height).saturating_sub(y);
    }
    workflow::draw_markdown_region(
        frame,
        &item.text,
        TuiRect {
            x: rect.x,
            y,
            width: rect.width,
            height: remaining,
        },
        &mut item.scroll,
    );
}

/// 用 ANSI TrueColor 半块字符绘制一张已解码图片。
///
/// 适用场景：TUI Markdown 和 socket gallery。示例：上下两个像素 -> 一个 `▀` 字符。
pub(super) fn draw_decoded_image(
    frame: &mut [RenderLine],
    rect: TuiRect,
    image: &local_image::DecodedLocalImage,
) -> u16 {
    if rect.width == 0 || rect.height == 0 || image.width == 0 || image.height == 0 {
        return 0;
    }
    let target_width = rect.width.min(image.width as u16).max(1);
    let proportional_rows =
        ((f64::from(target_width) * image.height as f64 / image.width as f64 / 2.0).round() as u16)
            .max(1);
    let target_rows = proportional_rows.min(rect.height).max(1);
    for row in 0..target_rows {
        let mut ansi = String::new();
        for column in 0..target_width {
            let source_x = usize::from(column) * image.width / usize::from(target_width);
            let top_y = usize::from(row) * 2 * image.height / (usize::from(target_rows) * 2);
            let bottom_y = ((usize::from(row) * 2 + 1) * image.height
                / (usize::from(target_rows) * 2))
                .min(image.height - 1);
            let top = rgba_pixel(image, source_x, top_y);
            let bottom = rgba_pixel(image, source_x, bottom_y);
            ansi.push_str(&format!(
                "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m▀",
                top.0, top.1, top.2, bottom.0, bottom.1, bottom.2
            ));
        }
        ansi.push_str("\x1b[0m");
        if let Some(line) = frame.get_mut(usize::from(rect.y.saturating_add(row))) {
            line.overlays.push((rect.x, ansi));
        }
    }
    target_rows
}

/// 从 RGBA8 图片读取一个像素，并把透明像素合成到深色背景。
///
/// 适用场景：ANSI 不支持 alpha。示例：透明像素 -> TUI 背景色。
fn rgba_pixel(image: &local_image::DecodedLocalImage, x: usize, y: usize) -> (u8, u8, u8) {
    let offset = (y * image.width + x) * 4;
    let pixel = image
        .rgba
        .get(offset..offset + 4)
        .unwrap_or(&[11, 16, 22, 255]);
    let alpha = u16::from(pixel[3]);
    let blend = |foreground: u8, background: u8| {
        ((u16::from(foreground) * alpha + u16::from(background) * (255 - alpha)) / 255) as u8
    };
    (
        blend(pixel[0], 11),
        blend(pixel[1], 16),
        blend(pixel[2], 22),
    )
}

/// 解析 Markdown 图片地址并转换成本地绝对路径。
///
/// 适用场景：文本或基准文件变化时更新，不在 draw 中解析。示例：`![x](a.png)` -> 绝对路径。
fn collect_markdown_image_paths(markdown: &str, markdown_file: Option<&Path>) -> Vec<PathBuf> {
    let Some(markdown_file) = markdown_file else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    for event in Parser::new_ext(markdown, Options::ENABLE_GFM) {
        if let Event::Start(Tag::Image { dest_url, .. }) = event
            && let Some(path) = local_image::resolve_markdown_image(markdown_file, &dest_url)
            && !paths.contains(&path)
        {
            paths.push(path);
        }
    }
    paths
}

/// 绘制纯文本 Markdown 编辑器。
///
/// 适用场景：终端输入只修改本地草稿。示例：光标所在字符使用反色背景。
fn draw_markdown_editor(item: &mut MarkdownWorkbench, frame: &mut [RenderLine], rect: TuiRect) {
    let lines = item.text.split('\n').collect::<Vec<_>>();
    let cursor_line = item.text[..item.cursor]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count();
    item.scroll = item
        .scroll
        .min(lines.len().saturating_sub(rect.height as usize));
    if item.follow_cursor {
        if cursor_line < item.scroll {
            item.scroll = cursor_line;
        } else if cursor_line >= item.scroll + rect.height as usize {
            item.scroll = cursor_line.saturating_sub(rect.height as usize - 1);
        }
        item.follow_cursor = false;
    }
    for (screen, line) in lines
        .iter()
        .skip(item.scroll)
        .take(rect.height as usize)
        .enumerate()
    {
        styled_overlay_text(
            frame,
            rect.y + screen as u16,
            rect.x,
            &clip_text(line, rect.width),
            (226, 232, 240),
            (11, 16, 22),
            false,
        );
    }
    let line_start = item.text[..item.cursor]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let cursor_column = display_width(&item.text[line_start..item.cursor]) as u16;
    let cursor_y = rect.y + cursor_line.saturating_sub(item.scroll) as u16;
    if cursor_y < rect.y.saturating_add(rect.height) && cursor_column < rect.width {
        styled_overlay_text(
            frame,
            cursor_y,
            rect.x + cursor_column,
            " ",
            (11, 16, 22),
            (226, 232, 240),
            true,
        );
    }
}

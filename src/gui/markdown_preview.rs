use crate::gui::theme;
use eframe::egui::{
    self, Align, Color32, FontId, Frame, Label, Margin, Pos2, Rect, RichText, Stroke, TextFormat,
    Ui, Vec2, text::LayoutJob,
};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct InlineStyle {
    strong: bool,
    emphasis: bool,
    strikethrough: bool,
    code: bool,
    link: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InlineSpan {
    text: String,
    style: InlineStyle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MarkdownList {
    start: Option<u64>,
    items: Vec<MarkdownListItem>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MarkdownListItem {
    checked: Option<bool>,
    blocks: Vec<MarkdownBlock>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TableAlignment {
    #[default]
    None,
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MarkdownTable {
    alignments: Vec<TableAlignment>,
    headers: Vec<Vec<InlineSpan>>,
    rows: Vec<Vec<Vec<InlineSpan>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MarkdownBlock {
    Paragraph(Vec<InlineSpan>),
    Heading {
        level: u8,
        spans: Vec<InlineSpan>,
    },
    List(MarkdownList),
    CodeBlock {
        language: Option<String>,
        text: String,
    },
    Table(MarkdownTable),
    Quote(Vec<MarkdownBlock>),
    FootnoteDefinition {
        label: String,
        blocks: Vec<MarkdownBlock>,
    },
    Rule,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MarkdownRenderMetrics {
    pub(crate) heading_offsets: Vec<f32>,
    pub(crate) block_tops: Vec<f32>,
    pub(crate) block_bottoms: Vec<f32>,
    pub(crate) content_height: f32,
}

enum BlockContext {
    Document(Vec<MarkdownBlock>),
    Paragraph(Vec<InlineSpan>),
    Heading {
        level: u8,
        spans: Vec<InlineSpan>,
    },
    List(MarkdownList),
    Item(MarkdownListItem),
    Quote(Vec<MarkdownBlock>),
    CodeBlock {
        language: Option<String>,
        text: String,
    },
    FootnoteDefinition {
        label: String,
        blocks: Vec<MarkdownBlock>,
    },
    Table {
        alignments: Vec<TableAlignment>,
        headers: Vec<Vec<InlineSpan>>,
        rows: Vec<Vec<Vec<InlineSpan>>>,
        in_head: bool,
    },
    TableRow(Vec<Vec<InlineSpan>>),
    TableCell(Vec<InlineSpan>),
}

pub(crate) fn render(ui: &mut Ui, markdown: &str, width: f32, mode: theme::ThemeMode) {
    let _ = render_with_heading_offsets(ui, markdown, width, mode);
}

/// Parses Markdown into reusable blocks for render-only preview frames.
pub(crate) fn parse(markdown: &str) -> Vec<MarkdownBlock> {
    parse_markdown(markdown)
}

/// Renders Markdown and returns heading top offsets in rendered content space.
pub(crate) fn render_with_heading_offsets(
    ui: &mut Ui,
    markdown: &str,
    width: f32,
    mode: theme::ThemeMode,
) -> Vec<f32> {
    let width = render_width(width);
    let blocks = parse_markdown(markdown);
    let mut heading_offsets = Vec::new();

    ui.set_width(width);
    ui.vertical(|ui| {
        ui.set_width(width);
        let origin_y = ui.cursor().top();
        render_blocks(
            ui,
            &blocks,
            width,
            mode,
            0,
            &mut heading_offsets,
            origin_y,
            None,
        );
    });
    heading_offsets
}

/// Renders pre-parsed Markdown blocks and returns heading top offsets.
pub(crate) fn render_blocks_with_heading_offsets(
    ui: &mut Ui,
    blocks: &[MarkdownBlock],
    width: f32,
    mode: theme::ThemeMode,
) -> Vec<f32> {
    render_blocks_with_metrics(ui, blocks, width, mode).heading_offsets
}

/// Renders pre-parsed Markdown blocks and records block positions.
pub(crate) fn render_blocks_with_metrics(
    ui: &mut Ui,
    blocks: &[MarkdownBlock],
    width: f32,
    mode: theme::ThemeMode,
) -> MarkdownRenderMetrics {
    let width = render_width(width);
    let mut metrics = MarkdownRenderMetrics::default();

    ui.set_width(width);
    ui.vertical(|ui| {
        ui.set_width(width);
        let origin_y = ui.cursor().top();
        let mut heading_offsets = Vec::new();
        render_blocks(
            ui,
            blocks,
            width,
            mode,
            0,
            &mut heading_offsets,
            origin_y,
            Some(&mut metrics),
        );
        metrics.heading_offsets = heading_offsets;
        metrics.content_height = (ui.cursor().top() - origin_y).max(0.0);
    });
    metrics
}

/// Renders only the Markdown blocks intersecting the current viewport.
pub(crate) fn render_blocks_virtualized(
    ui: &mut Ui,
    blocks: &[MarkdownBlock],
    width: f32,
    mode: theme::ThemeMode,
    metrics: &MarkdownRenderMetrics,
    viewport_top: f32,
    viewport_height: f32,
) {
    let width = render_width(width);
    let viewport_bottom = viewport_top + viewport_height;
    let overscan = viewport_height.max(240.0) * 0.5;
    let visible_top = (viewport_top - overscan).max(0.0);
    let visible_bottom = viewport_bottom + overscan;
    let mut first = None;
    let mut last = None;
    for (index, (top, bottom)) in metrics
        .block_tops
        .iter()
        .zip(metrics.block_bottoms.iter())
        .enumerate()
    {
        if *bottom >= visible_top && *top <= visible_bottom {
            first.get_or_insert(index);
            last = Some(index);
        }
    }
    let Some(first) = first else {
        ui.add_space(metrics.content_height.max(0.0));
        return;
    };
    let last = last.unwrap_or(first);

    ui.set_width(width);
    ui.vertical(|ui| {
        ui.set_width(width);
        let origin_y = ui.cursor().top();
        ui.add_space(metrics.block_tops.get(first).copied().unwrap_or_default());
        let mut ignored_heading_offsets = Vec::new();
        render_blocks_range(
            ui,
            blocks,
            first..=last,
            width,
            mode,
            0,
            &mut ignored_heading_offsets,
            origin_y,
            None,
        );
        let consumed = (ui.cursor().top() - origin_y).max(0.0);
        ui.add_space((metrics.content_height - consumed).max(0.0));
    });
}

fn render_width(width: f32) -> f32 {
    width.round().max(80.0)
}

fn parse_markdown(markdown: &str) -> Vec<MarkdownBlock> {
    let (front_matter, markdown) = split_yaml_front_matter(markdown).unwrap_or((None, markdown));
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_SMART_PUNCTUATION);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    options.insert(Options::ENABLE_GFM);

    let mut stack = vec![BlockContext::Document(Vec::new())];
    let mut inline_style = InlineStyle::default();
    let mut inline_style_stack = Vec::new();

    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(tag) => {
                start_tag(tag, &mut stack, &mut inline_style, &mut inline_style_stack)
            }
            Event::End(tag) => end_tag(tag, &mut stack, &mut inline_style, &mut inline_style_stack),
            Event::Text(text) | Event::InlineHtml(text) | Event::Html(text) => {
                append_text(&mut stack, text.as_ref(), &inline_style);
            }
            Event::Code(text) => {
                let mut style = inline_style.clone();
                style.code = true;
                append_text(&mut stack, text.as_ref(), &style);
            }
            Event::InlineMath(text) => append_text(&mut stack, text.as_ref(), &inline_style),
            Event::DisplayMath(text) => append_block(
                &mut stack,
                MarkdownBlock::CodeBlock {
                    language: Some("math".to_string()),
                    text: text.to_string(),
                },
            ),
            Event::SoftBreak => append_text(&mut stack, "\n", &inline_style),
            Event::HardBreak => append_text(&mut stack, "\n", &inline_style),
            Event::Rule => append_block(&mut stack, MarkdownBlock::Rule),
            Event::FootnoteReference(note) => {
                append_text(&mut stack, &format!("[^{note}]"), &inline_style)
            }
            Event::TaskListMarker(checked) => set_current_task_marker(&mut stack, checked),
        }
    }

    let parsed_blocks = match stack.pop() {
        Some(BlockContext::Document(blocks)) => blocks,
        _ => Vec::new(),
    };

    if let Some(front_matter) = front_matter {
        let mut blocks = vec![MarkdownBlock::CodeBlock {
            language: Some("yaml".to_string()),
            text: front_matter,
        }];
        blocks.extend(parsed_blocks);
        blocks
    } else {
        parsed_blocks
    }
}

fn split_yaml_front_matter(markdown: &str) -> Option<(Option<String>, &str)> {
    let first_line_end = markdown.find('\n')?;
    let first_line = markdown[..first_line_end].trim_end_matches('\r');
    if first_line != "---" {
        return None;
    }

    let mut offset = first_line_end + 1;
    while offset < markdown.len() {
        let line_start = offset;
        let line_end = markdown[offset..]
            .find('\n')
            .map(|index| offset + index + 1)
            .unwrap_or(markdown.len());
        let line = markdown[line_start..line_end].trim_end_matches(['\r', '\n']);
        if line == "---" {
            return Some((
                Some(markdown[..line_end].to_string()),
                &markdown[line_end..],
            ));
        }
        offset = line_end;
    }

    None
}

fn start_tag(
    tag: Tag<'_>,
    stack: &mut Vec<BlockContext>,
    inline_style: &mut InlineStyle,
    inline_style_stack: &mut Vec<InlineStyle>,
) {
    match tag {
        Tag::Paragraph => stack.push(BlockContext::Paragraph(Vec::new())),
        Tag::Heading { level, .. } => stack.push(BlockContext::Heading {
            level: heading_level(level),
            spans: Vec::new(),
        }),
        Tag::List(start) => stack.push(BlockContext::List(MarkdownList {
            start,
            items: Vec::new(),
        })),
        Tag::Item => stack.push(BlockContext::Item(MarkdownListItem::default())),
        Tag::BlockQuote(_) => stack.push(BlockContext::Quote(Vec::new())),
        Tag::CodeBlock(kind) => stack.push(BlockContext::CodeBlock {
            language: code_block_language(kind),
            text: String::new(),
        }),
        Tag::FootnoteDefinition(label) => stack.push(BlockContext::FootnoteDefinition {
            label: label.to_string(),
            blocks: Vec::new(),
        }),
        Tag::Table(alignments) => stack.push(BlockContext::Table {
            alignments: alignments
                .into_iter()
                .map(table_alignment_from_pulldown)
                .collect(),
            headers: Vec::new(),
            rows: Vec::new(),
            in_head: false,
        }),
        Tag::TableHead => {
            if let Some(BlockContext::Table { in_head, .. }) = stack.last_mut() {
                *in_head = true;
            }
        }
        Tag::TableRow => stack.push(BlockContext::TableRow(Vec::new())),
        Tag::TableCell => stack.push(BlockContext::TableCell(Vec::new())),
        Tag::Emphasis => push_inline_style(inline_style, inline_style_stack, |style| {
            style.emphasis = true;
        }),
        Tag::Strong => push_inline_style(inline_style, inline_style_stack, |style| {
            style.strong = true;
        }),
        Tag::Strikethrough => push_inline_style(inline_style, inline_style_stack, |style| {
            style.strikethrough = true;
        }),
        Tag::Link { dest_url, .. } => {
            let url = dest_url.to_string();
            push_inline_style(inline_style, inline_style_stack, move |style| {
                style.link = Some(url);
            });
        }
        Tag::Image { dest_url, .. } => {
            let url = dest_url.to_string();
            push_inline_style(inline_style, inline_style_stack, move |style| {
                style.link = Some(url);
            });
        }
        Tag::HtmlBlock
        | Tag::DefinitionList
        | Tag::DefinitionListTitle
        | Tag::DefinitionListDefinition
        | Tag::MetadataBlock(_) => {}
    }
}

fn end_tag(
    tag: TagEnd,
    stack: &mut Vec<BlockContext>,
    inline_style: &mut InlineStyle,
    inline_style_stack: &mut Vec<InlineStyle>,
) {
    match tag {
        TagEnd::Paragraph => {
            if let Some(BlockContext::Paragraph(spans)) = stack.pop()
                && !spans.is_empty()
            {
                append_block(stack, MarkdownBlock::Paragraph(spans));
            }
        }
        TagEnd::Heading(_) => {
            if let Some(BlockContext::Heading { level, spans }) = stack.pop()
                && !spans.is_empty()
            {
                append_block(stack, MarkdownBlock::Heading { level, spans });
            }
        }
        TagEnd::List(_) => {
            if let Some(BlockContext::List(list)) = stack.pop() {
                append_block(stack, MarkdownBlock::List(list));
            }
        }
        TagEnd::Item => {
            if let Some(BlockContext::Item(item)) = stack.pop()
                && let Some(BlockContext::List(list)) = stack.last_mut()
            {
                list.items.push(item);
            }
        }
        TagEnd::BlockQuote(_) => {
            if let Some(BlockContext::Quote(blocks)) = stack.pop() {
                append_block(stack, MarkdownBlock::Quote(blocks));
            }
        }
        TagEnd::CodeBlock => {
            if let Some(BlockContext::CodeBlock { language, text }) = stack.pop() {
                append_block(stack, MarkdownBlock::CodeBlock { language, text });
            }
        }
        TagEnd::FootnoteDefinition => {
            if let Some(BlockContext::FootnoteDefinition { label, blocks }) = stack.pop() {
                append_block(stack, MarkdownBlock::FootnoteDefinition { label, blocks });
            }
        }
        TagEnd::Table => {
            if let Some(BlockContext::Table {
                alignments,
                headers,
                rows,
                ..
            }) = stack.pop()
            {
                append_block(
                    stack,
                    MarkdownBlock::Table(MarkdownTable {
                        alignments,
                        headers,
                        rows,
                    }),
                );
            }
        }
        TagEnd::TableHead => {
            if let Some(BlockContext::Table { in_head, .. }) = stack.last_mut() {
                *in_head = false;
            }
        }
        TagEnd::TableRow => {
            if let Some(BlockContext::TableRow(cells)) = stack.pop()
                && let Some(BlockContext::Table {
                    headers,
                    rows,
                    in_head,
                    ..
                }) = stack.last_mut()
            {
                if *in_head {
                    *headers = cells;
                } else {
                    rows.push(cells);
                }
            }
        }
        TagEnd::TableCell => {
            if let Some(BlockContext::TableCell(spans)) = stack.pop() {
                match stack.last_mut() {
                    Some(BlockContext::TableRow(cells)) => cells.push(spans),
                    Some(BlockContext::Table {
                        headers, in_head, ..
                    }) if *in_head => headers.push(spans),
                    _ => {}
                }
            }
        }
        TagEnd::Emphasis
        | TagEnd::Strong
        | TagEnd::Strikethrough
        | TagEnd::Link
        | TagEnd::Image => {
            if let Some(previous) = inline_style_stack.pop() {
                *inline_style = previous;
            }
        }
        TagEnd::HtmlBlock
        | TagEnd::DefinitionList
        | TagEnd::DefinitionListTitle
        | TagEnd::DefinitionListDefinition
        | TagEnd::MetadataBlock(_) => {}
    }
}

fn push_inline_style(
    inline_style: &mut InlineStyle,
    inline_style_stack: &mut Vec<InlineStyle>,
    mutate: impl FnOnce(&mut InlineStyle),
) {
    inline_style_stack.push(inline_style.clone());
    mutate(inline_style);
}

fn append_text(stack: &mut Vec<BlockContext>, text: &str, style: &InlineStyle) {
    if text.is_empty() {
        return;
    }

    if let Some(BlockContext::CodeBlock { text: content, .. }) = stack.last_mut() {
        content.push_str(text);
        return;
    }

    let span = InlineSpan {
        text: text.to_string(),
        style: style.clone(),
    };

    match stack.last_mut() {
        Some(BlockContext::Paragraph(spans)) | Some(BlockContext::Heading { spans, .. }) => {
            spans.push(span);
        }
        Some(BlockContext::TableCell(spans)) => spans.push(span),
        Some(BlockContext::Document(blocks))
        | Some(BlockContext::Item(MarkdownListItem { blocks, .. }))
        | Some(BlockContext::Quote(blocks))
        | Some(BlockContext::FootnoteDefinition { blocks, .. }) => {
            append_span_to_blocks(blocks, span)
        }
        _ => {}
    }
}

fn append_block(stack: &mut [BlockContext], block: MarkdownBlock) {
    match stack.last_mut() {
        Some(BlockContext::Document(blocks))
        | Some(BlockContext::Item(MarkdownListItem { blocks, .. }))
        | Some(BlockContext::Quote(blocks))
        | Some(BlockContext::FootnoteDefinition { blocks, .. }) => blocks.push(block),
        _ => {}
    }
}

fn append_span_to_blocks(blocks: &mut Vec<MarkdownBlock>, span: InlineSpan) {
    if let Some(MarkdownBlock::Paragraph(spans)) = blocks.last_mut() {
        spans.push(span);
    } else {
        blocks.push(MarkdownBlock::Paragraph(vec![span]));
    }
}

fn set_current_task_marker(stack: &mut [BlockContext], checked: bool) {
    if let Some(BlockContext::Item(item)) = stack
        .iter_mut()
        .rev()
        .find(|context| matches!(context, BlockContext::Item(_)))
    {
        item.checked = Some(checked);
    }
}

fn render_blocks(
    ui: &mut Ui,
    blocks: &[MarkdownBlock],
    width: f32,
    mode: theme::ThemeMode,
    depth: usize,
    heading_offsets: &mut Vec<f32>,
    origin_y: f32,
    mut metrics: Option<&mut MarkdownRenderMetrics>,
) {
    render_blocks_range(
        ui,
        blocks,
        0..=blocks.len().saturating_sub(1),
        width,
        mode,
        depth,
        heading_offsets,
        origin_y,
        metrics.as_deref_mut(),
    );
}

fn render_blocks_range(
    ui: &mut Ui,
    blocks: &[MarkdownBlock],
    range: std::ops::RangeInclusive<usize>,
    width: f32,
    mode: theme::ThemeMode,
    depth: usize,
    heading_offsets: &mut Vec<f32>,
    origin_y: f32,
    mut metrics: Option<&mut MarkdownRenderMetrics>,
) {
    if blocks.is_empty() {
        return;
    }
    let start = *range.start();
    let end = (*range.end()).min(blocks.len() - 1);
    for index in start..=end {
        let block = &blocks[index];
        let block_top = (ui.cursor().top() - origin_y).max(0.0);
        match block {
            MarkdownBlock::Paragraph(spans) => {
                render_inline(ui, spans, width, 15.0, false, Align::LEFT, mode);
            }
            MarkdownBlock::Heading { level, spans } => {
                ui.add_space(if *level == 1 { 4.0 } else { 2.0 });
                heading_offsets.push((ui.cursor().top() - origin_y).max(0.0));
                render_inline(
                    ui,
                    spans,
                    width,
                    heading_size(*level),
                    true,
                    Align::LEFT,
                    mode,
                );
            }
            MarkdownBlock::List(list) => {
                render_list(ui, list, width, mode, depth, heading_offsets, origin_y);
            }
            MarkdownBlock::CodeBlock { language, text } => {
                render_code_block(ui, language.as_deref(), text, width, mode);
            }
            MarkdownBlock::Table(table) => {
                render_table(ui, table, width, mode);
            }
            MarkdownBlock::Quote(children) => {
                render_quote(ui, children, width, mode, depth, heading_offsets, origin_y);
            }
            MarkdownBlock::FootnoteDefinition { label, blocks } => {
                render_footnote_definition(
                    ui,
                    label,
                    blocks,
                    width,
                    mode,
                    depth,
                    heading_offsets,
                    origin_y,
                );
            }
            MarkdownBlock::Rule => {
                ui.separator();
            }
        }

        let mut block_bottom = (ui.cursor().top() - origin_y).max(block_top);
        if index + 1 < blocks.len() {
            ui.add_space(block_spacing(block));
            block_bottom = (ui.cursor().top() - origin_y).max(block_bottom);
        }
        if let Some(metrics) = metrics.as_deref_mut() {
            metrics.block_tops.push(block_top);
            metrics.block_bottoms.push(block_bottom);
        }
    }
}

fn block_spacing(block: &MarkdownBlock) -> f32 {
    match block {
        MarkdownBlock::Paragraph(_) => 8.0,
        MarkdownBlock::Heading { .. } => 10.0,
        MarkdownBlock::List(_) => 6.0,
        MarkdownBlock::CodeBlock { .. } => 10.0,
        MarkdownBlock::Table(_) => 12.0,
        MarkdownBlock::Quote(_)
        | MarkdownBlock::FootnoteDefinition { .. }
        | MarkdownBlock::Rule => 8.0,
    }
}

fn render_list(
    ui: &mut Ui,
    list: &MarkdownList,
    width: f32,
    mode: theme::ThemeMode,
    depth: usize,
    heading_offsets: &mut Vec<f32>,
    origin_y: f32,
) {
    let marker_width = if list.items.iter().any(|item| item.checked.is_some()) {
        22.0
    } else if list.start.is_some() {
        34.0
    } else {
        18.0
    };
    let gap = 8.0;
    let text_width = (width - marker_width - gap).max(80.0);
    let mut number = list.start.unwrap_or(1);

    for item in &list.items {
        ui.horizontal_top(|ui| {
            ui.set_width(width);
            draw_list_marker(
                ui,
                list.start.map(|_| number),
                item.checked,
                depth,
                marker_width,
                mode,
            );
            ui.add_space(gap);
            ui.vertical(|ui| {
                ui.set_width(text_width);
                render_blocks(
                    ui,
                    &item.blocks,
                    text_width,
                    mode,
                    depth + 1,
                    heading_offsets,
                    origin_y,
                    None,
                );
            });
        });
        number += 1;
    }
}

fn draw_list_marker(
    ui: &mut Ui,
    number: Option<u64>,
    checked: Option<bool>,
    depth: usize,
    width: f32,
    mode: theme::ThemeMode,
) {
    let height = ui.text_style_height(&egui::TextStyle::Body).max(18.0);
    if let Some(checked) = checked {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
        let side = 13.0;
        let box_rect = egui::Rect::from_center_size(rect.center(), Vec2::splat(side));
        ui.painter().rect(
            box_rect,
            egui::CornerRadius::same(3),
            if checked {
                theme::primary_for(mode).gamma_multiply(0.25)
            } else {
                Color32::TRANSPARENT
            },
            Stroke::new(1.2, theme::primary_for(mode)),
            egui::StrokeKind::Outside,
        );
        if checked {
            ui.painter().line_segment(
                [
                    box_rect.left_center() + Vec2::new(3.0, 0.0),
                    box_rect.center() + Vec2::new(-1.0, 4.0),
                ],
                Stroke::new(1.4, theme::primary_for(mode)),
            );
            ui.painter().line_segment(
                [
                    box_rect.center() + Vec2::new(-1.0, 4.0),
                    box_rect.right_top() + Vec2::new(-2.5, 3.0),
                ],
                Stroke::new(1.4, theme::primary_for(mode)),
            );
        }
        return;
    }

    if let Some(number) = number {
        ui.add_sized(
            [width, height],
            Label::new(
                RichText::new(format!("{number}."))
                    .color(theme::primary_for(mode))
                    .size(14.0),
            ),
        );
        return;
    }

    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    let center = rect.center();
    let radius = 3.7;
    let color = theme::primary_for(mode);
    if depth == 0 {
        ui.painter().circle_filled(center, radius, color);
    } else {
        ui.painter().circle(
            center,
            radius,
            Color32::TRANSPARENT,
            Stroke::new(1.1, color),
        );
    }
}

fn render_table(ui: &mut Ui, table: &MarkdownTable, width: f32, mode: theme::ThemeMode) {
    let columns = table_column_count(table);
    if columns == 0 {
        return;
    }

    let table_width = width.max(120.0);
    let column_widths = table_column_widths(table_width, columns);
    let padding = Vec2::new(10.0, 7.0);
    let border = theme::muted_for(mode).gamma_multiply(0.45);
    let header_fill = theme::bg_for(mode).gamma_multiply(1.35);

    let header_height = table_row_height(ui, &table.headers, &column_widths, true, padding, mode);
    let body_heights = table
        .rows
        .iter()
        .map(|row| table_row_height(ui, row, &column_widths, false, padding, mode))
        .collect::<Vec<_>>();
    let total_height = header_height + body_heights.iter().sum::<f32>();

    let (outer_rect, _) =
        ui.allocate_exact_size(Vec2::new(table_width, total_height), egui::Sense::hover());
    let painter = ui.painter();

    painter.rect(
        outer_rect,
        egui::CornerRadius::same(theme::RADIUS_SM),
        Color32::TRANSPARENT,
        Stroke::new(1.0, border),
        egui::StrokeKind::Outside,
    );

    let header_rect = Rect::from_min_max(
        outer_rect.left_top(),
        Pos2::new(outer_rect.right(), outer_rect.top() + header_height),
    );
    painter.rect_filled(
        header_rect,
        egui::CornerRadius::same(theme::RADIUS_SM),
        header_fill,
    );

    let mut y = outer_rect.top();
    render_table_row(
        ui,
        table,
        &table.headers,
        &column_widths,
        Rect::from_min_max(
            Pos2::new(outer_rect.left(), y),
            Pos2::new(outer_rect.right(), y + header_height),
        ),
        true,
        padding,
        mode,
    );
    y += header_height;

    for (row, row_height) in table.rows.iter().zip(body_heights.iter().copied()) {
        render_table_row(
            ui,
            table,
            row,
            &column_widths,
            Rect::from_min_max(
                Pos2::new(outer_rect.left(), y),
                Pos2::new(outer_rect.right(), y + row_height),
            ),
            false,
            padding,
            mode,
        );
        y += row_height;
    }

    let stroke = Stroke::new(1.0, border);
    let mut x = outer_rect.left();
    for width in column_widths.iter().take(columns.saturating_sub(1)) {
        x += *width;
        painter.line_segment(
            [
                Pos2::new(x, outer_rect.top()),
                Pos2::new(x, outer_rect.bottom()),
            ],
            stroke,
        );
    }

    let mut y = outer_rect.top() + header_height;
    painter.line_segment(
        [
            Pos2::new(outer_rect.left(), y),
            Pos2::new(outer_rect.right(), y),
        ],
        stroke,
    );
    for row_height in body_heights {
        y += row_height;
        painter.line_segment(
            [
                Pos2::new(outer_rect.left(), y),
                Pos2::new(outer_rect.right(), y),
            ],
            stroke,
        );
    }
}

fn render_table_row(
    ui: &Ui,
    table: &MarkdownTable,
    cells: &[Vec<InlineSpan>],
    column_widths: &[f32],
    row_rect: Rect,
    header: bool,
    padding: Vec2,
    mode: theme::ThemeMode,
) {
    let mut x = row_rect.left();
    for (index, column_width) in column_widths.iter().enumerate() {
        let cell_rect = Rect::from_min_max(
            Pos2::new(x, row_rect.top()),
            Pos2::new(x + column_width, row_rect.bottom()),
        );
        render_table_cell(
            ui,
            cells.get(index).map(Vec::as_slice).unwrap_or(&[]),
            cell_rect,
            header,
            table_alignment(table, index),
            padding,
            mode,
        );
        x += column_width;
    }
}

fn render_table_cell(
    ui: &Ui,
    spans: &[InlineSpan],
    rect: Rect,
    header: bool,
    alignment: Align,
    padding: Vec2,
    mode: theme::ThemeMode,
) {
    let mut spans = spans.to_vec();
    if header {
        for span in &mut spans {
            span.style.strong = true;
        }
    }

    let content_rect = rect.shrink2(padding);
    let job = inline_layout_job(
        &spans,
        content_rect.width(),
        14.0,
        false,
        alignment,
        mode,
        ui,
    );
    let galley = ui.fonts(|fonts| fonts.layout_job(job));
    let painter = ui.painter().with_clip_rect(rect);
    let text_pos = Pos2::new(
        content_rect.left(),
        content_rect.center().y - galley.size().y * 0.5,
    );
    painter.galley(text_pos, galley, theme::text_for(mode));
}

fn table_column_count(table: &MarkdownTable) -> usize {
    table.headers.len()
}

fn table_column_widths(width: f32, columns: usize) -> Vec<f32> {
    if columns == 0 {
        return Vec::new();
    }

    let base = (width / columns as f32).floor().max(72.0);
    let mut widths = vec![base; columns];
    let used = base * columns as f32;
    if let Some(last) = widths.last_mut() {
        *last += (width - used).max(0.0);
    }
    widths
}

fn table_row_height(
    ui: &Ui,
    cells: &[Vec<InlineSpan>],
    column_widths: &[f32],
    header: bool,
    padding: Vec2,
    mode: theme::ThemeMode,
) -> f32 {
    let content_height = column_widths
        .iter()
        .enumerate()
        .map(|(index, column_width)| {
            let mut spans = cells.get(index).cloned().unwrap_or_default();
            if header {
                for span in &mut spans {
                    span.style.strong = true;
                }
            }
            let inner_width = (*column_width - padding.x * 2.0).max(48.0);
            let job = inline_layout_job(&spans, inner_width, 14.0, false, Align::LEFT, mode, ui);
            ui.fonts(|fonts| fonts.layout_job(job)).size().y
        })
        .fold(0.0, f32::max);

    (content_height + padding.y * 2.0).max(36.0)
}

fn table_alignment(table: &MarkdownTable, index: usize) -> Align {
    match table
        .alignments
        .get(index)
        .copied()
        .unwrap_or(TableAlignment::None)
    {
        TableAlignment::None | TableAlignment::Left => Align::LEFT,
        TableAlignment::Center => Align::Center,
        TableAlignment::Right => Align::RIGHT,
    }
}

fn table_alignment_from_pulldown(alignment: Alignment) -> TableAlignment {
    match alignment {
        Alignment::None => TableAlignment::None,
        Alignment::Left => TableAlignment::Left,
        Alignment::Center => TableAlignment::Center,
        Alignment::Right => TableAlignment::Right,
    }
}

fn render_inline(
    ui: &mut Ui,
    spans: &[InlineSpan],
    width: f32,
    size: f32,
    heading: bool,
    alignment: Align,
    mode: theme::ThemeMode,
) {
    if spans.is_empty() {
        return;
    }

    let job = inline_layout_job(spans, width, size, heading, alignment, mode, ui);
    ui.add(Label::new(job).wrap());
}

fn inline_layout_job(
    spans: &[InlineSpan],
    width: f32,
    size: f32,
    heading: bool,
    alignment: Align,
    mode: theme::ThemeMode,
    ui: &Ui,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    job.wrap.break_anywhere = should_break_anywhere(spans);
    job.break_on_newline = true;
    job.halign = alignment;

    for span in spans {
        let font_family = match (span.style.code, span.style.strong || heading) {
            (true, true) => theme::markdown_strong_monospace_font_family(),
            (true, false) => theme::markdown_monospace_font_family(),
            (false, true) => theme::markdown_strong_font_family(),
            (false, false) => theme::editor_system_font_family(),
        };
        let font_family = available_font_family(
            ui,
            font_family,
            if span.style.code {
                egui::FontFamily::Monospace
            } else {
                egui::FontFamily::Proportional
            },
        );
        let mut format = TextFormat {
            font_id: if span.style.code {
                FontId::new((size - 1.0).max(11.0), font_family)
            } else {
                FontId::new(size, font_family)
            },
            color: if heading {
                theme::primary_for(mode)
            } else if span.style.link.is_some() {
                theme::primary_for(mode)
            } else if span.style.code {
                inline_code_text_color(mode)
            } else if span.style.strong {
                strong_text_color(mode)
            } else {
                theme::markdown_text_for(mode)
            },
            italics: span.style.emphasis,
            background: if span.style.code {
                inline_code_background(mode, ui.visuals().code_bg_color)
            } else {
                Color32::TRANSPARENT
            },
            ..Default::default()
        };

        if span.style.link.is_some() {
            format.underline = Stroke::new(1.0, theme::primary_for(mode));
        }
        if span.style.strikethrough {
            format.strikethrough = Stroke::new(1.0, theme::muted_for(mode));
        }
        job.append(&span.text, 0.0, format);
    }

    job
}

fn strong_text_color(mode: theme::ThemeMode) -> Color32 {
    match mode {
        theme::ThemeMode::Light => mix_color(
            theme::markdown_text_for(mode),
            theme::primary_for(mode),
            0.16,
        ),
        theme::ThemeMode::Dark => mix_color(
            theme::markdown_text_for(mode),
            theme::primary_for(mode),
            0.18,
        ),
    }
}

fn inline_code_text_color(mode: theme::ThemeMode) -> Color32 {
    match mode {
        theme::ThemeMode::Light => mix_color(
            theme::markdown_text_for(mode),
            theme::primary_for(mode),
            0.22,
        ),
        theme::ThemeMode::Dark => mix_color(
            theme::markdown_text_for(mode),
            theme::primary_for(mode),
            0.24,
        ),
    }
}

fn inline_code_background(mode: theme::ThemeMode, fallback_code_bg: Color32) -> Color32 {
    let tint = match mode {
        theme::ThemeMode::Light => 0.08,
        theme::ThemeMode::Dark => 0.10,
    };
    mix_color(fallback_code_bg, theme::primary_for(mode), tint)
}

fn mix_color(from: Color32, to: Color32, amount: f32) -> Color32 {
    let amount = amount.clamp(0.0, 1.0);
    let inverse = 1.0 - amount;
    Color32::from_rgba_premultiplied(
        (from.r() as f32 * inverse + to.r() as f32 * amount).round() as u8,
        (from.g() as f32 * inverse + to.g() as f32 * amount).round() as u8,
        (from.b() as f32 * inverse + to.b() as f32 * amount).round() as u8,
        (from.a() as f32 * inverse + to.a() as f32 * amount).round() as u8,
    )
}

fn available_font_family(
    ui: &Ui,
    preferred: egui::FontFamily,
    fallback: egui::FontFamily,
) -> egui::FontFamily {
    match &preferred {
        egui::FontFamily::Name(_) => {
            let available = ui.fonts(|fonts| fonts.families().contains(&preferred));
            if available { preferred } else { fallback }
        }
        _ => preferred,
    }
}

fn should_break_anywhere(spans: &[InlineSpan]) -> bool {
    let text = spans
        .iter()
        .map(|span| span.text.as_str())
        .collect::<String>();
    let has_cjk = text.chars().any(is_cjk);
    let has_whitespace = text.chars().any(char::is_whitespace);
    has_cjk && !has_whitespace
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x3040..=0x30FF
            | 0xAC00..=0xD7AF
    )
}

fn render_code_block(
    ui: &mut Ui,
    language: Option<&str>,
    text: &str,
    width: f32,
    mode: theme::ThemeMode,
) {
    Frame::new()
        .fill(ui.visuals().code_bg_color)
        .stroke(Stroke::new(
            1.0,
            theme::muted_for(mode).gamma_multiply(0.45),
        ))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_SM))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width((width - 20.0).max(80.0));
            if let Some(language) = language.filter(|language| !language.is_empty()) {
                ui.label(
                    RichText::new(language)
                        .font(FontId::new(11.0, theme::markdown_monospace_font_family()))
                        .size(11.0)
                        .color(theme::muted_for(mode)),
                );
                ui.add_space(4.0);
            }
            ui.add(
                Label::new(
                    RichText::new(text.trim_end_matches('\n'))
                        .font(FontId::new(13.0, theme::markdown_monospace_font_family()))
                        .size(13.0)
                        .color(theme::text_for(mode)),
                )
                .wrap(),
            );
        });
}

fn render_quote(
    ui: &mut Ui,
    children: &[MarkdownBlock],
    width: f32,
    mode: theme::ThemeMode,
    depth: usize,
    heading_offsets: &mut Vec<f32>,
    origin_y: f32,
) {
    ui.horizontal_top(|ui| {
        let marker_left = ui.cursor().left();
        ui.add_space(4.0);
        ui.add_space(10.0);
        let content = ui.vertical(|ui| {
            let child_width = (width - 14.0).max(80.0);
            ui.set_width(child_width);
            render_blocks(
                ui,
                children,
                child_width,
                mode,
                depth,
                heading_offsets,
                origin_y,
                None,
            );
        });
        let rect = quote_marker_rect(marker_left, content.response.rect);
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(2),
            theme::primary_for(mode).gamma_multiply(0.6),
        );
    });
}

fn quote_marker_rect(left: f32, content_rect: Rect) -> Rect {
    Rect::from_min_max(
        Pos2::new(left, content_rect.top()),
        Pos2::new(left + 4.0, content_rect.bottom()),
    )
}

fn render_footnote_definition(
    ui: &mut Ui,
    label: &str,
    blocks: &[MarkdownBlock],
    width: f32,
    mode: theme::ThemeMode,
    depth: usize,
    heading_offsets: &mut Vec<f32>,
    origin_y: f32,
) {
    ui.horizontal_top(|ui| {
        ui.add_sized(
            [34.0, ui.text_style_height(&egui::TextStyle::Body).max(18.0)],
            Label::new(
                RichText::new(format!("[^{label}]"))
                    .font(FontId::new(12.0, theme::markdown_monospace_font_family()))
                    .size(12.0)
                    .color(theme::muted_for(mode)),
            ),
        );
        ui.add_space(8.0);
        ui.vertical(|ui| {
            let child_width = (width - 42.0).max(80.0);
            ui.set_width(child_width);
            render_blocks(
                ui,
                blocks,
                child_width,
                mode,
                depth + 1,
                heading_offsets,
                origin_y,
                None,
            );
        });
    });
}

fn heading_size(level: u8) -> f32 {
    match level {
        1 => 25.0,
        2 => 21.0,
        3 => 18.0,
        4 => 16.0,
        _ => 15.0,
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn code_block_language(kind: CodeBlockKind<'_>) -> Option<String> {
    match kind {
        CodeBlockKind::Indented => None,
        CodeBlockKind::Fenced(language) => {
            let language = language.trim();
            (!language.is_empty()).then(|| language.to_string())
        }
    }
}

#[cfg(test)]
#[path = "markdown_preview_test.rs"]
mod markdown_preview_test;

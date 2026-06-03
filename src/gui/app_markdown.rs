//! Markdown 编辑、预览和文档内部 outline surface。
//!
//! 这里保留 Markdown route 的具体界面业务；`app.rs` 只决定何时进入该 surface。

use super::*;

impl GsdvGuiApp {
    pub(super) fn markdown_surface(&mut self, ui: &mut Ui, active_mode: CenterMode) {
        self.ensure_active_document_loaded();
        let document_error = self.active_document().and_then(|document| {
            document
                .load_error
                .as_ref()
                .or(document.save_error.as_ref())
        });
        if let Some(error) = document_error {
            document_error_strip(ui, &error);
            ui.add_space(8.0);
        }

        let available = ui.available_size();
        if available.x <= 1.0 || available.y <= 1.0 {
            return;
        }

        let (rect, _) = ui.allocate_exact_size(available, Sense::hover());
        let (preview_max_scroll_y, editor_max_scroll_y) = self
            .active_document()
            .map(|document| {
                (
                    document.markdown_preview_max_scroll_y,
                    document.markdown_editor_max_scroll_y,
                )
            })
            .unwrap_or_default();

        let scroll_y = self
            .active_document()
            .map(|document| document.markdown_scroll_y)
            .unwrap_or_default();

        let next_scroll_y = match active_mode {
            CenterMode::Preview => {
                self.markdown_preview_page(ui, rect, rect, preview_max_scroll_y, scroll_y)
            }
            CenterMode::Editor => {
                self.markdown_editor_page(ui, rect, rect, editor_max_scroll_y, scroll_y)
            }
            CenterMode::Agent | CenterMode::Terminal => scroll_y,
        };
        if let Some(document) = self.active_document_mut() {
            document.markdown_scroll_y = next_scroll_y.max(0.0);
        }
    }

    pub(super) fn markdown_preview_page(
        &mut self,
        ui: &mut Ui,
        page_rect: egui::Rect,
        clip_rect: egui::Rect,
        max_scroll_y: f32,
        scroll_y: f32,
    ) -> f32 {
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(page_rect)
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.set_clip_rect(clip_rect);
                ui.set_width(page_rect.width());
                ui.set_height(page_rect.height());
                Frame::new()
                    .fill(theme::bg())
                    .stroke(Stroke::new(1.0, theme::border()))
                    .corner_radius(CornerRadius::same(theme::RADIUS_LG))
                    .inner_margin(Margin::same(24))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width().max(80.0));
                        ui.set_min_height(ui.available_height().max(120.0));
                        self.markdown_with_outline(
                            ui,
                            "preview",
                            true,
                            max_scroll_y,
                            scroll_y,
                            |app, ui, scroll_y| {
                                let empty = app
                                    .active_document()
                                    .is_none_or(|document| document.text.trim().is_empty());
                                if empty {
                                    empty_document_panel(
                                        ui,
                                        i18n::text(
                                            app.app_language,
                                            "Select a markdown file to preview rendered content.",
                                        ),
                                        app.app_language,
                                    );
                                    scroll_y
                                } else {
                                    let preview_width = (ui.available_width() - 4.0).max(80.0);
                                    let preview_height = ui.available_height().max(120.0);
                                    let theme_mode = app.theme_mode;
                                    ui.scope(|ui| {
                                        ui.set_style(theme::markdown_style(theme_mode));
                                        let output = ScrollArea::vertical()
                                            .id_salt("workspace-preview")
                                            .vertical_scroll_offset(scroll_y)
                                            .max_width(preview_width)
                                            .max_height(preview_height)
                                            .auto_shrink([false, false])
                                            .show_viewport(ui, |ui, viewport| {
                                                ui.set_width(preview_width);
                                                ui.set_min_width(preview_width);
                                                ui.set_max_width(preview_width);
                                                if let Some(document) = app.active_document_mut() {
                                                    let needs_metrics = document
                                                        .markdown_preview_metrics
                                                        .as_ref()
                                                        .is_none_or(|metrics| {
                                                            metrics.block_tops.len()
                                                                != document
                                                                    .markdown_preview_blocks
                                                                    .len()
                                                        })
                                                        || (document
                                                            .markdown_preview_metrics_width
                                                            - preview_width)
                                                            .abs()
                                                            > 0.5;
                                                    if needs_metrics {
                                                        let metrics =
                                                            markdown_preview::render_blocks_with_metrics(
                                                                ui,
                                                                &document.markdown_preview_blocks,
                                                                preview_width,
                                                                theme_mode,
                                                            );
                                                        document.markdown_preview_heading_offsets =
                                                            metrics.heading_offsets.clone();
                                                        document.markdown_preview_max_scroll_y =
                                                            markdown_scroll_max_y(
                                                                metrics.content_height,
                                                                viewport.height(),
                                                            );
                                                        document.markdown_preview_metrics =
                                                            Some(metrics);
                                                        document.markdown_preview_metrics_width =
                                                            preview_width;
                                                    } else if let Some(metrics) =
                                                        document.markdown_preview_metrics.as_ref()
                                                    {
                                                        markdown_preview::render_blocks_virtualized(
                                                            ui,
                                                            &document.markdown_preview_blocks,
                                                            preview_width,
                                                            theme_mode,
                                                            metrics,
                                                            viewport.min.y,
                                                            viewport.height(),
                                                        );
                                                    }
                                                }
                                            });
                                        let max_scroll_y = app
                                            .active_document()
                                            .map(|document| document.markdown_preview_max_scroll_y)
                                            .unwrap_or_else(|| {
                                                markdown_scroll_max_y(
                                                    output.content_size.y,
                                                    output.inner_rect.height(),
                                                )
                                            });
                                        if let Some(document) = app.active_document_mut() {
                                            document.markdown_preview_max_scroll_y = max_scroll_y;
                                        }
                                        output.state.offset.y.min(max_scroll_y)
                                    })
                                    .inner
                                }
                            },
                        )
                    })
                    .inner
            },
        )
        .inner
    }

    pub(super) fn markdown_editor_page(
        &mut self,
        ui: &mut Ui,
        page_rect: egui::Rect,
        clip_rect: egui::Rect,
        max_scroll_y: f32,
        scroll_y: f32,
    ) -> f32 {
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(page_rect)
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.set_clip_rect(clip_rect);
                ui.set_width(page_rect.width());
                ui.set_height(page_rect.height());
                Frame::new()
                    .fill(theme::surface_elevated())
                    .stroke(Stroke::new(1.0, theme::border()))
                    .corner_radius(CornerRadius::same(theme::RADIUS_LG))
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width().max(80.0));
                        ui.set_min_height(ui.available_height().max(120.0));
                        self.markdown_with_outline(
                            ui,
                            "editor",
                            false,
                            max_scroll_y,
                            scroll_y,
                            |app, ui, scroll_y| {
                                let editor_font = effective_editor_font_id(&app.font_settings);
                                let editor_interactive = !app.suppress_editor_input
                                    && app.center_surface_accepts_keyboard_input();
                                let workspace_index = app.active_workspace;
                                let mut document_changed = false;
                                let mut editor_ime_repaint = false;
                                let mut edited_path = None;
                                let offset_y = if let Some(document) = app.active_document_mut() {
                                    let editor_width = ui.available_width().max(80.0);
                                    let editor_height = ui.available_height().max(120.0);
                                    let editor_id = (
                                        "workspace-markdown-editor",
                                        workspace_index,
                                        document.path.as_deref(),
                                    );
                                    let output = ScrollArea::vertical()
                                        .id_salt("workspace-editor")
                                        .vertical_scroll_offset(scroll_y)
                                        .max_width(editor_width)
                                        .max_height(editor_height)
                                        .auto_shrink([false, false])
                                        .show(ui, |ui| {
                                            ui.set_width(editor_width);
                                            ui.set_max_width(editor_width);
                                            // 触发条件：中文 IME commit 让 TextEdit 状态变化但文本没变。
                                            // 不能只看 response.changed：它不等价于已插入文本。
                                            // 防止 Markdown editor 中文提交被 fallback 误判为已处理。
                                            let text_before_ime =
                                                Self::markdown_editor_has_ime_commit(ui)
                                                    .then(|| document.text.clone());
                                            let gutter_width =
                                                editor_line_number_gutter_width(&document.text);
                                            let text_width =
                                                (editor_width - gutter_width - 8.0).max(80.0);
                                            let mut editor_output = ui
                                                .horizontal_top(|ui| {
                                                    let gutter_rect =
                                                        reserve_editor_line_number_gutter(
                                                            ui,
                                                            gutter_width,
                                                        );
                                                    let editor_output = egui::TextEdit::multiline(
                                                        &mut document.text,
                                                    )
                                                    .id_salt(editor_id)
                                                    .font(editor_font.clone())
                                                    .text_color(theme::markdown_text())
                                                    .desired_width(text_width)
                                                    .desired_rows(24)
                                                    .lock_focus(true)
                                                    .interactive(editor_interactive)
                                                    .show(ui);
                                                    stabilize_text_edit_ime_output(
                                                        ui,
                                                        &editor_output,
                                                        &editor_font,
                                                    );
                                                    paint_editor_line_number_gutter(
                                                        ui,
                                                        gutter_rect,
                                                        &editor_output,
                                                        editor_font.clone(),
                                                    );
                                                    editor_output
                                                })
                                                .inner;
                                            let text_changed = editor_output.response.changed();
                                            let text_unchanged_after_edit = text_before_ime
                                                .as_deref()
                                                .is_some_and(|before| before == document.text);
                                            let editor_ime_dirty =
                                                Self::apply_markdown_editor_ime_commit_fallback(
                                                    ui,
                                                    &mut editor_output.state,
                                                    &editor_output.response,
                                                    text_unchanged_after_edit,
                                                    &mut document.text,
                                                );
                                            document_changed = text_changed || editor_ime_dirty;
                                            if editor_ime_dirty {
                                                editor_ime_repaint = true;
                                            }
                                        });
                                    let max_scroll_y = markdown_scroll_max_y(
                                        output.content_size.y,
                                        output.inner_rect.height(),
                                    );
                                    document.markdown_editor_max_scroll_y = max_scroll_y;
                                    let offset_y = output.state.offset.y.min(max_scroll_y);
                                    if document_changed {
                                        edited_path = document.path.clone();
                                    }
                                    if editor_ime_repaint {
                                        app.pending_input_repaint = true;
                                    }
                                    offset_y
                                } else {
                                    empty_document_panel(
                                        ui,
                                        i18n::text(
                                            app.app_language,
                                            "Select a markdown file to start editing.",
                                        ),
                                        app.app_language,
                                    );
                                    scroll_y
                                };
                                if document_changed {
                                    app.pending_markdown_reparse.insert(workspace_index);
                                }
                                if let Some(path) = edited_path {
                                    app.record_recent_markdown_access(workspace_index, path);
                                }
                                offset_y
                            },
                        )
                    })
                    .inner
            },
        )
        .inner
    }

    /// Lays out a Markdown page with a temporary, document-local heading outline.
    pub(super) fn markdown_with_outline(
        &mut self,
        ui: &mut Ui,
        page_id: &'static str,
        use_preview_offsets: bool,
        max_scroll_y: f32,
        scroll_y: f32,
        add_content: impl FnOnce(&mut Self, &mut Ui, f32) -> f32,
    ) -> f32 {
        let outline_collapsed = self
            .current_workspace()
            .is_some_and(|workspace| workspace.markdown_outline_collapsed);
        if outline_collapsed {
            return add_content(self, ui, scroll_y);
        }

        let total_width = ui.available_width().max(1.0);
        let outline_width = (total_width * MARKDOWN_OUTLINE_WIDTH_FRACTION).max(1.0);
        let height = ui.available_height().max(120.0);
        let mut next_scroll_y = scroll_y;
        let mut target_scroll_y = None;
        let mut collapse_outline = false;
        let active_workspace = self.active_workspace;
        StripBuilder::new(ui)
            .size(Size::exact(outline_width))
            .size(Size::exact(8.0))
            .size(Size::remainder())
            .horizontal(|mut strip| {
                strip.cell(|ui| {
                    if let Some(document) = self.active_document() {
                        let heading_offsets = use_preview_offsets
                            .then_some(document.markdown_preview_heading_offsets.as_slice());
                        let result = markdown_outline_panel(
                            ui,
                            page_id,
                            active_workspace,
                            &document.markdown_outline_entries,
                            heading_offsets,
                            max_scroll_y,
                            outline_width,
                            height,
                            scroll_y,
                            self.app_language,
                        );
                        target_scroll_y = result.scroll_target;
                        collapse_outline = result.collapse_outline;
                    }
                });
                strip.empty();
                strip.cell(|ui| {
                    next_scroll_y = add_content(self, ui, target_scroll_y.unwrap_or(scroll_y));
                });
            });
        if collapse_outline {
            self.pending_markdown_outline_collapse
                .insert(active_workspace);
        }
        next_scroll_y
    }

    /// Applies IME commits that egui skipped because no preedit state was active.
    pub(super) fn apply_markdown_editor_ime_commit_fallback(
        ui: &Ui,
        state: &mut TextEditState,
        response: &egui::Response,
        text_unchanged_after_edit: bool,
        text: &mut String,
    ) -> bool {
        if !response.has_focus() || !text_unchanged_after_edit {
            return false;
        }
        let commits = Self::markdown_editor_ime_commit_texts(ui);
        if commits.is_empty() {
            return false;
        }

        let mut range = state.cursor.char_range().unwrap_or_else(|| {
            egui::text::CCursorRange::one(egui::text::CCursor::new(text.chars().count()))
        });
        for commit in commits {
            Self::insert_text_at_ccursor_range(text, &mut range, &commit);
        }
        state.cursor.set_char_range(Some(range));
        state.clone().store(ui.ctx(), response.id);
        true
    }

    /// Returns whether this frame carries an IME commit for editor fallback.
    pub(super) fn markdown_editor_has_ime_commit(ui: &Ui) -> bool {
        ui.input(|input| {
            input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Ime(egui::ImeEvent::Commit(text)) if !text.is_empty()))
        })
    }

    /// Extracts non-duplicated IME commit text for Markdown editor fallback input.
    pub(super) fn markdown_editor_ime_commit_texts(ui: &Ui) -> Vec<String> {
        ui.input(|input| markdown_editor_ime_commit_texts_from_events(&input.events))
    }

    /// Replaces the selected character range and moves the cursor after inserted text.
    pub(super) fn insert_text_at_ccursor_range(
        text: &mut String,
        range: &mut egui::text::CCursorRange,
        insert: &str,
    ) {
        let start_char = range.primary.index.min(range.secondary.index);
        let end_char = range.primary.index.max(range.secondary.index);
        let start = Self::byte_index_for_char(text, start_char);
        let end = Self::byte_index_for_char(text, end_char);
        text.replace_range(start..end, insert);

        let next = egui::text::CCursor::new(start_char + insert.chars().count());
        *range = egui::text::CCursorRange::one(next);
    }

    /// Converts a character boundary into a byte index for UTF-8 string edits.
    pub(super) fn byte_index_for_char(text: &str, char_index: usize) -> usize {
        text.char_indices()
            .map(|(index, _)| index)
            .nth(char_index)
            .unwrap_or(text.len())
    }
}

pub(super) fn document_error_strip(ui: &mut Ui, error: &str) {
    Frame::new()
        .fill(theme::danger_soft())
        .stroke(Stroke::new(1.0, theme::danger_border()))
        .corner_radius(CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                status_dot(ui, theme::danger());
                ui.colored_label(theme::danger(), error);
            });
        });
}

/// Renders the current Markdown document's heading list from prepared entries.
pub(super) fn markdown_outline_panel(
    ui: &mut Ui,
    page_id: &'static str,
    active_workspace: usize,
    outline: &[MarkdownOutlineEntry],
    heading_offsets: Option<&[f32]>,
    max_scroll_y: f32,
    width: f32,
    height: f32,
    scroll_y: f32,
    language: AppLanguage,
) -> MarkdownOutlinePanelResult {
    let mut result = MarkdownOutlinePanelResult::default();
    let active_index = markdown_outline_active_index(outline, heading_offsets, scroll_y);
    Frame::new()
        .fill(theme::surface())
        .stroke(Stroke::new(1.0, theme::border()))
        .corner_radius(CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(width);
            ui.set_height(height);
            ui.horizontal(|ui| {
                ui.label(section_label(i18n::text(language, "Outline")));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("<").clicked() {
                        result.collapse_outline = true;
                    }
                });
            });
            ui.add_space(6.0);
            ScrollArea::both()
                .id_salt(("markdown-document-outline", active_workspace, page_id))
                .max_width(ui.available_width())
                .max_height(ui.available_height())
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_width(width.max(160.0));
                    if outline.is_empty() {
                        ui.label(muted(i18n::text(language, "No headings")));
                    } else {
                        for (index, entry) in outline.iter().enumerate() {
                            let active = active_index == Some(index);
                            let indent = ((entry.level.saturating_sub(1)) as f32) * 14.0;
                            let row_width = ui.available_width().max(160.0);
                            let (rect, response) =
                                ui.allocate_exact_size(Vec2::new(row_width, 24.0), Sense::click());
                            if active || response.hovered() {
                                let fill = if active {
                                    theme::primary_soft()
                                } else {
                                    theme::surface_elevated()
                                };
                                ui.painter().rect_filled(
                                    rect,
                                    CornerRadius::same(theme::RADIUS_SM),
                                    fill,
                                );
                            }
                            let text_color = if active || response.hovered() {
                                theme::primary()
                            } else {
                                theme::text()
                            };
                            let font_id = if active {
                                egui::FontId::proportional(13.5)
                            } else {
                                egui::FontId::proportional(13.0)
                            };
                            ui.painter().text(
                                egui::pos2(rect.left() + indent, rect.center().y),
                                Align2::LEFT_CENTER,
                                &entry.title,
                                font_id,
                                text_color,
                            );
                            if response.clicked() {
                                result.scroll_target = Some(
                                    markdown_outline_scroll_y(
                                        entry.line,
                                        heading_offsets.and_then(|offsets| offsets.get(index)),
                                    )
                                    .min(max_scroll_y),
                                );
                            }
                        }
                    }
                });
        });
    result
}

pub(super) fn empty_document_panel(ui: &mut Ui, message: &str, language: AppLanguage) {
    ui.centered_and_justified(|ui| {
        Frame::new()
            .fill(theme::surface())
            .stroke(Stroke::new(1.0, theme::border()))
            .corner_radius(CornerRadius::same(theme::RADIUS_MD))
            .inner_margin(Margin::same(20))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(i18n::text(language, "No document selected"))
                        .strong()
                        .color(theme::text()),
                );
                ui.label(muted(message));
            });
    });
}

pub(super) fn normalize_markdown_name(value: &str) -> String {
    let trimmed = value.trim().trim_matches('/').trim();
    let fallback = if trimmed.is_empty() {
        "untitled"
    } else {
        trimmed
    };
    if fallback.ends_with(".md") {
        fallback.to_string()
    } else {
        format!("{fallback}.md")
    }
}

pub(super) fn normalize_folder_name(value: &str) -> String {
    let trimmed = value.trim().trim_matches('/').trim();
    if trimmed.is_empty() {
        "new-folder".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Extracts ATX headings for the current Markdown document outline.
pub(super) fn markdown_outline_entries(markdown: &str) -> Vec<MarkdownOutlineEntry> {
    let mut entries = Vec::new();
    let mut fenced = false;
    for (line_index, line) in markdown.lines().enumerate() {
        let trimmed_start = line.trim_start();
        if trimmed_start.starts_with("```") || trimmed_start.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let Some((level, title)) = markdown_heading_from_line(trimmed_start) else {
            continue;
        };
        entries.push(MarkdownOutlineEntry {
            level,
            line: line_index,
            title,
        });
    }
    entries
}

/// Converts a heading source or render position to the shared scroll offset.
pub(super) fn markdown_outline_scroll_y(line: usize, rendered_y: Option<&f32>) -> f32 {
    (rendered_y
        .copied()
        .unwrap_or_else(|| markdown_outline_line_y(line))
        - MARKDOWN_OUTLINE_SCROLL_TOP_PADDING)
        .max(0.0)
}

/// Computes the highest stable vertical offset for a Markdown scroll area.
pub(super) fn markdown_scroll_max_y(content_height: f32, viewport_height: f32) -> f32 {
    (content_height - viewport_height).max(0.0)
}

/// Converts a source heading line to its approximate unpadded scroll position.
pub(super) fn markdown_outline_line_y(line: usize) -> f32 {
    (line as f32) * 24.0
}

/// Chooses the heading closest to the top edge of the Markdown viewport.
pub(super) fn markdown_outline_active_index(
    outline: &[MarkdownOutlineEntry],
    heading_offsets: Option<&[f32]>,
    scroll_y: f32,
) -> Option<usize> {
    outline
        .iter()
        .enumerate()
        .min_by(|(left_index, left), (right_index, right)| {
            let left_y = markdown_outline_entry_y(left, heading_offsets, *left_index);
            let right_y = markdown_outline_entry_y(right, heading_offsets, *right_index);
            let left_distance = (left_y - scroll_y).abs();
            let right_distance = (right_y - scroll_y).abs();
            left_distance.total_cmp(&right_distance)
        })
        .map(|(index, _)| index)
}

/// Returns the best known rendered position for an outline entry.
pub(super) fn markdown_outline_entry_y(
    entry: &MarkdownOutlineEntry,
    heading_offsets: Option<&[f32]>,
    index: usize,
) -> f32 {
    heading_offsets
        .and_then(|offsets| offsets.get(index))
        .copied()
        .unwrap_or_else(|| markdown_outline_line_y(entry.line))
}

/// Parses one ATX heading line into its depth and visible title.
pub(super) fn markdown_heading_from_line(line: &str) -> Option<(usize, String)> {
    let level = line.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = line.get(level..)?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let title = rest.trim().trim_end_matches('#').trim_end().to_string();
    if title.is_empty() {
        return None;
    }
    Some((level, title))
}

pub(super) fn normalize_rename_name(current: &Path, value: &str) -> String {
    let trimmed = value.trim().trim_matches('/').trim();
    if trimmed.is_empty() {
        return current
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "untitled.md".to_string());
    }
    if current
        .extension()
        .is_some_and(|extension| extension == "md")
        && !trimmed.ends_with(".md")
    {
        format!("{trimmed}.md")
    } else {
        trimmed.to_string()
    }
}

pub(super) fn markdown_title(file_name: &str) -> String {
    file_name
        .trim_end_matches(".md")
        .replace(['-', '_'], " ")
        .split_whitespace()
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

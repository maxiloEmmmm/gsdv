//! Reviewer route 的 egui surface。
//!
//! 该模块只负责把 reviewer snapshot 画成 UI，并把交互回传给 app 调度。

use super::*;

impl GsdvGuiApp {
    pub(super) fn reviewer_surface(&mut self, ui: &mut Ui, _mode: ReviewerMode) {
        self.ensure_active_reviewer();

        ui.vertical(|ui| {
            let available = ui.available_size();
            let scroll_to_row = self
                .reviewer_diff_scroll_targets
                .get_mut(self.active_workspace)
                .and_then(Option::take);
            let snapshot = self.reviewer_snapshot_for_paint();
            let reviewer_scripts = &self.reviewer_scripts.scripts;
            let reviewer_script_error = self.reviewer_scripts.last_error.as_deref();

            if let Some(snapshot) = snapshot {
                if available.x < reviewer_min_width_px() {
                    Frame::new()
                        .fill(theme::surface_elevated())
                        .stroke(Stroke::new(1.0, theme::border()))
                        .corner_radius(CornerRadius::same(theme::RADIUS_LG))
                        .inner_margin(Margin::same(24))
                        .show(ui, |ui| {
                            ui.label(muted(&format!(
                                "{} {}",
                                i18n::text(
                                    self.app_language,
                                    "Need at least this many columns for the dense reviewer layout:",
                                ),
                                crate::reviewer::MIN_REVIEWER_WIDTH
                            )));
                        });
                } else {
                    let mut diff_action = crate::gui::diff_viewer::DiffViewerAction::None;
                    let mut observed_scroll_row = None;
                    let mut selected_diff_row = None;
                    let mut script_run_request = None;
                    let mut clicked_reviewer_row = None;
                    let mut refresh_dirty_row = None;
                    let mut collapse_file_tree = false;
                    let mut load_more_commits = None;
                    StripBuilder::new(ui)
                        .clip(true)
                        .cell_layout(Layout::top_down(Align::Min))
                        .size(Size::relative(0.15).at_least(164.0).at_most(238.0))
                        .size(Size::relative(0.125).at_least(148.0).at_most(218.0))
                        .size(Size::relative(0.125).at_least(148.0).at_most(226.0))
                        .size(Size::remainder().at_least(480.0))
                        .horizontal(|mut strip| {
                            strip.cell(|ui| {
                                let result = reviewer_structured_column(
                                    ui,
                                    0,
                                    &snapshot.columns[0].title,
                                    &snapshot.columns[0].rows,
                                    reviewer_scripts,
                                    reviewer_script_error,
                                    self.app_language,
                                );
                                if script_run_request.is_none() {
                                    script_run_request = result.script_run_request;
                                }
                                if clicked_reviewer_row.is_none() {
                                    clicked_reviewer_row = result.clicked_row;
                                }
                                if refresh_dirty_row.is_none() {
                                    refresh_dirty_row = result.refresh_dirty_row;
                                }
                                collapse_file_tree |= result.collapse_file_tree;
                                if result.commit_load_more_visible {
                                    load_more_commits = Some(result.commit_row_budget);
                                }
                            });
                            strip.cell(|ui| {
                                let result = reviewer_structured_column(
                                    ui,
                                    1,
                                    &snapshot.columns[1].title,
                                    &snapshot.columns[1].rows,
                                    reviewer_scripts,
                                    reviewer_script_error,
                                    self.app_language,
                                );
                                if script_run_request.is_none() {
                                    script_run_request = result.script_run_request;
                                }
                                if clicked_reviewer_row.is_none() {
                                    clicked_reviewer_row = result.clicked_row;
                                }
                                if refresh_dirty_row.is_none() {
                                    refresh_dirty_row = result.refresh_dirty_row;
                                }
                                collapse_file_tree |= result.collapse_file_tree;
                            });
                            strip.cell(|ui| {
                                let result = reviewer_structured_column(
                                    ui,
                                    2,
                                    &snapshot.columns[2].title,
                                    &snapshot.columns[2].rows,
                                    reviewer_scripts,
                                    reviewer_script_error,
                                    self.app_language,
                                );
                                if script_run_request.is_none() {
                                    script_run_request = result.script_run_request;
                                }
                                if clicked_reviewer_row.is_none() {
                                    clicked_reviewer_row = result.clicked_row;
                                }
                                if refresh_dirty_row.is_none() {
                                    refresh_dirty_row = result.refresh_dirty_row;
                                }
                                collapse_file_tree |= result.collapse_file_tree;
                            });
                            strip.cell(|ui| {
                                apply_editor_content_font_style(ui, &self.font_settings);
                                let result = reviewer_diff_column(
                                    ui,
                                    &snapshot.diff_title,
                                    &snapshot.diff_lines,
                                    snapshot.diff_content_chars,
                                    &snapshot,
                                    scroll_to_row,
                                );
                                diff_action = result.action;
                                observed_scroll_row = Some(result.scroll_row);
                                selected_diff_row = result.selected_row;
                            });
                        });
                    if let Some((column, row, commit_row_budget)) = clicked_reviewer_row {
                        if let Some(adapter) = self.active_reviewer_adapter_mut() {
                            let needs_load =
                                adapter.click_row_light(column, row, commit_row_budget);
                            let snapshot = adapter.snapshot().clone();
                            if let Some(snapshot_slot) =
                                self.reviewer_snapshots.get_mut(self.active_workspace)
                            {
                                *snapshot_slot = Some(snapshot);
                            }
                            if needs_load {
                                self.spawn_reviewer_git_data_task(
                                    ui.ctx(),
                                    commit_row_budget,
                                    false,
                                );
                            }
                        } else {
                            self.spawn_reviewer_adapter_task(
                                ui.ctx(),
                                ReviewerAdapterTask::ClickRow {
                                    column,
                                    row,
                                    commit_row_budget,
                                },
                                ReviewerAdapterTaskEffect::None,
                            );
                        }
                    }
                    if let Some(row_budget) = load_more_commits {
                        self.spawn_reviewer_git_data_task(ui.ctx(), row_budget, true);
                    }
                    if let Some(row) = refresh_dirty_row {
                        self.spawn_reviewer_adapter_task(
                            ui.ctx(),
                            ReviewerAdapterTask::RefreshRepoDirty(row),
                            ReviewerAdapterTaskEffect::None,
                        );
                    }
                    if collapse_file_tree
                        && let Some(adapter) = self.active_reviewer_adapter_mut()
                    {
                        adapter.collapse_file_tree_to_first_level();
                    }
                    if let Some(request) = script_run_request {
                        self.handle_reviewer_script_request(request);
                    }
                    if let Some(row) = observed_scroll_row
                        && let Some(adapter) = self.active_reviewer_adapter_mut()
                    {
                        adapter.set_diff_scroll_row(row);
                    }
                    if let Some(row) = selected_diff_row {
                        self.select_reviewer_diff_row_for_gui(ui.ctx(), row);
                    }
                    if diff_action != crate::gui::diff_viewer::DiffViewerAction::None {
                        self.dispatch_reviewer_diff_action(ui.ctx(), diff_action);
                    }
                }
            }
        });
    }
}

/// Events collected while painting a reviewer column.
struct ReviewerStructuredColumnResult {
    /// Row clicked by the user, identified by column and row.
    clicked_row: Option<(usize, usize, usize)>,
    /// Current dynamic commit row budget for history paging.
    commit_row_budget: usize,
    /// Whether the commit load-more row is currently visible.
    commit_load_more_visible: bool,
    /// Git repo row requested for dirty-state refresh.
    refresh_dirty_row: Option<usize>,
    /// Whether the current file tree should collapse after painting.
    collapse_file_tree: bool,
    /// Script request selected from a context menu.
    script_run_request: Option<ReviewerScriptRunRequest>,
}

/// Paints one reviewer column from precomputed rows and records interactions.
fn reviewer_structured_column(
    ui: &mut Ui,
    column_index: usize,
    title: &str,
    rows: &[crate::reviewer::app::GuiReviewerRow],
    scripts: &[ReviewerScript],
    script_error: Option<&str>,
    language: AppLanguage,
) -> ReviewerStructuredColumnResult {
    let mut result = ReviewerStructuredColumnResult {
        clicked_row: None,
        commit_row_budget: 20,
        commit_load_more_visible: false,
        refresh_dirty_row: None,
        collapse_file_tree: false,
        script_run_request: None,
    };
    let row_height = 24.0;
    let frame_height = ui.available_height();
    Frame::new()
        .fill(theme::bg())
        .stroke(Stroke::new(1.0, theme::border()))
        .corner_radius(CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(Margin::symmetric(8, 8))
        .show(ui, |ui| {
            ui.set_min_height((frame_height - 18.0).max(80.0));
            ui.spacing_mut().item_spacing = Vec2::new(0.0, 1.0);
            let collapse_tree = ui.ctx().input(tree_collapse_shortcut_pressed);
            ui.horizontal(|ui| {
                ui.label(section_label(title));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(rows.len().to_string())
                            .size(11.0)
                            .color(theme::muted()),
                    );
                });
            });
            ui.add_space(6.0);
            let scroll_height = (ui.available_height() - 2.0).max(80.0);
            result.commit_row_budget = commit_row_budget(scroll_height, row_height);
            let scroll_area = if rows.iter().any(|row| row.tree.is_some()) {
                ScrollArea::both()
            } else {
                ScrollArea::vertical()
            };
            scroll_area
                .id_salt(format!("reviewer-col-{column_index}-{title}"))
                .max_height(scroll_height)
                .auto_shrink([false, false])
                .show_rows(ui, row_height, rows.len(), |ui, row_range| {
                    if column_index == 1
                        && row_range.end >= rows.len()
                        && rows.last().is_some_and(|row| {
                            row.label == crate::reviewer::app::LOAD_MORE_COMMITS_LABEL
                        })
                    {
                        result.commit_load_more_visible = true;
                    }
                    for row_index in row_range {
                        let row = &rows[row_index];
                        let response = match &row.tree {
                            Some(crate::reviewer::app::GuiReviewerTreeRow::Dir {
                                depth,
                                expanded,
                            }) => compact_tree_row(
                                ui,
                                *depth,
                                TreeRowMarker::Dir {
                                    expanded: *expanded,
                                },
                                &row.label,
                                row.selected,
                            ),
                            Some(crate::reviewer::app::GuiReviewerTreeRow::File { depth }) => {
                                compact_tree_row(
                                    ui,
                                    *depth,
                                    TreeRowMarker::File { badge: None },
                                    &row.label,
                                    row.selected,
                                )
                            }
                            None => reviewer_list_row(ui, &row.label, row.selected, row.tone),
                        };
                        if response.clicked() {
                            result.clicked_row =
                                Some((column_index, row_index, result.commit_row_budget));
                        }
                        if column_index == 0 && response.secondary_clicked() {
                            result.refresh_dirty_row = Some(row_index);
                        }
                        if row.tree.is_some() && response.hovered() && collapse_tree {
                            result.collapse_file_tree = true;
                        }
                        if let Some(target) = row.script_target.as_ref() {
                            let target = ReviewerAdapter::reviewer_branch_target(target);
                            response.context_menu(|ui| {
                                reviewer_script_menu(
                                    ui,
                                    scripts,
                                    script_error,
                                    &target,
                                    &mut result.script_run_request,
                                    language,
                                );
                            });
                        }
                    }
                });
        });
    result
}

fn reviewer_script_menu(
    ui: &mut Ui,
    scripts: &[ReviewerScript],
    script_error: Option<&str>,
    target: &ReviewerBranchTarget,
    action: &mut Option<ReviewerScriptRunRequest>,
    language: AppLanguage,
) {
    ui.label(muted(&target.label));
    ui.separator();

    if let Some(error) = script_error {
        ui.add_enabled(
            false,
            Button::new(i18n::text_with_arg(
                language,
                "Script scan failed: {error}",
                "{error}",
                error,
            )),
        );
        return;
    }

    if scripts.is_empty() {
        ui.add_enabled(
            false,
            Button::new(i18n::text(language, "No scripts in ~/.gsdv/reviewer")),
        );
        return;
    }

    for script in scripts {
        if ui.button(&script.label).clicked() {
            *action = Some(ReviewerScriptRunRequest {
                script: script.clone(),
                target: target.clone(),
            });
            ui.close_menu();
        }
    }
}

/// Calculates two visible screens of commit rows, rounded up to a stable batch.
fn commit_row_budget(scroll_height: f32, row_height: f32) -> usize {
    let visible = (scroll_height / row_height).ceil().max(1.0) as usize;
    let two_screens = visible.saturating_mul(2).max(1);
    two_screens.div_ceil(10) * 10
}

fn reviewer_list_row(
    ui: &mut Ui,
    label: &str,
    selected: bool,
    tone: GuiReviewerRowTone,
) -> egui::Response {
    let row_height = 24.0;
    let width = ui.available_width().max(80.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, row_height), Sense::click());
    let fill = if selected {
        theme::primary_soft()
    } else if response.hovered() {
        theme::hover()
    } else {
        theme::transparent()
    };
    if fill != theme::transparent() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme::RADIUS_SM), fill);
    }

    let label_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 8.0, rect.top()),
        egui::pos2(
            rect.right() - if selected { 22.0 } else { 6.0 },
            rect.bottom(),
        ),
    );
    ui.painter().with_clip_rect(label_rect).text(
        egui::pos2(label_rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        label,
        egui::TextStyle::Body.resolve(ui.style()),
        reviewer_row_text_color(tone),
    );
    if selected {
        ui.painter().circle_filled(
            egui::pos2(rect.right() - 13.0, rect.center().y),
            3.0,
            theme::primary(),
        );
    }
    response
}

fn reviewer_row_text_color(tone: GuiReviewerRowTone) -> Color32 {
    match tone {
        GuiReviewerRowTone::Normal => theme::list_text(),
        GuiReviewerRowTone::Dirty => theme::warning(),
    }
}

/// Applies local diff row selection to a reviewer paint snapshot.
pub(super) fn apply_reviewer_diff_selection_override(
    snapshot: &mut crate::reviewer::app::GuiReviewerSnapshot,
    selected_row: Option<usize>,
) {
    let Some(selected_row) = selected_row else {
        return;
    };
    if selected_row >= snapshot.diff_lines.len() {
        return;
    }
    for (index, line) in snapshot.diff_lines.iter_mut().enumerate() {
        line.selected = index == selected_row;
    }
    snapshot.active_column = crate::reviewer::app::ActiveColumn::Diff;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReviewerDiffCopyKind {
    Line,
    Metadata,
}

struct ReviewerDiffColumnResult {
    action: crate::gui::diff_viewer::DiffViewerAction,
    scroll_row: usize,
    selected_row: Option<usize>,
}

fn reviewer_diff_column(
    ui: &mut Ui,
    title: &str,
    lines: &[crate::reviewer::app::GuiDiffLine],
    content_chars: usize,
    snapshot: &crate::reviewer::app::GuiReviewerSnapshot,
    scroll_to_row: Option<usize>,
) -> ReviewerDiffColumnResult {
    let viewer_lines = lines
        .iter()
        .map(reviewer_diff_viewer_line)
        .collect::<Vec<_>>();
    let result = crate::gui::diff_viewer::diff_viewer_column(
        ui,
        crate::gui::diff_viewer::DiffViewerSnapshot {
            title,
            lines: &viewer_lines,
            content_chars,
            scroll_row: snapshot.diff_scroll_row,
            mode: reviewer_diff_viewer_mode(snapshot.diff_view_mode),
            can_jump_previous_block: snapshot.can_jump_previous_block,
            can_jump_next_block: snapshot.can_jump_next_block,
            current_block: snapshot.current_diff_block,
            block_count: snapshot.diff_block_count,
            keyboard_active: true,
            copy_active: snapshot.active_column == crate::reviewer::app::ActiveColumn::Diff,
        },
        scroll_to_row,
        format!("reviewer-diff-{title}"),
    );
    ReviewerDiffColumnResult {
        action: result.action,
        scroll_row: result.scroll_row,
        selected_row: result.selected_row,
    }
}

/// Converts reviewer diff mode into the shared diff viewer mode.
fn reviewer_diff_viewer_mode(
    mode: crate::reviewer::app::DiffViewMode,
) -> crate::gui::diff_viewer::DiffViewerMode {
    match mode {
        crate::reviewer::app::DiffViewMode::Diff => crate::gui::diff_viewer::DiffViewerMode::Diff,
        crate::reviewer::app::DiffViewMode::Full => crate::gui::diff_viewer::DiffViewerMode::Full,
    }
}

/// Converts reviewer diff rows into shared diff viewer rows.
fn reviewer_diff_viewer_line(
    line: &crate::reviewer::app::GuiDiffLine,
) -> crate::gui::diff_viewer::DiffViewerLine {
    let kind = match line.kind {
        crate::reviewer::app::GuiDiffLineKind::Context => {
            crate::gui::diff_viewer::DiffViewerLineKind::Context
        }
        crate::reviewer::app::GuiDiffLineKind::Insert => {
            crate::gui::diff_viewer::DiffViewerLineKind::Insert
        }
        crate::reviewer::app::GuiDiffLineKind::Delete => {
            crate::gui::diff_viewer::DiffViewerLineKind::Delete
        }
        crate::reviewer::app::GuiDiffLineKind::Hunk => {
            crate::gui::diff_viewer::DiffViewerLineKind::Hunk
        }
        crate::reviewer::app::GuiDiffLineKind::Metadata => {
            crate::gui::diff_viewer::DiffViewerLineKind::Metadata
        }
    };
    crate::gui::diff_viewer::DiffViewerLine {
        kind,
        text: line.text.clone(),
        selected: line.selected,
        copy_line: line.single_click_copy.clone(),
        copy_diff: line.double_click_copy.clone(),
    }
}

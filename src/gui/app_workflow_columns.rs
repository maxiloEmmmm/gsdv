//! GUI Workflow 的 Finder 风格 Column View。
//!
//! 导航只消费已经加载的 WorkflowTree；文件读取、保存和弹窗继续走既有 AppEvent 链。

use super::app_shell_ui::{
    WorkflowStepSelect, workflow_project_copy_path, workflow_root_copy_path,
    workflow_task_copy_path,
};
use super::*;

/// 单行递归 checkbox 进度。
///
/// 适用场景：Repo、Project、Task 背景进度。示例：3/5 -> 60%。
#[derive(Clone, Copy, Default)]
struct WorkflowColumnProgress {
    /// 已完成 checkbox 数。
    done: usize,
    /// checkbox 总数。
    total: usize,
}

impl WorkflowColumnProgress {
    /// 返回背景填充比例。
    ///
    /// 适用场景：部分完成行。示例：1/4 -> 0.25。
    fn ratio(self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.done as f32 / self.total as f32
        }
    }

    /// 判断是否存在 checkbox 且全部完成。
    ///
    /// 适用场景：整行绿色。示例：4/4 -> true。
    fn complete(self) -> bool {
        self.total > 0 && self.done == self.total
    }
}

impl GsdvGuiApp {
    /// 绘制 Repo|Project|Task|Step|工作台 Workflow modal。
    ///
    /// 适用场景：底栏 Workflow 按钮。示例：选择 step -> 右侧 task/step 上下工作台。
    pub(super) fn workflow_column_modal_surface(&mut self, ui: &mut Ui) {
        if ui.available_width() < 900.0 {
            self.workflow_quick_modal_surface(ui);
            return;
        }
        let (loading, load_error, selected, tree) = self
            .workflow_states
            .get(self.active_workspace)
            .map(|state| {
                (
                    state.loading,
                    state.load_error.clone(),
                    state.selected.clone(),
                    state.tree.clone(),
                )
            })
            .unwrap_or((false, None, None, None));
        let Some(tree) = tree else {
            let message = if loading {
                i18n::text(self.app_language, "Loading workflow...").to_string()
            } else {
                load_error.unwrap_or_else(|| {
                    i18n::text(self.app_language, "Loading workflow...").to_string()
                })
            };
            workflow_column_empty(ui, &message);
            return;
        };

        let selected_tree =
            workflow_column_selected_tree(&tree, selected.as_ref()).unwrap_or(&tree);
        let selected_project = workflow_column_selected_project(selected_tree, selected.as_ref());
        let selected_task = workflow_column_selected_task(selected_tree, selected.as_ref());
        let selected_step_paths = self
            .workflow_states
            .get(self.active_workspace)
            .filter(|state| {
                selected_task.is_some_and(|task| {
                    state.step_selection_task_path.as_deref() == Some(task.path.as_path())
                })
            })
            .map(|state| state.selected_step_paths.clone())
            .unwrap_or_default();

        let mut target = None;
        let mut step_select = None;
        let mut context_dialog = None;
        let mut merge_step_paths = None;
        let mut copy_path = None;
        let mut open_agent_target = None;
        let mut destroy_agent_target = None;
        let workflow_agent_targets = self
            .workspaces
            .get(self.active_workspace)
            .map(|workspace| {
                workspace
                    .subagents
                    .iter()
                    .filter_map(|agent| agent.workflow_target.clone())
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let busy_workflow_agent_targets = self
            .workspaces
            .get(self.active_workspace)
            .map(|workspace| {
                workspace
                    .subagents
                    .iter()
                    .filter(|agent| agent.activity == WorkspaceActivity::Busy)
                    .filter_map(|agent| agent.workflow_target.clone())
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let mut scan_sub_workflows = false;
        let available = ui.available_size();
        let navigation_width = (available.x * 0.54).clamp(620.0, 820.0);
        let widths = [
            navigation_width * 0.19,
            navigation_width * 0.23,
            navigation_width * 0.27,
            navigation_width * 0.31,
        ];

        StripBuilder::new(ui)
            .size(Size::exact(widths[0]))
            .size(Size::exact(1.0))
            .size(Size::exact(widths[1]))
            .size(Size::exact(1.0))
            .size(Size::exact(widths[2]))
            .size(Size::exact(1.0))
            .size(Size::exact(widths[3]))
            .size(Size::exact(8.0))
            .size(Size::remainder())
            .horizontal(|mut strip| {
                strip.cell(|ui| {
                    self.workflow_repo_column(
                        ui,
                        &tree,
                        selected.as_ref(),
                        &mut target,
                        &mut context_dialog,
                        &mut copy_path,
                        &mut scan_sub_workflows,
                        &workflow_agent_targets,
                        &busy_workflow_agent_targets,
                        &mut open_agent_target,
                        &mut destroy_agent_target,
                    );
                });
                strip.cell(workflow_column_separator);
                strip.cell(|ui| {
                    self.workflow_project_column(
                        ui,
                        selected_tree,
                        selected.as_ref(),
                        &mut target,
                        &mut context_dialog,
                        &mut copy_path,
                        &workflow_agent_targets,
                        &busy_workflow_agent_targets,
                        &mut open_agent_target,
                        &mut destroy_agent_target,
                    );
                });
                strip.cell(workflow_column_separator);
                strip.cell(|ui| {
                    self.workflow_task_column(
                        ui,
                        selected_tree,
                        selected_project,
                        selected.as_ref(),
                        &mut target,
                        &mut context_dialog,
                        &mut copy_path,
                        &workflow_agent_targets,
                        &busy_workflow_agent_targets,
                        &mut open_agent_target,
                        &mut destroy_agent_target,
                    );
                });
                strip.cell(workflow_column_separator);
                strip.cell(|ui| {
                    self.workflow_step_column(
                        ui,
                        selected_tree,
                        selected_project,
                        selected_task,
                        selected.as_ref(),
                        &selected_step_paths,
                        &mut target,
                        &mut step_select,
                        &mut context_dialog,
                        &mut merge_step_paths,
                        &mut copy_path,
                        &workflow_agent_targets,
                        &busy_workflow_agent_targets,
                        &mut open_agent_target,
                        &mut destroy_agent_target,
                    );
                });
                strip.empty();
                strip.cell(|ui| {
                    self.workflow_column_workbench(ui, selected_tree, selected.as_ref());
                });
            });

        if let Some(target) = destroy_agent_target {
            if let Some(binding) = crate::gui::workflow::workflow_agent_binding(&tree, &target) {
                self.destroy_workflow_agent(ui.ctx(), self.active_workspace, &binding.target);
            }
        } else if let Some(target) = open_agent_target {
            self.request_workflow_agent(ui.ctx(), target);
        } else if let Some(dialog) = context_dialog {
            self.set_active_app_dialog(Some(dialog));
        } else if let Some(step_paths) = merge_step_paths {
            if let Some(task) = selected_task {
                let title = workflow_column_merge_title(task, &step_paths);
                self.set_active_app_dialog(Some(AppDialog::WorkflowMergeSteps {
                    task_path: task.path.clone(),
                    step_paths,
                    title,
                }));
            }
        } else if let Some(path) = copy_path {
            ui.ctx().copy_text(path);
            self.push_toast(
                i18n::text(self.app_language, "Workflow path copied"),
                theme::success(),
            );
        } else if scan_sub_workflows {
            self.request_sub_workflow_scan(ui.ctx(), self.active_workspace);
        } else if let Some(target) = target {
            self.request_workflow_quick_target(ui.ctx(), target);
        } else if let Some(step_select) = step_select
            && let Some(task) = selected_task
        {
            self.apply_workflow_step_selection(ui.ctx(), task, step_select);
        }
    }

    /// 绘制 Repo 列及其既有右键操作。
    ///
    /// 适用场景：主 repo 与子 repo 并列。示例：点击子 repo -> Project 列切换。
    fn workflow_repo_column(
        &self,
        ui: &mut Ui,
        tree: &WorkflowTree,
        selected: Option<&WorkflowSelectionTarget>,
        target: &mut Option<WorkflowSelectionTarget>,
        context_dialog: &mut Option<AppDialog>,
        copy_path: &mut Option<String>,
        scan_sub_workflows: &mut bool,
        workflow_agent_targets: &BTreeSet<data::WorkflowAgentTarget>,
        busy_workflow_agent_targets: &BTreeSet<data::WorkflowAgentTarget>,
        open_agent_target: &mut Option<WorkflowSelectionTarget>,
        destroy_agent_target: &mut Option<WorkflowSelectionTarget>,
    ) {
        workflow_column_frame(ui, "Repo", |ui| {
            for repo in workflow_trees_for_display(tree) {
                let repo_target = WorkflowSelectionTarget::WorkspaceRoot {
                    root_path: repo.root_path.clone(),
                };
                let label = if repo.repo_path.as_os_str().is_empty() {
                    "Root".to_string()
                } else {
                    display_path(&repo.repo_path)
                };
                let response = workflow_column_progress_row(
                    ui,
                    &label,
                    workflow_column_tree_progress(repo),
                    workflow_column_target_in_tree(repo, selected),
                    workflow_target_is_busy(repo, &repo_target, busy_workflow_agent_targets),
                );
                response.context_menu(|ui| {
                    workflow_agent_menu_items(
                        ui,
                        repo,
                        &repo_target,
                        workflow_agent_targets,
                        open_agent_target,
                        destroy_agent_target,
                    );
                    if ui
                        .button(i18n::text(self.app_language, "Copy path"))
                        .clicked()
                    {
                        *copy_path = Some(workflow_root_copy_path(repo));
                        ui.close_menu();
                    }
                    if ui
                        .button(i18n::text(self.app_language, "Open root.md"))
                        .clicked()
                    {
                        *target = Some(repo_target.clone());
                        ui.close_menu();
                    }
                    if ui
                        .button(i18n::text(self.app_language, "Add project"))
                        .clicked()
                    {
                        *context_dialog = Some(AppDialog::WorkflowAddProject {
                            spec_path: repo.spec_path.clone(),
                            key: String::new(),
                        });
                        ui.close_menu();
                    }
                    if ui
                        .button(i18n::text(self.app_language, "Scan sub-workflows"))
                        .clicked()
                    {
                        *scan_sub_workflows = true;
                        ui.close_menu();
                    }
                });
                if response.clicked() {
                    *target = Some(repo_target);
                }
            }
        });
    }

    /// 绘制 Project 列及其既有右键操作。
    ///
    /// 适用场景：Repo 选择后。示例：project 2/3 -> 三分之二绿色背景。
    fn workflow_project_column(
        &self,
        ui: &mut Ui,
        tree: &WorkflowTree,
        selected: Option<&WorkflowSelectionTarget>,
        target: &mut Option<WorkflowSelectionTarget>,
        context_dialog: &mut Option<AppDialog>,
        copy_path: &mut Option<String>,
        workflow_agent_targets: &BTreeSet<data::WorkflowAgentTarget>,
        busy_workflow_agent_targets: &BTreeSet<data::WorkflowAgentTarget>,
        open_agent_target: &mut Option<WorkflowSelectionTarget>,
        destroy_agent_target: &mut Option<WorkflowSelectionTarget>,
    ) {
        workflow_column_frame(ui, "Project", |ui| {
            for project in workflow_projects_for_display(tree) {
                let project_target = WorkflowSelectionTarget::Project {
                    root_path: project.root_path.clone(),
                };
                let response = workflow_column_progress_row(
                    ui,
                    &project.label,
                    workflow_column_project_progress(project),
                    workflow_column_target_in_project(project, selected),
                    workflow_target_is_busy(tree, &project_target, busy_workflow_agent_targets),
                );
                response.context_menu(|ui| {
                    workflow_agent_menu_items(
                        ui,
                        tree,
                        &project_target,
                        workflow_agent_targets,
                        open_agent_target,
                        destroy_agent_target,
                    );
                    if ui
                        .button(i18n::text(self.app_language, "Copy path"))
                        .clicked()
                    {
                        *copy_path = Some(workflow_project_copy_path(tree, project));
                        ui.close_menu();
                    }
                    if ui
                        .button(i18n::text(self.app_language, "Open project"))
                        .clicked()
                    {
                        *target = Some(project_target.clone());
                        ui.close_menu();
                    }
                    if ui
                        .button(i18n::text(self.app_language, "Rename project"))
                        .clicked()
                    {
                        *context_dialog = Some(AppDialog::WorkflowRenameProject {
                            project_path: tree.spec_path.join("ps").join(&project.key),
                            key: project.label.clone(),
                        });
                        ui.close_menu();
                    }
                    if ui
                        .button(i18n::text(self.app_language, "Add task"))
                        .clicked()
                    {
                        *context_dialog = Some(AppDialog::WorkflowAddTask {
                            spec_path: tree.spec_path.clone(),
                            project_key: project.key.clone(),
                            key: String::new(),
                        });
                        ui.close_menu();
                    }
                    if ui
                        .button(i18n::text(self.app_language, "Delete project"))
                        .clicked()
                    {
                        *context_dialog = Some(AppDialog::WorkflowDeleteConfirm {
                            target: WorkflowDeleteTarget::Project {
                                project_path: tree.spec_path.join("ps").join(&project.key),
                                project_key: project.key.clone(),
                            },
                        });
                        ui.close_menu();
                    }
                });
                if response.clicked() {
                    *target = Some(project_target);
                }
            }
        });
    }

    /// 绘制 Task 列及其既有右键操作。
    ///
    /// 适用场景：Project 选择后。示例：完成 task -> 整行绿色。
    fn workflow_task_column(
        &self,
        ui: &mut Ui,
        tree: &WorkflowTree,
        project: Option<&WorkflowProjectNode>,
        selected: Option<&WorkflowSelectionTarget>,
        target: &mut Option<WorkflowSelectionTarget>,
        context_dialog: &mut Option<AppDialog>,
        copy_path: &mut Option<String>,
        workflow_agent_targets: &BTreeSet<data::WorkflowAgentTarget>,
        busy_workflow_agent_targets: &BTreeSet<data::WorkflowAgentTarget>,
        open_agent_target: &mut Option<WorkflowSelectionTarget>,
        destroy_agent_target: &mut Option<WorkflowSelectionTarget>,
    ) {
        workflow_column_frame(ui, "Task", |ui| {
            let Some(project) = project else {
                workflow_column_empty(ui, "-");
                return;
            };
            for task in workflow_tasks_for_display(project) {
                let task_target = WorkflowSelectionTarget::Task {
                    task_path: task.path.clone(),
                };
                let response = workflow_column_progress_row(
                    ui,
                    &task.label,
                    workflow_column_task_progress(task),
                    workflow_column_target_in_task(task, selected),
                    workflow_target_is_busy(tree, &task_target, busy_workflow_agent_targets),
                );
                response.context_menu(|ui| {
                    workflow_agent_menu_items(
                        ui,
                        tree,
                        &task_target,
                        workflow_agent_targets,
                        open_agent_target,
                        destroy_agent_target,
                    );
                    if ui
                        .button(i18n::text(self.app_language, "Copy path"))
                        .clicked()
                    {
                        *copy_path = Some(workflow_task_copy_path(tree, project, task));
                        ui.close_menu();
                    }
                    if ui
                        .button(i18n::text(self.app_language, "Rename task"))
                        .clicked()
                    {
                        *context_dialog = Some(AppDialog::WorkflowRenameTask {
                            task_path: task.path.clone(),
                            key: workflow_column_task_key(task),
                        });
                        ui.close_menu();
                    }
                    if ui
                        .button(i18n::text(self.app_language, "Add step"))
                        .clicked()
                    {
                        *context_dialog = Some(AppDialog::WorkflowAddStep {
                            task_path: task.path.clone(),
                            key: String::new(),
                            desc: String::new(),
                        });
                        ui.close_menu();
                    }
                    if ui
                        .button(i18n::text(self.app_language, "Delete task"))
                        .clicked()
                    {
                        *context_dialog = Some(AppDialog::WorkflowDeleteConfirm {
                            target: WorkflowDeleteTarget::Task {
                                task_path: task.path.clone(),
                                label: task.label.clone(),
                            },
                        });
                        ui.close_menu();
                    }
                });
                if response.clicked() {
                    *target = Some(task_target);
                }
            }
        });
    }

    /// 绘制递归 Step 列及其既有右键操作。
    ///
    /// 适用场景：Task 选择后。示例：点击叶子 step -> 下方工作台切换。
    #[allow(clippy::too_many_arguments)]
    fn workflow_step_column(
        &self,
        ui: &mut Ui,
        tree: &WorkflowTree,
        project: Option<&WorkflowProjectNode>,
        task: Option<&WorkflowTaskNode>,
        selected: Option<&WorkflowSelectionTarget>,
        selected_step_paths: &BTreeSet<Vec<usize>>,
        target: &mut Option<WorkflowSelectionTarget>,
        step_select: &mut Option<WorkflowStepSelect>,
        context_dialog: &mut Option<AppDialog>,
        merge_step_paths: &mut Option<Vec<Vec<usize>>>,
        copy_path: &mut Option<String>,
        workflow_agent_targets: &BTreeSet<data::WorkflowAgentTarget>,
        busy_workflow_agent_targets: &BTreeSet<data::WorkflowAgentTarget>,
        open_agent_target: &mut Option<WorkflowSelectionTarget>,
        destroy_agent_target: &mut Option<WorkflowSelectionTarget>,
    ) {
        workflow_column_frame(ui, "Step", |ui| {
            let (Some(project), Some(task)) = (project, task) else {
                workflow_column_empty(ui, "-");
                return;
            };
            let mut path_parts = if tree.repo_path.as_os_str().is_empty() {
                Vec::new()
            } else {
                vec![display_path(&tree.repo_path)]
            };
            path_parts.push(project.label.clone());
            path_parts.push(task.label.clone());
            workflow_column_step_rows(
                ui,
                task,
                &task.steps,
                0,
                selected,
                selected_step_paths,
                target,
                step_select,
                context_dialog,
                merge_step_paths,
                copy_path,
                &path_parts,
                self.app_language,
                tree,
                workflow_agent_targets,
                busy_workflow_agent_targets,
                open_agent_target,
                destroy_agent_target,
            );
        });
    }

    /// 绘制最右侧 Markdown 工作台。
    ///
    /// 适用场景：task/step 使用上下工作台，root/project 使用只读预览。
    fn workflow_column_workbench(
        &mut self,
        ui: &mut Ui,
        tree: &WorkflowTree,
        selected: Option<&WorkflowSelectionTarget>,
    ) {
        if self.workflow_agent_workbench_surface(ui, tree, selected) {
            return;
        }
        if workflow_column_selected_task(tree, selected).is_some() {
            StripBuilder::new(ui)
                .size(Size::remainder())
                .size(Size::exact(8.0))
                .size(Size::remainder())
                .vertical(|mut strip| {
                    strip.cell(|ui| self.workflow_task_desc_editor_surface_inner(ui, true));
                    strip.empty();
                    strip.cell(|ui| self.workflow_step_editor_surface_inner(ui, true));
                });
            return;
        }
        let Some((path, markdown)) = workflow_column_selected_document(tree, selected) else {
            workflow_column_empty(ui, i18n::text(self.app_language, "Select a workflow task."));
            return;
        };
        let blocks = self.workflow_preview_blocks(markdown, path);
        workflow_column_markdown_preview(ui, &blocks, self.theme_mode);
    }
}

/// 绘制一列的标题、滚动区和边框。
///
/// 适用场景：四个导航列共享外观。示例：Project 列。
fn workflow_column_frame(ui: &mut Ui, title: &str, add_rows: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(theme::surface())
        .stroke(Stroke::new(1.0, theme::border()))
        .inner_margin(Margin::same(4))
        .show(ui, |ui| {
            ui.label(RichText::new(title).small().strong().color(theme::muted()));
            ui.separator();
            ScrollArea::vertical()
                .max_height(ui.available_height().max(1.0))
                .auto_shrink([false, false])
                .show(ui, add_rows);
        });
}

/// 绘制导航列之间的一像素分隔。
///
/// 适用场景：Column View。示例：Repo 与 Project 之间。
fn workflow_column_separator(ui: &mut Ui) {
    let rect = ui.max_rect();
    ui.painter().rect_filled(rect, 0.0, theme::border());
}

/// 绘制行内背景进度。
///
/// 适用场景：非 Step 行。示例：2/4 -> 左半绿色、右侧显示 2/4。
fn workflow_column_progress_row(
    ui: &mut Ui,
    label: &str,
    progress: WorkflowColumnProgress,
    selected: bool,
    busy: bool,
) -> egui::Response {
    let width = ui.available_width().max(60.0);
    let height = 26.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), theme::hover());
    }
    let ratio = if progress.complete() {
        1.0
    } else {
        progress.ratio()
    };
    if ratio > 0.0 {
        let progress_rect = Rect::from_min_max(
            rect.min,
            egui::pos2(rect.left() + rect.width() * ratio, rect.bottom()),
        );
        ui.painter().rect_filled(
            progress_rect,
            CornerRadius::same(4),
            theme::accent_soft(theme::success()),
        );
    }
    if selected {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(4),
            Stroke::new(1.0, theme::primary()),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().text(
        egui::pos2(rect.left() + 7.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        egui::TextStyle::Body.resolve(ui.style()),
        theme::text(),
    );
    if busy {
        ui.painter().text(
            egui::pos2(rect.right() - 7.0, rect.center().y),
            Align2::RIGHT_CENTER,
            "Busy",
            egui::TextStyle::Small.resolve(ui.style()),
            theme::primary(),
        );
    } else if progress.total > 0 && !progress.complete() {
        ui.painter().text(
            egui::pos2(rect.right() - 7.0, rect.center().y),
            Align2::RIGHT_CENTER,
            format!("{}/{}", progress.done, progress.total),
            egui::TextStyle::Small.resolve(ui.style()),
            theme::muted(),
        );
    }
    response
}

/// 递归绘制 Step 行。
///
/// 适用场景：嵌套 step。示例：父 step 下缩进展示子 step。
#[allow(clippy::too_many_arguments)]
fn workflow_column_step_rows(
    ui: &mut Ui,
    task: &WorkflowTaskNode,
    steps: &[WorkflowStepNode],
    depth: usize,
    selected: Option<&WorkflowSelectionTarget>,
    selected_step_paths: &BTreeSet<Vec<usize>>,
    target: &mut Option<WorkflowSelectionTarget>,
    step_select: &mut Option<WorkflowStepSelect>,
    context_dialog: &mut Option<AppDialog>,
    merge_step_paths: &mut Option<Vec<Vec<usize>>>,
    copy_path: &mut Option<String>,
    path_parts: &[String],
    language: AppLanguage,
    tree: &WorkflowTree,
    workflow_agent_targets: &BTreeSet<data::WorkflowAgentTarget>,
    busy_workflow_agent_targets: &BTreeSet<data::WorkflowAgentTarget>,
    open_agent_target: &mut Option<WorkflowSelectionTarget>,
    destroy_agent_target: &mut Option<WorkflowSelectionTarget>,
) {
    for step in workflow_steps_for_display(steps) {
        let step_target = WorkflowSelectionTarget::Step {
            task_path: task.path.clone(),
            step_path: step.path.clone(),
        };
        let step_selected = selected_step_paths.contains(&step.path);
        let row_selected = step_selected || selected == Some(&step_target);
        let width = ui.available_width().max(60.0);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::click());
        if row_selected || response.hovered() {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(4),
                if row_selected {
                    theme::primary_soft()
                } else {
                    theme::hover()
                },
            );
        }
        let marker = if step.checkable {
            if step.checked { "✓" } else { "○" }
        } else {
            "·"
        };
        ui.painter().text(
            egui::pos2(rect.left() + 6.0 + depth as f32 * 12.0, rect.center().y),
            Align2::LEFT_CENTER,
            format!("{marker} {}", step.title),
            egui::TextStyle::Body.resolve(ui.style()),
            if step.checked {
                theme::success()
            } else {
                theme::text()
            },
        );
        if workflow_target_is_busy(tree, &step_target, busy_workflow_agent_targets) {
            ui.painter().text(
                egui::pos2(rect.right() - 6.0, rect.center().y),
                Align2::RIGHT_CENTER,
                "Busy",
                egui::TextStyle::Small.resolve(ui.style()),
                theme::primary(),
            );
        }
        let mut step_parts = path_parts.to_vec();
        step_parts.push(step.title.clone());
        response.context_menu(|ui| {
            workflow_agent_menu_items(
                ui,
                tree,
                &step_target,
                workflow_agent_targets,
                open_agent_target,
                destroy_agent_target,
            );
            if step_selected
                && selected_step_paths.len() >= 2
                && ui.button(i18n::text(language, "Merge steps")).clicked()
            {
                *merge_step_paths = Some(selected_step_paths.iter().cloned().collect());
                ui.close_menu();
            }
            if ui.button(i18n::text(language, "Copy path")).clicked() {
                *copy_path = Some(step_parts.join(" > "));
                ui.close_menu();
            }
            if ui.button(i18n::text(language, "Rename step")).clicked() {
                *context_dialog = Some(AppDialog::WorkflowRenameStep {
                    task_path: task.path.clone(),
                    step_path: step.path.clone(),
                    key: step.title.clone(),
                });
                ui.close_menu();
            }
            if ui.button(i18n::text(language, "Delete step")).clicked() {
                *context_dialog = Some(AppDialog::WorkflowDeleteConfirm {
                    target: WorkflowDeleteTarget::Step {
                        task_path: task.path.clone(),
                        step_path: step.path.clone(),
                        title: step.title.clone(),
                    },
                });
                ui.close_menu();
            }
        });
        if response.clicked() {
            if ui.input(|input| input.modifiers.ctrl) {
                *step_select = Some(WorkflowStepSelect::Toggle {
                    step_path: step.path.clone(),
                });
            } else if ui.input(|input| input.modifiers.shift) {
                *step_select = Some(WorkflowStepSelect::Range {
                    step_path: step.path.clone(),
                });
            } else {
                *target = Some(step_target);
            }
        }
        workflow_column_step_rows(
            ui,
            task,
            &step.children,
            depth + 1,
            selected,
            selected_step_paths,
            target,
            step_select,
            context_dialog,
            merge_step_paths,
            copy_path,
            &step_parts,
            language,
            tree,
            workflow_agent_targets,
            busy_workflow_agent_targets,
            open_agent_target,
            destroy_agent_target,
        );
    }
}

/// 判断目标自身或任一后代是否存在 Busy Workflow Agent。
///
/// 适用场景：上级没有 Agent 也必须冒泡显示 Busy。例：Step Busy -> Task/Project/Repo Busy。
fn workflow_target_is_busy(
    tree: &WorkflowTree,
    selection: &WorkflowSelectionTarget,
    busy_targets: &BTreeSet<data::WorkflowAgentTarget>,
) -> bool {
    let Some(binding) = crate::gui::workflow::workflow_agent_binding(tree, selection) else {
        return false;
    };
    busy_targets.iter().any(|busy| {
        if busy.repo_path != binding.target.repo_path || busy.repo_id != binding.target.repo_id {
            return false;
        }
        match binding.target.kind {
            data::WorkflowAgentTargetKind::Repo => true,
            data::WorkflowAgentTargetKind::Project => busy.project_id == binding.target.project_id,
            data::WorkflowAgentTargetKind::Task => {
                busy.project_id == binding.target.project_id
                    && busy.task_id == binding.target.task_id
            }
            data::WorkflowAgentTargetKind::Step => busy == &binding.target,
        }
    })
}

/// 绘制所有 Workflow 层级共用的 Agent / Destroy Agent 菜单项。
///
/// 适用场景：Repo、Project、Task、Step 右键菜单。例：已有 target -> 同时显示销毁项。
fn workflow_agent_menu_items(
    ui: &mut Ui,
    tree: &WorkflowTree,
    selection: &WorkflowSelectionTarget,
    existing: &BTreeSet<data::WorkflowAgentTarget>,
    open_target: &mut Option<WorkflowSelectionTarget>,
    destroy_target: &mut Option<WorkflowSelectionTarget>,
) {
    let Some(binding) = crate::gui::workflow::workflow_agent_binding(tree, selection) else {
        return;
    };
    if ui.button("Agent").clicked() {
        *open_target = Some(selection.clone());
        ui.close_menu();
    }
    if existing.contains(&binding.target) && ui.button("Destroy Agent").clicked() {
        *destroy_target = Some(selection.clone());
        ui.close_menu();
    }
    ui.separator();
}

/// 绘制 root/project 只读 Markdown 工作台。
///
/// 适用场景：尚未选 task。示例：Repo -> root.md Preview。
fn workflow_column_markdown_preview(
    ui: &mut Ui,
    blocks: &[markdown_preview::MarkdownBlock],
    theme_mode: theme::ThemeMode,
) {
    ScrollArea::vertical()
        .max_height(ui.available_height().max(1.0))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_style(theme::markdown_style(theme_mode));
            markdown_preview::render_blocks_with_heading_offsets(
                ui,
                blocks,
                ui.available_width().max(120.0),
                theme_mode,
            );
        });
}

/// 绘制列空态。
///
/// 适用场景：尚未选择上级节点。示例：未选 Project -> Task 列显示 `-`。
fn workflow_column_empty(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::muted()));
}

/// 汇总 repo checkbox 进度。
///
/// 适用场景：Repo 行。示例：多个 project 合并统计。
fn workflow_column_tree_progress(tree: &WorkflowTree) -> WorkflowColumnProgress {
    tree.projects
        .iter()
        .fold(WorkflowColumnProgress::default(), |mut total, project| {
            let progress = workflow_column_project_progress(project);
            total.done += progress.done;
            total.total += progress.total;
            total
        })
}

/// 汇总 project checkbox 进度。
///
/// 适用场景：Project 行。示例：多个 task 合并统计。
fn workflow_column_project_progress(project: &WorkflowProjectNode) -> WorkflowColumnProgress {
    project
        .tasks
        .iter()
        .fold(WorkflowColumnProgress::default(), |mut total, task| {
            let progress = workflow_column_task_progress(task);
            total.done += progress.done;
            total.total += progress.total;
            total
        })
}

/// 汇总 task checkbox 进度。
///
/// 适用场景：Task 行。示例：嵌套 step 递归统计。
fn workflow_column_task_progress(task: &WorkflowTaskNode) -> WorkflowColumnProgress {
    workflow_column_steps_progress(&task.steps)
}

/// 递归汇总 step checkbox 进度。
///
/// 适用场景：父子 step。示例：非 checkbox 标题不计入总数。
fn workflow_column_steps_progress(steps: &[WorkflowStepNode]) -> WorkflowColumnProgress {
    let mut result = WorkflowColumnProgress::default();
    for step in steps {
        if step.checkable {
            result.total += 1;
            result.done += usize::from(step.checked);
        }
        let children = workflow_column_steps_progress(&step.children);
        result.done += children.done;
        result.total += children.total;
    }
    result
}

/// 返回 target 所属 repo。
///
/// 适用场景：选中任意层级后保持 Repo 列高亮。
fn workflow_column_selected_tree<'a>(
    root: &'a WorkflowTree,
    target: Option<&WorkflowSelectionTarget>,
) -> Option<&'a WorkflowTree> {
    workflow_trees(root).find(|tree| workflow_column_target_in_tree(tree, target))
}

/// 判断 target 是否属于 repo。
///
/// 适用场景：Repo 行选中态。示例：task path 命中该 repo。
fn workflow_column_target_in_tree(
    tree: &WorkflowTree,
    target: Option<&WorkflowSelectionTarget>,
) -> bool {
    target.is_some_and(|target| match target {
        WorkflowSelectionTarget::WorkspaceRoot { root_path } => root_path == &tree.root_path,
        WorkflowSelectionTarget::Project { root_path } => tree
            .projects
            .iter()
            .any(|project| &project.root_path == root_path),
        WorkflowSelectionTarget::Task { task_path }
        | WorkflowSelectionTarget::Step { task_path, .. } => tree
            .projects
            .iter()
            .flat_map(|project| &project.tasks)
            .any(|task| &task.path == task_path),
    })
}

/// 返回 target 所属 project。
///
/// 适用场景：Task 列数据源。示例：step -> 父 project。
fn workflow_column_selected_project<'a>(
    tree: &'a WorkflowTree,
    target: Option<&WorkflowSelectionTarget>,
) -> Option<&'a WorkflowProjectNode> {
    tree.projects
        .iter()
        .find(|project| workflow_column_target_in_project(project, target))
}

/// 判断 target 是否属于 project。
///
/// 适用场景：Project 行选中态。示例：task path 命中。
fn workflow_column_target_in_project(
    project: &WorkflowProjectNode,
    target: Option<&WorkflowSelectionTarget>,
) -> bool {
    target.is_some_and(|target| match target {
        WorkflowSelectionTarget::Project { root_path } => root_path == &project.root_path,
        WorkflowSelectionTarget::Task { task_path }
        | WorkflowSelectionTarget::Step { task_path, .. } => {
            project.tasks.iter().any(|task| &task.path == task_path)
        }
        WorkflowSelectionTarget::WorkspaceRoot { .. } => false,
    })
}

/// 返回 target 所属 task。
///
/// 适用场景：Step 列和工作台。示例：step -> 父 task。
fn workflow_column_selected_task<'a>(
    tree: &'a WorkflowTree,
    target: Option<&WorkflowSelectionTarget>,
) -> Option<&'a WorkflowTaskNode> {
    tree.projects
        .iter()
        .flat_map(|project| &project.tasks)
        .find(|task| workflow_column_target_in_task(task, target))
}

/// 判断 target 是否属于 task。
///
/// 适用场景：Task 行选中态。示例：step task_path 相同 -> true。
fn workflow_column_target_in_task(
    task: &WorkflowTaskNode,
    target: Option<&WorkflowSelectionTarget>,
) -> bool {
    target.is_some_and(|target| match target {
        WorkflowSelectionTarget::Task { task_path }
        | WorkflowSelectionTarget::Step { task_path, .. } => task_path == &task.path,
        WorkflowSelectionTarget::WorkspaceRoot { .. } | WorkflowSelectionTarget::Project { .. } => {
            false
        }
    })
}

/// 返回 root/project 对应的 Markdown 文件与内容。
///
/// 适用场景：最右只读工作台。示例：Project -> project root.md。
fn workflow_column_selected_document<'a>(
    tree: &'a WorkflowTree,
    selected: Option<&WorkflowSelectionTarget>,
) -> Option<(&'a Path, &'a str)> {
    match selected? {
        WorkflowSelectionTarget::WorkspaceRoot { root_path } if root_path == &tree.root_path => {
            Some((&tree.root_path, tree.content.as_str()))
        }
        WorkflowSelectionTarget::Project { root_path } => tree
            .projects
            .iter()
            .find(|project| &project.root_path == root_path)
            .map(|project| (project.root_path.as_path(), project.content.as_str())),
        WorkflowSelectionTarget::Task { .. } | WorkflowSelectionTarget::Step { .. } => None,
        WorkflowSelectionTarget::WorkspaceRoot { .. } => None,
    }
}

/// 返回 task 文件名中的逻辑 key。
///
/// 适用场景：Rename task dialog。示例：task-build.md -> build。
fn workflow_column_task_key(task: &WorkflowTaskNode) -> String {
    task.label.clone()
}

/// 返回多选 step 合并默认标题。
///
/// 适用场景：Merge steps dialog。示例：首个 step 标题作为默认值。
fn workflow_column_merge_title(task: &WorkflowTaskNode, paths: &[Vec<usize>]) -> String {
    paths
        .first()
        .and_then(|path| workflow_column_step_at_path(&task.steps, path))
        .map(|step| step.title.clone())
        .unwrap_or_else(|| "merged-step".to_string())
}

/// 按索引路径返回递归 step。
///
/// 适用场景：合并标题和选择恢复。示例：`[1,0]` -> 第二个父项的首个子项。
fn workflow_column_step_at_path<'a>(
    steps: &'a [WorkflowStepNode],
    path: &[usize],
) -> Option<&'a WorkflowStepNode> {
    let (head, tail) = path.split_first()?;
    let step = steps.get(*head)?;
    if tail.is_empty() {
        Some(step)
    } else {
        workflow_column_step_at_path(&step.children, tail)
    }
}

//! App shell 布局 UI。
//!
//! 本模块绘制 rail、outline panel、center panel、bottom bar 和 drawer 容器；
//! 具体 route surface 继续交给各自模块。

use super::*;

impl GsdvGuiApp {
    pub(super) fn bottom_bar(&mut self, ui: &mut Ui) {
        let language = self.app_language;
        let (workspace_name, mode, file_label, center_mode) = self
            .current_workspace()
            .map(|workspace| {
                let mode = match workspace.center_mode {
                    CenterMode::Agent => "Agent",
                    CenterMode::Terminal => "Agent",
                    CenterMode::Editor => "Editor",
                    CenterMode::Preview => "Preview",
                };
                let file_label = workspace
                    .selected_file
                    .as_ref()
                    .map(|path| workspace_title_path(workspace, path))
                    .unwrap_or_else(|| i18n::text(language, "No file").to_string());
                (
                    workspace.name.clone(),
                    i18n::text(language, mode),
                    file_label,
                    workspace.center_mode,
                )
            })
            .unwrap_or_else(|| {
                (
                    i18n::text(language, "no workspace").to_string(),
                    i18n::text(language, "Agent"),
                    i18n::text(language, "No file").to_string(),
                    CenterMode::Agent,
                )
            });
        let dirty = self
            .active_document()
            .is_some_and(|document| document.is_dirty());
        let md_state = i18n::text(language, if dirty { "[md]-Unsaved" } else { "[md]-Saved" });
        let memo_save_failed = self
            .memo_save_errors
            .get(self.active_workspace)
            .and_then(Option::as_ref)
            .is_some();
        let memo_state = if memo_save_failed {
            i18n::text(language, "[memo]-Error")
        } else {
            i18n::text(language, "[memo]-Saved")
        };
        let work_remaining = self.pomodoro_work_remaining_footer_state();
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), ui.available_height()),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.label(muted(mode));
                if let Some((remaining_fraction, warning)) = work_remaining {
                    separator_dot(ui);
                    pomodoro_work_remaining_footer_bar(ui, remaining_fraction, warning);
                }
                if matches!(center_mode, CenterMode::Editor | CenterMode::Preview) {
                    separator_dot(ui);
                    if dirty {
                        ui.colored_label(theme::warning(), md_state);
                    } else {
                        ui.label(muted(md_state));
                    }
                }
                if self.notifications.open {
                    separator_dot(ui);
                    if memo_save_failed {
                        ui.colored_label(theme::danger(), memo_state);
                    } else {
                        ui.label(muted(memo_state));
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(muted(&truncate_middle(&workspace_name, 28)));
                    separator_dot(ui);
                    ui.label(muted(&truncate_middle(&file_label, 48)));
                });
            },
        );
    }

    /// 返回 footer 工作剩余进度条的渲染状态。
    fn pomodoro_work_remaining_footer_state(&self) -> Option<(f32, bool)> {
        if !self.runtime_settings.pomodoro_enabled || self.pomodoro.phase != PomodoroPhase::Working
        {
            return None;
        }
        let total = pomodoro_work_duration(&self.runtime_settings);
        let elapsed = Instant::now().duration_since(self.pomodoro.phase_started_at);
        let remaining = total.saturating_sub(elapsed);
        let remaining_fraction =
            (remaining.as_secs_f32() / total.as_secs_f32().max(1.0)).clamp(0.0, 1.0);
        let warning = pomodoro_work_progress(&self.runtime_settings, &self.pomodoro)
            >= pomodoro_warning_progress(&self.runtime_settings);
        Some((remaining_fraction, warning))
    }

    /// Draws the full-window bottom bar after panels and drawers.
    pub(super) fn bottom_bar_overlay(&mut self, ctx: &egui::Context) {
        let screen = ctx.screen_rect();
        let height = BOTTOM_BAR_HEIGHT;
        let rect = egui::Rect::from_min_size(
            egui::pos2(screen.left(), screen.bottom() - height),
            Vec2::new(screen.width(), height),
        );
        egui::Area::new("window_bottombar_overlay".into())
            .order(egui::Order::Tooltip)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                ui.set_min_size(rect.size());
                ui.set_max_size(rect.size());
                bottom_bar_frame().show(ui, |ui| {
                    ui.set_min_size(rect.size());
                    ui.set_max_size(rect.size());
                    self.bottom_bar(ui);
                });
            });
    }

    pub(super) fn workspace_rail(&mut self, ui: &mut Ui) {
        if self.rail_collapsed {
            self.compact_workspace_rail(ui);
            return;
        }

        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(ui.max_rect())
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.set_clip_rect(ui.max_rect());
                let in_reviewer_route = self
                    .current_workspace()
                    .is_some_and(|workspace| workspace.route == Route::Reviewer);
                let mut toggle_theme = false;
                let mut collapse_rail = false;
                let mut primary_header_action = false;
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), 28.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        if theme_mode_switch(ui, self.theme_mode, self.app_language).clicked() {
                            toggle_theme = true;
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.add_space(RAIL_EDGE_INSET);
                            if in_reviewer_route {
                                if rail_header_back_button(ui).clicked() {
                                    primary_header_action = true;
                                }
                            } else if rail_header_add_button(ui).clicked() {
                                primary_header_action = true;
                            }
                            ui.add_space(6.0);
                            if rail_header_collapse_button(ui, true).clicked() {
                                collapse_rail = true;
                            }
                        });
                    },
                );
                if toggle_theme {
                    self.request_theme_switch(ui.ctx());
                }
                if collapse_rail {
                    self.rail_collapsed = true;
                    self.persist_workspaces();
                } else if primary_header_action {
                    if in_reviewer_route {
                        self.exit_reviewer_route();
                    } else {
                        self.add_workspace_from_dialog(ui.ctx());
                    }
                }
                ui.add_space(18.0);
                ui.label(section_label(i18n::text(self.app_language, "WORKSPACES")));
                ui.add_space(8.0);

                let mut rail_action = None;
                for (index, workspace) in self.workspaces.iter().enumerate() {
                    let response = workspace_rail_row(
                        ui,
                        workspace,
                        index == self.active_workspace,
                        &self.repaint_controller,
                    );
                    response.context_menu(|ui| {
                        if ui
                            .button(i18n::text(self.app_language, "Close workspace"))
                            .clicked()
                        {
                            rail_action = Some(WorkspaceRailAction::Close(index));
                            ui.close_menu();
                        }
                    });
                    if response.clicked() {
                        rail_action = Some(WorkspaceRailAction::Switch(index));
                    }
                    ui.add_space(4.0);
                }
                if let Some(action) = rail_action {
                    self.handle_workspace_rail_action(ui.ctx(), action);
                }

                let bottom_height = 30.0 + 10.0 + 30.0;
                ui.add_space((ui.available_height() - bottom_height).max(24.0));
                if rail_nav_row(ui, "+", i18n::text(self.app_language, "New Workspace")).clicked() {
                    self.add_workspace_from_dialog(ui.ctx());
                }
                ui.add_space(10.0);
                if rail_nav_row(ui, "gear", i18n::text(self.app_language, "Settings")).clicked() {
                    self.set_active_app_dialog(Some(AppDialog::Settings));
                }
            },
        );
    }

    pub(super) fn compact_workspace_rail(&mut self, ui: &mut Ui) {
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(ui.max_rect())
                .layout(Layout::top_down(Align::Center)),
            |ui| {
                ui.set_clip_rect(ui.max_rect());
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), 28.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.add_space((ui.available_width() - 22.0) * 0.5);
                        if rail_header_collapse_button(ui, false).clicked() {
                            self.rail_collapsed = false;
                            self.persist_workspaces();
                        }
                    },
                );
                ui.add_space(18.0);

                let mut rail_action = None;
                for (index, workspace) in self.workspaces.iter().enumerate() {
                    let response = compact_workspace_rail_row(
                        ui,
                        workspace,
                        index == self.active_workspace,
                        &self.repaint_controller,
                    );
                    response.context_menu(|ui| {
                        if ui
                            .button(i18n::text(self.app_language, "Close workspace"))
                            .clicked()
                        {
                            rail_action = Some(WorkspaceRailAction::Close(index));
                            ui.close_menu();
                        }
                    });
                    if response.clicked() {
                        rail_action = Some(WorkspaceRailAction::Switch(index));
                    }
                    ui.add_space(8.0);
                }
                if let Some(action) = rail_action {
                    self.handle_workspace_rail_action(ui.ctx(), action);
                }

                let bottom_height = 28.0 + 10.0 + 28.0;
                ui.add_space((ui.available_height() - bottom_height).max(24.0));
                if compact_rail_nav_button(
                    ui,
                    i18n::text(self.app_language, "New workspace"),
                    |ui, center, color| {
                        paint_plus_icon(ui, center, 6.0, color, 1.4);
                    },
                )
                .clicked()
                {
                    self.add_workspace_from_dialog(ui.ctx());
                }
                ui.add_space(10.0);
                if compact_rail_nav_button(
                    ui,
                    i18n::text(self.app_language, "Settings"),
                    |ui, center, color| {
                        paint_gear_icon(ui, center, color);
                    },
                )
                .clicked()
                {
                    self.set_active_app_dialog(Some(AppDialog::Settings));
                }
            },
        );
    }

    pub(super) fn outline_panel(&mut self, ui: &mut Ui) {
        ui.vertical(|ui| {
            let mut workflow_header_dialog = None;
            let mut init_workflow_root = false;
            ui.horizontal(|ui| {
                let active_tab = self
                    .outline_panel_tabs
                    .get(self.active_workspace)
                    .copied()
                    .unwrap_or_default();
                if ui
                    .selectable_label(
                        active_tab == OutlinePanelTab::Outline,
                        i18n::text(self.app_language, "Outline"),
                    )
                    .clicked()
                {
                    self.set_outline_panel_tab(ui.ctx(), OutlinePanelTab::Outline);
                }
                let workflow_root_missing = self
                    .current_workspace()
                    .and_then(|workspace| {
                        self.workflow_states
                            .get(self.active_workspace)
                            .and_then(|state| state.load_error.as_deref())
                            .map(|error| workflow_root_missing_error(&workspace.path, error))
                    })
                    .unwrap_or(false);
                let workflow_can_add_project = self
                    .workflow_states
                    .get(self.active_workspace)
                    .is_some_and(|state| state.tree.is_some());
                let workflow_response = ui.selectable_label(
                    active_tab == OutlinePanelTab::Workflow,
                    i18n::text(self.app_language, "Work-flow"),
                );
                workflow_response.context_menu(|ui| {
                    if workflow_root_missing
                        && ui
                            .button(i18n::text(self.app_language, "Create root.md"))
                            .clicked()
                    {
                        init_workflow_root = true;
                        ui.close_menu();
                    }
                    if workflow_can_add_project
                        && ui
                            .button(i18n::text(self.app_language, "Add project"))
                            .clicked()
                    {
                        workflow_header_dialog =
                            Some(AppDialog::WorkflowAddProject { key: String::new() });
                        ui.close_menu();
                    }
                });
                if workflow_response.clicked() {
                    self.set_outline_panel_tab(ui.ctx(), OutlinePanelTab::Workflow);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if active_tab == OutlinePanelTab::Outline
                        && subtle_icon_button(ui, i18n::text(self.app_language, "Add")).clicked()
                        && let Some(workspace) = self.current_workspace()
                    {
                        self.set_active_app_dialog(Some(AppDialog::CreateMarkdown {
                            dir: workspace.path.clone(),
                            name: String::new(),
                        }));
                    }
                });
            });
            if let Some(dialog) = workflow_header_dialog {
                self.set_active_app_dialog(Some(dialog));
            } else if init_workflow_root {
                self.request_workflow_mutation(ui.ctx(), WorkflowMutationRequest::InitRoot);
            }
            ui.add_space(8.0);
            match self
                .outline_panel_tabs
                .get(self.active_workspace)
                .copied()
                .unwrap_or_default()
            {
                OutlinePanelTab::Outline => self.outline_tree_panel(ui),
                OutlinePanelTab::Workflow => self.workflow_tree_panel(ui),
            }
        });
    }

    /// 绘制原始 Markdown outline tree。
    fn outline_tree_panel(&mut self, ui: &mut Ui) {
        let tree_height = ui.available_height().max(120.0);
        let tree_rect = ui.available_rect_before_wrap();
        if let Some(rect) = self.outline_tree_rects.get_mut(self.active_workspace) {
            *rect = Some(tree_rect);
        }
        let tree_panel_hovered = ui.ctx().input(|input| {
            input
                .pointer
                .hover_pos()
                .is_some_and(|pos| tree_rect.contains(pos))
        });
        ScrollArea::both()
            .max_height(tree_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let collapse_tree = ui.ctx().input(tree_collapse_shortcut_pressed);
                let toggle_favorites_only =
                    ui.ctx().input(outline_favorite_filter_shortcut_pressed);
                let selected = self
                    .current_workspace()
                    .and_then(|workspace| workspace.selected_file.clone());
                let favorites_only = self
                    .outline_favorites_only
                    .get(self.active_workspace)
                    .copied()
                    .unwrap_or(false);
                let workspace_favorites = self
                    .current_workspace()
                    .map(|workspace| workspace.outline_favorites.clone())
                    .unwrap_or_default();
                let global_favorites = self.global_outline_favorites.clone();
                let language = self.app_language;
                let mut action = None;
                let mut tree_hovered = false;
                let mut rendered_any = false;
                if let Some(workspace) = self.current_workspace_mut() {
                    for node in &mut workspace.outline {
                        rendered_any |= if favorites_only {
                            render_favorite_outline_node(
                                ui,
                                node,
                                0,
                                selected.as_deref(),
                                &workspace_favorites,
                                &global_favorites,
                                &mut action,
                                &mut tree_hovered,
                                language,
                            )
                        } else {
                            render_outline_node(
                                ui,
                                node,
                                0,
                                selected.as_deref(),
                                &workspace_favorites,
                                &global_favorites,
                                &mut action,
                                &mut tree_hovered,
                                language,
                            );
                            true
                        };
                    }
                    if favorites_only && !rendered_any {
                        ui.label(muted(i18n::text(language, "No favorites")));
                    }
                    if tree_hovered && collapse_tree {
                        collapse_outline_to_first_level(&mut workspace.outline);
                    }
                }

                if tree_hovered && collapse_tree {
                    self.suppress_default_agent_input = true;
                }
                if (tree_hovered || tree_panel_hovered) && toggle_favorites_only {
                    self.toggle_outline_favorites_only(ui.ctx());
                    self.suppress_default_agent_input = true;
                    self.suppress_editor_input = true;
                }
                if let Some(action) = action {
                    self.handle_outline_action(ui.ctx(), action);
                }
            });
    }

    /// 绘制 gsdv-spec workflow tree。
    fn workflow_tree_panel(&mut self, ui: &mut Ui) {
        let tree_height = ui.available_height().max(120.0);
        let tree_rect = ui.available_rect_before_wrap();
        if let Some(rect) = self.outline_tree_rects.get_mut(self.active_workspace) {
            *rect = Some(tree_rect);
        }
        let (loading, load_error, tree, selected, collapsed_project_keys) = self
            .workflow_states
            .get(self.active_workspace)
            .map(|state| {
                (
                    state.loading,
                    state.load_error.clone(),
                    state.tree.clone(),
                    state.selected.clone(),
                    state.collapsed_project_keys.clone(),
                )
            })
            .unwrap_or_default();
        let workflow_not_initialized = self
            .current_workspace()
            .and_then(|workspace| {
                load_error
                    .as_deref()
                    .map(|error| workflow_root_missing_error(&workspace.path, error))
            })
            .unwrap_or(false);
        if tree.is_none() && !loading && load_error.is_none() {
            self.request_workflow_tree_refresh(ui.ctx(), self.active_workspace);
        }
        let mut target = None;
        let mut toggled_project_key = None;
        let mut context_dialog = None;
        let mut copy_path = None;
        let mut init_workflow_root = false;
        ScrollArea::both()
            .max_height(tree_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if loading && tree.is_none() {
                    ui.label(muted(i18n::text(self.app_language, "Loading workflow...")));
                }
                if workflow_not_initialized {
                    if primary_action(ui, i18n::text(self.app_language, "Create root.md")).clicked()
                    {
                        init_workflow_root = true;
                    }
                } else if let Some(error) = load_error {
                    ui.colored_label(theme::warning(), error);
                }
                let Some(tree) = tree else {
                    return;
                };
                self.render_workflow_root_node(
                    ui,
                    &tree,
                    selected.as_ref(),
                    &mut target,
                    &mut context_dialog,
                    &mut copy_path,
                );
                for project in &tree.projects {
                    let collapsed = collapsed_project_keys.contains(&project.key);
                    self.render_workflow_project_node(
                        ui,
                        project,
                        collapsed,
                        selected.as_ref(),
                        &mut target,
                        &mut toggled_project_key,
                        &mut context_dialog,
                        &mut copy_path,
                    );
                }
                if tree.projects.is_empty() {
                    let response =
                        ui.label(muted(i18n::text(self.app_language, "No workflow projects")));
                    response.context_menu(|ui| {
                        if ui
                            .button(i18n::text(self.app_language, "Add project"))
                            .clicked()
                        {
                            context_dialog =
                                Some(AppDialog::WorkflowAddProject { key: String::new() });
                            ui.close_menu();
                        }
                    });
                }
            });
        if let Some(dialog) = context_dialog {
            self.set_active_app_dialog(Some(dialog));
        } else if init_workflow_root {
            self.request_workflow_mutation(ui.ctx(), WorkflowMutationRequest::InitRoot);
        } else if let Some(path) = copy_path {
            ui.ctx().copy_text(path);
            self.push_toast(
                i18n::text(self.app_language, "Workflow path copied"),
                theme::success(),
            );
        } else if let Some(key) = toggled_project_key {
            if let Some(state) = self.workflow_states.get_mut(self.active_workspace) {
                if !state.collapsed_project_keys.remove(&key) {
                    state.collapsed_project_keys.insert(key);
                }
            }
            self.request_app_repaint();
        } else if let Some(target) = target {
            self.request_workflow_target(ui.ctx(), target);
        }
    }

    /// 绘制 workspace 级 workflow root.md 节点。
    fn render_workflow_root_node(
        &self,
        ui: &mut Ui,
        tree: &WorkflowTree,
        selected: Option<&WorkflowSelectionTarget>,
        target: &mut Option<WorkflowSelectionTarget>,
        context_dialog: &mut Option<AppDialog>,
        copy_path: &mut Option<String>,
    ) {
        let root_target = WorkflowSelectionTarget::WorkspaceRoot {
            root_path: tree.root_path.clone(),
        };
        let (response, _, _) = workflow_tree_row(
            ui,
            0,
            None,
            None,
            None,
            false,
            "root.md",
            selected == Some(&root_target),
            Some("ROOT"),
        );
        response.context_menu(|ui| {
            if ui
                .button(i18n::text(self.app_language, "Copy path"))
                .clicked()
            {
                *copy_path = Some("root.md".to_string());
                ui.close_menu();
            }
            if ui
                .button(i18n::text(self.app_language, "Open root.md"))
                .clicked()
            {
                *target = Some(root_target.clone());
                ui.close_menu();
            }
            if ui
                .button(i18n::text(self.app_language, "Add project"))
                .clicked()
            {
                *context_dialog = Some(AppDialog::WorkflowAddProject { key: String::new() });
                ui.close_menu();
            }
        });
        if response.clicked() {
            *target = Some(root_target);
        }
    }

    /// 绘制 workflow 项目节点。
    fn render_workflow_project_node(
        &self,
        ui: &mut Ui,
        project: &WorkflowProjectNode,
        collapsed: bool,
        selected: Option<&WorkflowSelectionTarget>,
        target: &mut Option<WorkflowSelectionTarget>,
        toggled_project_key: &mut Option<String>,
        context_dialog: &mut Option<AppDialog>,
        copy_path: &mut Option<String>,
    ) {
        let project_target = WorkflowSelectionTarget::Project {
            root_path: project.root_path.clone(),
        };
        let marker = if collapsed { "▸" } else { "▾" };
        let (response, marker_clicked, _) = workflow_tree_row(
            ui,
            0,
            Some(marker),
            Some(ui.id().with(("workflow-project-marker", &project.key))),
            None,
            false,
            &project.label,
            selected == Some(&project_target),
            Some("PROJECT"),
        );
        response.context_menu(|ui| {
            if ui
                .button(i18n::text(self.app_language, "Copy path"))
                .clicked()
            {
                *copy_path = Some(workflow_project_copy_path(project));
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
                    project_key: project.key.clone(),
                    key: project.key.clone(),
                });
                ui.close_menu();
            }
            if ui
                .button(i18n::text(self.app_language, "Add task"))
                .clicked()
            {
                *context_dialog = Some(AppDialog::WorkflowAddTask {
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
                        project_key: project.key.clone(),
                    },
                });
                ui.close_menu();
            }
        });
        if marker_clicked || response.clicked() {
            *toggled_project_key = Some(project.key.clone());
        }
        if !collapsed {
            for task in &project.tasks {
                self.render_workflow_task_node(
                    ui,
                    project,
                    task,
                    selected,
                    target,
                    context_dialog,
                    copy_path,
                );
            }
        }
    }

    /// 绘制 workflow task 节点。
    fn render_workflow_task_node(
        &self,
        ui: &mut Ui,
        project: &WorkflowProjectNode,
        task: &WorkflowTaskNode,
        selected: Option<&WorkflowSelectionTarget>,
        target: &mut Option<WorkflowSelectionTarget>,
        context_dialog: &mut Option<AppDialog>,
        copy_path: &mut Option<String>,
    ) {
        let task_target = WorkflowSelectionTarget::Task {
            task_path: task.path.clone(),
        };
        let selected = workflow_task_is_selected(task, selected);
        let (response, _, _) =
            workflow_tree_row(ui, 1, None, None, None, false, &task.label, selected, None);
        response.context_menu(|ui| {
            if ui
                .button(i18n::text(self.app_language, "Copy path"))
                .clicked()
            {
                *copy_path = Some(workflow_task_copy_path(project, task));
                ui.close_menu();
            }
            if ui
                .button(i18n::text(self.app_language, "Rename task"))
                .clicked()
            {
                *context_dialog = Some(AppDialog::WorkflowRenameTask {
                    task_path: task.path.clone(),
                    key: workflow_task_dialog_key(task),
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

    /// 绘制 workflow step 节点。
    fn render_workflow_step_node(
        ui: &mut Ui,
        task: &WorkflowTaskNode,
        step: &WorkflowStepNode,
        depth: usize,
        parent_child_base_left: Option<f32>,
        selected_target: Option<&WorkflowSelectionTarget>,
        selected_step_paths: &BTreeSet<Vec<usize>>,
        target: &mut Option<WorkflowSelectionTarget>,
        step_select: &mut Option<WorkflowStepSelect>,
        context_dialog: &mut Option<AppDialog>,
        merge_step_paths: &mut Option<Vec<Vec<usize>>>,
        copy_path: &mut Option<String>,
        path_parts: &[String],
        language: AppLanguage,
    ) {
        let step_target = WorkflowSelectionTarget::Step {
            task_path: task.path.clone(),
            step_path: step.path.clone(),
        };
        let mut step_path_parts = path_parts.to_vec();
        step_path_parts.push(step.title.clone());
        let step_selected = selected_step_paths.contains(&step.path);
        let selected = step_selected || selected_target == Some(&step_target);
        let label_left_override =
            parent_child_base_left.map(|left| left + workflow_step_child_indent(ui));
        let (response, _, _) = workflow_tree_row(
            ui,
            depth,
            None,
            None,
            label_left_override,
            step.checked,
            &step.title,
            selected,
            None,
        );
        response.context_menu(|ui| {
            if step_selected
                && selected_step_paths.len() >= 2
                && ui.button(i18n::text(language, "Merge steps")).clicked()
            {
                *merge_step_paths = Some(selected_step_paths.iter().cloned().collect());
                ui.close_menu();
            }
            if ui.button(i18n::text(language, "Copy path")).clicked() {
                *copy_path = Some(workflow_path_from_parts(&step_path_parts));
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
                *step_select = Some(WorkflowStepSelect::Single {
                    step_path: step.path.clone(),
                });
                *target = Some(step_target);
            }
        }
    }

    pub(super) fn center_panel(&mut self, ui: &mut Ui) {
        if self.workspaces.is_empty() {
            self.empty_workspace_surface(ui);
            return;
        }

        let (current_mode, route, reviewer_mode) = self
            .current_workspace()
            .map(|workspace| {
                (
                    workspace.center_mode,
                    workspace.route,
                    workspace.reviewer_mode,
                )
            })
            .unwrap_or((CenterMode::Agent, Route::Workspace, ReviewerMode::Git));
        let markdown_outline_collapsed = self
            .current_workspace()
            .is_some_and(|workspace| workspace.markdown_outline_collapsed);

        if route == Route::Workspace {
            let mut agent_tab_action = None;
            StripBuilder::new(ui)
                .clip(true)
                .size(Size::exact(30.0))
                .size(Size::remainder())
                .vertical(|mut strip| {
                    strip.strip(|builder| {
                        builder
                            .size(Size::remainder())
                            .size(Size::exact(334.0))
                            .horizontal(|mut strip| {
                                strip.cell(|ui| {
                                    agent_tab_action = workspace_mode_tabs(
                                        ui,
                                        current_mode,
                                        markdown_outline_collapsed,
                                        self.app_language,
                                        |mode| {
                                            if let Some(workspace) = self.current_workspace_mut() {
                                                workspace.center_mode = mode;
                                                self.persist_workspaces();
                                            }
                                        },
                                    );
                                });
                                strip.cell(|ui| {
                                    ui.horizontal(|ui| {
                                        if rest_entry_button(
                                            ui,
                                            self.runtime_settings.pomodoro_enabled,
                                            self.app_language,
                                        )
                                        .clicked()
                                        {
                                            self.pomodoro.start_resting(Instant::now());
                                            self.push_pomodoro_notification(i18n::text_with_arg(
                                                self.app_language,
                                                "Manual rest started for {minutes} minutes",
                                                "{minutes}",
                                                self.runtime_settings
                                                    .pomodoro_rest_minutes
                                                    .to_string(),
                                            ));
                                            self.request_app_repaint();
                                        }
                                        if work_entry_button(
                                            ui,
                                            self.runtime_settings.pomodoro_enabled
                                                && self.pomodoro.phase != PomodoroPhase::Working,
                                            self.app_language,
                                        )
                                        .clicked()
                                        {
                                            self.pomodoro.start_working(Instant::now());
                                            self.push_pomodoro_notification(i18n::text_with_arg(
                                                self.app_language,
                                                "Starting work for {minutes} minutes",
                                                "{minutes}",
                                                self.runtime_settings
                                                    .pomodoro_work_minutes
                                                    .to_string(),
                                            ));
                                            self.request_app_repaint();
                                        }
                                        if help_entry_button(ui, self.app_language).clicked() {
                                            self.set_active_app_dialog(Some(AppDialog::Help));
                                        }
                                    });
                                });
                            });
                    });
                    strip.cell(|ui| match current_mode {
                        CenterMode::Agent => self.agent_surface(ui, current_mode),
                        CenterMode::Terminal => self.agent_surface(ui, current_mode),
                        CenterMode::Editor | CenterMode::Preview => {
                            if self.workflow_task_surface_visible() {
                                self.workflow_task_surface(ui)
                            } else {
                                self.markdown_surface(ui, current_mode)
                            }
                        }
                    });
                });
            if let Some(action) = agent_tab_action {
                self.handle_agent_tab_action(ui.ctx(), action);
            }
        } else {
            self.reviewer_surface(ui, reviewer_mode);
        }
    }

    pub(super) fn agent_surface(&mut self, ui: &mut Ui, mode: CenterMode) {
        ui.vertical(|ui| {
            self.agent_slot_tabs(ui);
            ui.add_space(4.0);
            StripBuilder::new(ui)
                .clip(true)
                .cell_layout(Layout::top_down(Align::Min))
                .size(Size::remainder().at_least(360.0))
                .horizontal(|mut strip| {
                    strip.cell(|ui| match mode {
                        CenterMode::Agent | CenterMode::Terminal => {
                            self.terminal_host_surface(ui, TerminalSurfaceKind::Agent)
                        }
                        CenterMode::Editor | CenterMode::Preview => {}
                    });
                });
        });
    }

    /// Renders per-workspace main/subagent tabs.
    pub(super) fn agent_slot_tabs(&mut self, ui: &mut Ui) {
        let Some(workspace) = self.current_workspace().cloned() else {
            return;
        };
        let active = self.active_agent_slot();
        let mut next_slot = None;
        let mut add_clicked = false;
        let mut remove_subagent = None;
        let mut agent_tab_action = None;
        let workspace_targets = self
            .workspaces
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != self.active_workspace)
            .map(|(index, workspace)| (index, workspace.name.clone()))
            .collect::<Vec<_>>();
        ScrollArea::horizontal()
            .id_salt(("agent-slot-tabs", self.active_workspace))
            .max_height(32.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let main_response = agent_slot_tab_button(
                        ui,
                        "main",
                        workspace.activity,
                        active == AgentSlotId::Main,
                        &self.repaint_controller,
                    );
                    main_response.context_menu(|ui| {
                        agent_slot_context_menu(
                            ui,
                            AgentSlotId::Main,
                            workspace.agent_kind,
                            workspace.agent_model.as_deref(),
                            workspace.agent_model_provider.as_deref(),
                            workspace.agent_effort.as_deref(),
                            workspace.agent_fast_mode,
                            workspace.agent_work_dir.as_deref(),
                            workspace.session_id.as_deref(),
                            &mut agent_tab_action,
                            self.app_language,
                        );
                    });
                    if main_response.clicked() {
                        next_slot = Some(AgentSlotId::Main);
                    }
                    for (subagent_index, subagent) in workspace.subagents.iter().enumerate() {
                        let subagent_id = subagent.id.clone();
                        let slot = AgentSlotId::Subagent(subagent.id.clone());
                        let response = agent_slot_tab_button(
                            ui,
                            &subagent.name,
                            subagent.activity,
                            active == slot,
                            &self.repaint_controller,
                        );
                        response.context_menu(|ui| {
                            agent_slot_context_menu(
                                ui,
                                slot.clone(),
                                subagent.agent_kind,
                                subagent.agent_model.as_deref(),
                                subagent.agent_model_provider.as_deref(),
                                subagent.agent_effort.as_deref(),
                                subagent.agent_fast_mode,
                                subagent.agent_work_dir.as_deref(),
                                subagent.session_id.as_deref(),
                                &mut agent_tab_action,
                                self.app_language,
                            );
                            ui.separator();
                            ui.menu_button(
                                i18n::text(self.app_language, "Move to workspace"),
                                |ui| {
                                    if workspace_targets.is_empty() {
                                        ui.add_enabled(
                                            false,
                                            Button::new(i18n::text(
                                                self.app_language,
                                                "No other workspace",
                                            )),
                                        );
                                    }
                                    for (target_index, target_name) in &workspace_targets {
                                        if ui.button(target_name).clicked() {
                                            agent_tab_action =
                                                Some(AgentTabAction::MoveSubagentToWorkspace {
                                                    id: subagent_id.clone(),
                                                    target_index: *target_index,
                                                });
                                            ui.close_menu();
                                        }
                                    }
                                },
                            );
                            if ui
                                .add_enabled(
                                    subagent_index > 0,
                                    Button::new(i18n::text(self.app_language, "Move left")),
                                )
                                .clicked()
                            {
                                agent_tab_action =
                                    Some(AgentTabAction::MoveSubagentLeft(subagent_id.clone()));
                                ui.close_menu();
                            }
                            if ui
                                .add_enabled(
                                    subagent_index + 1 < workspace.subagents.len(),
                                    Button::new(i18n::text(self.app_language, "Move right")),
                                )
                                .clicked()
                            {
                                agent_tab_action =
                                    Some(AgentTabAction::MoveSubagentRight(subagent_id.clone()));
                                ui.close_menu();
                            }
                            if ui
                                .add_enabled(
                                    subagent_index > 0,
                                    Button::new(i18n::text(self.app_language, "Move to head")),
                                )
                                .clicked()
                            {
                                agent_tab_action =
                                    Some(AgentTabAction::MoveSubagentToHead(subagent_id.clone()));
                                ui.close_menu();
                            }
                            if ui
                                .add_enabled(
                                    subagent_index + 1 < workspace.subagents.len(),
                                    Button::new(i18n::text(self.app_language, "Move to tail")),
                                )
                                .clicked()
                            {
                                agent_tab_action =
                                    Some(AgentTabAction::MoveSubagentToTail(subagent_id.clone()));
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button(i18n::text(self.app_language, "Remove")).clicked() {
                                remove_subagent = Some(subagent.id.clone());
                                ui.close_menu();
                            }
                        });
                        if response.clicked() {
                            next_slot = Some(slot);
                        }
                    }
                    if ui.add_sized([30.0, 24.0], Button::new("+")).clicked() {
                        add_clicked = true;
                    }
                });
            });
        if let Some(slot) = next_slot {
            if let Some(active) = self.active_agent_slots.get_mut(self.active_workspace) {
                *active = slot;
            }
            self.request_app_repaint();
        }
        if add_clicked {
            self.set_active_app_dialog(Some(AppDialog::AddSubagent {
                index: self.active_workspace,
                name: String::new(),
                agent_kind: workspace.agent_kind,
                agent_model: String::new(),
                agent_model_provider: String::new(),
                model_providers: data::load_codex_model_provider_names(),
                agent_effort: String::new(),
                agent_fast_mode: None,
                agent_work_dir: String::new(),
                session_id: String::new(),
            }));
        }
        if let Some(id) = remove_subagent {
            self.remove_subagent(ui.ctx(), self.active_workspace, &id);
        }
        if let Some(action) = agent_tab_action {
            self.handle_agent_tab_action(ui.ctx(), action);
        }
    }

    /// 绘制 workflow task 专属工作台。
    pub(super) fn workflow_task_surface(&mut self, ui: &mut Ui) {
        let Some((project_label, task)) = self.current_workflow_task_context() else {
            empty_document_panel(
                ui,
                i18n::text(self.app_language, "Select a workflow task."),
                self.app_language,
            );
            return;
        };
        let selected = self
            .workflow_states
            .get(self.active_workspace)
            .and_then(|state| state.selected.clone());
        let selected_step_paths = self
            .workflow_states
            .get(self.active_workspace)
            .filter(|state| state.step_selection_task_path.as_deref() == Some(task.path.as_path()))
            .map(|state| state.selected_step_paths.clone())
            .unwrap_or_default();
        let mut target = None;
        let mut step_select = None;
        let mut context_dialog = None;
        let mut merge_step_paths = None;
        let available = ui.available_size();
        if available.x <= 1.0 || available.y <= 1.0 {
            return;
        }
        let mut copy_path = None;
        let left_width = (ui.available_width() * 0.32).clamp(220.0, 360.0);
        let desc_height = (available.y * 0.28)
            .clamp(110.0, 220.0)
            .min((available.y - 160.0).max(80.0));
        StripBuilder::new(ui)
            .size(Size::exact(desc_height))
            .size(Size::exact(10.0))
            .size(Size::remainder())
            .vertical(|mut strip| {
                strip.cell(|ui| {
                    self.workflow_task_desc_editor_surface(ui);
                });
                strip.empty();
                strip.strip(|builder| {
                    builder
                        .size(Size::exact(left_width))
                        .size(Size::exact(10.0))
                        .size(Size::remainder())
                        .horizontal(|mut strip| {
                            strip.cell(|ui| {
                                workflow_task_step_tree_panel(
                                    ui,
                                    &task,
                                    selected.as_ref(),
                                    &selected_step_paths,
                                    &mut target,
                                    &mut step_select,
                                    &mut context_dialog,
                                    &mut merge_step_paths,
                                    &mut copy_path,
                                    &project_label,
                                    self.app_language,
                                );
                            });
                            strip.empty();
                            strip.cell(|ui| {
                                self.workflow_step_editor_surface(ui);
                            });
                        });
                });
            });
        if let Some(dialog) = context_dialog {
            self.set_active_app_dialog(Some(dialog));
        } else if let Some(step_paths) = merge_step_paths {
            let title = workflow_merge_default_title(&task, &step_paths);
            self.set_active_app_dialog(Some(AppDialog::WorkflowMergeSteps {
                task_path: task.path.clone(),
                step_paths,
                title,
            }));
        } else if let Some(path) = copy_path {
            ui.ctx().copy_text(path);
            self.push_toast(
                i18n::text(self.app_language, "Workflow path copied"),
                theme::success(),
            );
        } else if let Some(target) = target {
            self.request_workflow_target(ui.ctx(), target);
        } else if let Some(step_select) = step_select {
            self.apply_workflow_step_selection(ui.ctx(), &task, step_select);
        }
    }

    /// 应用 task 工作台里的 step 单选或 Shift 范围选择。
    fn apply_workflow_step_selection(
        &mut self,
        ctx: &egui::Context,
        task: &WorkflowTaskNode,
        select: WorkflowStepSelect,
    ) {
        let Some(state) = self.workflow_states.get_mut(self.active_workspace) else {
            return;
        };
        match select {
            WorkflowStepSelect::Single { step_path } => {
                state.selected_step_paths.clear();
                state.selected_step_paths.insert(step_path.clone());
                state.step_selection_anchor = Some(step_path);
                state.step_selection_task_path = Some(task.path.clone());
            }
            WorkflowStepSelect::Range { step_path } => {
                let anchor = state
                    .step_selection_task_path
                    .as_deref()
                    .filter(|path| *path == task.path.as_path())
                    .and_then(|_| state.step_selection_anchor.clone())
                    .unwrap_or_else(|| step_path.clone());
                state.selected_step_paths = workflow_step_path_range(task, &anchor, &step_path);
                state.step_selection_anchor = Some(anchor);
                state.step_selection_task_path = Some(task.path.clone());
            }
            WorkflowStepSelect::Toggle { step_path } => {
                if state.step_selection_task_path.as_deref() != Some(task.path.as_path()) {
                    state.selected_step_paths.clear();
                }
                if !state.selected_step_paths.remove(&step_path) {
                    state.selected_step_paths.insert(step_path.clone());
                }
                state.step_selection_anchor = Some(step_path);
                state.step_selection_task_path = Some(task.path.clone());
            }
        }
        self.request_app_repaint();
    }

    /// 返回当前 workflow task 工作台对应的项目名和 task 节点。
    fn current_workflow_task_context(&self) -> Option<(String, WorkflowTaskNode)> {
        let state = self.workflow_states.get(self.active_workspace)?;
        let selected = state.selected.as_ref()?;
        let task_path = match selected {
            WorkflowSelectionTarget::Task { task_path }
            | WorkflowSelectionTarget::Step { task_path, .. } => task_path,
            WorkflowSelectionTarget::WorkspaceRoot { .. }
            | WorkflowSelectionTarget::Project { .. } => {
                return None;
            }
        };
        state.tree.as_ref()?.projects.iter().find_map(|project| {
            project
                .tasks
                .iter()
                .find(|task| task.path == *task_path)
                .cloned()
                .map(|task| (project.label.clone(), task))
        })
    }

    /// 绘制 workflow task 说明编辑器。
    pub(super) fn workflow_task_desc_editor_surface(&mut self, ui: &mut Ui) {
        let Some(editor) = self
            .workflow_states
            .get(self.active_workspace)
            .and_then(|state| state.task_editor.as_ref())
            .cloned()
        else {
            workflow_empty_editor_message(
                ui,
                i18n::text(self.app_language, "Select a workflow task."),
            );
            return;
        };
        let editor_font = effective_editor_font_id(&self.font_settings);
        let interactive = self.center_surface_accepts_keyboard_input();
        let mut next_task_text = editor.task_text.clone();
        let save_error = editor.save_error.clone();
        let preview = self
            .workflow_states
            .get(self.active_workspace)
            .is_some_and(|state| state.preview_fragments);
        let available = ui.available_size();
        if available.x <= 1.0 || available.y <= 1.0 {
            return;
        }
        if let Some(error) = save_error {
            document_error_strip(ui, &error);
            ui.add_space(8.0);
        }
        if preview {
            workflow_fragment_preview(
                ui,
                (
                    "workflow-task-desc-preview",
                    self.active_workspace,
                    editor.task_path.clone(),
                ),
                &next_task_text,
                self.theme_mode,
                ui.available_height().max(1.0),
            );
        } else {
            let ime_dirty = workflow_fragment_editor(
                ui,
                (
                    "workflow-task-desc-editor",
                    self.active_workspace,
                    editor.task_path.clone(),
                ),
                &mut next_task_text,
                editor_font,
                interactive,
                ui.available_height().max(1.0),
            );
            if ime_dirty {
                self.pending_input_repaint = true;
            }
            if let Some(state) = self.workflow_states.get_mut(self.active_workspace)
                && let Some(current) = state.task_editor.as_mut()
            {
                current.task_text = next_task_text;
            }
        }
    }

    /// 绘制 workflow step 片段编辑器。
    pub(super) fn workflow_step_editor_surface(&mut self, ui: &mut Ui) {
        let Some(editor) = self
            .workflow_states
            .get(self.active_workspace)
            .and_then(|state| state.editor.as_ref())
            .cloned()
        else {
            workflow_empty_editor_message(
                ui,
                i18n::text(self.app_language, "Select a workflow step to edit."),
            );
            return;
        };
        let editor_font = effective_editor_font_id(&self.font_settings);
        let interactive = self.center_surface_accepts_keyboard_input();
        let mut next_step_text = editor.step_text.clone();
        let save_error = editor.save_error.clone();
        let preview = self
            .workflow_states
            .get(self.active_workspace)
            .is_some_and(|state| state.preview_fragments);
        let available = ui.available_size();
        if available.x <= 1.0 || available.y <= 1.0 {
            return;
        }
        if let Some(error) = save_error {
            document_error_strip(ui, &error);
            ui.add_space(8.0);
        }
        if preview {
            workflow_fragment_preview(
                ui,
                (
                    "workflow-step-preview",
                    self.active_workspace,
                    editor.task_path.clone(),
                    editor.step_path.clone(),
                ),
                &next_step_text,
                self.theme_mode,
                ui.available_height().max(1.0),
            );
        } else {
            let ime_dirty = workflow_fragment_editor(
                ui,
                (
                    "workflow-step-editor",
                    self.active_workspace,
                    editor.task_path.clone(),
                    editor.step_path.clone(),
                ),
                &mut next_step_text,
                editor_font,
                interactive,
                ui.available_height().max(1.0),
            );
            if ime_dirty {
                self.pending_input_repaint = true;
            }
            if let Some(state) = self.workflow_states.get_mut(self.active_workspace)
                && let Some(current) = state.editor.as_mut()
            {
                current.step_text = next_step_text;
            }
        }
    }

    pub(super) fn workspace_terminal_overlay(&mut self, ctx: &egui::Context) {
        if !self.workspace_terminal_drawer_is_open() {
            return;
        }

        let screen = ctx.screen_rect();
        let rail_width = if self.rail_collapsed {
            COMPACT_WORKSPACE_RAIL_WIDTH
        } else {
            WORKSPACE_RAIL_WIDTH
        }
        .min(screen.width());
        let width = (screen.width() - rail_width).max(1.0);
        let height = (screen.height() - BOTTOM_BAR_HEIGHT).max(1.0);
        let pos = egui::pos2(screen.right() - width, screen.top());
        let size = Vec2::new(width, height);

        egui::Area::new("workspace_terminal_overlay".into())
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                ui.set_min_size(size);
                ui.set_max_size(size);
                Frame::new()
                    .fill(theme::bg())
                    .stroke(Stroke::new(1.0, theme::border()))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_min_size(Vec2::new(
                            (size.x - 20.0).max(1.0),
                            (size.y - 20.0).max(1.0),
                        ));
                        self.workspace_terminal_drawer(ui);
                    });
            });
    }

    pub(super) fn reviewer_helix_overlay(&mut self, ctx: &egui::Context) {
        if !self.reviewer_helix_drawer_is_open() {
            return;
        }

        let screen = ctx.screen_rect();
        let rail_width = if self.rail_collapsed {
            COMPACT_WORKSPACE_RAIL_WIDTH
        } else {
            WORKSPACE_RAIL_WIDTH
        }
        .min(screen.width());
        let width = (screen.width() - rail_width).max(1.0);
        let height = (screen.height() - BOTTOM_BAR_HEIGHT).max(1.0);
        let pos = egui::pos2(screen.right() - width, screen.top());
        let size = Vec2::new(width, height);

        egui::Area::new("reviewer_helix_overlay".into())
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                ui.set_min_size(size);
                ui.set_max_size(size);
                Frame::new()
                    .fill(theme::bg())
                    .stroke(Stroke::new(1.0, theme::border()))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_min_size(Vec2::new(
                            (size.x - 20.0).max(1.0),
                            (size.y - 20.0).max(1.0),
                        ));
                        self.reviewer_helix_drawer(ui);
                    });
            });
    }

    pub(super) fn notification_overlay(&mut self, ctx: &egui::Context) {
        if !self.notifications.open {
            return;
        }

        let screen = ctx.screen_rect();
        let rail_width = if self.rail_collapsed {
            COMPACT_WORKSPACE_RAIL_WIDTH
        } else {
            WORKSPACE_RAIL_WIDTH
        }
        .min(screen.width());
        let width = (screen.width() - rail_width).max(1.0);
        let height = (screen.height() - BOTTOM_BAR_HEIGHT).max(1.0);
        let pos = egui::pos2(screen.right() - width, screen.top());
        let size = Vec2::new(width, height);

        egui::Area::new("notification_overlay".into())
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                ui.set_min_size(size);
                ui.set_max_size(size);
                Frame::new()
                    .fill(theme::bg())
                    .stroke(Stroke::new(1.0, theme::border()))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_min_size(Vec2::new(
                            (size.x - 20.0).max(1.0),
                            (size.y - 20.0).max(1.0),
                        ));
                        self.notification_drawer(ui);
                    });
            });
    }

    pub(super) fn workspace_terminal_drawer(&mut self, ui: &mut Ui) {
        ui.set_min_width(ui.available_width());
        self.terminal_host_surface(ui, TerminalSurfaceKind::Workspace);
    }

    pub(super) fn reviewer_helix_drawer(&mut self, ui: &mut Ui) {
        ui.set_min_width(ui.available_width());
        self.helix_host_surface(ui);
    }

    pub(super) fn notification_drawer(&mut self, ui: &mut Ui) {
        ui.set_min_width(ui.available_width());
        ui.columns(2, |columns| {
            self.notification_lines_panel(&mut columns[0]);
            self.workspace_memo_panel(&mut columns[1]);
        });
    }

    /// Renders the existing notification stream in the left drawer column.
    pub(super) fn notification_lines_panel(&mut self, ui: &mut Ui) {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(i18n::text(self.app_language, "Notifications"))
                        .strong()
                        .color(theme::text()),
                );
                ui.label(
                    RichText::new(i18n::text_with_arg(
                        self.app_language,
                        "{count} lines",
                        "{count}",
                        self.notifications.lines.len().to_string(),
                    ))
                    .size(12.0)
                    .color(theme::muted()),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if secondary_action(ui, i18n::text(self.app_language, "Clear")).clicked() {
                        self.notifications.clear();
                    }
                });
            });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            let scroll_to_bottom = self.notifications.scroll_to_bottom;
            ScrollArea::vertical()
                .id_salt("notification-lines")
                .max_height(ui.available_height().max(1.0))
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if self.notifications.lines.is_empty() {
                        ui.label(muted(i18n::text(self.app_language, "No notifications")));
                    } else {
                        for line in &self.notifications.lines {
                            ui.label(
                                RichText::new(line)
                                    .font(notification_font_id(&effective_font_surface_settings(
                                        &self.font_settings,
                                        &self.font_settings.terminal,
                                    )))
                                    .color(theme::notification_text()),
                            );
                        }
                    }
                    if scroll_to_bottom {
                        ui.scroll_to_cursor(Some(Align::BOTTOM));
                    }
                });
            self.notifications.scroll_to_bottom = false;
        });
    }

    /// Renders and persists the workspace-scoped Markdown memo editor.
    pub(super) fn workspace_memo_panel(&mut self, ui: &mut Ui) {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(i18n::text(self.app_language, "Memo"))
                        .strong()
                        .color(theme::text()),
                );
            });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            let editor_font = effective_editor_font_id(&self.font_settings);
            let workspace_index = self.active_workspace;
            let Some(workspace) = self.current_workspace_mut() else {
                empty_document_panel(
                    ui,
                    i18n::text(self.app_language, "No active workspace."),
                    self.app_language,
                );
                return;
            };
            let editor_width = ui.available_width().max(80.0);
            let editor_height = ui.available_height().max(120.0);
            let editor_id = (
                "workspace-memo-editor",
                workspace_index,
                workspace.path.clone(),
            );
            let mut memo_changed = false;
            let mut memo_ime_repaint = false;

            ScrollArea::vertical()
                .id_salt(("workspace-memo-scroll", workspace_index))
                .max_width(editor_width)
                .max_height(editor_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(editor_width);
                    ui.set_max_width(editor_width);
                    // 触发条件：中文 IME commit 让 TextEdit 状态变化但文本没变。
                    // 不能只看 response.changed：它不等价于已插入文本。
                    // 防止 memo 中文提交被 fallback 误判为已处理。
                    let text_before_ime =
                        Self::markdown_editor_has_ime_commit(ui).then(|| workspace.memo.clone());
                    let gutter_width = editor_line_number_gutter_width(&workspace.memo);
                    let text_width = (editor_width - gutter_width - 8.0).max(80.0);
                    let mut editor_output = ui
                        .horizontal_top(|ui| {
                            let gutter_rect = reserve_editor_line_number_gutter(ui, gutter_width);
                            let editor_output = egui::TextEdit::multiline(&mut workspace.memo)
                                .id_salt(editor_id)
                                .font(editor_font.clone())
                                .text_color(theme::markdown_text())
                                .desired_width(text_width)
                                .desired_rows(24)
                                .lock_focus(true)
                                .show(ui);
                            stabilize_text_edit_ime_output(ui, &editor_output, &editor_font);
                            paint_editor_line_number_gutter(
                                ui,
                                gutter_rect,
                                &editor_output,
                                editor_font,
                            );
                            editor_output
                        })
                        .inner;
                    let text_changed = editor_output.response.changed();
                    let text_unchanged_after_edit = text_before_ime
                        .as_deref()
                        .is_some_and(|before| before == workspace.memo);
                    let memo_ime_dirty = Self::apply_markdown_editor_ime_commit_fallback(
                        ui,
                        &mut editor_output.state,
                        &editor_output.response,
                        text_unchanged_after_edit,
                        &mut workspace.memo,
                    );
                    memo_changed = text_changed || memo_ime_dirty;
                    if memo_ime_dirty {
                        memo_ime_repaint = true;
                    }
                });

            if memo_changed {
                self.pending_memo_saves.insert(workspace_index);
            }
            if memo_ime_repaint {
                self.pending_input_repaint = true;
            }
        });
    }

    pub(super) fn empty_workspace_surface(&mut self, ui: &mut Ui) {
        let rect = ui.available_rect_before_wrap();
        let (_, _) = ui.allocate_exact_size(rect.size(), Sense::hover());
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, theme::bg());

        let panel_width = rect.width().min(620.0).max(320.0);
        let panel_height = 232.0_f32.min(rect.height().max(1.0));
        let panel_top = rect.top() + ((rect.height() - panel_height) * 0.38).max(24.0);
        let panel_rect = Rect::from_center_size(
            egui::pos2(rect.center().x, panel_top + panel_height * 0.5),
            Vec2::new(panel_width, panel_height),
        );
        let radius = CornerRadius::same(theme::RADIUS_LG);
        ui.painter()
            .rect_filled(panel_rect, radius, theme::surface_elevated());
        ui.painter().rect_stroke(
            panel_rect,
            radius,
            Stroke::new(1.0, theme::border()),
            egui::StrokeKind::Inside,
        );
        let accent_rect = Rect::from_min_size(panel_rect.min, Vec2::new(panel_rect.width(), 3.0));
        ui.painter()
            .rect_filled(accent_rect, CornerRadius::same(2), theme::primary());

        let icon_rect =
            Rect::from_min_size(panel_rect.min + egui::vec2(28.0, 30.0), Vec2::splat(42.0));
        ui.painter().rect_filled(
            icon_rect,
            CornerRadius::same(theme::RADIUS_MD),
            theme::primary_soft(),
        );
        ui.painter().rect_stroke(
            icon_rect,
            CornerRadius::same(theme::RADIUS_MD),
            Stroke::new(1.0, theme::primary_border()),
            egui::StrokeKind::Inside,
        );
        paint_plus_icon(ui, icon_rect.center(), 8.0, theme::primary(), 2.0);

        let content_rect = Rect::from_min_max(
            egui::pos2(icon_rect.right() + 18.0, panel_rect.top() + 28.0),
            egui::pos2(panel_rect.right() - 28.0, panel_rect.bottom() - 24.0),
        );
        let mut content_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(content_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        content_ui.set_clip_rect(content_rect);
        content_ui.label(
            RichText::new(i18n::text(self.app_language, "Open a workspace"))
                .strong()
                .size(22.0)
                .color(theme::text()),
        );
        content_ui.add_space(8.0);
        content_ui.label(muted(i18n::text(
            self.app_language,
            "Add a project directory to start an agent session, browse Markdown, or inspect reviewer data.",
        )));
        content_ui.add_space(20.0);
        content_ui.horizontal(|ui| {
            if primary_action(ui, i18n::text(self.app_language, "Add workspace")).clicked() {
                self.add_workspace_from_dialog(ui.ctx());
            }
            if secondary_action(ui, i18n::text(self.app_language, "Help")).clicked() {
                self.set_active_app_dialog(Some(AppDialog::Help));
            }
        });
    }
}

/// 绘制 footer 上的工作剩余进度条。
fn pomodoro_work_remaining_footer_bar(ui: &mut Ui, remaining_fraction: f32, warning: bool) {
    let size = Vec2::new(132.0, 8.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let radius = CornerRadius::same(3);
    let fill_width = rect.width() * remaining_fraction.clamp(0.0, 1.0);
    let fill_rect = Rect::from_min_size(rect.min, Vec2::new(fill_width, rect.height()));
    let color = if warning {
        theme::warning()
    } else {
        theme::success()
    };
    ui.painter()
        .rect_filled(rect, radius, theme::surface_elevated());
    if fill_width > 0.5 {
        ui.painter().rect_filled(fill_rect, radius, color);
    }
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, theme::border()),
        egui::StrokeKind::Inside,
    );
}

/// 绘制 workflow 单个片段 editor。
fn workflow_fragment_editor(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash + Clone,
    text: &mut String,
    editor_font: egui::FontId,
    interactive: bool,
    height: f32,
) -> bool {
    let width = ui.available_width().max(120.0);
    let height = height.min(ui.available_height().max(1.0)).max(1.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let radius = CornerRadius::same(6);
    ui.painter().rect_filled(rect, radius, theme::surface());
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, theme::border()),
        egui::StrokeKind::Inside,
    );
    let inner_rect = rect.shrink(8.0);
    if inner_rect.width() <= 1.0 || inner_rect.height() <= 1.0 {
        return false;
    }
    let mut editor_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("workflow-fragment-inner", id_salt.clone()))
            .max_rect(inner_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    editor_ui.set_clip_rect(inner_rect);
    let text_before_ime =
        GsdvGuiApp::markdown_editor_has_ime_commit(&editor_ui).then(|| text.clone());
    let inner_width = inner_rect.width();
    let inner_height = inner_rect.height();
    let row_height = ui.fonts(|fonts| fonts.row_height(&editor_font)).max(1.0);
    let desired_rows = (inner_height / row_height).floor().max(1.0) as usize;
    let mut ime_dirty = false;
    ScrollArea::both()
        .id_salt(("workflow-fragment-scroll", id_salt.clone()))
        .max_width(inner_width)
        .max_height(inner_height)
        .auto_shrink([false, false])
        .show(&mut editor_ui, |ui| {
            ui.set_min_width(inner_width);
            ui.set_max_width(inner_width);
            let mut output = egui::TextEdit::multiline(text)
                .id_salt(id_salt)
                .font(editor_font.clone())
                .text_color(theme::markdown_text())
                .desired_width(inner_width)
                .desired_rows(desired_rows)
                .lock_focus(true)
                .interactive(interactive)
                .frame(false)
                .show(ui);
            stabilize_text_edit_ime_output(ui, &output, &editor_font);
            let text_unchanged_after_edit = text_before_ime
                .as_deref()
                .is_some_and(|before| before == text.as_str());
            ime_dirty = GsdvGuiApp::apply_markdown_editor_ime_commit_fallback(
                ui,
                &mut output.state,
                &output.response,
                text_unchanged_after_edit,
                text,
            );
        });
    ime_dirty
}

/// 绘制 workflow 单个片段的 Markdown preview。
fn workflow_fragment_preview(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash + Clone,
    text: &str,
    theme_mode: theme::ThemeMode,
    height: f32,
) {
    let width = ui.available_width().max(120.0);
    let height = height.min(ui.available_height().max(1.0)).max(1.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let radius = CornerRadius::same(6);
    ui.painter().rect_filled(rect, radius, theme::surface());
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, theme::border()),
        egui::StrokeKind::Inside,
    );
    let inner_rect = rect.shrink(12.0);
    if inner_rect.width() <= 1.0 || inner_rect.height() <= 1.0 {
        return;
    }
    let mut preview_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("workflow-fragment-preview-inner", id_salt.clone()))
            .max_rect(inner_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    preview_ui.set_clip_rect(inner_rect);
    preview_ui.scope(|ui| {
        ui.set_style(theme::markdown_style(theme_mode));
        ScrollArea::both()
            .id_salt(("workflow-fragment-preview-scroll", id_salt))
            .max_width(inner_rect.width())
            .max_height(inner_rect.height())
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_width(inner_rect.width());
                ui.set_max_width(inner_rect.width());
                if text.trim().is_empty() {
                    ui.label(muted(""));
                } else {
                    markdown_preview::render(ui, text, inner_rect.width(), theme_mode);
                }
            });
    });
}

/// 绘制 workflow editor 的轻量空态，不复用 Markdown 主视图卡片。
fn workflow_empty_editor_message(ui: &mut Ui, message: &str) {
    ui.centered_and_justified(|ui| {
        ui.label(muted(message));
    });
}

/// 绘制 task 工作台左侧 step tree。
fn workflow_task_step_tree_panel(
    ui: &mut Ui,
    task: &WorkflowTaskNode,
    selected: Option<&WorkflowSelectionTarget>,
    selected_step_paths: &BTreeSet<Vec<usize>>,
    target: &mut Option<WorkflowSelectionTarget>,
    step_select: &mut Option<WorkflowStepSelect>,
    context_dialog: &mut Option<AppDialog>,
    merge_step_paths: &mut Option<Vec<Vec<usize>>>,
    copy_path: &mut Option<String>,
    project_label: &str,
    language: AppLanguage,
) {
    let path_parts = vec![project_label.to_string(), task.label.clone()];
    ScrollArea::vertical()
        .id_salt(("workflow-task-step-tree", task.path.clone()))
        .max_height(ui.available_height().max(1.0))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if task.steps.is_empty() {
                ui.label(muted(i18n::text(language, "No steps")));
                return;
            }
            for step in &task.steps {
                GsdvGuiApp::render_workflow_step_node(
                    ui,
                    task,
                    step,
                    0,
                    None,
                    selected,
                    selected_step_paths,
                    target,
                    step_select,
                    context_dialog,
                    merge_step_paths,
                    copy_path,
                    &path_parts,
                    language,
                );
            }
        });
}

/// task 工作台中的 step 选择动作。
enum WorkflowStepSelect {
    /// 普通点击，只保留当前 step。
    Single {
        /// 被点击的 step 路径。
        step_path: Vec<usize>,
    },
    /// Shift 点击，从锚点到当前 step 做范围选择。
    Range {
        /// 被点击的 step 路径。
        step_path: Vec<usize>,
    },
    /// Ctrl 点击，切换当前 step 是否被选中。
    Toggle {
        /// 被点击的 step 路径。
        step_path: Vec<usize>,
    },
}

/// 返回两个 step 路径之间的闭区间路径集合。
fn workflow_step_path_range(
    task: &WorkflowTaskNode,
    anchor: &[usize],
    clicked: &[usize],
) -> BTreeSet<Vec<usize>> {
    let anchor_index = task.steps.iter().position(|step| step.path == anchor);
    let clicked_index = task.steps.iter().position(|step| step.path == clicked);
    let (Some(anchor_index), Some(clicked_index)) = (anchor_index, clicked_index) else {
        return BTreeSet::from([clicked.to_vec()]);
    };
    let (start, end) = if anchor_index <= clicked_index {
        (anchor_index, clicked_index)
    } else {
        (clicked_index, anchor_index)
    };
    task.steps[start..=end]
        .iter()
        .map(|step| step.path.clone())
        .collect()
}

/// 返回合并弹窗默认标题。
fn workflow_merge_default_title(task: &WorkflowTaskNode, step_paths: &[Vec<usize>]) -> String {
    step_paths
        .first()
        .and_then(|path| task.steps.iter().find(|step| step.path == *path))
        .map(|step| step.title.clone())
        .unwrap_or_else(|| "merged-step".to_string())
}

/// 绘制 workflow tree 行。
fn workflow_tree_row(
    ui: &mut Ui,
    depth: usize,
    marker: Option<&str>,
    marker_id: Option<egui::Id>,
    label_left_override: Option<f32>,
    done: bool,
    label: &str,
    selected: bool,
    badge: Option<&str>,
) -> (egui::Response, bool, f32) {
    let row_height = 22.0;
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
    let center_y = rect.center().y;
    let indent = depth as f32 * 12.0;
    let marker_left = rect.left() + 6.0 + indent;
    let marker_clicked = if let Some(marker) = marker {
        let marker_rect = egui::Rect::from_min_size(
            egui::pos2(marker_left - 4.0, rect.top()),
            Vec2::new(16.0, row_height),
        );
        let marker_response = marker_id.map(|id| ui.interact(marker_rect, id, Sense::click()));
        ui.painter().text(
            egui::pos2(marker_left, center_y),
            Align2::LEFT_CENTER,
            marker,
            egui::TextStyle::Small.resolve(ui.style()),
            theme::muted(),
        );
        marker_response.is_some_and(|response| response.clicked())
    } else {
        false
    };
    let mut label_left = label_left_override.unwrap_or_else(|| {
        if marker.is_some() {
            marker_left + 16.0
        } else {
            marker_left
        }
    });
    let child_base_left = label_left;
    if let Some(badge) = badge {
        let badge_font = egui::TextStyle::Small.resolve(ui.style());
        let badge_color = if done {
            theme::success()
        } else {
            theme::muted()
        };
        let badge_width = ui
            .painter()
            .layout_no_wrap(badge.to_string(), badge_font.clone(), badge_color)
            .rect
            .width();
        let space_width = ui
            .painter()
            .layout_no_wrap(" ".to_string(), badge_font.clone(), badge_color)
            .rect
            .width();
        ui.painter().text(
            egui::pos2(label_left, center_y),
            Align2::LEFT_CENTER,
            badge,
            badge_font,
            badge_color,
        );
        label_left += badge_width + space_width;
    }
    ui.painter().text(
        egui::pos2(label_left, center_y),
        Align2::LEFT_CENTER,
        label,
        egui::TextStyle::Body.resolve(ui.style()),
        if done {
            theme::success()
        } else {
            theme::list_text()
        },
    );
    if selected {
        ui.painter().circle_filled(
            egui::pos2(rect.right() - 14.0, center_y),
            3.5,
            theme::primary(),
        );
    }
    (response, marker_clicked, child_base_left)
}

/// 计算子 step 相对父级基准的两个空格缩进。
fn workflow_step_child_indent(ui: &Ui) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.painter()
        .layout_no_wrap("  ".to_string(), font, theme::list_text())
        .rect
        .width()
}

/// 返回 task 重命名弹窗里显示和编辑的 key。
fn workflow_task_dialog_key(task: &WorkflowTaskNode) -> String {
    let stem = task
        .path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| task.label.clone());
    stem.strip_prefix("task-")
        .map(str::to_string)
        .unwrap_or(stem)
}

/// 返回 workflow project 的可复制路径。
fn workflow_project_copy_path(project: &WorkflowProjectNode) -> String {
    project.label.clone()
}

/// 返回 workflow task 的可复制路径。
fn workflow_task_copy_path(project: &WorkflowProjectNode, task: &WorkflowTaskNode) -> String {
    workflow_path_from_parts(&[project.label.clone(), task.label.clone()])
}

/// 将 workflow 层级拼成右键复制用的路径。
fn workflow_path_from_parts(parts: &[String]) -> String {
    parts.join(" > ")
}

/// 判断 task 是否是当前 workflow 选择。
fn workflow_task_is_selected(
    task: &WorkflowTaskNode,
    selected: Option<&WorkflowSelectionTarget>,
) -> bool {
    match selected {
        Some(WorkflowSelectionTarget::Task { task_path })
        | Some(WorkflowSelectionTarget::Step { task_path, .. }) => task_path == &task.path,
        Some(WorkflowSelectionTarget::WorkspaceRoot { .. })
        | Some(WorkflowSelectionTarget::Project { .. })
        | None => false,
    }
}

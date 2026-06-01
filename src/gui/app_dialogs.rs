//! 应用级弹窗和设置页业务。
//!
//! 这个模块是 `app.rs` 的子模块，专门承接 modal、settings、help、about
//! 等界面业务；主 app 文件只负责状态拥有、事件调度和顶层渲染编排。

use super::*;

impl GsdvGuiApp {
    /// Draws the non-modal Agent input translation popup above the composer.
    pub(super) fn agent_input_translation_popup(&mut self, ctx: &egui::Context) {
        let Some(popup) = self.agent_input_translation_popup.clone() else {
            return;
        };
        let Some(anchor_rect) = self.active_agent_terminal_rect else {
            return;
        };
        let popup_lines = agent_translation_popup_lines(&popup.message, anchor_rect.width());
        let pos = egui::pos2(anchor_rect.left(), anchor_rect.top() - 8.0);
        egui::Area::new("agent_input_translation_popup".into())
            .order(egui::Order::Foreground)
            .pivot(Align2::LEFT_BOTTOM)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                ui.set_width(anchor_rect.width());
                Frame::new()
                    .fill(theme::bg())
                    .stroke(Stroke::new(1.0, theme::border()))
                    .corner_radius(CornerRadius::same(theme::RADIUS_MD))
                    .inner_margin(Margin::symmetric(12, 9))
                    .show(ui, |ui| {
                        ui.set_width(anchor_rect.width() - 24.0);
                        for line in &popup_lines {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(line)
                                        .monospace()
                                        .size(13.0)
                                        .color(theme::text()),
                                )
                                .truncate(),
                            );
                        }
                    });
            });
    }

    pub(super) fn active_app_dialog(&self) -> Option<AppDialog> {
        if self.workspaces.is_empty() {
            return self.global_app_dialog.clone();
        }
        self.app_dialogs
            .get(self.active_workspace)
            .and_then(Clone::clone)
    }

    pub(super) fn set_active_app_dialog(&mut self, dialog: Option<AppDialog>) {
        let previous_dialog = self.active_app_dialog();
        if matches!(dialog, Some(AppDialog::Settings))
            && !matches!(previous_dialog, Some(AppDialog::Settings))
        {
            self.network_settings_dialog_baseline = Some(self.network_settings.clone());
        } else if matches!(previous_dialog, Some(AppDialog::Settings))
            && !matches!(dialog, Some(AppDialog::Settings))
        {
            self.network_settings_dialog_baseline = None;
        }
        if self.workspaces.is_empty() {
            self.global_app_dialog = dialog;
        } else if let Some(slot) = self.app_dialogs.get_mut(self.active_workspace) {
            *slot = dialog;
        }
    }

    pub(super) fn app_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.active_app_dialog() else {
            return;
        };
        let mut next_dialog = Some(dialog.clone());
        let mut save_then_open = None;
        let mut discard_then_open = None;
        let mut workflow_save_then_select = None;
        let mut workflow_discard_then_select = None;
        let mut workflow_mutation = None;
        let mut create_markdown = None;
        let mut create_folder = None;
        let mut rename_path = None;
        let mut delete_markdown = None;
        let mut close_workspace = None;
        let mut add_subagent = None;
        let mut restart_agent = None;
        let mut switch_agent = None;
        let mut set_agent_model = None;
        let mut confirm_theme_switch = None;
        let mut restart_agent_without_resume = false;
        let mut start_codex_auth = false;
        let mut outline_action = None;

        egui::Window::new(app_dialog_title(&dialog, self.app_language))
            .order(modal_dialog_order())
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .fixed_size(match dialog {
                AppDialog::RecentMarkdownOutline { .. } => Vec2::new(620.0, 520.0),
                AppDialog::CreateMarkdown { .. }
                | AppDialog::CreateFolder { .. }
                | AppDialog::RenamePath { .. } => Vec2::new(460.0, 300.0),
                AppDialog::CloseWorkspace { .. } => Vec2::new(500.0, 280.0),
                AppDialog::WorkflowUnsavedSwitch { .. } => Vec2::new(500.0, 260.0),
                AppDialog::WorkflowAddTask { .. } => Vec2::new(500.0, 260.0),
                AppDialog::WorkflowAddStep { .. } => Vec2::new(520.0, 420.0),
                AppDialog::WorkflowRenameProject { .. }
                | AppDialog::WorkflowRenameTask { .. }
                | AppDialog::WorkflowRenameStep { .. } => Vec2::new(500.0, 260.0),
                AppDialog::WorkflowDeleteConfirm { .. } => Vec2::new(500.0, 260.0),
                AppDialog::AddSubagent { .. } => Vec2::new(460.0, 470.0),
                AppDialog::RestartAgent { .. } => Vec2::new(500.0, 260.0),
                AppDialog::SwitchAgent { .. } => Vec2::new(520.0, 280.0),
                AppDialog::SetAgentModel { .. } => Vec2::new(520.0, 260.0),
                AppDialog::ConfirmThemeSwitch { .. } => Vec2::new(460.0, 250.0),
                AppDialog::AgentExitedAbnormally { .. } => Vec2::new(620.0, 340.0),
                AppDialog::Help => Vec2::new(760.0, 640.0),
                AppDialog::Settings => Vec2::new(620.0, 600.0),
                AppDialog::About => Vec2::new(560.0, 460.0),
                AppDialog::CodexAuth => Vec2::new(560.0, 320.0),
                _ => Vec2::new(440.0, 250.0),
            })
            .frame(
                Frame::new()
                    .fill(theme::bg())
                    .stroke(Stroke::new(1.0, theme::border()))
                    .corner_radius(CornerRadius::same(theme::RADIUS_LG))
                    .inner_margin(Margin::same(18)),
            )
            .show(ctx, |ui| match dialog.clone() {
                AppDialog::RecentMarkdownOutline { mut nodes } => {
                    let mut action = None;
                    let selected = self
                        .current_workspace()
                        .and_then(|workspace| workspace.selected_file.clone());
                    let workspace_favorites = self
                        .current_workspace()
                        .map(|workspace| workspace.outline_favorites.clone())
                        .unwrap_or_default();
                    recent_markdown_outline_dialog_content(
                        ui,
                        &mut nodes,
                        selected.as_deref(),
                        &workspace_favorites,
                        &self.global_outline_favorites,
                        &mut action,
                        self.app_language,
                    );
                    if let Some(action) = action {
                        let keep_open = matches!(
                            &action,
                            OutlineAction::ToggleFavorite(_) | OutlineAction::Refresh
                        );
                        outline_action = Some(action);
                        next_dialog =
                            keep_open.then_some(AppDialog::RecentMarkdownOutline { nodes });
                    } else {
                        next_dialog = Some(AppDialog::RecentMarkdownOutline { nodes });
                    }
                }
                AppDialog::UnsavedSwitch { target } => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("!").size(30.0).color(theme::warning()));
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(i18n::text(
                                    self.app_language,
                                    "You have unsaved changes",
                                ))
                                .strong(),
                            );
                            ui.label(muted(i18n::text(
                                self.app_language,
                                "Save before switching files, discard edits, or stay on the current file.",
                            )));
                        });
                    });
                    ui.add_space(18.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked() {
                            next_dialog = None;
                        }
                        if secondary_action(ui, i18n::text(self.app_language, "Discard")).clicked()
                        {
                            discard_then_open = Some(target.clone());
                            next_dialog = None;
                        }
                        if primary_action(ui, i18n::text(self.app_language, "Save")).clicked() {
                            save_then_open = Some(target.clone());
                            next_dialog = None;
                        }
                    });
                }
                AppDialog::WorkflowUnsavedSwitch { target } => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("!").size(30.0).color(theme::warning()));
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(i18n::text(
                                    self.app_language,
                                    "You have unsaved workflow changes",
                                ))
                                .strong(),
                            );
                            ui.label(muted(i18n::text(
                                self.app_language,
                                "Save both panes before switching, discard edits, or stay here.",
                            )));
                        });
                    });
                    ui.add_space(18.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked() {
                            next_dialog = None;
                        }
                        if secondary_action(ui, i18n::text(self.app_language, "Discard")).clicked()
                        {
                            workflow_discard_then_select = Some(target.clone());
                            next_dialog = None;
                        }
                        if primary_action(ui, i18n::text(self.app_language, "Save")).clicked() {
                            workflow_save_then_select = Some(target.clone());
                            next_dialog = None;
                        }
                    });
                }
                AppDialog::WorkflowAddTask {
                    project_key,
                    mut key,
                } => {
                    ui.label(
                        RichText::new(i18n::text(self.app_language, "New workflow task")).strong(),
                    );
                    ui.add_space(12.0);
                    ui.label(section_label(i18n::text(self.app_language, "PROJECT")));
                    ui.label(RichText::new(&project_key).color(theme::text()));
                    ui.add_space(10.0);
                    ui.label(section_label(i18n::text(self.app_language, "KEY")));
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut key)
                            .hint_text("task-key")
                            .desired_width(f32::INFINITY),
                    );
                    response.request_focus();
                    let request = WorkflowMutationRequest::AddTask {
                        project_key: project_key.clone(),
                        task_key: key.clone(),
                    };
                    let error = self.workflow_mutation_key_error(&request);
                    if let Some(error) = error.as_ref() {
                        ui.add_space(6.0);
                        ui.colored_label(theme::warning(), error);
                    }
                    ui.add_space(12.0);
                    let can_create = error.is_none();
                    let enter_create = can_create
                        && response.has_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked() {
                            next_dialog = None;
                        }
                        if ui
                            .add_enabled(
                                can_create,
                                Button::new(i18n::text(self.app_language, "Create"))
                                    .fill(theme::primary()),
                            )
                            .clicked()
                            || enter_create
                        {
                            workflow_mutation = Some(request.clone());
                            next_dialog = None;
                        }
                    });
                    if next_dialog.is_some() {
                        next_dialog = Some(AppDialog::WorkflowAddTask { project_key, key });
                    }
                }
                AppDialog::WorkflowAddStep {
                    task_path,
                    mut key,
                    mut desc,
                } => {
                    ui.label(
                        RichText::new(i18n::text(self.app_language, "New workflow step")).strong(),
                    );
                    ui.add_space(12.0);
                    ui.label(section_label(i18n::text(self.app_language, "TASK")));
                    ui.label(RichText::new(display_path(&task_path)).color(theme::text()));
                    ui.add_space(10.0);
                    ui.label(section_label(i18n::text(self.app_language, "TITLE")));
                    let key_response = ui.add(
                        egui::TextEdit::singleline(&mut key)
                            .hint_text("Step title")
                            .desired_width(f32::INFINITY),
                    );
                    ui.add_space(10.0);
                    ui.label(section_label(i18n::text(self.app_language, "DESC")));
                    ui.add_sized(
                        [ui.available_width(), 140.0],
                        egui::TextEdit::multiline(&mut desc).desired_width(f32::INFINITY),
                    );
                    let request = WorkflowMutationRequest::AddStep {
                        task_path: task_path.clone(),
                        key: key.clone(),
                        desc: desc.clone(),
                    };
                    let error = self.workflow_mutation_key_error(&request);
                    if let Some(error) = error.as_ref() {
                        ui.add_space(6.0);
                        ui.colored_label(theme::warning(), error);
                    }
                    ui.add_space(12.0);
                    let can_create = error.is_none();
                    let enter_create = can_create
                        && key_response.has_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked() {
                            next_dialog = None;
                        }
                        if ui
                            .add_enabled(
                                can_create,
                                Button::new(i18n::text(self.app_language, "Create"))
                                    .fill(theme::primary()),
                            )
                            .clicked()
                            || enter_create
                        {
                            workflow_mutation = Some(request.clone());
                            next_dialog = None;
                        }
                    });
                    if next_dialog.is_some() {
                        next_dialog = Some(AppDialog::WorkflowAddStep {
                            task_path,
                            key,
                            desc,
                        });
                    }
                }
                AppDialog::WorkflowRenameProject {
                    project_key,
                    mut key,
                } => {
                    ui.label(
                        RichText::new(i18n::text(self.app_language, "Rename workflow project"))
                            .strong(),
                    );
                    ui.add_space(12.0);
                    ui.label(section_label(i18n::text(self.app_language, "CURRENT")));
                    ui.label(RichText::new(&project_key).color(theme::text()));
                    ui.add_space(10.0);
                    ui.label(section_label(i18n::text(self.app_language, "KEY")));
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut key)
                            .hint_text("project-key")
                            .desired_width(f32::INFINITY),
                    );
                    response.request_focus();
                    let request = WorkflowMutationRequest::RenameProject {
                        project_key: project_key.clone(),
                        new_key: key.clone(),
                    };
                    let error = self.workflow_mutation_key_error(&request);
                    if let Some(error) = error.as_ref() {
                        ui.add_space(6.0);
                        ui.colored_label(theme::warning(), error);
                    }
                    ui.add_space(12.0);
                    let can_save = error.is_none();
                    let enter_save = can_save
                        && response.has_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked() {
                            next_dialog = None;
                        }
                        if ui
                            .add_enabled(
                                can_save,
                                Button::new(i18n::text(self.app_language, "Save"))
                                    .fill(theme::primary()),
                            )
                            .clicked()
                            || enter_save
                        {
                            workflow_mutation = Some(request.clone());
                            next_dialog = None;
                        }
                    });
                    if next_dialog.is_some() {
                        next_dialog = Some(AppDialog::WorkflowRenameProject { project_key, key });
                    }
                }
                AppDialog::WorkflowRenameTask {
                    task_path,
                    mut key,
                } => {
                    ui.label(
                        RichText::new(i18n::text(self.app_language, "Rename workflow task"))
                            .strong(),
                    );
                    ui.add_space(12.0);
                    ui.label(section_label(i18n::text(self.app_language, "TASK")));
                    ui.label(RichText::new(display_path(&task_path)).color(theme::text()));
                    ui.add_space(10.0);
                    ui.label(section_label(i18n::text(self.app_language, "KEY")));
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut key)
                            .hint_text("task-key")
                            .desired_width(f32::INFINITY),
                    );
                    response.request_focus();
                    let request = WorkflowMutationRequest::RenameTask {
                        task_path: task_path.clone(),
                        new_key: key.clone(),
                    };
                    let error = self.workflow_mutation_key_error(&request);
                    if let Some(error) = error.as_ref() {
                        ui.add_space(6.0);
                        ui.colored_label(theme::warning(), error);
                    }
                    ui.add_space(12.0);
                    let can_save = error.is_none();
                    let enter_save = can_save
                        && response.has_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked() {
                            next_dialog = None;
                        }
                        if ui
                            .add_enabled(
                                can_save,
                                Button::new(i18n::text(self.app_language, "Save"))
                                    .fill(theme::primary()),
                            )
                            .clicked()
                            || enter_save
                        {
                            workflow_mutation = Some(request.clone());
                            next_dialog = None;
                        }
                    });
                    if next_dialog.is_some() {
                        next_dialog = Some(AppDialog::WorkflowRenameTask { task_path, key });
                    }
                }
                AppDialog::WorkflowRenameStep {
                    task_path,
                    step_path,
                    mut key,
                } => {
                    ui.label(
                        RichText::new(i18n::text(self.app_language, "Rename workflow step"))
                            .strong(),
                    );
                    ui.add_space(12.0);
                    ui.label(section_label(i18n::text(self.app_language, "TASK")));
                    ui.label(RichText::new(display_path(&task_path)).color(theme::text()));
                    ui.add_space(10.0);
                    ui.label(section_label(i18n::text(self.app_language, "KEY")));
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut key)
                            .hint_text("step.key")
                            .desired_width(f32::INFINITY),
                    );
                    response.request_focus();
                    let request = WorkflowMutationRequest::RenameStep {
                        task_path: task_path.clone(),
                        step_path: step_path.clone(),
                        new_key: key.clone(),
                    };
                    let error = self.workflow_mutation_key_error(&request);
                    if let Some(error) = error.as_ref() {
                        ui.add_space(6.0);
                        ui.colored_label(theme::warning(), error);
                    }
                    ui.add_space(12.0);
                    let can_save = error.is_none();
                    let enter_save = can_save
                        && response.has_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked() {
                            next_dialog = None;
                        }
                        if ui
                            .add_enabled(
                                can_save,
                                Button::new(i18n::text(self.app_language, "Save"))
                                    .fill(theme::primary()),
                            )
                            .clicked()
                            || enter_save
                        {
                            workflow_mutation = Some(request.clone());
                            next_dialog = None;
                        }
                    });
                    if next_dialog.is_some() {
                        next_dialog = Some(AppDialog::WorkflowRenameStep {
                            task_path,
                            step_path,
                            key,
                        });
                    }
                }
                AppDialog::WorkflowDeleteConfirm { target } => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("!").size(30.0).color(theme::danger()));
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(i18n::text(
                                    self.app_language,
                                    "Delete workflow item?",
                                ))
                                .strong(),
                            );
                            ui.label(muted(&workflow_delete_target_label(&target)));
                        });
                    });
                    ui.add_space(18.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary_action(ui, i18n::text(self.app_language, "Delete")).clicked() {
                            workflow_mutation = Some(workflow_delete_target_request(&target));
                            next_dialog = None;
                        }
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked() {
                            next_dialog = None;
                        }
                    });
                }
                AppDialog::CreateMarkdown { dir, mut name } => {
                    ui.label(
                        RichText::new(i18n::text(self.app_language, "Create New Markdown File"))
                            .strong(),
                    );
                    ui.add_space(12.0);
                    ui.label(muted(i18n::text(self.app_language, "Name")));
                    let response = ui.add_sized(
                        [ui.available_width(), 34.0],
                        egui::TextEdit::singleline(&mut name),
                    );
                    response.request_focus();
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(muted(i18n::text(self.app_language, "Common")));
                        for suggestion in MARKDOWN_NAME_SUGGESTIONS {
                            if markdown_name_suggestion_button(ui, suggestion).clicked() {
                                name = (*suggestion).to_string();
                                response.request_focus();
                            }
                        }
                    });
                    ui.add_space(10.0);
                    ui.label(muted(i18n::text(self.app_language, "Location")));
                    ui.label(RichText::new(display_path(&dir)).color(theme::text()));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary_action(ui, i18n::text(self.app_language, "Create")).clicked() {
                            create_markdown = Some((dir.clone(), name.clone()));
                            next_dialog = None;
                        }
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked() {
                            next_dialog = None;
                        }
                    });
                    if next_dialog.is_some() {
                        next_dialog = Some(AppDialog::CreateMarkdown { dir, name });
                    }
                }
                AppDialog::CreateFolder { dir, mut name } => {
                    ui.label(
                        RichText::new(i18n::text(self.app_language, "Create New Folder")).strong(),
                    );
                    ui.add_space(12.0);
                    ui.label(muted(i18n::text(self.app_language, "Name")));
                    ui.add_sized(
                        [ui.available_width(), 34.0],
                        egui::TextEdit::singleline(&mut name),
                    );
                    ui.add_space(10.0);
                    ui.label(muted(i18n::text(self.app_language, "Location")));
                    ui.label(RichText::new(display_path(&dir)).color(theme::text()));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary_action(ui, i18n::text(self.app_language, "Create")).clicked() {
                            create_folder = Some((dir.clone(), name.clone()));
                            next_dialog = None;
                        }
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked() {
                            next_dialog = None;
                        }
                    });
                    if next_dialog.is_some() {
                        next_dialog = Some(AppDialog::CreateFolder { dir, name });
                    }
                }
                AppDialog::RenamePath { path, mut name } => {
                    ui.label(RichText::new(i18n::text(self.app_language, "Rename")).strong());
                    ui.add_space(12.0);
                    ui.label(muted(i18n::text(self.app_language, "New name")));
                    ui.add_sized(
                        [ui.available_width(), 34.0],
                        egui::TextEdit::singleline(&mut name),
                    );
                    ui.add_space(10.0);
                    ui.label(muted(i18n::text(self.app_language, "Current path")));
                    ui.label(RichText::new(display_path(&path)).color(theme::text()));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary_action(ui, i18n::text(self.app_language, "Rename")).clicked() {
                            rename_path = Some((path.clone(), name.clone()));
                            next_dialog = None;
                        }
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked() {
                            next_dialog = None;
                        }
                    });
                    if next_dialog.is_some() {
                        next_dialog = Some(AppDialog::RenamePath { path, name });
                    }
                }
                AppDialog::DeleteMarkdown { path } => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("!").size(30.0).color(theme::danger()));
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(i18n::text(
                                    self.app_language,
                                    "Delete markdown file?",
                                ))
                                .strong(),
                            );
                            ui.label(muted(&display_path(&path)));
                        });
                    });
                    ui.add_space(18.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary_action(ui, i18n::text(self.app_language, "Delete")).clicked() {
                            delete_markdown = Some(path.clone());
                            next_dialog = None;
                        }
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked() {
                            next_dialog = None;
                        }
                    });
                }
                AppDialog::CloseWorkspace { index } => {
                    let workspace = self.workspaces.get(index);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("!").size(30.0).color(theme::danger()));
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(i18n::text(self.app_language, "Close workspace?"))
                                    .strong(),
                            );
                            if let Some(workspace) = workspace {
                                ui.label(muted(&workspace.name));
                                ui.label(muted(&display_path(&workspace.path)));
                            } else {
                                ui.label(muted(i18n::text(
                                    self.app_language,
                                    "This workspace no longer exists.",
                                )));
                            }
                        });
                    });
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(
                            i18n::text(
                                self.app_language,
                                "This removes the workspace from gsdv and deletes its workspace memo file under ~/.gsdv/workspaces/<workspace-hash>/memo.md. The project directory itself is not deleted.",
                            ),
                        )
                        .size(12.0)
                        .color(theme::warning()),
                    );
                    ui.add_space(14.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if workspace.is_some()
                            && primary_action(
                                ui,
                                i18n::text(self.app_language, "Close Workspace"),
                            )
                            .clicked()
                        {
                            close_workspace = Some(index);
                            next_dialog = None;
                        }
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked() {
                            next_dialog = None;
                        }
                    });
                }
                AppDialog::AddSubagent {
                    index,
                    mut name,
                    mut agent_kind,
                    mut agent_model,
                    mut agent_effort,
                    mut agent_fast_mode,
                    mut session_id,
                } => {
                    let workspace = self.workspaces.get(index);
                    ui.label(
                        RichText::new(i18n::text(self.app_language, "New subagent")).strong(),
                    );
                    if let Some(workspace) = workspace {
                        ui.label(muted(&workspace.name));
                    }
                    ui.add_space(12.0);
                    ui.label(section_label(i18n::text(self.app_language, "NAME")));
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut name)
                            .hint_text("reviewer, api-checker, ...")
                            .desired_width(f32::INFINITY),
                    );
                    if response.changed() {
                        next_dialog = Some(AppDialog::AddSubagent {
                            index,
                            name: name.clone(),
                            agent_kind,
                            agent_model: agent_model.clone(),
                            agent_effort: agent_effort.clone(),
                            agent_fast_mode,
                            session_id: session_id.clone(),
                        });
                    }
                    ui.add_space(10.0);
                    ui.label(section_label(i18n::text(self.app_language, "AGENT TYPE")));
                    let previous_agent_kind = agent_kind;
                    egui::ComboBox::from_id_salt(("add-subagent-agent-kind", index))
                        .selected_text(agent_kind.title())
                        .show_ui(ui, |ui| {
                            for option in AgentKind::all() {
                                ui.selectable_value(&mut agent_kind, option, option.title());
                            }
                        });
                    if agent_kind != previous_agent_kind {
                        if !agent_kind.supports_effort(&agent_effort) {
                            agent_effort.clear();
                        }
                        if !agent_kind.supports_fast_mode() {
                            agent_fast_mode = None;
                        }
                        next_dialog = Some(AppDialog::AddSubagent {
                            index,
                            name: name.clone(),
                            agent_kind,
                            agent_model: agent_model.clone(),
                            agent_effort: agent_effort.clone(),
                            agent_fast_mode,
                            session_id: session_id.clone(),
                        });
                    }
                    ui.add_space(10.0);
                    ui.label(section_label(i18n::text(
                        self.app_language,
                        "MODEL (OPTIONAL)",
                    )));
                    let model_response = ui.add(
                        egui::TextEdit::singleline(&mut agent_model)
                            .hint_text("gpt-5.5, sonnet, ...")
                            .desired_width(f32::INFINITY),
                    );
                    if model_response.changed() {
                        next_dialog = Some(AppDialog::AddSubagent {
                            index,
                            name: name.clone(),
                            agent_kind,
                            agent_model: agent_model.clone(),
                            agent_effort: agent_effort.clone(),
                            agent_fast_mode,
                            session_id: session_id.clone(),
                        });
                    }
                    ui.add_space(10.0);
                    ui.label(section_label(i18n::text(
                        self.app_language,
                        "EFFORT (OPTIONAL)",
                    )));
                    let effort_before = agent_effort.clone();
                    egui::ComboBox::from_id_salt(("add-subagent-agent-effort", index))
                        .selected_text(if agent_effort.is_empty() {
                            i18n::text(self.app_language, "Default effort")
                        } else {
                            agent_effort.as_str()
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut agent_effort,
                                String::new(),
                                i18n::text(self.app_language, "Default effort"),
                            );
                            for effort in agent_kind.effort_levels() {
                                ui.selectable_value(
                                    &mut agent_effort,
                                    (*effort).to_string(),
                                    *effort,
                                );
                            }
                        });
                    if agent_effort != effort_before {
                        next_dialog = Some(AppDialog::AddSubagent {
                            index,
                            name: name.clone(),
                            agent_kind,
                            agent_model: agent_model.clone(),
                            agent_effort: agent_effort.clone(),
                            agent_fast_mode,
                            session_id: session_id.clone(),
                        });
                    }
                    if agent_kind.supports_fast_mode() {
                        ui.add_space(10.0);
                        ui.label(section_label(i18n::text(
                            self.app_language,
                            "FAST MODE (OPTIONAL)",
                        )));
                        let fast_mode_before = agent_fast_mode;
                        egui::ComboBox::from_id_salt(("add-subagent-agent-fast-mode", index))
                            .selected_text(match agent_fast_mode {
                                None => i18n::text(self.app_language, "Default fast mode"),
                                Some(true) => "On",
                                Some(false) => "Off",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut agent_fast_mode,
                                    None,
                                    i18n::text(self.app_language, "Default fast mode"),
                                );
                                ui.selectable_value(&mut agent_fast_mode, Some(true), "On");
                                ui.selectable_value(&mut agent_fast_mode, Some(false), "Off");
                            });
                        if agent_fast_mode != fast_mode_before {
                            next_dialog = Some(AppDialog::AddSubagent {
                                index,
                                name: name.clone(),
                                agent_kind,
                                agent_model: agent_model.clone(),
                                agent_effort: agent_effort.clone(),
                                agent_fast_mode,
                                session_id: session_id.clone(),
                            });
                        }
                    }
                    ui.add_space(10.0);
                    ui.label(section_label(i18n::text(
                        self.app_language,
                        "SESSION ID (OPTIONAL)",
                    )));
                    let session_response = ui.add(
                        egui::TextEdit::singleline(&mut session_id)
                            .hint_text(i18n::text(self.app_language, "resume an existing session"))
                            .desired_width(f32::INFINITY),
                    );
                    if session_response.changed() {
                        next_dialog = Some(AppDialog::AddSubagent {
                            index,
                            name: name.clone(),
                            agent_kind,
                            agent_model: agent_model.clone(),
                            agent_effort: agent_effort.clone(),
                            agent_fast_mode,
                            session_id: session_id.clone(),
                        });
                    }
                    ui.add_space(14.0);
                    let can_create = workspace.is_some() && !name.trim().is_empty();
                    let enter_create = can_create
                        && (response.has_focus()
                            || model_response.has_focus()
                            || session_response.has_focus())
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    let mut create_requested = enter_create;
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked() {
                            next_dialog = None;
                        }
                        if ui
                            .add_enabled(
                                can_create,
                                Button::new(i18n::text(self.app_language, "Create"))
                                    .fill(theme::primary()),
                            )
                            .clicked()
                        {
                            create_requested = true;
                        }
                    });
                    if create_requested {
                        let trimmed_agent_model = agent_model.trim();
                        let agent_model = if trimmed_agent_model.is_empty() {
                            None
                        } else {
                            Some(trimmed_agent_model.to_string())
                        };
                        let agent_effort = agent_kind
                            .supports_effort(&agent_effort)
                            .then(|| agent_effort.clone())
                            .filter(|effort| !effort.is_empty());
                        let trimmed_session_id = session_id.trim();
                        let session_id = if trimmed_session_id.is_empty() {
                            None
                        } else {
                            Some(trimmed_session_id.to_string())
                        };
                        add_subagent = Some((
                            index,
                            name.trim().to_string(),
                            agent_kind,
                            agent_model,
                            agent_effort,
                            agent_kind
                                .supports_fast_mode()
                                .then_some(agent_fast_mode)
                                .flatten(),
                            session_id,
                        ));
                        next_dialog = None;
                    }
                }
                AppDialog::RestartAgent { index } => {
                    let workspace =
                        self.agent_workspace_for_slot(index, &self.active_agent_slot());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("!").size(30.0).color(theme::warning()));
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(i18n::text(self.app_language, "Restart agent?"))
                                    .strong(),
                            );
                            if let Some(workspace) = workspace.as_ref() {
                                ui.label(muted(&format!(
                                    "{} in {}",
                                    workspace.agent_kind.title(),
                                    workspace.name
                                )));
                            }
                        });
                    });
                    ui.add_space(12.0);
                    ui.label(muted(i18n::text(
                        self.app_language,
                        "Resume keeps the stored session id. New clears the stored session and starts fresh.",
                    )));
                    ui.add_space(14.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked() {
                            next_dialog = None;
                        }
                        if workspace.is_some()
                            && secondary_action(
                                ui,
                                i18n::text(self.app_language, "Restart New"),
                            )
                            .clicked()
                        {
                            restart_agent = Some((index, false));
                            next_dialog = None;
                        }
                        if workspace.is_some()
                            && primary_action(
                                ui,
                                i18n::text(self.app_language, "Restart Resume"),
                            )
                            .clicked()
                        {
                            restart_agent = Some((index, true));
                            next_dialog = None;
                        }
                    });
                }
                AppDialog::SwitchAgent { index, next_kind } => {
                    let workspace =
                        self.agent_workspace_for_slot(index, &self.active_agent_slot());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("!").size(30.0).color(theme::danger()));
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(i18n::text(self.app_language, "Switch agent?"))
                                    .strong(),
                            );
                            if let Some(workspace) = workspace.as_ref() {
                                ui.label(muted(&format!(
                                    "{} -> {}",
                                    workspace.agent_kind.title(),
                                    next_kind.title()
                                )));
                                ui.label(muted(&workspace.name));
                            }
                        });
                    });
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(
                            i18n::text(
                                self.app_language,
                                "Switching agent type clears this workspace's stored session id and starts a new session.",
                            ),
                        )
                        .size(12.0)
                        .color(theme::warning()),
                    );
                    ui.add_space(14.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if workspace.is_some()
                            && primary_action(ui, i18n::text(self.app_language, "Switch")).clicked()
                        {
                            switch_agent = Some((index, next_kind));
                            next_dialog = None;
                        }
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked() {
                            next_dialog = None;
                        }
                    });
                }
                AppDialog::SetAgentModel {
                    index,
                    slot,
                    mut model,
                } => {
                    let workspace = self.agent_workspace_for_slot(index, &slot);
                    ui.label(RichText::new(i18n::text(self.app_language, "Agent model")).strong());
                    if let Some(workspace) = workspace.as_ref() {
                        ui.label(muted(&format!(
                            "{} · {}",
                            workspace.agent_kind.title(),
                            workspace.name
                        )));
                    }
                    ui.add_space(12.0);
                    ui.label(section_label(i18n::text(
                        self.app_language,
                        "MODEL (OPTIONAL)",
                    )));
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut model)
                            .hint_text(i18n::text(
                                self.app_language,
                                "empty uses global default",
                            ))
                            .desired_width(f32::INFINITY),
                    );
                    if response.changed() {
                        next_dialog = Some(AppDialog::SetAgentModel {
                            index,
                            slot: slot.clone(),
                            model: model.clone(),
                        });
                    }
                    ui.add_space(14.0);
                    let enter_save = response.has_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    let mut save_requested = enter_save;
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary_action(ui, i18n::text(self.app_language, "Save")).clicked() {
                            save_requested = true;
                        }
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked() {
                            next_dialog = None;
                        }
                    });
                    if save_requested {
                        set_agent_model = Some((index, slot, model));
                        next_dialog = None;
                    }
                }
                AppDialog::ConfirmThemeSwitch { next_mode } => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("!").size(30.0).color(theme::warning()));
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(i18n::text(self.app_language, "Switch theme?"))
                                    .strong(),
                            );
                            ui.label(muted(&i18n::text_with_arg(
                                self.app_language,
                                "The active agent will gracefully exit, then resume so it can reload terminal colors for {mode} mode.",
                                "{mode}",
                                theme_mode_label(next_mode),
                            )));
                        });
                    });
                    ui.add_space(18.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary_action(ui, i18n::text(self.app_language, "Switch")).clicked() {
                            confirm_theme_switch = Some(next_mode);
                            next_dialog = None;
                        }
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked() {
                            next_dialog = None;
                        }
                    });
                }
                AppDialog::AgentExitedAbnormally { exit } => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("!").size(30.0).color(theme::warning()));
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(i18n::text(
                                    self.app_language,
                                    "Agent exited abnormally",
                                ))
                                .strong(),
                            );
                            ui.label(muted(&i18n::text_with_arg(
                                self.app_language,
                                "The embedded agent process ended with {status}. You can try the same command outside gsdv to inspect the failure.",
                                "{status}",
                                exit.status.to_string(),
                            )));
                        });
                    });
                    ui.add_space(12.0);
                    ui.label(muted(i18n::text(self.app_language, "Command")));
                    Frame::new()
                        .fill(theme::surface())
                        .stroke(Stroke::new(1.0, theme::border()))
                        .corner_radius(CornerRadius::same(theme::RADIUS_SM))
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(&exit.command)
                                    .monospace()
                                    .size(12.0)
                                    .color(theme::text()),
                            );
                        });
                    ui.add_space(12.0);
                    ui.label(muted(i18n::text(
                        self.app_language,
                        "If you confirm, gsdv will start this workspace's agent again without resume. Cancel leaves the terminal as-is.",
                    )));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary_action(
                            ui,
                            i18n::text(self.app_language, "Start Without Resume"),
                        )
                        .clicked()
                        {
                            restart_agent_without_resume = true;
                            next_dialog = None;
                        }
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked() {
                            next_dialog = None;
                        }
                    });
                }
                AppDialog::Help => {
                    help_dialog_content(
                        ui,
                        current_help_area(
                            self.current_workspace(),
                            self.workspace_terminal_drawer_is_open(),
                            self.reviewer_helix_drawer_is_open(),
                            self.notifications.open,
                        ),
                        self.app_language,
                    );
                    ui.with_layout(Layout::bottom_up(Align::RIGHT), |ui| {
                        if primary_action(ui, i18n::text(self.app_language, "Close")).clicked() {
                            next_dialog = None;
                        }
                    });
                }
                AppDialog::Settings => {
                    let language = self.app_language;
                    let footer_height = 40.0;
                    ScrollArea::vertical()
                        .id_salt("settings-dialog-scroll")
                        .max_height((ui.available_height() - footer_height).max(120.0))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(i18n::text(language, "Settings"))
                                    .strong()
                                    .size(18.0),
                            );
                            ui.add_space(12.0);
                            settings_row(
                                ui,
                                i18n::text(language, "Theme"),
                                i18n::text(language, "Design-spec light theme"),
                            );
                            settings_row(
                                ui,
                                i18n::text(language, "Terminal backend"),
                                "alacritty_terminal",
                            );
                            settings_row(
                                ui,
                                i18n::text(language, "Workspace persistence"),
                                "~/.gsdv/store",
                            );
                            settings_row(
                                ui,
                                i18n::text(language, "Agent status"),
                                "~/.gsdv/agent-status.json",
                            );
                            ui.add_space(12.0);
                            if language_settings_editor(ui, &mut self.app_language) {
                                self.pending_language_settings_save = true;
                            }
                            ui.add_space(12.0);
                            if codex_auth_settings_editor(ui, &self.codex_auth, language) {
                                start_codex_auth = true;
                                next_dialog = Some(AppDialog::CodexAuth);
                            }
                            ui.add_space(12.0);
                            if runtime_settings_editor(ui, &mut self.runtime_settings, language) {
                                self.pending_runtime_settings_save = true;
                                if !self.runtime_settings.pomodoro_enabled {
                                    self.pomodoro.start_working(Instant::now());
                                }
                            }
                            ui.add_space(12.0);
                            if font_settings_editor(
                                ui,
                                &mut self.font_settings,
                                &self.system_fonts,
                                &mut self.default_font_filter,
                                &mut self.default_fallback_font_filter,
                                &mut self.agent_font_filter,
                                &mut self.agent_fallback_font_filter,
                                &mut self.terminal_font_filter,
                                &mut self.terminal_fallback_font_filter,
                                &mut self.editor_font_filter,
                                &mut self.editor_fallback_font_filter,
                                language,
                            ) {
                                self.pending_font_settings_save = true;
                            }
                            ui.add_space(12.0);
                            if network_settings_editor(ui, &mut self.network_settings, language) {
                                self.pending_network_settings_save = true;
                            }
                            ui.add_space(12.0);
                            if network_settings_changed_since(
                                self.network_settings_dialog_baseline.as_ref(),
                                &self.network_settings,
                            ) {
                                ui.label(
                                    RichText::new(
                                        i18n::text(
                                            language,
                                            "Proxy changes are saved. Close Settings to restart open terminals, agents, and Helix sessions immediately.",
                                        ),
                                    )
                                    .size(12.0)
                                    .color(theme::warning()),
                                );
                            } else {
                                ui.label(muted(i18n::text(
                                    language,
                                    "Proxy changes apply when terminal-backed sessions start.",
                                )));
                            }
                        });
                    ui.with_layout(Layout::bottom_up(Align::RIGHT), |ui| {
                        ui.horizontal(|ui| {
                            if secondary_action(ui, i18n::text(language, "About")).clicked() {
                                next_dialog = Some(AppDialog::About);
                            }
                            if primary_action(ui, i18n::text(language, "Close")).clicked() {
                                next_dialog = None;
                            }
                        });
                    });
                }
                AppDialog::About => {
                    about_dialog_content(ui);
                    ui.with_layout(Layout::bottom_up(Align::RIGHT), |ui| {
                        if primary_action(ui, i18n::text(self.app_language, "Close")).clicked() {
                            next_dialog = None;
                        }
                    });
                }
                AppDialog::CodexAuth => {
                    codex_auth_dialog_content(ui, &self.codex_auth, self.app_language);
                    ui.with_layout(Layout::bottom_up(Align::RIGHT), |ui| {
                        ui.horizontal(|ui| {
                            if !self.codex_auth.in_flight
                                && secondary_action(
                                    ui,
                                    i18n::text(self.app_language, "Settings"),
                                )
                                .clicked()
                            {
                                next_dialog = Some(AppDialog::Settings);
                            }
                            if primary_action(
                                ui,
                                if self.codex_auth.in_flight {
                                    i18n::text(self.app_language, "Waiting")
                                } else {
                                    i18n::text(self.app_language, "Close")
                                },
                            )
                            .clicked()
                                && !self.codex_auth.in_flight
                            {
                                next_dialog = Some(AppDialog::Settings);
                            }
                        });
                    });
                }
                AppDialog::Message { message, .. } => {
                    ScrollArea::vertical()
                        .max_height((ui.available_height() - 42.0).max(80.0))
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(message).color(theme::text()))
                                    .wrap(),
                            );
                        });
                    ui.with_layout(Layout::bottom_up(Align::RIGHT), |ui| {
                        if primary_action(ui, i18n::text(self.app_language, "OK")).clicked() {
                            next_dialog = None;
                        }
                    });
                }
            });

        if start_codex_auth {
            self.spawn_codex_auth_task(ctx);
        }
        let restart_terminals_for_network_settings = matches!(dialog, AppDialog::Settings)
            && !matches!(next_dialog, Some(AppDialog::Settings))
            && network_settings_changed_since(
                self.network_settings_dialog_baseline.as_ref(),
                &self.network_settings,
            );
        self.set_active_app_dialog(next_dialog);
        if restart_terminals_for_network_settings {
            self.restart_open_terminal_hosts_for_network_settings(ctx);
        }
        if let Some(action) = outline_action {
            self.handle_outline_action(ctx, action);
        }
        if let Some(target) = save_then_open {
            self.save_active_document();
            if self
                .active_document()
                .is_none_or(|document| document.save_error.is_none())
            {
                self.open_file_now(target);
            }
        }
        if let Some(target) = discard_then_open {
            self.discard_and_open_file(target);
        }
        if let Some(target) = workflow_save_then_select {
            self.save_active_workflow_step(ctx, Some(target));
        }
        if let Some(target) = workflow_discard_then_select {
            self.open_workflow_target_now(ctx, target);
        }
        if let Some(request) = workflow_mutation {
            self.request_workflow_mutation(ctx, request);
        }
        if let Some((dir, name)) = create_markdown {
            self.create_markdown_file(dir, name);
        }
        if let Some((dir, name)) = create_folder {
            self.create_folder(dir, name);
        }
        if let Some((path, name)) = rename_path {
            self.rename_path(path, name);
        }
        if let Some(path) = delete_markdown {
            self.delete_markdown_file(path);
        }
        if let Some(index) = close_workspace {
            self.close_workspace(ctx, index);
        }
        if let Some((
            index,
            name,
            agent_kind,
            agent_model,
            agent_effort,
            agent_fast_mode,
            session_id,
        )) = add_subagent
        {
            self.add_subagent(
                ctx,
                index,
                name,
                agent_kind,
                agent_model,
                agent_effort,
                agent_fast_mode,
                session_id,
            );
        }
        if let Some((index, resume)) = restart_agent {
            self.restart_agent(ctx, index, resume);
        }
        if let Some((index, next_kind)) = switch_agent {
            self.switch_agent_kind(ctx, index, next_kind);
        }
        if let Some((index, slot, model)) = set_agent_model {
            self.set_agent_slot_model(ctx, index, slot, model);
        }
        if let Some(mode) = confirm_theme_switch {
            self.apply_theme_switch(ctx, mode);
            self.restart_active_agent_after_theme_switch(ctx);
        }
        if restart_agent_without_resume {
            self.restart_active_agent_without_resume(ctx);
        }
    }

    pub(super) fn active_reviewer_dialog(&self) -> Option<ReviewerDialog> {
        self.reviewer_dialogs
            .get(self.active_workspace)
            .and_then(Clone::clone)
    }

    pub(super) fn set_active_reviewer_dialog(&mut self, dialog: Option<ReviewerDialog>) {
        if let Some(slot) = self.reviewer_dialogs.get_mut(self.active_workspace) {
            *slot = dialog;
        }
    }

    pub(super) fn open_reviewer_branch_dialog(&mut self, ctx: &egui::Context) {
        let Some(adapter) = self.active_reviewer_adapter_mut() else {
            self.set_active_reviewer_dialog(Some(ReviewerDialog::Message {
                title: "Branch".to_string(),
                message: i18n::text(
                    self.app_language,
                    "Reviewer is not loaded for this workspace.",
                )
                .to_string(),
            }));
            return;
        };
        let Some(repo) = adapter.selected_branch_target() else {
            self.set_active_reviewer_dialog(Some(ReviewerDialog::Message {
                title: "Branch".to_string(),
                message: i18n::text(self.app_language, "No repository is selected.").to_string(),
            }));
            return;
        };
        self.set_active_reviewer_dialog(Some(ReviewerDialog::Message {
            title: "Branch".to_string(),
            message: i18n::text_with_arg(
                self.app_language,
                "Loading branches for {repo}...",
                "{repo}",
                &repo.label,
            ),
        }));
        self.spawn_reviewer_branch_choices_task(ctx, repo);
    }

    pub(super) fn confirm_reviewer_branch_switch(
        &mut self,
        ctx: &egui::Context,
        repo: ReviewerBranchTarget,
        branch: BranchInfo,
    ) {
        self.set_active_reviewer_dialog(Some(ReviewerDialog::Message {
            title: "Branch".to_string(),
            message: i18n::text(self.app_language, "Switching {repo} to {branch}...")
                .replace("{repo}", &repo.label)
                .replace("{branch}", &branch.label),
        }));
        self.spawn_reviewer_branch_switch_task(ctx, repo, branch);
    }

    pub(super) fn reviewer_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.active_reviewer_dialog() else {
            return;
        };

        let mut next_dialog = Some(dialog.clone());
        let mut branch_switch: Option<(ReviewerBranchTarget, BranchInfo)> = None;
        let mut script_run_request: Option<ReviewerScriptRunRequest> = None;

        egui::Window::new(reviewer_dialog_title(&dialog, self.app_language))
            .order(modal_dialog_order())
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .fixed_size(match dialog {
                ReviewerDialog::BranchList { .. } => Vec2::new(520.0, 560.0),
                ReviewerDialog::BranchConfirm { .. } => Vec2::new(460.0, 300.0),
                _ => Vec2::new(460.0, 240.0),
            })
            .frame(
                Frame::new()
                    .fill(theme::bg())
                    .stroke(Stroke::new(1.0, theme::border()))
                    .corner_radius(CornerRadius::same(theme::RADIUS_LG))
                    .inner_margin(Margin::same(18)),
            )
            .show(ctx, |ui| match dialog.clone() {
                ReviewerDialog::Message { message, .. } => {
                    ui.label(RichText::new(message).color(theme::text()));
                    ui.with_layout(Layout::bottom_up(Align::RIGHT), |ui| {
                        if primary_action(ui, i18n::text(self.app_language, "OK")).clicked() {
                            next_dialog = None;
                        }
                    });
                }
                ReviewerDialog::Dirty { repo, message } => {
                    Frame::new()
                        .fill(theme::danger_soft())
                        .stroke(Stroke::new(1.0, theme::danger_border()))
                        .corner_radius(CornerRadius::same(theme::RADIUS_MD))
                        .inner_margin(Margin::same(14))
                        .show(ui, |ui| {
                            ui.horizontal_top(|ui| {
                                ui.label(RichText::new("!").size(28.0).color(theme::danger()));
                                ui.vertical(|ui| {
                                    ui.label(
                                        RichText::new(i18n::text(
                                            self.app_language,
                                            "Uncommitted changes",
                                        ))
                                            .strong()
                                            .color(theme::text()),
                                    );
                                    ui.label(muted(i18n::text(
                                        self.app_language,
                                        "Commit, stash, or discard changes before switching branches.",
                                    )));
                                });
                            });
                        });
                    ui.add_space(12.0);
                    reviewer_kv(ui, i18n::text(self.app_language, "Repository"), &repo.label);
                    ui.label(muted(&message));
                    ui.with_layout(Layout::bottom_up(Align::RIGHT), |ui| {
                        if primary_action(ui, i18n::text(self.app_language, "OK")).clicked() {
                            next_dialog = None;
                        }
                    });
                }
                ReviewerDialog::BranchList {
                    repo,
                    current,
                    branches,
                    selected,
                    mut filter,
                    mut visible,
                } => {
                    ui.horizontal(|ui| {
                        ui.label(muted(i18n::text(self.app_language, "Repository")));
                        ui.label(RichText::new(&repo.label).strong().color(theme::text()));
                    });
                    ui.horizontal(|ui| {
                        ui.label(muted(i18n::text(self.app_language, "Current")));
                        ui.label(RichText::new(&current).color(theme::text()));
                    });
                    ui.add_space(14.0);
                    let filter_response = ui.add_sized(
                        [ui.available_width(), 34.0],
                        egui::TextEdit::singleline(&mut filter)
                            .hint_text(i18n::text(self.app_language, "Filter branches..."))
                            .desired_width(f32::INFINITY),
                    );
                    if filter_response.changed() {
                        visible = filtered_branch_indices(&branches, &filter);
                    }
                    ui.add_space(10.0);
                    ScrollArea::vertical()
                        .id_salt("reviewer-branch-list")
                        .max_height(340.0)
                        .show(ui, |ui| {
                            for index in visible.iter().copied() {
                                let Some(branch) = branches.get(index) else {
                                    continue;
                                };
                                let active = index == selected;
                                let response = Frame::new()
                                    .fill(if active {
                                        theme::primary_soft()
                                    } else {
                                        theme::transparent()
                                    })
                                    .corner_radius(CornerRadius::same(theme::RADIUS_SM))
                                    .inner_margin(Margin::symmetric(10, 8))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            if active {
                                                status_dot(ui, theme::primary());
                                            } else {
                                                ui.add_space(10.0);
                                            }
                                            ui.label(
                                                RichText::new(&branch.label).color(theme::text()),
                                            );
                                        });
                                    })
                                    .response
                                    .interact(Sense::click());
                                if response.clicked() {
                                    next_dialog = Some(ReviewerDialog::BranchList {
                                        repo: repo.clone(),
                                        current: current.clone(),
                                        branches: branches.clone(),
                                        selected: index,
                                        filter: filter.clone(),
                                        visible: visible.clone(),
                                    });
                                }
                                if response.double_clicked() {
                                    next_dialog = Some(ReviewerDialog::BranchConfirm {
                                        repo: repo.clone(),
                                        current: current.clone(),
                                        branch: branch.clone(),
                                    });
                                }
                            }
                        });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary_action(ui, i18n::text(self.app_language, "Switch")).clicked()
                            && let Some(branch) = branches.get(selected).cloned()
                        {
                            next_dialog = Some(ReviewerDialog::BranchConfirm {
                                repo: repo.clone(),
                                current: current.clone(),
                                branch,
                            });
                        }
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked()
                        {
                            next_dialog = None;
                        }
                    });
                    if next_dialog.is_some()
                        && !matches!(next_dialog, Some(ReviewerDialog::BranchConfirm { .. }))
                    {
                        next_dialog = Some(ReviewerDialog::BranchList {
                            repo,
                            current,
                            branches,
                            selected,
                            filter,
                            visible,
                        });
                    }
                }
                ReviewerDialog::BranchConfirm {
                    repo,
                    current,
                    branch,
                } => {
                    ui.label(RichText::new(&repo.label).strong().color(theme::text()));
                    ui.add_space(12.0);
                    reviewer_kv(ui, i18n::text(self.app_language, "From"), &current);
                    reviewer_kv(ui, i18n::text(self.app_language, "To"), &branch.label);
                    ui.add_space(16.0);
                    ui.label(muted(i18n::text(
                        self.app_language,
                        "Switching branches will reload reviewer data for this workspace.",
                    )));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary_action(ui, i18n::text(self.app_language, "Confirm")).clicked() {
                            branch_switch = Some((repo.clone(), branch.clone()));
                            next_dialog = None;
                        }
                        if secondary_action(ui, i18n::text(self.app_language, "Back")).clicked() {
                            next_dialog = None;
                        }
                    });
                }
                ReviewerDialog::ScriptConfirm { request } => {
                    ui.label(
                        RichText::new(i18n::text_with_arg(
                            self.app_language,
                            "Run {script}?",
                            "{script}",
                            &request.script.label,
                        ))
                            .strong()
                            .color(theme::text()),
                    );
                    if let Some(tip) = request.script.tip.as_deref() {
                        ui.add_space(8.0);
                        ui.label(muted(tip));
                    }
                    ui.add_space(14.0);
                    reviewer_kv(
                        ui,
                        i18n::text(self.app_language, "Repository"),
                        &request.target.label,
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if primary_action(ui, i18n::text(self.app_language, "Confirm")).clicked() {
                            script_run_request = Some(request.clone());
                            next_dialog = None;
                        }
                        if secondary_action(ui, i18n::text(self.app_language, "Cancel")).clicked()
                        {
                            next_dialog = None;
                        }
                    });
                }
            });

        self.set_active_reviewer_dialog(next_dialog);
        if let Some((repo, branch)) = branch_switch {
            self.confirm_reviewer_branch_switch(ctx, repo, branch);
        }
        if let Some(request) = script_run_request {
            self.run_reviewer_script(request);
        }
    }
}

/// 返回 reviewer 弹窗标题，适用于当前语言下的 modal 标题栏。
fn reviewer_dialog_title(dialog: &ReviewerDialog, language: AppLanguage) -> &'static str {
    let title = match dialog {
        ReviewerDialog::Message { title, .. } if title.contains("failed") => "Branch Error",
        ReviewerDialog::Message { .. } => "Branch",
        ReviewerDialog::Dirty { .. } => "Uncommitted Changes",
        ReviewerDialog::BranchList { .. } => "Switch Branch",
        ReviewerDialog::BranchConfirm { .. } => "Confirm Branch Switch",
        ReviewerDialog::ScriptConfirm { .. } => "Confirm Script",
    };
    i18n::text(language, title)
}

/// 返回应用弹窗标题，消息弹窗保留调用方给出的动态标题。
fn app_dialog_title(dialog: &AppDialog, language: AppLanguage) -> &str {
    let title = match dialog {
        AppDialog::RecentMarkdownOutline { .. } => "Recent Markdown",
        AppDialog::UnsavedSwitch { .. } => "Unsaved Changes",
        AppDialog::WorkflowUnsavedSwitch { .. } => "Unsaved Workflow",
        AppDialog::WorkflowAddTask { .. } => "New Workflow Task",
        AppDialog::WorkflowAddStep { .. } => "New Workflow Step",
        AppDialog::WorkflowRenameProject { .. }
        | AppDialog::WorkflowRenameTask { .. }
        | AppDialog::WorkflowRenameStep { .. } => "Rename Workflow",
        AppDialog::WorkflowDeleteConfirm { .. } => "Confirm Workflow Delete",
        AppDialog::CreateMarkdown { .. } => "Create Markdown",
        AppDialog::CreateFolder { .. } => "Create Folder",
        AppDialog::RenamePath { .. } => "Rename",
        AppDialog::DeleteMarkdown { .. } => "Confirm Delete",
        AppDialog::CloseWorkspace { .. } => "Close Workspace",
        AppDialog::AddSubagent { .. } => "Add Subagent",
        AppDialog::RestartAgent { .. } => "Restart Agent",
        AppDialog::SwitchAgent { .. } => "Switch Agent",
        AppDialog::SetAgentModel { .. } => "Agent model",
        AppDialog::ConfirmThemeSwitch { .. } => "Switch Theme",
        AppDialog::AgentExitedAbnormally { .. } => "Agent Exited",
        AppDialog::Help => "Help",
        AppDialog::Settings => "Settings",
        AppDialog::About => "About gsdv",
        AppDialog::CodexAuth => "Codex Auth",
        AppDialog::Message { title, .. } => return title,
    };
    i18n::text(language, title)
}

/// 生成 workflow 删除确认弹窗里的目标说明。
fn workflow_delete_target_label(target: &WorkflowDeleteTarget) -> String {
    match target {
        WorkflowDeleteTarget::Project { project_key } => format!("PROJECT {project_key}"),
        WorkflowDeleteTarget::Task { label, .. } => format!("TASK {label}"),
        WorkflowDeleteTarget::Step { title, .. } => format!("STEP {title}"),
    }
}

/// 将 workflow 删除确认目标转换成文件修改请求。
fn workflow_delete_target_request(target: &WorkflowDeleteTarget) -> WorkflowMutationRequest {
    match target {
        WorkflowDeleteTarget::Project { project_key } => WorkflowMutationRequest::DeleteProject {
            project_key: project_key.clone(),
        },
        WorkflowDeleteTarget::Task { task_path, .. } => WorkflowMutationRequest::DeleteTask {
            task_path: task_path.clone(),
        },
        WorkflowDeleteTarget::Step {
            task_path,
            step_path,
            ..
        } => WorkflowMutationRequest::DeleteStep {
            task_path: task_path.clone(),
            step_path: step_path.clone(),
        },
    }
}

fn reviewer_kv(ui: &mut Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.set_min_height(26.0);
        ui.label(RichText::new(label).color(theme::muted()).size(13.0));
        ui.label(RichText::new(value).color(theme::text()).strong());
    });
}

/// Wraps translation popup text into at most ten single-line rows.
fn agent_translation_popup_lines(message: &str, width: f32) -> Vec<String> {
    let max_chars = ((width - 48.0) / 8.0).floor().max(12.0) as usize;
    let mut lines = Vec::new();
    for raw_line in message.lines() {
        let mut current = String::new();
        for ch in raw_line.chars() {
            current.push(ch);
            if current.chars().count() >= max_chars {
                lines.push(std::mem::take(&mut current));
                if lines.len() == 10 {
                    break;
                }
            }
        }
        if lines.len() == 10 {
            break;
        }
        lines.push(current);
        if lines.len() == 10 {
            break;
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    let shown_chars = lines.iter().map(|line| line.chars().count()).sum::<usize>();
    let has_more = message.chars().count() > shown_chars;
    if has_more && let Some(last) = lines.last_mut() {
        truncate_line_with_ellipsis(last, max_chars);
    }
    lines
}

/// Truncates one popup row and appends ASCII ellipsis.
fn truncate_line_with_ellipsis(line: &mut String, max_chars: usize) {
    let keep = max_chars.saturating_sub(3);
    let mut value = line.chars().take(keep).collect::<String>();
    value.push_str("...");
    *line = value;
}

#[derive(Clone, Copy)]
struct HelpShortcut {
    keys: &'static [&'static str],
    action: &'static str,
    area: &'static str,
}

fn help_dialog_content(ui: &mut Ui, active_area: &'static str, language: AppLanguage) {
    ui.spacing_mut().item_spacing = Vec2::new(10.0, 10.0);
    Frame::new()
        .fill(theme::primary_soft())
        .stroke(Stroke::new(1.0, theme::primary_border()))
        .corner_radius(CornerRadius::same(theme::RADIUS_LG))
        .inner_margin(Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                pill_icon(ui, "?", theme::primary());
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(i18n::text(language, "Keyboard Help"))
                            .strong()
                            .size(22.0)
                            .color(theme::text()),
                    );
                    ui.label(muted(&i18n::text_with_arg(
                        language,
                        "{shortcut} opens this panel. Esc closes it.",
                        "{shortcut}",
                        help_shortcut_label(),
                    )));
                });
                ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                    badge(ui, i18n::text(language, active_area), theme::primary());
                    ui.label(section_label(i18n::text(language, "ACTIVE AREA")));
                });
            });
        });
    ui.add_space(2.0);
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            help_scope_card(
                ui,
                "Global",
                "Whole app",
                active_area == "Global",
                &[
                    HelpShortcut {
                        keys: help_shortcut_keys(),
                        action: "Show keyboard help",
                        area: "Available from every workspace surface",
                    },
                    HelpShortcut {
                        keys: &["Esc"],
                        action: "Close popups or leave Reviewer",
                        area: "Dialog, popup, Reviewer route",
                    },
                    HelpShortcut {
                        keys: &["Cmd/Ctrl+O"],
                        action: "Add workspace",
                        area: "When text fields are not focused",
                    },
                    HelpShortcut {
                        keys: &["Cmd/Ctrl+,"],
                        action: "Open settings",
                        area: "When text fields are not focused",
                    },
                    HelpShortcut {
                        keys: &["Cmd/Ctrl+Shift+P"],
                        action: "Capture an egui screenshot",
                        area: "Whole app",
                    },
                    HelpShortcut {
                        keys: &["Cmd/Ctrl+K", "Alt+K"],
                        action: "Toggle notifications",
                        area: "Whole app",
                    },
                ],
                language,
            );
            help_scope_card(
                ui,
                "Notifications",
                "Global output drawer",
                active_area == "Notifications",
                &[
                    HelpShortcut {
                        keys: &["Cmd/Ctrl+K", "Alt+K"],
                        action: "Toggle notifications",
                        area: "Whole app",
                    },
                    HelpShortcut {
                        keys: &["Clear"],
                        action: "Remove all notification lines",
                        area: "Notification drawer",
                    },
                ],
                language,
            );
            help_scope_card(
                ui,
                "Workspace",
                "Rail, outline, and center tabs",
                active_area == "Workspace",
                &[
                    HelpShortcut {
                        keys: &["Ctrl+`"],
                        action: "Switch Busy workspace",
                        area: "Workspace rail",
                    },
                    HelpShortcut {
                        keys: &["Ctrl+1"],
                        action: "Switch non-Busy workspace",
                        area: "Workspace rail",
                    },
                    HelpShortcut {
                        keys: &["Cmd/Alt+1"],
                        action: "Select main agent",
                        area: "Workspace center",
                    },
                    HelpShortcut {
                        keys: &["Cmd/Alt+2"],
                        action: "Select first subagent",
                        area: "Workspace center",
                    },
                    HelpShortcut {
                        keys: &["Cmd/Alt+3"],
                        action: "Select second subagent",
                        area: "Workspace center",
                    },
                    HelpShortcut {
                        keys: &["Cmd/Alt+4"],
                        action: "Paste recent Markdown diffs to Agent",
                        area: "Workspace center",
                    },
                    HelpShortcut {
                        keys: &["Cmd/Alt+M"],
                        action: "Translate current Agent input",
                        area: "Workspace center",
                    },
                    HelpShortcut {
                        keys: &["Cmd/Alt+N"],
                        action: "Apply last translation to Agent input",
                        area: "Workspace center",
                    },
                    HelpShortcut {
                        keys: &["Cmd/Ctrl+W", "Alt+W"],
                        action: "Toggle Agent and Markdown",
                        area: "Workspace center",
                    },
                    HelpShortcut {
                        keys: &["Click"],
                        action: "Open files, folders, tabs, and rail entries",
                        area: "Left rail and outline panel",
                    },
                    HelpShortcut {
                        keys: &["Right-click"],
                        action: "Open file and folder actions",
                        area: "Outline panel",
                    },
                ],
                language,
            );
            help_scope_card(
                ui,
                "Markdown",
                "Editor and preview",
                active_area == "Markdown",
                &[
                    HelpShortcut {
                        keys: &["Cmd/Ctrl+S"],
                        action: "Save active Markdown document",
                        area: "Markdown editor",
                    },
                    HelpShortcut {
                        keys: &["Cmd/Ctrl+E", "Alt+E"],
                        action: "Toggle editor and preview",
                        area: "Markdown surface",
                    },
                    HelpShortcut {
                        keys: &["Drag left/right"],
                        action: "Switch between preview and editor",
                        area: "Markdown surface",
                    },
                    HelpShortcut {
                        keys: &["Mouse wheel"],
                        action: "Scroll the rendered preview",
                        area: "Preview panel",
                    },
                    HelpShortcut {
                        keys: &["Click Md"],
                        action: "Return to the last Markdown mode",
                        area: "Center tabs",
                    },
                ],
                language,
            );
            help_scope_card(
                ui,
                "Terminal",
                "Embedded terminal surfaces",
                active_area == "Terminal",
                &[
                    HelpShortcut {
                        keys: &["Cmd/Ctrl+T", "Alt+T"],
                        action: "Toggle workspace terminal drawer",
                        area: "Workspace route",
                    },
                    HelpShortcut {
                        keys: &["Cmd/Ctrl+X", "Alt+X"],
                        action: "Toggle Reviewer Helix drawer",
                        area: "Workspace",
                    },
                    HelpShortcut {
                        keys: &["Type / paste"],
                        action: "Send input to the active terminal process",
                        area: "Agent, workspace terminal, Helix drawer",
                    },
                ],
                language,
            );
            help_scope_card(
                ui,
                "Reviewer",
                "GSD and git inspection route",
                active_area == "Reviewer",
                &[
                    HelpShortcut {
                        keys: &["Cmd/Ctrl+Enter", "Cmd/Ctrl/Alt+R"],
                        action: "Open Reviewer from workspace",
                        area: "Workspace route",
                    },
                    HelpShortcut {
                        keys: &["Cmd/Ctrl/Alt+R", "Esc"],
                        action: "Exit Reviewer",
                        area: "Reviewer route",
                    },
                    HelpShortcut {
                        keys: &["Left/Right", "Shift+Tab/Tab"],
                        action: "Move between reviewer columns",
                        area: "Reviewer route",
                    },
                    HelpShortcut {
                        keys: &["Up/Down"],
                        action: "Move selected reviewer row",
                        area: "Reviewer route",
                    },
                    HelpShortcut {
                        keys: &["F"],
                        action: "Toggle full-file and diff view",
                        area: "Reviewer route",
                    },
                    HelpShortcut {
                        keys: &["N", "Shift+N"],
                        action: "Jump between full-file blocks",
                        area: "Reviewer route",
                    },
                    HelpShortcut {
                        keys: &["C"],
                        action: "Copy selected reviewer prompt into Agent",
                        area: "Reviewer route",
                    },
                    HelpShortcut {
                        keys: &["R"],
                        action: "Reload reviewer data",
                        area: "Reviewer route",
                    },
                    HelpShortcut {
                        keys: &["B"],
                        action: "Open branch switcher",
                        area: "Reviewer route",
                    },
                ],
                language,
            );
        });
}

fn help_scope_card(
    ui: &mut Ui,
    title: &'static str,
    subtitle: &'static str,
    active: bool,
    shortcuts: &[HelpShortcut],
    language: AppLanguage,
) {
    let stroke_color = if active {
        theme::primary_border()
    } else {
        theme::border()
    };
    let fill = if active {
        theme::accent_soft(theme::primary())
    } else {
        theme::surface_elevated()
    };
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke_color))
        .corner_radius(CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(i18n::text(language, title))
                        .strong()
                        .size(15.0)
                        .color(theme::text()),
                );
                ui.label(muted(i18n::text(language, subtitle)));
                if active {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        badge(ui, i18n::text(language, "Current"), theme::primary());
                    });
                }
            });
            ui.add_space(8.0);
            for shortcut in shortcuts {
                help_shortcut_row(ui, *shortcut, language);
            }
        });
    ui.add_space(10.0);
}

fn help_shortcut_row(ui: &mut Ui, shortcut: HelpShortcut, language: AppLanguage) {
    ui.horizontal_top(|ui| {
        ui.set_min_height(30.0);
        ui.allocate_ui_with_layout(
            Vec2::new(270.0, 28.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                for (index, key) in shortcut.keys.iter().enumerate() {
                    if index > 0 {
                        ui.label(
                            RichText::new(i18n::text(language, "or"))
                                .size(11.0)
                                .color(theme::muted()),
                        );
                    }
                    key_chip(ui, key);
                }
            },
        );
        ui.vertical(|ui| {
            ui.label(
                RichText::new(i18n::text(language, shortcut.action))
                    .strong()
                    .color(theme::text()),
            );
            ui.label(
                RichText::new(i18n::text(language, shortcut.area))
                    .size(12.0)
                    .color(theme::muted()),
            );
        });
    });
}

fn key_chip(ui: &mut Ui, text: &str) {
    Frame::new()
        .fill(theme::bg())
        .stroke(Stroke::new(1.0, theme::border()))
        .corner_radius(CornerRadius::same(theme::RADIUS_SM))
        .inner_margin(Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .monospace()
                    .size(12.0)
                    .strong()
                    .color(theme::text()),
            );
        });
}

fn current_help_area(
    workspace: Option<&WorkspaceViewData>,
    workspace_terminal_open: bool,
    reviewer_helix_open: bool,
    notifications_open: bool,
) -> &'static str {
    if notifications_open {
        return "Notifications";
    }
    let Some(workspace) = workspace else {
        return "Global";
    };
    if reviewer_helix_open || workspace_terminal_open {
        return "Terminal";
    }
    if workspace.route == Route::Reviewer {
        return "Reviewer";
    }
    match workspace.center_mode {
        CenterMode::Editor | CenterMode::Preview => "Markdown",
        CenterMode::Agent | CenterMode::Terminal => "Workspace",
    }
}

fn settings_row(ui: &mut Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.set_min_height(30.0);
        ui.label(muted(label));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            badge(ui, value, theme::primary());
        });
    });
}

/// 绘制语言设置，适用于全局界面语言即时切换。
fn language_settings_editor(ui: &mut Ui, language: &mut AppLanguage) -> bool {
    let mut changed = false;
    Frame::new()
        .fill(theme::surface_elevated())
        .stroke(Stroke::new(1.0, theme::border()))
        .corner_radius(CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.label(
                RichText::new(i18n::text(*language, "Language"))
                    .strong()
                    .size(14.0)
                    .color(theme::text()),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(muted(i18n::text(*language, "Interface language")));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("app-language")
                        .width(180.0)
                        .selected_text(i18n::language_label(*language))
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show_ui(ui, |ui| {
                            for option in i18n::app_languages() {
                                changed |= ui
                                    .selectable_value(
                                        language,
                                        *option,
                                        i18n::language_label(*option),
                                    )
                                    .changed();
                            }
                        });
                });
            });
            ui.label(muted(i18n::text(
                *language,
                "Applied immediately and saved globally.",
            )));
        });
    changed
}

/// 绘制设置页里的 Codex 认证入口。
fn codex_auth_settings_editor(
    ui: &mut Ui,
    state: &CodexAuthUiState,
    language: AppLanguage,
) -> bool {
    let mut start_auth = false;
    Frame::new()
        .fill(theme::surface_elevated())
        .stroke(Stroke::new(1.0, theme::border()))
        .corner_radius(CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.label(
                RichText::new(i18n::text(language, "Codex auth"))
                    .strong()
                    .size(14.0)
                    .color(theme::text()),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(muted(i18n::text(language, "Account")));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if state.in_flight {
                        badge(ui, i18n::text(language, "Authorizing"), theme::warning());
                    } else if let Some(info) = state.info.as_ref() {
                        badge(ui, codex_auth_identity(info).as_str(), theme::success());
                    } else if primary_action(ui, i18n::text(language, "Auth")).clicked() {
                        start_auth = true;
                    }
                });
            });
            if let Some(error) = state.error.as_ref() {
                ui.add_space(6.0);
                ui.label(RichText::new(error).size(12.0).color(theme::danger()));
            }
        });
    start_auth
}

/// 绘制 Codex 认证弹窗内容。
fn codex_auth_dialog_content(ui: &mut Ui, state: &CodexAuthUiState, language: AppLanguage) {
    ui.label(
        RichText::new(if state.in_flight {
            i18n::text(language, "Waiting for browser authorization")
        } else if state.info.is_some() {
            i18n::text(language, "Codex authorization complete")
        } else {
            i18n::text(language, "Codex authorization failed")
        })
        .strong()
        .size(16.0)
        .color(theme::text()),
    );
    ui.add_space(10.0);
    if let Some(info) = state.info.as_ref()
        && !state.in_flight
    {
        ui.horizontal(|ui| {
            ui.label(muted(i18n::text(language, "Account")));
            badge(ui, codex_auth_identity(info).as_str(), theme::success());
        });
        return;
    }
    if let Some(error) = state.error.as_ref()
        && !state.in_flight
    {
        ui.label(RichText::new(error).color(theme::danger()));
        return;
    }
    if let Some(started_at) = state.started_at {
        ui.label(muted(&format!(
            "{} {}s",
            i18n::text(language, "Elapsed"),
            started_at.elapsed().as_secs()
        )));
    }
    if let Some(url) = state.auth_url.as_ref() {
        ui.add_space(8.0);
        ui.label(muted(i18n::text(language, "Authorization URL")));
        Frame::new()
            .fill(theme::surface())
            .stroke(Stroke::new(1.0, theme::border()))
            .corner_radius(CornerRadius::same(theme::RADIUS_SM))
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(url)
                        .monospace()
                        .size(11.0)
                        .color(theme::text()),
                );
            });
    }
}

/// Returns the best Codex account label for settings.
fn codex_auth_identity(info: &crate::ai::CodexAuthInfo) -> String {
    info.email
        .clone()
        .unwrap_or_else(|| info.account_id.clone())
}

/// 绘制运行时设置，适用于重绘、Agent 与番茄钟行为。
fn runtime_settings_editor(
    ui: &mut Ui,
    settings: &mut RuntimeSettings,
    language: AppLanguage,
) -> bool {
    let mut changed = false;
    Frame::new()
        .fill(theme::surface_elevated())
        .stroke(Stroke::new(1.0, theme::border()))
        .corner_radius(CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.label(
                RichText::new(i18n::text(language, "Rendering"))
                    .strong()
                    .size(14.0)
                    .color(theme::text()),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(muted(i18n::text(language, "Max FPS")));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut settings.max_frame_rate)
                                .range(data::MIN_MAX_FRAME_RATE..=data::MAX_MAX_FRAME_RATE)
                                .speed(1),
                        )
                        .changed();
                });
            });
            ui.label(muted(i18n::text(
                language,
                "Caps application-scheduled repaint requests.",
            )));
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(10.0);
            ui.label(
                RichText::new(i18n::text(language, "Agent"))
                    .strong()
                    .size(14.0)
                    .color(theme::text()),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(muted(i18n::text(language, "Auto go after idle minutes")));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut settings.agent_busy_auto_go_minutes)
                                .range(
                                    data::MIN_AGENT_BUSY_AUTO_GO_MINUTES
                                        ..=data::MAX_AGENT_BUSY_AUTO_GO_MINUTES,
                                )
                                .speed(1),
                        )
                        .changed();
                });
            });
            ui.label(muted(i18n::text(
                language,
                "Sends go once when a Busy agent has no new output.",
            )));
            ui.add_space(12.0);
            ui.label(muted(i18n::text(language, "Custom quick replies")));
            changed |= ui
                .add(
                    egui::TextEdit::multiline(&mut settings.agent_custom_quick_replies)
                        .hint_text(i18n::text(language, "one reply per line"))
                        .desired_width(f32::INFINITY)
                        .desired_rows(4),
                )
                .changed();
            ui.label(muted(i18n::text(
                language,
                "Shown as the second row in the Agent right-click menu.",
            )));
            ui.add_space(12.0);
            changed |= ui
                .checkbox(
                    &mut settings.agent_input_translation_auto_trigger,
                    i18n::text(language, "Auto translate Agent input after idle"),
                )
                .changed();
            ui.label(muted(i18n::text(
                language,
                "Runs Cmd/Alt+M after the Agent draft stops changing for about 500 ms.",
            )));
            ui.add_space(12.0);
            changed |= ui
                .checkbox(
                    &mut settings.codex_responses_http_fallback_enabled,
                    i18n::text(language, "Allow Codex HTTP fallback after 5 WS failures"),
                )
                .changed();
            ui.label(muted(i18n::text(
                language,
                "When disabled, Codex Responses keeps rebuilding WebSocket connections instead of silently using HTTP.",
            )));
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(10.0);
            ui.label(
                RichText::new(i18n::text(language, "Pomodoro"))
                    .strong()
                    .size(14.0)
                    .color(theme::text()),
            );
            ui.add_space(8.0);
            changed |= ui
                .checkbox(
                    &mut settings.pomodoro_enabled,
                    i18n::text(language, "Enable pomodoro"),
                )
                .changed();
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(muted(i18n::text(language, "Work minutes")));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut settings.pomodoro_work_minutes)
                                .range(data::MIN_POMODORO_MINUTES..=data::MAX_POMODORO_MINUTES)
                                .speed(1),
                        )
                        .changed();
                });
            });
            ui.horizontal(|ui| {
                ui.label(muted(i18n::text(language, "Rest minutes")));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut settings.pomodoro_rest_minutes)
                                .range(data::MIN_POMODORO_MINUTES..=data::MAX_POMODORO_MINUTES)
                                .speed(1),
                        )
                        .changed();
                });
            });
            ui.horizontal(|ui| {
                ui.label(muted(i18n::text(language, "Warning remaining percent")));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut settings.pomodoro_warning_remaining_percent)
                                .range(
                                    data::MIN_POMODORO_WARNING_REMAINING_PERCENT
                                        ..=data::MAX_POMODORO_WARNING_REMAINING_PERCENT,
                                )
                                .speed(1)
                                .suffix("%"),
                        )
                        .changed();
                });
            });
            ui.label(muted(i18n::text(
                language,
                "Rest shows a pixel cat; state resets when gsdv restarts.",
            )));
        });
    settings.max_frame_rate = settings
        .max_frame_rate
        .clamp(data::MIN_MAX_FRAME_RATE, data::MAX_MAX_FRAME_RATE);
    settings.agent_busy_auto_go_minutes = settings.agent_busy_auto_go_minutes.clamp(
        data::MIN_AGENT_BUSY_AUTO_GO_MINUTES,
        data::MAX_AGENT_BUSY_AUTO_GO_MINUTES,
    );
    settings.pomodoro_work_minutes = settings
        .pomodoro_work_minutes
        .clamp(data::MIN_POMODORO_MINUTES, data::MAX_POMODORO_MINUTES);
    settings.pomodoro_rest_minutes = settings
        .pomodoro_rest_minutes
        .clamp(data::MIN_POMODORO_MINUTES, data::MAX_POMODORO_MINUTES);
    settings.pomodoro_warning_remaining_percent =
        settings.pomodoro_warning_remaining_percent.clamp(
            data::MIN_POMODORO_WARNING_REMAINING_PERCENT,
            data::MAX_POMODORO_WARNING_REMAINING_PERCENT,
        );
    changed
}

/// 绘制网络代理设置，适用于所有新启动的终端后端进程。
fn network_settings_editor(
    ui: &mut Ui,
    settings: &mut NetworkSettings,
    language: AppLanguage,
) -> bool {
    let mut changed = false;
    Frame::new()
        .fill(theme::surface_elevated())
        .stroke(Stroke::new(1.0, theme::border()))
        .corner_radius(CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.label(
                RichText::new(i18n::text(language, "Network proxy"))
                    .strong()
                    .size(14.0)
                    .color(theme::text()),
            );
            ui.add_space(8.0);
            ui.label(muted(i18n::text(
                language,
                "HTTP / HTTPS / SOCKS proxy",
            )));
            changed |= ui
                .add_sized(
                    [ui.available_width(), 32.0],
                    settings_singleline_text_edit(&mut settings.proxy)
                        .hint_text("http://127.0.0.1:7890 or socks5://127.0.0.1:7891"),
                )
                .changed();
            ui.label(muted(i18n::text(
                language,
                "Leave empty to disable proxy.",
            )));
            ui.add_space(8.0);
            changed |= ui
                .checkbox(
                    &mut settings.mirror_proxy_protocol,
                    i18n::text(language, "Generate paired HTTP/SOCKS env vars"),
                )
                .changed();
            ui.label(muted(
                i18n::text(
                    language,
                    "When enabled, http:// also sets all_proxy=socks5://..., and socks5:// also sets http_proxy/https_proxy=http://....",
                ),
            ));
            ui.add_space(8.0);
            ui.label(muted(i18n::text(
                language,
                "Additional no_proxy entries",
            )));
            changed |= ui
                .add_sized(
                    [ui.available_width(), 32.0],
                    settings_singleline_text_edit(&mut settings.no_proxy)
                        .hint_text("example.com,.internal,10.0.0.0/8"),
                )
                .changed();
            ui.add_space(8.0);
            ui.label(muted(&format!(
                "{}: {}",
                i18n::text(language, "Built in, always included"),
                data::BUILTIN_NO_PROXY.join(", ")
            )));
            ui.label(muted(&format!(
                "{}: {}",
                i18n::text(language, "Effective no_proxy"),
                settings.effective_no_proxy()
            )));
        });
    changed
}

fn font_settings_editor(
    ui: &mut Ui,
    settings: &mut FontSettings,
    system_fonts: &[SystemFontEntry],
    default_filter: &mut FontPickerFilter,
    default_fallback_filter: &mut FontPickerFilter,
    agent_filter: &mut FontPickerFilter,
    agent_fallback_filter: &mut FontPickerFilter,
    terminal_filter: &mut FontPickerFilter,
    terminal_fallback_filter: &mut FontPickerFilter,
    editor_filter: &mut FontPickerFilter,
    editor_fallback_filter: &mut FontPickerFilter,
    language: AppLanguage,
) -> bool {
    let mut changed = false;
    Frame::new()
        .fill(theme::surface_elevated())
        .stroke(Stroke::new(1.0, theme::border()))
        .corner_radius(CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.label(
                RichText::new(i18n::text(language, "Fonts"))
                    .strong()
                    .size(14.0)
                    .color(theme::text()),
            );
            ui.add_space(8.0);
            changed |= font_surface_settings_editor(
                ui,
                i18n::text(language, "Default fonts"),
                &mut settings.default_fonts,
                system_fonts,
                default_filter,
                default_fallback_filter,
                false,
                language,
            );
            ui.add_space(8.0);
            changed |= font_surface_settings_editor(
                ui,
                i18n::text(language, "Agent"),
                &mut settings.agent,
                system_fonts,
                agent_filter,
                agent_fallback_filter,
                true,
                language,
            );
            ui.add_space(8.0);
            changed |= font_surface_settings_editor(
                ui,
                i18n::text(language, "Terminal"),
                &mut settings.terminal,
                system_fonts,
                terminal_filter,
                terminal_fallback_filter,
                true,
                language,
            );
            ui.add_space(8.0);
            changed |= font_surface_settings_editor(
                ui,
                i18n::text(language, "Markdown editor"),
                &mut settings.editor,
                system_fonts,
                editor_filter,
                editor_fallback_filter,
                true,
                language,
            );
        });
    changed
}

fn font_surface_settings_editor(
    ui: &mut Ui,
    label: &str,
    settings: &mut data::FontSurfaceSettings,
    system_fonts: &[SystemFontEntry],
    filter: &mut FontPickerFilter,
    fallback_filter: &mut FontPickerFilter,
    allow_default: bool,
    language: AppLanguage,
) -> bool {
    let mut changed = false;
    if !allow_default && settings.family == data::FontFamilySetting::Default {
        settings.family = data::FontFamilySetting::Monospace;
        changed = true;
    }
    ui.vertical(|ui| {
        ui.label(muted(label));
        ui.label(muted(i18n::text(language, "Primary font")));
        ui.horizontal(|ui| {
            ui.set_min_height(28.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                changed |= ui
                    .add_sized(
                        [68.0, 28.0],
                        egui::DragValue::new(&mut settings.size)
                            .range(9.0..=28.0)
                            .speed(0.25)
                            .suffix(" px"),
                    )
                    .changed();
                let combo_width = ui.available_width().max(120.0);
                changed |= primary_font_picker(
                    ui,
                    label,
                    settings,
                    system_fonts,
                    filter,
                    allow_default,
                    combo_width,
                    language,
                );
            });
        });
        if settings.family == data::FontFamilySetting::Default {
            settings.system_name = None;
            settings.system_path = None;
            settings.fallback_system_name = None;
            settings.fallback_system_path = None;
        } else {
            ui.add_space(4.0);
            ui.label(muted(i18n::text(language, "Fallback font")));
            changed |=
                fallback_font_picker(ui, label, settings, system_fonts, fallback_filter, language);
        }
    });
    settings.size = settings.size.clamp(9.0, 28.0);
    changed
}

/// Renders the primary font picker at the width assigned by its parent row.
fn primary_font_picker(
    ui: &mut Ui,
    label: &str,
    settings: &mut data::FontSurfaceSettings,
    system_fonts: &[SystemFontEntry],
    filter: &mut FontPickerFilter,
    allow_default: bool,
    width: f32,
    language: AppLanguage,
) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(format!("{label}-font-family"))
        .width(width)
        .selected_text(font_surface_label(settings))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            changed |= font_filter_search_box(ui, filter, i18n::text(language, "filter fonts"));
            if allow_default
                && ui
                    .selectable_value(
                        &mut settings.family,
                        data::FontFamilySetting::Default,
                        i18n::text(language, "Use default"),
                    )
                    .changed()
            {
                settings.system_name = None;
                settings.system_path = None;
                settings.fallback_system_name = None;
                settings.fallback_system_path = None;
                filter.clear();
                ui.memory_mut(|memory| memory.close_popup());
                changed = true;
            }
            if ui
                .selectable_value(
                    &mut settings.family,
                    data::FontFamilySetting::Monospace,
                    i18n::text(language, "Monospace"),
                )
                .changed()
            {
                filter.clear();
                ui.memory_mut(|memory| memory.close_popup());
                changed = true;
            }
            if ui
                .selectable_value(
                    &mut settings.family,
                    data::FontFamilySetting::Proportional,
                    i18n::text(language, "Proportional"),
                )
                .changed()
            {
                filter.clear();
                ui.memory_mut(|memory| memory.close_popup());
                changed = true;
            }
            ui.label(muted(i18n::text(language, "System fonts")));
            let mut visible = 0usize;
            for font in system_fonts {
                if !font_matches_filter_key(font, &filter.normalized) {
                    continue;
                }
                visible += 1;
                let font_path = font.path.to_string_lossy();
                let selected = settings.family == data::FontFamilySetting::System
                    && settings.system_path.as_deref() == Some(font_path.as_ref());
                if ui.selectable_label(selected, &font.name).clicked() {
                    settings.family = data::FontFamilySetting::System;
                    settings.system_name = Some(font.name.clone());
                    settings.system_path = Some(font_path.to_string());
                    filter.clear();
                    ui.memory_mut(|memory| memory.close_popup());
                    changed = true;
                }
            }
            if visible == 0 {
                ui.label(muted(i18n::text(language, "No matching system fonts")));
            }
        });
    changed
}

/// Renders a focused search field inside a font ComboBox popup.
fn font_filter_search_box(ui: &mut Ui, filter: &mut FontPickerFilter, hint: &str) -> bool {
    let response = ui.add_sized(
        [ui.available_width().max(120.0), 24.0],
        settings_singleline_text_edit(&mut filter.text).hint_text(hint),
    );
    if !response.has_focus() {
        response.request_focus();
    }
    if response.changed() {
        filter.sync_normalized();
        return true;
    }
    false
}

/// Builds a settings input whose text sits centered in fixed-height rows.
fn settings_singleline_text_edit(text: &mut String) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text).vertical_align(Align::Center)
}

/// Renders the optional second-font selector for missing glyph fallback.
fn fallback_font_picker(
    ui: &mut Ui,
    label: &str,
    settings: &mut data::FontSurfaceSettings,
    system_fonts: &[SystemFontEntry],
    filter: &mut FontPickerFilter,
    language: AppLanguage,
) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(format!("{label}-fallback-font-family"))
        .width(ui.available_width().max(120.0))
        .selected_text(fallback_font_surface_label(settings))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            changed |=
                font_filter_search_box(ui, filter, i18n::text(language, "filter fallback fonts"));
            if ui
                .selectable_label(
                    settings.fallback_system_path.is_none(),
                    i18n::text(language, "Automatic"),
                )
                .clicked()
            {
                settings.fallback_system_name = None;
                settings.fallback_system_path = None;
                filter.clear();
                ui.memory_mut(|memory| memory.close_popup());
                changed = true;
            }
            ui.label(muted(i18n::text(language, "System fonts")));
            let mut visible = 0usize;
            for font in system_fonts {
                if !font_matches_filter_key(font, &filter.normalized) {
                    continue;
                }
                visible += 1;
                let font_path = font.path.to_string_lossy();
                let selected = settings.fallback_system_path.as_deref() == Some(font_path.as_ref());
                if ui.selectable_label(selected, &font.name).clicked() {
                    settings.fallback_system_name = Some(font.name.clone());
                    settings.fallback_system_path = Some(font_path.to_string());
                    filter.clear();
                    ui.memory_mut(|memory| memory.close_popup());
                    changed = true;
                }
            }
            if visible == 0 {
                ui.label(muted(i18n::text(language, "No matching system fonts")));
            }
        });
    changed
}

fn font_family_setting_label(family: data::FontFamilySetting) -> &'static str {
    match family {
        data::FontFamilySetting::Default => "Use default",
        data::FontFamilySetting::Monospace => "Monospace",
        data::FontFamilySetting::Proportional => "Proportional",
        data::FontFamilySetting::System => "System",
    }
}

fn fallback_font_surface_label(settings: &data::FontSurfaceSettings) -> String {
    settings
        .fallback_system_name
        .clone()
        .unwrap_or_else(|| "Automatic".to_string())
}

fn font_surface_label(settings: &data::FontSurfaceSettings) -> String {
    match settings.family {
        data::FontFamilySetting::System => settings
            .system_name
            .clone()
            .unwrap_or_else(|| "System font".to_string()),
        family => font_family_setting_label(family).to_string(),
    }
}

fn about_dialog_content(ui: &mut Ui) {
    ui.spacing_mut().item_spacing = Vec2::new(10.0, 10.0);
    Frame::new()
        .fill(theme::primary_soft())
        .stroke(Stroke::new(1.0, theme::primary_border()))
        .corner_radius(CornerRadius::same(theme::RADIUS_LG))
        .inner_margin(Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                pill_icon(ui, "g", theme::primary());
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(APP_NAME)
                            .strong()
                            .size(24.0)
                            .color(theme::text()),
                    );
                    ui.label(
                        RichText::new(format!("Version {APP_VERSION}"))
                            .size(13.0)
                            .color(theme::muted()),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                    badge(ui, "egui desktop", theme::primary());
                });
            });
        });

    ui.add_space(2.0);
    ui.label(RichText::new(APP_DESCRIPTION).color(theme::text()));
    ui.add_space(6.0);

    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            about_row(
                ui,
                "Workspace",
                "Rail, outline tree, Markdown editor, rendered preview, embedded Agent, and workspace terminal.",
            );
            about_row(
                ui,
                "Reviewer",
                "GSD and git review surfaces for provenance, diffs, branch switching, and repo scripts.",
            );
            about_row(
                ui,
                "Runtime",
                "Single-process eframe app with direct alacritty_terminal embedded child processes.",
            );
            about_row(
                ui,
                "Storage",
                "Workspace metadata and UI state are persisted under ~/.gsdv/store.",
            );
            about_row(
                ui,
                "Design",
                "Light desktop workspace, dense navigation, shallow borders, and keyboard-first routing.",
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new(APP_COPYRIGHT)
                    .size(12.0)
                    .color(theme::muted()),
            );
        });
}

fn about_row(ui: &mut Ui, label: &str, value: &str) {
    Frame::new()
        .fill(theme::surface_elevated())
        .stroke(Stroke::new(1.0, theme::border()))
        .corner_radius(CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(104.0, 44.0),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.label(
                            RichText::new(label)
                                .strong()
                                .size(13.0)
                                .color(theme::text()),
                        );
                    },
                );
                ui.label(
                    RichText::new(value)
                        .size(13.0)
                        .color(theme::muted())
                        .line_height(Some(16.0)),
                );
            });
        });
    ui.add_space(8.0);
}

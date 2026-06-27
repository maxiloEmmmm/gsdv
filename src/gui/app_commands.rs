//! 输入命令 dispatch 和 route 切换 glue。
//!
//! 本模块把 input runtime 产出的 UI 命令路由到业务 owner；
//! 它只做轻量状态切换或派发后台任务。

use super::*;
use crate::gui::hook;

impl GsdvGuiApp {
    /// Returns whether the center workspace surface may consume keyboard text.
    pub(super) fn center_surface_accepts_keyboard_input(&self) -> bool {
        self.active_app_dialog().is_none()
            && self.active_reviewer_dialog().is_none()
            && !self.extra_tools.open
            && !self.notifications.open
            && !self.workspace_terminal_drawer_is_open()
            && !self.reviewer_helix_drawer_is_open()
            && !self.suppress_default_agent_input
    }

    /// 测试兼容入口，只解析传入的输入快照。
    #[cfg(test)]
    pub(super) fn read_base_route_command(&self, input: &egui::InputState) -> Option<UiCommand> {
        let in_reviewer_route = self
            .current_workspace()
            .is_some_and(|workspace| workspace.route == Route::Reviewer);
        read_base_route_command_for_input(input, in_reviewer_route)
    }

    /// Allows Helix toggling only where terminal-style shortcut capture is expected.
    pub(super) fn helix_shortcut_allowed(
        &self,
        in_reviewer_route: bool,
        text_input_focused: bool,
    ) -> bool {
        if self.active_app_dialog().is_some()
            || self.active_reviewer_dialog().is_some()
            || self.notifications.open
        {
            return false;
        }
        if in_reviewer_route
            || self.reviewer_helix_drawer_is_open()
            || self.workspace_terminal_drawer_is_open()
        {
            return true;
        }
        self.current_workspace().is_some_and(|workspace| {
            workspace.route == Route::Workspace
                && (!text_input_focused
                    || matches!(
                        workspace.center_mode,
                        CenterMode::Agent | CenterMode::Terminal
                    ))
        })
    }

    pub(super) fn dispatch_ui_command(&mut self, ctx: &egui::Context, command: UiCommand) {
        self.suppress_default_agent_input = true;
        match command {
            UiCommand::CloseTopLayer => self.close_top_keyboard_layer(),
            UiCommand::ToggleAppFullscreen => self.toggle_app_fullscreen(ctx),
            UiCommand::MoveAgentFocus(direction) => {
                self.move_agent_focus(self.active_workspace, direction)
            }
            UiCommand::OpenHelp => self.set_active_app_dialog(Some(AppDialog::Help)),
            UiCommand::SaveDocument => {
                if self.workflow_task_surface_visible() {
                    self.save_active_workflow_step(ctx, None);
                } else {
                    self.save_active_document();
                }
            }
            UiCommand::CopyWorkflowPath => self.copy_selected_workflow_path(ctx),
            UiCommand::CaptureScreenshot => self.request_egui_screenshot(ctx, "hotkey"),
            UiCommand::ToggleWorkspaceTerminal => self.route_to_workspace_terminal_drawer(),
            UiCommand::ToggleNotifications => self.toggle_notifications(),
            UiCommand::ToggleRecentMarkdownOutline => self.toggle_recent_markdown_outline_dialog(),
            UiCommand::ToggleOutlineWorkflowTab => self.toggle_outline_workflow_tab(ctx),
            UiCommand::PasteRecentMarkdownDiffsToAgent => {
                self.paste_recent_markdown_diffs_to_agent(ctx);
            }
            UiCommand::TranslateAgentInput => self.translate_active_agent_input(ctx),
            UiCommand::ApplyAgentInputTranslation => self.apply_last_agent_input_translation(ctx),
            UiCommand::ToggleExtraTools => self.toggle_extra_tools(ctx),
            UiCommand::ToggleRecentAgentHelixTargets => {
                self.toggle_recent_agent_helix_targets_dialog()
            }
            UiCommand::AgentMarkdownShortcut => self.route_agent_markdown_shortcut(),
            UiCommand::ToggleMarkdownEditorPreview => {
                self.suppress_editor_input = true;
                self.toggle_markdown_editor_preview();
            }
            UiCommand::SetCenterMode(mode) => self.set_center_mode(mode),
            UiCommand::ToggleReviewerHelix => {
                self.close_notifications_without_restore();
                self.toggle_reviewer_helix_drawer(ctx);
            }
            UiCommand::OpenReviewerRoute => self.open_reviewer_route(),
            UiCommand::ExitReviewerRoute => self.exit_reviewer_route(),
            UiCommand::AddWorkspace => self.add_workspace_from_dialog(ctx),
            UiCommand::OpenSettings => self.set_active_app_dialog(Some(AppDialog::Settings)),
            UiCommand::SwitchActiveWorkspace => self.switch_active_workspace_forward(),
            UiCommand::SwitchInactiveWorkspace => self.switch_inactive_workspace_forward(),
            UiCommand::SelectAgentSlot(slot_index) => self.select_agent_slot_by_index(slot_index),
            UiCommand::Reviewer(command) => self.dispatch_reviewer_command(ctx, command),
        }
    }

    /// Toggles app chrome visibility for focused center work.
    pub(super) fn toggle_app_fullscreen(&mut self, ctx: &egui::Context) {
        self.app_fullscreen = !self.app_fullscreen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(!self.app_fullscreen));
        self.request_app_repaint();
    }

    pub(super) fn dispatch_reviewer_command(
        &mut self,
        ctx: &egui::Context,
        command: ReviewerCommand,
    ) {
        match command {
            ReviewerCommand::PreviousColumn => {
                if let Some(adapter) = self.active_reviewer_adapter_mut() {
                    adapter.previous_column();
                }
            }
            ReviewerCommand::NextColumn => {
                if let Some(adapter) = self.active_reviewer_adapter_mut() {
                    adapter.next_column();
                }
            }
            ReviewerCommand::PreviousItem => {
                self.spawn_reviewer_adapter_task(
                    ctx,
                    ReviewerAdapterTask::PreviousItem,
                    ReviewerAdapterTaskEffect::None,
                );
            }
            ReviewerCommand::NextItem => {
                self.spawn_reviewer_adapter_task(
                    ctx,
                    ReviewerAdapterTask::NextItem,
                    ReviewerAdapterTaskEffect::None,
                );
            }
            ReviewerCommand::JumpPreviousBlock => self.jump_reviewer_full_block(ctx, true),
            ReviewerCommand::JumpNextBlock => self.jump_reviewer_full_block(ctx, false),
            ReviewerCommand::CopySelectionToAgent => {
                let copy_text = self
                    .selected_reviewer_diff_row_for_keyboard()
                    .and_then(|row| {
                        self.reviewer_snapshot_diff_copy_text(row, ReviewerDiffCopyKind::Line)
                    })
                    .or_else(|| {
                        self.active_reviewer_adapter()
                            .and_then(|adapter| adapter.selected_agent_paste_text())
                    });
                let Some(copy_text) = copy_text else {
                    self.push_toast(
                        i18n::text(self.app_language, "No reviewer row selected to copy"),
                        theme::warning(),
                    );
                    return;
                };
                self.copy_reviewer_text_to_agent(ctx, &copy_text);
            }
            ReviewerCommand::CopyDiffToAgent => {
                let copy_text = self
                    .selected_reviewer_diff_row_for_keyboard()
                    .and_then(|row| {
                        self.reviewer_snapshot_diff_copy_text(row, ReviewerDiffCopyKind::Metadata)
                    });
                let Some(copy_text) = copy_text else {
                    return;
                };
                self.copy_reviewer_text_to_agent(ctx, &copy_text);
            }
            ReviewerCommand::Reload => {
                self.spawn_reviewer_adapter_task(
                    ctx,
                    ReviewerAdapterTask::Reload,
                    ReviewerAdapterTaskEffect::Reloaded,
                );
            }
            ReviewerCommand::ToggleFullDiff => {
                self.spawn_reviewer_adapter_task(
                    ctx,
                    ReviewerAdapterTask::ToggleFullDiff,
                    ReviewerAdapterTaskEffect::SyncDiffScroll,
                );
            }
            ReviewerCommand::OpenBranchDialog => self.open_reviewer_branch_dialog(ctx),
        }
    }

    /// 复制 reviewer 文本到剪贴板并输入到 agent。
    pub(super) fn copy_reviewer_text_to_agent(&mut self, ctx: &egui::Context, text: &str) {
        if self.copy_to_clipboard_and_paste_to_agent(ctx, text) {
            self.push_toast(
                i18n::text(self.app_language, "Copied and pasted to agent"),
                theme::success(),
            );
        } else {
            self.push_toast(
                i18n::text(
                    self.app_language,
                    "Copied, but agent terminal is unavailable",
                ),
                theme::warning(),
            );
        }
    }

    /// 处理外部 hook 事件。
    pub(super) fn handle_external_hook(
        &mut self,
        ctx: &egui::Context,
        event: hook::ExternalHookEvent,
    ) {
        hook::hook_info(format_args!(
            "app handle key={} data={}",
            event.key, event.data
        ));
        match event.key.as_str() {
            hook::HELIX_CURRENT_KEY => {
                let Some(text) = normalize_helix_current_hook_data(&event.data) else {
                    hook::hook_info(format_args!(
                        "helix.current ignored invalid data={}",
                        event.data
                    ));
                    return;
                };
                hook::hook_info(format_args!("helix.current paste text={text}"));
                self.copy_reviewer_text_to_agent(ctx, &text);
            }
            hook::AGENT_STATUS_KEY => {
                if let Ok(status) = serde_json::from_str::<AgentStatusHookData>(&event.data) {
                    self.apply_agent_status_hook_data(status);
                }
            }
            _ => {}
        }
    }

    /// 应用 socket 直达的 Agent 状态变化。
    pub(super) fn apply_agent_status_hook_data(&mut self, status: AgentStatusHookData) {
        let activity = workspace_activity_from_hook_status(&status.status);
        let workspace_path = PathBuf::from(&status.workspace);
        let normalized_workspace = workspace_path.canonicalize().unwrap_or(workspace_path);
        let mut changed = false;
        for workspace in &mut self.workspaces {
            if let Some(subagent) = workspace
                .subagents
                .iter_mut()
                .find(|subagent| subagent.agent_id == status.agent_id)
            {
                if subagent.activity != activity {
                    subagent.activity = activity;
                    changed = true;
                }
                if !status.session_id.is_empty()
                    && subagent.session_id.as_deref() != Some(status.session_id.as_str())
                {
                    subagent.session_id = Some(status.session_id.clone());
                    changed = true;
                }
                continue;
            }
            let workspace_matches = workspace
                .path
                .canonicalize()
                .unwrap_or_else(|_| workspace.path.clone())
                == normalized_workspace;
            if workspace.agent_id != status.agent_id && !workspace_matches {
                continue;
            }
            if workspace.activity != activity {
                workspace.activity = activity;
                changed = true;
            }
            if !status.session_id.is_empty()
                && workspace.session_id.as_deref() != Some(status.session_id.as_str())
            {
                workspace.session_id = Some(status.session_id.clone());
                changed = true;
            }
        }
        if changed {
            self.mark_workspace_store_dirty();
            self.request_app_repaint();
        }
    }

    /// 处理共享 diff viewer 在 Reviewer 中产生的动作。
    pub(super) fn dispatch_reviewer_diff_action(
        &mut self,
        ctx: &egui::Context,
        action: crate::gui::diff_viewer::DiffViewerAction,
    ) {
        match action {
            crate::gui::diff_viewer::DiffViewerAction::None => {}
            crate::gui::diff_viewer::DiffViewerAction::ToggleMode => {
                self.spawn_reviewer_adapter_task(
                    ctx,
                    ReviewerAdapterTask::ToggleFullDiff,
                    ReviewerAdapterTaskEffect::SyncDiffScroll,
                );
            }
            crate::gui::diff_viewer::DiffViewerAction::PreviousBlock => {
                self.jump_reviewer_full_block(ctx, true);
            }
            crate::gui::diff_viewer::DiffViewerAction::NextBlock => {
                self.jump_reviewer_full_block(ctx, false);
            }
            crate::gui::diff_viewer::DiffViewerAction::CopyLine { row } => {
                self.select_reviewer_diff_row_for_gui(ctx, row);
                if let Some(text) =
                    self.reviewer_snapshot_diff_copy_text(row, ReviewerDiffCopyKind::Line)
                {
                    self.copy_reviewer_text_to_agent(ctx, &text);
                }
            }
            crate::gui::diff_viewer::DiffViewerAction::CopyDiff { row } => {
                self.select_reviewer_diff_row_for_gui(ctx, row);
                if let Some(text) =
                    self.reviewer_snapshot_diff_copy_text(row, ReviewerDiffCopyKind::Metadata)
                {
                    self.copy_reviewer_text_to_agent(ctx, &text);
                }
            }
        }
    }

    /// 返回 Reviewer diff 当前可复制选中行。
    pub(super) fn selected_reviewer_diff_row_for_keyboard(&self) -> Option<usize> {
        self.reviewer_diff_selected_rows
            .get(self.active_workspace)
            .copied()
            .flatten()
            .or_else(|| {
                self.active_reviewer_adapter().and_then(|adapter| {
                    adapter
                        .snapshot()
                        .diff_lines
                        .iter()
                        .position(|line| line.selected)
                })
            })
    }

    /// 同步 reviewer diff 行的 GUI 选中态。
    pub(super) fn select_reviewer_diff_row_for_gui(&mut self, ctx: &egui::Context, row: usize) {
        if let Some(slot) = self
            .reviewer_diff_selected_rows
            .get_mut(self.active_workspace)
        {
            *slot = Some(row);
        }
        if let Some(adapter) = self.active_reviewer_adapter_mut() {
            adapter.select_diff_row(row);
        } else {
            self.spawn_reviewer_adapter_task(
                ctx,
                ReviewerAdapterTask::SelectDiffRow(row),
                ReviewerAdapterTaskEffect::None,
            );
        }
        self.request_app_repaint();
    }

    /// 返回当前 reviewer snapshot 中 diff 行的复制文本。
    pub(super) fn reviewer_snapshot_diff_copy_text(
        &self,
        row: usize,
        kind: ReviewerDiffCopyKind,
    ) -> Option<String> {
        let line = self.reviewer_snapshot_diff_line(row)?;
        match kind {
            ReviewerDiffCopyKind::Line => line.single_click_copy.clone(),
            ReviewerDiffCopyKind::Metadata => line.metadata_copy.clone(),
        }
    }

    /// 读取当前屏幕上的 reviewer diff 行。
    ///
    /// 触发条件：reviewer adapter 正在后台刷新时，用户仍可点击当前
    /// snapshot 的第四列并按 c/d。
    /// 不能直接依赖 adapter：后台任务会临时拿走 adapter 所有权。
    /// 防止回归：刷新期间视觉选中有效，但复制快捷键静默失效。
    pub(super) fn reviewer_snapshot_diff_line(
        &self,
        row: usize,
    ) -> Option<&crate::reviewer::app::GuiDiffLine> {
        self.reviewer_snapshots
            .get(self.active_workspace)
            .and_then(Option::as_ref)
            .or_else(|| {
                self.active_reviewer_adapter()
                    .map(|adapter| adapter.snapshot())
            })
            .and_then(|snapshot| snapshot.diff_lines.get(row))
    }

    pub(super) fn keyboard_overlay_is_open(&self) -> bool {
        self.active_app_dialog().is_some()
            || self.active_reviewer_dialog().is_some()
            || self.extra_tools.open
            || self.notifications.open
    }

    pub(super) fn keyboard_layer_can_close_with_escape(&self) -> bool {
        self.keyboard_overlay_is_open()
            || self
                .current_workspace()
                .is_some_and(|workspace| workspace.route == Route::Reviewer)
    }

    pub(super) fn jump_reviewer_full_block(&mut self, ctx: &egui::Context, reverse: bool) {
        self.spawn_reviewer_adapter_task(
            ctx,
            ReviewerAdapterTask::JumpFullBlock { reverse },
            ReviewerAdapterTaskEffect::SyncDiffScroll,
        );
    }

    pub(super) fn queue_reviewer_diff_scroll_sync(&mut self) {
        let Some(row) = self
            .reviewer_adapters
            .get(self.active_workspace)
            .and_then(|adapter| adapter.as_ref())
            .map(ReviewerAdapter::diff_scroll_row)
        else {
            return;
        };
        if let Some(target) = self
            .reviewer_diff_scroll_targets
            .get_mut(self.active_workspace)
        {
            *target = Some(row);
        }
    }

    pub(super) fn set_center_mode(&mut self, mode: CenterMode) {
        let Some(workspace) = self.current_workspace_mut() else {
            return;
        };
        if workspace.route != Route::Workspace {
            return;
        }
        workspace.center_mode = match mode {
            CenterMode::Terminal => CenterMode::Agent,
            mode => mode,
        };
        self.persist_workspaces();
    }

    pub(super) fn toggle_agent_markdown_center_mode(&mut self) {
        let Some(workspace) = self.current_workspace_mut() else {
            return;
        };
        if workspace.route != Route::Workspace {
            return;
        }
        let (center_mode, previous_center_mode) =
            agent_markdown_toggle_modes(workspace.center_mode, workspace.previous_center_mode);
        workspace.center_mode = center_mode;
        workspace.previous_center_mode = previous_center_mode;
        self.persist_workspaces();
    }

    pub(super) fn close_top_keyboard_layer(&mut self) {
        if self.notifications.open {
            self.close_notifications_restoring_route();
            return;
        }
        if self.active_app_dialog().is_some() {
            self.set_active_app_dialog(None);
            return;
        }
        if self.active_reviewer_dialog().is_some() {
            self.set_active_reviewer_dialog(None);
            return;
        }
        if self.extra_tools.open {
            self.close_extra_tools();
            return;
        }
        if self
            .current_workspace()
            .is_some_and(|workspace| workspace.route == Route::Reviewer)
        {
            self.exit_reviewer_route();
        }
    }

    pub(super) fn close_input_overlays_for_navigation(&mut self) {
        self.close_notifications_without_restore();
        self.close_extra_tools();
        if let Some(open) = self
            .workspace_terminal_drawers
            .get_mut(self.active_workspace)
        {
            *open = false;
        }
        if let Some(open) = self.reviewer_helix_drawers.get_mut(self.active_workspace) {
            *open = false;
        }
        self.queue_app_event(AppEvent::SyncTerminalEventRepaintFlags);
    }

    pub(super) fn route_to_agent(&mut self) {
        self.close_input_overlays_for_navigation();
        let Some(workspace) = self.current_workspace_mut() else {
            return;
        };
        workspace.route = Route::Workspace;
        workspace.center_mode = CenterMode::Agent;
        workspace.previous_center_mode = CenterMode::Agent;
        self.persist_workspaces();
    }

    pub(super) fn route_agent_markdown_shortcut(&mut self) {
        let should_route_to_agent = self.notifications.open
            || self.extra_tools.open
            || self.workspace_terminal_drawer_is_open()
            || self.reviewer_helix_drawer_is_open()
            || self
                .current_workspace()
                .is_some_and(|workspace| workspace.route == Route::Reviewer);
        if should_route_to_agent {
            self.route_to_agent();
        } else {
            self.toggle_agent_markdown_center_mode();
        }
    }

    pub(super) fn route_to_workspace_terminal_drawer(&mut self) {
        self.close_notifications_without_restore();
        self.close_extra_tools();
        self.toggle_workspace_terminal_drawer();
    }

    /// Handles commands requested from the Agent tab context menu.
    pub(super) fn handle_agent_tab_action(&mut self, ctx: &egui::Context, action: AgentTabAction) {
        match action {
            AgentTabAction::AddSubagentToColumn { column_id } => {
                let agent_kind = self
                    .current_workspace()
                    .map(|workspace| workspace.agent_kind)
                    .unwrap_or(self.default_agent_kind);
                self.set_active_app_dialog(Some(AppDialog::AddSubagent {
                    index: self.active_workspace,
                    column_id,
                    name: String::new(),
                    agent_kind,
                    agent_model: String::new(),
                    agent_model_provider: String::new(),
                    model_providers: data::load_codex_model_provider_names(),
                    agent_effort: String::new(),
                    agent_fast_mode: None,
                    agent_work_dir: String::new(),
                    session_id: String::new(),
                }));
            }
            AgentTabAction::AddRow { row_index } => {
                let Some(column_id) = self.add_agent_row(self.active_workspace, row_index) else {
                    return;
                };
                let agent_kind = self
                    .current_workspace()
                    .map(|workspace| workspace.agent_kind)
                    .unwrap_or(self.default_agent_kind);
                self.set_active_app_dialog(Some(AppDialog::AddSubagent {
                    index: self.active_workspace,
                    column_id,
                    name: String::new(),
                    agent_kind,
                    agent_model: String::new(),
                    agent_model_provider: String::new(),
                    model_providers: data::load_codex_model_provider_names(),
                    agent_effort: String::new(),
                    agent_fast_mode: None,
                    agent_work_dir: String::new(),
                    session_id: String::new(),
                }));
            }
            AgentTabAction::AddColumn { row_index } => {
                let Some(column_id) = self.add_agent_column(self.active_workspace, row_index)
                else {
                    return;
                };
                let agent_kind = self
                    .current_workspace()
                    .map(|workspace| workspace.agent_kind)
                    .unwrap_or(self.default_agent_kind);
                self.set_active_app_dialog(Some(AppDialog::AddSubagent {
                    index: self.active_workspace,
                    column_id,
                    name: String::new(),
                    agent_kind,
                    agent_model: String::new(),
                    agent_model_provider: String::new(),
                    model_providers: data::load_codex_model_provider_names(),
                    agent_effort: String::new(),
                    agent_fast_mode: None,
                    agent_work_dir: String::new(),
                    session_id: String::new(),
                }));
            }
            AgentTabAction::CloseRow { row_index } => {
                self.set_active_app_dialog(Some(AppDialog::CloseAgentRow {
                    index: self.active_workspace,
                    row_index,
                }));
            }
            AgentTabAction::CloseColumn {
                row_index,
                column_index,
            } => {
                let Some(column_id) = self
                    .current_workspace()
                    .and_then(|workspace| workspace.agent_rows.get(row_index))
                    .and_then(|row| row.columns.get(column_index))
                    .map(|column| column.id.clone())
                else {
                    return;
                };
                self.set_active_app_dialog(Some(AppDialog::CloseAgentColumn {
                    index: self.active_workspace,
                    column_id,
                }));
            }
            AgentTabAction::CollapseRow { row_index } => {
                self.set_agent_row_collapsed(self.active_workspace, row_index, true);
            }
            AgentTabAction::CollapseOtherRows { row_index } => {
                self.collapse_other_agent_rows(self.active_workspace, row_index);
            }
            AgentTabAction::CollapseColumn {
                row_index,
                column_index,
            } => {
                self.set_agent_column_collapsed(
                    self.active_workspace,
                    row_index,
                    column_index,
                    true,
                );
            }
            AgentTabAction::CollapseOtherColumns {
                row_index,
                column_index,
            } => {
                self.collapse_other_agent_columns(self.active_workspace, row_index, column_index);
            }
            AgentTabAction::Restart(slot) => {
                self.set_active_agent_slot(self.active_workspace, slot);
                self.set_active_app_dialog(Some(AppDialog::RestartAgent {
                    index: self.active_workspace,
                }));
            }
            AgentTabAction::Switch { slot, next_kind } => {
                self.set_active_agent_slot(self.active_workspace, slot);
                self.set_active_app_dialog(Some(AppDialog::SwitchAgent {
                    index: self.active_workspace,
                    next_kind,
                }));
            }
            AgentTabAction::SetModel { slot, model } => {
                self.set_active_agent_slot(self.active_workspace, slot.clone());
                self.set_active_app_dialog(Some(AppDialog::SetAgentModel {
                    index: self.active_workspace,
                    slot,
                    model,
                }));
            }
            AgentTabAction::SetModelProvider {
                slot,
                model_provider,
            } => {
                self.set_active_agent_slot(self.active_workspace, slot.clone());
                self.set_active_app_dialog(Some(AppDialog::SetAgentModelProvider {
                    index: self.active_workspace,
                    slot,
                    model_provider,
                    model_providers: data::load_codex_model_provider_names(),
                }));
            }
            AgentTabAction::SetEffort { slot, effort } => {
                self.set_active_agent_slot(self.active_workspace, slot.clone());
                self.set_agent_slot_effort(ctx, self.active_workspace, slot, effort);
            }
            AgentTabAction::SetFastMode { slot, fast_mode } => {
                self.set_active_agent_slot(self.active_workspace, slot.clone());
                self.set_agent_slot_fast_mode(ctx, self.active_workspace, slot, fast_mode);
            }
            AgentTabAction::SetWorkDir { slot, work_dir } => {
                self.set_active_agent_slot(self.active_workspace, slot.clone());
                self.set_active_app_dialog(Some(AppDialog::SetAgentWorkDir {
                    index: self.active_workspace,
                    slot,
                    work_dir,
                }));
            }
            AgentTabAction::CopySessionId(session_id) => {
                ctx.copy_text(session_id);
                self.push_toast(
                    i18n::text(self.app_language, "Session ID copied"),
                    theme::success(),
                );
            }
            AgentTabAction::SetMarkdownOutlineCollapsed(collapsed) => {
                if let Some(document) = self.active_document_mut() {
                    document.markdown_outline_collapsed = collapsed;
                }
                if let Some(workspace) = self.current_workspace_mut() {
                    workspace.markdown_outline_collapsed = collapsed;
                    self.persist_workspaces();
                }
            }
            AgentTabAction::MoveSubagentLeft { column_id, id, .. } => {
                self.move_subagent_left(ctx, self.active_workspace, &column_id, &id);
            }
            AgentTabAction::MoveSubagentRight { column_id, id, .. } => {
                self.move_subagent_right(ctx, self.active_workspace, &column_id, &id);
            }
            AgentTabAction::MoveSubagentToHead { column_id, id, .. } => {
                self.move_subagent_to_head(ctx, self.active_workspace, &column_id, &id);
            }
            AgentTabAction::MoveSubagentToTail { column_id, id, .. } => {
                self.move_subagent_to_tail(ctx, self.active_workspace, &column_id, &id);
            }
            AgentTabAction::MoveSubagentToWorkspace { id, target_index } => {
                self.move_subagent_to_workspace(ctx, self.active_workspace, &id, target_index);
            }
        }
    }

    pub(super) fn toggle_workspace_terminal_drawer(&mut self) {
        if self.workspaces.is_empty() {
            return;
        }
        if self.active_workspace >= self.workspace_terminal_drawers.len() {
            self.workspace_terminal_drawers
                .resize(self.workspaces.len(), false);
        }
        if let Some(open) = self.reviewer_helix_drawers.get_mut(self.active_workspace) {
            *open = false;
        }
        if let Some(open) = self
            .workspace_terminal_drawers
            .get_mut(self.active_workspace)
        {
            *open = !*open;
        }
        self.queue_app_event(AppEvent::SyncTerminalEventRepaintFlags);
    }

    pub(super) fn open_reviewer_helix_drawer(&mut self, ctx: &egui::Context) {
        if self.workspaces.is_empty() {
            return;
        }
        if !self.helix_binary_available {
            self.spawn_helix_binary_check_task(ctx, HelixOpenRequest::ReviewerSelection);
            return;
        }
        self.open_checked_helix_request(ctx, HelixOpenRequest::ReviewerSelection);
    }

    pub(super) fn open_checked_reviewer_helix_drawer(&mut self, ctx: &egui::Context) {
        self.ensure_active_reviewer();
        let Some(target) = self
            .active_reviewer_adapter_mut()
            .and_then(|adapter| adapter.selected_helix_target())
        else {
            self.push_toast(
                i18n::text(self.app_language, "No reviewer target for Helix"),
                theme::warning(),
            );
            return;
        };
        let spec = HelixLaunchSpec {
            workdir: target.workdir,
            file: target.file,
            line: target.line,
        };
        self.ensure_helix_host(ctx, spec, HelixReusePolicy::ExactTarget);
        if self.active_workspace >= self.reviewer_helix_drawers.len() {
            self.reviewer_helix_drawers
                .resize(self.workspaces.len(), false);
        }
        if let Some(open) = self
            .workspace_terminal_drawers
            .get_mut(self.active_workspace)
        {
            *open = false;
        }
        if let Some(open) = self.reviewer_helix_drawers.get_mut(self.active_workspace) {
            *open = true;
        }
        self.queue_app_event(AppEvent::SyncTerminalEventRepaintFlags);
    }

    pub(super) fn open_workspace_helix_drawer(&mut self, ctx: &egui::Context) {
        if !self.helix_binary_available {
            self.spawn_helix_binary_check_task(ctx, HelixOpenRequest::WorkspaceRoot);
            return;
        }
        self.open_checked_helix_request(ctx, HelixOpenRequest::WorkspaceRoot);
    }

    /// Opens Helix for a file target resolved by the terminal surface.
    pub(super) fn open_terminal_file_helix_drawer(
        &mut self,
        ctx: &egui::Context,
        spec: HelixLaunchSpec,
    ) {
        self.record_recent_agent_helix_target(spec.clone());
        if !self.helix_binary_available {
            self.spawn_helix_binary_check_task(ctx, HelixOpenRequest::TerminalFile(spec));
            return;
        }
        self.ensure_helix_host(ctx, spec, HelixReusePolicy::ExactTarget);
        self.open_helix_drawer_for_active_workspace();
    }

    pub(super) fn open_checked_workspace_helix_drawer(&mut self, ctx: &egui::Context) {
        let Some(workdir) = self.active_agent_or_workspace_work_dir() else {
            return;
        };
        self.ensure_helix_host(
            ctx,
            HelixLaunchSpec {
                workdir,
                file: None,
                line: None,
            },
            HelixReusePolicy::SameWorkdir,
        );
        self.open_helix_drawer_for_active_workspace();
    }

    pub(super) fn open_checked_helix_request(
        &mut self,
        ctx: &egui::Context,
        request: HelixOpenRequest,
    ) {
        match request {
            HelixOpenRequest::ReviewerSelection => self.open_checked_reviewer_helix_drawer(ctx),
            HelixOpenRequest::WorkspaceRoot => self.open_checked_workspace_helix_drawer(ctx),
            HelixOpenRequest::TerminalFile(spec) => {
                self.ensure_helix_host(ctx, spec, HelixReusePolicy::ExactTarget);
                self.open_helix_drawer_for_active_workspace();
            }
        }
    }

    pub(super) fn open_helix_drawer_for_active_workspace(&mut self) {
        if self.active_workspace >= self.reviewer_helix_drawers.len() {
            self.reviewer_helix_drawers
                .resize(self.workspaces.len(), false);
        }
        if let Some(open) = self
            .workspace_terminal_drawers
            .get_mut(self.active_workspace)
        {
            *open = false;
        }
        if let Some(open) = self.reviewer_helix_drawers.get_mut(self.active_workspace) {
            *open = true;
        }
        self.queue_app_event(AppEvent::SyncTerminalEventRepaintFlags);
    }

    pub(super) fn toggle_reviewer_helix_drawer(&mut self, ctx: &egui::Context) {
        if self
            .current_workspace()
            .is_some_and(|workspace| workspace.route != Route::Reviewer)
        {
            if self.reviewer_helix_drawer_is_open() {
                if let Some(open) = self.reviewer_helix_drawers.get_mut(self.active_workspace) {
                    *open = false;
                }
                self.queue_app_event(AppEvent::SyncTerminalEventRepaintFlags);
            } else {
                self.open_workspace_helix_drawer(ctx);
            }
            return;
        }
        if self.reviewer_helix_drawer_is_open() {
            if let Some(open) = self.reviewer_helix_drawers.get_mut(self.active_workspace) {
                *open = false;
            }
            self.queue_app_event(AppEvent::SyncTerminalEventRepaintFlags);
        } else {
            self.open_reviewer_helix_drawer(ctx);
        }
    }

    /// 切换最近 Agent Helix 目标弹窗。
    pub(super) fn toggle_recent_agent_helix_targets_dialog(&mut self) {
        if matches!(
            self.active_app_dialog(),
            Some(AppDialog::RecentAgentHelixTargets)
        ) {
            self.set_active_app_dialog(None);
            return;
        }
        self.set_active_app_dialog(Some(AppDialog::RecentAgentHelixTargets));
    }

    /// 记录最近通过 Agent 文件行打开的 Helix 目标。
    pub(super) fn record_recent_agent_helix_target(&mut self, spec: HelixLaunchSpec) {
        let Some(targets) = self
            .recent_agent_helix_targets
            .get_mut(self.active_workspace)
        else {
            return;
        };
        let target = RecentHelixTarget {
            workdir: spec.workdir,
            file: spec.file,
            line: spec.line,
        };
        if let Some(index) = targets.iter().position(|current| current == &target) {
            targets.remove(index);
        }
        targets.insert(0, target);
        targets.truncate(10);
    }

    /// 从最近列表重新打开 Agent Helix 目标。
    pub(super) fn open_recent_agent_helix_target(
        &mut self,
        ctx: &egui::Context,
        target: RecentHelixTarget,
    ) {
        let spec = HelixLaunchSpec {
            workdir: target.workdir,
            file: target.file,
            line: target.line,
        };
        self.open_terminal_file_helix_drawer(ctx, spec);
    }

    /// Returns the workdir used for opening the generic Helix drawer.
    pub(super) fn active_agent_or_workspace_work_dir(&self) -> Option<PathBuf> {
        let workspace_path = self
            .current_workspace()
            .map(|workspace| workspace.path.clone())?;
        let slot = self.active_agent_slot();
        self.agent_slot_work_dir(self.active_workspace, &slot)
            .filter(|path| path.is_dir())
            .or(Some(workspace_path))
    }
}

/// 规范化 Helix 当前光标 hook 数据。
pub(super) fn normalize_helix_current_hook_data(data: &str) -> Option<String> {
    let (file, line) = data.rsplit_once(':')?;
    if file.trim().is_empty() || line.trim().is_empty() {
        return None;
    }
    Some(format!("{file}:{line}"))
}

/// 将 hook status 字符串转为 workspace activity。
fn workspace_activity_from_hook_status(status: &str) -> WorkspaceActivity {
    match status.trim().to_ascii_lowercase().as_str() {
        "busy" => WorkspaceActivity::Busy,
        "idle" => WorkspaceActivity::Idle,
        _ => WorkspaceActivity::Unknown,
    }
}

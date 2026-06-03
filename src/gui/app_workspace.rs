//! Workspace、route 和 agent slot 状态业务。
//!
//! 本模块只修改 app 内存状态并按需标记 store dirty；持久化写盘仍交给
//! store writer 合并处理。

use super::*;

impl GsdvGuiApp {
    pub(super) fn open_reviewer_route(&mut self) {
        let Some(workspace) = self.current_workspace_mut() else {
            return;
        };
        workspace.previous_center_mode = workspace.center_mode;
        workspace.route = Route::Reviewer;
        self.persist_workspaces();
        self.ensure_active_reviewer();
    }

    pub(super) fn exit_reviewer_route(&mut self) {
        if let Some(workspace) = self.current_workspace_mut() {
            workspace.route = Route::Workspace;
            workspace.center_mode = workspace.previous_center_mode;
        }
        self.persist_workspaces();
    }

    /// Switches to the next Busy workspace and skips idle or unknown tabs.
    pub(super) fn switch_active_workspace_forward(&mut self) {
        self.switch_workspace_by_activity(true);
    }

    /// Switches to the next non-Busy workspace.
    pub(super) fn switch_inactive_workspace_forward(&mut self) {
        self.switch_workspace_by_activity(false);
    }

    /// Switches workspace by the aggregate Busy state.
    pub(super) fn switch_workspace_by_activity(&mut self, busy: bool) {
        if self.workspaces.len() <= 1 {
            return;
        }
        let candidates = self
            .workspaces
            .iter()
            .enumerate()
            .filter_map(|(index, workspace)| {
                let is_busy = workspace_effective_activity(workspace) == WorkspaceActivity::Busy;
                (is_busy == busy).then_some(index)
            })
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return;
        }
        let position = candidates
            .iter()
            .position(|index| *index == self.active_workspace);
        let next = match position {
            Some(position) => candidates.get((position + 1) % candidates.len()).copied(),
            None => candidates
                .iter()
                .copied()
                .find(|index| *index > self.active_workspace)
                .or_else(|| candidates.first().copied()),
        };
        if let Some(next) = next {
            self.switch_workspace(next);
        }
    }

    pub(super) fn handle_workspace_rail_action(
        &mut self,
        _ctx: &egui::Context,
        action: WorkspaceRailAction,
    ) {
        match action {
            WorkspaceRailAction::Switch(index) => self.switch_workspace(index),
            WorkspaceRailAction::Close(index) => self.request_close_workspace(index),
        }
    }

    pub(super) fn add_workspace_from_dialog(&mut self, ctx: &egui::Context) {
        let start_dir = self
            .current_workspace()
            .map(|workspace| workspace.path.clone())
            .or_else(|| std::env::current_dir().ok());
        let mut dialog = rfd::FileDialog::new().set_title("Add Workspace");
        if let Some(start_dir) = start_dir {
            dialog = dialog.set_directory(start_dir);
        }
        let Some(path) = dialog.pick_folder() else {
            return;
        };
        let existing_paths = self
            .workspaces
            .iter()
            .map(|workspace| workspace.path.clone())
            .collect();
        self.spawn_workspace_add_task(ctx, path, self.default_agent_kind, existing_paths);
    }

    /// 应用添加 workspace 后台任务结果。
    pub(super) fn apply_workspace_add_result(
        &mut self,
        ctx: &egui::Context,
        result: Result<WorkspaceAddTaskResult, String>,
    ) {
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                self.set_active_app_dialog(Some(AppDialog::Message {
                    title: i18n::text(self.app_language, "Add Workspace Failed").to_string(),
                    message: error,
                }));
                return;
            }
        };
        match result {
            WorkspaceAddTaskResult::Existing { index, path } => {
                if self
                    .workspaces
                    .get(index)
                    .is_some_and(|workspace| workspace.path == path)
                {
                    self.switch_workspace(index);
                } else if let Some(index) = self
                    .workspaces
                    .iter()
                    .position(|workspace| workspace.path == path)
                {
                    self.switch_workspace(index);
                }
                return;
            }
            WorkspaceAddTaskResult::New { workspace } => {
                if let Some(index) = self
                    .workspaces
                    .iter()
                    .position(|current| current.path == workspace.path)
                {
                    self.switch_workspace(index);
                    return;
                }
                self.workspaces.push(workspace);
            }
        }
        self.spawn_runtime_fonts_apply_task(ctx, self.font_settings.clone());
        self.reviewer_adapters.push(None);
        self.reviewer_snapshots.push(None);
        self.reviewer_dialogs.push(None);
        self.reviewer_diff_scroll_targets.push(None);
        self.reviewer_diff_selected_rows.push(None);
        self.terminal_hosts.push(WorkspaceTerminalHosts::default());
        self.active_agent_slots.push(AgentSlotId::Main);
        self.agent_busy_watchdogs.push(BTreeMap::from([(
            AgentSlotId::Main,
            AgentBusyWatchdogState::default(),
        )]));
        self.workspace_terminal_drawers.push(false);
        self.reviewer_helix_drawers.push(false);
        self.pending_agent_theme_restarts.push(None);
        self.documents.push(DocumentState::default());
        self.app_dialogs.push(None);
        self.global_app_dialog = None;
        self.outline_tree_rects.push(None);
        self.outline_favorites_only.push(false);
        self.outline_panel_tabs.push(OutlinePanelTab::Outline);
        self.workflow_states.push(WorkflowUiState::default());
        self.memo_save_errors.push(None);
        self.active_workspace = self.workspaces.len().saturating_sub(1);
        self.mark_extra_tools_scan_due();
        self.queue_app_event(AppEvent::SyncTerminalEventRepaintFlags);
        self.sync_fs_watches();
        self.persist_workspaces();
    }

    /// Opens the destructive close confirmation for a workspace.
    pub(super) fn request_close_workspace(&mut self, index: usize) {
        if index >= self.workspaces.len() {
            return;
        }
        self.set_active_app_dialog(Some(AppDialog::CloseWorkspace { index }));
    }

    /// 确认关闭 workspace 后派发 sidecar 删除任务。
    pub(super) fn close_workspace(&mut self, ctx: &egui::Context, index: usize) {
        if index >= self.workspaces.len() {
            return;
        }
        let workspace_path = self.workspaces[index].path.clone();
        self.set_active_app_dialog(Some(AppDialog::Message {
            title: i18n::text(self.app_language, "Close Workspace").to_string(),
            message: i18n::text(self.app_language, "Closing workspace...").to_string(),
        }));
        self.spawn_workspace_close_sidecar_delete_task(ctx, index, workspace_path);
    }

    /// 应用 workspace sidecar 删除结果，并只在成功后移除 UI 状态。
    pub(super) fn apply_workspace_close_sidecar_result(
        &mut self,
        _ctx: &egui::Context,
        index: usize,
        workspace_path: PathBuf,
        result: Result<(), String>,
    ) {
        if let Err(error) = result {
            self.set_active_app_dialog(Some(AppDialog::Message {
                title: i18n::text(self.app_language, "Close Workspace Failed").to_string(),
                message: error,
            }));
            return;
        }
        let Some(index) = self
            .workspaces
            .get(index)
            .is_some_and(|workspace| workspace.path == workspace_path)
            .then_some(index)
            .or_else(|| {
                self.workspaces
                    .iter()
                    .position(|workspace| workspace.path == workspace_path)
            })
        else {
            self.set_active_app_dialog(None);
            return;
        };
        self.workspaces.remove(index);
        self.reviewer_adapters.remove(index);
        self.reviewer_snapshots.remove(index);
        self.pending_reviewer_adapter_tasks.clear();
        self.reviewer_git_data_in_flight.clear();
        self.pending_reviewer_git_data_budget.clear();
        self.reviewer_dialogs.remove(index);
        self.reviewer_diff_scroll_targets.remove(index);
        self.reviewer_diff_selected_rows.remove(index);
        self.terminal_hosts.remove(index);
        self.active_agent_slots.remove(index);
        self.agent_busy_watchdogs.remove(index);
        self.workspace_terminal_drawers.remove(index);
        self.reviewer_helix_drawers.remove(index);
        self.pending_agent_theme_restarts.remove(index);
        self.documents.remove(index);
        self.app_dialogs.remove(index);
        self.outline_tree_rects.remove(index);
        self.outline_favorites_only.remove(index);
        self.outline_panel_tabs.remove(index);
        self.workflow_states.remove(index);
        self.memo_save_errors.remove(index);
        if self.workspaces.is_empty() {
            self.active_workspace = 0;
        } else if self.active_workspace == index {
            self.active_workspace = index.saturating_sub(1).min(self.workspaces.len() - 1);
        } else if self.active_workspace > index {
            self.active_workspace = self.active_workspace.saturating_sub(1);
        }
        self.mark_extra_tools_scan_due();
        self.queue_app_event(AppEvent::SyncTerminalEventRepaintFlags);
        self.sync_fs_watches();
        self.persist_workspaces();
        self.set_active_app_dialog(None);
    }

    pub(super) fn switch_workspace(&mut self, index: usize) {
        if index < self.workspaces.len() && index != self.active_workspace {
            self.clear_agent_input_translation_state();
            self.active_workspace = index;
            self.mark_extra_tools_scan_due();
            self.queue_app_event(AppEvent::SyncTerminalEventRepaintFlags);
            self.persist_workspaces();
        }
    }

    /// Selects main or one of the first two subagents by shortcut index.
    pub(super) fn select_agent_slot_by_index(&mut self, slot_index: usize) {
        let Some(workspace) = self.workspaces.get(self.active_workspace) else {
            return;
        };
        let slot = match slot_index {
            0 => AgentSlotId::Main,
            1 => workspace
                .subagents
                .first()
                .map(|subagent| AgentSlotId::Subagent(subagent.id.clone()))
                .unwrap_or(AgentSlotId::Main),
            2 => workspace
                .subagents
                .get(1)
                .map(|subagent| AgentSlotId::Subagent(subagent.id.clone()))
                .unwrap_or(AgentSlotId::Main),
            _ => return,
        };
        if slot_index > 0 && slot == AgentSlotId::Main {
            return;
        }
        if let Some(active) = self.active_agent_slots.get_mut(self.active_workspace) {
            *active = slot;
        }
    }

    /// Adds a named subagent to a workspace and selects it.
    pub(super) fn add_subagent(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        name: String,
        agent_kind: AgentKind,
        agent_model: Option<String>,
        agent_effort: Option<String>,
        agent_fast_mode: Option<bool>,
        agent_work_dir: Option<PathBuf>,
        session_id: Option<String>,
    ) {
        let Some(workspace) = self.workspaces.get_mut(index) else {
            return;
        };
        let subagent = data::new_subagent(
            &workspace.path,
            name,
            agent_kind,
            agent_model,
            agent_effort,
            agent_fast_mode,
            agent_work_dir,
            session_id,
        );
        let slot = AgentSlotId::Subagent(subagent.id.clone());
        workspace.subagents.push(subagent);
        if let Some(active) = self.active_agent_slots.get_mut(index) {
            *active = slot.clone();
        }
        if let Some(states) = self.agent_busy_watchdogs.get_mut(index) {
            states.entry(slot).or_default();
        }
        self.persist_workspaces();
        self.spawn_terminal_host_for_workspace(ctx, index, TerminalSurfaceKind::Agent);
    }

    /// Removes one subagent and shuts down its terminal host.
    pub(super) fn remove_subagent(&mut self, ctx: &egui::Context, index: usize, id: &str) {
        let slot = AgentSlotId::Subagent(id.to_string());
        let Some(workspace) = self.workspaces.get_mut(index) else {
            return;
        };
        let before = workspace.subagents.len();
        workspace.subagents.retain(|subagent| subagent.id != id);
        if workspace.subagents.len() == before {
            return;
        }
        if self.active_agent_slots.get(index) == Some(&slot) {
            if let Some(active) = self.active_agent_slots.get_mut(index) {
                *active = AgentSlotId::Main;
            }
        }
        if let Some(hosts) = self.terminal_hosts.get_mut(index) {
            if let Some(mut agent) = hosts.agents.remove(&slot)
                && let Some(host) = agent.host.as_mut()
            {
                host.request_graceful_exit();
            }
        }
        if let Some(states) = self.agent_busy_watchdogs.get_mut(index) {
            states.remove(&slot);
        }
        self.persist_workspaces();
        self.request_app_repaint(ctx);
    }

    /// Moves one subagent one tab to the left inside its workspace.
    pub(super) fn move_subagent_left(&mut self, ctx: &egui::Context, index: usize, id: &str) {
        let Some(current) = self.subagent_index(index, id) else {
            return;
        };
        if current == 0 {
            return;
        }
        self.move_subagent_to_position(ctx, index, id, current - 1);
    }

    /// Moves one subagent one tab to the right inside its workspace.
    pub(super) fn move_subagent_right(&mut self, ctx: &egui::Context, index: usize, id: &str) {
        let Some(current) = self.subagent_index(index, id) else {
            return;
        };
        self.move_subagent_to_position(ctx, index, id, current + 1);
    }

    /// Moves one subagent to the first subagent tab inside its workspace.
    pub(super) fn move_subagent_to_head(&mut self, ctx: &egui::Context, index: usize, id: &str) {
        self.move_subagent_to_position(ctx, index, id, 0);
    }

    /// Moves one subagent to the last subagent tab inside its workspace.
    pub(super) fn move_subagent_to_tail(&mut self, ctx: &egui::Context, index: usize, id: &str) {
        let Some(workspace) = self.workspaces.get(index) else {
            return;
        };
        let Some(target) = workspace.subagents.len().checked_sub(1) else {
            return;
        };
        self.move_subagent_to_position(ctx, index, id, target);
    }

    /// Moves one subagent from the source workspace to another workspace.
    pub(super) fn move_subagent_to_workspace(
        &mut self,
        ctx: &egui::Context,
        source_index: usize,
        id: &str,
        target_index: usize,
    ) {
        if source_index == target_index
            || source_index >= self.workspaces.len()
            || target_index >= self.workspaces.len()
        {
            return;
        }
        if self.workspaces[target_index]
            .subagents
            .iter()
            .any(|subagent| subagent.id == id)
        {
            self.push_toast(
                i18n::text(
                    self.app_language,
                    "Subagent already exists in target workspace",
                ),
                theme::warning(),
            );
            return;
        }

        let Some(source_position) = self.subagent_index(source_index, id) else {
            return;
        };
        let slot = AgentSlotId::Subagent(id.to_string());
        let subagent = self.workspaces[source_index]
            .subagents
            .remove(source_position);
        self.workspaces[target_index].subagents.push(subagent);

        if self.active_agent_slots.get(source_index) == Some(&slot)
            && let Some(active) = self.active_agent_slots.get_mut(source_index)
        {
            *active = AgentSlotId::Main;
        }
        if let Some(active) = self.active_agent_slots.get_mut(target_index) {
            *active = slot.clone();
        }

        let busy_state = self
            .agent_busy_watchdogs
            .get_mut(source_index)
            .and_then(|states| states.remove(&slot));
        if let Some(states) = self.agent_busy_watchdogs.get_mut(target_index) {
            states.insert(slot.clone(), busy_state.unwrap_or_default());
        }

        // Trigger: moving across workspaces changes the terminal launch root.
        // Why: an already-running child process cannot change its original cwd.
        // Prevents: a moved tab showing a target workspace while the PTY still
        // executes commands in the old workspace.
        if let Some(hosts) = self.terminal_hosts.get_mut(source_index)
            && let Some(mut agent) = hosts.agents.remove(&slot)
            && let Some(host) = agent.host.as_mut()
        {
            host.request_graceful_exit();
        }
        if let Some(hosts) = self.terminal_hosts.get_mut(target_index) {
            hosts.agents.remove(&slot);
        }

        self.persist_workspaces();
        self.request_app_repaint(ctx);
    }

    /// Returns the current subagent index inside one workspace.
    fn subagent_index(&self, index: usize, id: &str) -> Option<usize> {
        self.workspaces
            .get(index)?
            .subagents
            .iter()
            .position(|subagent| subagent.id == id)
    }

    /// Reorders one subagent inside a workspace and persists the sidecar.
    fn move_subagent_to_position(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        id: &str,
        target: usize,
    ) {
        let Some(workspace) = self.workspaces.get_mut(index) else {
            return;
        };
        let Some(current) = workspace
            .subagents
            .iter()
            .position(|subagent| subagent.id == id)
        else {
            return;
        };
        let Some(last) = workspace.subagents.len().checked_sub(1) else {
            return;
        };
        let target = target.min(last);
        if current == target {
            return;
        }
        let subagent = workspace.subagents.remove(current);
        workspace.subagents.insert(target, subagent);
        self.persist_workspaces();
        self.request_app_repaint(ctx);
    }

    /// 标记 workspace metadata 和 subagent sidecar 需要异步保存。
    pub(super) fn persist_workspaces(&mut self) {
        self.mark_workspace_store_dirty();
    }
}

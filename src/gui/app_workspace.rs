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
        self.recent_agent_helix_targets.push(Vec::new());
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
        self.workflow_quick_overlay_dialogs.push(None);
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
        self.recent_agent_helix_targets.remove(index);
        self.active_agent_slots.remove(index);
        self.agent_busy_watchdogs.remove(index);
        self.workspace_terminal_drawers.remove(index);
        self.reviewer_helix_drawers.remove(index);
        self.pending_agent_theme_restarts.remove(index);
        self.documents.remove(index);
        self.app_dialogs.remove(index);
        self.workflow_quick_overlay_dialogs.remove(index);
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
            *active = slot.clone();
        }
        let column_slot = slot.to_column_slot();
        if let Some(workspace) = self.workspaces.get_mut(self.active_workspace)
            && let Some(column) = workspace
                .agent_rows
                .iter_mut()
                .flat_map(|row| row.columns.iter_mut())
                .find(|column| column.tabs.contains(&column_slot))
        {
            column.active_slot = column_slot;
        }
    }

    /// Adds a named subagent to a workspace and selects it.
    pub(super) fn add_subagent(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        column_id: String,
        name: String,
        agent_kind: AgentKind,
        agent_model: Option<String>,
        agent_model_provider: Option<String>,
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
            agent_model_provider,
            agent_effort,
            agent_fast_mode,
            agent_work_dir,
            session_id,
        );
        let slot = AgentSlotId::Subagent(subagent.id.clone());
        let column_slot = slot.to_column_slot();
        workspace.subagents.push(subagent);
        let mut next_focus = None;
        if let Some((row_index, column_index, column)) = workspace
            .agent_rows
            .iter_mut()
            .enumerate()
            .find_map(|(row_index, row)| {
                row.columns
                    .iter_mut()
                    .enumerate()
                    .find(|(_, column)| column.id == column_id)
                    .map(|(column_index, column)| (row_index, column_index, column))
            })
        {
            column.tabs.push(column_slot.clone());
            column.active_slot = column_slot;
            next_focus = Some(data::AgentFocusViewData {
                row_index,
                column_index,
            });
        } else if let Some(column) = workspace
            .agent_rows
            .first_mut()
            .and_then(|row| row.columns.first_mut())
        {
            column.tabs.push(column_slot.clone());
            column.active_slot = column_slot;
            next_focus = Some(data::AgentFocusViewData {
                row_index: 0,
                column_index: 0,
            });
        }
        workspace.agent_focus = next_focus;
        if let Some(active) = self.active_agent_slots.get_mut(index) {
            *active = slot.clone();
        }
        if let Some(states) = self.agent_busy_watchdogs.get_mut(index) {
            states.entry(slot).or_default();
        }
        self.persist_workspaces();
        self.spawn_terminal_host_for_workspace(ctx, index, TerminalSurfaceKind::Agent);
    }

    /// Adds a visible Agent column to one row and returns its stable id.
    pub(super) fn add_agent_column(&mut self, index: usize, row_index: usize) -> Option<String> {
        let workspace = self.workspaces.get_mut(index)?;
        let row = workspace.agent_rows.get_mut(row_index)?;
        let id = new_agent_column_id(row.columns.len());
        row.columns.push(data::AgentColumnViewData {
            id: id.clone(),
            tabs: Vec::new(),
            active_slot: data::AgentColumnSlot::Main,
            width_weight: 1.0,
            collapsed: false,
        });
        data::normalize_agent_column_widths(&mut row.columns);
        self.persist_workspaces();
        Some(id)
    }

    /// Adds a visible Agent row below the given row and returns its first column id.
    pub(super) fn add_agent_row(&mut self, index: usize, after_row: usize) -> Option<String> {
        let workspace = self.workspaces.get_mut(index)?;
        let id = new_agent_column_id(0);
        let insert_at = (after_row + 1).min(workspace.agent_rows.len());
        workspace.agent_rows.insert(
            insert_at,
            data::AgentRowViewData {
                columns: vec![data::AgentColumnViewData {
                    id: id.clone(),
                    tabs: Vec::new(),
                    active_slot: data::AgentColumnSlot::Main,
                    width_weight: 1.0,
                    collapsed: false,
                }],
                height_weight: 1.0,
                collapsed: false,
            },
        );
        data::normalize_agent_row_heights(&mut workspace.agent_rows);
        self.persist_workspaces();
        Some(id)
    }

    /// Removes a non-primary Agent column and deletes all subagents inside it.
    pub(super) fn remove_agent_column(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        column_id: &str,
    ) {
        let Some((row_index, column_index, slots)) =
            self.workspaces.get(index).and_then(|workspace| {
                workspace
                    .agent_rows
                    .iter()
                    .enumerate()
                    .find_map(|(row_index, row)| {
                        row.columns
                            .iter()
                            .enumerate()
                            .find(|(_, column)| column.id == column_id)
                            .map(|(column_index, column)| {
                                (row_index, column_index, column.tabs.clone())
                            })
                    })
            })
        else {
            return;
        };
        if row_index == 0 && column_index == 0 {
            return;
        }
        for slot in slots {
            if let data::AgentColumnSlot::Subagent(id) = slot {
                self.remove_subagent(ctx, index, &id);
            }
        }
        if let Some(workspace) = self.workspaces.get_mut(index) {
            if let Some(row) = workspace.agent_rows.get_mut(row_index) {
                row.columns.retain(|column| column.id != column_id);
                data::normalize_agent_column_widths(&mut row.columns);
            }
            workspace.agent_rows.retain(|row| !row.columns.is_empty());
            workspace.agent_focus =
                data::normalize_agent_focus_for_rows(workspace.agent_focus, &workspace.agent_rows);
        }
        self.persist_workspaces();
        self.request_app_repaint();
    }

    /// Removes a non-primary Agent row and deletes all subagents inside it.
    pub(super) fn remove_agent_row(&mut self, ctx: &egui::Context, index: usize, row_index: usize) {
        if row_index == 0 {
            return;
        }
        let Some(slots) = self
            .workspaces
            .get(index)
            .and_then(|workspace| workspace.agent_rows.get(row_index))
            .map(|row| {
                row.columns
                    .iter()
                    .flat_map(|column| column.tabs.iter().cloned())
                    .collect::<Vec<_>>()
            })
        else {
            return;
        };
        for slot in slots {
            if let data::AgentColumnSlot::Subagent(id) = slot {
                self.remove_subagent(ctx, index, &id);
            }
        }
        if let Some(workspace) = self.workspaces.get_mut(index)
            && row_index < workspace.agent_rows.len()
        {
            workspace.agent_rows.remove(row_index);
            data::normalize_agent_row_heights(&mut workspace.agent_rows);
            workspace.agent_focus =
                data::normalize_agent_focus_for_rows(workspace.agent_focus, &workspace.agent_rows);
        }
        self.persist_workspaces();
        self.request_app_repaint();
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
        let column_slot = slot.to_column_slot();
        for row in &mut workspace.agent_rows {
            for column in &mut row.columns {
                column.tabs.retain(|tab| tab != &column_slot);
                if column.active_slot == column_slot {
                    column.active_slot = column
                        .tabs
                        .first()
                        .cloned()
                        .unwrap_or(data::AgentColumnSlot::Main);
                }
            }
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
        self.request_app_repaint();
    }

    /// Moves one subagent one tab to the left inside its Agent column.
    pub(super) fn move_subagent_left(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        column_id: &str,
        id: &str,
    ) {
        let Some(current) = self.column_subagent_index(index, column_id, id) else {
            return;
        };
        if current
            <= self
                .column_first_movable_index(index, column_id)
                .unwrap_or(0)
        {
            return;
        }
        self.move_subagent_to_position(ctx, index, column_id, id, current - 1);
    }

    /// Moves one subagent one tab to the right inside its Agent column.
    pub(super) fn move_subagent_right(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        column_id: &str,
        id: &str,
    ) {
        let Some(current) = self.column_subagent_index(index, column_id, id) else {
            return;
        };
        self.move_subagent_to_position(ctx, index, column_id, id, current + 1);
    }

    /// Moves one subagent to the first subagent tab inside its Agent column.
    pub(super) fn move_subagent_to_head(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        column_id: &str,
        id: &str,
    ) {
        self.move_subagent_to_position(ctx, index, column_id, id, 0);
    }

    /// Moves one subagent to the last subagent tab inside its Agent column.
    pub(super) fn move_subagent_to_tail(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        column_id: &str,
        id: &str,
    ) {
        let Some(column) = self.workspaces.get(index).and_then(|workspace| {
            workspace
                .agent_rows
                .iter()
                .find_map(|row| row.columns.iter().find(|column| column.id == column_id))
        }) else {
            return;
        };
        let Some(target) = column.tabs.len().checked_sub(1) else {
            return;
        };
        self.move_subagent_to_position(ctx, index, column_id, id, target);
    }

    /// Moves one subagent to a neighboring column in the same Agent row.
    ///
    /// Example: row 0 col 1 with `direction = -1` -> append to row 0 col 0.
    pub(super) fn move_subagent_to_adjacent_column(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        row_index: usize,
        column_index: usize,
        id: &str,
        direction: isize,
    ) {
        let Some(target_column_index) = column_index.checked_add_signed(direction) else {
            return;
        };
        self.move_subagent_to_grid_cell(
            ctx,
            index,
            row_index,
            column_index,
            row_index,
            target_column_index,
            id,
        );
    }

    /// Moves one subagent to the first column of a neighboring Agent row.
    ///
    /// Example: row 2 col 3 with `direction = -1` -> append to row 1 col 0.
    pub(super) fn move_subagent_to_adjacent_row(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        row_index: usize,
        column_index: usize,
        id: &str,
        direction: isize,
    ) {
        let Some(target_row_index) = row_index.checked_add_signed(direction) else {
            return;
        };
        self.move_subagent_to_grid_cell(
            ctx,
            index,
            row_index,
            column_index,
            target_row_index,
            0,
            id,
        );
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
        let column_slot = slot.to_column_slot();
        let subagent = self.workspaces[source_index]
            .subagents
            .remove(source_position);
        self.workspaces[target_index].subagents.push(subagent);
        for row in &mut self.workspaces[source_index].agent_rows {
            for column in &mut row.columns {
                column.tabs.retain(|tab| tab != &column_slot);
                if column.active_slot == column_slot {
                    column.active_slot = column
                        .tabs
                        .first()
                        .cloned()
                        .unwrap_or(data::AgentColumnSlot::Main);
                }
            }
        }
        if let Some(column) = self.workspaces[target_index]
            .agent_rows
            .first_mut()
            .and_then(|row| row.columns.first_mut())
        {
            column.tabs.push(column_slot.clone());
            column.active_slot = column_slot;
        }

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
        self.request_app_repaint();
    }

    /// Moves one subagent tab between two Agent grid cells.
    ///
    /// Example: source row 0 col 0 and target row 1 col 0 -> target gets the
    /// subagent as its last tab and active slot.
    fn move_subagent_to_grid_cell(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        source_row_index: usize,
        source_column_index: usize,
        target_row_index: usize,
        target_column_index: usize,
        id: &str,
    ) {
        if source_row_index == target_row_index && source_column_index == target_column_index {
            return;
        }
        let slot = data::AgentColumnSlot::Subagent(id.to_string());
        let Some(workspace) = self.workspaces.get_mut(index) else {
            return;
        };
        if workspace
            .agent_rows
            .get(target_row_index)
            .and_then(|row| row.columns.get(target_column_index))
            .is_none()
        {
            return;
        }
        let Some(source_column) = workspace
            .agent_rows
            .get_mut(source_row_index)
            .and_then(|row| row.columns.get_mut(source_column_index))
        else {
            return;
        };
        let Some(source_tab_index) = source_column.tabs.iter().position(|tab| tab == &slot) else {
            return;
        };
        source_column.tabs.remove(source_tab_index);
        if source_column.active_slot == slot {
            source_column.active_slot = source_column
                .tabs
                .first()
                .cloned()
                .unwrap_or(data::AgentColumnSlot::Main);
        }

        let Some(target_column) = workspace
            .agent_rows
            .get_mut(target_row_index)
            .and_then(|row| row.columns.get_mut(target_column_index))
        else {
            return;
        };
        target_column.tabs.retain(|tab| tab != &slot);
        target_column.tabs.push(slot.clone());
        target_column.active_slot = slot.clone();
        workspace.agent_focus = Some(data::AgentFocusViewData {
            row_index: target_row_index,
            column_index: target_column_index,
        });
        if let Some(active) = self.active_agent_slots.get_mut(index) {
            *active = AgentSlotId::from_column_slot(&slot);
        }
        self.persist_workspaces();
        self.request_app_repaint();
        ctx.request_repaint();
    }

    /// Returns the current subagent index inside one workspace.
    fn subagent_index(&self, index: usize, id: &str) -> Option<usize> {
        self.workspaces
            .get(index)?
            .subagents
            .iter()
            .position(|subagent| subagent.id == id)
    }

    /// Returns the current subagent tab index inside one Agent column.
    fn column_subagent_index(&self, index: usize, column_id: &str, id: &str) -> Option<usize> {
        let slot = data::AgentColumnSlot::Subagent(id.to_string());
        self.workspaces
            .get(index)?
            .agent_rows
            .iter()
            .find_map(|row| row.columns.iter().find(|column| column.id == column_id))?
            .tabs
            .iter()
            .position(|tab| tab == &slot)
    }

    /// Reorders one subagent inside an Agent column and persists the sidecar.
    fn move_subagent_to_position(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        column_id: &str,
        id: &str,
        target: usize,
    ) {
        let first_movable = self
            .column_first_movable_index(index, column_id)
            .unwrap_or(0);
        let Some(workspace) = self.workspaces.get_mut(index) else {
            return;
        };
        let slot = data::AgentColumnSlot::Subagent(id.to_string());
        let Some(column) = workspace
            .agent_rows
            .iter_mut()
            .find_map(|row| row.columns.iter_mut().find(|column| column.id == column_id))
        else {
            return;
        };
        let Some(current) = column.tabs.iter().position(|tab| tab == &slot) else {
            return;
        };
        let Some(last) = column.tabs.len().checked_sub(1) else {
            return;
        };
        let target = target.max(first_movable).min(last);
        if current == target {
            return;
        }
        let tab = column.tabs.remove(current);
        column.tabs.insert(target, tab);
        self.persist_workspaces();
        self.request_app_repaint();
    }

    /// Returns the first movable tab index for one Agent column.
    fn column_first_movable_index(&self, index: usize, column_id: &str) -> Option<usize> {
        self.workspaces
            .get(index)?
            .agent_rows
            .iter()
            .enumerate()
            .find_map(|(row_index, row)| {
                row.columns
                    .iter()
                    .position(|column| column.id == column_id)
                    .map(|column_index| usize::from(row_index == 0 && column_index == 0))
            })
    }

    /// Sets focus to a visible Agent grid cell and routes keyboard input there.
    pub(super) fn set_agent_focus(&mut self, index: usize, row_index: usize, column_index: usize) {
        let Some(workspace) = self.workspaces.get_mut(index) else {
            return;
        };
        if workspace
            .agent_rows
            .get(row_index)
            .is_none_or(|row| row.collapsed)
            || workspace
                .agent_rows
                .get(row_index)
                .and_then(|row| row.columns.get(column_index))
                .is_none_or(|column| column.collapsed)
        {
            return;
        }
        workspace.agent_focus = Some(data::AgentFocusViewData {
            row_index,
            column_index,
        });
        let slot = workspace.agent_rows[row_index].columns[column_index]
            .active_slot
            .clone();
        if let Some(active) = self.active_agent_slots.get_mut(index) {
            *active = AgentSlotId::from_column_slot(&slot);
        }
        self.persist_workspaces();
        self.request_app_repaint();
    }

    /// Moves Agent focus with keyboard navigation inside visible grid cells.
    ///
    /// Example: `Right` at row 0 col 0 -> focus row 0 col 1 when that column is visible.
    pub(super) fn move_agent_focus(&mut self, index: usize, direction: AgentFocusMove) {
        let Some(workspace) = self.workspaces.get(index) else {
            return;
        };
        let Some(focus) =
            data::normalize_agent_focus_for_rows(workspace.agent_focus, &workspace.agent_rows)
        else {
            return;
        };
        let Some(next_focus) = next_agent_focus(&workspace.agent_rows, focus, direction) else {
            return;
        };
        self.set_agent_focus(index, next_focus.row_index, next_focus.column_index);
    }

    /// Normalizes focus after a row or column becomes folded or removed.
    pub(super) fn normalize_agent_focus(&mut self, index: usize) {
        let Some(workspace) = self.workspaces.get_mut(index) else {
            return;
        };
        workspace.agent_focus =
            data::normalize_agent_focus_for_rows(workspace.agent_focus, &workspace.agent_rows);
        if let Some(focus) = workspace.agent_focus
            && let Some(slot) = workspace
                .agent_rows
                .get(focus.row_index)
                .and_then(|row| row.columns.get(focus.column_index))
                .map(|column| AgentSlotId::from_column_slot(&column.active_slot))
            && let Some(active) = self.active_agent_slots.get_mut(index)
        {
            *active = slot;
        }
    }

    /// Updates one Agent column folded state and repairs focus.
    pub(super) fn set_agent_column_collapsed(
        &mut self,
        index: usize,
        row_index: usize,
        column_index: usize,
        collapsed: bool,
    ) {
        if let Some(column) = self
            .workspaces
            .get_mut(index)
            .and_then(|workspace| workspace.agent_rows.get_mut(row_index))
            .and_then(|row| row.columns.get_mut(column_index))
        {
            column.collapsed = collapsed;
        }
        self.normalize_agent_focus(index);
        self.persist_workspaces();
        self.request_app_repaint();
    }

    /// Folds every column except one inside the selected row.
    pub(super) fn collapse_other_agent_columns(
        &mut self,
        index: usize,
        row_index: usize,
        column_index: usize,
    ) {
        if let Some(row) = self
            .workspaces
            .get_mut(index)
            .and_then(|workspace| workspace.agent_rows.get_mut(row_index))
        {
            for (index, column) in row.columns.iter_mut().enumerate() {
                if index != column_index {
                    column.collapsed = true;
                }
            }
        }
        self.normalize_agent_focus(index);
        self.persist_workspaces();
        self.request_app_repaint();
    }

    /// Updates one Agent row folded state and repairs focus.
    pub(super) fn set_agent_row_collapsed(
        &mut self,
        index: usize,
        row_index: usize,
        collapsed: bool,
    ) {
        if let Some(row) = self
            .workspaces
            .get_mut(index)
            .and_then(|workspace| workspace.agent_rows.get_mut(row_index))
        {
            row.collapsed = collapsed;
        }
        self.normalize_agent_focus(index);
        self.persist_workspaces();
        self.request_app_repaint();
    }

    /// Folds every Agent row except the selected one.
    pub(super) fn collapse_other_agent_rows(&mut self, index: usize, row_index: usize) {
        if let Some(workspace) = self.workspaces.get_mut(index) {
            for (index, row) in workspace.agent_rows.iter_mut().enumerate() {
                if index != row_index {
                    row.collapsed = true;
                }
            }
        }
        self.normalize_agent_focus(index);
        self.persist_workspaces();
        self.request_app_repaint();
    }

    /// 标记 workspace metadata 和 subagent sidecar 需要异步保存。
    pub(super) fn persist_workspaces(&mut self) {
        self.mark_workspace_store_dirty();
    }
}

/// Finds the next visible Agent focus cell for arrow-key movement.
///
/// Example: `Right` at `{ row: 0, col: 0 }` -> first visible column after col 0.
fn next_agent_focus(
    rows: &[data::AgentRowViewData],
    focus: data::AgentFocusViewData,
    direction: AgentFocusMove,
) -> Option<data::AgentFocusViewData> {
    match direction {
        AgentFocusMove::Left => previous_visible_agent_column(rows, focus),
        AgentFocusMove::Right => next_visible_agent_column(rows, focus),
        AgentFocusMove::Up => visible_agent_row_focus(rows, focus, false),
        AgentFocusMove::Down => visible_agent_row_focus(rows, focus, true),
    }
}

/// Finds the previous visible column in the focused row.
///
/// Example: focus col 2 with col 1 visible -> focus col 1.
fn previous_visible_agent_column(
    rows: &[data::AgentRowViewData],
    focus: data::AgentFocusViewData,
) -> Option<data::AgentFocusViewData> {
    let row = rows.get(focus.row_index)?;
    (0..focus.column_index)
        .rev()
        .find(|column_index| {
            row.columns
                .get(*column_index)
                .is_some_and(|column| !column.collapsed)
        })
        .map(|column_index| data::AgentFocusViewData {
            row_index: focus.row_index,
            column_index,
        })
}

/// Finds the next visible column in the focused row.
///
/// Example: focus col 0 with col 1 visible -> focus col 1.
fn next_visible_agent_column(
    rows: &[data::AgentRowViewData],
    focus: data::AgentFocusViewData,
) -> Option<data::AgentFocusViewData> {
    let row = rows.get(focus.row_index)?;
    ((focus.column_index + 1)..row.columns.len())
        .find(|column_index| {
            row.columns
                .get(*column_index)
                .is_some_and(|column| !column.collapsed)
        })
        .map(|column_index| data::AgentFocusViewData {
            row_index: focus.row_index,
            column_index,
        })
}

/// Finds a visible cell in a neighboring visible row.
///
/// Example: `down` from row 0 col 2 -> row 1 col 2, or the nearest visible column.
fn visible_agent_row_focus(
    rows: &[data::AgentRowViewData],
    focus: data::AgentFocusViewData,
    forward: bool,
) -> Option<data::AgentFocusViewData> {
    let range: Box<dyn Iterator<Item = usize>> = if forward {
        Box::new((focus.row_index + 1)..rows.len())
    } else {
        Box::new((0..focus.row_index).rev())
    };
    for row_index in range {
        let Some(row) = rows.get(row_index).filter(|row| !row.collapsed) else {
            continue;
        };
        if let Some(column_index) = nearest_visible_agent_column(row, focus.column_index) {
            return Some(data::AgentFocusViewData {
                row_index,
                column_index,
            });
        }
    }
    None
}

/// Finds the visible column nearest to the preferred column index.
///
/// Example: preferred col 3 in a two-column row -> visible col 1.
fn nearest_visible_agent_column(
    row: &data::AgentRowViewData,
    preferred_column_index: usize,
) -> Option<usize> {
    if row
        .columns
        .get(preferred_column_index)
        .is_some_and(|column| !column.collapsed)
    {
        return Some(preferred_column_index);
    }
    let mut best = None;
    let mut best_distance = usize::MAX;
    for (column_index, column) in row.columns.iter().enumerate() {
        if column.collapsed {
            continue;
        }
        let distance = column_index.abs_diff(preferred_column_index);
        if distance < best_distance {
            best = Some(column_index);
            best_distance = distance;
        }
    }
    best
}

/// Creates a stable-enough Agent column id for persisted UI layout.
fn new_agent_column_id(index: usize) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("col-{now:x}-{index:x}")
}

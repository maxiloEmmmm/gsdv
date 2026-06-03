//! Agent、主题切换和 terminal-backed session 重启业务。
//!
//! 本模块管理 agent slot 会话、主题切换后的重启，以及代理设置变化后的
//! 已打开终端重启。

use super::*;

impl GsdvGuiApp {
    /// Returns the opposite app theme for the top-level theme toggle.
    pub(super) fn next_theme_mode(&self) -> theme::ThemeMode {
        match self.theme_mode {
            theme::ThemeMode::Light => theme::ThemeMode::Dark,
            theme::ThemeMode::Dark => theme::ThemeMode::Light,
        }
    }

    /// Starts a theme switch, asking first when the active agent is running.
    pub(super) fn request_theme_switch(&mut self, ctx: &egui::Context) {
        let next_mode = self.next_theme_mode();
        if self.active_agent_host_is_running() {
            self.set_active_app_dialog(Some(AppDialog::ConfirmThemeSwitch { next_mode }));
        } else {
            self.apply_theme_switch(ctx, next_mode);
        }
    }

    /// Returns whether the selected workspace agent slot currently has a host.
    pub(super) fn active_agent_host_is_running(&self) -> bool {
        let slot = self.active_agent_slot();
        self.terminal_hosts
            .get(self.active_workspace)
            .and_then(|hosts| hosts.agents.get(&slot))
            .is_some_and(|slot| slot.host.is_some())
    }

    /// Returns the selected agent slot for the active workspace.
    pub(super) fn active_agent_slot(&self) -> AgentSlotId {
        let slot = self
            .active_agent_slots
            .get(self.active_workspace)
            .cloned()
            .unwrap_or(AgentSlotId::Main);
        if self.agent_slot_exists(self.active_workspace, &slot) {
            slot
        } else {
            AgentSlotId::Main
        }
    }

    /// 设置指定 workspace 的 Agent 槽位，适用于槽位右键菜单动作。
    pub(super) fn set_active_agent_slot(&mut self, index: usize, slot: AgentSlotId) {
        if !self.agent_slot_exists(index, &slot) {
            return;
        }
        if let Some(active) = self.active_agent_slots.get_mut(index) {
            *active = slot;
        }
    }

    /// Checks whether an agent slot still exists in the workspace model.
    pub(super) fn agent_slot_exists(&self, index: usize, slot: &AgentSlotId) -> bool {
        let Some(workspace) = self.workspaces.get(index) else {
            return false;
        };
        match slot {
            AgentSlotId::Main => true,
            AgentSlotId::Subagent(id) => workspace
                .subagents
                .iter()
                .any(|subagent| &subagent.id == id),
        }
    }

    /// Clears an agent slot session when a restart should not resume.
    pub(super) fn clear_agent_slot_session(
        &mut self,
        index: usize,
        slot: &AgentSlotId,
        resume: bool,
    ) {
        if resume {
            return;
        }
        match slot {
            AgentSlotId::Main => {
                if let Some(workspace) = self.workspaces.get_mut(index) {
                    workspace.session_id = None;
                    workspace.activity = WorkspaceActivity::Unknown;
                }
            }
            AgentSlotId::Subagent(id) => {
                if let Some(subagent) = self.workspaces.get_mut(index).and_then(|workspace| {
                    workspace
                        .subagents
                        .iter_mut()
                        .find(|subagent| &subagent.id == id)
                }) {
                    subagent.session_id = None;
                    subagent.activity = WorkspaceActivity::Unknown;
                }
            }
        }
    }

    /// Returns the configured agent kind for one slot.
    pub(super) fn agent_slot_kind(&self, index: usize, slot: &AgentSlotId) -> Option<AgentKind> {
        let workspace = self.workspaces.get(index)?;
        match slot {
            AgentSlotId::Main => Some(workspace.agent_kind),
            AgentSlotId::Subagent(id) => workspace
                .subagents
                .iter()
                .find(|subagent| &subagent.id == id)
                .map(|subagent| subagent.agent_kind),
        }
    }

    /// Updates the configured agent kind and starts a fresh session for one slot.
    pub(super) fn set_agent_slot_kind(
        &mut self,
        index: usize,
        slot: &AgentSlotId,
        next_kind: AgentKind,
    ) {
        let Some(workspace) = self.workspaces.get_mut(index) else {
            return;
        };
        match slot {
            AgentSlotId::Main => {
                workspace.agent_kind = next_kind;
                if !workspace
                    .agent_effort
                    .as_deref()
                    .is_some_and(|effort| next_kind.supports_effort(effort))
                {
                    workspace.agent_effort = None;
                }
                if !next_kind.supports_fast_mode() {
                    workspace.agent_fast_mode = None;
                }
                workspace.session_id = None;
                workspace.activity = WorkspaceActivity::Unknown;
            }
            AgentSlotId::Subagent(id) => {
                if let Some(subagent) = workspace
                    .subagents
                    .iter_mut()
                    .find(|subagent| &subagent.id == id)
                {
                    subagent.agent_kind = next_kind;
                    if !subagent
                        .agent_effort
                        .as_deref()
                        .is_some_and(|effort| next_kind.supports_effort(effort))
                    {
                        subagent.agent_effort = None;
                    }
                    if !next_kind.supports_fast_mode() {
                        subagent.agent_fast_mode = None;
                    }
                    subagent.session_id = None;
                    subagent.activity = WorkspaceActivity::Unknown;
                }
            }
        }
    }

    /// Updates one agent slot model override without clearing its session id.
    pub(super) fn set_agent_slot_model(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        slot: AgentSlotId,
        model: String,
    ) {
        let next_model = normalized_agent_model_value(&model);
        if self.agent_slot_model(index, &slot) == next_model {
            return;
        }
        let Some(workspace) = self.workspaces.get_mut(index) else {
            return;
        };
        match &slot {
            AgentSlotId::Main => {
                workspace.agent_model = next_model.clone();
            }
            AgentSlotId::Subagent(id) => {
                let Some(subagent) = workspace
                    .subagents
                    .iter_mut()
                    .find(|subagent| &subagent.id == id)
                else {
                    return;
                };
                subagent.agent_model = next_model.clone();
            }
        }
        self.persist_workspaces();
        if let Some(hosts) = self.terminal_hosts.get_mut(index) {
            hosts.agents.remove(&slot);
        }
        self.spawn_terminal_host_for_workspace(ctx, index, TerminalSurfaceKind::Agent);
        self.push_toast(
            if next_model.is_some() {
                i18n::text(self.app_language, "Agent model set")
            } else {
                i18n::text(self.app_language, "Agent model override cleared")
            },
            theme::success(),
        );
    }

    /// Updates one agent slot working directory override and restarts that slot.
    pub(super) fn set_agent_slot_work_dir(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        slot: AgentSlotId,
        work_dir: String,
    ) {
        let trimmed_work_dir = work_dir.trim();
        let next_work_dir = normalized_agent_work_dir_value(&work_dir);
        if !trimmed_work_dir.is_empty() && next_work_dir.is_none() {
            self.push_toast(
                i18n::text(self.app_language, "Directory not found"),
                theme::warning(),
            );
            return;
        }
        if self.agent_slot_work_dir(index, &slot) == next_work_dir {
            return;
        }
        let Some(workspace) = self.workspaces.get_mut(index) else {
            return;
        };
        match &slot {
            AgentSlotId::Main => {
                workspace.agent_work_dir = next_work_dir.clone();
            }
            AgentSlotId::Subagent(id) => {
                let Some(subagent) = workspace
                    .subagents
                    .iter_mut()
                    .find(|subagent| &subagent.id == id)
                else {
                    return;
                };
                subagent.agent_work_dir = next_work_dir.clone();
            }
        }
        self.persist_workspaces();
        if let Some(hosts) = self.terminal_hosts.get_mut(index) {
            hosts.agents.remove(&slot);
        }
        self.spawn_terminal_host_for_workspace(ctx, index, TerminalSurfaceKind::Agent);
        self.push_toast(
            if next_work_dir.is_some() {
                i18n::text(self.app_language, "Agent work-dir set")
            } else {
                i18n::text(self.app_language, "Agent work-dir override cleared")
            },
            theme::success(),
        );
    }

    /// Updates one agent slot effort override without clearing its session id.
    pub(super) fn set_agent_slot_effort(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        slot: AgentSlotId,
        effort: Option<String>,
    ) {
        let Some(kind) = self.agent_slot_kind(index, &slot) else {
            return;
        };
        let next_effort = normalized_agent_effort_value(kind, effort.as_deref());
        if self.agent_slot_effort(index, &slot) == next_effort {
            return;
        }
        let Some(workspace) = self.workspaces.get_mut(index) else {
            return;
        };
        match &slot {
            AgentSlotId::Main => {
                workspace.agent_effort = next_effort.clone();
            }
            AgentSlotId::Subagent(id) => {
                let Some(subagent) = workspace
                    .subagents
                    .iter_mut()
                    .find(|subagent| &subagent.id == id)
                else {
                    return;
                };
                subagent.agent_effort = next_effort.clone();
            }
        }
        self.persist_workspaces();
        if let Some(hosts) = self.terminal_hosts.get_mut(index) {
            hosts.agents.remove(&slot);
        }
        self.spawn_terminal_host_for_workspace(ctx, index, TerminalSurfaceKind::Agent);
        self.push_toast(
            if next_effort.is_some() {
                i18n::text(self.app_language, "Agent effort set")
            } else {
                i18n::text(self.app_language, "Agent effort override cleared")
            },
            theme::success(),
        );
    }

    /// Updates one agent slot fast-mode override without touching other slots.
    pub(super) fn set_agent_slot_fast_mode(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        slot: AgentSlotId,
        fast_mode: Option<bool>,
    ) {
        let Some(kind) = self.agent_slot_kind(index, &slot) else {
            return;
        };
        let next_fast_mode = normalized_agent_fast_mode_value(kind, fast_mode);
        if self.agent_slot_fast_mode(index, &slot) == next_fast_mode {
            return;
        }
        let Some(workspace) = self.workspaces.get_mut(index) else {
            return;
        };
        match &slot {
            AgentSlotId::Main => {
                workspace.agent_fast_mode = next_fast_mode;
            }
            AgentSlotId::Subagent(id) => {
                let Some(subagent) = workspace
                    .subagents
                    .iter_mut()
                    .find(|subagent| &subagent.id == id)
                else {
                    return;
                };
                subagent.agent_fast_mode = next_fast_mode;
            }
        }
        self.persist_workspaces();
        if let Some(hosts) = self.terminal_hosts.get_mut(index) {
            hosts.agents.remove(&slot);
        }
        self.spawn_terminal_host_for_workspace(ctx, index, TerminalSurfaceKind::Agent);
        self.push_toast(
            if next_fast_mode.is_some() {
                i18n::text(self.app_language, "Agent fast mode set")
            } else {
                i18n::text(self.app_language, "Agent fast mode override cleared")
            },
            theme::success(),
        );
    }

    /// Returns the configured model override for one agent slot.
    fn agent_slot_model(&self, index: usize, slot: &AgentSlotId) -> Option<String> {
        let workspace = self.workspaces.get(index)?;
        match slot {
            AgentSlotId::Main => workspace.agent_model.clone(),
            AgentSlotId::Subagent(id) => workspace
                .subagents
                .iter()
                .find(|subagent| &subagent.id == id)
                .and_then(|subagent| subagent.agent_model.clone()),
        }
    }

    /// Returns the configured working directory override for one agent slot.
    pub(super) fn agent_slot_work_dir(&self, index: usize, slot: &AgentSlotId) -> Option<PathBuf> {
        let workspace = self.workspaces.get(index)?;
        match slot {
            AgentSlotId::Main => workspace.agent_work_dir.clone(),
            AgentSlotId::Subagent(id) => workspace
                .subagents
                .iter()
                .find(|subagent| &subagent.id == id)
                .and_then(|subagent| subagent.agent_work_dir.clone()),
        }
    }

    /// Returns the configured effort override for one agent slot.
    fn agent_slot_effort(&self, index: usize, slot: &AgentSlotId) -> Option<String> {
        let workspace = self.workspaces.get(index)?;
        match slot {
            AgentSlotId::Main => workspace.agent_effort.clone(),
            AgentSlotId::Subagent(id) => workspace
                .subagents
                .iter()
                .find(|subagent| &subagent.id == id)
                .and_then(|subagent| subagent.agent_effort.clone()),
        }
    }

    /// Returns the configured fast-mode override for one agent slot.
    fn agent_slot_fast_mode(&self, index: usize, slot: &AgentSlotId) -> Option<bool> {
        let workspace = self.workspaces.get(index)?;
        match slot {
            AgentSlotId::Main => workspace.agent_fast_mode,
            AgentSlotId::Subagent(id) => workspace
                .subagents
                .iter()
                .find(|subagent| &subagent.id == id)
                .and_then(|subagent| subagent.agent_fast_mode),
        }
    }

    /// Applies the theme immediately and schedules persistence.
    pub(super) fn apply_theme_switch(&mut self, ctx: &egui::Context, mode: theme::ThemeMode) {
        self.theme_mode = mode;
        theme::set_mode(ctx, mode);
        self.spawn_theme_mode_save(mode);
        self.queue_app_event(AppEvent::SyncTerminalEventRepaintFlags);
        self.request_app_repaint(ctx);
    }

    /// Restarts the selected agent after a theme change requires a terminal reload.
    pub(super) fn restart_active_agent_after_theme_switch(&mut self, ctx: &egui::Context) {
        let index = self.active_workspace;
        let Some(hosts) = self.terminal_hosts.get_mut(index) else {
            return;
        };
        let slot = self
            .active_agent_slots
            .get(index)
            .cloned()
            .unwrap_or(AgentSlotId::Main);
        let Some(agent) = hosts
            .agents
            .get_mut(&slot)
            .and_then(|slot| slot.host.as_mut())
        else {
            return;
        };
        agent.request_graceful_exit();
        if index >= self.pending_agent_theme_restarts.len() {
            self.pending_agent_theme_restarts
                .resize_with(index + 1, || None);
        }
        self.pending_agent_theme_restarts[index] = Some(Instant::now() + AGENT_THEME_RESTART_DELAY);
        self.push_toast(
            i18n::text(self.app_language, "Agent will restart to apply the theme"),
            theme::warning(),
        );
        self.request_app_repaint_after(ctx, AGENT_THEME_RESTART_DELAY);
    }

    /// Restarts the active agent slot and forces it to create a new session.
    pub(super) fn restart_active_agent_without_resume(&mut self, ctx: &egui::Context) {
        self.restart_agent(ctx, self.active_workspace, false);
    }

    /// 重启所有已经打开的 PTY，让新的网络代理配置进入子进程环境。
    pub(super) fn restart_open_terminal_hosts_for_network_settings(&mut self, ctx: &egui::Context) {
        let mut agent_requests = Vec::new();
        let mut workspace_requests = Vec::new();
        let mut helix_requests = Vec::new();

        for index in 0..self.terminal_hosts.len() {
            let agent_slots = self.terminal_hosts[index]
                .agents
                .iter()
                .filter(|(_, slot)| slot.host.is_some())
                .map(|(slot_id, _)| slot_id.clone())
                .collect::<Vec<_>>();
            for slot_id in agent_slots {
                if let Some(workspace) = self.agent_workspace_for_slot(index, &slot_id) {
                    agent_requests.push((index, slot_id, workspace));
                }
            }

            if self.terminal_hosts[index].workspace.is_some()
                && let Some(workspace) = self.workspaces.get(index).cloned()
            {
                workspace_requests.push((index, workspace));
            }

            let helix_spec = self.terminal_hosts[index]
                .helix
                .as_ref()
                .and_then(|host| host.helix_spec().cloned());
            if let Some(spec) = helix_spec
                && let Some(workspace) = self.workspaces.get(index).cloned()
            {
                helix_requests.push((index, workspace, spec));
            }
        }

        let restart_count = agent_requests.len() + workspace_requests.len() + helix_requests.len();

        for (index, slot_id, workspace) in agent_requests {
            if let Some(slot) = self
                .terminal_hosts
                .get_mut(index)
                .and_then(|hosts| hosts.agents.get_mut(&slot_id))
            {
                slot.host = None;
                slot.error = None;
            }
            let key = TerminalSpawnKey {
                index,
                kind: TerminalSurfaceKind::Agent,
                agent_slot: slot_id,
            };
            self.spawn_terminal_host_task(ctx, key, workspace);
        }

        for (index, workspace) in workspace_requests {
            if let Some(hosts) = self.terminal_hosts.get_mut(index) {
                hosts.workspace = None;
                hosts.workspace_error = None;
            }
            let key = TerminalSpawnKey {
                index,
                kind: TerminalSurfaceKind::Workspace,
                agent_slot: AgentSlotId::Main,
            };
            self.spawn_terminal_host_task(ctx, key, workspace);
        }

        for (index, workspace, spec) in helix_requests {
            if let Some(hosts) = self.terminal_hosts.get_mut(index) {
                hosts.helix = None;
                hosts.helix_error = None;
            }
            let key = TerminalSpawnKey {
                index,
                kind: TerminalSurfaceKind::Helix,
                agent_slot: AgentSlotId::Main,
            };
            self.spawn_helix_host_task(ctx, key, workspace, spec);
        }

        if restart_count > 0 {
            self.push_toast(
                i18n::text_with_arg(
                    self.app_language,
                    "Restarted {count} terminal-backed session(s) for proxy changes",
                    "{count}",
                    restart_count.to_string(),
                ),
                theme::success(),
            );
        }
    }

    /// Restarts the selected workspace agent, optionally keeping its session id.
    pub(super) fn restart_agent(&mut self, ctx: &egui::Context, index: usize, resume: bool) {
        let slot = self
            .active_agent_slots
            .get(index)
            .cloned()
            .unwrap_or(AgentSlotId::Main);
        self.clear_agent_slot_session(index, &slot, resume);
        self.persist_workspaces();

        if let Some(hosts) = self.terminal_hosts.get_mut(index) {
            hosts.agents.remove(&slot);
        }
        self.spawn_terminal_host_for_workspace(ctx, index, TerminalSurfaceKind::Agent);
        self.push_toast(
            if resume {
                i18n::text(self.app_language, "Agent restarted with resume")
            } else {
                i18n::text(self.app_language, "Agent restarted with a new session")
            },
            theme::success(),
        );
    }

    /// Switches agent kind and starts a fresh session for that workspace.
    pub(super) fn switch_agent_kind(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        next_kind: AgentKind,
    ) {
        let slot = self
            .active_agent_slots
            .get(index)
            .cloned()
            .unwrap_or(AgentSlotId::Main);
        if self.agent_slot_kind(index, &slot) == Some(next_kind) {
            return;
        }
        self.set_agent_slot_kind(index, &slot, next_kind);
        self.persist_workspaces();
        if let Some(hosts) = self.terminal_hosts.get_mut(index) {
            hosts.agents.remove(&slot);
        }
        self.spawn_terminal_host_for_workspace(ctx, index, TerminalSurfaceKind::Agent);
        self.push_toast(
            i18n::text_with_arg(
                self.app_language,
                "Switched agent to {kind}",
                "{kind}",
                next_kind.title(),
            ),
            theme::success(),
        );
    }

    pub(super) fn finish_pending_agent_theme_restarts(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let mut restart_indexes = Vec::new();
        let mut next_deadline = None;
        for (index, pending) in self.pending_agent_theme_restarts.iter().enumerate() {
            let Some(deadline) = *pending else {
                continue;
            };
            if now >= deadline {
                restart_indexes.push(index);
            } else {
                next_deadline =
                    Some(next_deadline.map_or(deadline, |current: Instant| current.min(deadline)));
            }
        }
        for index in restart_indexes {
            let slot = self
                .active_agent_slots
                .get(index)
                .cloned()
                .unwrap_or(AgentSlotId::Main);
            if let Some(hosts) = self.terminal_hosts.get_mut(index) {
                hosts.agents.remove(&slot);
            }
            self.spawn_terminal_host_for_workspace(ctx, index, TerminalSurfaceKind::Agent);
            if let Some(pending) = self.pending_agent_theme_restarts.get_mut(index) {
                *pending = None;
            }
            self.push_toast(
                i18n::text(self.app_language, "Agent resumed after theme switch"),
                theme::success(),
            );
        }
        if let Some(deadline) = next_deadline {
            self.request_app_repaint_after(ctx, deadline.saturating_duration_since(now));
        }
    }
}

/// Normalizes a model override entered by the user.
fn normalized_agent_model_value(model: &str) -> Option<String> {
    let model = model.trim();
    (!model.is_empty()).then(|| model.to_string())
}

/// Normalizes a working directory override entered by the user.
fn normalized_agent_work_dir_value(work_dir: &str) -> Option<PathBuf> {
    let work_dir = work_dir.trim();
    if work_dir.is_empty() {
        return None;
    }
    data::normalize_stored_agent_work_dir(Some(PathBuf::from(work_dir)))
}

/// Normalizes an effort override for one agent kind.
fn normalized_agent_effort_value(kind: AgentKind, effort: Option<&str>) -> Option<String> {
    effort
        .map(str::trim)
        .filter(|value| kind.supports_effort(value))
        .map(str::to_string)
}

/// Keeps fast mode scoped to Codex slots because Claude has no service tier.
fn normalized_agent_fast_mode_value(kind: AgentKind, fast_mode: Option<bool>) -> Option<bool> {
    kind.supports_fast_mode().then_some(fast_mode).flatten()
}

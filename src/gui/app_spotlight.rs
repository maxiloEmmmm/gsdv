//! Cross-workspace Agent spotlight mode.
//!
//! Membership and priority are persisted by stable Agent id. The visible list
//! remains global and reorders when activity, priority, or stop order changes.

use super::*;

pub(super) const SPOTLIGHT_SIDEBAR_WIDTH: f32 = 220.0;
const SPOTLIGHT_ROW_HEIGHT: f32 = 42.0;

impl GsdvGuiApp {
    /// Returns the stable id for one workspace Agent slot.
    ///
    /// Example: workspace 0 main slot -> `gsdv-abc`.
    fn spotlight_agent_id(&self, workspace_index: usize, slot: &AgentSlotId) -> Option<String> {
        let workspace = self.workspaces.get(workspace_index)?;
        match slot {
            AgentSlotId::Main => Some(workspace.agent_id.clone()),
            AgentSlotId::Subagent(id) => workspace
                .subagents
                .iter()
                .find(|subagent| &subagent.id == id)
                .map(|subagent| subagent.agent_id.clone()),
        }
    }

    /// Checks whether one Agent belongs to spotlight mode.
    ///
    /// Example: persisted id present -> true.
    pub(super) fn agent_slot_in_spotlight(
        &self,
        workspace_index: usize,
        slot: &AgentSlotId,
    ) -> bool {
        self.spotlight_agent_level(workspace_index, slot).is_some()
    }

    /// Returns one Agent's persisted spotlight priority level.
    ///
    /// Example: registered at level 1 -> `Some(1)`.
    pub(super) fn spotlight_agent_level(
        &self,
        workspace_index: usize,
        slot: &AgentSlotId,
    ) -> Option<u8> {
        let agent_id = self.spotlight_agent_id(workspace_index, slot)?;
        self.spotlight_agents.get(&agent_id).copied()
    }

    /// Adds one Agent to spotlight or changes its persisted priority level.
    ///
    /// Example: ordinary Agent tab -> level 1 -> starred tab at priority 1.
    pub(super) fn set_agent_spotlight_level(
        &mut self,
        workspace_index: usize,
        slot: AgentSlotId,
        level: u8,
    ) {
        let Some(agent_id) = self.spotlight_agent_id(workspace_index, &slot) else {
            return;
        };
        self.spotlight_agents.insert(agent_id, level.clamp(1, 3));
        self.sync_spotlight_slot_stop_order(workspace_index, &slot);
        self.mark_workspace_store_dirty();
        self.reconcile_spotlight_leader();
        self.request_app_repaint();
    }

    /// Removes one Agent from spotlight without closing its underlying tab.
    ///
    /// Example: registered Agent -> cancel registration -> ordinary Agent tab.
    pub(super) fn remove_agent_spotlight(&mut self, workspace_index: usize, slot: AgentSlotId) {
        let Some(agent_id) = self.spotlight_agent_id(workspace_index, &slot) else {
            return;
        };
        if self.spotlight_agents.remove(&agent_id).is_none() {
            return;
        }
        self.mark_workspace_store_dirty();
        self.normalize_spotlight_selection();
        self.reconcile_spotlight_leader();
        self.request_app_repaint();
    }

    /// Applies the persisted rank-mode switch and reconciles the current leader.
    ///
    /// Example: disabled -> enabled -> select the first sorted spotlight Agent.
    pub(super) fn set_spotlight_rank_mode(&mut self, enabled: bool) {
        if self.spotlight_rank_mode == enabled {
            return;
        }
        self.spotlight_rank_mode = enabled;
        self.spotlight_leader_agent_id = None;
        self.mark_workspace_store_dirty();
        self.reconcile_spotlight_leader();
        self.request_app_repaint();
    }

    /// Seeds runtime stop order for persisted stopped Agents during app startup.
    ///
    /// Example: two stored Idle Agents -> deterministic stop order 0 then 1.
    pub(super) fn initialize_spotlight_stopped_order(&mut self) {
        let stopped_ids = self
            .workspaces
            .iter()
            .flat_map(|workspace| {
                let main = std::iter::once((workspace.agent_id.clone(), workspace.activity));
                let subagents = workspace
                    .subagents
                    .iter()
                    .map(|subagent| (subagent.agent_id.clone(), subagent.activity));
                main.chain(subagents)
            })
            .filter(|(agent_id, activity)| {
                self.spotlight_agents.contains_key(agent_id)
                    && spotlight_activity_is_stopped(*activity)
            })
            .map(|(agent_id, _)| agent_id)
            .collect::<Vec<_>>();
        for agent_id in stopped_ids {
            self.record_spotlight_stopped_state(&agent_id, true);
        }
    }

    /// Updates runtime stop order from one Agent slot's current state.
    ///
    /// Example: manually stopped Unknown Agent -> assign the next stop sequence.
    pub(super) fn sync_spotlight_slot_stop_order(
        &mut self,
        workspace_index: usize,
        slot: &AgentSlotId,
    ) {
        let Some(agent_id) = self.spotlight_agent_id(workspace_index, slot) else {
            return;
        };
        let stopped = self.agent_slot_manually_stopped(workspace_index, slot)
            || spotlight_activity_is_stopped(self.agent_slot_activity(workspace_index, slot));
        self.record_spotlight_stopped_state(&agent_id, stopped);
        self.reconcile_spotlight_leader();
    }

    /// Updates runtime stop order directly from a hook activity transition.
    ///
    /// Example: Busy -> Error -> assign stop sequence; Error -> Busy -> clear it.
    pub(super) fn sync_spotlight_hook_activity(
        &mut self,
        agent_id: &str,
        activity: WorkspaceActivity,
    ) {
        let stopped = spotlight_activity_is_stopped(activity)
            || self.spotlight_agent_manually_stopped(agent_id);
        self.record_spotlight_stopped_state(agent_id, stopped);
        self.reconcile_spotlight_leader();
    }

    /// Finds whether one stable Agent id is currently manually stopped.
    ///
    /// Example: a late Busy hook after manual stop -> true keeps its stop order.
    fn spotlight_agent_manually_stopped(&self, agent_id: &str) -> bool {
        self.workspaces
            .iter()
            .enumerate()
            .find_map(|(workspace_index, workspace)| {
                if workspace.agent_id == agent_id {
                    return Some(
                        self.agent_slot_manually_stopped(workspace_index, &AgentSlotId::Main),
                    );
                }
                workspace
                    .subagents
                    .iter()
                    .find(|subagent| subagent.agent_id == agent_id)
                    .map(|subagent| {
                        self.agent_slot_manually_stopped(
                            workspace_index,
                            &AgentSlotId::Subagent(subagent.id.clone()),
                        )
                    })
            })
            .unwrap_or(false)
    }

    /// Records or clears one Agent's runtime stop sequence.
    ///
    /// Example: first stopped transition -> sequence 4; repeated Idle -> keep 4.
    fn record_spotlight_stopped_state(&mut self, agent_id: &str, stopped: bool) {
        if stopped {
            if self.spotlight_stopped_order.contains_key(agent_id) {
                return;
            }
            let order = self.spotlight_next_stopped_order;
            self.spotlight_next_stopped_order = self.spotlight_next_stopped_order.saturating_add(1);
            self.spotlight_stopped_order
                .insert(agent_id.to_string(), order);
        } else {
            self.spotlight_stopped_order.remove(agent_id);
        }
    }

    /// Enters the global Agent browser and temporarily forces its dark theme.
    ///
    /// Example: workspace rail spotlight button -> dark global Agent list.
    pub(super) fn enter_spotlight_mode(&mut self, ctx: &egui::Context) {
        if self.spotlight_mode {
            return;
        }
        self.spotlight_return_theme = Some(self.theme_mode);
        self.spotlight_mode = true;
        self.workspace_terminal_drawers.fill(false);
        self.reviewer_helix_drawers.fill(false);
        self.notifications.open = false;
        self.extra_tools.open = false;
        self.theme_mode = theme::ThemeMode::Dark;
        theme::set_mode(ctx, theme::ThemeMode::Dark);
        self.spotlight_leader_agent_id = None;
        if self.spotlight_rank_mode {
            self.reconcile_spotlight_leader();
        } else {
            self.normalize_spotlight_selection();
        }
        self.request_app_repaint();
    }

    /// Exits spotlight mode and keeps the selected Agent as the active workspace target.
    ///
    /// Example: selected reader Agent -> exit -> reader workspace Agent tab.
    pub(super) fn exit_spotlight_mode(&mut self, ctx: &egui::Context) {
        if !self.spotlight_mode {
            return;
        }
        self.spotlight_mode = false;
        if let Some(mode) = self.spotlight_return_theme.take() {
            self.theme_mode = mode;
            theme::set_mode(ctx, mode);
        }
        self.request_app_repaint();
    }

    /// Collects spotlight Agents and orders stopped/error entries before active entries.
    ///
    /// Example: `[Busy A, Idle B]` -> `[Idle B, Busy A]`.
    fn spotlight_agents(&self) -> Vec<SpotlightAgentEntry> {
        let mut entries = Vec::new();
        for (workspace_index, workspace) in self.workspaces.iter().enumerate() {
            if let Some(level) = self.spotlight_agents.get(&workspace.agent_id).copied() {
                let slot = AgentSlotId::Main;
                entries.push(SpotlightAgentEntry {
                    workspace_index,
                    slot: slot.clone(),
                    agent_id: workspace.agent_id.clone(),
                    spotlight_level: level,
                    title: "main".to_string(),
                    workspace_name: workspace.name.clone(),
                    activity: workspace.activity,
                    manually_stopped: self.agent_slot_manually_stopped(workspace_index, &slot),
                    stopped_order: self
                        .spotlight_stopped_order
                        .get(&workspace.agent_id)
                        .copied(),
                });
            }
            for subagent in &workspace.subagents {
                let Some(level) = self.spotlight_agents.get(&subagent.agent_id).copied() else {
                    continue;
                };
                let slot = AgentSlotId::Subagent(subagent.id.clone());
                entries.push(SpotlightAgentEntry {
                    workspace_index,
                    slot: slot.clone(),
                    agent_id: subagent.agent_id.clone(),
                    spotlight_level: level,
                    title: subagent.name.clone(),
                    workspace_name: workspace.name.clone(),
                    activity: subagent.activity,
                    manually_stopped: self.agent_slot_manually_stopped(workspace_index, &slot),
                    stopped_order: self
                        .spotlight_stopped_order
                        .get(&subagent.agent_id)
                        .copied(),
                });
            }
        }
        entries.sort_by_key(spotlight_agent_sort_rank);
        entries
    }

    /// Selects a new first Agent only when rank mode observes a leader change.
    ///
    /// Example: level-1 Agent stops before current level-2 leader -> switch once.
    pub(super) fn reconcile_spotlight_leader(&mut self) {
        if !self.spotlight_rank_mode {
            self.spotlight_leader_agent_id = None;
            return;
        }
        let leader = self.spotlight_agents().into_iter().next();
        let next_id = leader.as_ref().map(|entry| entry.agent_id.clone());
        if self.spotlight_leader_agent_id == next_id {
            return;
        }
        self.spotlight_leader_agent_id = next_id;
        if self.spotlight_mode
            && let Some(entry) = leader
        {
            self.prepare_spotlight_workspace(entry.workspace_index, entry.slot);
        }
    }

    /// Selects the first spotlight Agent when the current target is no longer a member.
    ///
    /// Example: remove selected member -> select the next sorted member.
    fn normalize_spotlight_selection(&mut self) {
        let entries = self.spotlight_agents();
        let current_slot = self.active_agent_slot();
        if entries.iter().any(|entry| {
            entry.workspace_index == self.active_workspace && entry.slot == current_slot
        }) {
            self.prepare_spotlight_workspace(self.active_workspace, current_slot);
            return;
        }
        if let Some(entry) = entries.first() {
            self.prepare_spotlight_workspace(entry.workspace_index, entry.slot.clone());
        }
    }

    /// Routes Agent input and terminal rendering to one spotlight entry.
    ///
    /// Example: click Agent from workspace 2 -> workspace 2 main terminal owns input.
    fn prepare_spotlight_workspace(&mut self, workspace_index: usize, slot: AgentSlotId) {
        self.active_workspace = workspace_index;
        if let Some(workspace) = self.workspaces.get_mut(workspace_index) {
            workspace.route = Route::Workspace;
            workspace.center_mode = CenterMode::Agent;
        }
        self.set_active_agent_slot(workspace_index, slot);
    }

    /// Draws the fixed dark spotlight sidebar and returns whether exit was requested.
    ///
    /// Example: click top back button -> true.
    pub(super) fn spotlight_sidebar(&mut self, ui: &mut Ui) -> bool {
        let mut exit_requested = false;
        ui.set_min_width(SPOTLIGHT_SIDEBAR_WIDTH);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            if spotlight_back_button(ui, self.app_language).clicked() {
                exit_requested = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(8.0);
                if spotlight_rank_checkbox(ui, self.spotlight_rank_mode, self.app_language)
                    .clicked()
                {
                    self.set_spotlight_rank_mode(!self.spotlight_rank_mode);
                }
            });
        });
        ui.add_space(6.0);
        ui.separator();

        let entries = self.spotlight_agents();
        let active_slot = self.active_agent_slot();
        ScrollArea::vertical().show(ui, |ui| {
            for entry in entries {
                let selected =
                    entry.workspace_index == self.active_workspace && entry.slot == active_slot;
                let response = spotlight_agent_row(ui, &entry, selected, &self.repaint_controller);
                let mut action = None;
                let menu = self.spotlight_agent_menu_values(&entry);
                response.context_menu(|ui| {
                    if let Some((
                        kind,
                        model,
                        provider,
                        effort,
                        fast_mode,
                        work_dir,
                        helix_dir,
                        name,
                        session,
                    )) = menu.as_ref()
                    {
                        agent_slot_context_menu(
                            ui,
                            entry.slot.clone(),
                            entry.manually_stopped,
                            *kind,
                            model.as_deref(),
                            provider.as_deref(),
                            effort.as_deref(),
                            *fast_mode,
                            work_dir.as_deref(),
                            helix_dir.as_deref(),
                            name.as_deref(),
                            session.as_deref(),
                            Some(entry.spotlight_level),
                            &mut action,
                            self.app_language,
                        );
                    }
                });
                if response.clicked() || response.secondary_clicked() {
                    self.prepare_spotlight_workspace(entry.workspace_index, entry.slot.clone());
                }
                if let Some(action) = action {
                    self.prepare_spotlight_workspace(entry.workspace_index, entry.slot.clone());
                    self.handle_agent_tab_action(ui.ctx(), action);
                }
            }
        });
        exit_requested
    }

    /// Draws only the selected Agent surface on the spotlight right side.
    ///
    /// Example: selected stopped Agent -> existing start panel fills the right side.
    pub(super) fn spotlight_agent_surface(&mut self, ui: &mut Ui) {
        let entries = self.spotlight_agents();
        let active_slot = self.active_agent_slot();
        let selected = entries.into_iter().find(|entry| {
            entry.workspace_index == self.active_workspace && entry.slot == active_slot
        });
        let Some(entry) = selected else {
            ui.centered_and_justified(|ui| {
                ui.label(muted(i18n::text(self.app_language, "No spotlight agents")));
            });
            return;
        };
        self.agent_terminal_host_surface(ui, entry.slot, None);
    }

    /// Copies the small Agent configuration subset needed by a context menu.
    ///
    /// Example: subagent slot -> its own model, paths, name, and session.
    #[allow(clippy::type_complexity)]
    fn spotlight_agent_menu_values(
        &self,
        entry: &SpotlightAgentEntry,
    ) -> Option<(
        AgentKind,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<bool>,
        Option<PathBuf>,
        Option<PathBuf>,
        Option<String>,
        Option<String>,
    )> {
        let workspace = self.workspaces.get(entry.workspace_index)?;
        match &entry.slot {
            AgentSlotId::Main => Some((
                workspace.agent_kind,
                workspace.agent_model.clone(),
                workspace.agent_model_provider.clone(),
                workspace.agent_effort.clone(),
                workspace.agent_fast_mode,
                workspace.agent_work_dir.clone(),
                workspace.helix_work_dir.clone(),
                None,
                workspace.session_id.clone(),
            )),
            AgentSlotId::Subagent(id) => workspace
                .subagents
                .iter()
                .find(|subagent| &subagent.id == id)
                .map(|subagent| {
                    (
                        subagent.agent_kind,
                        subagent.agent_model.clone(),
                        subagent.agent_model_provider.clone(),
                        subagent.agent_effort.clone(),
                        subagent.agent_fast_mode,
                        subagent.agent_work_dir.clone(),
                        subagent.helix_work_dir.clone(),
                        Some(subagent.name.clone()),
                        subagent.session_id.clone(),
                    )
                }),
        }
    }
}

/// Returns the stable activity group used by spotlight ordering.
///
/// Example: manually stopped -> 0; unknown -> 1; busy -> 2.
fn spotlight_agent_sort_rank(entry: &SpotlightAgentEntry) -> (u8, u8, u64) {
    let activity_rank = if entry.manually_stopped
        || matches!(
            entry.activity,
            WorkspaceActivity::Idle | WorkspaceActivity::Error
        ) {
        0
    } else if entry.activity == WorkspaceActivity::Unknown {
        1
    } else {
        2
    };
    (
        activity_rank,
        entry.spotlight_level,
        entry.stopped_order.unwrap_or(u64::MAX),
    )
}

/// Returns whether activity belongs to the stopped spotlight group.
///
/// Example: Error -> true; Busy -> false.
fn spotlight_activity_is_stopped(activity: WorkspaceActivity) -> bool {
    matches!(activity, WorkspaceActivity::Idle | WorkspaceActivity::Error)
}

/// Draws the icon-only spotlight exit control.
///
/// Example: click the left arrow -> exit spotlight mode.
fn spotlight_back_button(ui: &mut Ui, language: AppLanguage) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let fill = if response.hovered() {
        Color32::from_rgb(0x1D, 0x2A, 0x38)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
    let center = rect.center();
    let color = theme::text();
    ui.painter().line_segment(
        [
            egui::pos2(center.x - 6.0, center.y),
            egui::pos2(center.x + 6.0, center.y),
        ],
        Stroke::new(1.5_f32, color),
    );
    ui.painter().line_segment(
        [
            egui::pos2(center.x - 6.0, center.y),
            egui::pos2(center.x - 1.0, center.y - 5.0),
        ],
        Stroke::new(1.5_f32, color),
    );
    ui.painter().line_segment(
        [
            egui::pos2(center.x - 6.0, center.y),
            egui::pos2(center.x - 1.0, center.y + 5.0),
        ],
        Stroke::new(1.5_f32, color),
    );
    response
        .on_hover_text(i18n::text(language, "Exit spotlight mode"))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Draws the icon-only rank-mode checkbox.
///
/// Example: unchecked square -> click -> checked rank mode.
fn spotlight_rank_checkbox(ui: &mut Ui, checked: bool, language: AppLanguage) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let box_rect = Rect::from_center_size(rect.center(), Vec2::splat(15.0));
    let border = if checked {
        Color32::from_rgb(0x60, 0xA5, 0xFA)
    } else {
        Color32::from_rgb(0x58, 0x6C, 0x84)
    };
    ui.painter().rect(
        box_rect,
        CornerRadius::same(3),
        if checked {
            border
        } else {
            Color32::TRANSPARENT
        },
        Stroke::new(1.2_f32, border),
        egui::StrokeKind::Inside,
    );
    if checked {
        let center = box_rect.center();
        ui.painter().line_segment(
            [
                egui::pos2(center.x - 4.0, center.y),
                egui::pos2(center.x - 1.0, center.y + 3.0),
            ],
            Stroke::new(1.5_f32, Color32::WHITE),
        );
        ui.painter().line_segment(
            [
                egui::pos2(center.x - 1.0, center.y + 3.0),
                egui::pos2(center.x + 5.0, center.y - 4.0),
            ],
            Stroke::new(1.5_f32, Color32::WHITE),
        );
    }
    response
        .on_hover_text(i18n::text(language, "Spotlight rank mode"))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Returns the final workspace path segment for compact spotlight rows.
///
/// Example: `tmp/project` -> `project`.
fn spotlight_workspace_leaf(name: &str) -> &str {
    name.rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(name)
}

/// Draws one compact dark Agent selector row.
///
/// Example: selected Idle Agent -> blue edge, yellow star, green dot.
fn spotlight_agent_row(
    ui: &mut Ui,
    entry: &SpotlightAgentEntry,
    selected: bool,
    repaint_controller: &repaint_gate::RepaintController,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), SPOTLIGHT_ROW_HEIGHT),
        Sense::click(),
    );
    let fill = if selected {
        Color32::from_rgb(0x17, 0x2C, 0x4A)
    } else if response.hovered() {
        Color32::from_rgb(0x1D, 0x2A, 0x38)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
    if selected {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())),
            CornerRadius::ZERO,
            theme::primary(),
        );
    }
    ui.painter().text(
        egui::pos2(rect.left() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        "★",
        egui::FontId::proportional(16.0),
        Color32::from_rgb(0xF5, 0xB8, 0x00),
    );
    let label = format!(
        "{} · {}",
        entry.title,
        spotlight_workspace_leaf(&entry.workspace_name)
    );
    ui.painter().text(
        egui::pos2(rect.left() + 36.0, rect.center().y),
        Align2::LEFT_CENTER,
        truncate_middle(&label, 24),
        egui::FontId::proportional(13.0),
        theme::text(),
    );
    let (status_color, _) = activity_style(entry.activity);
    paint_activity_dot(
        ui,
        egui::pos2(rect.right() - 13.0, rect.center().y),
        status_color,
        entry.activity,
        true,
        repaint_controller,
    );
    response
        .on_hover_text(format!("{}\n{}", entry.title, entry.workspace_name))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

//! Cross-workspace Agent spotlight mode.
//!
//! Membership is persisted by stable Agent id. The visible list remains global
//! and only reorders when runtime activity changes.

use super::*;

const SPOTLIGHT_SIDEBAR_WIDTH: f32 = 244.0;
const SPOTLIGHT_ROW_HEIGHT: f32 = 54.0;

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
        self.spotlight_agent_id(workspace_index, slot)
            .is_some_and(|agent_id| self.spotlight_agent_ids.contains(&agent_id))
    }

    /// Toggles one Agent's persisted spotlight membership without entering the mode.
    ///
    /// Example: ordinary Agent tab -> `Add to spotlight` -> starred tab.
    pub(super) fn toggle_agent_spotlight(&mut self, workspace_index: usize, slot: AgentSlotId) {
        let Some(agent_id) = self.spotlight_agent_id(workspace_index, &slot) else {
            return;
        };
        if !self.spotlight_agent_ids.remove(&agent_id) {
            self.spotlight_agent_ids.insert(agent_id);
        }
        self.mark_workspace_store_dirty();
        if self.spotlight_mode {
            self.normalize_spotlight_selection();
        }
        self.request_app_repaint();
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
        self.normalize_spotlight_selection();
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
            if self.spotlight_agent_ids.contains(&workspace.agent_id) {
                let slot = AgentSlotId::Main;
                entries.push(SpotlightAgentEntry {
                    workspace_index,
                    slot: slot.clone(),
                    agent_id: workspace.agent_id.clone(),
                    title: "main".to_string(),
                    workspace_name: workspace.name.clone(),
                    activity: workspace.activity,
                    manually_stopped: self.agent_slot_manually_stopped(workspace_index, &slot),
                });
            }
            for subagent in &workspace.subagents {
                if !self.spotlight_agent_ids.contains(&subagent.agent_id) {
                    continue;
                }
                let slot = AgentSlotId::Subagent(subagent.id.clone());
                entries.push(SpotlightAgentEntry {
                    workspace_index,
                    slot: slot.clone(),
                    agent_id: subagent.agent_id.clone(),
                    title: subagent.name.clone(),
                    workspace_name: workspace.name.clone(),
                    activity: subagent.activity,
                    manually_stopped: self.agent_slot_manually_stopped(workspace_index, &slot),
                });
            }
        }
        entries.sort_by_key(spotlight_agent_sort_rank);
        entries
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
        ui.add_space(8.0);
        let back = ui.add_sized(
            [ui.available_width(), 36.0],
            Button::new(
                RichText::new(format!(
                    "←  {}",
                    i18n::text(self.app_language, "Exit spotlight mode")
                ))
                .color(theme::text()),
            )
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE),
        );
        if back.clicked() {
            exit_requested = true;
        }
        ui.add_space(8.0);

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
                            true,
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
fn spotlight_agent_sort_rank(entry: &SpotlightAgentEntry) -> u8 {
    if entry.manually_stopped
        || matches!(
            entry.activity,
            WorkspaceActivity::Idle | WorkspaceActivity::Error
        )
    {
        0
    } else if entry.activity == WorkspaceActivity::Unknown {
        1
    } else {
        2
    }
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
        egui::pos2(rect.left() + 12.0, rect.center().y - 1.0),
        Align2::LEFT_CENTER,
        "★",
        egui::FontId::proportional(16.0),
        Color32::from_rgb(0xF5, 0xB8, 0x00),
    );
    ui.painter().text(
        egui::pos2(rect.left() + 36.0, rect.top() + 17.0),
        Align2::LEFT_CENTER,
        truncate_middle(&entry.title, 22),
        egui::FontId::proportional(13.0),
        theme::text(),
    );
    ui.painter().text(
        egui::pos2(rect.left() + 36.0, rect.top() + 37.0),
        Align2::LEFT_CENTER,
        truncate_middle(&entry.workspace_name, 24),
        egui::FontId::proportional(11.0),
        theme::muted(),
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
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

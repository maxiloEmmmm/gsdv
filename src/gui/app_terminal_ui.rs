//! Terminal 和 Helix 抽屉 surface。
//!
//! 该模块负责 terminal host 的绘制、点击输出处理和 Helix 抽屉启动 glue。

use super::*;

impl GsdvGuiApp {
    pub(super) fn terminal_host_surface(&mut self, ui: &mut Ui, kind: TerminalSurfaceKind) {
        self.ensure_terminal_host(ui.ctx(), kind);
        let surface_route_active = match kind {
            TerminalSurfaceKind::Agent => {
                !self.workspace_terminal_drawer_is_open() && !self.reviewer_helix_drawer_is_open()
            }
            TerminalSurfaceKind::Workspace => self.workspace_terminal_drawer_is_open(),
            TerminalSurfaceKind::Helix => false,
        };
        let accept_input = surface_route_active
            && self.active_app_dialog().is_none()
            && self.active_reviewer_dialog().is_none()
            && !self.extra_tools.open
            && !self.notifications.open
            && !self.suppress_default_agent_input;
        let request_focus = accept_input;
        self.respawn_exited_terminal_host(ui.ctx(), kind);
        let agent_slot = self
            .active_agent_slots
            .get(self.active_workspace)
            .cloned()
            .unwrap_or(AgentSlotId::Main);
        let Some(hosts) = self.terminal_hosts.get_mut(self.active_workspace) else {
            return;
        };
        let (host, error) = match kind {
            TerminalSurfaceKind::Agent => {
                let slot = hosts.agents.entry(agent_slot.clone()).or_default();
                (slot.host.as_mut(), slot.error.clone())
            }
            TerminalSurfaceKind::Workspace => {
                (hosts.workspace.as_mut(), hosts.workspace_error.clone())
            }
            TerminalSurfaceKind::Helix => (hosts.helix.as_mut(), hosts.helix_error.clone()),
        };
        let Some(host) = host else {
            let retry = terminal_error_panel(
                ui,
                match kind {
                    TerminalSurfaceKind::Agent => "Agent terminal failed to start",
                    TerminalSurfaceKind::Workspace => "Workspace terminal failed to start",
                    TerminalSurfaceKind::Helix => "Helix failed to start",
                },
                error.as_deref().unwrap_or("No backend error was reported."),
                self.app_language,
            );
            if retry && let Some(hosts) = self.terminal_hosts.get_mut(self.active_workspace) {
                match kind {
                    TerminalSurfaceKind::Agent => {
                        hosts.agents.entry(agent_slot).or_default().error = None;
                    }
                    TerminalSurfaceKind::Workspace => hosts.workspace_error = None,
                    TerminalSurfaceKind::Helix => hosts.helix_error = None,
                }
            }
            return;
        };
        let terminal_size = Vec2::new(ui.available_width(), ui.available_height().max(1.0));
        let theme_mode = self.theme_mode;
        let terminal_font_size = terminal_font_size_for_kind(&self.font_settings, kind);
        let custom_quick_replies = self.runtime_settings.agent_custom_quick_replies.clone();
        let terminal_output = ui.allocate_ui(terminal_size, |ui| {
            let shortcut_scope = match kind {
                TerminalSurfaceKind::Agent => TerminalInputShortcutScope::AgentSurface,
                TerminalSurfaceKind::Workspace => {
                    TerminalInputShortcutScope::WorkspaceDrawerSurface
                }
                TerminalSurfaceKind::Helix => TerminalInputShortcutScope::HelixDrawerSurface,
            };
            let host_output = host.ui(
                ui,
                theme_mode,
                terminal_font_size,
                request_focus,
                accept_input,
                shortcut_scope,
            );
            let agent_exit = if kind == TerminalSurfaceKind::Agent && host.launched_with_resume() {
                host.take_abnormal_agent_exit()
            } else {
                None
            };
            (agent_exit, host_output)
        });
        let (agent_exit, host_output) = terminal_output.inner;
        if kind == TerminalSurfaceKind::Agent {
            self.active_agent_terminal_rect =
                host_output.as_ref().and_then(|output| output.input_rect);
            if host_output
                .as_ref()
                .is_some_and(|output| output.input_submitted)
            {
                self.clear_agent_input_translation_state();
            }
        }
        let mut quick_reply = None;
        if kind == TerminalSurfaceKind::Agent {
            if let Some(host_output) = host_output.as_ref() {
                host_output.response.context_menu(|ui| {
                    ui.horizontal(|ui| {
                        for reply in AGENT_QUICK_REPLIES {
                            if ui.button(reply).clicked() {
                                quick_reply = Some(reply);
                                ui.close_menu();
                            }
                        }
                    });
                    let custom_replies = agent_custom_quick_reply_lines(&custom_quick_replies);
                    if !custom_replies.is_empty() {
                        ui.separator();
                        ui.horizontal_wrapped(|ui| {
                            for reply in custom_replies {
                                if ui.button(reply).clicked() {
                                    quick_reply = Some(reply);
                                    ui.close_menu();
                                }
                            }
                        });
                    }
                });
            }
        }
        if let Some(reply) = quick_reply {
            self.submit_active_agent_quick_reply(ui.ctx(), reply);
        }
        if let Some(output) = host_output
            && let Some(click) = output.output_click
        {
            self.handle_terminal_output_click(ui.ctx(), click, output.output_click_copy_only);
        }
        if let Some(exit) = agent_exit
            && self.active_app_dialog().is_none()
        {
            self.set_active_app_dialog(Some(AppDialog::AgentExitedAbnormally { exit }));
        }
    }

    /// Handles a clicked Agent terminal output token.
    pub(super) fn handle_terminal_output_click(
        &mut self,
        ctx: &egui::Context,
        click: TerminalOutputClick,
        copy_only: bool,
    ) {
        if copy_only {
            self.copy_terminal_output_click(ctx, &click);
            return;
        }
        match click {
            TerminalOutputClick::PathCandidate(click) => {
                self.spawn_terminal_output_path_classify_task(ctx, self.active_workspace, click);
            }
            TerminalOutputClick::FileLine(click) => {
                self.open_terminal_file_line(ctx, click);
            }
            TerminalOutputClick::RevealPath(path) => {
                self.spawn_reveal_path_task(ctx, path);
            }
            TerminalOutputClick::Url(url) => self.open_terminal_url_in_browser(&url),
        }
    }

    /// Copies the resolved Agent terminal output target without opening it.
    pub(super) fn copy_terminal_output_click(
        &mut self,
        ctx: &egui::Context,
        click: &TerminalOutputClick,
    ) {
        let Some(text) = (match click {
            TerminalOutputClick::FileLine(click) => self
                .terminal_file_line_click_target(click)
                .map(|(_, target)| target),
            TerminalOutputClick::PathCandidate(click) => self
                .terminal_file_line_click_target(click)
                .map(|(_, target)| target),
            TerminalOutputClick::RevealPath(path) => Some(path.display().to_string()),
            TerminalOutputClick::Url(url) => Some(url.clone()),
        }) else {
            return;
        };
        ctx.copy_text(text);
    }

    /// 应用后台分类后的终端路径点击动作。
    pub(super) fn apply_classified_terminal_output_click(
        &mut self,
        ctx: &egui::Context,
        click: TerminalOutputClick,
    ) {
        match click {
            TerminalOutputClick::FileLine(click) => self.open_terminal_file_line(ctx, click),
            TerminalOutputClick::RevealPath(path) => self.spawn_reveal_path_task(ctx, path),
            TerminalOutputClick::Url(url) => self.open_terminal_url_in_browser(&url),
            TerminalOutputClick::PathCandidate(click) => {
                self.spawn_terminal_output_path_classify_task(ctx, self.active_workspace, click);
            }
        }
    }

    /// Translates the visible active Agent input draft in a helper popup.
    pub(super) fn translate_active_agent_input(&mut self, ctx: &egui::Context) {
        let workspace_index = self.active_workspace;
        let agent_slot = self.active_agent_slot();
        let Some(text) = self.active_agent_input_text_snapshot() else {
            self.push_toast(
                i18n::text(self.app_language, "No Agent input draft to translate"),
                theme::warning(),
            );
            return;
        };
        self.start_agent_input_translation(ctx, workspace_index, agent_slot, text, true);
    }

    /// Starts translation for a captured Agent input draft.
    fn start_agent_input_translation(
        &mut self,
        ctx: &egui::Context,
        workspace_index: usize,
        agent_slot: AgentSlotId,
        text: String,
        show_popup: bool,
    ) {
        let translatable_text = Self::agent_input_translation_model_text(&text);
        if translatable_text.trim().is_empty()
            || !Self::agent_input_translation_needs_translation(&translatable_text)
        {
            self.agent_input_translation_watch = None;
            self.agent_input_translation_popup = None;
            if show_popup {
                self.push_toast(
                    i18n::text(self.app_language, "No translatable Agent input text"),
                    theme::warning(),
                );
            }
            return;
        }
        let source_has_images = Self::agent_input_has_image_placeholder(&text);
        let key = (workspace_index, agent_slot.clone());
        if self.agent_input_translation_in_flight.contains(&key) {
            return;
        }
        self.agent_input_translation_in_flight.insert(key);
        self.agent_input_translation_popup = Some(AgentInputTranslationPopup {
            message: if show_popup {
                i18n::text(self.app_language, "Translating...").to_string()
            } else {
                i18n::text(self.app_language, "Auto translating...").to_string()
            },
        });
        self.spawn_agent_input_translation_task(
            ctx,
            workspace_index,
            agent_slot,
            text,
            translatable_text,
            source_has_images,
        );
    }

    /// Checks whether the Agent draft has been idle long enough to auto-translate.
    pub(super) fn process_agent_input_translation_auto_trigger(&mut self, ctx: &egui::Context) {
        if !self.runtime_settings.agent_input_translation_auto_trigger {
            self.agent_input_translation_watch = None;
            return;
        }
        if self.active_app_dialog().is_some()
            || self.active_reviewer_dialog().is_some()
            || self.extra_tools.open
            || self.notifications.open
        {
            return;
        }
        let Some(workspace) = self.current_workspace() else {
            self.agent_input_translation_watch = None;
            return;
        };
        if workspace.route != Route::Workspace || workspace.center_mode != CenterMode::Agent {
            self.agent_input_translation_watch = None;
            self.agent_input_translation_popup = None;
            return;
        }
        let workspace_index = self.active_workspace;
        let agent_slot = self.active_agent_slot();
        if self.agent_slot_activity(workspace_index, &agent_slot) == WorkspaceActivity::Busy {
            self.agent_input_translation_watch = None;
            self.agent_input_translation_popup = None;
            return;
        }
        let Some(text) = self.active_agent_input_text_snapshot() else {
            self.agent_input_translation_watch = None;
            self.agent_input_translation_popup = None;
            return;
        };
        if text.trim().is_empty() {
            self.agent_input_translation_watch = None;
            self.agent_input_translation_popup = None;
            return;
        }
        let now = Instant::now();
        if self
            .agent_input_translation_watch
            .as_ref()
            .is_none_or(|watch| {
                watch.workspace_index != workspace_index
                    || watch.agent_slot != agent_slot
                    || watch.text != text
            })
        {
            self.agent_input_translation_watch = Some(AgentInputTranslationWatch {
                workspace_index,
                agent_slot,
                text,
                changed_at: now,
                last_requested_text: None,
            });
            self.request_app_repaint_after(ctx, AGENT_INPUT_TRANSLATION_IDLE_DEBOUNCE);
            return;
        }
        let Some(watch) = self.agent_input_translation_watch.as_mut() else {
            return;
        };
        let idle = now.saturating_duration_since(watch.changed_at);
        if idle < AGENT_INPUT_TRANSLATION_IDLE_DEBOUNCE {
            self.request_app_repaint_after(ctx, AGENT_INPUT_TRANSLATION_IDLE_DEBOUNCE - idle);
            return;
        }
        if watch.last_requested_text.as_ref() == Some(&watch.text) {
            return;
        }
        let key = (watch.workspace_index, watch.agent_slot.clone());
        if self.agent_input_translation_in_flight.contains(&key) {
            self.request_app_repaint_after(ctx, AGENT_INPUT_TRANSLATION_IDLE_DEBOUNCE);
            return;
        }
        let workspace_index = watch.workspace_index;
        let agent_slot = watch.agent_slot.clone();
        let text = watch.text.clone();
        watch.last_requested_text = Some(text.clone());
        self.start_agent_input_translation(ctx, workspace_index, agent_slot, text, false);
    }

    /// Applies the last translation by replacing the current Agent input draft.
    pub(super) fn apply_last_agent_input_translation(&mut self, ctx: &egui::Context) {
        let Some(translation) = self.last_agent_input_translation.clone() else {
            self.push_toast(
                i18n::text(self.app_language, "No Agent input translation to apply"),
                theme::warning(),
            );
            return;
        };
        let active_slot = self.active_agent_slot();
        if translation.workspace_index != self.active_workspace
            || translation.agent_slot != active_slot
        {
            self.push_toast(
                i18n::text(
                    self.app_language,
                    "Translation belongs to another Agent slot",
                ),
                theme::warning(),
            );
            return;
        }
        if !self.active_agent_translation_source_matches(
            translation.workspace_index,
            &translation.agent_slot,
            &translation.source_text,
        ) {
            self.agent_input_translation_popup = None;
            self.push_toast(
                i18n::text(
                    self.app_language,
                    "Agent input changed; translation discarded",
                ),
                theme::warning(),
            );
            return;
        }
        self.ensure_terminal_host(ctx, TerminalSurfaceKind::Agent);
        let current_text_present = self.active_agent_input_text_snapshot().is_some();
        let Some(host) = self
            .terminal_hosts
            .get_mut(self.active_workspace)
            .and_then(|hosts| hosts.agents.get_mut(&active_slot))
            .and_then(|slot| slot.host.as_mut())
        else {
            self.push_toast(
                i18n::text(self.app_language, "Agent not ready"),
                theme::warning(),
            );
            return;
        };
        if translation.source_has_images {
            let target_text = Self::agent_input_restore_image_placeholders(&translation.text);
            if !host
                .replace_agent_input_text_preserving_images(&translation.source_text, &target_text)
            {
                self.push_toast(
                    i18n::text(
                        self.app_language,
                        "Image placeholder translation did not match source",
                    ),
                    theme::warning(),
                );
                return;
            }
        } else {
            host.replace_agent_input_text(&translation.text, current_text_present);
        }
        self.agent_input_translation_popup = None;
    }

    /// Returns whether the current Agent draft still matches a translation request.
    pub(super) fn active_agent_translation_source_matches(
        &self,
        workspace_index: usize,
        agent_slot: &AgentSlotId,
        source_text: &str,
    ) -> bool {
        workspace_index == self.active_workspace
            && agent_slot == &self.active_agent_slot()
            && self.agent_slot_activity(workspace_index, agent_slot) != WorkspaceActivity::Busy
            && self
                .active_agent_input_text_snapshot()
                .as_deref()
                .is_some_and(|current| current == source_text)
    }

    /// Returns the current active Agent terminal draft without mutating the child process.
    fn active_agent_input_text_snapshot(&self) -> Option<String> {
        let slot = self.active_agent_slot();
        self.terminal_hosts
            .get(self.active_workspace)?
            .agents
            .get(&slot)?
            .host
            .as_ref()?
            .agent_input_text_snapshot()
    }

    /// Returns draft text that should be sent to the translation model.
    fn agent_input_translation_model_text(text: &str) -> String {
        let mut output = String::new();
        let mut cursor = 0;
        while cursor < text.len() {
            let Some(relative_start) = text[cursor..].find("[Image #") else {
                output.push_str(&text[cursor..]);
                break;
            };
            let start = cursor + relative_start;
            output.push_str(&text[cursor..start]);
            let after_prefix = start + "[Image #".len();
            let Some(end) = Self::agent_image_placeholder_end(text, after_prefix) else {
                output.push_str(&text[start..]);
                break;
            };
            let number = &text[after_prefix..end - 1];
            output.push_str(&format!("{{#{number}}}"));
            cursor = end;
        }
        output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Converts model-preserved `{#N}` markers back to Codex image labels.
    fn agent_input_restore_image_placeholders(text: &str) -> String {
        let mut output = String::new();
        let mut cursor = 0;
        while cursor < text.len() {
            let Some(relative_start) = text[cursor..].find("{#") else {
                output.push_str(&text[cursor..]);
                break;
            };
            let start = cursor + relative_start;
            output.push_str(&text[cursor..start]);
            let after_prefix = start + "{#".len();
            let Some(end) = Self::agent_model_image_placeholder_end(text, after_prefix) else {
                output.push_str(&text[start..]);
                break;
            };
            let number = &text[after_prefix..end - 1];
            output.push_str(&format!("[Image #{number}]"));
            cursor = end;
        }
        output
    }

    /// Returns whether a draft contains a Codex image attachment placeholder.
    fn agent_input_has_image_placeholder(text: &str) -> bool {
        let mut cursor = 0;
        while cursor < text.len() {
            let Some(relative_start) = text[cursor..].find("[Image #") else {
                return false;
            };
            let after_prefix = cursor + relative_start + "[Image #".len();
            if Self::agent_image_placeholder_end(text, after_prefix).is_some() {
                return true;
            }
            cursor = after_prefix;
        }
        false
    }

    /// Returns the end byte index for `[Image #N]` starting after `#`.
    fn agent_image_placeholder_end(text: &str, after_prefix: usize) -> Option<usize> {
        let mut saw_digit = false;
        for (offset, ch) in text[after_prefix..].char_indices() {
            if ch.is_ascii_digit() {
                saw_digit = true;
                continue;
            }
            return (saw_digit && ch == ']').then_some(after_prefix + offset + ch.len_utf8());
        }
        None
    }

    /// Returns the end byte index for `{#N}` starting after `#`.
    fn agent_model_image_placeholder_end(text: &str, after_prefix: usize) -> Option<usize> {
        let mut saw_digit = false;
        for (offset, ch) in text[after_prefix..].char_indices() {
            if ch.is_ascii_digit() {
                saw_digit = true;
                continue;
            }
            return (saw_digit && ch == '}').then_some(after_prefix + offset + ch.len_utf8());
        }
        None
    }

    /// Clears all transient translation state for the active Agent input.
    pub(super) fn clear_agent_input_translation_state(&mut self) {
        self.agent_input_translation_watch = None;
        self.agent_input_translation_popup = None;
        self.last_agent_input_translation = None;
    }

    /// Returns whether quick translation is useful for the captured draft.
    fn agent_input_translation_needs_translation(text: &str) -> bool {
        text.chars().any(is_cjk_or_japanese_letter)
    }

    /// Opens a clicked Agent terminal URL in the system browser.
    pub(super) fn open_terminal_url_in_browser(&mut self, url: &str) {
        if let Err(error) = webbrowser::open(url) {
            self.push_toast(
                i18n::text_with_arg(
                    self.app_language,
                    "Failed to open browser: {error}",
                    "{error}",
                    error.to_string(),
                ),
                theme::warning(),
            );
        }
    }

    pub(super) fn ensure_terminal_host(&mut self, ctx: &egui::Context, kind: TerminalSurfaceKind) {
        self.spawn_terminal_host_for_workspace(ctx, self.active_workspace, kind);
    }

    /// Builds terminal launch metadata for an agent slot.
    pub(super) fn agent_workspace_for_slot(
        &self,
        workspace_index: usize,
        slot: &AgentSlotId,
    ) -> Option<WorkspaceViewData> {
        let mut workspace = self.workspaces.get(workspace_index)?.clone();
        if let AgentSlotId::Subagent(id) = slot {
            let subagent = workspace
                .subagents
                .iter()
                .find(|subagent| &subagent.id == id)?;
            workspace.name = format!("{} · {}", workspace.name, subagent.name);
            workspace.agent_kind = subagent.agent_kind;
            workspace.agent_id = subagent.agent_id.clone();
            workspace.session_id = subagent.session_id.clone();
            workspace.activity = subagent.activity;
        }
        Some(workspace)
    }

    /// Returns the last known activity for an agent slot.
    pub(super) fn agent_slot_activity(
        &self,
        workspace_index: usize,
        slot: &AgentSlotId,
    ) -> WorkspaceActivity {
        let Some(workspace) = self.workspaces.get(workspace_index) else {
            return WorkspaceActivity::Unknown;
        };
        match slot {
            AgentSlotId::Main => workspace.activity,
            AgentSlotId::Subagent(id) => workspace
                .subagents
                .iter()
                .find(|subagent| &subagent.id == id)
                .map(|subagent| subagent.activity)
                .unwrap_or(WorkspaceActivity::Unknown),
        }
    }

    /// Recreates a terminal host after its child process exits.
    pub(super) fn respawn_exited_terminal_host(
        &mut self,
        ctx: &egui::Context,
        kind: TerminalSurfaceKind,
    ) {
        let Some(hosts) = self.terminal_hosts.get_mut(self.active_workspace) else {
            return;
        };
        let agent_slot = self
            .active_agent_slots
            .get(self.active_workspace)
            .cloned()
            .unwrap_or(AgentSlotId::Main);
        let exited = match kind {
            TerminalSurfaceKind::Agent => hosts
                .agents
                .get(&agent_slot)
                .and_then(|slot| slot.host.as_ref())
                .is_some_and(GuiTerminalHost::has_exited),
            TerminalSurfaceKind::Workspace => hosts
                .workspace
                .as_ref()
                .is_some_and(GuiTerminalHost::has_exited),
            TerminalSurfaceKind::Helix => false,
        };
        if exited {
            match kind {
                TerminalSurfaceKind::Agent => {
                    hosts.agents.entry(agent_slot).or_default().host = None;
                }
                TerminalSurfaceKind::Workspace => hosts.workspace = None,
                TerminalSurfaceKind::Helix => {}
            }
            self.spawn_terminal_host_for_workspace(ctx, self.active_workspace, kind);
        }
    }

    pub(super) fn spawn_terminal_host_for_workspace(
        &mut self,
        ctx: &egui::Context,
        workspace_index: usize,
        kind: TerminalSurfaceKind,
    ) {
        let repaint_interval = self.max_repaint_interval();
        if workspace_index >= self.terminal_hosts.len() {
            return;
        }
        if kind == TerminalSurfaceKind::Helix {
            return;
        };
        match kind {
            TerminalSurfaceKind::Agent => {
                let slot_id = self
                    .active_agent_slots
                    .get(workspace_index)
                    .cloned()
                    .unwrap_or(AgentSlotId::Main);
                let Some(workspace) = self.agent_workspace_for_slot(workspace_index, &slot_id)
                else {
                    return;
                };
                let Some(hosts) = self.terminal_hosts.get_mut(workspace_index) else {
                    return;
                };
                let slot = hosts.agents.entry(slot_id.clone()).or_default();
                if slot.host.as_ref().is_some_and(GuiTerminalHost::has_exited) {
                    slot.host = None;
                }
                match slot.host.as_mut() {
                    Some(host) => host.sync_workspace_metadata(&workspace),
                    None => {
                        let key = TerminalSpawnKey {
                            index: workspace_index,
                            kind,
                            agent_slot: slot_id,
                        };
                        self.spawn_terminal_host_task(ctx, key, workspace);
                    }
                }
            }
            TerminalSurfaceKind::Workspace => {
                let Some(hosts) = self.terminal_hosts.get_mut(workspace_index) else {
                    return;
                };
                if hosts
                    .workspace
                    .as_ref()
                    .is_some_and(GuiTerminalHost::has_exited)
                {
                    hosts.workspace = None;
                }
                let Some(workspace) = self.workspaces.get(workspace_index).cloned() else {
                    return;
                };
                match hosts.workspace.as_mut() {
                    Some(host) => host.sync_workspace_metadata(&workspace),
                    None => {
                        let key = TerminalSpawnKey {
                            index: workspace_index,
                            kind,
                            agent_slot: AgentSlotId::Main,
                        };
                        self.spawn_terminal_host_task(ctx, key, workspace);
                    }
                }
            }
            TerminalSurfaceKind::Helix => unreachable!("helix is handled by ensure_helix_host"),
        }
    }

    pub(super) fn helix_host_surface(&mut self, ui: &mut Ui) {
        let Some(hosts) = self.terminal_hosts.get_mut(self.active_workspace) else {
            return;
        };
        let restart_spec = hosts
            .helix
            .as_ref()
            .filter(|host| host.has_exited())
            .and_then(|host| host.helix_spec().cloned());
        if let Some(spec) = restart_spec {
            hosts.helix = None;
            self.ensure_helix_host(ui.ctx(), spec);
        }
        let accept_input = self.reviewer_helix_drawer_is_open()
            && self.active_app_dialog().is_none()
            && self.active_reviewer_dialog().is_none()
            && !self.extra_tools.open
            && !self.notifications.open
            && !self.suppress_default_agent_input;
        let Some(hosts) = self.terminal_hosts.get_mut(self.active_workspace) else {
            return;
        };
        let Some(host) = hosts.helix.as_mut() else {
            let retry = terminal_error_panel(
                ui,
                "Helix failed to start",
                hosts
                    .helix_error
                    .as_deref()
                    .unwrap_or("No backend error was reported."),
                self.app_language,
            );
            if retry {
                hosts.helix_error = None;
            }
            return;
        };
        let terminal_size = Vec2::new(ui.available_width(), ui.available_height().max(1.0));
        let theme_mode = self.theme_mode;
        let terminal_font_size =
            effective_surface_font_size(&self.font_settings, &self.font_settings.terminal);
        ui.allocate_ui(terminal_size, |ui| {
            let _ = host.ui(
                ui,
                theme_mode,
                terminal_font_size,
                true,
                accept_input,
                TerminalInputShortcutScope::HelixDrawerSurface,
            );
        });
    }

    pub(super) fn ensure_helix_host(&mut self, ctx: &egui::Context, spec: HelixLaunchSpec) {
        if self.active_workspace >= self.terminal_hosts.len() {
            return;
        }
        let workspace_index = self.active_workspace;
        let Some(workspace) = self.workspaces.get(workspace_index).cloned() else {
            return;
        };
        let Some(hosts) = self.terminal_hosts.get_mut(workspace_index) else {
            return;
        };
        let needs_spawn = hosts.helix.as_ref().and_then(GuiTerminalHost::helix_spec) != Some(&spec);
        if !needs_spawn {
            hosts.helix_error = None;
            return;
        }
        hosts.helix = None;
        let key = TerminalSpawnKey {
            index: workspace_index,
            kind: TerminalSurfaceKind::Helix,
            agent_slot: AgentSlotId::Main,
        };
        self.spawn_helix_host_task(ctx, key, workspace, spec);
    }

    pub(super) fn copy_to_clipboard_and_paste_to_agent(
        &mut self,
        ctx: &egui::Context,
        text: &str,
    ) -> bool {
        ctx.copy_text(text.to_string());
        self.ensure_terminal_host(ctx, TerminalSurfaceKind::Agent);
        let slot_id = self.active_agent_slot();
        let Some(host) = self
            .terminal_hosts
            .get_mut(self.active_workspace)
            .and_then(|hosts| hosts.agents.get_mut(&slot_id))
            .and_then(|slot| slot.host.as_mut())
        else {
            return false;
        };
        host.paste_text(text);
        true
    }

    /// 向当前 Agent slot 直接提交一条快捷回复。
    pub(super) fn submit_active_agent_quick_reply(
        &mut self,
        ctx: &egui::Context,
        text: &str,
    ) -> bool {
        self.ensure_terminal_host(ctx, TerminalSurfaceKind::Agent);
        let slot_id = self.active_agent_slot();
        let Some(host) = self
            .terminal_hosts
            .get_mut(self.active_workspace)
            .and_then(|hosts| hosts.agents.get_mut(&slot_id))
            .and_then(|slot| slot.host.as_mut())
        else {
            return false;
        };
        // 触发条件：右键菜单需要把预设短语作为完整请求提交给 Agent。
        // 不能走普通字节输入：Codex 新 keymap 下裸 \r 可能是 Ctrl+M 换行。
        // 防止菜单项只把文本写进输入框，却没有真正发起请求。
        host.paste_text(text);
        host.submit_current_input();
        true
    }

    /// Opens a clicked Agent terminal `path:line` with the file-type owner.
    pub(super) fn open_terminal_file_line(
        &mut self,
        ctx: &egui::Context,
        click: TerminalFileLineClick,
    ) {
        let Some((file, target)) = self.terminal_file_line_click_target(&click) else {
            return;
        };
        // 触发条件：Agent 输出里的 Markdown 文件引用被左键点击。
        // 不能走常规 Helix 路径：Markdown 是本应用的原生文档类型。
        // 防止回归：点击 .md 时跳出内置 editor，破坏 recent/diff 记录。
        if terminal_file_line_is_markdown(&file) {
            self.open_terminal_markdown_file(file);
            return;
        }
        self.open_terminal_file_line_in_helix(ctx, click, file, target);
    }

    /// Opens a clicked Agent terminal Markdown file in the native editor.
    fn open_terminal_markdown_file(&mut self, file: std::path::PathBuf) {
        let path = self.workspace_relative_or_absolute(&file);
        self.request_open_file(path);
        if let Some(workspace) = self.current_workspace_mut() {
            workspace.route = Route::Workspace;
            workspace.center_mode = CenterMode::Editor;
            workspace.previous_center_mode = CenterMode::Editor;
        }
        self.persist_workspaces();
    }

    /// Opens a clicked Agent terminal non-Markdown `path:line` in Helix.
    fn open_terminal_file_line_in_helix(
        &mut self,
        ctx: &egui::Context,
        click: TerminalFileLineClick,
        file: std::path::PathBuf,
        target: String,
    ) {
        let Some(workspace_dir) = self
            .current_workspace()
            .map(|workspace| workspace.path.clone())
        else {
            return;
        };
        ctx.copy_text(target);
        self.spawn_terminal_file_helix_spec_task(
            ctx,
            self.active_workspace,
            workspace_dir,
            file,
            click.line,
        );
    }

    /// Formats the final Helix target for a clicked Agent file reference.
    pub(super) fn terminal_file_line_click_target(
        &self,
        click: &TerminalFileLineClick,
    ) -> Option<(std::path::PathBuf, String)> {
        let workspace_dir = self
            .current_workspace()
            .map(|workspace| workspace.path.clone())?;
        let file = resolve_terminal_file_line_path(&workspace_dir, &click.path);
        let target = if let Some(end_line) = click.end_line {
            format!("{}:{}-{}", file.display(), click.line, end_line)
        } else {
            format!("{}:{}", file.display(), click.line)
        };
        Some((file, target))
    }
}

/// Detects Markdown files that should stay inside the native editor.
fn terminal_file_line_is_markdown(path: &std::path::Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("md"))
}

/// Detects Chinese/Japanese letters while ignoring CJK punctuation-only text.
fn is_cjk_or_japanese_letter(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3040..=0x30ff
            | 0x3400..=0x4dbf
            | 0x4e00..=0x9fff
            | 0xf900..=0xfaff
            | 0x20000..=0x2ebef
            | 0x30000..=0x323af
    )
}

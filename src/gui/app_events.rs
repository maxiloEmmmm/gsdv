//! AppEvent 唯一队列和事件处理。
//!
//! 本模块是 UI 状态变更的核心通道：只能 drain `AppEvent`、修改内存
//! 中的渲染状态，或者派发后台任务；不能在这里做阻塞 IO 或长计算。

use super::*;

impl GsdvGuiApp {
    /// 事件总控入口，只允许通过唯一队列修改跨组件 UI 状态。
    pub(super) fn process_update_events(&mut self, ctx: &egui::Context) {
        crate::gui::perf_log::count("app.process_update_events");
        let started_at = Instant::now();
        let budget = self.max_repaint_interval();
        self.suppress_default_agent_input = false;
        self.suppress_editor_input = false;

        self.enqueue_update_events(ctx);
        self.send_input_runtime_request(ctx);
        self.drain_app_events(ctx, started_at, budget);
        crate::gui::perf_log::duration_us("app.process_update_events_us", started_at.elapsed());
    }

    /// 把本帧到期信号转换成 AppEvent，避免 update 分散消费多路状态。
    ///
    /// 这里只允许做轻量到期判断和事件入队。
    /// 不能在这里直接改业务状态，也不能 drain 子系统队列。
    pub(super) fn enqueue_update_events(&self, ctx: &egui::Context) {
        if self.has_pending_settings_side_effects() {
            self.queue_app_event(AppEvent::ProcessPendingSettingsSideEffects);
        }
        if !self.pending_reviewer_loads.is_empty() {
            self.queue_app_event(AppEvent::ProcessPendingReviewerLoads);
        }
        if !self.pending_markdown_reparse.is_empty() {
            self.queue_app_event(AppEvent::ProcessPendingMarkdownReparse);
        }
        if !self.pending_markdown_outline_collapse.is_empty() {
            self.queue_app_event(AppEvent::ProcessPendingMarkdownOutlineCollapse);
        }
        if !self.pending_memo_saves.is_empty() {
            self.queue_app_event(AppEvent::ProcessPendingMemoSaves);
        }
        if self.pending_input_repaint {
            self.queue_app_event(AppEvent::ProcessPendingInputRepaint);
        }
        if self.screenshot_request_poll_due() {
            self.queue_app_event(AppEvent::HandleScreenshotRequestFile);
        }
        if self
            .next_fs_watch_dirty_delay()
            .is_some_and(|delay| delay.is_zero())
        {
            self.queue_app_event(AppEvent::ProcessFsWatchDirty);
        }
        if self
            .next_pending_agent_theme_restart_delay()
            .is_some_and(|delay| delay.is_zero())
        {
            self.queue_app_event(AppEvent::FinishPendingAgentThemeRestarts);
        }
        if self
            .next_agent_busy_watchdog_delay()
            .is_some_and(|delay| delay.is_zero())
        {
            self.queue_app_event(AppEvent::ProcessAgentBusyWatchdogs);
        }
        if self.pomodoro_state_event_due() {
            self.queue_app_event(AppEvent::ProcessPomodoroState);
        }
        if self.extra_tools_event_due() {
            self.queue_app_event(AppEvent::ProcessExtraTools);
        }
    }

    /// 投递 AppEvent，是跨线程/跨模块请求 UI 状态变化的唯一入口。
    ///
    /// channel 关闭只会发生在 app 退出阶段，丢弃错误即可。
    /// 入队后主动唤醒 UI，避免 render/handler 后半段产生的事件滞留。
    pub(super) fn queue_app_event(&self, event: AppEvent) {
        crate::gui::perf_log::count("app.queue_event");
        if self.app_event_tx.send(event).is_ok()
            && let Some(ctx) = self.app_repaint_ctx.as_ref()
        {
            self.repaint_controller.request_repaint(ctx);
        }
    }

    /// 将本帧 egui 原生输入快照交给 input runtime 解析。
    pub(super) fn send_input_runtime_request(&self, ctx: &egui::Context) {
        let input = ctx.input(Clone::clone);
        if input.events.is_empty() {
            return;
        }
        let Some(request) = self.input_runtime_request(ctx, input) else {
            return;
        };
        let _ = self.input_runtime_tx.send(request);
    }

    /// 构建 input runtime 所需的只读 UI 状态快照。
    pub(super) fn input_runtime_request(
        &self,
        ctx: &egui::Context,
        input: egui::InputState,
    ) -> Option<InputRuntimeRequest> {
        let workspace = self.current_workspace()?;
        let workspace_terminal_open = self.workspace_terminal_drawer_is_open();
        let reviewer_helix_open = self.reviewer_helix_drawer_is_open();
        let active_workspace = self.active_workspace;
        let wants_keyboard_input = ctx.wants_keyboard_input();
        let active_app_dialog_open = self.active_app_dialog().is_some();
        let agent_translation_dialog_open =
            self.active_app_dialog().is_some_and(|dialog| match dialog {
                AppDialog::Message { title, .. } => title.starts_with("Agent Input Translation"),
                _ => false,
            });
        let active_reviewer_dialog_open = self.active_reviewer_dialog().is_some();
        let notifications_open = self.notifications.open;
        let route = workspace.route;
        let outline_visible = should_show_outline_panel(Some(workspace));
        let recent_markdown_dialog_open = matches!(
            self.active_app_dialog(),
            Some(AppDialog::RecentMarkdownOutline { .. })
        );
        let center_mode = workspace.center_mode;
        let keyboard_layer_can_close_with_escape = self.keyboard_layer_can_close_with_escape();
        let in_reviewer_route = route == Route::Reviewer;
        let helix_shortcut_allowed =
            self.helix_shortcut_allowed(in_reviewer_route, wants_keyboard_input);
        let terminal_input_target = if reviewer_helix_open {
            None
        } else {
            default_terminal_input_target(workspace, workspace_terminal_open, reviewer_helix_open)
        };
        let terminal_surface_owns_input = terminal_input_target.is_some()
            && !active_app_dialog_open
            && !active_reviewer_dialog_open
            && !self.extra_tools.open
            && !notifications_open;
        let active_agent_slot = self.active_agent_slot();
        let terminal_kitty_keyboard_protocol = terminal_input_target.is_some_and(|target| {
            self.terminal_input_kitty_keyboard_protocol(
                active_workspace,
                target,
                &active_agent_slot,
            )
        });
        let active_agent_busy = self.agent_slot_activity(active_workspace, &active_agent_slot)
            == WorkspaceActivity::Busy;
        Some(InputRuntimeRequest {
            input,
            wants_keyboard_input,
            active_workspace,
            active_agent_slot,
            active_agent_busy,
            route,
            center_mode,
            active_app_dialog_open,
            extra_tools_open: self.extra_tools.open,
            agent_translation_dialog_open,
            active_reviewer_dialog_open,
            notifications_open,
            workspace_terminal_open,
            reviewer_helix_open,
            outline_visible,
            recent_markdown_dialog_open,
            keyboard_layer_can_close_with_escape,
            outline_tree_rect: self
                .outline_tree_rects
                .get(active_workspace)
                .copied()
                .flatten(),
            helix_shortcut_allowed,
            selected_reviewer_diff_row: self.selected_reviewer_diff_row_for_keyboard(),
            terminal_input_target,
            terminal_surface_owns_input,
            terminal_kitty_keyboard_protocol,
            repaint_ctx: ctx.clone(),
            repaint_controller: self.repaint_controller.clone(),
            pomodoro_enabled: self.runtime_settings.pomodoro_enabled,
            pomodoro_phase: self.pomodoro.phase,
        })
    }

    /// 判断是否存在 settings 保存副作用。
    pub(super) fn has_pending_settings_side_effects(&self) -> bool {
        self.pending_runtime_settings_save
            || self.pending_language_settings_save
            || self.pending_font_settings_save
            || self.pending_network_settings_save
            || self.pending_default_agent_kind_save
    }

    /// 查询 input runtime 默认 terminal 目标的 kitty 协议状态。
    fn terminal_input_kitty_keyboard_protocol(
        &self,
        workspace_index: usize,
        target: TerminalSurfaceKind,
        agent_slot: &AgentSlotId,
    ) -> bool {
        let Some(hosts) = self.terminal_hosts.get(workspace_index) else {
            return false;
        };
        let host = match target {
            TerminalSurfaceKind::Agent => hosts
                .agents
                .get(agent_slot)
                .and_then(|slot| slot.host.as_ref()),
            TerminalSurfaceKind::Workspace => hosts.workspace.as_ref(),
            TerminalSurfaceKind::Helix => hosts.helix.as_ref(),
        };
        // 触发条件：input runtime 在绘制路径之外补发 terminal 输入。
        // 不能默认旧协议：Codex working 时已可能启用 kitty 增强键盘。
        // 防止回归：事件队列先写裸 Esc，破坏后续 CSI-u 解析。
        host.is_some_and(GuiTerminalHost::kitty_keyboard_protocol_enabled)
    }

    /// 判断截图请求文件轮询是否到期。
    pub(super) fn screenshot_request_poll_due(&self) -> bool {
        self.screenshot_request_poll_enabled
            && !self.screenshot_request_read_in_flight
            && self.last_screenshot_request_poll.elapsed() >= SCREENSHOT_REQUEST_POLL_INTERVAL
    }

    /// 根据当前前台 route 开关 terminal 事件 repaint。
    pub(super) fn sync_terminal_event_repaint_flags(&mut self) {
        let active_workspace = self.active_workspace;
        let workspace_terminal_open = self.workspace_terminal_drawer_is_open();
        let reviewer_helix_open = self.reviewer_helix_drawer_is_open();
        for (index, hosts) in self.terminal_hosts.iter_mut().enumerate() {
            let is_active = index == active_workspace;
            for agent in hosts.agents.values() {
                if let Some(host) = agent.host.as_ref() {
                    host.set_runtime_theme_mode(self.theme_mode);
                    // 触发条件：subagent 可能隐藏在别的 slot 或 workspace 后。
                    // 不能关掉事件唤醒：隐藏输出也要推动状态流消费。
                    // 防止回归：Agent 输出要等无关 UI 输入后才被处理。
                    host.set_event_repaint_enabled(true);
                }
            }
            if let Some(host) = hosts.workspace.as_ref() {
                host.set_runtime_theme_mode(self.theme_mode);
                host.set_event_repaint_enabled(is_active && workspace_terminal_open);
            }
            if let Some(host) = hosts.helix.as_ref() {
                host.set_runtime_theme_mode(self.theme_mode);
                host.set_event_repaint_enabled(is_active && reviewer_helix_open);
            }
        }
    }

    /// 应用 terminal runtime 的粗粒度 UI 通知。
    ///
    /// 原始 PTY event 已由 terminal 自有 runtime 处理完成；这里不能
    /// 再 drain terminal 内部队列，只能更新 app 级轻量状态。
    pub(super) fn apply_terminal_runtime_event(
        &mut self,
        ctx: &egui::Context,
        event: TerminalRuntimeEvent,
    ) {
        crate::gui::perf_log::count(terminal_runtime_app_event_label(event.kind));
        if std::env::var_os("GSDV_AGENT_ESC_DEBUG").is_some()
            && let Some((workspace_index, slot)) = self.agent_terminal_slot_by_id(event.id)
        {
            eprintln!(
                "[gsdv][agent-esc][app-event-runtime] workspace={} slot={:?} id={} kind={:?}",
                workspace_index, slot, event.id, event.kind
            );
        }
        if event.kind == TerminalRuntimeEventKind::Output {
            self.record_agent_terminal_output(event.id);
        }
        if crate::gui::perf_log::enabled() {
            if self.terminal_runtime_event_visible(event.id) {
                crate::gui::perf_log::count("app.terminal_runtime.visible");
            } else {
                crate::gui::perf_log::count("app.terminal_runtime.hidden");
            }
        }
        self.request_app_repaint();
    }

    /// 判断 terminal runtime 事件是否来自当前可见 terminal surface。
    pub(super) fn terminal_runtime_event_visible(&self, terminal_id: u64) -> bool {
        let Some(workspace) = self.current_workspace() else {
            return false;
        };
        if workspace.route == Route::Workspace
            && workspace.center_mode == CenterMode::Agent
            && self
                .agent_terminal_slot_by_id(terminal_id)
                .is_some_and(|(index, slot)| {
                    index == self.active_workspace && slot == self.active_agent_slot()
                })
        {
            return true;
        }
        if self.workspace_terminal_drawer_is_open()
            && self
                .terminal_hosts
                .get(self.active_workspace)
                .and_then(|hosts| hosts.workspace.as_ref())
                .is_some_and(|host| host.id() == terminal_id)
        {
            return true;
        }
        self.reviewer_helix_drawer_is_open()
            && self
                .terminal_hosts
                .get(self.active_workspace)
                .and_then(|hosts| hosts.helix.as_ref())
                .is_some_and(|host| host.id() == terminal_id)
    }

    /// 记录指定 terminal 的 Agent 输出，用于 busy watchdog。
    pub(super) fn record_agent_terminal_output(&mut self, terminal_id: u64) {
        let now = Instant::now();
        let Some((index, slot)) = self.agent_terminal_slot_by_id(terminal_id) else {
            return;
        };
        if let Some(states) = self.agent_busy_watchdogs.get_mut(index) {
            states.entry(slot).or_default().record_output(now);
        }
    }

    /// 查找 terminal id 对应的 Agent slot。
    pub(super) fn agent_terminal_slot_by_id(
        &self,
        terminal_id: u64,
    ) -> Option<(usize, AgentSlotId)> {
        for (index, hosts) in self.terminal_hosts.iter().enumerate() {
            for (slot, agent) in hosts.agents.iter() {
                if agent
                    .host
                    .as_ref()
                    .is_some_and(|host| host.id() == terminal_id)
                {
                    return Some((index, slot.clone()));
                }
            }
        }
        None
    }

    /// 处理 Busy Agent 的无输出自动继续。
    pub(super) fn process_agent_busy_watchdogs(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let delay = agent_busy_auto_go_delay(&self.runtime_settings);
        let mut sends = Vec::new();
        let workspace_count = self.workspaces.len().min(self.agent_busy_watchdogs.len());

        for index in 0..workspace_count {
            let slots = agent_slots_for_workspace(&self.workspaces[index]);
            for slot in slots {
                let is_busy = self.agent_slot_activity(index, &slot) == WorkspaceActivity::Busy;
                let Some(states) = self.agent_busy_watchdogs.get_mut(index) else {
                    continue;
                };
                let state = states.entry(slot.clone()).or_default();

                if !is_busy {
                    state.reset_idle();
                    continue;
                }
                if state.busy_started_at.is_none() {
                    state.start_busy(now);
                    continue;
                }
                if state.auto_go_sent || !state.auto_go_due(now, delay) {
                    continue;
                }

                state.auto_go_sent = true;
                sends.push((index, slot));
            }
        }

        for (index, slot) in sends {
            let Some(host) = self
                .terminal_hosts
                .get_mut(index)
                .and_then(|hosts| hosts.agents.get_mut(&slot))
                .and_then(|slot| slot.host.as_mut())
            else {
                continue;
            };
            if host.has_exited() {
                continue;
            }
            // 触发条件：Busy 超过阈值且期间没有 PTY Wakeup 输出。
            // 不能走普通按键路径：当前焦点可能在别的 workspace 或弹窗里。
            // 防止后台 Codex 卡在等待继续输入时无人值守。
            host.write_bytes(AGENT_BUSY_AUTO_GO_INPUT);
            self.request_app_repaint();
        }
    }

    /// Advances the pomodoro timer and handles ready-to-work input.
    pub(super) fn process_pomodoro_state(&mut self, ctx: &egui::Context) {
        if !self.runtime_settings.pomodoro_enabled {
            return;
        }
        let now = Instant::now();
        match self.pomodoro.phase {
            PomodoroPhase::Working => {
                if now.duration_since(self.pomodoro.phase_started_at)
                    >= pomodoro_work_duration(&self.runtime_settings)
                {
                    self.pomodoro.start_resting(now);
                    self.push_pomodoro_notification(i18n::text_with_arg(
                        self.app_language,
                        "Work ended, resting for {minutes} minutes",
                        "{minutes}",
                        self.runtime_settings.pomodoro_rest_minutes.to_string(),
                    ));
                    self.request_app_repaint();
                }
            }
            PomodoroPhase::WaitingForRestQuiet => {
                if now.duration_since(self.pomodoro.phase_started_at)
                    >= POMODORO_REST_QUIET_DURATION
                {
                    self.pomodoro.start_resting(now);
                    self.push_pomodoro_notification(i18n::text(
                        self.app_language,
                        "Quiet now, continue resting",
                    ));
                    self.request_app_repaint();
                }
            }
            PomodoroPhase::Resting => {
                if now.duration_since(self.pomodoro.phase_started_at)
                    >= pomodoro_rest_duration(&self.runtime_settings)
                {
                    self.pomodoro.wait_for_work_input(now);
                    self.push_pomodoro_notification(i18n::text(
                        self.app_language,
                        "Rest ended, press any key to start work",
                    ));
                    self.request_app_repaint();
                }
            }
            PomodoroPhase::ReadyToWork => {}
            PomodoroPhase::ReturningToWork => {
                if now.duration_since(self.pomodoro.phase_started_at)
                    >= POMODORO_RETURN_TO_WORK_DURATION
                {
                    self.pomodoro.start_working(now);
                    self.push_pomodoro_notification(i18n::text_with_arg(
                        self.app_language,
                        "Working for {minutes} minutes",
                        "{minutes}",
                        self.runtime_settings.pomodoro_work_minutes.to_string(),
                    ));
                    self.request_app_repaint();
                }
            }
        }
    }

    /// 处理 input runtime 检测到的番茄钟相关输入。
    pub(super) fn process_pomodoro_input_detected(&mut self, ctx: &egui::Context) {
        if !self.runtime_settings.pomodoro_enabled {
            return;
        }
        let now = Instant::now();
        match self.pomodoro.phase {
            PomodoroPhase::WaitingForRestQuiet => {
                self.pomodoro.wait_for_rest_quiet(now);
                self.request_app_repaint();
            }
            PomodoroPhase::Resting => {
                self.pomodoro.wait_for_rest_quiet(now);
                self.push_pomodoro_notification(i18n::text(
                    self.app_language,
                    "Input detected, waiting for quiet to continue rest",
                ));
                self.request_app_repaint();
            }
            PomodoroPhase::ReadyToWork => {
                self.pomodoro.start_returning_to_work(now);
                self.push_pomodoro_notification(i18n::text(
                    self.app_language,
                    "Input detected, returning to work",
                ));
                self.request_app_repaint();
            }
            PomodoroPhase::Working | PomodoroPhase::ReturningToWork => {}
        }
    }

    /// Schedules another update when lightweight event handling used its slice.
    pub(super) fn event_budget_spent(
        &mut self,
        ctx: &egui::Context,
        started_at: Instant,
        budget: Duration,
    ) -> bool {
        if started_at.elapsed() < budget {
            return false;
        }
        self.request_app_repaint();
        true
    }

    /// 在当前帧预算内只负责取事件和调度消费。
    ///
    /// 这里不能直接做业务判断、IO、扫描或同步等待。
    /// 触发界面变化的 producer 必须先投 AppEvent。
    /// 每消费一个事件就检查预算，避免长队列独占 UI 线程。
    pub(super) fn drain_app_events(
        &mut self,
        ctx: &egui::Context,
        started_at: Instant,
        budget: Duration,
    ) {
        while let Ok(event) = self.app_event_rx.try_recv() {
            crate::gui::perf_log::count("app.drain_event");
            self.handle_app_event(ctx, event);
            if started_at.elapsed() >= budget {
                self.request_app_repaint();
                break;
            }
        }
    }

    /// 消费单个 AppEvent，只允许轻量改状态或派发后台任务。
    ///
    /// 禁止在这里做阻塞 IO、重 CPU、递归扫描或等待后台结果。
    /// 需要慢操作时只能 spawn，完成后再投递新的 AppEvent。
    /// 这里的 match 是业务路由边界，不是第二套事件循环。
    pub(super) fn handle_app_event(&mut self, ctx: &egui::Context, event: AppEvent) {
        match event {
            AppEvent::MarkdownParsed {
                index,
                source_text,
                outline_entries,
                preview_blocks,
            } => {
                if let Some(document) = self.documents.get_mut(index)
                    && document.text == source_text
                {
                    document.markdown_outline_entries = outline_entries;
                    document.markdown_preview_blocks = preview_blocks;
                    document.markdown_preview_metrics = None;
                    document.markdown_preview_metrics_width = 0.0;
                    document.markdown_preview_heading_offsets.clear();
                    document.markdown_preview_max_scroll_y = 0.0;
                }
            }
            AppEvent::MemoSaved { index, error } => {
                if let Some(slot) = self.memo_save_errors.get_mut(index) {
                    *slot = error;
                }
            }
            AppEvent::WorkspaceOutlineRefreshed { index, workspace } => {
                if let Some(current) = self.workspaces.get_mut(index)
                    && current.path == workspace.path
                {
                    current.outline = workspace.outline;
                    current.selected_file = workspace.selected_file;
                }
            }
            AppEvent::WorkflowTreeLoaded {
                index,
                workspace_path,
                result,
            } => {
                self.apply_workflow_tree_loaded(ctx, index, workspace_path, result);
            }
            AppEvent::WorkspaceAddPrepared { result } => {
                self.apply_workspace_add_result(ctx, result);
            }
            AppEvent::AgentStatusesRefreshed {
                workspaces,
                changed: statuses_changed,
            } => {
                if statuses_changed {
                    self.merge_agent_status_refresh(workspaces);
                }
            }
            AppEvent::ReviewerScriptsLoaded { result } => match result {
                Ok(scripts) => {
                    self.reviewer_scripts.scripts = scripts;
                    self.reviewer_scripts.last_error = None;
                }
                Err(error) => {
                    self.reviewer_scripts.scripts.clear();
                    self.reviewer_scripts.last_error = Some(error);
                }
            },
            AppEvent::ExtraToolsScanned {
                workspace_path,
                result,
            } => {
                self.apply_extra_tools_scan_result(ctx, workspace_path, result);
            }
            AppEvent::ExtraToolValueLoaded { key, result } => {
                self.apply_extra_tool_value_result(ctx, key, result);
            }
            AppEvent::ExtraToolActionFinished {
                key,
                action,
                result,
            } => {
                self.apply_extra_tool_action_result(ctx, key, action, result);
            }
            AppEvent::ExtraToolActionRequested { key, action } => {
                self.start_extra_tool_action(ctx, key, action);
            }
            AppEvent::ExtraToolInterruptRequested { key } => {
                self.interrupt_extra_tool_action(ctx, key);
            }
            AppEvent::DocumentLoaded {
                index,
                path,
                absolute,
                markdown_outline_collapsed,
                result,
            } => {
                if let Some(document) = self.documents.get_mut(index)
                    && document.loading_path.as_ref().map_or_else(
                        || document.path.as_ref() == Some(&path),
                        |loading| loading == &path,
                    )
                {
                    document.markdown_outline_collapsed = markdown_outline_collapsed;
                    match result {
                        Ok(loaded) => {
                            document.path = Some(path.clone());
                            document.loading_path = None;
                            document.text = loaded.text.clone();
                            document.saved_text = loaded.text;
                            document.markdown_outline_entries = loaded.outline_entries;
                            document.markdown_preview_blocks = loaded.preview_blocks;
                            document.markdown_preview_metrics = None;
                            document.markdown_preview_metrics_width = 0.0;
                            document.load_error = None;
                        }
                        Err(error) => {
                            document.path = Some(path.clone());
                            document.loading_path = None;
                            document.text.clear();
                            document.saved_text.clear();
                            document.markdown_outline_entries.clear();
                            document.markdown_preview_blocks.clear();
                            document.markdown_preview_metrics = None;
                            document.markdown_preview_metrics_width = 0.0;
                            document.load_error =
                                Some(format!("Failed to read {}: {error}", absolute.display()));
                        }
                    }
                    document.markdown_preview_heading_offsets.clear();
                    document.markdown_preview_max_scroll_y = 0.0;
                    document.markdown_editor_max_scroll_y = 0.0;
                }
            }
            AppEvent::DocumentSaved {
                index,
                path,
                text,
                result,
                diff_history_error,
            } => {
                if let Some(document) = self.documents.get_mut(index)
                    && document.path.as_ref() == Some(&path)
                {
                    match result {
                        Ok(()) => {
                            if document.text == text {
                                document.saved_text = text;
                            }
                            document.save_error = None;
                            if let Some(error) = diff_history_error {
                                self.push_toast(
                                    format!("Saved markdown; diff history failed: {error}"),
                                    theme::warning(),
                                );
                            } else {
                                self.push_toast(
                                    i18n::text(self.app_language, "Saved markdown file"),
                                    theme::success(),
                                );
                            }
                        }
                        Err(error) => {
                            document.save_error = Some(error);
                            self.push_toast(
                                i18n::text(self.app_language, "Save failed"),
                                theme::danger(),
                            );
                        }
                    }
                }
            }
            AppEvent::WorkflowStepSaved {
                index,
                target,
                result,
            } => {
                self.apply_workflow_step_saved(ctx, index, target, result);
            }
            AppEvent::WorkflowMutationFinished {
                index,
                request,
                result,
            } => {
                self.apply_workflow_mutation_finished(ctx, index, request, result);
            }
            AppEvent::MarkdownDiffPromptBuilt { result } => match result {
                Ok(text) if text.trim().is_empty() => {
                    self.push_toast(
                        i18n::text(self.app_language, "No recent Markdown diffs"),
                        theme::warning(),
                    );
                }
                Ok(text) => {
                    if self.copy_to_clipboard_and_paste_to_agent(ctx, &text) {
                        self.push_toast(
                            i18n::text(self.app_language, "Markdown diffs pasted to Agent"),
                            theme::success(),
                        );
                    } else {
                        self.push_toast(
                            i18n::text(self.app_language, "Agent not ready; Markdown diffs copied"),
                            theme::warning(),
                        );
                    }
                }
                Err(error) => {
                    self.push_toast(
                        format!("Markdown diff paste failed: {error}"),
                        theme::danger(),
                    );
                }
            },
            AppEvent::AgentInputTranslationFinished {
                workspace_index,
                agent_slot,
                source_text,
                source_has_images,
                result,
            } => {
                self.agent_input_translation_in_flight
                    .remove(&(workspace_index, agent_slot.clone()));
                if !self.active_agent_translation_source_matches(
                    workspace_index,
                    &agent_slot,
                    &source_text,
                ) {
                    self.agent_input_translation_popup = None;
                    return;
                }
                let message = match result {
                    Ok(text) if text.trim().is_empty() => {
                        i18n::text(self.app_language, "No translation returned.").to_string()
                    }
                    Ok(text) => {
                        self.last_agent_input_translation = Some(AgentInputTranslation {
                            workspace_index,
                            agent_slot: agent_slot.clone(),
                            source_text,
                            source_has_images,
                            text: text.clone(),
                        });
                        text
                    }
                    Err(error) => error,
                };
                self.agent_input_translation_popup = Some(AgentInputTranslationPopup { message });
            }
            AppEvent::RuntimeFontsPrepared { settings, fonts } => {
                if self.font_settings == settings {
                    theme::apply_runtime_font_definitions(ctx, fonts);
                }
            }
            AppEvent::FileMutationFinished(result) => {
                self.apply_file_mutation_result(ctx, result);
            }
            AppEvent::WorkspaceCloseSidecarsDeleted {
                index,
                workspace_path,
                result,
            } => {
                self.apply_workspace_close_sidecar_result(ctx, index, workspace_path, result);
            }
            AppEvent::ScreenshotSaved { path, result } => match result {
                Ok(()) => {
                    self.last_screenshot_path = Some(path);
                }
                Err(error) => {
                    self.push_toast(
                        i18n::text_with_arg(
                            self.app_language,
                            "Screenshot failed: {error}",
                            "{error}",
                            error,
                        ),
                        theme::danger(),
                    );
                }
            },
            AppEvent::ScreenshotCaptured { purpose, image } => {
                self.handle_screenshot_captured(ctx, purpose, image);
            }
            AppEvent::ScreenshotRequestLoaded { result } => {
                self.screenshot_request_read_in_flight = false;
                match result {
                    Ok(Some(command)) => {
                        let source = self.apply_debug_capture_command(command.trim());
                        self.request_egui_screenshot(ctx, source);
                    }
                    Ok(None) => {}
                    Err(error) => self.push_toast(
                        i18n::text_with_arg(
                            self.app_language,
                            "Screenshot request file failed: {error}",
                            "{error}",
                            error,
                        ),
                        theme::danger(),
                    ),
                }
            }
            AppEvent::TerminalHostSpawned { key, result } => {
                let spawned = result.is_ok();
                self.pending_terminal_spawns.remove(&key);
                self.apply_terminal_spawn_result(key, result);
                if spawned {
                    self.queue_app_event(AppEvent::SyncTerminalEventRepaintFlags);
                }
            }
            AppEvent::TerminalRuntime(event) => {
                self.apply_terminal_runtime_event(ctx, event);
            }
            AppEvent::TerminalFileHelixSpecBuilt {
                workspace_index,
                spec,
            } => {
                if workspace_index == self.active_workspace {
                    self.open_terminal_file_helix_drawer(ctx, spec);
                }
            }
            AppEvent::TerminalOutputPathClassified {
                workspace_index,
                click,
            } => {
                if workspace_index == self.active_workspace {
                    self.apply_classified_terminal_output_click(ctx, click);
                }
            }
            AppEvent::HelixBinaryChecked { available } => {
                self.helix_binary_check_in_flight = false;
                self.helix_binary_available = available;
                if available {
                    if let Some(request) = self.pending_helix_open_request.take() {
                        self.open_checked_helix_request(ctx, request);
                    }
                } else {
                    self.pending_helix_open_request = None;
                    self.push_toast(
                        i18n::text(self.app_language, "Helix executable `hx` was not found"),
                        theme::warning(),
                    );
                }
            }
            AppEvent::RevealPathFinished { result } => {
                if let Err(error) = result {
                    self.set_active_app_dialog(Some(AppDialog::Message {
                        title: i18n::text(self.app_language, "Reveal Failed").to_string(),
                        message: error,
                    }));
                }
            }
            AppEvent::ReviewerBranchChoicesLoaded { repo, result } => {
                self.apply_reviewer_branch_choices_result(repo, result);
            }
            AppEvent::ReviewerBranchSwitchFinished {
                repo,
                target,
                result,
            } => {
                self.apply_reviewer_branch_switch_result(repo, target, result);
            }
            AppEvent::ReviewerAdapterLoaded { index, result } => {
                self.reviewer_loads_in_flight.remove(&index);
                let mut next_reviewer_task = None;
                match result {
                    Ok(adapter) => {
                        if let Some(snapshot_slot) = self.reviewer_snapshots.get_mut(index) {
                            *snapshot_slot = Some(adapter.snapshot().clone());
                        }
                        if let Some(slot) = self.reviewer_adapters.get_mut(index) {
                            *slot = Some(adapter);
                        }
                        if let Some((task, effect)) = self.pop_pending_reviewer_adapter_task(index)
                        {
                            next_reviewer_task = Some((index, task, effect));
                        }
                    }
                    Err(error) => {
                        self.push_toast(
                            i18n::text_with_arg(
                                self.app_language,
                                "Reviewer failed to load: {error}",
                                "{error}",
                                error,
                            ),
                            theme::danger(),
                        );
                    }
                }
                if let Some((index, task, effect)) = next_reviewer_task {
                    self.spawn_indexed_reviewer_adapter_task(ctx, index, task, effect);
                } else if let Some((row_budget, load_more)) =
                    self.pending_reviewer_git_data_budget.remove(&index)
                {
                    self.spawn_indexed_reviewer_git_data_task(ctx, index, row_budget, load_more);
                }
            }
            AppEvent::ReviewerAdapterTaskFinished {
                index,
                result,
                effect,
            } => {
                self.reviewer_adapter_tasks_in_flight.remove(&index);
                let mut next_reviewer_task = None;
                match result {
                    Ok(adapter) => {
                        if let Some(snapshot_slot) = self.reviewer_snapshots.get_mut(index) {
                            *snapshot_slot = Some(adapter.snapshot().clone());
                        }
                        if let Some(slot) = self.reviewer_adapters.get_mut(index) {
                            *slot = Some(adapter);
                        }
                        match effect {
                            ReviewerAdapterTaskEffect::None => {}
                            ReviewerAdapterTaskEffect::Reloaded => {
                                self.push_toast(
                                    i18n::text(self.app_language, "Reviewer reloaded"),
                                    theme::success(),
                                );
                            }
                            ReviewerAdapterTaskEffect::SyncDiffScroll => {
                                self.queue_reviewer_diff_scroll_sync();
                            }
                        }
                    }
                    Err(error) => {
                        self.push_toast(
                            i18n::text_with_arg(
                                self.app_language,
                                "Reviewer action failed: {error}",
                                "{error}",
                                error,
                            ),
                            theme::danger(),
                        );
                        self.pending_reviewer_loads.insert(index);
                    }
                }
                if let Some((task, effect)) = self.pop_pending_reviewer_adapter_task(index) {
                    next_reviewer_task = Some((index, task, effect));
                }
                if let Some((index, task, effect)) = next_reviewer_task {
                    self.spawn_indexed_reviewer_adapter_task(ctx, index, task, effect);
                } else if let Some((row_budget, load_more)) =
                    self.pending_reviewer_git_data_budget.remove(&index)
                {
                    self.spawn_indexed_reviewer_git_data_task(ctx, index, row_budget, load_more);
                }
            }
            AppEvent::ReviewerGitDataLoaded { index, result } => {
                self.reviewer_git_data_in_flight.remove(&index);
                match result {
                    Ok(result) => {
                        if let Some(adapter) = self
                            .reviewer_adapters
                            .get_mut(index)
                            .and_then(Option::as_mut)
                        {
                            adapter.apply_git_data_result(result);
                            if let Some(snapshot_slot) = self.reviewer_snapshots.get_mut(index) {
                                *snapshot_slot = Some(adapter.snapshot().clone());
                            }
                        }
                    }
                    Err(error) => {
                        self.push_toast(
                            i18n::text_with_arg(
                                self.app_language,
                                "Reviewer git load failed: {error}",
                                "{error}",
                                error,
                            ),
                            theme::danger(),
                        );
                    }
                }
                if let Some((row_budget, load_more)) =
                    self.pending_reviewer_git_data_budget.remove(&index)
                {
                    self.spawn_indexed_reviewer_git_data_task(ctx, index, row_budget, load_more);
                }
            }
            AppEvent::CodexAuthFinished { result } => {
                self.codex_auth.in_flight = false;
                self.codex_auth.auth_url = None;
                self.codex_auth.started_at = None;
                self.raise_window_for_codex_auth_result(ctx);
                match result {
                    Ok(info) => {
                        self.codex_auth.info = Some(info);
                        self.codex_auth.error = None;
                        self.push_toast(
                            i18n::text(self.app_language, "Codex authorized"),
                            theme::success(),
                        );
                    }
                    Err(error) => {
                        self.codex_auth.error = Some(error);
                        self.push_toast(
                            i18n::text(self.app_language, "Codex auth failed"),
                            theme::danger(),
                        );
                    }
                }
            }
            AppEvent::FsWatch(event) => {
                self.apply_fs_watch_event(event);
            }
            AppEvent::Notification(line) => {
                self.push_notification_line(line);
            }
            AppEvent::InputUiCommand(command) => {
                self.dispatch_ui_command(ctx, command);
            }
            AppEvent::InputReviewerDiffAction(action) => {
                self.dispatch_reviewer_diff_action(ctx, action);
            }
            AppEvent::InputTerminalBytes {
                workspace_index,
                target,
                agent_slot,
                bytes,
            } => {
                self.apply_input_terminal_bytes(ctx, workspace_index, target, agent_slot, bytes);
            }
            AppEvent::PomodoroInputDetected => {
                self.process_pomodoro_input_detected(ctx);
            }
            AppEvent::ProcessPendingSettingsSideEffects => {
                self.process_pending_settings_side_effects(ctx);
            }
            AppEvent::ProcessPendingReviewerLoads => {
                self.process_pending_reviewer_loads(ctx);
            }
            AppEvent::ProcessPendingMarkdownReparse => {
                self.process_pending_markdown_reparse(ctx);
            }
            AppEvent::ProcessPendingMarkdownOutlineCollapse => {
                self.process_pending_markdown_outline_collapse(ctx);
            }
            AppEvent::ProcessPendingMemoSaves => {
                self.process_pending_memo_saves(ctx);
            }
            AppEvent::ProcessPendingInputRepaint => {
                self.process_pending_input_repaint(ctx);
            }
            AppEvent::HandleScreenshotRequestFile => {
                self.handle_screenshot_request_file(ctx);
            }
            AppEvent::ProcessFsWatchDirty => {
                self.process_fs_watch_dirty(ctx);
            }
            AppEvent::FinishPendingAgentThemeRestarts => {
                self.finish_pending_agent_theme_restarts(ctx);
            }
            AppEvent::SyncTerminalEventRepaintFlags => {
                self.sync_terminal_event_repaint_flags();
            }
            AppEvent::ProcessAgentBusyWatchdogs => {
                self.process_agent_busy_watchdogs(ctx);
            }
            AppEvent::ProcessPomodoroState => {
                self.process_pomodoro_state(ctx);
            }
            AppEvent::ProcessExtraTools => {
                self.process_extra_tools(ctx);
            }
        }
    }

    /// 将单个原始文件系统 watcher 事件转成粗粒度 dirty 状态。
    pub(super) fn apply_fs_watch_event(&mut self, event: notify::Result<notify::Event>) {
        let mut events = Vec::new();
        let mut watcher = match self.fs_watcher.try_lock() {
            Ok(watcher) => watcher,
            Err(std::sync::TryLockError::WouldBlock) => {
                self.queue_app_event(AppEvent::FsWatch(event));
                return;
            }
            Err(std::sync::TryLockError::Poisoned(_)) => return,
        };
        match event {
            Ok(event) => watcher.map_notify_event(event, &mut events),
            Err(error) => {
                let error = error.to_string();
                watcher.last_error = Some(error.clone());
                events.push(FsWatchAppEvent::WatcherError(error));
            }
        }
        for event in events {
            match event {
                FsWatchAppEvent::WorkspaceChanged { index, workflow } => {
                    self.fs_watch_dirty.mark_outline_dirty(index);
                    if workflow {
                        self.fs_watch_dirty.mark_workflow_dirty(index);
                    }
                    self.fs_watch_dirty.mark_reviewer_dirty(index);
                }
                FsWatchAppEvent::ReviewerScriptsChanged => {
                    self.fs_watch_dirty.mark_reviewer_scripts_dirty();
                }
                FsWatchAppEvent::AgentStatusChanged => {
                    self.fs_watch_dirty.mark_agent_status_dirty();
                }
                FsWatchAppEvent::WatcherError(error) => {
                    self.reviewer_scripts.last_error = Some(error);
                }
            }
        }
    }

    /// Merges background agent status data without replacing route UI state.
    pub(super) fn merge_agent_status_refresh(&mut self, refreshed: Vec<WorkspaceViewData>) {
        for refreshed_workspace in refreshed {
            let Some(workspace) = self.workspaces.iter_mut().find(|workspace| {
                workspace.path == refreshed_workspace.path
                    && workspace.agent_kind == refreshed_workspace.agent_kind
                    && workspace.agent_id == refreshed_workspace.agent_id
            }) else {
                continue;
            };
            workspace.activity = refreshed_workspace.activity;
            workspace.session_id = refreshed_workspace.session_id;
            workspace.subagents = refreshed_workspace.subagents;
        }
        for (index, workspace) in self.workspaces.iter().enumerate() {
            let slots = agent_slots_for_workspace(workspace);
            for slot_id in slots {
                let Some(slot_workspace) = self.agent_workspace_for_slot(index, &slot_id) else {
                    continue;
                };
                if let Some(hosts) = self.terminal_hosts.get_mut(index)
                    && let Some(agent) = hosts
                        .agents
                        .get_mut(&slot_id)
                        .and_then(|slot| slot.host.as_mut())
                {
                    agent.sync_workspace_metadata(&slot_workspace);
                }
            }
        }
    }

    /// Applies branch choice data returned by the reviewer background task.
    pub(super) fn apply_reviewer_branch_choices_result(
        &mut self,
        repo: ReviewerBranchTarget,
        result: Result<(String, Vec<BranchInfo>), String>,
    ) {
        match result {
            Ok((current, branches)) => {
                self.set_active_reviewer_dialog(Some(ReviewerDialog::BranchList {
                    repo,
                    current,
                    visible: (0..branches.len()).collect(),
                    branches,
                    selected: 0,
                    filter: String::new(),
                }));
            }
            Err(error) => {
                self.set_active_reviewer_dialog(Some(ReviewerDialog::Message {
                    title: "Branch".to_string(),
                    message: error,
                }));
            }
        }
    }

    /// Applies a finished reviewer branch switch and refreshed adapter.
    pub(super) fn apply_reviewer_branch_switch_result(
        &mut self,
        repo: ReviewerBranchTarget,
        target: String,
        result: Result<ReviewerAdapter, String>,
    ) {
        match result {
            Ok(adapter) => {
                if let Some(snapshot_slot) = self.reviewer_snapshots.get_mut(self.active_workspace)
                {
                    *snapshot_slot = Some(adapter.snapshot().clone());
                }
                if let Some(slot) = self.reviewer_adapters.get_mut(self.active_workspace) {
                    *slot = Some(adapter);
                }
                let message = i18n::text(self.app_language, "Switched {repo} to {target}")
                    .replace("{repo}", &repo.label)
                    .replace("{target}", &target);
                self.push_toast(message.clone(), theme::success());
                self.set_active_reviewer_dialog(Some(ReviewerDialog::Message {
                    title: "Branch".to_string(),
                    message,
                }));
            }
            Err(error) => self.set_active_reviewer_dialog(Some(ReviewerDialog::Message {
                title: i18n::text(self.app_language, "Branch switch failed").to_string(),
                message: error,
            })),
        }
    }

    /// 应用文件系统修改结果里轻量的 UI 状态部分。
    pub(super) fn apply_file_mutation_result(
        &mut self,
        ctx: &egui::Context,
        result: FileMutationResult,
    ) {
        match result {
            FileMutationResult::CreateMarkdown {
                index,
                target,
                result,
            } => match result {
                Ok(()) => {
                    self.active_workspace = index.min(self.workspaces.len().saturating_sub(1));
                    self.queue_app_event(AppEvent::SyncTerminalEventRepaintFlags);
                    self.spawn_outline_refresh_tasks(ctx, BTreeSet::from([index]));
                    self.open_file_now(target);
                    self.push_toast(
                        i18n::text(self.app_language, "Created markdown file"),
                        theme::success(),
                    );
                }
                Err(error) => self.set_active_app_dialog(Some(AppDialog::Message {
                    title: i18n::text(self.app_language, "Create Markdown Failed").to_string(),
                    message: error,
                })),
            },
            FileMutationResult::CreateFolder { index, result } => match result {
                Ok(()) => {
                    self.spawn_outline_refresh_tasks(ctx, BTreeSet::from([index]));
                    self.push_toast(
                        i18n::text(self.app_language, "Created folder"),
                        theme::success(),
                    );
                }
                Err(error) => self.set_active_app_dialog(Some(AppDialog::Message {
                    title: i18n::text(self.app_language, "Create Folder Failed").to_string(),
                    message: error,
                })),
            },
            FileMutationResult::Rename {
                index,
                old_relative,
                new_relative,
                result,
            } => match result {
                Ok(()) => {
                    if let Some(document) = self.documents.get_mut(index)
                        && document.path.as_ref() == Some(&old_relative)
                    {
                        document.path = Some(new_relative.clone());
                    }
                    if let Some(workspace) = self.workspaces.get_mut(index)
                        && workspace.selected_file.as_ref() == Some(&old_relative)
                    {
                        workspace.selected_file = Some(new_relative.clone());
                    }
                    self.rename_recent_markdown_path(index, &old_relative, &new_relative);
                    self.spawn_outline_refresh_tasks(ctx, BTreeSet::from([index]));
                    self.push_toast(
                        i18n::text(self.app_language, "Renamed item"),
                        theme::success(),
                    );
                }
                Err(error) => self.set_active_app_dialog(Some(AppDialog::Message {
                    title: i18n::text(self.app_language, "Rename Failed").to_string(),
                    message: error,
                })),
            },
            FileMutationResult::DeleteMarkdown {
                index,
                path,
                result,
            } => match result {
                Ok(()) => {
                    if let Some(document) = self.documents.get_mut(index)
                        && document.path.as_ref() == Some(&path)
                    {
                        *document = DocumentState::default();
                    }
                    if let Some(workspace) = self.workspaces.get_mut(index)
                        && workspace.selected_file.as_ref() == Some(&path)
                    {
                        workspace.selected_file = None;
                    }
                    self.remove_recent_markdown_path(index, &path);
                    self.spawn_outline_refresh_tasks(ctx, BTreeSet::from([index]));
                    self.push_toast(
                        i18n::text(self.app_language, "Deleted markdown file"),
                        theme::success(),
                    );
                }
                Err(error) => self.set_active_app_dialog(Some(AppDialog::Message {
                    title: i18n::text(self.app_language, "Delete Markdown Failed").to_string(),
                    message: error,
                })),
            },
        }
    }

    /// 写入 input runtime 解析出的 terminal 输入字节。
    pub(super) fn apply_input_terminal_bytes(
        &mut self,
        ctx: &egui::Context,
        workspace_index: usize,
        target: TerminalSurfaceKind,
        agent_slot: AgentSlotId,
        bytes: Vec<u8>,
    ) {
        if bytes.is_empty() || workspace_index >= self.terminal_hosts.len() {
            return;
        }
        if target == TerminalSurfaceKind::Agent && terminal_agent_input_submit_bytes(&bytes) {
            crate::gui::perf_log::count("app.input_terminal_submitted");
            self.clear_agent_input_translation_state();
        }
        if target != TerminalSurfaceKind::Helix {
            self.spawn_terminal_host_for_workspace(ctx, workspace_index, target);
        }
        let Some(hosts) = self.terminal_hosts.get_mut(workspace_index) else {
            return;
        };
        let host = match target {
            TerminalSurfaceKind::Agent => hosts
                .agents
                .get_mut(&agent_slot)
                .and_then(|slot| slot.host.as_mut()),
            TerminalSurfaceKind::Workspace => hosts.workspace.as_mut(),
            TerminalSurfaceKind::Helix => hosts.helix.as_mut(),
        };
        if let Some(host) = host {
            if target == TerminalSurfaceKind::Agent
                && bytes.contains(&0x1b)
                && std::env::var_os("GSDV_AGENT_ESC_DEBUG").is_some()
            {
                eprintln!(
                    "[gsdv][agent-esc][app-event-write] workspace={} slot={:?} target={:?} host={} bytes={:?}",
                    workspace_index,
                    agent_slot,
                    target,
                    host.id(),
                    bytes
                );
            }
            host.write_bytes(&bytes);
        }
    }
}

/// 返回 AppEvent 层 terminal runtime 事件的性能日志标签。
fn terminal_runtime_app_event_label(kind: TerminalRuntimeEventKind) -> &'static str {
    match kind {
        TerminalRuntimeEventKind::Output => "app.terminal_runtime.output",
        TerminalRuntimeEventKind::StateChanged => "app.terminal_runtime.state_changed",
        TerminalRuntimeEventKind::Repaint => "app.terminal_runtime.repaint",
    }
}

//! 通知抽屉、toast 和 reviewer script 输出入口。
//!
//! 本模块只管理通知相关 UI 状态；脚本执行输出通过 AppEvent::Notification
//! 回到唯一事件队列。

use super::*;

impl GsdvGuiApp {
    pub(super) fn push_toast(&mut self, message: impl Into<String>, color: Color32) {
        self.toasts.push(Toast {
            message: message.into(),
            color,
            created_at: Instant::now(),
        });
    }

    pub(super) fn open_notifications(&mut self) {
        if !self.notifications.open {
            self.notification_return_context = self.capture_notification_return_context();
        }
        self.notifications.open();
    }

    pub(super) fn toggle_notifications(&mut self) {
        if self.notifications.open {
            self.close_notifications_restoring_route();
        } else {
            self.open_notifications();
        }
    }

    pub(super) fn close_notifications_without_restore(&mut self) {
        self.notifications.close();
        self.notification_return_context = None;
    }

    pub(super) fn close_notifications_restoring_route(&mut self) {
        self.notifications.close();
        let Some(context) = self.notification_return_context.take() else {
            return;
        };
        self.restore_notification_return_context(context);
    }

    pub(super) fn capture_notification_return_context(&self) -> Option<NotificationReturnContext> {
        let workspace = self.current_workspace()?;
        Some(NotificationReturnContext {
            workspace_index: self.active_workspace,
            route: workspace.route,
            center_mode: workspace.center_mode,
            previous_center_mode: workspace.previous_center_mode,
            workspace_terminal_open: self.workspace_terminal_drawer_is_open(),
            reviewer_helix_open: self.reviewer_helix_drawer_is_open(),
        })
    }

    pub(super) fn restore_notification_return_context(
        &mut self,
        context: NotificationReturnContext,
    ) {
        if context.workspace_index >= self.workspaces.len() {
            return;
        }
        self.active_workspace = context.workspace_index;
        if let Some(workspace) = self.workspaces.get_mut(context.workspace_index) {
            workspace.route = context.route;
            workspace.center_mode = context.center_mode;
            workspace.previous_center_mode = context.previous_center_mode;
        }
        if context.workspace_index >= self.workspace_terminal_drawers.len() {
            self.workspace_terminal_drawers
                .resize(self.workspaces.len(), false);
        }
        if context.workspace_index >= self.reviewer_helix_drawers.len() {
            self.reviewer_helix_drawers
                .resize(self.workspaces.len(), false);
        }
        if let Some(open) = self
            .workspace_terminal_drawers
            .get_mut(context.workspace_index)
        {
            *open = context.workspace_terminal_open;
        }
        if let Some(open) = self.reviewer_helix_drawers.get_mut(context.workspace_index) {
            *open = context.reviewer_helix_open;
        }
        self.queue_app_event(AppEvent::SyncTerminalEventRepaintFlags);
        self.persist_workspaces();
    }

    pub(super) fn push_notification_line(&mut self, line: impl Into<String>) {
        self.notifications.push_line(line);
    }

    /// 保留番茄钟生命周期入口，但不再写入通知栏。
    pub(super) fn push_pomodoro_notification(&mut self, _message: impl AsRef<str>) {
        // 触发条件：番茄钟状态机会频繁经过休息和返回工作阶段。
        // 不能继续走通知栏：外置工具和 reviewer 输出需要保持可扫描。
        // 防止回归：哈基米提示刷屏挤掉脚本执行结果。
    }

    pub(super) fn run_reviewer_script(&mut self, request: ReviewerScriptRunRequest) {
        let Some(workspace) = self.current_workspace() else {
            self.push_toast(
                i18n::text(self.app_language, "No active workspace for reviewer script"),
                theme::warning(),
            );
            return;
        };
        let project_dir = workspace.path.clone();
        let ReviewerScriptRunRequest { script, target } = request;
        let repo_name = target.repo.unwrap_or_default();
        let script_label = script.label;
        let script_path = script.path;
        let target_label = target.label;
        let target_root = target.root;
        let network_settings = self.network_settings.clone();
        let tx = self.app_event_tx.clone();
        let repaint_ctx = self.app_repaint_ctx.clone();
        let repaint_after = self.max_repaint_interval();

        self.open_notifications();
        self.push_notification_line(reviewer_script_reason_line(
            &script_label,
            &target_label,
            &target_root,
        ));
        self.push_toast(
            i18n::text(self.app_language, "Started {script} for {target}")
                .replace("{script}", &script_label)
                .replace("{target}", &target_label),
            theme::success(),
        );

        thread::spawn(move || {
            run_reviewer_script_process(
                tx,
                repaint_ctx,
                repaint_after,
                script_label,
                script_path,
                target_label,
                target_root,
                project_dir,
                repo_name,
                network_settings,
            );
        });
    }

    pub(super) fn handle_reviewer_script_request(&mut self, request: ReviewerScriptRunRequest) {
        if reviewer_script_requires_confirm(&request.script) {
            self.set_active_reviewer_dialog(Some(ReviewerDialog::ScriptConfirm { request }));
        } else {
            self.run_reviewer_script(request);
        }
    }

    pub(super) fn toast_overlay(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.toasts
            .retain(|toast| now.duration_since(toast.created_at) < Duration::from_secs(4));
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new("toast-overlay".into())
            .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-24.0, -56.0))
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    for toast in self.toasts.iter().rev().take(4) {
                        toast_card(ui, toast);
                        ui.add_space(8.0);
                    }
                });
            });
    }
}

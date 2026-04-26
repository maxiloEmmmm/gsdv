//! 截图请求和截图保存入口。
//!
//! 本模块只处理截图路径、debug capture 命令和 egui screenshot 请求；
//! 图片写盘仍通过后台任务完成。

use super::*;

impl GsdvGuiApp {
    /// 处理 input runtime 投递的截图结果。
    pub(super) fn handle_screenshot_captured(
        &mut self,
        ctx: &egui::Context,
        path: Option<PathBuf>,
        image: Arc<egui::ColorImage>,
    ) {
        let path = path.unwrap_or_else(|| self.next_screenshot_path("event"));
        // 触发条件：用户在 Agent 等界面触发截图复制。
        // 不能只保存文件：系统剪贴板需要显式写入图片命令。
        // 防止快捷键截图后只能找到文件、无法直接粘贴图片。
        ctx.copy_image((*image).clone());
        self.spawn_screenshot_save_task(path, image);
    }

    pub(super) fn handle_screenshot_request_file(&mut self, ctx: &egui::Context) {
        if !self.screenshot_request_poll_enabled {
            return;
        }
        if self.screenshot_request_read_in_flight {
            return;
        }
        if self.last_screenshot_request_poll.elapsed() < SCREENSHOT_REQUEST_POLL_INTERVAL {
            return;
        }
        self.last_screenshot_request_poll = Instant::now();
        self.spawn_screenshot_request_load_task(ctx);
    }

    pub(super) fn apply_debug_capture_command(&mut self, command: &str) -> &'static str {
        if let Some(scroll_y) = preview_scroll_capture_offset(command) {
            self.exit_reviewer_route();
            self.set_center_mode(CenterMode::Preview);
            if let Some(document) = self.active_document_mut() {
                document.markdown_scroll_y = scroll_y;
            }
            return "preview-scroll-file";
        }

        match command {
            "agent" => {
                self.exit_reviewer_route();
                self.set_center_mode(CenterMode::Agent);
                "agent-file"
            }
            "terminal" => {
                self.exit_reviewer_route();
                if let Some(open) = self
                    .workspace_terminal_drawers
                    .get_mut(self.active_workspace)
                {
                    *open = true;
                }
                self.queue_app_event(AppEvent::SyncTerminalEventRepaintFlags);
                "terminal-file"
            }
            "editor" => {
                self.exit_reviewer_route();
                self.set_center_mode(CenterMode::Editor);
                "editor-file"
            }
            "preview" => {
                self.exit_reviewer_route();
                self.set_center_mode(CenterMode::Preview);
                "preview-file"
            }
            "reviewer" => {
                self.open_reviewer_route();
                "reviewer-file"
            }
            "workspace" => {
                self.exit_reviewer_route();
                "workspace-file"
            }
            _ => "file",
        }
    }

    pub(super) fn toggle_markdown_editor_preview(&mut self) {
        let Some(workspace) = self.current_workspace_mut() else {
            return;
        };
        if workspace.route != Route::Workspace {
            return;
        }
        workspace.center_mode = match workspace.center_mode {
            CenterMode::Editor => CenterMode::Preview,
            CenterMode::Preview => CenterMode::Editor,
            CenterMode::Agent | CenterMode::Terminal => return,
        };
        workspace.previous_center_mode = workspace.center_mode;
    }

    pub(super) fn request_egui_screenshot(&mut self, ctx: &egui::Context, source: &str) {
        let path = self.next_screenshot_path(source);
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(path)));
        self.request_app_repaint(ctx);
    }

    pub(super) fn next_screenshot_path(&mut self, source: &str) -> PathBuf {
        self.screenshot_sequence = self.screenshot_sequence.saturating_add(1);
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default();
        let mode = self
            .current_workspace()
            .map(|workspace| match workspace.route {
                Route::Reviewer => "reviewer",
                Route::Workspace => match workspace.center_mode {
                    CenterMode::Agent => "agent",
                    CenterMode::Terminal => "terminal",
                    CenterMode::Editor => "editor",
                    CenterMode::Preview => "preview",
                },
            })
            .unwrap_or("empty");
        screenshot_dir().join(format!(
            "gsdv-{millis}-{mode}-{source}-{}.png",
            self.screenshot_sequence
        ))
    }
}

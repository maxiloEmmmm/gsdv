//! 文件系统 watcher 和防抖派发。
//!
//! watcher 回调只能投递粗粒度事件；本模块负责把文件变化合并成
//! outline、reviewer、agent status 等后续后台任务。

use super::*;

impl GsdvGuiApp {
    /// Connects filesystem watcher callbacks to egui repaint wakeups.
    pub(super) fn set_fs_watch_repaint_context(&mut self, ctx: egui::Context) {
        if let Ok(mut watcher) = self.fs_watcher.lock() {
            watcher.set_repaint_context(ctx);
        }
    }

    /// 返回文件系统 dirty 防抖的最近截止时间。
    pub(super) fn next_fs_watch_dirty_delay(&self) -> Option<Duration> {
        let mut next = None;
        if let Some(dirty_at) = self.fs_watch_dirty.outline_dirty_at {
            next =
                min_optional_duration(next, Some(duration_until_due(dirty_at, FS_WATCH_DEBOUNCE)));
        }
        if let Some(dirty_at) = self.fs_watch_dirty.reviewer_dirty_at {
            next =
                min_optional_duration(next, Some(duration_until_due(dirty_at, FS_WATCH_DEBOUNCE)));
        }
        if let Some(dirty_at) = self.fs_watch_dirty.reviewer_scripts_dirty_at {
            next =
                min_optional_duration(next, Some(duration_until_due(dirty_at, FS_WATCH_DEBOUNCE)));
        }
        if let Some(dirty_at) = self.fs_watch_dirty.agent_status_dirty_at {
            next =
                min_optional_duration(next, Some(duration_until_due(dirty_at, FS_WATCH_DEBOUNCE)));
        }
        next
    }

    /// 同步全局文件系统 watcher 注册的所有路径。
    pub(super) fn sync_fs_watches(&mut self) {
        let watcher = Arc::clone(&self.fs_watcher);
        let workspace_paths = self
            .workspaces
            .iter()
            .map(|workspace| workspace.path.clone())
            .collect::<Vec<_>>();
        self.fs_watch_dirty
            .clamp_workspace_indexes(self.workspaces.len());
        self.background_runtime.spawn(async move {
            let _ = tokio::task::spawn_blocking(move || {
                if let Ok(mut watcher) = watcher.lock() {
                    watcher.sync_workspace_roots(&workspace_paths);
                    watcher.sync_global_paths();
                }
            })
            .await;
        });
    }

    /// 派发文件系统 watcher 防抖后的后台工作。
    pub(super) fn process_fs_watch_dirty(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if self
            .fs_watch_dirty
            .agent_status_dirty_at
            .is_some_and(|dirty_at| now.duration_since(dirty_at) >= FS_WATCH_DEBOUNCE)
        {
            self.fs_watch_dirty.agent_status = false;
            self.fs_watch_dirty.agent_status_dirty_at = None;
            self.spawn_agent_status_refresh_task(ctx);
        }
        if self
            .fs_watch_dirty
            .outline_dirty_at
            .is_some_and(|dirty_at| now.duration_since(dirty_at) >= FS_WATCH_DEBOUNCE)
        {
            let dirty_workspaces = std::mem::take(&mut self.fs_watch_dirty.outline_workspaces);
            let workflow_dirty_workspaces = dirty_workspaces.clone();
            self.fs_watch_dirty.outline_dirty_at = None;
            self.reload_clean_selected_documents(ctx, &dirty_workspaces);
            self.spawn_outline_refresh_tasks(ctx, dirty_workspaces);
            for index in workflow_dirty_workspaces {
                let workflow_visible = self
                    .outline_panel_tabs
                    .get(index)
                    .is_some_and(|tab| *tab == OutlinePanelTab::Workflow);
                if workflow_visible {
                    self.request_workflow_tree_refresh(ctx, index);
                }
            }
        }
        if self
            .fs_watch_dirty
            .reviewer_dirty_at
            .is_some_and(|dirty_at| now.duration_since(dirty_at) >= FS_WATCH_DEBOUNCE)
        {
            let dirty_workspaces = std::mem::take(&mut self.fs_watch_dirty.reviewer_workspaces);
            self.fs_watch_dirty.reviewer_dirty_at = None;
            self.spawn_reviewer_uncommitted_refresh_tasks(ctx, dirty_workspaces);
        }
        if self
            .fs_watch_dirty
            .reviewer_scripts_dirty_at
            .is_some_and(|dirty_at| now.duration_since(dirty_at) >= FS_WATCH_DEBOUNCE)
        {
            self.fs_watch_dirty.reviewer_scripts = false;
            self.fs_watch_dirty.reviewer_scripts_dirty_at = None;
            self.spawn_reviewer_scripts_refresh_task(ctx);
        }
    }
}

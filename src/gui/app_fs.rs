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
        next
    }

    /// 同步全局文件系统 watcher 注册的所有路径。
    pub(super) fn sync_fs_watches(&mut self) {
        let watcher = Arc::clone(&self.fs_watcher);
        let workspace_specs = self.workspace_watch_specs();
        self.fs_watch_dirty
            .clamp_workspace_indexes(self.workspaces.len());
        self.background_runtime.spawn(async move {
            let _ = tokio::task::spawn_blocking(move || {
                if let Ok(mut watcher) = watcher.lock() {
                    watcher.sync_workspace_watches(&workspace_specs);
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
            .outline_dirty_at
            .is_some_and(|dirty_at| now.duration_since(dirty_at) >= FS_WATCH_DEBOUNCE)
        {
            let dirty_workspaces = std::mem::take(&mut self.fs_watch_dirty.outline_workspaces);
            let workflow_dirty_workspaces =
                std::mem::take(&mut self.fs_watch_dirty.workflow_workspaces);
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

    /// Builds the currently opened filesystem watch scope for all workspaces.
    ///
    /// Example: a collapsed outline dir is omitted until the user expands it.
    fn workspace_watch_specs(&self) -> Vec<WorkspaceWatchSpec> {
        self.workspaces
            .iter()
            .enumerate()
            .map(|(index, workspace)| WorkspaceWatchSpec {
                root: workspace.path.clone(),
                opened_dirs: self.opened_workspace_watch_dirs(index, workspace),
            })
            .collect()
    }

    /// Returns directories that are currently opened in one workspace.
    ///
    /// Example: selected `docs/a.md` -> watch the workspace `docs` dir.
    fn opened_workspace_watch_dirs(
        &self,
        index: usize,
        workspace: &WorkspaceViewData,
    ) -> BTreeSet<PathBuf> {
        let mut dirs = BTreeSet::new();
        dirs.insert(workspace.path.clone());
        if let Some(selected) = workspace.selected_file.as_deref() {
            add_relative_file_parent(&mut dirs, &workspace.path, selected);
        }
        if let Some(document_path) = self
            .documents
            .get(index)
            .and_then(|document| document.path.as_deref())
        {
            add_relative_file_parent(&mut dirs, &workspace.path, document_path);
        }
        if let Some(state) = self.workflow_states.get(index) {
            add_workflow_watch_dirs(&mut dirs, &workspace.path, state);
        }
        collect_open_outline_watch_dirs(&workspace.path, &workspace.outline, &mut dirs);
        dirs
    }
}

/// Adds the parent directory for a workspace-relative or absolute file path.
///
/// Example: `src/lib.rs` -> inserts `<workspace>/src`.
fn add_relative_file_parent(dirs: &mut BTreeSet<PathBuf>, workspace_root: &Path, path: &Path) {
    let absolute = resolve_workspace_path(workspace_root, path);
    if let Some(parent) = absolute.parent() {
        dirs.insert(parent.to_path_buf());
    }
}

/// Adds watch dirs for the currently opened workflow target.
///
/// Example: selected task `gsdv-spec/ps/a/task-x.md` -> watch that task dir.
fn add_workflow_watch_dirs(
    dirs: &mut BTreeSet<PathBuf>,
    workspace_root: &Path,
    state: &WorkflowUiState,
) {
    if let Some(tree) = state.tree.as_ref() {
        dirs.insert(resolve_workspace_path(workspace_root, &tree.spec_path));
    }
    if let Some(target) = state.selected.as_ref() {
        add_relative_file_parent(dirs, workspace_root, workflow_target_path(target));
    }
    if let Some(target) = state.last_task_surface_target.as_ref() {
        add_relative_file_parent(dirs, workspace_root, workflow_target_path(target));
    }
    if let Some(editor) = state.task_editor.as_ref() {
        add_relative_file_parent(dirs, workspace_root, &editor.task_path);
    }
    if let Some(editor) = state.editor.as_ref() {
        add_relative_file_parent(dirs, workspace_root, &editor.task_path);
    }
}

/// Returns the Markdown file path represented by a workflow selection target.
///
/// Example: `Step { task_path }` -> returns that task file.
fn workflow_target_path(target: &WorkflowSelectionTarget) -> &Path {
    match target {
        WorkflowSelectionTarget::WorkspaceRoot { root_path }
        | WorkflowSelectionTarget::Project { root_path } => root_path.as_path(),
        WorkflowSelectionTarget::Task { task_path }
        | WorkflowSelectionTarget::Step { task_path, .. } => task_path.as_path(),
    }
}

/// Collects expanded outline directories as opened watch dirs.
///
/// Example: expanded workspace `src` -> inserts `<workspace>/src`.
fn collect_open_outline_watch_dirs(
    workspace_root: &Path,
    nodes: &[OutlineNode],
    dirs: &mut BTreeSet<PathBuf>,
) {
    for node in nodes {
        match node {
            OutlineNode::Root {
                root_kind,
                key,
                expanded,
                children,
                ..
            } => {
                if !*expanded {
                    continue;
                }
                if let Some(dir) = outline_key_watch_dir(workspace_root, root_kind, key) {
                    dirs.insert(dir);
                }
                collect_open_outline_child_dirs(workspace_root, root_kind, children, dirs);
            }
            OutlineNode::Dir { .. } | OutlineNode::File { .. } => {}
        }
    }
}

/// Collects expanded child outline directories under a known root kind.
///
/// Example: expanded home `~/.codex` -> inserts `<home>/.codex`.
fn collect_open_outline_child_dirs(
    workspace_root: &Path,
    root_kind: &data::OutlineRootKind,
    nodes: &[OutlineNode],
    dirs: &mut BTreeSet<PathBuf>,
) {
    for node in nodes {
        match node {
            OutlineNode::Dir {
                key,
                expanded,
                children,
                ..
            } => {
                if !*expanded {
                    continue;
                }
                if let Some(dir) = outline_key_watch_dir(workspace_root, root_kind, key) {
                    dirs.insert(dir);
                }
                collect_open_outline_child_dirs(workspace_root, root_kind, children, dirs);
            }
            OutlineNode::Root { .. } | OutlineNode::File { .. } => {}
        }
    }
}

/// Resolves an outline node key into an absolute directory path.
///
/// Example: workspace key `docs` -> `<workspace>/docs`.
fn outline_key_watch_dir(
    workspace_root: &Path,
    root_kind: &data::OutlineRootKind,
    key: &Path,
) -> Option<PathBuf> {
    match root_kind {
        data::OutlineRootKind::Workspace => Some(resolve_workspace_path(workspace_root, key)),
        data::OutlineRootKind::Home => home_outline_key_watch_dir(key),
        data::OutlineRootKind::Attached => Some(key.to_path_buf()),
    }
}

/// Resolves a `~` outline key into an absolute home path.
///
/// Example: `~/.codex` -> `<home>/.codex`.
fn home_outline_key_watch_dir(key: &Path) -> Option<PathBuf> {
    let home = crate::home::home_dir()?;
    if key == Path::new("~") {
        return Some(home);
    }
    key.strip_prefix("~").ok().map(|suffix| home.join(suffix))
}

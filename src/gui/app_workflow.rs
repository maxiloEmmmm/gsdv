//! workflow tree 和片段编辑业务。

use super::*;

impl GsdvGuiApp {
    /// 切换左侧面板 tab，并在进入 workflow 时派发加载任务。
    pub(super) fn set_outline_panel_tab(&mut self, ctx: &egui::Context, tab: OutlinePanelTab) {
        let Some(current_tab) = self.outline_panel_tabs.get_mut(self.active_workspace) else {
            return;
        };
        if *current_tab == tab {
            return;
        }
        *current_tab = tab;
        if tab == OutlinePanelTab::Workflow {
            self.request_workflow_tree_refresh(ctx, self.active_workspace);
        }
        self.request_app_repaint(ctx);
    }

    /// 当前 workspace 是否正在显示 workflow step 片段编辑器。
    pub(super) fn workflow_step_editor_visible(&self) -> bool {
        self.outline_panel_tabs
            .get(self.active_workspace)
            .is_some_and(|tab| *tab == OutlinePanelTab::Workflow)
            && self
                .workflow_states
                .get(self.active_workspace)
                .and_then(|state| state.editor.as_ref())
                .is_some()
    }

    /// 当前 workspace 是否正在显示 workflow task 工作台。
    pub(super) fn workflow_task_surface_visible(&self) -> bool {
        self.outline_panel_tabs
            .get(self.active_workspace)
            .is_some_and(|tab| *tab == OutlinePanelTab::Workflow)
            && self
                .workflow_states
                .get(self.active_workspace)
                .and_then(|state| state.selected.as_ref())
                .is_some_and(|target| {
                    matches!(
                        target,
                        WorkflowSelectionTarget::Task { .. } | WorkflowSelectionTarget::Step { .. }
                    )
                })
    }

    /// 当前 workflow step 片段是否有未保存修改。
    pub(super) fn workflow_step_editor_dirty(&self) -> bool {
        self.workflow_states
            .get(self.active_workspace)
            .is_some_and(|state| {
                state
                    .task_editor
                    .as_ref()
                    .is_some_and(WorkflowTaskEditor::is_dirty)
                    || state
                        .editor
                        .as_ref()
                        .is_some_and(WorkflowStepEditor::is_dirty)
            })
    }

    /// 复制当前选中的 workflow 逻辑路径。
    pub(super) fn copy_selected_workflow_path(&mut self, ctx: &egui::Context) {
        let Some(path) = self.selected_workflow_copy_path() else {
            return;
        };
        ctx.copy_text(path);
        self.push_toast(
            i18n::text(self.app_language, "Workflow path copied"),
            theme::success(),
        );
    }

    /// 返回当前选中 workflow 节点的逻辑路径。
    fn selected_workflow_copy_path(&self) -> Option<String> {
        if self
            .outline_panel_tabs
            .get(self.active_workspace)
            .is_none_or(|tab| *tab != OutlinePanelTab::Workflow)
        {
            return None;
        }
        let state = self.workflow_states.get(self.active_workspace)?;
        let selected = state.selected.as_ref()?;
        let tree = state.tree.as_ref()?;
        match selected {
            WorkflowSelectionTarget::Project { root_path } => tree
                .projects
                .iter()
                .find(|project| project.root_path == *root_path)
                .map(|project| project.label.clone()),
            WorkflowSelectionTarget::Task { task_path } => {
                workflow_task_copy_path_from_tree(tree, task_path)
            }
            WorkflowSelectionTarget::Step {
                task_path,
                step_path,
            } => workflow_step_copy_path_from_tree(tree, task_path, step_path),
        }
    }

    /// 请求刷新指定 workspace 的 workflow tree。
    pub(super) fn request_workflow_tree_refresh(&mut self, ctx: &egui::Context, index: usize) {
        let Some(state) = self.workflow_states.get_mut(index) else {
            return;
        };
        if state.loading {
            return;
        }
        let Some(workspace) = self.workspaces.get(index) else {
            return;
        };
        state.loading = true;
        state.load_error = None;
        self.spawn_workflow_tree_load_task(ctx, index, workspace.path.clone());
    }

    /// 后台加载 workflow tree。
    fn spawn_workflow_tree_load_task(
        &self,
        ctx: &egui::Context,
        index: usize,
        workspace_path: PathBuf,
    ) {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_after = self.max_repaint_interval();
        self.background_runtime.spawn(async move {
            let event_workspace_path = workspace_path.clone();
            let result = tokio::task::spawn_blocking(move || {
                crate::gui::workflow::load_workflow_tree(&workspace_path)
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(AppEvent::WorkflowTreeLoaded {
                index,
                workspace_path: event_workspace_path,
                result,
            });
            repaint_ctx.request_repaint_after(repaint_after);
        });
    }

    /// 应用 workflow tree 加载结果。
    pub(super) fn apply_workflow_tree_loaded(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        workspace_path: PathBuf,
        result: Result<WorkflowTree, String>,
    ) {
        let Some(workspace) = self.workspaces.get(index) else {
            return;
        };
        if workspace.path != workspace_path {
            return;
        }
        let Some(state) = self.workflow_states.get_mut(index) else {
            return;
        };
        state.loading = false;
        let mut pending_target = None;
        match result {
            Ok(tree) => {
                let project_keys: BTreeSet<String> = tree
                    .projects
                    .iter()
                    .map(|project| project.key.clone())
                    .collect();
                state
                    .collapsed_project_keys
                    .retain(|key| project_keys.contains(key));
                state.tree = Some(tree);
                state.load_error = None;
                pending_target = state.pending_target_after_save.take();
            }
            Err(error) => {
                state.tree = None;
                state.load_error = Some(error);
            }
        }
        if let Some(target) = pending_target {
            self.open_workflow_target_now(ctx, target);
        }
    }

    /// 请求打开 workflow 目标，必要时先弹出未保存片段确认。
    pub(super) fn request_workflow_target(
        &mut self,
        ctx: &egui::Context,
        target: WorkflowSelectionTarget,
    ) {
        if self.workflow_switch_requires_save(&target) {
            self.set_active_app_dialog(Some(AppDialog::WorkflowUnsavedSwitch { target }));
            return;
        }
        self.open_workflow_target_now(ctx, target);
    }

    /// 不再询问未保存状态，直接打开 workflow 目标。
    pub(super) fn open_workflow_target_now(
        &mut self,
        ctx: &egui::Context,
        target: WorkflowSelectionTarget,
    ) {
        match target.clone() {
            WorkflowSelectionTarget::Project { root_path } => {
                if let Some(state) = self.workflow_states.get_mut(self.active_workspace) {
                    state.selected = Some(target);
                    state.task_editor = None;
                    state.editor = None;
                }
                self.request_open_file(root_path);
                if let Some(workspace) = self.current_workspace_mut() {
                    workspace.center_mode = CenterMode::Editor;
                }
                self.persist_workspaces();
            }
            WorkflowSelectionTarget::Task { task_path } => {
                let task_editor = self.reusable_or_fresh_workflow_task_editor(&task_path);
                if let Some(state) = self.workflow_states.get_mut(self.active_workspace) {
                    state.selected = Some(target);
                    state.task_editor = task_editor;
                    state.editor = None;
                }
                if let Some(workspace) = self.current_workspace_mut() {
                    workspace.center_mode = CenterMode::Editor;
                }
                self.request_app_repaint(ctx);
            }
            WorkflowSelectionTarget::Step {
                task_path,
                step_path,
            } => {
                let task_editor = self.reusable_or_fresh_workflow_task_editor(&task_path);
                let Some(editor) = self.workflow_editor_for_step(&task_path, &step_path) else {
                    self.push_toast(
                        i18n::text(self.app_language, "Workflow step not found"),
                        theme::warning(),
                    );
                    return;
                };
                if let Some(state) = self.workflow_states.get_mut(self.active_workspace) {
                    state.selected = Some(target);
                    state.task_editor = task_editor;
                    state.editor = Some(editor);
                }
                if let Some(workspace) = self.current_workspace_mut() {
                    workspace.center_mode = CenterMode::Editor;
                }
                self.request_app_repaint(ctx);
            }
        }
    }

    /// 判断切换目标是否会丢弃当前 workflow 未保存内容。
    fn workflow_switch_requires_save(&self, target: &WorkflowSelectionTarget) -> bool {
        let Some(state) = self.workflow_states.get(self.active_workspace) else {
            return false;
        };
        if let Some(task_editor) = state.task_editor.as_ref()
            && task_editor.is_dirty()
            && workflow_target_task_path(target) != Some(task_editor.task_path.as_path())
        {
            return true;
        }
        if let Some(editor) = state.editor.as_ref()
            && editor.is_dirty()
            && editor.target != *target
        {
            return true;
        }
        false
    }

    /// 优先复用同 task 的未保存编辑器，否则从当前 tree 重建。
    fn reusable_or_fresh_workflow_task_editor(
        &self,
        task_path: &Path,
    ) -> Option<WorkflowTaskEditor> {
        self.workflow_states
            .get(self.active_workspace)
            .and_then(|state| state.task_editor.as_ref())
            .filter(|editor| editor.task_path == task_path)
            .cloned()
            .or_else(|| self.workflow_editor_for_task(task_path))
    }

    /// 从已加载 tree 中查找 task 并构建说明编辑器。
    fn workflow_editor_for_task(&self, task_path: &Path) -> Option<WorkflowTaskEditor> {
        let state = self.workflow_states.get(self.active_workspace)?;
        let tree = state.tree.as_ref()?;
        tree.projects
            .iter()
            .flat_map(|project| project.tasks.iter())
            .find(|task| task.path == task_path)
            .map(workflow_task_editor_from_node)
    }

    /// 从已加载 tree 中查找 step 并构建编辑器。
    fn workflow_editor_for_step(
        &self,
        task_path: &Path,
        step_path: &[usize],
    ) -> Option<WorkflowStepEditor> {
        let state = self.workflow_states.get(self.active_workspace)?;
        let tree = state.tree.as_ref()?;
        for project in &tree.projects {
            for task in &project.tasks {
                if task.path == task_path
                    && let Some(node) = workflow_step_node_at_path(&task.steps, step_path)
                {
                    return Some(workflow_step_editor_from_node(task_path, node));
                }
            }
        }
        None
    }

    /// 保存当前 workflow step 片段。
    pub(super) fn save_active_workflow_step(
        &mut self,
        ctx: &egui::Context,
        pending_target: Option<WorkflowSelectionTarget>,
    ) {
        let active_workspace = self.active_workspace;
        let Some(workspace) = self.workspaces.get(active_workspace) else {
            return;
        };
        let Some(state) = self.workflow_states.get_mut(active_workspace) else {
            return;
        };
        let selected = state.selected.clone();
        let Some(task_editor) = state.task_editor.as_mut() else {
            return;
        };
        task_editor.save_error = None;
        if let Some(editor) = state.editor.as_mut() {
            editor.save_error = None;
        }
        state.pending_target_after_save = pending_target;
        let step_path = state.editor.as_ref().map(|editor| editor.step_path.clone());
        let step_text = state.editor.as_ref().map(|editor| editor.step_text.clone());
        let request = WorkflowSaveRequest {
            task_path: task_editor.task_path.clone(),
            task_text: task_editor.task_text.clone(),
            step_path,
            step_text,
        };
        self.spawn_workflow_step_save_task(
            ctx,
            active_workspace,
            workspace.path.clone(),
            selected,
            request,
        );
    }

    /// 后台保存 workflow step 片段。
    fn spawn_workflow_step_save_task(
        &self,
        ctx: &egui::Context,
        index: usize,
        workspace_path: PathBuf,
        selected: Option<WorkflowSelectionTarget>,
        request: WorkflowSaveRequest,
    ) {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_after = self.max_repaint_interval();
        let target = selected.unwrap_or_else(|| WorkflowSelectionTarget::Task {
            task_path: request.task_path.clone(),
        });
        self.background_runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                crate::gui::workflow::save_workflow_step_editor(&workspace_path, request)
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(AppEvent::WorkflowStepSaved {
                index,
                target,
                result,
            });
            repaint_ctx.request_repaint_after(repaint_after);
        });
    }

    /// 应用 workflow 片段保存结果。
    pub(super) fn apply_workflow_step_saved(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        target: WorkflowSelectionTarget,
        result: Result<WorkflowSaveSuccess, String>,
    ) {
        let Some(state) = self.workflow_states.get_mut(index) else {
            return;
        };
        if state.selected.as_ref() != Some(&target) {
            return;
        }
        let Some(task_editor) = state.task_editor.as_mut() else {
            return;
        };
        match result {
            Ok(saved) => {
                task_editor.saved_task_text = saved.task_text;
                task_editor.save_error = None;
                if let Some(editor) = state.editor.as_mut()
                    && let Some(step_text) = saved.step_text
                {
                    editor.saved_step_text = step_text;
                    editor.save_error = None;
                }
                self.push_toast(
                    i18n::text(self.app_language, "Workflow saved"),
                    theme::success(),
                );
                self.request_workflow_tree_refresh(ctx, index);
            }
            Err(error) => {
                task_editor.save_error = Some(error.clone());
                if let Some(editor) = state.editor.as_mut() {
                    editor.save_error = Some(error);
                }
                self.push_toast(
                    i18n::text(self.app_language, "Workflow save failed"),
                    theme::danger(),
                );
            }
        }
    }

    /// 派发 workflow tree 右键菜单文件修改任务。
    pub(super) fn request_workflow_mutation(
        &mut self,
        ctx: &egui::Context,
        request: WorkflowMutationRequest,
    ) {
        let index = self.active_workspace;
        let Some(workspace) = self.workspaces.get(index) else {
            return;
        };
        self.spawn_workflow_mutation_task(ctx, index, workspace.path.clone(), request);
    }

    /// 后台执行 workflow tree 文件修改。
    fn spawn_workflow_mutation_task(
        &self,
        ctx: &egui::Context,
        index: usize,
        workspace_path: PathBuf,
        request: WorkflowMutationRequest,
    ) {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_after = self.max_repaint_interval();
        self.background_runtime.spawn(async move {
            let request_for_task = request.clone();
            let result = tokio::task::spawn_blocking(move || {
                crate::gui::workflow::apply_workflow_mutation(&workspace_path, request_for_task)
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(AppEvent::WorkflowMutationFinished {
                index,
                request,
                result,
            });
            repaint_ctx.request_repaint_after(repaint_after);
        });
    }

    /// 应用 workflow tree 文件修改完成后的 UI 状态变化。
    pub(super) fn apply_workflow_mutation_finished(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        request: WorkflowMutationRequest,
        result: Result<(), String>,
    ) {
        match result {
            Ok(()) => {
                self.cleanup_after_workflow_mutation(index, &request);
                self.spawn_outline_refresh_tasks(ctx, BTreeSet::from([index]));
                self.request_workflow_tree_refresh(ctx, index);
                self.push_toast(
                    i18n::text(self.app_language, "Workflow updated"),
                    theme::success(),
                );
            }
            Err(error) => {
                self.set_active_app_dialog(Some(AppDialog::Message {
                    title: i18n::text(self.app_language, "Workflow Update Failed").to_string(),
                    message: error,
                }));
            }
        }
    }

    /// 清理 workflow 修改后已经失效的当前选择和文档状态。
    fn cleanup_after_workflow_mutation(&mut self, index: usize, request: &WorkflowMutationRequest) {
        match request {
            WorkflowMutationRequest::RenameProject {
                project_key,
                new_key,
            } => {
                let old_prefix = PathBuf::from("gsdv-spec").join("ps").join(project_key);
                let new_prefix = PathBuf::from("gsdv-spec").join("ps").join(new_key);
                self.rename_workflow_document_prefix(index, &old_prefix, &new_prefix);
                if let Some(state) = self.workflow_states.get_mut(index)
                    && state.collapsed_project_keys.remove(project_key)
                {
                    state.collapsed_project_keys.insert(new_key.clone());
                }
            }
            WorkflowMutationRequest::RenameTask { task_path, new_key } => {
                let next_path = workflow_task_path_after_rename(task_path, new_key);
                self.rename_workflow_document_path(index, task_path, &next_path);
            }
            WorkflowMutationRequest::RenameStep {
                task_path,
                step_path,
                ..
            } => {
                self.clear_workflow_editor_for_step_subtree(index, task_path, step_path);
            }
            WorkflowMutationRequest::DeleteProject { project_key } => {
                let prefix = PathBuf::from("gsdv-spec").join("ps").join(project_key);
                self.clear_workflow_document_under_prefix(index, &prefix);
            }
            WorkflowMutationRequest::DeleteTask { task_path } => {
                self.clear_workflow_document_path(index, task_path);
            }
            WorkflowMutationRequest::DeleteStep {
                task_path,
                step_path,
            } => {
                self.clear_workflow_editor_for_step_subtree(index, task_path, step_path);
            }
            WorkflowMutationRequest::InitRoot
            | WorkflowMutationRequest::AddTask { .. }
            | WorkflowMutationRequest::AddStep { .. } => {}
        }
    }

    /// 重写指向已重命名目录前缀的文档和 workflow 状态路径。
    fn rename_workflow_document_prefix(
        &mut self,
        index: usize,
        old_prefix: &Path,
        new_prefix: &Path,
    ) {
        if let Some(document) = self.documents.get_mut(index) {
            rewrite_optional_path_prefix(&mut document.path, old_prefix, new_prefix);
            rewrite_optional_path_prefix(&mut document.loading_path, old_prefix, new_prefix);
        }
        if let Some(workspace) = self.workspaces.get_mut(index) {
            rewrite_optional_path_prefix(&mut workspace.selected_file, old_prefix, new_prefix);
        }
        if let Some(state) = self.workflow_states.get_mut(index) {
            rewrite_workflow_selection_prefix(&mut state.selected, old_prefix, new_prefix);
            if let Some(editor) = state.task_editor.as_mut()
                && let Some(next_task_path) =
                    rewritten_path_prefix(&editor.task_path, old_prefix, new_prefix)
            {
                editor.task_path = next_task_path;
            }
            if let Some(editor) = state.editor.as_mut()
                && let Some(next_task_path) =
                    rewritten_path_prefix(&editor.task_path, old_prefix, new_prefix)
            {
                editor.task_path = next_task_path.clone();
                editor.target = WorkflowSelectionTarget::Step {
                    task_path: next_task_path,
                    step_path: editor.step_path.clone(),
                };
            }
        }
    }

    /// 重写指向已重命名 task 文件的文档和 workflow 状态路径。
    fn rename_workflow_document_path(&mut self, index: usize, old_path: &Path, new_path: &Path) {
        if let Some(document) = self.documents.get_mut(index) {
            rewrite_optional_exact_path(&mut document.path, old_path, new_path);
            rewrite_optional_exact_path(&mut document.loading_path, old_path, new_path);
        }
        if let Some(workspace) = self.workspaces.get_mut(index) {
            rewrite_optional_exact_path(&mut workspace.selected_file, old_path, new_path);
        }
        if let Some(state) = self.workflow_states.get_mut(index) {
            rewrite_workflow_selection_path(&mut state.selected, old_path, new_path);
            if let Some(editor) = state.task_editor.as_mut()
                && editor.task_path == old_path
            {
                editor.task_path = new_path.to_path_buf();
            }
            if let Some(editor) = state.editor.as_mut()
                && editor.task_path == old_path
            {
                editor.task_path = new_path.to_path_buf();
                editor.target = WorkflowSelectionTarget::Step {
                    task_path: new_path.to_path_buf(),
                    step_path: editor.step_path.clone(),
                };
            }
        }
    }

    /// 清理指向某个 step 子树的 workflow 片段编辑器。
    fn clear_workflow_editor_for_step_subtree(
        &mut self,
        index: usize,
        task_path: &Path,
        step_path: &[usize],
    ) {
        if let Some(state) = self.workflow_states.get_mut(index)
            && let Some(editor) = state.editor.as_ref()
            && editor.task_path == task_path
            && editor.step_path.starts_with(step_path)
        {
            state.editor = None;
            state.selected = None;
        }
    }

    /// 清理指向已删除路径前缀的文档和 workflow 状态。
    fn clear_workflow_document_under_prefix(&mut self, index: usize, prefix: &Path) {
        if let Some(document) = self.documents.get_mut(index)
            && document
                .path
                .as_ref()
                .is_some_and(|path| path.starts_with(prefix))
        {
            *document = DocumentState::default();
        }
        if let Some(workspace) = self.workspaces.get_mut(index)
            && workspace
                .selected_file
                .as_ref()
                .is_some_and(|path| path.starts_with(prefix))
        {
            workspace.selected_file = None;
        }
        if let Some(state) = self.workflow_states.get_mut(index) {
            state.task_editor = None;
            state.editor = None;
            state.selected = None;
        }
    }

    /// 清理指向已删除单文件的文档和 workflow 状态。
    fn clear_workflow_document_path(&mut self, index: usize, path: &Path) {
        if let Some(document) = self.documents.get_mut(index)
            && document.path.as_deref() == Some(path)
        {
            *document = DocumentState::default();
        }
        if let Some(workspace) = self.workspaces.get_mut(index)
            && workspace.selected_file.as_deref() == Some(path)
        {
            workspace.selected_file = None;
        }
        if let Some(state) = self.workflow_states.get_mut(index) {
            let selected_deleted = match state.selected.as_ref() {
                Some(WorkflowSelectionTarget::Task { task_path }) => task_path == path,
                Some(WorkflowSelectionTarget::Step { task_path, .. }) => task_path == path,
                Some(WorkflowSelectionTarget::Project { .. }) | None => false,
            };
            if selected_deleted {
                state.task_editor = None;
                state.editor = None;
                state.selected = None;
            }
        }
    }

    /// 返回 workflow 修改 key 在当前 tree 中的校验错误。
    pub(super) fn workflow_mutation_key_error(
        &self,
        request: &WorkflowMutationRequest,
    ) -> Option<String> {
        match request {
            WorkflowMutationRequest::InitRoot => None,
            WorkflowMutationRequest::AddTask {
                project_key,
                task_key,
            } => {
                if let Err(error) = crate::gui::workflow::validate_workflow_key(task_key) {
                    return Some(error);
                }
                self.workflow_task_duplicate_error(project_key, task_key, None)
            }
            WorkflowMutationRequest::AddStep { task_path, key, .. } => {
                let key = match crate::gui::workflow::validate_workflow_step_title(key) {
                    Ok(key) => key,
                    Err(error) => return Some(error),
                };
                self.workflow_step_duplicate_error(task_path, key, None)
            }
            WorkflowMutationRequest::RenameProject {
                project_key,
                new_key,
            } => {
                if let Err(error) = crate::gui::workflow::validate_workflow_key(new_key) {
                    return Some(error);
                }
                if project_key == new_key {
                    return None;
                }
                self.workflow_project_duplicate_error(project_key, new_key)
            }
            WorkflowMutationRequest::RenameTask { task_path, new_key } => {
                if let Err(error) = crate::gui::workflow::validate_workflow_key(new_key) {
                    return Some(error);
                }
                let current_key = self.workflow_task_key_for_path(task_path)?;
                if current_key == *new_key {
                    return None;
                }
                let project_key = task_path
                    .parent()
                    .and_then(Path::file_name)
                    .map(|name| name.to_string_lossy().to_string())?;
                self.workflow_task_duplicate_error(&project_key, new_key, Some(task_path))
            }
            WorkflowMutationRequest::RenameStep {
                task_path,
                step_path,
                new_key,
            } => {
                let new_key = match crate::gui::workflow::validate_workflow_step_title(new_key) {
                    Ok(new_key) => new_key,
                    Err(error) => return Some(error),
                };
                let current = self.workflow_step_title(task_path, step_path)?;
                if current == new_key {
                    return None;
                }
                self.workflow_step_duplicate_error(task_path, new_key, Some(step_path))
            }
            _ => None,
        }
    }

    /// 检查 project key 是否和现有 project 重复。
    fn workflow_project_duplicate_error(&self, current_key: &str, key: &str) -> Option<String> {
        let tree = self
            .workflow_states
            .get(self.active_workspace)?
            .tree
            .as_ref()?;
        tree.projects
            .iter()
            .any(|project| project.key != current_key && project.key == key)
            .then(|| format!("project already exists: {key}"))
    }

    /// 检查 task 是否和同 project 现有 task key 重复。
    fn workflow_task_duplicate_error(
        &self,
        project_key: &str,
        key: &str,
        exclude_task_path: Option<&Path>,
    ) -> Option<String> {
        let tree = self
            .workflow_states
            .get(self.active_workspace)?
            .tree
            .as_ref()?;
        let project = tree
            .projects
            .iter()
            .find(|project| project.key == project_key)?;
        project
            .tasks
            .iter()
            .any(|task| {
                Some(task.path.as_path()) != exclude_task_path && workflow_task_key(task) == key
            })
            .then(|| format!("task already exists at this level: {key}"))
    }

    /// 检查 step 是否和同级 step key 重复。
    fn workflow_step_duplicate_error(
        &self,
        task_path: &Path,
        key: &str,
        exclude_step_path: Option<&[usize]>,
    ) -> Option<String> {
        let tree = self
            .workflow_states
            .get(self.active_workspace)?
            .tree
            .as_ref()?;
        let task = tree
            .projects
            .iter()
            .flat_map(|project| project.tasks.iter())
            .find(|task| task.path == task_path)?;
        task.steps
            .iter()
            .any(|step| exclude_step_path != Some(step.path.as_slice()) && step.title == key)
            .then(|| format!("step already exists at this level: {key}"))
    }

    /// 返回指定 task path 当前对应的 task key。
    fn workflow_task_key_for_path(&self, task_path: &Path) -> Option<String> {
        let tree = self
            .workflow_states
            .get(self.active_workspace)?
            .tree
            .as_ref()?;
        tree.projects
            .iter()
            .flat_map(|project| project.tasks.iter())
            .find(|task| task.path == task_path)
            .map(workflow_task_key)
    }

    /// 返回指定 step path 当前对应的 step title。
    fn workflow_step_title(&self, task_path: &Path, step_path: &[usize]) -> Option<String> {
        let tree = self
            .workflow_states
            .get(self.active_workspace)?
            .tree
            .as_ref()?;
        let task = tree
            .projects
            .iter()
            .flat_map(|project| project.tasks.iter())
            .find(|task| task.path == task_path)?;
        workflow_step_node_at_path(&task.steps, step_path).map(|step| step.title.clone())
    }
}

/// 递归查找指定路径的 step 节点。
fn workflow_step_node_at_path<'a>(
    nodes: &'a [WorkflowStepNode],
    path: &[usize],
) -> Option<&'a WorkflowStepNode> {
    let (first, rest) = path.split_first()?;
    let node = nodes.get(*first)?;
    if rest.is_empty() {
        Some(node)
    } else {
        workflow_step_node_at_path(&node.children, rest)
    }
}

/// 返回 workflow 目标所属的 task 路径。
fn workflow_target_task_path(target: &WorkflowSelectionTarget) -> Option<&Path> {
    match target {
        WorkflowSelectionTarget::Task { task_path }
        | WorkflowSelectionTarget::Step { task_path, .. } => Some(task_path.as_path()),
        WorkflowSelectionTarget::Project { .. } => None,
    }
}

/// 从 workflow tree 里生成 task 的复制路径。
fn workflow_task_copy_path_from_tree(tree: &WorkflowTree, task_path: &Path) -> Option<String> {
    tree.projects.iter().find_map(|project| {
        project
            .tasks
            .iter()
            .find(|task| task.path == task_path)
            .map(|task| workflow_copy_path_from_parts(&[project.label.clone(), task.label.clone()]))
    })
}

/// 从 workflow tree 里生成 step 的复制路径。
fn workflow_step_copy_path_from_tree(
    tree: &WorkflowTree,
    task_path: &Path,
    step_path: &[usize],
) -> Option<String> {
    tree.projects.iter().find_map(|project| {
        project.tasks.iter().find_map(|task| {
            if task.path != task_path {
                return None;
            }
            let mut parts = vec![project.label.clone(), task.label.clone()];
            parts.extend(workflow_step_titles_at_path(&task.steps, step_path)?);
            Some(workflow_copy_path_from_parts(&parts))
        })
    })
}

/// 返回 step 路径上的各级 step 标题。
fn workflow_step_titles_at_path(nodes: &[WorkflowStepNode], path: &[usize]) -> Option<Vec<String>> {
    let (first, rest) = path.split_first()?;
    let node = nodes.get(*first)?;
    let mut titles = vec![node.title.clone()];
    if !rest.is_empty() {
        titles.extend(workflow_step_titles_at_path(&node.children, rest)?);
    }
    Some(titles)
}

/// 将 workflow 层级拼成可复制路径。
fn workflow_copy_path_from_parts(parts: &[String]) -> String {
    parts.join(" > ")
}

/// 从 task 节点提取新增弹窗使用的 task key。
fn workflow_task_key(task: &WorkflowTaskNode) -> String {
    let stem = task
        .path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| task.label.clone());
    stem.strip_prefix("task-")
        .map(str::to_string)
        .unwrap_or(stem)
}

/// 计算 task 重命名后的 workspace 相对路径。
fn workflow_task_path_after_rename(task_path: &Path, new_key: &str) -> PathBuf {
    task_path
        .parent()
        .map(|parent| parent.join(format!("task-{new_key}.md")))
        .unwrap_or_else(|| PathBuf::from(format!("task-{new_key}.md")))
}

/// 如果路径正好匹配旧路径，则改写成新路径。
fn rewrite_optional_exact_path(path: &mut Option<PathBuf>, old_path: &Path, new_path: &Path) {
    if path.as_deref() == Some(old_path) {
        *path = Some(new_path.to_path_buf());
    }
}

/// 如果路径位于旧前缀下，则改写成新前缀下的路径。
fn rewrite_optional_path_prefix(path: &mut Option<PathBuf>, old_prefix: &Path, new_prefix: &Path) {
    if let Some(next_path) = path
        .as_deref()
        .and_then(|path| rewritten_path_prefix(path, old_prefix, new_prefix))
    {
        *path = Some(next_path);
    }
}

/// 返回替换前缀后的路径。
fn rewritten_path_prefix(path: &Path, old_prefix: &Path, new_prefix: &Path) -> Option<PathBuf> {
    path.strip_prefix(old_prefix)
        .ok()
        .map(|suffix| new_prefix.join(suffix))
}

/// 重写 workflow selection 中的路径前缀。
fn rewrite_workflow_selection_prefix(
    selection: &mut Option<WorkflowSelectionTarget>,
    old_prefix: &Path,
    new_prefix: &Path,
) {
    match selection {
        Some(WorkflowSelectionTarget::Project { root_path }) => {
            if let Some(next_path) = rewritten_path_prefix(root_path, old_prefix, new_prefix) {
                *root_path = next_path;
            }
        }
        Some(WorkflowSelectionTarget::Task { task_path }) => {
            if let Some(next_path) = rewritten_path_prefix(task_path, old_prefix, new_prefix) {
                *task_path = next_path;
            }
        }
        Some(WorkflowSelectionTarget::Step { task_path, .. }) => {
            if let Some(next_path) = rewritten_path_prefix(task_path, old_prefix, new_prefix) {
                *task_path = next_path;
            }
        }
        None => {}
    }
}

/// 重写 workflow selection 中的精确 task 路径。
fn rewrite_workflow_selection_path(
    selection: &mut Option<WorkflowSelectionTarget>,
    old_path: &Path,
    new_path: &Path,
) {
    match selection {
        Some(WorkflowSelectionTarget::Task { task_path })
        | Some(WorkflowSelectionTarget::Step { task_path, .. })
            if task_path == old_path =>
        {
            *task_path = new_path.to_path_buf();
        }
        _ => {}
    }
}

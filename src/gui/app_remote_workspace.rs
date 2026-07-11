//! SSH Remote Workspace 的后台任务派发与轻量 UI 状态合并。

use super::*;

impl GsdvGuiApp {
    /// 为持久化 Remote Workspace 各执行一次直接 SSH 加载。
    ///
    /// 适用场景：启动 gsdv -> 读取远端 Outline，不保留后台健康检查。
    pub(super) fn start_remote_workspace_loads(&self, ctx: &egui::Context) {
        for (index, runtime) in self.remote_workspaces.iter().enumerate() {
            let Some(runtime) = runtime else {
                continue;
            };
            let Some(config) = self
                .workspaces
                .get(index)
                .and_then(|workspace| workspace.remote.clone())
            else {
                continue;
            };
            let id = runtime.id;
            let tx = self.app_event_tx.clone();
            let repaint_ctx = ctx.clone();
            let repaint_controller = self.repaint_controller.clone();
            self.background_runtime.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    let shell = remote_workspace::detect_remote_shell(&config)?;
                    let snapshot = remote_workspace::load_remote_snapshot(&config, shell)?;
                    Ok::<_, anyhow::Error>((shell, snapshot))
                })
                .await;
                match result {
                    Ok(Ok((shell, snapshot))) => {
                        let _ = tx.send(AppEvent::RemoteWorkspaceConnected {
                            id,
                            shell,
                            snapshot,
                        });
                    }
                    Ok(Err(error)) => {
                        let _ = tx.send(AppEvent::RemoteWorkspaceConnectionLost {
                            id,
                            error: error.to_string(),
                        });
                    }
                    Err(error) => {
                        let _ = tx.send(AppEvent::RemoteWorkspaceConnectionLost {
                            id,
                            error: error.to_string(),
                        });
                    }
                }
                repaint_controller.request_repaint(&repaint_ctx);
            });
        }
    }

    /// 后台验证新 Remote 配置并加载首份远端快照。
    ///
    /// 适用场景：Remote 创建弹窗点击连接。例：认证失败 -> 返回弹窗错误且不创建。
    pub(super) fn spawn_remote_workspace_add_task(
        &self,
        ctx: &egui::Context,
        mode: RemoteWorkspaceDialogMode,
        config: RemoteWorkspaceConfig,
    ) {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let task_config = config.clone();
            let result = tokio::task::spawn_blocking(move || {
                let shell = remote_workspace::detect_remote_shell(&task_config)
                    .map_err(|error| error.to_string())?;
                let snapshot = remote_workspace::load_remote_snapshot(&task_config, shell)
                    .map_err(|error| error.to_string())?;
                Ok((shell, snapshot))
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);
            let _ = tx.send(AppEvent::RemoteWorkspaceAddPrepared {
                mode,
                config,
                result,
            });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// 应用新 Remote Workspace 的验证结果。
    ///
    /// 适用场景：后台认证结束。例：成功 -> 加入 Rail 并启动 Agent。
    pub(super) fn apply_remote_workspace_add_result(
        &mut self,
        ctx: &egui::Context,
        mode: RemoteWorkspaceDialogMode,
        config: RemoteWorkspaceConfig,
        result: Result<(RemoteShell, RemoteWorkspaceSnapshot), String>,
    ) {
        let (shell, snapshot) = match result {
            Ok(result) => result,
            Err(error) => {
                self.set_active_app_dialog(Some(AppDialog::RemoteWorkspace {
                    mode,
                    form: RemoteWorkspaceForm::from_config(config),
                    error: Some(error),
                    in_flight: false,
                }));
                return;
            }
        };
        if let RemoteWorkspaceDialogMode::Edit(index) = mode {
            self.replace_remote_workspace_connection(ctx, index, config, shell, snapshot);
            self.clear_remote_workspace_dialogs();
            return;
        }
        let workspace = data::new_remote_workspace(config.clone(), self.default_agent_kind);
        self.apply_workspace_add_result(ctx, Ok(WorkspaceAddTaskResult::New { workspace }));
        let index = self.active_workspace;
        let mut runtime = RemoteWorkspaceRuntime::disconnected(&config);
        runtime.shell = Some(shell);
        runtime.connected = true;
        let id = runtime.id;
        if let Some(slot) = self.remote_workspaces.get_mut(index) {
            *slot = Some(runtime);
        }
        self.apply_remote_workspace_connected(ctx, id, shell, snapshot);
        self.clear_remote_workspace_dialogs();
    }

    /// 清理创建/编辑 Remote 时可能留在旧 workspace 槽位的表单。
    ///
    /// 适用场景：创建成功后 active workspace 已切到新项。例：切回旧项 -> 不再弹出旧表单。
    fn clear_remote_workspace_dialogs(&mut self) {
        for dialog in &mut self.app_dialogs {
            if matches!(dialog, Some(AppDialog::RemoteWorkspace { .. })) {
                *dialog = None;
            }
        }
        if matches!(
            self.global_app_dialog,
            Some(AppDialog::RemoteWorkspace { .. })
        ) {
            self.global_app_dialog = None;
        }
    }

    /// 用已验证的新连接替换现有 Remote Workspace 配置。
    ///
    /// 适用场景：编辑 host、用户或认证成功。例：验证成功 -> 替换配置。
    fn replace_remote_workspace_connection(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        config: RemoteWorkspaceConfig,
        shell: RemoteShell,
        snapshot: RemoteWorkspaceSnapshot,
    ) {
        if index >= self.workspaces.len() {
            return;
        }
        let _old_runtime = self.remote_workspaces.get_mut(index).and_then(Option::take);
        self.terminal_hosts[index].agents.clear();
        self.workspaces[index].name = config.name.clone();
        self.workspaces[index].remote = Some(config.clone());
        let mut runtime = RemoteWorkspaceRuntime::disconnected(&config);
        runtime.shell = Some(shell);
        runtime.connected = true;
        let id = runtime.id;
        self.remote_workspaces[index] = Some(runtime);
        self.apply_remote_workspace_connected(ctx, id, shell, snapshot);
        self.persist_workspaces();
    }

    /// 合并 Remote Workspace 成功连接事件。
    ///
    /// 适用场景：首次连接或重连。例：重连 -> 刷新布局并恢复当前可见 Agent。
    pub(super) fn apply_remote_workspace_connected(
        &mut self,
        ctx: &egui::Context,
        id: u64,
        shell: RemoteShell,
        snapshot: RemoteWorkspaceSnapshot,
    ) {
        let Some(index) = self.remote_workspace_index(id) else {
            return;
        };
        let was_disconnected = self
            .remote_workspaces
            .get(index)
            .and_then(Option::as_ref)
            .is_some_and(|runtime| !runtime.connected);
        self.capture_selected_remote_project(index);
        let old_projects = self
            .remote_workspaces
            .get_mut(index)
            .and_then(Option::as_mut)
            .map(|runtime| std::mem::take(&mut runtime.projects))
            .unwrap_or_default();
        let network_settings = snapshot.network_settings.clone();
        let mut projects = snapshot
            .projects
            .into_iter()
            .map(|project| {
                let agents = old_projects
                    .iter()
                    .find(|current| current.snapshot.path == project.path)
                    .map(|_| BTreeMap::new())
                    .unwrap_or_default();
                RemoteProjectRuntime {
                    snapshot: project,
                    agents,
                }
            })
            .collect::<Vec<_>>();
        for old in old_projects {
            if let Some(project) = projects
                .iter_mut()
                .find(|project| project.snapshot.path == old.snapshot.path)
            {
                project.agents = old.agents;
            }
        }
        if was_disconnected {
            for project in &mut projects {
                for slot in project.agents.values_mut() {
                    slot.host = None;
                    slot.error = None;
                }
            }
        }
        let selected = snapshot.selected_store_index.and_then(|store_index| {
            projects
                .iter()
                .position(|project| project.snapshot.store_index == store_index)
        });
        if let Some(runtime) = self
            .remote_workspaces
            .get_mut(index)
            .and_then(Option::as_mut)
        {
            runtime.shell = Some(shell);
            runtime.connected = true;
            runtime.error = None;
            runtime.network_settings = network_settings;
            runtime.projects = projects;
            runtime.selected_project = selected;
        }
        self.restore_selected_remote_project(index);
        if index == self.active_workspace {
            self.spawn_visible_remote_agents(ctx, index);
        }
        if was_disconnected {
            self.spawn_background_remote_agents(ctx, index);
        }
    }

    /// 标记 Remote Workspace 已断线，但保留现有 terminal buffer。
    ///
    /// 适用场景：monitor 检查失败。例：断网 -> Rail 显示错误并等待自动重连。
    pub(super) fn apply_remote_workspace_connection_lost(&mut self, id: u64, error: String) {
        let Some(index) = self.remote_workspace_index(id) else {
            return;
        };
        if let Some(runtime) = self
            .remote_workspaces
            .get_mut(index)
            .and_then(Option::as_mut)
        {
            runtime.connected = false;
            runtime.error = Some(error);
        }
    }

    /// 应用切回 Remote Rail 项触发的快照刷新。
    ///
    /// 适用场景：远端另一份 gsdv 改过布局。例：成功 -> 远端布局成为来源。
    pub(super) fn apply_remote_workspace_refresh(
        &mut self,
        ctx: &egui::Context,
        id: u64,
        result: Result<RemoteWorkspaceSnapshot, String>,
    ) {
        match result {
            Ok(snapshot) => {
                let shell = self
                    .remote_workspace_index(id)
                    .and_then(|index| self.remote_workspaces.get(index))
                    .and_then(Option::as_ref)
                    .and_then(|runtime| runtime.shell);
                if let Some(shell) = shell {
                    self.apply_remote_workspace_connected(ctx, id, shell, snapshot);
                }
            }
            Err(error) => self.apply_remote_workspace_connection_lost(id, error),
        }
    }

    /// 记录远端写入结果，失败时展示到 Remote Outline 状态。
    ///
    /// 适用场景：active/layout 后台保存。例：失败 -> 不阻塞 UI，但保留错误。
    pub(super) fn apply_remote_workspace_saved(&mut self, id: u64, result: Result<(), String>) {
        let Some(index) = self.remote_workspace_index(id) else {
            return;
        };
        if let Err(error) = result
            && let Some(runtime) = self
                .remote_workspaces
                .get_mut(index)
                .and_then(Option::as_mut)
        {
            runtime.error = Some(error);
        }
    }

    /// 切换 Remote Outline 的选中项目并回写远端 active。
    ///
    /// 适用场景：用户点击项目列表。例：A -> B，A 的 Agent terminal 进入缓存。
    pub(super) fn select_remote_project(
        &mut self,
        ctx: &egui::Context,
        workspace_index: usize,
        project_index: usize,
    ) {
        let valid = self
            .remote_workspaces
            .get(workspace_index)
            .and_then(Option::as_ref)
            .is_some_and(|runtime| project_index < runtime.projects.len());
        if !valid {
            return;
        }
        self.capture_selected_remote_project(workspace_index);
        if let Some(runtime) = self
            .remote_workspaces
            .get_mut(workspace_index)
            .and_then(Option::as_mut)
        {
            runtime.selected_project = Some(project_index);
        }
        self.restore_selected_remote_project(workspace_index);
        self.spawn_remote_active_save(workspace_index, project_index, ctx);
        self.spawn_visible_remote_agents(ctx, workspace_index);
    }

    /// 将当前 WorkspaceViewData 和 Agent hosts 放回远端项目缓存。
    ///
    /// 适用场景：项目切换或刷新前。例：当前 A -> runtime.projects[A]。
    fn capture_selected_remote_project(&mut self, index: usize) {
        let Some(project_index) = self
            .remote_workspaces
            .get(index)
            .and_then(Option::as_ref)
            .and_then(|runtime| runtime.selected_project)
        else {
            return;
        };
        let Some(workspace) = self.workspaces.get(index) else {
            return;
        };
        let payload = RemoteLayoutPayload {
            workspace_path: workspace.path.clone(),
            agent_kind: workspace.agent_kind,
            agent_model: workspace.agent_model.clone(),
            agent_model_provider: workspace.agent_model_provider.clone(),
            agent_effort: workspace.agent_effort.clone(),
            agent_fast_mode: workspace.agent_fast_mode,
            agent_work_dir: workspace.agent_work_dir.clone(),
            subagents: workspace.subagents.clone(),
            rows: workspace.agent_rows.clone(),
            focus: workspace.agent_focus,
        };
        if let Some(project) = self
            .remote_workspaces
            .get_mut(index)
            .and_then(Option::as_mut)
            .and_then(|runtime| runtime.projects.get_mut(project_index))
        {
            project.snapshot.agent_kind = workspace.agent_kind;
            project.snapshot.agent_model = workspace.agent_model.clone();
            project.snapshot.agent_model_provider = workspace.agent_model_provider.clone();
            project.snapshot.agent_effort = workspace.agent_effort.clone();
            project.snapshot.agent_fast_mode = workspace.agent_fast_mode;
            project.snapshot.agent_work_dir = workspace.agent_work_dir.clone();
            project.snapshot.session_id = workspace.session_id.clone();
            project.snapshot.subagents = payload.subagents;
            project.snapshot.rows = payload.rows;
            project.snapshot.focus = payload.focus;
            if let Some(hosts) = self.terminal_hosts.get_mut(index) {
                project.agents = std::mem::take(&mut hosts.agents);
            }
        }
    }

    /// 把选中远端项目恢复到现有 Workspace/terminal 渲染槽。
    ///
    /// 适用场景：Remote Outline 选择或重连。例：项目 B -> Agent 网格显示 B。
    fn restore_selected_remote_project(&mut self, index: usize) {
        let selected = self
            .remote_workspaces
            .get(index)
            .and_then(Option::as_ref)
            .and_then(|runtime| runtime.selected_project);
        let Some(project_index) = selected else {
            if let Some(hosts) = self.terminal_hosts.get_mut(index) {
                hosts.agents.clear();
            }
            return;
        };
        let Some(runtime) = self
            .remote_workspaces
            .get_mut(index)
            .and_then(Option::as_mut)
        else {
            return;
        };
        let Some(project) = runtime.projects.get_mut(project_index) else {
            return;
        };
        let snapshot = &project.snapshot;
        if let Some(workspace) = self.workspaces.get_mut(index) {
            workspace.path = snapshot.path.clone();
            workspace.agent_kind = snapshot.agent_kind;
            workspace.agent_model = snapshot.agent_model.clone();
            workspace.agent_model_provider = snapshot.agent_model_provider.clone();
            workspace.agent_effort = snapshot.agent_effort.clone();
            workspace.agent_fast_mode = snapshot.agent_fast_mode;
            workspace.agent_work_dir = snapshot.agent_work_dir.clone();
            workspace.agent_id = snapshot.agent_id.clone();
            workspace.session_id = snapshot.session_id.clone();
            workspace.subagents = snapshot.subagents.clone();
            workspace.agent_rows = snapshot.rows.clone();
            workspace.agent_focus = snapshot.focus;
            workspace.center_mode = CenterMode::Agent;
            workspace.route = Route::Workspace;
        }
        if let Some(hosts) = self.terminal_hosts.get_mut(index) {
            hosts.agents = std::mem::take(&mut project.agents);
        }
    }

    /// 将当前 Remote 布局异步写回远端 sidecar。
    ///
    /// 适用场景：复用现有 persist_workspaces 调用点。例：移动 tab -> 后台 helper 保存。
    pub(super) fn spawn_active_remote_layout_save(&self, ctx: &egui::Context) {
        let index = self.active_workspace;
        let Some(workspace) = self
            .workspaces
            .get(index)
            .filter(|workspace| workspace.remote.is_some())
        else {
            return;
        };
        let Some(runtime) = self.remote_workspaces.get(index).and_then(Option::as_ref) else {
            return;
        };
        let payload = RemoteLayoutPayload {
            workspace_path: workspace.path.clone(),
            agent_kind: workspace.agent_kind,
            agent_model: workspace.agent_model.clone(),
            agent_model_provider: workspace.agent_model_provider.clone(),
            agent_effort: workspace.agent_effort.clone(),
            agent_fast_mode: workspace.agent_fast_mode,
            agent_work_dir: workspace.agent_work_dir.clone(),
            subagents: workspace.subagents.clone(),
            rows: workspace.agent_rows.clone(),
            focus: workspace.agent_focus,
        };
        self.spawn_remote_write(ctx, runtime, move |config, shell| {
            remote_workspace::save_remote_layout(&config, shell, &payload)
        });
    }

    /// 切回 Remote Workspace 时异步刷新远端 store 与布局。
    ///
    /// 适用场景：Workspace Rail 切换。例：Local -> Remote 后读取一次。
    pub(super) fn spawn_remote_workspace_refresh(&self, ctx: &egui::Context, index: usize) {
        let Some(runtime) = self.remote_workspaces.get(index).and_then(Option::as_ref) else {
            return;
        };
        let Some(config) = self
            .workspaces
            .get(index)
            .and_then(|workspace| workspace.remote.clone())
        else {
            return;
        };
        let id = runtime.id;
        let Some(shell) = runtime.shell else {
            return;
        };
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                remote_workspace::load_remote_snapshot(&config, shell)
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);
            let _ = tx.send(AppEvent::RemoteWorkspaceRefreshed { id, result });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// 完成 Remote Workspace 删除所需的异步收尾事件。
    ///
    /// 适用场景：删除 Rail 项。例：没有额外连接资源 -> 直接完成。
    pub(super) fn spawn_remote_workspace_close(&self, ctx: &egui::Context, index: usize) {
        let workspace_path = self.workspaces[index].path.clone();
        let _ = self
            .app_event_tx
            .send(AppEvent::WorkspaceCloseSidecarsDeleted {
                index,
                workspace_path,
                result: Ok(()),
            });
        self.repaint_controller.request_repaint(ctx);
    }

    /// 为远端项目当前可见 Agent 请求统一 terminal host 创建路径。
    ///
    /// 适用场景：首次选中或切换项目。例：每个未折叠列 active_slot -> 直接 SSH。
    fn spawn_visible_remote_agents(&mut self, ctx: &egui::Context, index: usize) {
        let slots = self
            .workspaces
            .get(index)
            .map(|workspace| {
                workspace
                    .agent_rows
                    .iter()
                    .filter(|row| !row.collapsed)
                    .flat_map(|row| row.columns.iter().filter(|column| !column.collapsed))
                    .map(|column| AgentSlotId::from_column_slot(&column.active_slot))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for slot in slots {
            if let Some(active) = self.active_agent_slots.get_mut(index) {
                *active = slot;
            }
            self.spawn_terminal_host_for_workspace(ctx, index, TerminalSurfaceKind::Agent);
        }
    }

    /// 启动其他远端项目已经选中的 Agent slots。
    ///
    /// 适用场景：项目 A 可见、项目 B 后台运行时主连接断开。例：重连 -> B 后台 resume。
    fn spawn_background_remote_agents(&mut self, ctx: &egui::Context, index: usize) {
        let Some(workspace) = self.workspaces.get(index) else {
            return;
        };
        let Some(config) = workspace.remote.clone() else {
            return;
        };
        let workspace_name = workspace.name.clone();
        let Some(runtime) = self.remote_workspaces.get(index).and_then(Option::as_ref) else {
            return;
        };
        let Some(shell) = runtime.shell else {
            return;
        };
        let network_settings = runtime.network_settings.clone();
        let selected = runtime.selected_project;
        let requests = runtime
            .projects
            .iter()
            .enumerate()
            .filter(|(project_index, _)| Some(*project_index) != selected)
            .flat_map(|(_, project)| {
                let slots = (!project.agents.is_empty())
                    .then(|| {
                        project
                            .snapshot
                            .rows
                            .iter()
                            .filter(|row| !row.collapsed)
                            .flat_map(|row| row.columns.iter().filter(|column| !column.collapsed))
                            .map(|column| AgentSlotId::from_column_slot(&column.active_slot))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                slots.into_iter().filter_map(|slot| {
                    remote_project_terminal_metadata(
                        &workspace_name,
                        &config,
                        shell,
                        &network_settings,
                        &project.snapshot,
                        &slot,
                    )
                    .map(|metadata| (slot, metadata))
                })
            })
            .collect::<Vec<_>>();
        for (slot, metadata) in requests {
            let key = TerminalSpawnKey {
                index,
                workspace_path: metadata.path.clone(),
                kind: TerminalSurfaceKind::Agent,
                agent_slot: slot,
            };
            self.spawn_terminal_host_task(ctx, key, metadata);
        }
    }

    /// 保存 Remote Outline 选择到远端 store.active。
    ///
    /// 适用场景：项目点击后。例：过滤列表下标 1 -> 原始 store_index 3。
    fn spawn_remote_active_save(&self, index: usize, project_index: usize, ctx: &egui::Context) {
        let Some(runtime) = self.remote_workspaces.get(index).and_then(Option::as_ref) else {
            return;
        };
        let Some(store_index) = runtime
            .projects
            .get(project_index)
            .map(|project| project.snapshot.store_index)
        else {
            return;
        };
        self.spawn_remote_write(ctx, runtime, move |config, shell| {
            remote_workspace::save_remote_active(&config, shell, store_index)
        });
    }

    /// 派发一个独立 SSH 远端写入任务。
    ///
    /// 适用场景：active 和 layout 共用事件语义。例：失败 -> RemoteWorkspaceSaved Err。
    fn spawn_remote_write(
        &self,
        ctx: &egui::Context,
        runtime: &RemoteWorkspaceRuntime,
        operation: impl FnOnce(RemoteWorkspaceConfig, RemoteShell) -> anyhow::Result<()>
        + Send
        + 'static,
    ) {
        let Some(config) = self
            .workspaces
            .get(self.active_workspace)
            .and_then(|workspace| workspace.remote.clone())
        else {
            return;
        };
        let id = runtime.id;
        let Some(shell) = runtime.shell else {
            return;
        };
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || operation(config, shell))
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
            let _ = tx.send(AppEvent::RemoteWorkspaceSaved { id, result });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// 根据稳定运行时 ID 查找当前 Rail 下标。
    ///
    /// 适用场景：删除/排序后丢弃旧异步结果。例：ID 不存在 -> None。
    fn remote_workspace_index(&self, id: u64) -> Option<usize> {
        self.remote_workspaces
            .iter()
            .position(|runtime| runtime.as_ref().is_some_and(|runtime| runtime.id == id))
    }
}

/// 为未显示的远端项目构造一个 Agent slot 启动快照。
///
/// 适用场景：重连后无需切换 Outline 即可后台 resume。例：Subagent(id) -> 对应 subagent 元数据。
fn remote_project_terminal_metadata(
    workspace_name: &str,
    config: &RemoteWorkspaceConfig,
    shell: RemoteShell,
    network_settings: &NetworkSettings,
    project: &RemoteProjectSnapshot,
    slot: &AgentSlotId,
) -> Option<TerminalWorkspaceMetadata> {
    let mut metadata = TerminalWorkspaceMetadata {
        remote: Some(RemoteTerminalMetadata {
            config: config.clone(),
            shell,
            network_settings: network_settings.clone(),
        }),
        name: workspace_name.to_string(),
        path: project.path.clone(),
        agent_kind: project.agent_kind,
        agent_model: project.agent_model.clone(),
        agent_model_provider: project.agent_model_provider.clone(),
        agent_effort: project.agent_effort.clone(),
        agent_fast_mode: project.agent_fast_mode,
        agent_work_dir: project.agent_work_dir.clone(),
        agent_id: project.agent_id.clone(),
        session_id: project.session_id.clone(),
        activity: WorkspaceActivity::Unknown,
    };
    if let AgentSlotId::Subagent(id) = slot {
        let subagent = project
            .subagents
            .iter()
            .find(|subagent| &subagent.id == id)?;
        metadata = metadata.for_subagent(subagent);
    }
    Some(metadata)
}

//! workflow tree 和片段编辑业务。

use super::*;

/// 新 Workflow Agent 首次启动后自动发送的固定提示词模板；`$path` 会替换为逻辑路径。
const WORKFLOW_AGENT_BOOTSTRAP_PROMPT_TEMPLATE: &str =
    "当前gsdv-wf 关注的路径是$path,请你对它进行综合的评估, 简短的回答概况";

impl GsdvGuiApp {
    /// 绘制通用 Workflow Agent 右键菜单项。
    ///
    /// 适用场景：Outline 与 Quick Modal 共用。例：已有 Agent -> Agent + Destroy Agent。
    pub(super) fn workflow_agent_context_items(
        &self,
        ui: &mut Ui,
        tree: &WorkflowTree,
        selection: &WorkflowSelectionTarget,
        action: &mut Option<WorkflowAgentUiAction>,
    ) {
        let Some(binding) = crate::gui::workflow::workflow_agent_binding(tree, selection) else {
            return;
        };
        if ui.button("Agent").clicked() {
            *action = Some(WorkflowAgentUiAction::Open(selection.clone()));
            ui.close_menu();
        }
        let exists = self
            .workspaces
            .get(self.active_workspace)
            .is_some_and(|workspace| {
                workspace
                    .subagents
                    .iter()
                    .any(|agent| agent.workflow_target.as_ref() == Some(&binding.target))
            });
        if exists && ui.button("Destroy Agent").clicked() {
            *action = Some(WorkflowAgentUiAction::Destroy(selection.clone()));
            ui.close_menu();
        }
        ui.separator();
    }

    /// 执行 Workflow 右键菜单产生的 Agent 动作。
    ///
    /// 适用场景：render 完成后再修改 app 状态。例：Destroy -> 稳定 target 批量清理。
    pub(super) fn apply_workflow_agent_ui_action(
        &mut self,
        ctx: &egui::Context,
        tree: &WorkflowTree,
        action: WorkflowAgentUiAction,
    ) {
        match action {
            WorkflowAgentUiAction::Open(selection) => {
                self.request_workflow_agent(ctx, selection);
            }
            WorkflowAgentUiAction::Destroy(selection) => {
                if let Some(binding) =
                    crate::gui::workflow::workflow_agent_binding(tree, &selection)
                {
                    self.destroy_workflow_agent(ctx, self.active_workspace, &binding.target);
                }
            }
        }
    }

    /// 判断目标自身或任一后代是否存在 Busy Workflow Agent。
    ///
    /// 适用场景：Outline/Quick Modal 状态标签；上级自身没有 Agent 也会冒泡。例：Step Busy -> Repo Busy。
    pub(super) fn workflow_selection_is_busy(
        &self,
        tree: &WorkflowTree,
        selection: &WorkflowSelectionTarget,
    ) -> bool {
        let Some(binding) = crate::gui::workflow::workflow_agent_binding(tree, selection) else {
            return false;
        };
        self.workspaces
            .get(self.active_workspace)
            .into_iter()
            .flat_map(|workspace| workspace.subagents.iter())
            .filter(|agent| agent.activity == WorkspaceActivity::Busy)
            .filter_map(|agent| agent.workflow_target.as_ref())
            .any(|busy| {
                if busy.repo_path != binding.target.repo_path
                    || busy.repo_id != binding.target.repo_id
                {
                    return false;
                }
                match binding.target.kind {
                    data::WorkflowAgentTargetKind::Repo => true,
                    data::WorkflowAgentTargetKind::Project => {
                        busy.project_id == binding.target.project_id
                    }
                    data::WorkflowAgentTargetKind::Task => {
                        busy.project_id == binding.target.project_id
                            && busy.task_id == binding.target.task_id
                    }
                    data::WorkflowAgentTargetKind::Step => busy == &binding.target,
                }
            })
    }

    /// 若当前选择正显示 Workflow Agent，则绘制 terminal surface 并返回 true。
    ///
    /// 适用场景：宽版 Column View 与窄版 Quick Modal 共用。例：Markdown 按钮 -> 返回普通编辑器。
    pub(super) fn workflow_agent_workbench_surface(
        &mut self,
        ui: &mut Ui,
        tree: &WorkflowTree,
        selected: Option<&WorkflowSelectionTarget>,
    ) -> bool {
        let active_target = self
            .workflow_states
            .get(self.active_workspace)
            .and_then(|state| state.active_agent_target.clone());
        let selected_binding = selected
            .and_then(|selection| crate::gui::workflow::workflow_agent_binding(tree, selection));
        let active_agent = active_target
            .filter(|target| {
                selected_binding
                    .as_ref()
                    .is_some_and(|binding| binding.target == *target)
            })
            .and_then(|target| {
                self.workspaces
                    .get(self.active_workspace)?
                    .subagents
                    .iter()
                    .find(|agent| agent.workflow_target.as_ref() == Some(&target))
                    .cloned()
                    .map(|agent| (target, agent))
            });
        let Some((target, agent)) = active_agent else {
            return false;
        };
        let slot = AgentSlotId::Subagent(agent.id.clone());
        let mut show_markdown = false;
        let mut destroy = false;
        let mut agent_action = None;
        let header = ui.horizontal(|ui| {
            ui.label(RichText::new(&agent.name).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Markdown").clicked() {
                    show_markdown = true;
                }
            });
        });
        header.response.context_menu(|ui| {
            agent_slot_context_menu(
                ui,
                slot.clone(),
                self.agent_slot_manually_stopped(self.active_workspace, &slot),
                agent.agent_kind,
                agent.agent_model.as_deref(),
                agent.agent_model_provider.as_deref(),
                agent.agent_effort.as_deref(),
                agent.agent_fast_mode,
                agent.agent_work_dir.as_deref(),
                agent.helix_work_dir.as_deref(),
                Some(agent.name.as_str()),
                agent.session_id.as_deref(),
                None,
                false,
                &mut agent_action,
                self.app_language,
            );
            ui.separator();
            if ui.button("Destroy Agent").clicked() {
                destroy = true;
                ui.close_menu();
            }
        });
        if let Some(action) = agent_action {
            self.handle_agent_tab_action(ui.ctx(), action);
        }
        if destroy {
            self.destroy_workflow_agent(ui.ctx(), self.active_workspace, &target);
            return false;
        }
        ui.separator();
        if show_markdown {
            if let Some(state) = self.workflow_states.get_mut(self.active_workspace)
                && state.active_agent_target.as_ref() == Some(&target)
            {
                state.active_agent_target = None;
            }
            return false;
        }
        self.agent_terminal_host_surface(ui, slot, None, true);
        true
    }

    /// 在 Workflow Agent 首次 host 创建成功后发送且只发送一次固定提示词。
    ///
    /// 适用场景：新建且未 resume 的 Agent；已有 session 不发送。例：pending + task path -> paste + submit。
    pub(super) fn submit_workflow_agent_bootstrap(&mut self, key: &TerminalSpawnKey) {
        if key.kind != TerminalSurfaceKind::Agent {
            return;
        }
        let AgentSlotId::Subagent(id) = &key.agent_slot else {
            return;
        };
        let pending = self.workspaces.get(key.index).and_then(|workspace| {
            workspace
                .subagents
                .iter()
                .find(|agent| agent.id == *id && agent.workflow_target.is_some())
                .filter(|agent| agent.bootstrap_pending && agent.session_id.is_none())
                .and_then(|agent| {
                    Some((
                        agent.workflow_sidecar_path.clone()?,
                        agent.workflow_target.clone()?,
                    ))
                })
        });
        let Some((sidecar_path, target)) = pending else {
            return;
        };
        let path = self
            .workflow_states
            .get(key.index)
            .and_then(|state| state.tree.as_ref())
            .and_then(|tree| workflow_agent_logical_path(tree, &target));
        let Some(path) = path else {
            return;
        };
        let prompt = WORKFLOW_AGENT_BOOTSTRAP_PROMPT_TEMPLATE.replace("$path", &path);
        let Some(host) = self
            .terminal_hosts
            .get_mut(key.index)
            .and_then(|hosts| hosts.agents.get_mut(&key.agent_slot))
            .and_then(|slot| slot.host.as_mut())
        else {
            return;
        };
        if !host.ready_for_remote_input() || host.submit_remote_text(&prompt).is_err() {
            return;
        }
        if let Some(agent) = self
            .workspaces
            .get_mut(key.index)
            .and_then(|workspace| workspace.subagents.iter_mut().find(|agent| agent.id == *id))
        {
            agent.bootstrap_pending = false;
        }
        self.request_workflow_agent_sidecar_save(key.index, sidecar_path);
    }

    /// 在 terminal runtime 更新后重试尚未发送的 Workflow Agent 首次提示词。
    ///
    /// 适用场景：Codex 冷启动时 composer 晚于 PTY 建立。例：首个可输入帧 -> 原子发送提示词与 Enter。
    pub(super) fn submit_workflow_agent_bootstrap_for_terminal(&mut self, terminal_id: u64) {
        let Some((index, agent_slot)) = self.agent_terminal_slot_by_id(terminal_id) else {
            return;
        };
        let Some(workspace_path) = self
            .workspaces
            .get(index)
            .map(|workspace| workspace.path.clone())
        else {
            return;
        };
        self.submit_workflow_agent_bootstrap(&TerminalSpawnKey {
            index,
            workspace_path,
            kind: TerminalSurfaceKind::Agent,
            agent_slot,
        });
    }
    /// 打开目标已有 Workflow Agent，或复用 AddSubagent 面板创建它。
    ///
    /// 适用场景：Repo/Project/Task/Step 右键 Agent。例：已有记录 -> 右侧直接显示。
    pub(super) fn request_workflow_agent(
        &mut self,
        ctx: &egui::Context,
        target: WorkflowSelectionTarget,
    ) {
        let index = self.active_workspace;
        let binding = self
            .workflow_states
            .get(index)
            .and_then(|state| state.tree.as_ref())
            .and_then(|tree| crate::gui::workflow::workflow_agent_binding(tree, &target));
        let Some(binding) = binding else {
            return;
        };
        if let Some(state) = self.workflow_states.get_mut(index) {
            state.selected = Some(target.clone());
        }
        let existing = self.workspaces.get(index).and_then(|workspace| {
            workspace
                .subagents
                .iter()
                .find(|agent| agent.workflow_target.as_ref() == Some(&binding.target))
                .map(|agent| agent.id.clone())
        });
        if let Some(id) = existing {
            if let Some(state) = self.workflow_states.get_mut(index) {
                state.active_agent_target = Some(binding.target);
            }
            if let Some(tab) = self.outline_panel_tabs.get_mut(index) {
                *tab = OutlinePanelTab::Workflow;
            }
            if let Some(workspace) = self.workspaces.get_mut(index) {
                workspace.center_mode = CenterMode::Editor;
            }
            self.ensure_agent_terminal_host(ctx, index, &AgentSlotId::Subagent(id));
            self.request_app_repaint();
            return;
        }
        let agent_kind = self
            .workspaces
            .get(index)
            .map(|workspace| workspace.agent_kind)
            .unwrap_or(self.default_agent_kind);
        self.pending_workflow_agent_binding = Some((index, binding.clone()));
        self.set_active_app_dialog(Some(AppDialog::AddSubagent {
            index,
            column_id: "main".to_string(),
            name: binding.label,
            agent_kind,
            agent_model: String::new(),
            agent_model_provider: String::new(),
            model_providers: data::load_codex_model_provider_names(),
            agent_effort: String::new(),
            agent_fast_mode: None,
            agent_work_dir: String::new(),
            helix_work_dir: String::new(),
            session_id: String::new(),
        }));
    }

    /// 创建不进入普通 Agent 网格的 Workflow Agent 并启动现有 terminal host。
    ///
    /// 适用场景：AddSubagent 面板带 pending Workflow binding 提交。例：Task -> task sidecar + host。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn add_workflow_agent(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        binding: WorkflowAgentBinding,
        name: String,
        agent_kind: AgentKind,
        agent_model: Option<String>,
        agent_model_provider: Option<String>,
        agent_effort: Option<String>,
        agent_fast_mode: Option<bool>,
        agent_work_dir: Option<PathBuf>,
        helix_work_dir: Option<PathBuf>,
        session_id: Option<String>,
    ) {
        let Some(workspace) = self.workspaces.get_mut(index) else {
            return;
        };
        if workspace
            .subagents
            .iter()
            .any(|agent| agent.workflow_target.as_ref() == Some(&binding.target))
        {
            return;
        }
        let mut agent = data::new_subagent(
            &workspace.path,
            name,
            agent_kind,
            agent_model,
            agent_model_provider,
            agent_effort,
            agent_fast_mode,
            agent_work_dir,
            helix_work_dir,
            session_id.clone(),
        );
        let agent_scope = crate::gui::workflow::workflow_agent_workspace_scope(&workspace.path);
        let id = crate::gui::workflow::workflow_agent_runtime_id(&agent_scope, &binding.target);
        agent.id = id.clone();
        agent.agent_id = id.clone();
        agent.workflow_target = Some(binding.target.clone());
        agent.workflow_sidecar_path = Some(binding.sidecar_path.clone());
        agent.bootstrap_pending = session_id.is_none();
        workspace.subagents.push(agent);
        if let Some(state) = self.workflow_states.get_mut(index) {
            state.selected = Some(binding.selection.clone());
            state.active_agent_target = Some(binding.target);
        }
        if let Some(tab) = self.outline_panel_tabs.get_mut(index) {
            *tab = OutlinePanelTab::Workflow;
        }
        if let Some(workspace) = self.workspaces.get_mut(index) {
            workspace.center_mode = CenterMode::Editor;
        }
        if let Some(states) = self.agent_busy_watchdogs.get_mut(index) {
            states.entry(AgentSlotId::Subagent(id.clone())).or_default();
        }
        self.request_workflow_agent_sidecar_save(index, binding.sidecar_path);
        self.ensure_agent_terminal_host(ctx, index, &AgentSlotId::Subagent(id));
        self.request_app_repaint();
    }

    /// 把指定 Workflow sidecar 的当前内存快照交给后台 writer。
    ///
    /// 适用场景：创建、session 更新、配置变更与销毁；AppEvent drain 不做 IO。
    /// 例：内存快照 -> atomic file。
    pub(super) fn request_workflow_agent_sidecar_save(
        &mut self,
        index: usize,
        sidecar_path: PathBuf,
    ) {
        let Some(workspace) = self.workspaces.get(index) else {
            return;
        };
        let key = (index, sidecar_path.clone());
        let workspace_path = workspace.path.clone();
        let agents = workspace.subagents.clone();
        let should_start = match self.workflow_agent_sidecar_saves.get_mut(&key) {
            Some(state) => {
                state.workspace_path = workspace_path;
                state.agents = agents;
                state.revision = state.revision.saturating_add(1);
                !state.in_flight
            }
            None => {
                self.workflow_agent_sidecar_saves.insert(
                    key,
                    WorkflowAgentSidecarSaveState {
                        workspace_path,
                        agents,
                        revision: 1,
                        in_flight: false,
                    },
                );
                true
            }
        };
        if should_start {
            self.start_workflow_agent_sidecar_save(index, sidecar_path);
        }
    }

    /// 启动一个 sidecar 当前最新快照的后台写入。
    ///
    /// 适用场景：首次 dirty 或上一代写完后仍有更新。例：revision 2 等 revision 1 完成后启动。
    fn start_workflow_agent_sidecar_save(&mut self, index: usize, sidecar_path: PathBuf) {
        let key = (index, sidecar_path.clone());
        let Some(state) = self.workflow_agent_sidecar_saves.get_mut(&key) else {
            return;
        };
        if state.in_flight {
            return;
        }
        state.in_flight = true;
        let workspace_path = state.workspace_path.clone();
        let agents = state.agents.clone();
        let revision = state.revision;
        let tx = self.app_event_tx.clone();
        let repaint_controller = self.repaint_controller.clone();
        let repaint_ctx = self.app_repaint_ctx.clone();
        self.background_runtime.spawn(async move {
            let task_sidecar = sidecar_path.clone();
            let result = tokio::task::spawn_blocking(move || {
                crate::gui::workflow::save_local_workflow_agent_sidecar(
                    &workspace_path,
                    &task_sidecar,
                    &agents,
                )
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(AppEvent::WorkflowAgentSidecarSaved {
                index,
                sidecar_path,
                revision,
                result,
            });
            if let Some(ctx) = repaint_ctx.as_ref() {
                repaint_controller.request_repaint(ctx);
            }
        });
    }

    /// 消费一次 sidecar 写入结果，并在期间又变脏时启动下一代。
    ///
    /// 适用场景：后台 writer AppEvent。例：revision 1 完成但 latest=3 -> 直接写 revision 3。
    pub(super) fn finish_workflow_agent_sidecar_save(
        &mut self,
        index: usize,
        sidecar_path: PathBuf,
        revision: u64,
    ) -> bool {
        let key = (index, sidecar_path.clone());
        let Some(state) = self.workflow_agent_sidecar_saves.get_mut(&key) else {
            return false;
        };
        state.in_flight = false;
        let restart = state.revision > revision;
        if restart {
            self.start_workflow_agent_sidecar_save(index, sidecar_path);
        } else {
            self.workflow_agent_sidecar_saves.remove(&key);
        }
        !restart
    }

    /// 返回一个运行时 Agent 槽位对应的 Workflow sidecar。
    ///
    /// 适用场景：复用普通 Agent 设置菜单后选择正确 persistence writer。例：普通 subagent -> None。
    pub(super) fn workflow_agent_sidecar_for_slot(
        &self,
        index: usize,
        slot: &AgentSlotId,
    ) -> Option<PathBuf> {
        let AgentSlotId::Subagent(id) = slot else {
            return None;
        };
        self.workspaces
            .get(index)?
            .subagents
            .iter()
            .find(|agent| agent.id == *id && agent.workflow_target.is_some())?
            .workflow_sidecar_path
            .clone()
    }

    /// 按 Agent 类型把配置变化写入普通 store 或 Workflow sidecar。
    ///
    /// 适用场景：Switch/Model/Effort/Workdir 等复用菜单。例：WF slot -> 只写分层 JSON。
    pub(super) fn persist_agent_slot_change(&mut self, index: usize, slot: &AgentSlotId) {
        if let Some(sidecar_path) = self.workflow_agent_sidecar_for_slot(index, slot) {
            self.request_workflow_agent_sidecar_save(index, sidecar_path);
        } else {
            self.persist_workspaces();
        }
    }

    /// 销毁一个 Workflow Agent：先断开 host，再删除关联记录并写回 sidecar。
    ///
    /// 适用场景：右键 Destroy Agent 与 Workflow 级联删除。例：Step Agent -> host exit + JSON record removed。
    pub(super) fn destroy_workflow_agent(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        target: &data::WorkflowAgentTarget,
    ) {
        self.destroy_workflow_agent_targets(ctx, index, &BTreeSet::from([target.clone()]));
    }

    /// 批量销毁目标集合，确保同一 sidecar 只写一次最终快照。
    ///
    /// 适用场景：删除 Project/Task 与 Merge Steps。例：三个 Step Agent -> 一次 task sidecar 写回。
    fn destroy_workflow_agent_targets(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        targets: &BTreeSet<data::WorkflowAgentTarget>,
    ) {
        if targets.is_empty() {
            return;
        }
        let removed = self
            .workspaces
            .get(index)
            .map(|workspace| {
                workspace
                    .subagents
                    .iter()
                    .filter(|agent| {
                        agent
                            .workflow_target
                            .as_ref()
                            .is_some_and(|target| targets.contains(target))
                    })
                    .map(|agent| {
                        (
                            agent.id.clone(),
                            agent.workflow_sidecar_path.clone(),
                            agent.workflow_target.clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let sidecars = removed
            .iter()
            .filter_map(|(_, path, _)| path.clone())
            .collect::<BTreeSet<_>>();
        for (id, _, _) in &removed {
            self.remove_subagent(ctx, index, id);
        }
        if let Some(state) = self.workflow_states.get_mut(index)
            && state
                .active_agent_target
                .as_ref()
                .is_some_and(|target| targets.contains(target))
        {
            state.active_agent_target = None;
        }
        for sidecar_path in sidecars {
            self.request_workflow_agent_sidecar_save(index, sidecar_path);
        }
    }

    /// 返回一次 Workflow mutation 成功后必须销毁的稳定 Agent 目标。
    ///
    /// 适用场景：文件修改成功后再清理关联，失败不误杀 Agent。例：Merge -> 所有参与 Step target。
    fn workflow_agent_targets_removed_by_mutation(
        &self,
        index: usize,
        request: &WorkflowMutationRequest,
    ) -> BTreeSet<data::WorkflowAgentTarget> {
        let Some(tree) = self
            .workflow_states
            .get(index)
            .and_then(|state| state.tree.as_ref())
        else {
            return BTreeSet::new();
        };
        let mut removed = BTreeSet::new();
        match request {
            WorkflowMutationRequest::DeleteProject { project_path } => {
                if let Some((owner, project)) = workflow_trees(tree).find_map(|owner| {
                    owner.projects.iter().find_map(|project| {
                        (project.root_path.parent() == Some(project_path.as_path()))
                            .then_some((owner, project))
                    })
                }) {
                    for agent in self
                        .workspaces
                        .get(index)
                        .into_iter()
                        .flat_map(|workspace| workspace.subagents.iter())
                    {
                        if let Some(target) = agent.workflow_target.as_ref()
                            && target.repo_path == owner.repo_path
                            && target.repo_id == owner.id
                            && target.project_id.as_deref() == Some(project.id.as_str())
                        {
                            removed.insert(target.clone());
                        }
                    }
                }
            }
            WorkflowMutationRequest::DeleteTask { task_path } => {
                if let Some((owner, project, task)) = workflow_trees(tree).find_map(|owner| {
                    owner.projects.iter().find_map(|project| {
                        project
                            .tasks
                            .iter()
                            .find(|task| task.path == *task_path)
                            .map(|task| (owner, project, task))
                    })
                }) {
                    for agent in self
                        .workspaces
                        .get(index)
                        .into_iter()
                        .flat_map(|workspace| workspace.subagents.iter())
                    {
                        if let Some(target) = agent.workflow_target.as_ref()
                            && target.repo_path == owner.repo_path
                            && target.repo_id == owner.id
                            && target.project_id.as_deref() == Some(project.id.as_str())
                            && target.task_id.as_deref() == Some(task.id.as_str())
                        {
                            removed.insert(target.clone());
                        }
                    }
                }
            }
            WorkflowMutationRequest::DeleteStep {
                task_path,
                step_path,
            } => {
                let selection = WorkflowSelectionTarget::Step {
                    task_path: task_path.clone(),
                    step_path: step_path.clone(),
                };
                if let Some(binding) =
                    crate::gui::workflow::workflow_agent_binding(tree, &selection)
                {
                    removed.insert(binding.target);
                }
            }
            WorkflowMutationRequest::MergeSteps {
                task_path,
                step_paths,
                ..
            } => {
                for step_path in step_paths {
                    let selection = WorkflowSelectionTarget::Step {
                        task_path: task_path.clone(),
                        step_path: step_path.clone(),
                    };
                    if let Some(binding) =
                        crate::gui::workflow::workflow_agent_binding(tree, &selection)
                    {
                        removed.insert(binding.target);
                    }
                }
            }
            _ => {}
        }
        removed
    }

    /// 判断 workflow 后台结果是否仍属于当前 workspace。
    ///
    /// 适用场景：切换 workspace 目录后丢弃旧结果。例：`same path -> true`。
    fn workflow_request_identity_matches(&self, index: usize, workspace_path: &Path) -> bool {
        self.workspaces
            .get(index)
            .is_some_and(|workspace| workspace.path == workspace_path)
    }

    /// 切换左侧面板 tab，并在进入 workflow 时派发加载任务。
    pub(super) fn set_outline_panel_tab(&mut self, ctx: &egui::Context, tab: OutlinePanelTab) {
        if !self.set_outline_panel_tab_only(ctx, tab) {
            return;
        }
        match tab {
            OutlinePanelTab::Outline => self.route_outline_tab_to_agent(),
            OutlinePanelTab::Workflow => self.route_workflow_tab_to_task_surface(ctx),
        }
    }

    /// 只切换左侧面板 tab，不改变右侧 surface。
    fn set_outline_panel_tab_only(&mut self, ctx: &egui::Context, tab: OutlinePanelTab) -> bool {
        let Some(current_tab) = self.outline_panel_tabs.get_mut(self.active_workspace) else {
            return false;
        };
        if *current_tab == tab {
            return false;
        }
        *current_tab = tab;
        if tab == OutlinePanelTab::Workflow {
            self.request_workflow_tree_refresh(ctx, self.active_workspace);
        }
        self.request_app_repaint();
        true
    }

    /// 在可见侧栏中切换 tab；侧栏折叠时改为打开 workflow modal。
    ///
    /// 适用场景：普通状态 `Alt/Cmd+Z` 根据本地 Outline 状态选择 surface。
    /// 例：`outline collapsed -> toggle WorkflowQuickModal`。
    pub(super) fn toggle_outline_workflow_tab(&mut self, ctx: &egui::Context) {
        if self.current_workspace().is_some_and(|workspace| {
            workspace.route == Route::Workspace && workspace.outline_panel_collapsed
        }) {
            self.toggle_workflow_quick_modal(ctx);
            return;
        }
        let Some(current_tab) = self.outline_panel_tabs.get(self.active_workspace).copied() else {
            return;
        };
        let next_tab = match current_tab {
            OutlinePanelTab::Outline => OutlinePanelTab::Workflow,
            OutlinePanelTab::Workflow => OutlinePanelTab::Outline,
        };
        self.set_outline_panel_tab(ctx, next_tab);
    }

    /// Toggles the fullscreen workflow quick modal without changing the main center mode.
    ///
    /// Example: fullscreen `Alt+Z` -> open modal and restore the last task selection.
    pub(super) fn toggle_workflow_quick_modal(&mut self, ctx: &egui::Context) {
        if matches!(
            self.active_app_dialog(),
            Some(AppDialog::WorkflowQuickModal)
        ) {
            self.set_active_app_dialog(None);
            return;
        }
        self.request_workflow_tree_refresh(ctx, self.active_workspace);
        if let Some(target) = self.workflow_task_surface_target_to_restore() {
            self.request_workflow_quick_target(ctx, target);
        } else if let Some(state) = self.workflow_states.get_mut(self.active_workspace) {
            state.pending_task_restore_after_load = Some(WorkflowTargetOpenMode::QuickModal);
            self.set_active_app_dialog(Some(AppDialog::WorkflowQuickModal));
        }
    }

    /// 切到普通 outline 时显示 Agent surface。
    fn route_outline_tab_to_agent(&mut self) {
        let Some(workspace) = self.current_workspace_mut() else {
            return;
        };
        if workspace.route != Route::Workspace {
            return;
        }
        workspace.center_mode = CenterMode::Agent;
        workspace.previous_center_mode = CenterMode::Agent;
        self.persist_workspaces();
    }

    /// 切到 workflow 时恢复最近 task 并显示 task 工作台。
    fn route_workflow_tab_to_task_surface(&mut self, ctx: &egui::Context) {
        let Some(workspace) = self.current_workspace_mut() else {
            return;
        };
        if workspace.route != Route::Workspace {
            return;
        }
        workspace.center_mode = CenterMode::Editor;
        workspace.previous_center_mode = CenterMode::Editor;
        self.persist_workspaces();
        if let Some(target) = self.workflow_task_surface_target_to_restore() {
            self.request_workflow_target(ctx, target);
        } else if let Some(state) = self.workflow_states.get_mut(self.active_workspace) {
            state.pending_task_restore_after_load = Some(WorkflowTargetOpenMode::Workspace);
        }
    }

    /// 返回切回 workflow 时要恢复的 task 工作台目标。
    fn workflow_task_surface_target_to_restore(&self) -> Option<WorkflowSelectionTarget> {
        self.workflow_task_surface_target_to_restore_for(self.active_workspace)
    }

    /// 返回指定 workspace 切回 workflow 时要恢复的 task 工作台目标。
    fn workflow_task_surface_target_to_restore_for(
        &self,
        index: usize,
    ) -> Option<WorkflowSelectionTarget> {
        let state = self.workflow_states.get(index)?;
        state
            .last_task_surface_target
            .clone()
            .or_else(|| first_workflow_task_target(state.tree.as_ref()?))
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
        if !self.workflow_copy_context_active() {
            return None;
        }
        let state = self.workflow_states.get(self.active_workspace)?;
        let selected = state.selected.as_ref()?;
        let tree = state.tree.as_ref()?;
        match selected {
            WorkflowSelectionTarget::WorkspaceRoot { root_path } => workflow_trees(tree)
                .find(|tree| tree.root_path == *root_path)
                .map(|tree| {
                    let mut parts = workflow_repo_copy_parts(&tree.repo_path);
                    parts.push("root.md".to_string());
                    workflow_copy_path_from_parts(&parts)
                }),
            WorkflowSelectionTarget::Project { root_path } => {
                workflow_trees(tree).find_map(|tree| {
                    tree.projects
                        .iter()
                        .find(|project| project.root_path == *root_path)
                        .map(|project| {
                            let mut parts = workflow_repo_copy_parts(&tree.repo_path);
                            parts.push(project.label.clone());
                            workflow_copy_path_from_parts(&parts)
                        })
                })
            }
            WorkflowSelectionTarget::Task { task_path } => {
                workflow_task_copy_path_from_tree(tree, task_path)
            }
            WorkflowSelectionTarget::Step {
                task_path,
                step_path,
            } => workflow_step_copy_path_from_tree(tree, task_path, step_path),
        }
    }

    /// 判断当前 UI 是否允许复制 workflow selection 路径。
    fn workflow_copy_context_active(&self) -> bool {
        matches!(
            self.active_app_dialog(),
            Some(AppDialog::WorkflowQuickModal)
        ) || self
            .outline_panel_tabs
            .get(self.active_workspace)
            .is_some_and(|tab| *tab == OutlinePanelTab::Workflow)
    }

    /// 请求刷新指定 workspace 的 workflow tree。
    pub(super) fn request_workflow_tree_refresh(&mut self, ctx: &egui::Context, index: usize) {
        let scan_sub_workflows = self
            .workflow_states
            .get(index)
            .is_some_and(|state| !state.sub_workflows_scanned);
        self.request_workflow_tree_refresh_with_scan(ctx, index, scan_sub_workflows);
    }

    /// 手动重新扫描指定 workspace 的子 repo workflow。
    ///
    /// 适用场景：Work-flow tab 右键菜单显式发现新增 repo。
    /// 例：运行期间新增 `services/api/.git` -> 扫描后加入 tree。
    pub(super) fn request_sub_workflow_scan(&mut self, ctx: &egui::Context, index: usize) {
        self.request_workflow_tree_refresh_with_scan(ctx, index, true);
    }

    /// 按指定扫描策略请求 workflow tree 刷新。
    ///
    /// 适用场景：首次/手动扫描发现 repo，普通 watcher 刷新复用缓存。
    /// 例：`scan=false + cached=[api] -> 只加载主 repo 和 api`。
    fn request_workflow_tree_refresh_with_scan(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        scan_sub_workflows: bool,
    ) {
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
        let known_repo_paths = state.sub_workflow_repo_paths.clone();
        self.spawn_workflow_tree_load_task(
            ctx,
            index,
            workspace.path.clone(),
            scan_sub_workflows,
            known_repo_paths,
        );
    }

    /// 后台加载 workflow tree。
    fn spawn_workflow_tree_load_task(
        &self,
        ctx: &egui::Context,
        index: usize,
        workspace_path: PathBuf,
        scan_sub_workflows: bool,
        known_repo_paths: Vec<PathBuf>,
    ) {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let event_workspace_path = workspace_path.clone();
            let result = tokio::task::spawn_blocking(move || {
                crate::gui::workflow::load_workflow_tree_with_sub_workflows(
                    &workspace_path,
                    scan_sub_workflows,
                    &known_repo_paths,
                )
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(AppEvent::WorkflowTreeLoaded {
                index,
                workspace_path: event_workspace_path,
                scanned_sub_workflows: scan_sub_workflows,
                result,
            });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// 应用 workflow tree 加载结果。
    pub(super) fn apply_workflow_tree_loaded(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        workspace_path: PathBuf,
        scanned_sub_workflows: bool,
        result: Result<WorkflowTree, String>,
    ) {
        if !self.workflow_request_identity_matches(index, &workspace_path) {
            return;
        }
        let result = result.map(|tree| {
            self.merge_loaded_workflow_agents(index, &tree);
            tree
        });
        let Some(state) = self.workflow_states.get_mut(index) else {
            return;
        };
        state.loading = false;
        let mut pending_target = None;
        let mut pending_target_mode = WorkflowTargetOpenMode::Workspace;
        match result {
            Ok(tree) => {
                let project_keys: BTreeSet<String> = workflow_trees(&tree)
                    .flat_map(|tree| {
                        tree.projects
                            .iter()
                            .map(|project| workflow_project_state_key(tree, &project.key))
                    })
                    .collect();
                state.sub_workflow_repo_paths = tree
                    .sub_workflows
                    .iter()
                    .map(|tree| tree.repo_path.clone())
                    .collect();
                if scanned_sub_workflows {
                    state.sub_workflows_scanned = true;
                }
                let pending_task_restore = state.pending_task_restore_after_load.take();
                state
                    .collapsed_project_keys
                    .retain(|key| project_keys.contains(key));
                state.tree = Some(tree);
                state.load_error = None;
                pending_target = state.pending_target_after_save.take();
                if pending_target.is_some() {
                    pending_target_mode = state
                        .pending_target_after_save_mode
                        .take()
                        .unwrap_or(WorkflowTargetOpenMode::Workspace);
                }
                if pending_target.is_none()
                    && let Some(open_mode) = pending_task_restore
                {
                    pending_target = self.workflow_task_surface_target_to_restore_for(index);
                    pending_target_mode = open_mode;
                }
            }
            Err(error) => {
                state.tree = None;
                state.load_error = Some(error);
            }
        }
        if let Some(target) = pending_target {
            self.open_workflow_target_now_with_mode(ctx, target, pending_target_mode);
        }
        self.sync_fs_watches();
    }

    /// 用 sidecar 快照替换当前 workspace 的 Workflow Agent，同时保留运行态 activity。
    ///
    /// 适用场景：Workflow 刷新或 Remote 重载。例：普通 subagent + 2 个 WF Agent -> 后两者按 target 更新。
    fn merge_loaded_workflow_agents(&mut self, index: usize, tree: &WorkflowTree) {
        let loaded = workflow_trees(tree)
            .flat_map(|tree| tree.agents.iter().cloned())
            .collect::<Vec<_>>();
        let Some(workspace) = self.workspaces.get_mut(index) else {
            return;
        };
        let existing = workspace
            .subagents
            .iter()
            .filter_map(|agent| Some((agent.workflow_target.clone()?, agent.clone())))
            .collect::<BTreeMap<_, _>>();
        workspace
            .subagents
            .retain(|agent| agent.workflow_target.is_none());
        for mut agent in loaded {
            if let Some(existing) = agent
                .workflow_target
                .as_ref()
                .and_then(|target| existing.get(target))
            {
                let sidecar_path = agent.workflow_sidecar_path.clone();
                agent = existing.clone();
                agent.workflow_sidecar_path = sidecar_path;
            }
            if let Some(states) = self.agent_busy_watchdogs.get_mut(index) {
                states
                    .entry(AgentSlotId::Subagent(agent.id.clone()))
                    .or_default();
            }
            workspace.subagents.push(agent);
        }
    }

    /// 请求打开 workflow 目标，必要时先弹出未保存片段确认。
    pub(super) fn request_workflow_target(
        &mut self,
        ctx: &egui::Context,
        target: WorkflowSelectionTarget,
    ) {
        self.request_workflow_target_with_mode(ctx, target, WorkflowTargetOpenMode::Workspace);
    }

    /// 请求在 quick modal 内打开 workflow 目标，不改背后的 workspace surface。
    pub(super) fn request_workflow_quick_target(
        &mut self,
        ctx: &egui::Context,
        target: WorkflowSelectionTarget,
    ) {
        self.request_workflow_target_with_mode(ctx, target, WorkflowTargetOpenMode::QuickModal);
    }

    /// 按指定 UI 影响范围请求打开 workflow 目标。
    fn request_workflow_target_with_mode(
        &mut self,
        ctx: &egui::Context,
        target: WorkflowSelectionTarget,
        open_mode: WorkflowTargetOpenMode,
    ) {
        if self.workflow_switch_requires_save(&target) {
            if open_mode == WorkflowTargetOpenMode::QuickModal {
                self.set_active_app_dialog(Some(AppDialog::WorkflowQuickModal));
                self.set_workflow_quick_overlay_dialog(Some(AppDialog::WorkflowUnsavedSwitch {
                    target,
                    open_mode,
                }));
                return;
            }
            self.set_active_app_dialog(Some(AppDialog::WorkflowUnsavedSwitch {
                target,
                open_mode,
            }));
            return;
        }
        self.open_workflow_target_now_with_mode(ctx, target, open_mode);
    }

    /// 不再询问未保存状态，直接打开 workflow 目标。
    pub(super) fn open_workflow_target_now(
        &mut self,
        ctx: &egui::Context,
        target: WorkflowSelectionTarget,
    ) {
        self.open_workflow_target_now_with_mode(ctx, target, WorkflowTargetOpenMode::Workspace);
    }

    /// 按指定 UI 影响范围直接打开 workflow 目标。
    pub(super) fn open_workflow_target_now_with_mode(
        &mut self,
        ctx: &egui::Context,
        target: WorkflowSelectionTarget,
        open_mode: WorkflowTargetOpenMode,
    ) {
        if open_mode == WorkflowTargetOpenMode::QuickModal {
            self.set_active_app_dialog(Some(AppDialog::WorkflowQuickModal));
        }
        match target.clone() {
            WorkflowSelectionTarget::WorkspaceRoot { root_path } => {
                if let Some(state) = self.workflow_states.get_mut(self.active_workspace) {
                    state.selected = Some(target);
                    state.task_editor = None;
                    state.editor = None;
                    clear_workflow_step_selection(state);
                }
                if open_mode == WorkflowTargetOpenMode::Workspace {
                    self.request_open_file(root_path);
                    if let Some(workspace) = self.current_workspace_mut() {
                        workspace.center_mode = CenterMode::Editor;
                    }
                    self.persist_workspaces();
                } else {
                    self.request_app_repaint();
                }
            }
            WorkflowSelectionTarget::Project { root_path } => {
                if let Some(state) = self.workflow_states.get_mut(self.active_workspace) {
                    state.selected = Some(target);
                    state.task_editor = None;
                    state.editor = None;
                    clear_workflow_step_selection(state);
                }
                if open_mode == WorkflowTargetOpenMode::Workspace {
                    self.request_open_file(root_path);
                    if let Some(workspace) = self.current_workspace_mut() {
                        workspace.center_mode = CenterMode::Editor;
                    }
                    self.persist_workspaces();
                } else {
                    self.request_app_repaint();
                }
            }
            WorkflowSelectionTarget::Task { task_path } => {
                let task_editor = self.reusable_or_fresh_workflow_task_editor(&task_path);
                if let Some(state) = self.workflow_states.get_mut(self.active_workspace) {
                    state.selected = Some(target);
                    state.last_task_surface_target = Some(WorkflowSelectionTarget::Task {
                        task_path: task_path.clone(),
                    });
                    state.task_editor = task_editor;
                    state.editor = None;
                    clear_workflow_step_selection(state);
                }
                if open_mode == WorkflowTargetOpenMode::Workspace
                    && let Some(workspace) = self.current_workspace_mut()
                {
                    workspace.center_mode = CenterMode::Editor;
                }
                self.request_app_repaint();
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
                    state.last_task_surface_target = Some(WorkflowSelectionTarget::Step {
                        task_path: task_path.clone(),
                        step_path: step_path.clone(),
                    });
                    state.task_editor = task_editor;
                    state.editor = Some(editor);
                    set_single_workflow_step_selection(state, &task_path, &step_path);
                }
                if open_mode == WorkflowTargetOpenMode::Workspace
                    && let Some(workspace) = self.current_workspace_mut()
                {
                    workspace.center_mode = CenterMode::Editor;
                }
                self.request_app_repaint();
            }
        }
        self.sync_fs_watches();
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
        workflow_tasks(tree)
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
        for task in workflow_tasks(tree) {
            if task.path == task_path
                && let Some(node) = workflow_step_node_at_path(&task.steps, step_path)
            {
                return Some(workflow_step_editor_from_node(task_path, node));
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
        self.save_active_workflow_step_with_mode(
            ctx,
            pending_target,
            WorkflowTargetOpenMode::Workspace,
        );
    }

    /// 保存当前 workflow step，并按指定 UI 范围打开后续目标。
    pub(super) fn save_active_workflow_step_with_mode(
        &mut self,
        ctx: &egui::Context,
        pending_target: Option<WorkflowSelectionTarget>,
        pending_target_mode: WorkflowTargetOpenMode,
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
        state.pending_target_after_save_mode = state
            .pending_target_after_save
            .as_ref()
            .map(|_| pending_target_mode);
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
        let repaint_controller = self.repaint_controller.clone();
        let target = selected.unwrap_or_else(|| WorkflowSelectionTarget::Task {
            task_path: request.task_path.clone(),
        });
        self.background_runtime.spawn(async move {
            let task_workspace_path = workspace_path.clone();
            let result = tokio::task::spawn_blocking(move || {
                crate::gui::workflow::save_workflow_step_editor(&task_workspace_path, request)
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(AppEvent::WorkflowStepSaved {
                index,
                workspace_path,
                target,
                result,
            });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// 应用 workflow 片段保存结果。
    pub(super) fn apply_workflow_step_saved(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        workspace_path: PathBuf,
        target: WorkflowSelectionTarget,
        result: Result<WorkflowSaveSuccess, String>,
    ) {
        if !self.workflow_request_identity_matches(index, &workspace_path) {
            return;
        }
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
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let request_for_task = request.clone();
            let task_workspace_path = workspace_path.clone();
            let result = tokio::task::spawn_blocking(move || {
                crate::gui::workflow::apply_workflow_mutation(
                    &task_workspace_path,
                    request_for_task,
                )
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(AppEvent::WorkflowMutationFinished {
                index,
                workspace_path,
                request,
                result,
            });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// 应用 workflow tree 文件修改完成后的 UI 状态变化。
    pub(super) fn apply_workflow_mutation_finished(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        workspace_path: PathBuf,
        request: WorkflowMutationRequest,
        result: Result<(), String>,
    ) {
        if !self.workflow_request_identity_matches(index, &workspace_path) {
            return;
        }
        match result {
            Ok(()) => {
                let removed_targets =
                    self.workflow_agent_targets_removed_by_mutation(index, &request);
                self.destroy_workflow_agent_targets(ctx, index, &removed_targets);
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
                project_path,
                new_key,
            } => {
                let old_prefix = project_path.clone();
                let project_id = project_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| {
                        crate::gui::workflow::split_stable_name(
                            name,
                            crate::gui::workflow::NODE_ID_LEN,
                        )
                        .map(|(id, _)| id.to_string())
                    })
                    .unwrap_or_else(|| "project".to_string());
                let new_prefix = project_path
                    .parent()
                    .map(|parent| parent.join(format!("{project_id}.{new_key}")))
                    .unwrap_or_else(|| PathBuf::from(format!("{project_id}.{new_key}")));
                self.rename_workflow_document_prefix(index, &old_prefix, &new_prefix);
                self.rename_workflow_agent_sidecars(index, &old_prefix, &new_prefix);
                if let Some(state) = self.workflow_states.get_mut(index) {
                    let old_state_key = workflow_project_state_key_from_path(project_path);
                    let new_state_key = workflow_project_state_key_from_path(&new_prefix);
                    if let (Some(old_state_key), Some(new_state_key)) =
                        (old_state_key, new_state_key)
                        && state.collapsed_project_keys.remove(&old_state_key)
                    {
                        state.collapsed_project_keys.insert(new_state_key);
                    }
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
            WorkflowMutationRequest::DeleteProject { project_path } => {
                self.clear_workflow_document_under_prefix(index, project_path);
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
            WorkflowMutationRequest::MergeSteps {
                task_path,
                step_paths,
                ..
            } => {
                self.clear_after_workflow_step_merge(index, task_path, step_paths);
            }
            WorkflowMutationRequest::AddStep { task_path, .. } => {
                self.select_newly_added_workflow_step_after_reload(index, task_path);
            }
            WorkflowMutationRequest::InitRoot
            | WorkflowMutationRequest::AddProject { .. }
            | WorkflowMutationRequest::AddTask { .. } => {}
        }
    }

    /// 在 AddStep 刷新 tree 后自动打开新追加的 step。
    fn select_newly_added_workflow_step_after_reload(&mut self, index: usize, task_path: &Path) {
        let Some(state) = self.workflow_states.get_mut(index) else {
            return;
        };
        let Some(tree) = state.tree.as_ref() else {
            return;
        };
        let Some(task) = workflow_tasks(tree).find(|task| task.path == task_path) else {
            return;
        };
        state.pending_target_after_save = Some(WorkflowSelectionTarget::Step {
            task_path: task_path.to_path_buf(),
            step_path: vec![task.steps.len()],
        });
        state.pending_target_after_save_mode = Some(WorkflowTargetOpenMode::Workspace);
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

    /// 重写 Project 重命名后 Workflow Agent 的 sidecar 路径并写回最新快照。
    ///
    /// 适用场景：目录已在后台完成 rename；旧 writer 不得重建旧目录。例：`a.old -> a.new`。
    fn rename_workflow_agent_sidecars(
        &mut self,
        index: usize,
        old_prefix: &Path,
        new_prefix: &Path,
    ) {
        let mut sidecars = BTreeSet::new();
        if let Some(workspace) = self.workspaces.get_mut(index) {
            for agent in workspace
                .subagents
                .iter_mut()
                .filter(|agent| agent.workflow_target.is_some())
            {
                let Some(path) = agent.workflow_sidecar_path.as_ref() else {
                    continue;
                };
                let Some(next) = rewritten_path_prefix(path, old_prefix, new_prefix) else {
                    continue;
                };
                agent.workflow_sidecar_path = Some(next.clone());
                sidecars.insert(next);
            }
        }
        self.workflow_agent_sidecar_saves
            .retain(|(workspace_index, path), _| {
                *workspace_index != index || !path.starts_with(old_prefix)
            });
        for sidecar in sidecars {
            self.request_workflow_agent_sidecar_save(index, sidecar);
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
        if let Some(state) = self.workflow_states.get_mut(index) {
            let selected_deleted = state.selected.as_ref().is_some_and(|selected| {
                matches!(
                    selected,
                    WorkflowSelectionTarget::Step {
                        task_path: selected_task_path,
                        step_path: selected_step_path,
                    } if selected_task_path == task_path && selected_step_path.starts_with(step_path)
                )
            });
            let editor_deleted = state.editor.as_ref().is_some_and(|editor| {
                editor.task_path == task_path && editor.step_path.starts_with(step_path)
            });
            if editor_deleted {
                state.editor = None;
            }
            if selected_deleted {
                // 触发条件：删除当前选中的 step 或其父 step。
                // 不能清空 selection：center mode 仍是 Editor，普通 Markdown
                // surface 会接管并打开上一个 selected_file。
                // 保持 task 选中，防止删除 step 后跳到 AGENTS.md 等旧文档。
                state.selected = Some(WorkflowSelectionTarget::Task {
                    task_path: task_path.to_path_buf(),
                });
            }
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
                Some(WorkflowSelectionTarget::WorkspaceRoot { .. })
                | Some(WorkflowSelectionTarget::Project { .. })
                | None => false,
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
            WorkflowMutationRequest::AddProject {
                spec_path,
                project_key,
            } => {
                if let Err(error) = crate::gui::workflow::validate_workflow_key(project_key) {
                    return Some(error);
                }
                self.workflow_project_duplicate_error(spec_path, "", project_key)
            }
            WorkflowMutationRequest::AddTask {
                spec_path,
                project_key,
                task_key,
            } => {
                if let Err(error) = crate::gui::workflow::validate_workflow_key(task_key) {
                    return Some(error);
                }
                let project_path = spec_path.join("ps").join(project_key);
                self.workflow_task_duplicate_error(&project_path, task_key, None)
            }
            WorkflowMutationRequest::AddStep { task_path, key, .. } => {
                let key = match crate::gui::workflow::validate_workflow_step_title(key) {
                    Ok(key) => key,
                    Err(error) => return Some(error),
                };
                self.workflow_step_duplicate_error(task_path, key, None)
            }
            WorkflowMutationRequest::RenameProject {
                project_path,
                new_key,
            } => {
                if let Err(error) = crate::gui::workflow::validate_workflow_key(new_key) {
                    return Some(error);
                }
                let project_key = project_path.file_name()?.to_string_lossy();
                let current_label = crate::gui::workflow::split_stable_name(
                    &project_key,
                    crate::gui::workflow::NODE_ID_LEN,
                )
                .map(|(_, label)| label)
                .unwrap_or(&project_key);
                if current_label == new_key.as_str() {
                    return None;
                }
                let spec_path = project_path.parent()?.parent()?;
                self.workflow_project_duplicate_error(spec_path, &project_key, new_key)
            }
            WorkflowMutationRequest::RenameTask { task_path, new_key } => {
                if let Err(error) = crate::gui::workflow::validate_workflow_key(new_key) {
                    return Some(error);
                }
                let current_key = self.workflow_task_key_for_path(task_path)?;
                if current_key == *new_key {
                    return None;
                }
                let project_path = task_path.parent()?;
                self.workflow_task_duplicate_error(project_path, new_key, Some(task_path))
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
            WorkflowMutationRequest::MergeSteps {
                task_path,
                step_paths,
                title,
            } => {
                let title = match crate::gui::workflow::validate_workflow_step_title(title) {
                    Ok(title) => title,
                    Err(error) => return Some(error),
                };
                if step_paths.len() < 2 {
                    return Some("Select at least two steps to merge".to_string());
                }
                self.workflow_step_duplicate_error_for_merge(task_path, title, step_paths)
            }
            _ => None,
        }
    }

    /// 检查 project key 是否和现有 project 重复。
    fn workflow_project_duplicate_error(
        &self,
        spec_path: &Path,
        current_key: &str,
        key: &str,
    ) -> Option<String> {
        let tree = self
            .workflow_states
            .get(self.active_workspace)?
            .tree
            .as_ref()?;
        workflow_trees(tree)
            .find(|tree| tree.spec_path == spec_path)?
            .projects
            .iter()
            .any(|project| project.key != current_key && project.label == key)
            .then(|| format!("project already exists: {key}"))
    }

    /// 检查 task 是否和同 project 现有 task key 重复。
    fn workflow_task_duplicate_error(
        &self,
        project_path: &Path,
        key: &str,
        exclude_task_path: Option<&Path>,
    ) -> Option<String> {
        let tree = self
            .workflow_states
            .get(self.active_workspace)?
            .tree
            .as_ref()?;
        let project = workflow_projects(tree)
            .find(|project| project.root_path.parent() == Some(project_path))?;
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
        let task = workflow_tasks(tree).find(|task| task.path == task_path)?;
        task.steps
            .iter()
            .any(|step| exclude_step_path != Some(step.path.as_slice()) && step.title == key)
            .then(|| format!("step already exists at this level: {key}"))
    }

    /// 合并 step 时只检查未参与合并的同名 step。
    fn workflow_step_duplicate_error_for_merge(
        &self,
        task_path: &Path,
        key: &str,
        merged_step_paths: &[Vec<usize>],
    ) -> Option<String> {
        let tree = self
            .workflow_states
            .get(self.active_workspace)?
            .tree
            .as_ref()?;
        let task = workflow_tasks(tree).find(|task| task.path == task_path)?;
        task.steps
            .iter()
            .any(|step| !merged_step_paths.contains(&step.path) && step.title == key)
            .then(|| format!("step already exists at this level: {key}"))
    }

    /// 返回指定 task path 当前对应的 task key。
    fn workflow_task_key_for_path(&self, task_path: &Path) -> Option<String> {
        let tree = self
            .workflow_states
            .get(self.active_workspace)?
            .tree
            .as_ref()?;
        workflow_tasks(tree)
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
        let task = workflow_tasks(tree).find(|task| task.path == task_path)?;
        workflow_step_node_at_path(&task.steps, step_path).map(|step| step.title.clone())
    }

    /// 清理合并 step 后失效的编辑器和多选状态。
    fn clear_after_workflow_step_merge(
        &mut self,
        index: usize,
        task_path: &Path,
        step_paths: &[Vec<usize>],
    ) {
        if let Some(state) = self.workflow_states.get_mut(index) {
            let selected_merged = state.selected.as_ref().is_some_and(|selected| {
                matches!(
                    selected,
                    WorkflowSelectionTarget::Step {
                        task_path: selected_task_path,
                        step_path,
                    } if selected_task_path == task_path && step_paths.contains(step_path)
                )
            });
            let editor_merged = state.editor.as_ref().is_some_and(|editor| {
                editor.task_path == task_path && step_paths.contains(&editor.step_path)
            });
            if selected_merged {
                state.selected = Some(WorkflowSelectionTarget::Task {
                    task_path: task_path.to_path_buf(),
                });
            }
            if editor_merged {
                state.editor = None;
            }
            clear_workflow_step_selection(state);
        }
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
        WorkflowSelectionTarget::WorkspaceRoot { .. } | WorkflowSelectionTarget::Project { .. } => {
            None
        }
    }
}

/// 从 workflow tree 里生成 task 的复制路径。
fn workflow_task_copy_path_from_tree(tree: &WorkflowTree, task_path: &Path) -> Option<String> {
    workflow_trees(tree).find_map(|tree| {
        tree.projects.iter().find_map(|project| {
            project
                .tasks
                .iter()
                .find(|task| task.path == task_path)
                .map(|task| {
                    let mut parts = workflow_repo_copy_parts(&tree.repo_path);
                    parts.push(project.label.clone());
                    parts.push(task.label.clone());
                    workflow_copy_path_from_parts(&parts)
                })
        })
    })
}

/// 从 workflow tree 里生成 step 的复制路径。
fn workflow_step_copy_path_from_tree(
    tree: &WorkflowTree,
    task_path: &Path,
    step_path: &[usize],
) -> Option<String> {
    workflow_trees(tree).find_map(|tree| {
        tree.projects.iter().find_map(|project| {
            project.tasks.iter().find_map(|task| {
                if task.path != task_path {
                    return None;
                }
                let mut parts = workflow_repo_copy_parts(&tree.repo_path);
                parts.push(project.label.clone());
                parts.push(task.label.clone());
                parts.extend(workflow_step_titles_at_path(&task.steps, step_path)?);
                Some(workflow_copy_path_from_parts(&parts))
            })
        })
    })
}

/// 返回 repo 相对路径对应的复制前缀。
///
/// 适用场景：子 repo 的 task/step 复制必须带 workspace 相对 repo 路径。
/// 例：`services/api -> ["services/api"]`。
fn workflow_repo_copy_parts(repo_path: &Path) -> Vec<String> {
    (!repo_path.as_os_str().is_empty())
        .then(|| repo_path.to_string_lossy().replace('\\', "/"))
        .into_iter()
        .collect()
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

/// 从稳定 Agent 目标恢复不含文件名的 Workflow 层级路径。
///
/// 适用场景：Agent 首次提示词不能依赖当前 UI 选择。例：Step -> `Root > project > task > step`。
fn workflow_agent_logical_path(
    tree: &WorkflowTree,
    target: &data::WorkflowAgentTarget,
) -> Option<String> {
    for owner in workflow_trees(tree) {
        if owner.repo_path != target.repo_path || owner.id != target.repo_id {
            continue;
        }
        let mut parts = vec![if owner.repo_path.as_os_str().is_empty() {
            "Root".to_string()
        } else {
            owner.repo_path.to_string_lossy().replace('\\', "/")
        }];
        if target.kind == data::WorkflowAgentTargetKind::Repo {
            return Some(workflow_copy_path_from_parts(&parts));
        }
        let project_id = target.project_id.as_deref()?;
        let project = owner
            .projects
            .iter()
            .find(|project| project.id == project_id)?;
        parts.push(project.label.clone());
        if target.kind == data::WorkflowAgentTargetKind::Project {
            return Some(workflow_copy_path_from_parts(&parts));
        }
        let task_id = target.task_id.as_deref()?;
        let task = project.tasks.iter().find(|task| task.id == task_id)?;
        parts.push(task.label.clone());
        if target.kind == data::WorkflowAgentTargetKind::Task {
            return Some(workflow_copy_path_from_parts(&parts));
        }
        let step_id = target.step_id.as_deref()?;
        parts.extend(workflow_step_titles_for_id(&task.steps, step_id)?);
        return Some(workflow_copy_path_from_parts(&parts));
    }
    None
}

/// 返回目标 Step 从顶层到自身的标题链。
///
/// 适用场景：Step Agent 只持久化稳定 ID。例：父子 Step -> `[父, 子]`。
fn workflow_step_titles_for_id(nodes: &[WorkflowStepNode], step_id: &str) -> Option<Vec<String>> {
    for node in nodes {
        if node.id == step_id {
            return Some(vec![node.title.clone()]);
        }
        if let Some(mut titles) = workflow_step_titles_for_id(&node.children, step_id) {
            titles.insert(0, node.title.clone());
            return Some(titles);
        }
    }
    None
}

/// 从 task 节点提取新增弹窗使用的 task key。
fn workflow_task_key(task: &WorkflowTaskNode) -> String {
    task.label.clone()
}

/// 计算 task 重命名后的 workspace 相对路径。
fn workflow_task_path_after_rename(task_path: &Path, new_key: &str) -> PathBuf {
    let task_id = task_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| {
            crate::gui::workflow::split_stable_name(stem, crate::gui::workflow::NODE_ID_LEN)
                .map(|(id, _)| id.to_string())
        })
        .unwrap_or_else(|| "task".to_string());
    task_path
        .parent()
        .map(|parent| parent.join(format!("{task_id}.task-{new_key}.md")))
        .unwrap_or_else(|| PathBuf::from(format!("{task_id}.task-{new_key}.md")))
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
        Some(
            WorkflowSelectionTarget::WorkspaceRoot { root_path }
            | WorkflowSelectionTarget::Project { root_path },
        ) => {
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

/// 清空 workflow step 多选状态。
pub(super) fn clear_workflow_step_selection(state: &mut WorkflowUiState) {
    state.selected_step_paths.clear();
    state.step_selection_anchor = None;
    state.step_selection_task_path = None;
}

/// 将 workflow step 多选状态设置成单个 step。
pub(super) fn set_single_workflow_step_selection(
    state: &mut WorkflowUiState,
    task_path: &Path,
    step_path: &[usize],
) {
    state.selected_step_paths.clear();
    state.selected_step_paths.insert(step_path.to_vec());
    state.step_selection_anchor = Some(step_path.to_vec());
    state.step_selection_task_path = Some(task_path.to_path_buf());
}

/// 返回 tree 中第一条 task，适用于首次切入 workflow task 工作台。
fn first_workflow_task_target(tree: &WorkflowTree) -> Option<WorkflowSelectionTarget> {
    workflow_tasks(tree)
        .next()
        .map(|task| WorkflowSelectionTarget::Task {
            task_path: task.path.clone(),
        })
}

//! 后台任务派发和异步结果 glue。
//!
//! 本模块只负责把阻塞 IO、文件扫描、terminal/reviewer 创建等工作派发到
//! 后台，并在完成后投递 `AppEvent`；不能直接从后台修改 UI 状态。

use super::*;
use crate::gui::hook;
use similar::TextDiff;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

/// Prompt used for the quick Agent input translation helper.
const AGENT_INPUT_TRANSLATION_PROMPT: &str = "I’m a software developer. Please translate the following content into English. Don’t make it too stiff; just give me a natural translation quickly, with no extra output. Preserve the full structure, line order, bullets, indentation, and line breaks. If the input has multiple lines, the output must also have multiple corresponding lines, not one merged sentence. Keep existing English lines unchanged unless small grammar cleanup is needed. The final answer must be entirely English except image markers. Markers like {#1}, {#2}, etc. are image placeholders. Keep those markers exactly as-is, do not translate or remove them.";
/// Fast translation model used only by the Agent input helper.
const AGENT_INPUT_TRANSLATION_MODEL: &str = "gpt-5.4-mini";

impl GsdvGuiApp {
    /// 启动 app 级 hook server，接收 Helix 等外部 hook 数据。
    pub(super) fn spawn_external_hook_server(&self) {
        let tx = self.app_event_tx.clone();
        self.background_runtime.spawn(async move {
            if let Err(error) = run_external_hook_server(tx).await {
                eprintln!("gsdv hook server failed: {error}");
            }
        });
    }

    /// 派发未提交文件变化触发的 reviewer 刷新任务。
    pub(super) fn spawn_reviewer_uncommitted_refresh_tasks(
        &mut self,
        ctx: &egui::Context,
        dirty_workspaces: BTreeSet<usize>,
    ) {
        for index in dirty_workspaces {
            self.spawn_indexed_reviewer_adapter_task(
                ctx,
                index,
                ReviewerAdapterTask::RefreshUncommitted,
                ReviewerAdapterTaskEffect::None,
            );
        }
    }

    /// workspace 文件变化后重载未被本地编辑污染的 Markdown 文档。
    pub(super) fn reload_clean_selected_documents(
        &mut self,
        ctx: &egui::Context,
        dirty_workspaces: &BTreeSet<usize>,
    ) {
        for index in dirty_workspaces {
            let Some(workspace) = self.workspaces.get(*index) else {
                continue;
            };
            let Some(path) = workspace.selected_file.clone() else {
                continue;
            };
            let clean_selected_document = self.documents.get(*index).is_some_and(|document| {
                document.path.as_ref() == Some(&path) && !document.is_dirty()
            });
            if !clean_selected_document {
                continue;
            }
            // 触发条件: notify 只知道 workspace 有文件变化。
            // 不能走常规路径: 当前文档路径没变，ensure 不会重读。
            // 防止: 外部保存后 editor/preview 继续显示旧内容。
            let absolute = resolve_workspace_path(&workspace.path, &path);
            let markdown_outline_collapsed = workspace.markdown_outline_collapsed;
            self.spawn_document_load_task(*index, path, absolute, markdown_outline_collapsed);
        }
        self.request_app_repaint();
    }

    /// Dispatches outline refreshes whose workspace roots emitted filesystem events.
    pub(super) fn spawn_outline_refresh_tasks(
        &mut self,
        ctx: &egui::Context,
        dirty_workspaces: BTreeSet<usize>,
    ) {
        for index in dirty_workspaces {
            let Some(workspace) = self.workspaces.get(index).cloned() else {
                continue;
            };
            let tx = self.app_event_tx.clone();
            let repaint_ctx = ctx.clone();
            let repaint_controller = self.repaint_controller.clone();
            self.background_runtime.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    let mut workspace = workspace;
                    data::refresh_workspace_outline(&mut workspace);
                    workspace
                })
                .await;
                if let Ok(workspace) = result {
                    let _ = tx.send(AppEvent::WorkspaceOutlineRefreshed { index, workspace });
                }
                repaint_controller.request_repaint(&repaint_ctx);
            });
        }
    }

    /// 将 reviewer adapter 加载派发到 render 之外。
    pub(super) fn process_pending_reviewer_loads(&mut self, ctx: &egui::Context) {
        let pending = std::mem::take(&mut self.pending_reviewer_loads);
        for index in pending {
            if self
                .reviewer_adapters
                .get(index)
                .is_some_and(Option::is_some)
                || self.reviewer_loads_in_flight.contains(&index)
                || self.reviewer_adapter_tasks_in_flight.contains(&index)
            {
                continue;
            }
            let Some(workspace) = self.workspaces.get(index) else {
                continue;
            };
            self.spawn_reviewer_adapter_load_task(ctx, index, workspace.path.clone());
        }
    }

    /// 派发设置 UI 控件请求的持久化任务。
    pub(super) fn process_pending_settings_side_effects(&mut self, ctx: &egui::Context) {
        if self.pending_runtime_settings_save {
            self.pending_runtime_settings_save = false;
            self.spawn_runtime_settings_save(self.runtime_settings.clone());
        }
        if self.pending_language_settings_save {
            self.pending_language_settings_save = false;
            self.spawn_app_language_save(self.app_language);
        }
        if self.pending_font_settings_save {
            self.pending_font_settings_save = false;
            self.spawn_runtime_fonts_apply_task(ctx, self.font_settings.clone());
            self.spawn_font_settings_save(self.font_settings.clone());
        }
        if self.pending_network_settings_save {
            self.pending_network_settings_save = false;
            self.spawn_network_settings_save(self.network_settings.clone());
        }
        if self.pending_default_agent_kind_save {
            self.pending_default_agent_kind_save = false;
            self.spawn_default_agent_kind_save(self.default_agent_kind);
        }
    }

    /// 为 UI 输入中编辑过的文档重建 Markdown 派生数据。
    pub(super) fn process_pending_markdown_reparse(&mut self, ctx: &egui::Context) {
        let pending = std::mem::take(&mut self.pending_markdown_reparse);
        for index in pending {
            let Some(text) = self
                .documents
                .get(index)
                .map(|document| document.text.clone())
            else {
                continue;
            };
            self.spawn_markdown_reparse_task(ctx, index, text);
        }
    }

    /// 处理 UI 控件发出的 Markdown outline 折叠请求。
    pub(super) fn process_pending_markdown_outline_collapse(&mut self, ctx: &egui::Context) {
        let pending = std::mem::take(&mut self.pending_markdown_outline_collapse);
        let mut changed = false;
        for index in pending {
            if let Some(document) = self.documents.get_mut(index) {
                document.markdown_outline_collapsed = true;
                changed = true;
            }
            if let Some(workspace) = self.workspaces.get_mut(index) {
                workspace.markdown_outline_collapsed = true;
                changed = true;
            }
        }
        if changed {
            self.mark_workspace_store_dirty();
        }
    }

    /// 保存 memo 编辑器请求的 memo 修改。
    pub(super) fn process_pending_memo_saves(&mut self, ctx: &egui::Context) {
        let pending = std::mem::take(&mut self.pending_memo_saves);
        for index in pending {
            let Some(workspace) = self.workspaces.get(index) else {
                continue;
            };
            self.spawn_memo_save_task(ctx, index, workspace.path.clone(), workspace.memo.clone());
        }
    }

    /// 处理 IME fallback 请求的下一帧 repaint。
    pub(super) fn process_pending_input_repaint(&mut self, _ctx: &egui::Context) {
        if !self.pending_input_repaint {
            return;
        }
        self.pending_input_repaint = false;
        self.request_app_repaint();
    }

    /// 后台读取字体文件并构建 egui 字体定义。
    pub(super) fn spawn_runtime_fonts_apply_task(
        &self,
        ctx: &egui::Context,
        settings: FontSettings,
    ) {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let event_settings = settings.clone();
            let fonts =
                tokio::task::spawn_blocking(move || build_runtime_font_definitions(&settings))
                    .await;
            if let Ok(fonts) = fonts {
                let _ = tx.send(AppEvent::RuntimeFontsPrepared {
                    settings: event_settings,
                    fonts,
                });
            }
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// 后台规范化并加载待添加的 workspace。
    pub(super) fn spawn_workspace_add_task(
        &self,
        ctx: &egui::Context,
        path: PathBuf,
        agent_kind: AgentKind,
        existing_paths: Vec<PathBuf>,
    ) {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let path = path.canonicalize().unwrap_or(path);
                if let Some((index, existing_path)) =
                    existing_paths
                        .into_iter()
                        .enumerate()
                        .find(|(_, existing_path)| {
                            existing_path
                                .canonicalize()
                                .unwrap_or_else(|_| existing_path.clone())
                                == path
                        })
                {
                    return WorkspaceAddTaskResult::Existing {
                        index,
                        path: existing_path,
                    };
                }
                WorkspaceAddTaskResult::New {
                    workspace: data::new_workspace(path, agent_kind),
                }
            })
            .await
            .map_err(|error| error.to_string());
            let _ = tx.send(AppEvent::WorkspaceAddPrepared { result });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// 将 Markdown 解析派发到 egui update 路径之外。
    pub(super) fn spawn_markdown_reparse_task(
        &self,
        ctx: &egui::Context,
        index: usize,
        text: String,
    ) {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let outline_entries = markdown_outline_entries(&text);
                let preview_blocks = markdown_preview::parse(&text);
                (text, outline_entries, preview_blocks)
            })
            .await;
            if let Ok((source_text, outline_entries, preview_blocks)) = result {
                let _ = tx.send(AppEvent::MarkdownParsed {
                    index,
                    source_text,
                    outline_entries,
                    preview_blocks,
                });
            }
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// Dispatches memo persistence away from the egui update path.
    pub(super) fn spawn_memo_save_task(
        &self,
        ctx: &egui::Context,
        index: usize,
        workspace_path: PathBuf,
        memo: String,
    ) {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let error = async {
                let Some(path) = data::workspace_memo_path(&workspace_path) else {
                    return None;
                };
                if let Some(parent) = path.parent()
                    && let Err(error) = tokio::fs::create_dir_all(parent).await
                {
                    return Some(error.to_string());
                }
                tokio::fs::write(path, memo)
                    .await
                    .err()
                    .map(|error| error.to_string())
            }
            .await;
            let _ = tx.send(AppEvent::MemoSaved { index, error });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// Dispatches Markdown file loading and parsing away from update.
    pub(super) fn spawn_document_load_task(
        &self,
        index: usize,
        path: PathBuf,
        absolute: PathBuf,
        markdown_outline_collapsed: bool,
    ) {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = self.app_repaint_ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let result = match tokio::fs::read_to_string(&absolute).await {
                Ok(text) => tokio::task::spawn_blocking(move || LoadedDocument {
                    outline_entries: markdown_outline_entries(&text),
                    preview_blocks: markdown_preview::parse(&text),
                    text,
                })
                .await
                .map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = tx.send(AppEvent::DocumentLoaded {
                index,
                path,
                absolute,
                markdown_outline_collapsed,
                result,
            });
            // The request is made by the task because completion may happen
            // after the dispatch-triggered frame has already been consumed.
            if let Some(ctx) = repaint_ctx {
                repaint_controller.request_repaint(&ctx);
            }
        });
    }

    /// Dispatches Markdown file saving away from update.
    pub(super) fn spawn_document_save_task(
        &self,
        index: usize,
        workspace_root: PathBuf,
        path: PathBuf,
        absolute: PathBuf,
        previous_text: String,
        text: String,
    ) {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = self.app_repaint_ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let result = tokio::fs::write(&absolute, text.as_bytes())
                .await
                .map_err(|error| format!("Failed to save {}: {error}", absolute.display()));
            let diff_history_error = if result.is_ok() {
                save_markdown_diff_history(&workspace_root, &path, &absolute, &previous_text, &text)
                    .await
                    .err()
            } else {
                None
            };
            let _ = tx.send(AppEvent::DocumentSaved {
                index,
                path,
                text,
                result,
                diff_history_error,
            });
            if let Some(ctx) = repaint_ctx {
                repaint_controller.request_repaint(&ctx);
            }
        });
    }

    /// 将关闭 workspace 的 sidecar 删除派发到后台。
    pub(super) fn spawn_workspace_close_sidecar_delete_task(
        &self,
        ctx: &egui::Context,
        index: usize,
        workspace_path: PathBuf,
    ) {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let event_workspace_path = workspace_path.clone();
            let result = tokio::task::spawn_blocking(move || {
                delete_workspace_close_sidecars(&workspace_path)
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(AppEvent::WorkspaceCloseSidecarsDeleted {
                index,
                workspace_path: event_workspace_path,
                result,
            });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// 派发最近 Markdown diff prompt 构建任务。
    pub(super) fn spawn_recent_markdown_diff_prompt_task(
        &self,
        ctx: &egui::Context,
        workspace_root: PathBuf,
        recent_paths: Vec<PathBuf>,
        since_ms: u128,
    ) {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                build_recent_markdown_diff_prompt(&workspace_root, &recent_paths, since_ms)
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(AppEvent::MarkdownDiffPromptBuilt { result });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// Dispatches current Agent input translation through the configured AI backend.
    pub(super) fn spawn_agent_input_translation_task(
        &mut self,
        ctx: &egui::Context,
        workspace_index: usize,
        agent_slot: AgentSlotId,
        source_text: String,
        content: String,
        source_has_images: bool,
    ) {
        let client = match self.agent_input_translation_client() {
            Ok(client) => client,
            Err(error) => {
                self.queue_app_event(AppEvent::AgentInputTranslationFinished {
                    workspace_index,
                    agent_slot,
                    source_text,
                    source_has_images,
                    result: Err(error),
                });
                return;
            }
        };
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let result = translate_agent_input_with_ai(client, content).await;
            let _ = tx.send(AppEvent::AgentInputTranslationFinished {
                workspace_index,
                agent_slot,
                source_text,
                source_has_images,
                result,
            });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// Returns the reusable Codex client used by quick Agent input translation.
    fn agent_input_translation_client(&mut self) -> Result<crate::ai::CodexClient, String> {
        let allow_http_fallback = self.runtime_settings.codex_responses_http_fallback_enabled;
        let cache_valid = self
            .agent_input_translation_client
            .as_ref()
            .is_some_and(|cache| {
                cache.network_settings == self.network_settings
                    && cache.allow_http_fallback == allow_http_fallback
            });
        if !cache_valid {
            let http = codex_auth_client(&self.network_settings)?;
            let mut remote = crate::ai::CodexRemote::new("gsdv-agent-input-translation");
            // 触发条件：快捷翻译需要低延迟，而不是默认 Agent 的重模型。
            // 不能复用 Codex 默认模型：默认模型服务长任务和复杂编码判断。
            // 防止回归：一次短翻译占用过高延迟/额度，影响输入流畅度。
            remote.model = AGENT_INPUT_TRANSLATION_MODEL.to_string();
            remote.proxy = self.network_settings.proxy.clone();
            remote.no_proxy = self.network_settings.effective_no_proxy();
            remote.allow_http_fallback = allow_http_fallback;
            self.agent_input_translation_client = Some(AgentInputTranslationClientCache {
                network_settings: self.network_settings.clone(),
                allow_http_fallback,
                client: crate::ai::CodexClient::with_http(remote, http),
            });
        }
        self.agent_input_translation_client
            .as_ref()
            .map(|cache| cache.client.clone())
            .ok_or_else(|| "failed to initialize Codex translation client".to_string())
    }

    /// Dispatches explicit filesystem mutations away from update.
    pub(super) fn spawn_file_mutation_task(&self, task: FileMutationTask) {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = self.app_repaint_ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let event = match task {
                FileMutationTask::CreateMarkdown {
                    index,
                    absolute_dir,
                    absolute_file,
                    target,
                    content,
                } => {
                    let result = async {
                        tokio::fs::create_dir_all(&absolute_dir)
                            .await
                            .map_err(|error| error.to_string())?;
                        tokio::fs::write(&absolute_file, content)
                            .await
                            .map_err(|error| error.to_string())
                    }
                    .await;
                    FileMutationResult::CreateMarkdown {
                        index,
                        target,
                        result,
                    }
                }
                FileMutationTask::CreateFolder {
                    index,
                    absolute_dir,
                } => {
                    let result = tokio::fs::create_dir_all(absolute_dir)
                        .await
                        .map_err(|error| error.to_string());
                    FileMutationResult::CreateFolder { index, result }
                }
                FileMutationTask::Rename {
                    index,
                    absolute,
                    target,
                    old_relative,
                    new_relative,
                } => {
                    let result = tokio::fs::rename(absolute, target)
                        .await
                        .map_err(|error| error.to_string());
                    FileMutationResult::Rename {
                        index,
                        old_relative,
                        new_relative,
                        result,
                    }
                }
                FileMutationTask::DeleteMarkdown {
                    index,
                    path,
                    absolute,
                } => {
                    let result = tokio::fs::remove_file(absolute)
                        .await
                        .map_err(|error| error.to_string());
                    FileMutationResult::DeleteMarkdown {
                        index,
                        path,
                        result,
                    }
                }
            };
            let _ = tx.send(AppEvent::FileMutationFinished(event));
            if let Some(ctx) = repaint_ctx {
                repaint_controller.request_repaint(&ctx);
            }
        });
    }

    /// Dispatches screenshot image persistence away from update.
    pub(super) fn spawn_screenshot_save_task(&self, path: PathBuf, image: Arc<egui::ColorImage>) {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = self.app_repaint_ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let event_path = path.clone();
            let result = tokio::task::spawn_blocking(move || {
                save_color_image_png(&path, &image)?;
                fs::write(screenshot_latest_path(), path.to_string_lossy().as_ref())
                    .map_err(|error| error.to_string())
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(AppEvent::ScreenshotSaved {
                path: event_path,
                result,
            });
            if let Some(ctx) = repaint_ctx {
                repaint_controller.request_repaint(&ctx);
            }
        });
    }

    /// Dispatches optional screenshot request file IO away from update.
    pub(super) fn spawn_screenshot_request_load_task(&mut self, ctx: &egui::Context) {
        self.screenshot_request_read_in_flight = true;
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let request_path = screenshot_request_path();
            let result = match tokio::fs::read_to_string(&request_path).await {
                Ok(command) => match tokio::fs::remove_file(&request_path).await {
                    Ok(()) => Ok(Some(command)),
                    Err(error) => Err(error.to_string()),
                },
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error.to_string()),
            };
            let _ = tx.send(AppEvent::ScreenshotRequestLoaded { result });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// Dispatches reviewer script scanning away from the egui update path.
    pub(super) fn spawn_reviewer_scripts_refresh_task(&mut self, ctx: &egui::Context) {
        self.reviewer_scripts.last_refresh = Some(Instant::now());
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(|| {
                load_reviewer_scripts().map_err(|error| error.to_string())
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(AppEvent::ReviewerScriptsLoaded { result });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// Dispatches terminal host creation away from render paths.
    pub(super) fn spawn_terminal_host_task(
        &mut self,
        ctx: &egui::Context,
        key: TerminalSpawnKey,
        workspace: WorkspaceViewData,
    ) {
        if !self.pending_terminal_spawns.insert(key.clone()) {
            return;
        }
        let agent_launch = self.agent_launch.clone();
        let network_settings = self.network_settings.clone();
        let spawn_ctx = ctx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        let theme_mode = self.theme_mode;
        let terminal_runtime_handle = self.background_runtime.handle().clone();
        let terminal_event_sink = self.terminal_runtime_event_sink(ctx);
        let tx = self.app_event_tx.clone();
        self.background_runtime.spawn(async move {
            let terminal_repaint_controller = repaint_controller.clone();
            let result = tokio::task::spawn_blocking(move || {
                GuiTerminalHost::spawn(
                    &spawn_ctx,
                    &workspace,
                    key.kind,
                    &agent_launch,
                    &network_settings,
                    terminal_repaint_controller,
                    theme_mode,
                    terminal_runtime_handle,
                    terminal_event_sink,
                )
                .map_err(|error| error.to_string())
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(AppEvent::TerminalHostSpawned { key, result });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// Applies a completed terminal host creation result.
    pub(super) fn apply_terminal_spawn_result(
        &mut self,
        key: TerminalSpawnKey,
        result: Result<GuiTerminalHost, String>,
    ) {
        let Some(hosts) = self.terminal_hosts.get_mut(key.index) else {
            return;
        };
        match result {
            Ok(host) => match key.kind {
                TerminalSurfaceKind::Agent => {
                    let slot = hosts.agents.entry(key.agent_slot).or_default();
                    slot.host = Some(host);
                    slot.error = None;
                }
                TerminalSurfaceKind::Workspace => {
                    hosts.workspace = Some(host);
                    hosts.workspace_error = None;
                }
                TerminalSurfaceKind::Helix => {
                    hosts.helix = Some(host);
                    hosts.helix_error = None;
                }
            },
            Err(error) => match key.kind {
                TerminalSurfaceKind::Agent => {
                    hosts.agents.entry(key.agent_slot).or_default().error = Some(error)
                }
                TerminalSurfaceKind::Workspace => hosts.workspace_error = Some(error),
                TerminalSurfaceKind::Helix => hosts.helix_error = Some(error),
            },
        }
    }

    /// Dispatches Helix terminal creation away from render paths.
    pub(super) fn spawn_helix_host_task(
        &mut self,
        ctx: &egui::Context,
        key: TerminalSpawnKey,
        workspace: WorkspaceViewData,
        spec: HelixLaunchSpec,
    ) {
        if !self.pending_terminal_spawns.insert(key.clone()) {
            return;
        }
        let network_settings = self.network_settings.clone();
        let spawn_ctx = ctx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        let theme_mode = self.theme_mode;
        let terminal_runtime_handle = self.background_runtime.handle().clone();
        let terminal_event_sink = self.terminal_runtime_event_sink(ctx);
        let tx = self.app_event_tx.clone();
        self.background_runtime.spawn(async move {
            let terminal_repaint_controller = repaint_controller.clone();
            let result = tokio::task::spawn_blocking(move || {
                GuiTerminalHost::spawn_helix(
                    &spawn_ctx,
                    &workspace,
                    spec,
                    &network_settings,
                    terminal_repaint_controller,
                    theme_mode,
                    terminal_runtime_handle,
                    terminal_event_sink,
                )
                .map_err(|error| error.to_string())
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(AppEvent::TerminalHostSpawned { key, result });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// 创建 terminal runtime 向唯一 AppEvent 队列投递事件的出口。
    pub(super) fn terminal_runtime_event_sink(
        &self,
        ctx: &egui::Context,
    ) -> TerminalRuntimeEventSink {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        Arc::new(move |event| {
            crate::gui::perf_log::count(terminal_runtime_sink_event_label(event.kind));
            let _ = tx.send(AppEvent::TerminalRuntime(event));
            repaint_controller.request_repaint(&repaint_ctx);
        })
    }

    /// 后台构建终端文件点击需要的 Helix 启动参数。
    pub(super) fn spawn_terminal_file_helix_spec_task(
        &self,
        ctx: &egui::Context,
        workspace_index: usize,
        workspace_dir: PathBuf,
        file: PathBuf,
        line: usize,
    ) {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let spec = tokio::task::spawn_blocking(move || HelixLaunchSpec {
                workdir: terminal_file_helix_workdir(&workspace_dir, &file),
                file: Some(file),
                line: Some(line),
            })
            .await;
            if let Ok(spec) = spec {
                let _ = tx.send(AppEvent::TerminalFileHelixSpecBuilt {
                    workspace_index,
                    spec,
                });
            }
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// 后台分类终端路径点击，避免 render 路径读取文件 metadata。
    pub(super) fn spawn_terminal_output_path_classify_task(
        &self,
        ctx: &egui::Context,
        workspace_index: usize,
        click: TerminalFileLineClick,
    ) {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let click =
                tokio::task::spawn_blocking(move || classify_terminal_output_path_click(click))
                    .await;
            if let Ok(click) = click {
                let _ = tx.send(AppEvent::TerminalOutputPathClassified {
                    workspace_index,
                    click,
                });
            }
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// Dispatches `hx` availability detection away from event handling.
    pub(super) fn spawn_helix_binary_check_task(
        &mut self,
        ctx: &egui::Context,
        request: HelixOpenRequest,
    ) {
        self.pending_helix_open_request = Some(request);
        if self.helix_binary_check_in_flight {
            return;
        }
        self.helix_binary_check_in_flight = true;
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let available = tokio::task::spawn_blocking(|| {
                Command::new("hx")
                    .arg("--version")
                    .output()
                    .is_ok_and(|output| output.status.success())
            })
            .await
            .unwrap_or(false);
            let _ = tx.send(AppEvent::HelixBinaryChecked { available });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// Dispatches file-manager reveal command away from event handling.
    pub(super) fn spawn_reveal_path_task(&self, ctx: &egui::Context, absolute: PathBuf) {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || reveal_path_command(&absolute))
                .await
                .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(AppEvent::RevealPathFinished { result });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// Dispatches Codex OAuth browser flow away from render and input paths.
    pub(super) fn spawn_codex_auth_task(&mut self, ctx: &egui::Context) {
        if self.codex_auth.in_flight {
            return;
        }
        let flow = crate::ai::start_browser_auth_flow();
        self.codex_auth.in_flight = true;
        self.codex_auth.auth_url = Some(flow.url.clone());
        self.codex_auth.error = None;
        self.codex_auth.started_at = Some(Instant::now());

        let network_settings = self.network_settings.clone();
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let result = match codex_auth_client(&network_settings) {
                Ok(client) => crate::ai::complete_browser_auth_flow(&client, flow)
                    .await
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error),
            };
            let _ = tx.send(AppEvent::CodexAuthFinished { result });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// Dispatches reviewer branch list loading away from UI actions.
    pub(super) fn spawn_reviewer_branch_choices_task(
        &self,
        ctx: &egui::Context,
        repo: ReviewerBranchTarget,
    ) {
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let task_repo = repo.clone();
            let result = tokio::task::spawn_blocking(move || {
                crate::reviewer::app::ensure_clean_repo(&task_repo.root)
                    .map_err(|error| error.to_string())?;
                crate::reviewer::app::load_branch_choices(&task_repo.root)
                    .map_err(|error| error.to_string())
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(AppEvent::ReviewerBranchChoicesLoaded { repo, result });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// Dispatches reviewer branch checkout and reload away from UI actions.
    pub(super) fn spawn_reviewer_branch_switch_task(
        &self,
        ctx: &egui::Context,
        repo: ReviewerBranchTarget,
        branch: BranchInfo,
    ) {
        let Some(project_dir) = self
            .current_workspace()
            .map(|workspace| workspace.path.clone())
        else {
            return;
        };
        let target = branch.label.clone();
        let checkout = branch.checkout.clone();
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let task_repo = repo.clone();
            let result = tokio::task::spawn_blocking(move || {
                crate::reviewer::app::checkout_branch(&task_repo.root, &checkout)
                    .map_err(|error| error.to_string())?;
                ReviewerAdapter::new(project_dir).map_err(|error| error.to_string())
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(AppEvent::ReviewerBranchSwitchFinished {
                repo,
                target,
                result,
            });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// Dispatches reviewer adapter construction away from render paths.
    pub(super) fn spawn_reviewer_adapter_load_task(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        project_dir: PathBuf,
    ) {
        if !self.reviewer_loads_in_flight.insert(index) {
            return;
        }
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                ReviewerAdapter::new(project_dir).map_err(|error| error.to_string())
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(AppEvent::ReviewerAdapterLoaded { index, result });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// Dispatches reviewer adapter mutation away from render and input paths.
    pub(super) fn spawn_reviewer_adapter_task(
        &mut self,
        ctx: &egui::Context,
        task: ReviewerAdapterTask,
        effect: ReviewerAdapterTaskEffect,
    ) {
        let index = self.active_workspace;
        self.spawn_indexed_reviewer_adapter_task(ctx, index, task, effect);
    }

    /// 派发当前 workspace 的轻量 git 数据加载。
    pub(super) fn spawn_reviewer_git_data_task(
        &mut self,
        ctx: &egui::Context,
        row_budget: usize,
        load_more: bool,
    ) {
        let index = self.active_workspace;
        self.spawn_indexed_reviewer_git_data_task(ctx, index, row_budget, load_more);
    }

    /// 派发指定 workspace 的轻量 git 数据加载。
    pub(super) fn spawn_indexed_reviewer_git_data_task(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        row_budget: usize,
        load_more: bool,
    ) {
        if !self.reviewer_git_data_in_flight.insert(index) {
            let entry = self
                .pending_reviewer_git_data_budget
                .entry(index)
                .or_insert((row_budget, load_more));
            entry.0 = entry.0.max(row_budget);
            entry.1 |= load_more;
            return;
        }
        let request = self
            .reviewer_adapters
            .get(index)
            .and_then(|adapter| adapter.as_ref())
            .and_then(|adapter| adapter.git_data_request(row_budget, load_more));
        let Some(request) = request else {
            self.reviewer_git_data_in_flight.remove(&index);
            return;
        };
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                crate::reviewer::app::load_reviewer_git_data(request)
                    .map_err(|error| error.to_string())
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(AppEvent::ReviewerGitDataLoaded { index, result });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// Dispatches one indexed reviewer adapter mutation away from UI paths.
    pub(super) fn spawn_indexed_reviewer_adapter_task(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        task: ReviewerAdapterTask,
        effect: ReviewerAdapterTaskEffect,
    ) {
        if !self.reviewer_adapter_tasks_in_flight.insert(index) {
            self.queue_reviewer_adapter_task(index, task, effect);
            return;
        }
        let adapter = self.reviewer_adapters.get_mut(index).and_then(Option::take);
        let Some(adapter) = adapter else {
            self.reviewer_adapter_tasks_in_flight.remove(&index);
            if self.reviewer_loads_in_flight.contains(&index) {
                self.queue_reviewer_adapter_task(index, task, effect);
            }
            return;
        };
        let tx = self.app_event_tx.clone();
        let repaint_ctx = ctx.clone();
        let repaint_controller = self.repaint_controller.clone();
        self.background_runtime.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let mut adapter = adapter;
                match task {
                    ReviewerAdapterTask::Reload => adapter.reload(),
                    ReviewerAdapterTask::RefreshUncommitted => adapter.refresh_uncommitted(),
                    ReviewerAdapterTask::ToggleFullDiff => adapter.toggle_full_diff(),
                    ReviewerAdapterTask::RefreshRepoDirty(row) => adapter.refresh_repo_dirty(row),
                    ReviewerAdapterTask::EnsureSelectedGitData { row_budget } => {
                        adapter.ensure_selected_git_data(row_budget)
                    }
                    ReviewerAdapterTask::LoadMoreSelectedRepoCommits { row_budget } => {
                        adapter.load_more_selected_repo_commits(row_budget)
                    }
                    ReviewerAdapterTask::SelectDiffRow(row) => {
                        adapter.select_diff_row(row);
                        Ok(())
                    }
                    ReviewerAdapterTask::ClickRow {
                        column,
                        row,
                        commit_row_budget,
                    } => adapter
                        .click_row(column, row, commit_row_budget)
                        .map(|_| ()),
                    ReviewerAdapterTask::JumpFullBlock { reverse } => {
                        adapter.jump_full_block(reverse);
                        Ok(())
                    }
                    ReviewerAdapterTask::PreviousItem => adapter.previous_item(),
                    ReviewerAdapterTask::NextItem => adapter.next_item(),
                }
                .map(|()| adapter)
                .map_err(|error| error.to_string())
            })
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
            let _ = tx.send(AppEvent::ReviewerAdapterTaskFinished {
                index,
                result,
                effect,
            });
            repaint_controller.request_repaint(&repaint_ctx);
        });
    }

    /// Queues a reviewer mutation while its adapter is owned by a worker.
    pub(super) fn queue_reviewer_adapter_task(
        &mut self,
        index: usize,
        task: ReviewerAdapterTask,
        effect: ReviewerAdapterTaskEffect,
    ) {
        let queue = self
            .pending_reviewer_adapter_tasks
            .entry(index)
            .or_default();
        if task == ReviewerAdapterTask::RefreshUncommitted
            && queue
                .iter()
                .any(|(queued, _)| *queued == ReviewerAdapterTask::RefreshUncommitted)
        {
            return;
        }
        if matches!(
            task,
            ReviewerAdapterTask::LoadMoreSelectedRepoCommits { .. }
        ) && queue.iter().any(|(queued, _)| {
            matches!(
                queued,
                ReviewerAdapterTask::LoadMoreSelectedRepoCommits { .. }
            )
        }) {
            return;
        }
        queue.push_back((task, effect));
    }

    /// Takes the next queued reviewer mutation for a workspace.
    pub(super) fn pop_pending_reviewer_adapter_task(
        &mut self,
        index: usize,
    ) -> Option<(ReviewerAdapterTask, ReviewerAdapterTaskEffect)> {
        let queue = self.pending_reviewer_adapter_tasks.get_mut(&index)?;
        let next = queue.pop_front();
        if queue.is_empty() {
            self.pending_reviewer_adapter_tasks.remove(&index);
        }
        next
    }
}

/// 返回 terminal runtime sink 事件的性能日志标签。
fn terminal_runtime_sink_event_label(kind: TerminalRuntimeEventKind) -> &'static str {
    match kind {
        TerminalRuntimeEventKind::Output => "terminal.sink.output",
        TerminalRuntimeEventKind::StateChanged => "terminal.sink.state_changed",
        TerminalRuntimeEventKind::Repaint => "terminal.sink.repaint",
    }
}

/// Builds the text pasted to Agent for recent Markdown diff context.
fn build_recent_markdown_diff_prompt(
    workspace_root: &Path,
    recent_paths: &[PathBuf],
    since_ms: u128,
) -> Result<String, String> {
    let candidates = recent_markdown_diff_candidates(workspace_root, recent_paths);
    if candidates.is_empty() {
        return Ok(String::new());
    }
    let Some(dir) = data::workspace_markdown_diffs_dir(workspace_root) else {
        return Ok(String::new());
    };
    let mut entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(format!("Failed to read {}: {error}", dir.display())),
    };

    let mut groups = BTreeMap::<String, Vec<(u128, String, String)>>::new();
    while let Some(entry) = entries
        .next()
        .transpose()
        .map_err(|error| format!("Failed to read {}: {error}", dir.display()))?
    {
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Failed to inspect {}: {error}", entry.path().display()))?;
        if !file_type.is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        let Some((path, timestamp)) =
            parse_markdown_diff_file_name(&file_name, &candidates, since_ms)
        else {
            continue;
        };
        let content = fs::read_to_string(entry.path())
            .map_err(|error| format!("Failed to read {}: {error}", file_name))?;
        if content.trim().is_empty() {
            continue;
        }
        groups
            .entry(path)
            .or_default()
            .push((timestamp, file_name, content));
    }
    Ok(format_recent_markdown_diff_prompt(groups))
}

/// 删除关闭 workspace 时需要清理的 app sidecar。
fn delete_workspace_close_sidecars(workspace_path: &Path) -> Result<(), String> {
    data::delete_workspace_memo(workspace_path)
        .map_err(|error| format!("Memo file could not be deleted: {error}"))?;
    data::delete_workspace_subagents(workspace_path)
        .map_err(|error| format!("Subagent file could not be deleted: {error}"))?;
    data::delete_workspace_outline_favorites(workspace_path)
        .map_err(|error| format!("Favorite file could not be deleted: {error}"))?;
    data::delete_workspace_recent_markdowns(workspace_path)
        .map_err(|error| format!("Recent markdown file could not be deleted: {error}"))?;
    data::delete_workspace_markdown_diffs(workspace_path)
        .map_err(|error| format!("Markdown diff history could not be deleted: {error}"))
}

/// Maps recent Markdown paths to their absolute-path hash prefixes.
fn recent_markdown_diff_candidates(
    workspace_root: &Path,
    recent_paths: &[PathBuf],
) -> BTreeMap<String, String> {
    let mut candidates = BTreeMap::new();
    for path in recent_paths {
        let absolute = resolve_workspace_path(workspace_root, path);
        let hash = data::stable_path_hash(&absolute);
        candidates.insert(hash, path.to_string_lossy().to_string());
    }
    candidates
}

/// Parses a diff filename and returns its recent Markdown path plus timestamp.
fn parse_markdown_diff_file_name(
    file_name: &str,
    candidates: &BTreeMap<String, String>,
    since_ms: u128,
) -> Option<(String, u128)> {
    let stem = file_name.strip_suffix(".diff")?;
    let (hash, rest) = stem.split_once('-')?;
    let path = candidates.get(hash)?.clone();
    let timestamp_text = rest
        .split_once('-')
        .map_or(rest, |(timestamp, _)| timestamp);
    let timestamp = timestamp_text.parse::<u128>().ok()?;
    (timestamp > since_ms).then_some((path, timestamp))
}

/// Formats Markdown diff groups for direct Agent input.
fn format_recent_markdown_diff_prompt(
    mut groups: BTreeMap<String, Vec<(u128, String, String)>>,
) -> String {
    let mut output = String::new();
    for (path, entries) in &mut groups {
        entries.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(path);
        output.push('\n');
        for (timestamp, _file_name, content) in entries {
            output.push_str("\ntime: ");
            output.push_str(&timestamp.to_string());
            output.push('\n');
            output.push_str(content.trim_end());
            output.push('\n');
        }
    }
    output
}

/// Saves one unified diff file for a successful Markdown save.
async fn save_markdown_diff_history(
    workspace_root: &Path,
    path: &Path,
    absolute: &Path,
    previous_text: &str,
    text: &str,
) -> Result<(), String> {
    if previous_text == text {
        return Ok(());
    }
    let diff = markdown_save_diff(path, previous_text, text);
    if diff.is_empty() {
        return Ok(());
    }
    let Some(dir) = data::workspace_markdown_diffs_dir(workspace_root) else {
        return Ok(());
    };
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|error| format!("Failed to create {}: {error}", dir.display()))?;
    let file_prefix = data::stable_path_hash(absolute);
    let path = next_markdown_diff_path(&dir, &file_prefix).await?;
    tokio::fs::write(&path, diff.as_bytes())
        .await
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))?;
    prune_markdown_diff_history(&dir).await
}

/// Builds the unified diff body stored for one Markdown save.
fn markdown_save_diff(path: &Path, previous_text: &str, text: &str) -> String {
    let label = path.to_string_lossy();
    TextDiff::from_lines(previous_text, text)
        .unified_diff()
        .header(&format!("a/{label}"), &format!("b/{label}"))
        .to_string()
}

/// Creates a path-hash-prefixed diff path without overwriting same-millisecond saves.
async fn next_markdown_diff_path(dir: &Path, file_prefix: &str) -> Result<PathBuf, String> {
    let timestamp = current_markdown_diff_timestamp();
    for suffix in 0..1000usize {
        let name = if suffix == 0 {
            format!("{file_prefix}-{timestamp}.diff")
        } else {
            format!("{file_prefix}-{timestamp}-{suffix}.diff")
        };
        let path = dir.join(name);
        if !tokio::fs::try_exists(&path)
            .await
            .map_err(|error| format!("Failed to inspect {}: {error}", path.display()))?
        {
            return Ok(path);
        }
    }
    Err(format!(
        "Failed to allocate Markdown diff file under {}",
        dir.display()
    ))
}

/// Returns a millisecond timestamp used as the primary diff filename.
fn current_markdown_diff_timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

/// Translates Agent input through Codex without enabling reasoning mode.
async fn translate_agent_input_with_ai(
    client: crate::ai::CodexClient,
    content: String,
) -> Result<String, String> {
    let mut request = crate::ai::ChatRequest::new(vec![crate::ai::Message::user(format!(
        "{AGENT_INPUT_TRANSLATION_PROMPT}\n\n{content}"
    ))]);
    request.thinking = crate::ai::ThinkingMode::Disabled;
    request.max_completion_tokens = Some(1024);
    let response = client
        .chat(&request)
        .await
        .map_err(|error| error.to_string())?;
    Ok(response.content.trim().to_string())
}

/// Keeps only the newest workspace-level Markdown diff files.
async fn prune_markdown_diff_history(dir: &Path) -> Result<(), String> {
    let mut entries = tokio::fs::read_dir(dir)
        .await
        .map_err(|error| format!("Failed to read {}: {error}", dir.display()))?;
    let mut diff_paths = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| format!("Failed to read {}: {error}", dir.display()))?
    {
        let path = entry.path();
        let is_diff = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("diff"));
        if is_diff {
            diff_paths.push(path);
        }
    }
    if diff_paths.len() <= MARKDOWN_DIFF_HISTORY_LIMIT {
        return Ok(());
    }
    diff_paths.sort();
    let remove_count = diff_paths.len() - MARKDOWN_DIFF_HISTORY_LIMIT;
    for path in diff_paths.into_iter().take(remove_count) {
        tokio::fs::remove_file(&path)
            .await
            .map_err(|error| format!("Failed to remove {}: {error}", path.display()))?;
    }
    Ok(())
}

/// 运行平台相关 hook server。
async fn run_external_hook_server(tx: Sender<AppEvent>) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use tokio::io::AsyncReadExt;
        use tokio::net::UnixListener;

        let endpoint = hook::app_hook_endpoint();
        let _ = std::fs::remove_file(&endpoint);
        let listener = UnixListener::bind(&endpoint)?;
        hook::hook_info(format_args!("server listen endpoint={endpoint}"));
        loop {
            let (mut stream, _) = listener.accept().await?;
            hook::hook_info(format_args!(
                "server accepted connection endpoint={endpoint}"
            ));
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut len = [0_u8; 4];
                if let Err(error) = stream.read_exact(&mut len).await {
                    hook::hook_info(format_args!("server read length failed error={error}"));
                    return;
                }
                let len = u32::from_be_bytes(len) as usize;
                if len > hook::MAX_HOOK_PAYLOAD_LEN {
                    hook::hook_info(format_args!("server rejected payload len={len}"));
                    return;
                }
                let mut payload = vec![0_u8; len];
                if let Err(error) = stream.read_exact(&mut payload).await {
                    hook::hook_info(format_args!(
                        "server read payload failed len={len} error={error}"
                    ));
                    return;
                }
                match hook::parse_hook_payload(&payload) {
                    Ok(event) => {
                        hook::hook_info(format_args!(
                            "server parsed key={} data={}",
                            event.key, event.data
                        ));
                        if let Err(error) = tx.send(AppEvent::ExternalHook(event)) {
                            hook::hook_info(format_args!("server enqueue failed error={error}"));
                        }
                    }
                    Err(error) => {
                        hook::hook_info(format_args!("server parse failed error={error:#}"));
                    }
                }
            });
        }
    }

    #[cfg(windows)]
    {
        use tokio::io::AsyncReadExt;
        use tokio::net::windows::named_pipe::ServerOptions;

        let endpoint = hook::app_hook_endpoint();
        hook::hook_info(format_args!("server listen endpoint={endpoint}"));
        loop {
            let mut pipe = ServerOptions::new().create(&endpoint)?;
            pipe.connect().await?;
            hook::hook_info(format_args!(
                "server accepted connection endpoint={endpoint}"
            ));
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut len = [0_u8; 4];
                if let Err(error) = pipe.read_exact(&mut len).await {
                    hook::hook_info(format_args!("server read length failed error={error}"));
                    return;
                }
                let len = u32::from_be_bytes(len) as usize;
                if len > hook::MAX_HOOK_PAYLOAD_LEN {
                    hook::hook_info(format_args!("server rejected payload len={len}"));
                    return;
                }
                let mut payload = vec![0_u8; len];
                if let Err(error) = pipe.read_exact(&mut payload).await {
                    hook::hook_info(format_args!(
                        "server read payload failed len={len} error={error}"
                    ));
                    return;
                }
                match hook::parse_hook_payload(&payload) {
                    Ok(event) => {
                        hook::hook_info(format_args!(
                            "server parsed key={} data={}",
                            event.key, event.data
                        ));
                        if let Err(error) = tx.send(AppEvent::ExternalHook(event)) {
                            hook::hook_info(format_args!("server enqueue failed error={error}"));
                        }
                    }
                    Err(error) => {
                        hook::hook_info(format_args!("server parse failed error={error:#}"));
                    }
                }
            });
        }
    }

    #[allow(unreachable_code)]
    Ok(())
}

//! Markdown 文档、outline 操作和最近访问记录。
//!
//! 本模块负责文档打开/保存、outline 菜单动作、收藏和最近访问列表，
//! 慢 IO 只通过后台任务派发。

use super::*;

impl GsdvGuiApp {
    pub(super) fn active_document(&self) -> Option<&DocumentState> {
        self.documents.get(self.active_workspace)
    }

    pub(super) fn active_document_mut(&mut self) -> Option<&mut DocumentState> {
        self.documents.get_mut(self.active_workspace)
    }

    pub(super) fn request_open_file(&mut self, path: PathBuf) {
        let dirty = self
            .active_document()
            .is_some_and(|document| document.is_dirty() && document.path.as_ref() != Some(&path));
        if dirty {
            self.set_active_app_dialog(Some(AppDialog::UnsavedSwitch { target: path }));
        } else {
            self.open_file_now(path);
        }
    }

    pub(super) fn open_file_now(&mut self, path: PathBuf) {
        let index = self.active_workspace;
        if let Some(workspace) = self.current_workspace_mut() {
            workspace.selected_file = Some(path.clone());
            workspace.center_mode = document_open_mode(workspace.center_mode);
        }
        self.record_recent_markdown_access(index, path.clone());
        self.mark_workspace_store_dirty();
        if self.active_document().is_some_and(|document| {
            document.path.as_ref() == Some(&path)
                && document.loading_path.is_none()
                && document.load_error.is_none()
        }) {
            return;
        }
        self.load_document(path);
    }

    pub(super) fn ensure_active_document_loaded(&mut self) {
        let Some(path) = self
            .current_workspace()
            .and_then(|workspace| workspace.selected_file.clone())
        else {
            return;
        };
        let should_load = self.active_document().is_none_or(|document| {
            document.path.as_ref() != Some(&path)
                && document.loading_path.as_ref() != Some(&path)
                && !document.is_dirty()
        });
        if should_load {
            self.load_document(path);
        }
    }

    pub(super) fn load_document(&mut self, path: PathBuf) {
        let Some(workspace_root) = self
            .current_workspace()
            .map(|workspace| workspace.path.clone())
        else {
            return;
        };
        let absolute = resolve_workspace_path(&workspace_root, &path);
        let markdown_outline_collapsed = self
            .current_workspace()
            .is_some_and(|workspace| workspace.markdown_outline_collapsed);
        let index = self.active_workspace;
        if let Some(document) = self.active_document_mut() {
            if document.loading_path.as_ref() == Some(&path) {
                return;
            }
            document.save_error = None;
            document.markdown_outline_collapsed = markdown_outline_collapsed;
            document.load_error = None;
            document.loading_path = Some(path.clone());
        }
        self.spawn_document_load_task(index, path, absolute, markdown_outline_collapsed);
    }

    pub(super) fn save_active_document(&mut self) {
        let Some(workspace_root) = self
            .current_workspace()
            .map(|workspace| workspace.path.clone())
        else {
            return;
        };
        let Some(document) = self.active_document_mut() else {
            return;
        };
        let Some(path) = document.path.clone() else {
            return;
        };
        let text = document.text.clone();
        let previous_text = document.saved_text.clone();
        let index = self.active_workspace;
        let absolute = resolve_workspace_path(&workspace_root, &path);
        self.spawn_document_save_task(index, workspace_root, path, absolute, previous_text, text);
    }

    pub(super) fn discard_and_open_file(&mut self, target: PathBuf) {
        if let Some(document) = self.active_document_mut() {
            document.text = document.saved_text.clone();
            document.markdown_preview_metrics = None;
            document.markdown_preview_metrics_width = 0.0;
            document.markdown_preview_heading_offsets.clear();
            document.markdown_preview_max_scroll_y = 0.0;
            document.markdown_editor_max_scroll_y = 0.0;
            document.loading_path = None;
            document.save_error = None;
        }
        self.open_file_now(target);
    }

    pub(super) fn handle_outline_action(&mut self, ctx: &egui::Context, action: OutlineAction) {
        match action {
            OutlineAction::OpenEditor(path) => {
                self.request_open_file(path);
                if let Some(workspace) = self.current_workspace_mut() {
                    workspace.center_mode = CenterMode::Editor;
                }
                self.persist_workspaces();
            }
            OutlineAction::OpenPreview(path) => {
                self.request_open_file(path);
                if let Some(workspace) = self.current_workspace_mut() {
                    workspace.center_mode = CenterMode::Preview;
                }
                self.persist_workspaces();
            }
            OutlineAction::CreateMarkdown(dir) => {
                self.set_active_app_dialog(Some(AppDialog::CreateMarkdown {
                    dir,
                    name: String::new(),
                }));
            }
            OutlineAction::CreateFolder(dir) => {
                self.set_active_app_dialog(Some(AppDialog::CreateFolder {
                    dir,
                    name: "new-folder".to_string(),
                }));
            }
            OutlineAction::Rename(path) => {
                let name = path
                    .file_name()
                    .map(|value| value.to_string_lossy().to_string())
                    .unwrap_or_else(|| "untitled.md".to_string());
                self.set_active_app_dialog(Some(AppDialog::RenamePath { path, name }));
            }
            OutlineAction::DeleteMarkdown(path) => {
                self.set_active_app_dialog(Some(AppDialog::DeleteMarkdown { path }));
            }
            OutlineAction::CopyAbsolute(path) => {
                let absolute = self.resolve_outline_target(&path);
                ctx.copy_text(absolute.to_string_lossy().to_string());
                self.push_toast(
                    i18n::text(self.app_language, "Copied absolute path"),
                    theme::success(),
                );
            }
            OutlineAction::CopyRelative(path) => {
                ctx.copy_text(path.to_string_lossy().to_string());
                self.push_toast(
                    i18n::text(self.app_language, "Copied relative path"),
                    theme::success(),
                );
            }
            OutlineAction::Reveal(path) => {
                self.reveal_path(ctx, path);
            }
            OutlineAction::ToggleFavorite(path) => {
                self.toggle_outline_favorite(path);
            }
            OutlineAction::Refresh => {
                self.spawn_outline_refresh_tasks(ctx, BTreeSet::from([self.active_workspace]));
            }
        }
    }

    /// 切换当前 workspace 的 outline 收藏过滤状态。
    pub(super) fn toggle_outline_favorites_only(&mut self, ctx: &egui::Context) {
        let Some(value) = self.outline_favorites_only.get_mut(self.active_workspace) else {
            return;
        };
        *value = !*value;
        let enabled = *value;
        self.push_toast(
            if enabled {
                i18n::text(self.app_language, "Outline favorites only")
            } else {
                i18n::text(self.app_language, "Outline all files")
            },
            theme::success(),
        );
        self.request_app_repaint(ctx);
    }

    /// 切换一个 Markdown 文件的收藏状态，并按 Home/workspace 分区保存。
    pub(super) fn toggle_outline_favorite(&mut self, path: PathBuf) {
        let Some((scope, key)) = self.outline_favorite_key(&path) else {
            return;
        };
        match scope {
            OutlineFavoriteScope::Global => {
                let favorited = toggle_path_in_set(&mut self.global_outline_favorites, key);
                self.spawn_global_outline_favorites_save(self.global_outline_favorites.clone());
                self.push_toast(
                    if favorited {
                        i18n::text(self.app_language, "Favorite added")
                    } else {
                        i18n::text(self.app_language, "Favorite removed")
                    },
                    theme::success(),
                );
            }
            OutlineFavoriteScope::Workspace => {
                let Some(workspace) = self.current_workspace_mut() else {
                    return;
                };
                let favorited = toggle_path_in_set(&mut workspace.outline_favorites, key);
                let workspace_path = workspace.path.clone();
                let favorites = workspace.outline_favorites.clone();
                self.spawn_workspace_outline_favorites_save(workspace_path, favorites);
                self.push_toast(
                    if favorited {
                        i18n::text(self.app_language, "Favorite added")
                    } else {
                        i18n::text(self.app_language, "Favorite removed")
                    },
                    theme::success(),
                );
            }
        }
    }

    /// 切换最近访问 Markdown modal。
    pub(super) fn toggle_recent_markdown_outline_dialog(&mut self) {
        if matches!(
            self.active_app_dialog(),
            Some(AppDialog::RecentMarkdownOutline { .. })
        ) {
            self.set_active_app_dialog(None);
            return;
        }
        if self.active_app_dialog().is_some()
            || self.active_reviewer_dialog().is_some()
            || !should_show_outline_panel(self.current_workspace())
        {
            return;
        }
        let nodes = self.recent_markdown_outline_nodes();
        self.set_active_app_dialog(Some(AppDialog::RecentMarkdownOutline { nodes }));
    }

    /// 记录当前 workspace 内一次 Markdown 查看或编辑。
    pub(super) fn record_recent_markdown_access(&mut self, index: usize, path: PathBuf) {
        let edited_at_ms = current_unix_millis();
        let Some(workspace) = self.workspaces.get_mut(index) else {
            return;
        };
        let Some(position) = workspace
            .recent_markdowns
            .iter()
            .position(|entry| entry.path == path)
        else {
            workspace
                .recent_markdowns
                .insert(0, data::RecentMarkdownEntry { path, edited_at_ms });
            workspace.recent_markdowns.truncate(RECENT_MARKDOWN_LIMIT);
            self.mark_workspace_store_dirty();
            return;
        };
        let mut entry = workspace.recent_markdowns.remove(position);
        entry.edited_at_ms = edited_at_ms;
        workspace.recent_markdowns.insert(0, entry);
        workspace.recent_markdowns.truncate(RECENT_MARKDOWN_LIMIT);
        self.mark_workspace_store_dirty();
    }

    /// 粘贴上次请求之后的 Markdown diff 上下文到当前 Agent 输入。
    pub(super) fn paste_recent_markdown_diffs_to_agent(&mut self, ctx: &egui::Context) {
        let Some(workspace) = self.current_workspace() else {
            return;
        };
        let recent_paths = workspace
            .recent_markdowns
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        let workspace_root = workspace.path.clone();
        if recent_paths.is_empty() {
            self.push_toast(
                i18n::text(self.app_language, "No recent Markdown files"),
                theme::warning(),
            );
            return;
        }
        let since_ms = self.markdown_diff_paste_since_ms;
        self.markdown_diff_paste_since_ms = u128::from(current_unix_millis());
        self.spawn_recent_markdown_diff_prompt_task(ctx, workspace_root, recent_paths, since_ms);
    }

    /// 重命名后同步最近访问 Markdown 记录。
    pub(super) fn rename_recent_markdown_path(
        &mut self,
        index: usize,
        old_path: &Path,
        new_path: &Path,
    ) {
        let Some(workspace) = self.workspaces.get_mut(index) else {
            return;
        };
        let mut changed = false;
        for entry in &mut workspace.recent_markdowns {
            if entry.path == old_path {
                entry.path = new_path.to_path_buf();
                changed = true;
            }
        }
        if changed {
            self.mark_workspace_store_dirty();
        }
    }

    /// 删除后移除最近访问 Markdown 记录。
    pub(super) fn remove_recent_markdown_path(&mut self, index: usize, path: &Path) {
        let Some(workspace) = self.workspaces.get_mut(index) else {
            return;
        };
        let before = workspace.recent_markdowns.len();
        workspace
            .recent_markdowns
            .retain(|entry| entry.path != path);
        if workspace.recent_markdowns.len() != before {
            self.mark_workspace_store_dirty();
        }
    }

    /// 生成最近访问 Markdown modal 使用的 outline tree。
    pub(super) fn recent_markdown_outline_nodes(&self) -> Vec<OutlineNode> {
        let Some(workspace) = self.current_workspace() else {
            return Vec::new();
        };
        recent_markdown_outline_nodes(&workspace.outline, &workspace.recent_markdowns)
    }

    /// 返回 outline 收藏分区和持久化 key。
    pub(super) fn outline_favorite_key(
        &self,
        path: &Path,
    ) -> Option<(OutlineFavoriteScope, PathBuf)> {
        let absolute = self.resolve_outline_target(path);
        if outline_path_is_global(path, &absolute) {
            return Some((OutlineFavoriteScope::Global, absolute));
        }
        let workspace = self.current_workspace()?;
        let key = absolute
            .strip_prefix(&workspace.path)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| path.to_path_buf());
        Some((OutlineFavoriteScope::Workspace, key))
    }

    pub(super) fn create_markdown_file(&mut self, dir: PathBuf, name: String) {
        let name = normalize_markdown_name(&name);
        let absolute_dir = self.resolve_outline_target(&dir);
        let absolute_file = absolute_dir.join(&name);
        let target = self.workspace_relative_or_absolute(&absolute_file);
        let content = format!("# {}\n", markdown_title(&name));
        self.spawn_file_mutation_task(FileMutationTask::CreateMarkdown {
            index: self.active_workspace,
            absolute_dir,
            absolute_file,
            target,
            content,
        });
    }

    pub(super) fn create_folder(&mut self, dir: PathBuf, name: String) {
        let name = normalize_folder_name(&name);
        let absolute_dir = self.resolve_outline_target(&dir).join(&name);
        self.spawn_file_mutation_task(FileMutationTask::CreateFolder {
            index: self.active_workspace,
            absolute_dir,
        });
    }

    pub(super) fn rename_path(&mut self, path: PathBuf, name: String) {
        let absolute = self.resolve_outline_target(&path);
        let Some(parent) = absolute.parent() else {
            self.set_active_app_dialog(Some(AppDialog::Message {
                title: i18n::text(self.app_language, "Rename Failed").to_string(),
                message: i18n::text(self.app_language, "Cannot resolve parent directory.")
                    .to_string(),
            }));
            return;
        };
        let target = parent.join(normalize_rename_name(&absolute, &name));
        if target == absolute {
            return;
        }
        let old_relative = self.workspace_relative_or_absolute(&absolute);
        let new_relative = self.workspace_relative_or_absolute(&target);
        self.spawn_file_mutation_task(FileMutationTask::Rename {
            index: self.active_workspace,
            absolute,
            target,
            old_relative,
            new_relative,
        });
    }

    pub(super) fn delete_markdown_file(&mut self, path: PathBuf) {
        let absolute = self.resolve_outline_target(&path);
        self.spawn_file_mutation_task(FileMutationTask::DeleteMarkdown {
            index: self.active_workspace,
            path,
            absolute,
        });
    }

    pub(super) fn reveal_path(&mut self, ctx: &egui::Context, path: PathBuf) {
        let absolute = self.resolve_outline_target(&path);
        self.spawn_reveal_path_task(ctx, absolute);
    }

    pub(super) fn resolve_outline_target(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            return path.to_path_buf();
        }
        if let Some(rest) = path.strip_prefix("~").ok() {
            if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
                return home.join(rest);
            }
        }
        self.current_workspace()
            .map(|workspace| workspace.path.join(path))
            .unwrap_or_else(|| path.to_path_buf())
    }

    pub(super) fn workspace_relative_or_absolute(&self, absolute: &Path) -> PathBuf {
        self.current_workspace()
            .and_then(|workspace| absolute.strip_prefix(&workspace.path).ok())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| absolute.to_path_buf())
    }
}

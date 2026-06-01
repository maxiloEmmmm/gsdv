//! Store 与设置持久化副作用。
//!
//! 这里处理 dirty 合并和后台写盘。持久化不是渲染语义，不能通过
//! AppEvent drain 直接写盘，只能由业务 owner 标记 dirty 后异步合并保存。

use super::*;

impl GsdvGuiApp {
    /// 标记 workspace store 需要保存，由独立 writer 合并实际磁盘写入。
    ///
    /// 这里只记录 dirty 和唤醒时间，不做序列化，也不写磁盘。
    pub(super) fn mark_workspace_store_dirty(&mut self) {
        self.workspace_store_dirty_at
            .get_or_insert_with(Instant::now);
        if let Some(ctx) = self.app_repaint_ctx.as_ref() {
            ctx.request_repaint_after(WORKSPACE_STORE_SAVE_DEBOUNCE);
        }
    }

    /// 处理 workspace store writer 的 dirty 合并和后台保存。
    ///
    /// 触发条件：UI state owner 标记 workspace store dirty。
    /// 不能作为 AppEvent：持久化是业务副作用，不是 render state。
    /// 防止回归：高频 UI 变更在事件 drain 里同步写盘。
    pub(super) fn process_workspace_store_writer(&mut self, ctx: &egui::Context) {
        let Some(dirty_at) = self.workspace_store_dirty_at else {
            return;
        };
        let now = Instant::now();
        if now.duration_since(dirty_at) < WORKSPACE_STORE_SAVE_DEBOUNCE {
            ctx.request_repaint_after(duration_until_due(dirty_at, WORKSPACE_STORE_SAVE_DEBOUNCE));
            return;
        }
        if self.workspace_store_save_in_flight.load(Ordering::SeqCst) {
            // 触发条件：保存期间又有业务状态被标记为 dirty。
            // 不能直接再开保存：短时间多次修改会把 writer 打爆。
            // 防止回归：大量 UI 状态变化导致同一 store 快照重复写盘。
            ctx.request_repaint_after(WORKSPACE_STORE_SAVE_DEBOUNCE);
            return;
        }
        self.workspace_store_dirty_at = None;
        let workspaces = self.workspaces.clone();
        let active_workspace = self.active_workspace;
        let rail_collapsed = self.rail_collapsed;
        let save_in_flight = Arc::clone(&self.workspace_store_save_in_flight);
        let repaint_ctx = ctx.clone();
        let repaint_after = self.max_repaint_interval();
        save_in_flight.store(true, Ordering::SeqCst);
        self.background_runtime.spawn(async move {
            let _ = tokio::task::spawn_blocking(move || {
                data::save_workspace_store(&workspaces, active_workspace, rail_collapsed);
            })
            .await;
            save_in_flight.store(false, Ordering::SeqCst);
            repaint_ctx.request_repaint_after(repaint_after);
        });
    }

    /// Dispatches theme persistence away from update.
    pub(super) fn spawn_theme_mode_save(&self, mode: theme::ThemeMode) {
        self.background_runtime.spawn(async move {
            let _ = tokio::task::spawn_blocking(move || data::save_theme_mode(mode)).await;
        });
    }

    /// Dispatches runtime settings persistence away from update.
    pub(super) fn spawn_runtime_settings_save(&self, settings: RuntimeSettings) {
        self.background_runtime.spawn(async move {
            let _ =
                tokio::task::spawn_blocking(move || data::save_runtime_settings(&settings)).await;
        });
    }

    /// 将语言设置持久化派发到后台，避免阻塞 UI 更新。
    pub(super) fn spawn_app_language_save(&self, language: AppLanguage) {
        self.background_runtime.spawn(async move {
            let _ = tokio::task::spawn_blocking(move || data::save_app_language(language)).await;
        });
    }

    /// 将默认 agent 类型持久化派发到后台。
    pub(super) fn spawn_default_agent_kind_save(&self, agent_kind: AgentKind) {
        self.background_runtime.spawn(async move {
            let _ = tokio::task::spawn_blocking(move || data::save_default_agent_kind(agent_kind))
                .await;
        });
    }

    /// 将全局 outline 收藏保存派发到后台。
    pub(super) fn spawn_global_outline_favorites_save(&self, favorites: BTreeSet<PathBuf>) {
        self.background_runtime.spawn(async move {
            let _ = tokio::task::spawn_blocking(move || {
                data::save_global_outline_favorites(&favorites)
            })
            .await;
        });
    }

    /// 将 workspace outline 收藏保存派发到后台。
    pub(super) fn spawn_workspace_outline_favorites_save(
        &self,
        workspace_path: PathBuf,
        favorites: BTreeSet<PathBuf>,
    ) {
        self.background_runtime.spawn(async move {
            let _ = tokio::task::spawn_blocking(move || {
                data::save_workspace_outline_favorites(&workspace_path, &favorites)
            })
            .await;
        });
    }

    /// Dispatches font settings persistence away from update.
    pub(super) fn spawn_font_settings_save(&self, settings: FontSettings) {
        self.background_runtime.spawn(async move {
            let _ = tokio::task::spawn_blocking(move || data::save_font_settings(&settings)).await;
        });
    }

    /// Dispatches network settings persistence away from update.
    pub(super) fn spawn_network_settings_save(&self, settings: NetworkSettings) {
        self.background_runtime.spawn(async move {
            let _ =
                tokio::task::spawn_blocking(move || data::save_network_settings(&settings)).await;
        });
    }
}

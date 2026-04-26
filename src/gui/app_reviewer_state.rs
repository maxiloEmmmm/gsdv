//! Reviewer adapter 状态访问和快照同步。
//!
//! 这里保存 reviewer adapter/snapshot 的读取与选择同步逻辑，绘制仍在
//! reviewer UI 模块，后台 mutation 仍在 tasks 模块。

use super::*;

impl GsdvGuiApp {
    pub(super) fn ensure_active_reviewer(&mut self) {
        if self.active_workspace >= self.reviewer_adapters.len() {
            return;
        }
        if self.reviewer_adapters[self.active_workspace].is_none() {
            self.pending_reviewer_loads.insert(self.active_workspace);
        }
    }

    pub(super) fn active_reviewer_adapter_mut(&mut self) -> Option<&mut ReviewerAdapter> {
        self.reviewer_adapters
            .get_mut(self.active_workspace)
            .and_then(Option::as_mut)
    }

    pub(super) fn active_reviewer_adapter(&self) -> Option<&ReviewerAdapter> {
        self.reviewer_adapters
            .get(self.active_workspace)
            .and_then(Option::as_ref)
    }

    /// Returns a reviewer snapshot that remains drawable during background refresh.
    pub(super) fn reviewer_snapshot_for_paint(
        &mut self,
    ) -> Option<crate::reviewer::app::GuiReviewerSnapshot> {
        let selected_row = self
            .reviewer_diff_selected_rows
            .get(self.active_workspace)
            .copied()
            .flatten();
        if let Some(mut snapshot) = self
            .active_reviewer_adapter()
            .map(|adapter| adapter.snapshot().clone())
        {
            apply_reviewer_diff_selection_override(&mut snapshot, selected_row);
            if let Some(slot) = self.reviewer_snapshots.get_mut(self.active_workspace) {
                *slot = Some(snapshot.clone());
            }
            return Some(snapshot);
        }
        let mut snapshot = self
            .reviewer_snapshots
            .get(self.active_workspace)
            .and_then(Clone::clone)?;
        apply_reviewer_diff_selection_override(&mut snapshot, selected_row);
        Some(snapshot)
    }

    /// 将主窗口拉到前台，适用于 Codex Auth 回调已收到的场景。
    pub(super) fn raise_window_for_codex_auth_result(&mut self, ctx: &egui::Context) {
        // 触发条件：用户完成浏览器授权后，gsdv 可能在其他窗口后面。
        // 不能用 AlwaysOnTop：那是持久窗口层级，必须再恢复。
        // 防止回归：OAuth 成功或失败后用户误以为 UI 没收到回调。
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
            egui::UserAttentionType::Informational,
        ));
    }
}

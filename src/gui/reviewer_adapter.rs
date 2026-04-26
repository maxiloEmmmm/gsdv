use crate::gui::data::ReviewerMode as GuiReviewerMode;
use crate::reviewer::app::{
    GuiReviewerBranchTarget, GuiReviewerSnapshot, ReviewerGitDataRequest, ReviewerGitDataResult,
    ReviewerHelixTarget,
};
use crate::reviewer::{
    MIN_REVIEWER_WIDTH, ReviewerContext, ReviewerExitMode, ReviewerMode, ReviewerRuntime,
};
use crate::{BranchCheckout, BranchInfo};
use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerBranchTarget {
    pub label: String,
    pub repo: Option<String>,
    pub root: PathBuf,
}

pub struct ReviewerAdapter {
    runtime: ReviewerRuntime,
    last_width: u16,
    last_height: u16,
}

impl ReviewerAdapter {
    pub fn new(project_dir: PathBuf) -> Result<Self> {
        let runtime = ReviewerRuntime::load_with_exit_mode(
            ReviewerContext {
                project_dir,
                phase_id: None,
                workstream: None,
            },
            ReviewerExitMode::Quit,
        )?;
        Ok(Self {
            runtime,
            last_width: MIN_REVIEWER_WIDTH,
            last_height: 24,
        })
    }

    pub fn mode(&self) -> GuiReviewerMode {
        match self.runtime.mode() {
            ReviewerMode::Gsd => GuiReviewerMode::Gsd,
            ReviewerMode::Git => GuiReviewerMode::Git,
        }
    }

    pub fn set_mode(&mut self, desired: GuiReviewerMode) -> Result<()> {
        if self.mode() == desired {
            return Ok(());
        }
        self.runtime.gui_set_mode(match desired {
            GuiReviewerMode::Gsd => ReviewerMode::Gsd,
            GuiReviewerMode::Git => ReviewerMode::Git,
        })
    }

    pub fn reload(&mut self) -> Result<()> {
        self.runtime.reload()
    }

    /// Refreshes uncommitted reviewer rows after filesystem changes.
    pub fn refresh_uncommitted(&mut self) -> Result<()> {
        self.runtime.gui_refresh_uncommitted()
    }

    pub fn sync_viewport_size(&mut self, width_px: f32, height_px: f32) {
        let width_cols = px_to_cols(width_px).max(MIN_REVIEWER_WIDTH);
        let height_rows = px_to_rows(height_px).max(8);
        self.last_width = width_cols;
        self.last_height = height_rows;
    }

    pub fn snapshot(&self) -> &GuiReviewerSnapshot {
        self.runtime.gui_snapshot()
    }

    pub fn click_row(
        &mut self,
        column: usize,
        row_index: usize,
        commit_row_budget: usize,
    ) -> Result<bool> {
        self.runtime
            .gui_select_row(column, row_index, commit_row_budget)?;
        Ok(true)
    }

    pub fn load_more_selected_repo_commits(&mut self, row_budget: usize) -> Result<()> {
        self.runtime.gui_load_more_selected_repo_commits(row_budget)
    }

    pub fn click_row_light(
        &mut self,
        column: usize,
        row_index: usize,
        commit_row_budget: usize,
    ) -> bool {
        self.runtime
            .gui_select_row_light(column, row_index, commit_row_budget)
    }

    pub fn ensure_selected_git_data(&mut self, row_budget: usize) -> Result<()> {
        self.runtime.gui_ensure_selected_git_data(row_budget)
    }

    pub fn git_data_request(
        &self,
        row_budget: usize,
        load_more: bool,
    ) -> Option<ReviewerGitDataRequest> {
        self.runtime.gui_git_data_request(row_budget, load_more)
    }

    pub fn apply_git_data_result(&mut self, result: ReviewerGitDataResult) {
        self.runtime.gui_apply_git_data_result(result);
    }

    pub fn refresh_repo_dirty(&mut self, row_index: usize) -> Result<()> {
        self.runtime.refresh_git_repo_dirty(row_index)
    }

    pub fn toggle_full_diff(&mut self) -> Result<()> {
        self.runtime
            .gui_toggle_full_diff(self.last_width, self.last_height)
    }

    pub fn jump_full_block(&mut self, reverse: bool) {
        self.runtime.gui_jump_full_block(reverse);
    }

    pub fn set_diff_scroll_row(&mut self, row: usize) {
        self.runtime.gui_set_diff_scroll_row(row, self.last_width);
    }

    pub fn select_diff_row(&mut self, row: usize) {
        self.runtime.gui_select_diff_row(row, self.last_width);
    }

    pub fn collapse_file_tree_to_first_level(&mut self) {
        self.runtime.gui_collapse_file_tree_to_first_level();
    }

    pub fn diff_scroll_row(&self) -> usize {
        self.runtime.gui_diff_scroll_row()
    }

    pub fn selected_helix_target(&mut self) -> Option<ReviewerHelixTarget> {
        self.runtime.gui_selected_helix_target(self.last_width)
    }

    pub fn next_item(&mut self) -> Result<()> {
        self.runtime.gui_next_item(self.last_width)
    }

    pub fn previous_item(&mut self) -> Result<()> {
        self.runtime.gui_previous_item(self.last_width)
    }

    pub fn next_column(&mut self) {
        self.runtime.gui_next_column();
    }

    pub fn previous_column(&mut self) {
        self.runtime.gui_previous_column();
    }

    pub fn selected_agent_paste_text(&self) -> Option<String> {
        self.runtime.gui_selected_agent_paste_text()
    }

    pub fn selected_branch_target(&self) -> Option<ReviewerBranchTarget> {
        self.runtime
            .selected_repo_for_branch_action()
            .map(|(label, repo, root)| ReviewerBranchTarget { label, repo, root })
    }

    pub fn reviewer_branch_target(target: &GuiReviewerBranchTarget) -> ReviewerBranchTarget {
        ReviewerBranchTarget {
            label: target.label.clone(),
            repo: target.repo.clone(),
            root: target.root.clone(),
        }
    }

    pub fn ensure_clean_branch_target(&self, root: &Path) -> Result<()> {
        crate::reviewer::app::ensure_clean_repo(root)
    }

    pub fn load_branch_choices(&self, root: &Path) -> Result<(String, Vec<BranchInfo>)> {
        crate::reviewer::app::load_branch_choices(root)
    }

    pub fn checkout_branch(&mut self, root: &Path, checkout: &BranchCheckout) -> Result<()> {
        crate::reviewer::app::checkout_branch(root, checkout)?;
        self.reload()
    }
}

fn px_to_cols(width_px: f32) -> u16 {
    ((width_px / 8.5).floor() as u16).max(1)
}

fn px_to_rows(height_px: f32) -> u16 {
    ((height_px / 20.0).floor() as u16).max(1)
}

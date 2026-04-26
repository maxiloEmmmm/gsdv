use crate::reviewer::git::UNCOMMITTED_COMMIT_ID;
use crate::reviewer::{
    ChangeGroup, DiffBody, DiffLine, DiffLineKind, DiffLineMetadata, DiffPayload, DiffSource,
    FileEntry, GIT_COMMIT_PAGE_SIZE, GitCommitReview, GitFileReview, GitRepoReview,
    LoadPhaseOptions, PhaseProvenance, load_diff, load_full_file_payload, load_git_commit_files,
    load_git_dirty_commit, load_git_repo_commit_page, load_git_review, load_phase_provenance,
};
use crate::scrolling::{page_delta, scroll_offset};
use crate::{BranchCheckout, BranchInfo};
use anyhow::{Context, Result, bail};
use std::cmp;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::iter::Peekable;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::Chars;
use std::time::Instant;

pub const MIN_REVIEWER_WIDTH: u16 = 120;
const HEADER_ROWS: u16 = 2;
const FOOTER_ROWS: u16 = 1;
const COLUMN_SEPARATOR: &str = "|";
const EMPTY_REPOS_COPY: &str = "No repos for this change group";
const EMPTY_FILES_COPY: &str = "No files for this repo bucket";
const EMPTY_DIFF_COPY: &str = "No commit-backed diff for the selected file";
pub(crate) const LOAD_MORE_COMMITS_LABEL: &str = "Load more...";
const COLUMN_COUNT: usize = 4;
const MIN_DIFF_WIDTH: usize = 40;
const WIDE_MIN_DIFF_WIDTH: usize = 46;
const CLEAR_EOL_SENTINEL: char = '\u{FFF0}';
const CONTEXT_PREFIX: &str = "C|";
const INSERT_PREFIX: &str = "I|";
const DELETE_PREFIX: &str = "D|";
const HUNK_PREFIX: &str = "H|";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveColumn {
    ChangeGroups,
    Repos,
    Files,
    Diff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewerMode {
    Gsd,
    Git,
}

impl ActiveColumn {
    fn next(self) -> Self {
        match self {
            Self::ChangeGroups => Self::Repos,
            Self::Repos => Self::Files,
            Self::Files => Self::Diff,
            Self::Diff => Self::ChangeGroups,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::ChangeGroups => Self::Diff,
            Self::Repos => Self::ChangeGroups,
            Self::Files => Self::Repos,
            Self::Diff => Self::Files,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerSelection {
    pub change_row_index: usize,
    pub repo_index: usize,
    pub file_index: usize,
    pub column_scrolls: [usize; 3],
    pub row_offsets: [usize; 3],
}

impl Default for ReviewerSelection {
    fn default() -> Self {
        Self {
            change_row_index: 0,
            repo_index: 0,
            file_index: 0,
            column_scrolls: [0; 3],
            row_offsets: [0; 3],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReviewerDiffState {
    pub diff_scroll: usize,
    pub full_scroll: usize,
    pub diff_cursor_row: usize,
    pub full_cursor_row: usize,
    pub view_mode: DiffViewMode,
    pub diff_scroll_before_full: Option<usize>,
    pub diff_payload_cache: BTreeMap<String, DiffPayload>,
    pub diff_render_cache_width: Option<usize>,
    pub diff_render_cache: Vec<String>,
    /// Full 文件内容是否已经完成加载。
    pub full_file_loaded: bool,
    pub full_lines: Vec<String>,
    pub full_jump_targets: Vec<usize>,
    pub full_jump_render_rows: Vec<usize>,
    pub full_changed_lines: BTreeSet<usize>,
    pub full_deleted_before: BTreeMap<usize, Vec<String>>,
    pub full_render_cache_width: Option<usize>,
    pub full_render_cache: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiffViewMode {
    #[default]
    Diff,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HoverTarget {
    column: usize,
    row: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadState {
    Loading,
    Ready,
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerSession {
    pub active_column: ActiveColumn,
    pub selection: ReviewerSelection,
    pub diff_state: ReviewerDiffState,
    /// Current GUI-facing data prepared by event handlers.
    pub gui_state: GuiReviewerPreparedState,
    hovered: Option<HoverTarget>,
    pub mode: ReviewerMode,
    pub gsd_load_state: LoadState,
    pub git_load_state: LoadState,
    pub phase: Option<PhaseProvenance>,
    pub git_repos: Vec<GitRepoReview>,
    pub git_collapsed_dirs: BTreeSet<String>,
    git_active_file: Option<GitFileKey>,
    mouse_capture_suspended: bool,
    pub diff: Option<DiffPayload>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerContext {
    pub project_dir: PathBuf,
    pub phase_id: Option<String>,
    pub workstream: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerRuntime {
    pub context: ReviewerContext,
    pub session: ReviewerSession,
    pub exit_mode: ReviewerExitMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewerExitMode {
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectionKey {
    plan_id: String,
    task_index: Option<usize>,
    repo: Option<String>,
    unmatched: bool,
    file_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitSelectionKey {
    repo: Option<String>,
    commit: Option<String>,
    file_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GitFileKey {
    repo: Option<String>,
    commit: String,
    file_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GitFileTreeRow {
    Dir {
        path: String,
        depth: usize,
        expanded: bool,
    },
    File {
        file_index: usize,
        depth: usize,
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ChangeListRow {
    Plan {
        plan_index: usize,
    },
    Task {
        plan_index: usize,
        task_index: usize,
        flat_group_index: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UiRepoBucket {
    repo: Option<String>,
    unmatched: bool,
    files: Vec<UiFileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UiFileEntry {
    path: String,
    entries: Vec<FileEntry>,
}

/// Stores reviewer data that egui can paint without recomputing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiReviewerPreparedState {
    /// Last complete snapshot consumed by the GUI renderer.
    pub snapshot: GuiReviewerSnapshot,
}

impl Default for GuiReviewerPreparedState {
    fn default() -> Self {
        Self {
            snapshot: GuiReviewerSnapshot::empty(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedReviewer {
    pub header: String,
    pub subheader: String,
    pub columns: [Vec<String>; 4],
    pub column_widths: [usize; 4],
    pub content_height: usize,
    pub viewport_width: usize,
    pub row_hits: Vec<[Option<usize>; 4]>,
    pub footer: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiReviewerRow {
    /// Text already stripped of terminal markers and ready for painting.
    pub label: String,
    /// Whether the row is selected in its column.
    pub selected: bool,
    /// Visual tone already decided by the reviewer event state.
    pub tone: GuiReviewerRowTone,
    /// Optional tree metadata already prepared for file tree rows.
    pub tree: Option<GuiReviewerTreeRow>,
    /// Optional repo script target prepared when the row became current UI data.
    pub script_target: Option<GuiReviewerBranchTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiReviewerRowTone {
    Normal,
    Dirty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuiReviewerTreeRow {
    Dir { depth: usize, expanded: bool },
    File { depth: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiReviewerColumn {
    /// Column title ready for display.
    pub title: String,
    /// Rows already prepared for direct painting.
    pub rows: Vec<GuiReviewerRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiReviewerSnapshot {
    /// Current reviewer mode.
    pub mode: ReviewerMode,
    /// Current active logical column.
    pub active_column: ActiveColumn,
    /// Current diff display mode.
    pub diff_view_mode: DiffViewMode,
    /// Current top diff row.
    pub diff_scroll_row: usize,
    /// Whether a previous full-diff block exists.
    pub can_jump_previous_block: bool,
    /// Whether a next full-diff block exists.
    pub can_jump_next_block: bool,
    /// One-based current full-diff block index.
    pub current_diff_block: usize,
    /// Total full-diff block count.
    pub diff_block_count: usize,
    /// Header text already assembled by event handling.
    pub header: String,
    /// Subheader text already assembled by event handling.
    pub subheader: String,
    /// Footer text already assembled by event handling.
    pub footer: String,
    /// Whether the current area is too narrow for reviewer UI.
    pub too_narrow: bool,
    /// Minimum reviewer width in terminal-style columns.
    pub min_width: u16,
    /// Three structured columns ready for direct painting.
    pub columns: [GuiReviewerColumn; 3],
    /// Diff column title ready for display.
    pub diff_title: String,
    /// Diff rows ready for direct painting.
    pub diff_lines: Vec<GuiDiffLine>,
    /// Longest prepared diff line in characters.
    pub diff_content_chars: usize,
}

impl GuiReviewerSnapshot {
    /// Creates an empty paint-ready snapshot for unloaded reviewer state.
    fn empty() -> Self {
        Self {
            mode: ReviewerMode::Git,
            active_column: ActiveColumn::ChangeGroups,
            diff_view_mode: DiffViewMode::Diff,
            diff_scroll_row: 0,
            can_jump_previous_block: false,
            can_jump_next_block: false,
            current_diff_block: 0,
            diff_block_count: 0,
            header: String::new(),
            subheader: String::new(),
            footer: String::new(),
            too_narrow: false,
            min_width: MIN_REVIEWER_WIDTH,
            columns: std::array::from_fn(|_| GuiReviewerColumn {
                title: String::new(),
                rows: Vec::new(),
            }),
            diff_title: String::new(),
            diff_lines: Vec::new(),
            diff_content_chars: 0,
        }
    }
}

/// Repo target that has already been attached to a visible GUI row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiReviewerBranchTarget {
    /// Human-readable repository label.
    pub label: String,
    /// Repository path relative to the project root when available.
    pub repo: Option<String>,
    /// Absolute repository root used by branch/script commands.
    pub root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiDiffLineKind {
    Context,
    Insert,
    Delete,
    Hunk,
    Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiDiffLine {
    pub kind: GuiDiffLineKind,
    pub text: String,
    pub selected: bool,
    pub target_line: Option<usize>,
    pub single_click_copy: Option<String>,
    pub double_click_copy: Option<String>,
    pub metadata_copy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerHelixTarget {
    pub label: String,
    pub workdir: PathBuf,
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerGitDataRequest {
    /// Project root used by diff loading.
    pub project_dir: PathBuf,
    /// Git repo row that owns the requested data.
    pub repo_index: usize,
    /// Commit row that should have files and diff loaded.
    pub commit_index: usize,
    /// Real git repository root used by gix.
    pub repo_root: PathBuf,
    /// Optional repo path relative to the workspace root.
    pub repo: Option<String>,
    /// Already-loaded commits copied from the UI runtime.
    pub commits: Vec<GitCommitReview>,
    /// Whether all commits for the repo are already loaded.
    pub commits_loaded: bool,
    /// Commit history batch size derived from the visible column height.
    pub row_budget: usize,
    /// Whether the request was caused by the bottom load-more row.
    pub load_more: bool,
    /// Previously selected file path, used to preserve file selection.
    pub active_file_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerGitDataResult {
    /// Git repo row that owns the loaded data.
    pub repo_index: usize,
    /// Commit row that owns the loaded files and diff.
    pub commit_index: usize,
    /// Updated commit list for the repo.
    pub commits: Vec<GitCommitReview>,
    /// Whether all commits for the repo are now loaded.
    pub commits_loaded: bool,
    /// File path selected after files were loaded.
    pub active_file_path: Option<String>,
    /// Diff payload loaded for the active file.
    pub diff: Option<DiffPayload>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiffCopyContext {
    commit: String,
    absolute_path: String,
}

impl ReviewerRuntime {
    #[cfg(test)]
    pub fn load(context: ReviewerContext) -> Result<Self> {
        Self::load_with_exit_mode(context, ReviewerExitMode::Quit)
    }

    pub fn load_with_exit_mode(
        context: ReviewerContext,
        exit_mode: ReviewerExitMode,
    ) -> Result<Self> {
        let (phase, gsd_load_state) = if let Some(phase_id) = context.phase_id.as_deref() {
            match load_phase_provenance(
                &context.project_dir,
                phase_id,
                LoadPhaseOptions {
                    workstream: context.workstream.as_deref(),
                },
            ) {
                Ok(phase) => (Some(phase), LoadState::Ready),
                Err(error) => (None, LoadState::Error(error.to_string())),
            }
        } else {
            (None, LoadState::Ready)
        };
        let (git_repos, git_load_state) = match load_git_review(&context.project_dir) {
            Ok(repos) => (repos, LoadState::Ready),
            Err(error) => (Vec::new(), LoadState::Error(error.to_string())),
        };
        let mut runtime = Self {
            session: ReviewerSession {
                active_column: ActiveColumn::ChangeGroups,
                selection: ReviewerSelection::default(),
                diff_state: ReviewerDiffState::default(),
                gui_state: GuiReviewerPreparedState::default(),
                hovered: None,
                mode: if context.phase_id.is_some() {
                    ReviewerMode::Gsd
                } else {
                    ReviewerMode::Git
                },
                gsd_load_state,
                git_load_state,
                phase,
                git_repos,
                git_collapsed_dirs: BTreeSet::new(),
                git_active_file: None,
                mouse_capture_suspended: false,
                diff: None,
            },
            context,
            exit_mode,
        };
        runtime.clamp_selection();
        runtime.ensure_selected_git_repo_commits(GIT_COMMIT_PAGE_SIZE)?;
        runtime.clamp_selection();
        runtime.ensure_selected_git_commit_files(GIT_COMMIT_PAGE_SIZE)?;
        runtime.ensure_git_active_file();
        runtime.refresh_diff()?;
        runtime.rebuild_gui_state();
        Ok(runtime)
    }

    pub fn gui_snapshot(&self) -> &GuiReviewerSnapshot {
        &self.session.gui_state.snapshot
    }

    /// Rebuilds the GUI snapshot after reviewer data or selection changes.
    fn rebuild_gui_state(&mut self) {
        let copy_context = self.diff_copy_context();
        let diff_lines = self.gui_diff_lines(copy_context.as_ref());
        let diff_content_chars = diff_lines
            .iter()
            .map(|line| line.text.chars().count())
            .max()
            .unwrap_or(0);
        let (current_diff_block, diff_block_count) =
            self.gui_full_diff_block_position().unwrap_or((0, 0));
        self.session.gui_state.snapshot = GuiReviewerSnapshot {
            mode: self.session.mode,
            active_column: self.session.active_column,
            diff_view_mode: self.session.diff_state.view_mode,
            diff_scroll_row: self.gui_diff_scroll_row(),
            can_jump_previous_block: self.can_jump_full_block(true),
            can_jump_next_block: self.can_jump_full_block(false),
            current_diff_block,
            diff_block_count,
            header: self.header_text(),
            subheader: self.status_context(),
            footer: self.footer_text(),
            too_narrow: false,
            min_width: MIN_REVIEWER_WIDTH,
            columns: [self.gui_column(0), self.gui_column(1), self.gui_column(2)],
            diff_title: self.diff_title(),
            diff_lines,
            diff_content_chars,
        };
    }

    /// Updates cheap scroll/block fields without rebuilding row payloads.
    fn sync_gui_snapshot_scroll_state(&mut self) {
        let (current_diff_block, diff_block_count) =
            self.gui_full_diff_block_position().unwrap_or((0, 0));
        let previous = self.can_jump_full_block(true);
        let next = self.can_jump_full_block(false);
        let diff_scroll_row = self.gui_diff_scroll_row();
        let snapshot = &mut self.session.gui_state.snapshot;
        snapshot.mode = self.session.mode;
        snapshot.active_column = self.session.active_column;
        snapshot.diff_view_mode = self.session.diff_state.view_mode;
        snapshot.diff_scroll_row = diff_scroll_row;
        snapshot.can_jump_previous_block = previous;
        snapshot.can_jump_next_block = next;
        snapshot.current_diff_block = current_diff_block;
        snapshot.diff_block_count = diff_block_count;
    }

    /// Converts the current diff body into paint-ready GUI rows.
    fn gui_diff_lines(&mut self, copy_context: Option<&DiffCopyContext>) -> Vec<GuiDiffLine> {
        match self.session.diff_state.view_mode {
            DiffViewMode::Diff => {
                let lines = self.diff_lines();
                lines
                    .iter()
                    .enumerate()
                    .map(|(index, line)| {
                        let block = diff_block_info(&lines, index);
                        gui_diff_line_from_diff_line(
                            line,
                            copy_context,
                            block,
                            index == self.session.diff_state.diff_cursor_row,
                        )
                    })
                    .collect()
            }
            DiffViewMode::Full => {
                let full_lines = self.gui_full_diff_lines();
                full_lines
                    .iter()
                    .enumerate()
                    .map(|(index, line)| {
                        let block = rendered_diff_block_info(&full_lines, index);
                        gui_diff_line_from_rendered(
                            line,
                            copy_context,
                            block,
                            index == self.session.diff_state.full_cursor_row,
                        )
                    })
                    .collect()
            }
        }
    }

    /// Builds unwrapped full-file lines for egui diff painting.
    fn gui_full_diff_lines(&mut self) -> Vec<String> {
        if !self.session.diff_state.full_file_loaded {
            return vec![format!("{CONTEXT_PREFIX}Full file unavailable")];
        }

        let mut rendered = Vec::new();
        let mut jump_render_rows = Vec::new();
        let jump_targets = self.session.diff_state.full_jump_targets.clone();
        for (index, line) in self.session.diff_state.full_lines.iter().enumerate() {
            let line_number = index + 1;
            if jump_targets.contains(&line_number) {
                jump_render_rows.push(rendered.len());
            }
            if let Some(deleted_lines) = self
                .session
                .diff_state
                .full_deleted_before
                .get(&line_number)
            {
                for deleted in deleted_lines {
                    rendered.push(format!("{DELETE_PREFIX}    │ -{deleted}"));
                }
            }
            let changed = self
                .session
                .diff_state
                .full_changed_lines
                .contains(&line_number);
            let prefix = if changed {
                INSERT_PREFIX
            } else {
                CONTEXT_PREFIX
            };
            let marker = if changed { "+" } else { "" };
            rendered.push(format!("{prefix}{line_number:>4}│ {marker}{line}"));
        }
        let trailing_key = self.session.diff_state.full_lines.len() + 1;
        if let Some(deleted_lines) = self
            .session
            .diff_state
            .full_deleted_before
            .get(&trailing_key)
        {
            if jump_targets.contains(&trailing_key) {
                jump_render_rows.push(rendered.len());
            }
            for deleted in deleted_lines {
                rendered.push(format!("{DELETE_PREFIX}    │ -{deleted}"));
            }
        }
        if rendered.is_empty() {
            rendered.push(format!("{CONTEXT_PREFIX}Full file is empty"));
        }
        self.session.diff_state.full_jump_render_rows = jump_render_rows;
        rendered
    }

    pub fn gui_select_row(
        &mut self,
        column: usize,
        row_index: usize,
        commit_row_budget: usize,
    ) -> Result<()> {
        let result = match column {
            0 => {
                self.set_active_column(ActiveColumn::ChangeGroups);
                self.select_group(row_index, commit_row_budget)
            }
            1 => {
                self.set_active_column(ActiveColumn::Repos);
                self.select_repo(row_index, commit_row_budget)
            }
            2 => {
                self.set_active_column(ActiveColumn::Files);
                self.move_file_cursor(row_index)
            }
            3 => {
                self.set_active_column(ActiveColumn::Diff);
                Ok(())
            }
            _ => Ok(()),
        };
        if result.is_ok() {
            self.rebuild_gui_state();
        }
        result
    }

    pub fn gui_load_more_selected_repo_commits(&mut self, row_budget: usize) -> Result<()> {
        self.load_more_selected_git_commits(row_budget)?;
        self.rebuild_gui_state();
        Ok(())
    }

    pub fn gui_select_row_light(
        &mut self,
        column: usize,
        row_index: usize,
        commit_row_budget: usize,
    ) -> bool {
        match column {
            0 => {
                self.set_active_column(ActiveColumn::ChangeGroups);
                self.select_group_light(row_index);
                self.rebuild_gui_state();
                true
            }
            1 => {
                self.set_active_column(ActiveColumn::Repos);
                let load_more = self.session.mode == ReviewerMode::Git
                    && self.selected_git_repo().is_some_and(|repo| {
                        row_index >= repo.commits.len() && !repo.commits_loaded
                    });
                if load_more {
                    return true;
                }
                self.select_repo_light(row_index);
                self.rebuild_gui_state();
                true
            }
            2 => {
                self.set_active_column(ActiveColumn::Files);
                self.select_file_light(row_index);
                self.rebuild_gui_state();
                true
            }
            3 => {
                self.set_active_column(ActiveColumn::Diff);
                self.rebuild_gui_state();
                false
            }
            _ => false,
        }
    }

    pub fn gui_ensure_selected_git_data(&mut self, row_budget: usize) -> Result<()> {
        if self.session.mode == ReviewerMode::Git {
            self.ensure_selected_git_repo_commits(row_budget)?;
            self.ensure_selected_git_commit_files(row_budget)?;
            self.ensure_git_active_file();
        }
        self.refresh_diff()?;
        self.rebuild_gui_state();
        Ok(())
    }

    pub fn gui_git_data_request(
        &self,
        row_budget: usize,
        load_more: bool,
    ) -> Option<ReviewerGitDataRequest> {
        if self.session.mode != ReviewerMode::Git {
            return None;
        }
        let repo_index = self.session.selection.change_row_index;
        let commit_index = self.session.selection.repo_index;
        let repo = self.session.git_repos.get(repo_index)?;
        Some(ReviewerGitDataRequest {
            project_dir: self.context.project_dir.clone(),
            repo_index,
            commit_index,
            repo_root: repo.root.clone(),
            repo: repo.repo.clone(),
            commits: repo.commits.clone(),
            commits_loaded: repo.commits_loaded,
            row_budget,
            load_more,
            active_file_path: self
                .session
                .git_active_file
                .as_ref()
                .map(|key| key.file_path.clone()),
        })
    }

    pub fn gui_apply_git_data_result(&mut self, result: ReviewerGitDataResult) {
        if self.session.mode != ReviewerMode::Git {
            return;
        }
        let Some(repo) = self.session.git_repos.get_mut(result.repo_index) else {
            return;
        };
        repo.commits = result.commits;
        repo.commits_loaded = result.commits_loaded;
        if self.session.selection.change_row_index == result.repo_index
            && self.session.selection.repo_index == result.commit_index
        {
            if let Some(file_path) = result.active_file_path {
                self.session.git_active_file = Some(GitFileKey {
                    repo: repo.repo.clone(),
                    commit: repo
                        .commits
                        .get(result.commit_index)
                        .map(|commit| commit.commit.clone())
                        .unwrap_or_default(),
                    file_path,
                });
            }
            self.session.diff = result.diff;
        }
        self.rebuild_gui_state();
    }

    pub fn gui_collapse_file_tree_to_first_level(&mut self) {
        if self.session.mode != ReviewerMode::Git {
            return;
        }
        self.ensure_selected_git_commit_files(GIT_COMMIT_PAGE_SIZE)
            .ok();
        let dirs = self.git_file_tree_dirs();
        for (path, depth) in dirs {
            let key = self.git_tree_dir_key(&path);
            if depth == 0 {
                self.session.git_collapsed_dirs.remove(&key);
            } else {
                self.session.git_collapsed_dirs.insert(key);
            }
        }
        self.clamp_selection();
        self.ensure_git_active_file();
        self.rebuild_gui_state();
    }

    pub fn gui_next_item(&mut self, width: u16) -> Result<()> {
        let result = self.move_row(1, width);
        if result.is_ok() {
            self.rebuild_gui_state();
        }
        result
    }

    pub fn gui_previous_item(&mut self, width: u16) -> Result<()> {
        let result = self.move_row(-1, width);
        if result.is_ok() {
            self.rebuild_gui_state();
        }
        result
    }

    pub fn gui_next_column(&mut self) {
        self.move_column(1);
        self.rebuild_gui_state();
    }

    pub fn gui_previous_column(&mut self) {
        self.move_column(-1);
        self.rebuild_gui_state();
    }

    pub fn gui_selected_agent_paste_text(&self) -> Option<String> {
        match (self.session.mode, self.session.active_column) {
            (ReviewerMode::Gsd, ActiveColumn::ChangeGroups) => self.selected_gsd_change_copy_text(),
            (ReviewerMode::Gsd, ActiveColumn::Repos) => self.selected_gsd_repo_copy_text(),
            (ReviewerMode::Gsd, ActiveColumn::Files) => self.selected_gsd_file_copy_text(),
            (ReviewerMode::Git, ActiveColumn::ChangeGroups) => self.selected_git_repo_copy_text(),
            (ReviewerMode::Git, ActiveColumn::Repos) => self.selected_git_commit_copy_text(),
            (ReviewerMode::Git, ActiveColumn::Files) => self.selected_git_file_copy_text(),
            (_, ActiveColumn::Diff) => self.selected_diff_copy_text(),
        }
    }

    /// 返回 GUI 快捷键 `d` 使用的当前 diff 元信息。
    pub fn gui_selected_diff_metadata_paste_text(&mut self, width: u16) -> Option<String> {
        if self.session.active_column != ActiveColumn::Diff {
            return None;
        }
        let context = self.diff_copy_context()?;
        match self.session.diff_state.view_mode {
            DiffViewMode::Diff => {
                let row = self.session.diff_state.diff_cursor_row;
                let lines = self.diff_lines();
                let line = lines.get(row)?;
                if matches!(
                    line.kind,
                    DiffLineKind::Hunk | DiffLineKind::Insert | DiffLineKind::Delete
                ) {
                    return diff_block_info(&lines, row)
                        .map(|block| diff_block_metadata_copy_text(&context, &block));
                }
                diff_line_target_line(line).map(|line| single_diff_line_copy_text(&context, line))
            }
            DiffViewMode::Full => {
                self.ensure_full_render_cache(self.diff_scroll_width(width));
                let row = self.session.diff_state.full_cursor_row;
                let line = self.session.diff_state.full_lines.get(row)?;
                if line.starts_with(INSERT_PREFIX) || line.starts_with(DELETE_PREFIX) {
                    return rendered_diff_block_info(&self.session.diff_state.full_lines, row)
                        .map(|block| diff_block_metadata_copy_text(&context, &block));
                }
                parse_rendered_target_line(line)
                    .map(|line| single_diff_line_copy_text(&context, line))
            }
        }
    }

    pub fn gui_set_mode(&mut self, mode: ReviewerMode) -> Result<()> {
        if self.session.mode == mode {
            return Ok(());
        }
        self.session.mode = mode;
        self.clamp_selection();
        self.ensure_selected_git_commit_files(GIT_COMMIT_PAGE_SIZE)?;
        self.ensure_git_active_file();
        self.refresh_diff()?;
        self.rebuild_gui_state();
        Ok(())
    }

    pub fn gui_toggle_full_diff(&mut self, width: u16, height: u16) -> Result<()> {
        self.toggle_full_mode(width, height)?;
        self.rebuild_gui_state();
        Ok(())
    }

    pub fn gui_jump_full_block(&mut self, reverse: bool) {
        self.jump_full_block(reverse);
        self.sync_gui_snapshot_scroll_state();
    }

    pub fn gui_set_diff_scroll_row(&mut self, row: usize, width: u16) {
        let max_scroll = self.max_diff_scroll(self.diff_scroll_width(width));
        let row = row.min(max_scroll);
        match self.session.diff_state.view_mode {
            DiffViewMode::Diff => self.session.diff_state.diff_scroll = row,
            DiffViewMode::Full => self.session.diff_state.full_scroll = row,
        }
        self.sync_gui_snapshot_scroll_state();
    }

    pub fn gui_select_diff_row(&mut self, row: usize, width: u16) {
        self.session.active_column = ActiveColumn::Diff;
        self.set_diff_cursor_row(row, width);
        self.rebuild_gui_state();
    }

    pub fn gui_diff_scroll_row(&self) -> usize {
        match self.session.diff_state.view_mode {
            DiffViewMode::Diff => self.session.diff_state.diff_scroll,
            DiffViewMode::Full => self.session.diff_state.full_scroll,
        }
    }

    pub fn gui_selected_helix_target(&mut self, width: u16) -> Option<ReviewerHelixTarget> {
        self.selected_helix_target(width)
    }

    fn gui_full_diff_block_position(&self) -> Option<(usize, usize)> {
        if self.session.diff_state.view_mode != DiffViewMode::Full {
            return None;
        }
        let rows = &self.session.diff_state.full_jump_render_rows;
        if rows.is_empty() {
            return None;
        }
        let current_row = self.session.diff_state.full_scroll;
        let index = rows
            .iter()
            .copied()
            .take_while(|row| *row <= current_row)
            .count()
            .saturating_sub(1);
        Some((index + 1, rows.len()))
    }

    fn gui_column(&self, column: usize) -> GuiReviewerColumn {
        match column {
            0 => GuiReviewerColumn {
                title: self.first_column_title().to_string(),
                rows: if self.session.mode == ReviewerMode::Git {
                    self.gui_git_repo_rows()
                } else {
                    self.first_column_rows()
                        .into_iter()
                        .map(|row| GuiReviewerRow {
                            label: gui_clean_row_label(&row),
                            selected: row.starts_with("> ") || row.starts_with("* "),
                            tone: GuiReviewerRowTone::Normal,
                            tree: None,
                            script_target: None,
                        })
                        .collect()
                },
            },
            1 => GuiReviewerColumn {
                title: self.second_column_title().to_string(),
                rows: self
                    .second_column_rows()
                    .into_iter()
                    .enumerate()
                    .map(|(index, row)| GuiReviewerRow {
                        label: gui_clean_row_label(&row),
                        selected: row.starts_with("> ") || row.starts_with("* "),
                        tone: GuiReviewerRowTone::Normal,
                        tree: None,
                        script_target: self.gui_repo_script_target(1, index),
                    })
                    .collect(),
            },
            2 if self.session.mode == ReviewerMode::Git => GuiReviewerColumn {
                title: self.third_column_title().to_string(),
                rows: self.gui_git_file_tree_rows(),
            },
            _ => GuiReviewerColumn {
                title: self.third_column_title().to_string(),
                rows: self
                    .third_column_rows()
                    .into_iter()
                    .map(|row| GuiReviewerRow {
                        label: gui_clean_row_label(&row),
                        selected: row.starts_with("> ") || row.starts_with("* "),
                        tone: GuiReviewerRowTone::Normal,
                        tree: None,
                        script_target: None,
                    })
                    .collect(),
            },
        }
    }

    fn gui_git_file_tree_rows(&self) -> Vec<GuiReviewerRow> {
        let Some(_commit) = self.selected_git_commit() else {
            return vec![GuiReviewerRow {
                label: "No files in the selected commit".to_string(),
                selected: false,
                tone: GuiReviewerRowTone::Normal,
                tree: None,
                script_target: None,
            }];
        };
        let rows = self.git_file_tree_rows();
        if rows.is_empty() {
            return vec![GuiReviewerRow {
                label: "No files in the selected commit".to_string(),
                selected: false,
                tone: GuiReviewerRowTone::Normal,
                tree: None,
                script_target: None,
            }];
        }

        rows.iter()
            .enumerate()
            .map(|(index, row)| match row {
                GitFileTreeRow::Dir {
                    path,
                    depth,
                    expanded,
                } => GuiReviewerRow {
                    label: tree_basename(path).to_string(),
                    selected: index == self.session.selection.file_index,
                    tone: GuiReviewerRowTone::Normal,
                    tree: Some(GuiReviewerTreeRow::Dir {
                        depth: *depth,
                        expanded: *expanded,
                    }),
                    script_target: None,
                },
                GitFileTreeRow::File {
                    file_index: _,
                    depth,
                    name,
                } => GuiReviewerRow {
                    label: name.to_string(),
                    selected: index == self.session.selection.file_index,
                    tone: GuiReviewerRowTone::Normal,
                    tree: Some(GuiReviewerTreeRow::File { depth: *depth }),
                    script_target: None,
                },
            })
            .collect()
    }

    fn gui_git_repo_rows(&self) -> Vec<GuiReviewerRow> {
        if self.session.git_repos.is_empty() {
            return vec![GuiReviewerRow {
                label: "No git repos found under this directory".to_string(),
                selected: false,
                tone: GuiReviewerRowTone::Normal,
                tree: None,
                script_target: None,
            }];
        }

        self.session
            .git_repos
            .iter()
            .enumerate()
            .map(|(index, repo)| GuiReviewerRow {
                label: repo.label.clone(),
                selected: index == self.session.selection.change_row_index,
                tone: if git_repo_has_uncommitted_changes(repo) {
                    GuiReviewerRowTone::Dirty
                } else {
                    GuiReviewerRowTone::Normal
                },
                tree: None,
                script_target: self.gui_repo_script_target(0, index),
            })
            .collect()
    }

    /// Returns the row-level script target prepared outside the render path.
    fn gui_repo_script_target(
        &self,
        column: usize,
        row_index: usize,
    ) -> Option<GuiReviewerBranchTarget> {
        match (self.session.mode, column) {
            (ReviewerMode::Git, 0) => {
                let repo = self.session.git_repos.get(row_index)?;
                Some(GuiReviewerBranchTarget {
                    label: repo.label.clone(),
                    repo: repo.repo.clone(),
                    root: repo.root.clone(),
                })
            }
            (ReviewerMode::Gsd, 1) => {
                let repo = self.current_repo_buckets().into_iter().nth(row_index)?;
                Some(GuiReviewerBranchTarget {
                    label: ui_repo_label(&repo),
                    repo: repo.repo.clone(),
                    root: repo_root(&self.context.project_dir, repo.repo.as_deref()),
                })
            }
            _ => None,
        }
    }

    fn change_list_rows(&self) -> Vec<ChangeListRow> {
        let mut rows = Vec::new();
        let mut flat_group_index = 0usize;
        let Some(phase) = self.session.phase.as_ref() else {
            return rows;
        };
        for (plan_index, plan) in phase.plans.iter().enumerate() {
            rows.push(ChangeListRow::Plan { plan_index });
            for task_index in 0..plan.change_groups.len() {
                rows.push(ChangeListRow::Task {
                    plan_index,
                    task_index,
                    flat_group_index,
                });
                flat_group_index += 1;
            }
        }
        rows
    }

    fn change_group_rows(&self) -> Vec<String> {
        let Some(phase) = self.session.phase.as_ref() else {
            return Vec::new();
        };
        self.change_list_rows()
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let label = match row {
                    ChangeListRow::Plan { plan_index } => {
                        let plan = &phase.plans[plan_index];
                        plan.id.clone()
                    }
                    ChangeListRow::Task {
                        plan_index,
                        task_index,
                        ..
                    } => {
                        let group = &phase.plans[plan_index].change_groups[task_index];
                        format!(
                            "  {} {}",
                            group.task_index,
                            compact_task_name(&group.task_name)
                        )
                    }
                };
                format!(
                    "{} {}",
                    selected_prefix(
                        self.session.active_column == ActiveColumn::ChangeGroups,
                        index == self.session.selection.change_row_index,
                        self.session.hovered
                            == Some(HoverTarget {
                                column: 0,
                                row: index
                            }),
                    ),
                    label,
                )
            })
            .collect()
    }

    fn current_repo_buckets(&self) -> Vec<UiRepoBucket> {
        let debug_perf = env::var_os("GSDV_DEBUG_PERF").is_some();
        let started = Instant::now();
        let Some(phase) = self.session.phase.as_ref() else {
            return Vec::new();
        };
        match self.selected_row() {
            Some(ChangeListRow::Plan { plan_index }) => {
                let result = aggregate_repo_buckets(&phase.plans[plan_index].change_groups);
                if debug_perf {
                    eprintln!(
                        "[perf] current_repo_buckets kind=plan:{} repos={} ms={}",
                        phase.plans[plan_index].id,
                        result.len(),
                        started.elapsed().as_millis()
                    );
                }
                result
            }
            Some(ChangeListRow::Task {
                plan_index,
                task_index,
                ..
            }) => {
                let group = &phase.plans[plan_index].change_groups[task_index];
                let result = aggregate_repo_buckets(std::slice::from_ref(group));
                if debug_perf {
                    eprintln!(
                        "[perf] current_repo_buckets kind=task:{}:{} repos={} ms={}",
                        group.plan_id,
                        group.task_index,
                        result.len(),
                        started.elapsed().as_millis()
                    );
                }
                result
            }
            None => {
                if debug_perf {
                    eprintln!(
                        "[perf] current_repo_buckets kind=none repos=0 ms={}",
                        started.elapsed().as_millis()
                    );
                }
                Vec::new()
            }
        }
    }

    fn repo_rows(&self) -> Vec<String> {
        let repo_buckets = self.current_repo_buckets();
        if repo_buckets.is_empty() {
            return vec![EMPTY_REPOS_COPY.to_string()];
        }

        repo_buckets
            .iter()
            .enumerate()
            .map(|(index, bucket)| {
                format!(
                    "{} {}",
                    selected_prefix(
                        self.session.active_column == ActiveColumn::Repos,
                        index == self.session.selection.repo_index,
                        self.session.hovered
                            == Some(HoverTarget {
                                column: 1,
                                row: index
                            }),
                    ),
                    ui_repo_label(bucket),
                )
            })
            .collect()
    }

    fn file_rows(&self) -> Vec<String> {
        let Some(repo) = self.selected_repo() else {
            return vec![EMPTY_FILES_COPY.to_string()];
        };
        if repo.files.is_empty() {
            return vec![EMPTY_FILES_COPY.to_string()];
        }

        repo.files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                format!(
                    "{} {}",
                    selected_prefix(
                        self.session.active_column == ActiveColumn::Files,
                        index == self.session.selection.file_index,
                        self.session.hovered
                            == Some(HoverTarget {
                                column: 2,
                                row: index
                            }),
                    ),
                    display_ui_file_name(file),
                )
            })
            .collect()
    }

    fn git_repo_rows(&self) -> Vec<String> {
        if self.session.git_repos.is_empty() {
            return vec!["No git repos found under this directory".to_string()];
        }

        self.session
            .git_repos
            .iter()
            .enumerate()
            .map(|(index, repo)| {
                format!(
                    "{} {}",
                    selected_prefix(
                        self.session.active_column == ActiveColumn::ChangeGroups,
                        index == self.session.selection.change_row_index,
                        self.session.hovered
                            == Some(HoverTarget {
                                column: 0,
                                row: index
                            }),
                    ),
                    repo.label,
                )
            })
            .collect()
    }

    fn git_commit_rows(&self) -> Vec<String> {
        let Some(repo) = self.selected_git_repo() else {
            return vec!["No commits for the selected repo".to_string()];
        };
        if repo.commits.is_empty() {
            return vec!["No commits for the selected repo".to_string()];
        }

        repo.commits
            .iter()
            .enumerate()
            .map(|(index, commit)| {
                let label = if commit.commit == UNCOMMITTED_COMMIT_ID {
                    "uncommit".to_string()
                } else {
                    format!(
                        "{}.{} {}",
                        commit.author, commit.short_hash, commit.display_time
                    )
                };
                format!(
                    "{} {}",
                    selected_prefix(
                        self.session.active_column == ActiveColumn::Repos,
                        index == self.session.selection.repo_index,
                        self.session.hovered
                            == Some(HoverTarget {
                                column: 1,
                                row: index
                            }),
                    ),
                    label,
                )
            })
            .chain((!repo.commits_loaded).then(|| format!("  {LOAD_MORE_COMMITS_LABEL}")))
            .collect()
    }

    fn git_file_rows(&self) -> Vec<String> {
        let Some(commit) = self.selected_git_commit() else {
            return vec!["No files in the selected commit".to_string()];
        };
        let rows = self.git_file_tree_rows();
        if rows.is_empty() {
            return vec!["No files in the selected commit".to_string()];
        }

        rows.iter()
            .enumerate()
            .map(|(index, row)| {
                let label = match row {
                    GitFileTreeRow::Dir {
                        path,
                        depth,
                        expanded,
                    } => format!(
                        "{}{}{}/",
                        tree_prefix(*depth),
                        if *expanded { "-" } else { "+" },
                        tree_basename(path)
                    ),
                    GitFileTreeRow::File {
                        file_index,
                        depth,
                        name,
                    } => {
                        let prefix = commit
                            .files
                            .get(*file_index)
                            .map(git_working_tree_prefix)
                            .unwrap_or("");
                        format!("{}{}{}", tree_prefix(*depth), prefix, name)
                    }
                };
                format!(
                    "{} {}",
                    selected_prefix(
                        self.session.active_column == ActiveColumn::Files,
                        index == self.session.selection.file_index,
                        self.session.hovered
                            == Some(HoverTarget {
                                column: 2,
                                row: index
                            }),
                    ),
                    label,
                )
            })
            .collect()
    }

    fn first_column_rows(&self) -> Vec<String> {
        match self.session.mode {
            ReviewerMode::Gsd => self.change_group_rows(),
            ReviewerMode::Git => self.git_repo_rows(),
        }
    }

    fn second_column_rows(&self) -> Vec<String> {
        match self.session.mode {
            ReviewerMode::Gsd => self.repo_rows(),
            ReviewerMode::Git => self.git_commit_rows(),
        }
    }

    fn third_column_rows(&self) -> Vec<String> {
        match self.session.mode {
            ReviewerMode::Gsd => self.file_rows(),
            ReviewerMode::Git => self.git_file_rows(),
        }
    }

    fn first_column_title(&self) -> &'static str {
        match self.session.mode {
            ReviewerMode::Gsd => "CHANGE GROUPS",
            ReviewerMode::Git => "REPOS",
        }
    }

    fn second_column_title(&self) -> &'static str {
        match self.session.mode {
            ReviewerMode::Gsd => "REPOS",
            ReviewerMode::Git => "COMMITS",
        }
    }

    fn third_column_title(&self) -> &'static str {
        "FILES"
    }

    fn first_column_len(&self) -> usize {
        match self.session.mode {
            ReviewerMode::Gsd => self.change_list_rows().len(),
            ReviewerMode::Git => self.session.git_repos.len(),
        }
    }

    fn second_column_len(&self) -> usize {
        match self.session.mode {
            ReviewerMode::Gsd => self.current_repo_buckets().len(),
            ReviewerMode::Git => self
                .selected_git_repo()
                .map(|repo| repo.commits.len() + usize::from(!repo.commits_loaded))
                .unwrap_or(0),
        }
    }

    fn third_column_len(&self) -> usize {
        match self.session.mode {
            ReviewerMode::Gsd => self
                .selected_repo()
                .map(|repo| repo.files.len())
                .unwrap_or(0),
            ReviewerMode::Git => self.git_file_tree_rows().len(),
        }
    }

    fn change_groups(&self) -> Vec<&ChangeGroup> {
        self.session
            .phase
            .as_ref()
            .map(|phase| {
                phase
                    .plans
                    .iter()
                    .flat_map(|plan| plan.change_groups.iter())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn selected_row(&self) -> Option<ChangeListRow> {
        self.change_list_rows()
            .into_iter()
            .nth(self.session.selection.change_row_index)
    }

    fn selected_group(&self) -> Option<&ChangeGroup> {
        match self.selected_row() {
            Some(ChangeListRow::Task {
                plan_index,
                task_index,
                ..
            }) => self
                .session
                .phase
                .as_ref()
                .map(|phase| &phase.plans[plan_index].change_groups[task_index]),
            _ => None,
        }
    }

    fn selected_plan(&self) -> Option<&crate::reviewer::PlanProvenance> {
        match self.selected_row() {
            Some(ChangeListRow::Plan { plan_index })
            | Some(ChangeListRow::Task { plan_index, .. }) => self
                .session
                .phase
                .as_ref()
                .map(|phase| &phase.plans[plan_index]),
            None => None,
        }
    }

    fn selected_repo(&self) -> Option<UiRepoBucket> {
        self.current_repo_buckets()
            .into_iter()
            .nth(self.session.selection.repo_index)
    }

    fn selected_file(&self) -> Option<UiFileEntry> {
        self.selected_repo().and_then(|repo| {
            repo.files
                .into_iter()
                .nth(self.session.selection.file_index)
        })
    }

    fn selected_git_repo(&self) -> Option<&GitRepoReview> {
        self.session
            .git_repos
            .get(self.session.selection.change_row_index)
    }

    fn selected_git_commit(&self) -> Option<&GitCommitReview> {
        self.selected_git_repo()
            .and_then(|repo| repo.commits.get(self.session.selection.repo_index))
    }

    pub fn refresh_git_repo_dirty(&mut self, row_index: usize) -> Result<()> {
        let Some(repo) = self.session.git_repos.get_mut(row_index) else {
            return Ok(());
        };
        refresh_repo_dirty_commit(repo)?;
        self.rebuild_gui_state();
        Ok(())
    }

    /// Refreshes Git uncommit rows and the selected working-tree diff.
    pub fn gui_refresh_uncommitted(&mut self) -> Result<()> {
        if self.session.mode != ReviewerMode::Git {
            return Ok(());
        }

        let key = self.git_selection_key();
        for repo in &mut self.session.git_repos {
            refresh_repo_dirty_commit(repo)?;
        }
        // Special logic:
        // Trigger: notify reports a workspace file change while reviewer is open.
        // Why: working-tree diff cache keys do not include file mtimes or content hashes.
        // Prevents: stale uncommit rows and stale diff rows after file edits.
        self.session.diff_state.diff_payload_cache.clear();
        self.restore_git_selection(key);
        if self
            .selected_git_commit()
            .is_some_and(|commit| commit.commit == UNCOMMITTED_COMMIT_ID)
        {
            self.ensure_selected_git_commit_files(GIT_COMMIT_PAGE_SIZE)?;
            self.ensure_git_active_file();
            self.refresh_diff_preserving_diff_position()?;
        }
        self.rebuild_gui_state();
        Ok(())
    }

    fn ensure_selected_git_repo_commits(&mut self, row_budget: usize) -> Result<()> {
        if self.session.mode != ReviewerMode::Git {
            return Ok(());
        }

        let repo_index = self.session.selection.change_row_index;
        let Some(repo) = self.session.git_repos.get_mut(repo_index) else {
            return Ok(());
        };
        if repo.commits_loaded {
            return Ok(());
        }

        refresh_repo_dirty_commit(repo)?;
        let has_uncommit = repo
            .commits
            .iter()
            .any(|commit| commit.commit == UNCOMMITTED_COMMIT_ID);
        let target_history_count = row_budget.saturating_sub(usize::from(has_uncommit)).max(1);
        let loaded_history = repo
            .commits
            .iter()
            .filter(|commit| commit.commit != UNCOMMITTED_COMMIT_ID)
            .count();
        if loaded_history >= target_history_count {
            return Ok(());
        }
        let limit = target_history_count - loaded_history;
        let page = load_git_repo_commit_page(&repo.root, loaded_history, limit)?;
        for commit in page.iter().cloned() {
            if !repo
                .commits
                .iter()
                .any(|existing| existing.commit == commit.commit)
            {
                repo.commits.push(commit);
            }
        }
        repo.commits_loaded = page.len() < limit;
        Ok(())
    }

    fn load_more_selected_git_commits(&mut self, row_budget: usize) -> Result<()> {
        if self.session.mode != ReviewerMode::Git {
            return Ok(());
        }
        let repo_index = self.session.selection.change_row_index;
        let Some(repo) = self.session.git_repos.get_mut(repo_index) else {
            return Ok(());
        };
        if repo.commits_loaded {
            return Ok(());
        }
        let loaded_history = repo
            .commits
            .iter()
            .filter(|commit| commit.commit != UNCOMMITTED_COMMIT_ID)
            .count();
        let limit = row_budget.max(1);
        let page = load_git_repo_commit_page(&repo.root, loaded_history, limit)?;
        for commit in page.iter().cloned() {
            if !repo
                .commits
                .iter()
                .any(|existing| existing.commit == commit.commit)
            {
                repo.commits.push(commit);
            }
        }
        repo.commits_loaded = page.len() < limit;
        Ok(())
    }

    fn ensure_selected_git_commit_files(&mut self, row_budget: usize) -> Result<()> {
        if self.session.mode != ReviewerMode::Git {
            return Ok(());
        }

        let repo_index = self.session.selection.change_row_index;
        let commit_index = self.session.selection.repo_index;
        let needs_commit = self
            .session
            .git_repos
            .get(repo_index)
            .and_then(|repo| repo.commits.get(commit_index))
            .is_none();
        if needs_commit {
            self.ensure_selected_git_repo_commits(row_budget)?;
        }
        let Some(repo) = self.session.git_repos.get_mut(repo_index) else {
            return Ok(());
        };
        let Some(commit) = repo.commits.get(commit_index) else {
            return Ok(());
        };
        if commit.files_loaded {
            return Ok(());
        }

        let repo_root = repo.root.clone();
        let repo_name = repo.repo.clone();
        let commit_id = commit.commit.clone();
        let files = load_git_commit_files(&repo_root, repo_name.as_deref(), &commit_id)?;
        let Some(commit) = repo.commits.get_mut(commit_index) else {
            return Ok(());
        };
        commit.files = files;
        commit.files_loaded = true;
        Ok(())
    }

    fn selected_git_file(&self) -> Option<&GitFileReview> {
        let key = self.session.git_active_file.as_ref()?;
        let repo = self
            .session
            .git_repos
            .iter()
            .find(|repo| repo.repo == key.repo)?;
        let commit = repo
            .commits
            .iter()
            .find(|commit| commit.commit == key.commit)?;
        commit
            .files
            .iter()
            .find(|file| file.display_path == key.file_path)
    }

    fn selected_git_cursor_row(&self) -> Option<GitFileTreeRow> {
        self.git_file_tree_rows()
            .into_iter()
            .nth(self.session.selection.file_index)
    }

    fn git_file_tree_rows(&self) -> Vec<GitFileTreeRow> {
        let Some(commit) = self.selected_git_commit() else {
            return Vec::new();
        };

        let mut rows = Vec::new();
        let mut seen_dirs = BTreeSet::new();
        for (file_index, file) in commit.files.iter().enumerate() {
            let parts = file
                .display_path
                .split('/')
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>();
            if parts.is_empty() {
                continue;
            }

            let mut dir_path = String::new();
            let mut hidden = false;
            for (depth, part) in parts.iter().take(parts.len().saturating_sub(1)).enumerate() {
                if !dir_path.is_empty() {
                    dir_path.push('/');
                }
                dir_path.push_str(part);
                if seen_dirs.insert(dir_path.clone()) && !hidden {
                    let key = self.git_tree_dir_key(&dir_path);
                    let expanded = !self.session.git_collapsed_dirs.contains(&key);
                    rows.push(GitFileTreeRow::Dir {
                        path: dir_path.clone(),
                        depth,
                        expanded,
                    });
                }
                if self
                    .session
                    .git_collapsed_dirs
                    .contains(&self.git_tree_dir_key(&dir_path))
                {
                    hidden = true;
                }
            }

            if !hidden {
                rows.push(GitFileTreeRow::File {
                    file_index,
                    depth: parts.len().saturating_sub(1),
                    name: parts
                        .last()
                        .unwrap_or(&file.display_path.as_str())
                        .to_string(),
                });
            }
        }
        rows
    }

    fn git_file_tree_dirs(&self) -> Vec<(String, usize)> {
        let Some(commit) = self.selected_git_commit() else {
            return Vec::new();
        };

        let mut dirs = Vec::new();
        let mut seen_dirs = BTreeSet::new();
        for file in &commit.files {
            let parts = file
                .display_path
                .split('/')
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>();
            if parts.is_empty() {
                continue;
            }

            let mut dir_path = String::new();
            for (depth, part) in parts.iter().take(parts.len().saturating_sub(1)).enumerate() {
                if !dir_path.is_empty() {
                    dir_path.push('/');
                }
                dir_path.push_str(part);
                if seen_dirs.insert(dir_path.clone()) {
                    dirs.push((dir_path.clone(), depth));
                }
            }
        }
        dirs
    }

    fn git_tree_dir_key(&self, path: &str) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}",
            self.selected_git_repo()
                .and_then(|repo| repo.repo.as_deref())
                .unwrap_or(""),
            self.selected_git_commit()
                .map(|commit| commit.commit.as_str())
                .unwrap_or(""),
            path
        )
    }

    fn ensure_git_active_file(&mut self) {
        if self.session.mode != ReviewerMode::Git {
            return;
        }
        if let Err(error) = self.ensure_selected_git_commit_files(GIT_COMMIT_PAGE_SIZE) {
            self.session.git_load_state = LoadState::Error(error.to_string());
            self.session.git_active_file = None;
            return;
        }

        let current_is_valid = self.selected_git_file().is_some();
        if current_is_valid {
            return;
        }

        let Some(commit) = self.selected_git_commit() else {
            self.session.git_active_file = None;
            return;
        };
        let Some(first_file) = commit.files.first() else {
            self.session.git_active_file = None;
            return;
        };

        let repo = self.selected_git_repo().and_then(|repo| repo.repo.clone());
        let commit_hash = commit.commit.clone();
        let file_path = first_file.display_path.clone();
        let row_index = self
            .git_file_tree_rows()
            .iter()
            .position(|row| {
                matches!(
                    row,
                    GitFileTreeRow::File { file_index, .. }
                        if commit
                            .files
                            .get(*file_index)
                            .map(|file| file.display_path.as_str())
                            == Some(file_path.as_str())
                )
            })
            .unwrap_or(0);
        self.session.git_active_file = Some(GitFileKey {
            repo,
            commit: commit_hash,
            file_path,
        });
        self.session.selection.file_index = row_index;
    }

    fn clamp_selection(&mut self) {
        let row_count = self.first_column_len();
        self.session.selection.change_row_index =
            clamp_index(self.session.selection.change_row_index, row_count);

        let repo_count = self.second_column_len();
        self.session.selection.repo_index =
            clamp_index(self.session.selection.repo_index, repo_count);

        let file_count = self.third_column_len();
        self.session.selection.file_index =
            clamp_index(self.session.selection.file_index, file_count);
    }

    fn refresh_diff(&mut self) -> Result<()> {
        let debug_perf = env::var_os("GSDV_DEBUG_PERF").is_some();
        let started = Instant::now();
        self.session.diff = match self.session.mode {
            ReviewerMode::Gsd => match self.selected_file() {
                Some(file) => {
                    let key = ui_file_cache_key(&file);
                    let payload = if let Some(cached) =
                        self.session.diff_state.diff_payload_cache.get(&key)
                    {
                        cached.clone()
                    } else {
                        let loaded = load_ui_diff(&self.context.project_dir, &file)?;
                        self.session
                            .diff_state
                            .diff_payload_cache
                            .insert(key, loaded.clone());
                        loaded
                    };
                    if debug_perf {
                        eprintln!(
                            "[perf] refresh_diff file={} entries={} total_ms={}",
                            file.path,
                            file.entries.len(),
                            started.elapsed().as_millis()
                        );
                    }
                    Some(payload)
                }
                None => None,
            },
            ReviewerMode::Git => match self.selected_git_file().cloned() {
                Some(file) => {
                    let key = git_file_cache_key(&file);
                    let payload = if let Some(cached) =
                        self.session.diff_state.diff_payload_cache.get(&key)
                    {
                        cached.clone()
                    } else {
                        let loaded = load_diff(&self.context.project_dir, &file.entry)?;
                        self.session
                            .diff_state
                            .diff_payload_cache
                            .insert(key, loaded.clone());
                        loaded
                    };
                    if debug_perf {
                        eprintln!(
                            "[perf] refresh_diff git_file={} total_ms={}",
                            file.display_path,
                            started.elapsed().as_millis()
                        );
                    }
                    Some(payload)
                }
                None => None,
            },
        };
        self.session.diff_state.view_mode = DiffViewMode::Diff;
        self.session.diff_state.diff_render_cache_width = None;
        self.session.diff_state.diff_render_cache.clear();
        self.session.diff_state.diff_cursor_row = 0;
        self.session.diff_state.full_scroll = 0;
        self.session.diff_state.full_cursor_row = 0;
        self.session.diff_state.diff_scroll_before_full = None;
        self.session.diff_state.full_file_loaded = false;
        self.session.diff_state.full_lines.clear();
        self.session.diff_state.full_jump_targets.clear();
        self.session.diff_state.full_jump_render_rows.clear();
        self.session.diff_state.full_changed_lines.clear();
        self.session.diff_state.full_deleted_before.clear();
        self.session.diff_state.full_render_cache_width = None;
        self.session.diff_state.full_render_cache.clear();
        if debug_perf && self.session.diff.is_none() {
            eprintln!(
                "[perf] refresh_diff file=none total_ms={}",
                started.elapsed().as_millis()
            );
        }
        Ok(())
    }

    /// 刷新 diff 内容时保留当前 diff 视图位置。
    fn refresh_diff_preserving_diff_position(&mut self) -> Result<()> {
        let view_mode = self.session.diff_state.view_mode;
        let diff_cursor_row = self.session.diff_state.diff_cursor_row;
        let full_cursor_row = self.session.diff_state.full_cursor_row;
        let diff_scroll = self.session.diff_state.diff_scroll;
        let full_scroll = self.session.diff_state.full_scroll;
        self.refresh_diff()?;
        // Special logic:
        // Trigger: uncommit 文件变化后 notify 自动刷新当前 diff。
        // Why: refresh_diff 用于切换文件时会重置 cursor 到第一行。
        // Prevents: 自动刷新把用户正在看的 diff 行跳回第 0 行。
        self.session.diff_state.view_mode = view_mode;
        if view_mode == DiffViewMode::Full {
            // 触发条件：Full 模式下 uncommit 文件变化触发自动刷新。
            // 不能只恢复 view_mode：refresh_diff 会清空 full 文件缓存。
            // 防止回归：Full 内容先正常显示，再被刷新成 unavailable。
            self.ensure_full_file_loaded()?;
        }
        match view_mode {
            DiffViewMode::Diff => {
                let max_cursor = self.max_diff_cursor_row(MIN_REVIEWER_WIDTH);
                self.session.diff_state.diff_cursor_row = diff_cursor_row.min(max_cursor);
                let max_scroll = self.max_diff_scroll(self.diff_scroll_width(MIN_REVIEWER_WIDTH));
                self.session.diff_state.diff_scroll = diff_scroll.min(max_scroll);
            }
            DiffViewMode::Full => {
                let max_cursor = self.max_diff_cursor_row(MIN_REVIEWER_WIDTH);
                self.session.diff_state.full_cursor_row = full_cursor_row.min(max_cursor);
                let max_scroll = self.max_diff_scroll(self.diff_scroll_width(MIN_REVIEWER_WIDTH));
                self.session.diff_state.full_scroll = full_scroll.min(max_scroll);
            }
        }
        Ok(())
    }

    fn ensure_full_file_loaded(&mut self) -> Result<()> {
        if self.session.diff_state.full_file_loaded {
            return Ok(());
        }
        let payload = match self.session.mode {
            ReviewerMode::Gsd => {
                let Some(file) = self.selected_file() else {
                    return Ok(());
                };
                load_ui_full_file_payload(&self.context.project_dir, &file)?
            }
            ReviewerMode::Git => {
                let Some(file) = self.selected_git_file() else {
                    return Ok(());
                };
                load_full_file_payload(&self.context.project_dir, &file.entry)?
            }
        };
        self.session.diff_state.full_file_loaded = true;
        self.session.diff_state.full_lines = payload.lines;
        self.session.diff_state.full_jump_targets = payload.jump_targets;
        self.session.diff_state.full_changed_lines = payload.changed_lines;
        self.session.diff_state.full_deleted_before = payload.deleted_before;
        self.session.diff_state.full_render_cache_width = None;
        self.session.diff_state.full_render_cache.clear();
        Ok(())
    }

    fn select_group(&mut self, index: usize, commit_row_budget: usize) -> Result<()> {
        let debug_perf = env::var_os("GSDV_DEBUG_PERF").is_some();
        let started = Instant::now();
        let row_count = self.first_column_len();
        if row_count == 0 {
            return Ok(());
        }
        self.session.selection.change_row_index = clamp_index(index, row_count);
        self.session.selection.repo_index = 0;
        self.session.selection.file_index = 0;
        self.session.selection.column_scrolls[1] = 0;
        self.session.selection.column_scrolls[2] = 0;
        self.session.selection.row_offsets[1] = 0;
        self.session.selection.row_offsets[2] = 0;
        self.session.git_active_file = None;
        self.session.diff_state.diff_scroll = 0;
        self.clamp_selection();
        self.ensure_selected_git_repo_commits(commit_row_budget)?;
        self.ensure_selected_git_commit_files(commit_row_budget)?;
        self.ensure_git_active_file();
        let result = self.refresh_diff();
        if debug_perf {
            eprintln!(
                "[perf] select_group row={} total_ms={}",
                index,
                started.elapsed().as_millis()
            );
        }
        result
    }

    fn select_group_light(&mut self, index: usize) {
        let row_count = self.first_column_len();
        if row_count == 0 {
            return;
        }
        self.session.selection.change_row_index = clamp_index(index, row_count);
        self.session.selection.repo_index = 0;
        self.session.selection.file_index = 0;
        self.session.selection.column_scrolls[1] = 0;
        self.session.selection.column_scrolls[2] = 0;
        self.session.selection.row_offsets[1] = 0;
        self.session.selection.row_offsets[2] = 0;
        self.session.git_active_file = None;
        self.session.diff_state.diff_scroll = 0;
        self.session.diff_state.full_scroll = 0;
        self.session.diff_state.diff_cursor_row = 0;
        self.session.diff_state.full_cursor_row = 0;
        self.session.diff_state.view_mode = DiffViewMode::Diff;
        self.session.diff_state.full_file_loaded = false;
        self.session.diff_state.full_lines.clear();
        self.session.diff = None;
        self.clamp_selection();
    }

    fn select_repo(&mut self, index: usize, commit_row_budget: usize) -> Result<()> {
        let repo_count = self.second_column_len();
        if repo_count == 0 {
            return Ok(());
        }
        if self.session.mode == ReviewerMode::Git
            && self
                .selected_git_repo()
                .is_some_and(|repo| index >= repo.commits.len() && !repo.commits_loaded)
        {
            return self.load_more_selected_git_commits(commit_row_budget);
        }
        self.session.selection.repo_index = clamp_index(index, repo_count);
        self.session.selection.file_index = 0;
        self.session.selection.column_scrolls[2] = 0;
        self.session.selection.row_offsets[2] = 0;
        self.session.git_active_file = None;
        self.session.diff_state.diff_scroll = 0;
        self.clamp_selection();
        self.ensure_selected_git_commit_files(commit_row_budget)?;
        self.ensure_git_active_file();
        self.refresh_diff()
    }

    fn select_repo_light(&mut self, index: usize) {
        let repo_count = match self.session.mode {
            ReviewerMode::Git => self
                .selected_git_repo()
                .map(|repo| repo.commits.len())
                .unwrap_or(0),
            ReviewerMode::Gsd => self.current_repo_buckets().len(),
        };
        if repo_count == 0 {
            return;
        }
        self.session.selection.repo_index = clamp_index(index, repo_count);
        self.session.selection.file_index = 0;
        self.session.selection.column_scrolls[2] = 0;
        self.session.selection.row_offsets[2] = 0;
        self.session.git_active_file = None;
        self.session.diff_state.diff_scroll = 0;
        self.session.diff_state.full_scroll = 0;
        self.session.diff_state.diff_cursor_row = 0;
        self.session.diff_state.full_cursor_row = 0;
        self.session.diff_state.view_mode = DiffViewMode::Diff;
        self.session.diff_state.full_file_loaded = false;
        self.session.diff_state.full_lines.clear();
        self.session.diff = None;
        self.clamp_selection();
    }

    fn select_file(&mut self, index: usize) -> Result<()> {
        self.select_file_at(index, true)
    }

    fn move_file_cursor(&mut self, index: usize) -> Result<()> {
        self.select_file_at(index, false)
    }

    fn select_file_light(&mut self, index: usize) {
        let file_count = self.third_column_len();
        if file_count == 0 {
            return;
        }
        self.session.selection.file_index = clamp_index(index, file_count);
        self.clamp_selection();
        if self.session.mode == ReviewerMode::Git {
            match self.selected_git_cursor_row() {
                Some(GitFileTreeRow::Dir { path, .. }) => {
                    let key = self.git_tree_dir_key(&path);
                    if !self.session.git_collapsed_dirs.remove(&key) {
                        self.session.git_collapsed_dirs.insert(key);
                    }
                }
                Some(GitFileTreeRow::File { file_index, .. }) => {
                    let next_key = self
                        .selected_git_repo()
                        .zip(self.selected_git_commit())
                        .and_then(|(repo, commit)| {
                            commit.files.get(file_index).map(|file| GitFileKey {
                                repo: repo.repo.clone(),
                                commit: commit.commit.clone(),
                                file_path: file.display_path.clone(),
                            })
                        });
                    self.session.git_active_file = next_key;
                    self.session.diff = None;
                    self.session.diff_state.diff_scroll = 0;
                    self.session.diff_state.full_scroll = 0;
                    self.session.diff_state.diff_cursor_row = 0;
                    self.session.diff_state.full_cursor_row = 0;
                    self.session.diff_state.view_mode = DiffViewMode::Diff;
                    self.session.diff_state.full_file_loaded = false;
                    self.session.diff_state.full_lines.clear();
                }
                None => {}
            }
        } else {
            self.session.diff = None;
            self.session.diff_state.diff_scroll = 0;
            self.session.diff_state.full_scroll = 0;
            self.session.diff_state.diff_cursor_row = 0;
            self.session.diff_state.full_cursor_row = 0;
            self.session.diff_state.view_mode = DiffViewMode::Diff;
            self.session.diff_state.full_file_loaded = false;
            self.session.diff_state.full_lines.clear();
        }
    }

    fn select_file_at(&mut self, index: usize, toggle_git_dirs: bool) -> Result<()> {
        self.ensure_selected_git_commit_files(GIT_COMMIT_PAGE_SIZE)?;
        let file_count = self.third_column_len();
        if file_count == 0 {
            return Ok(());
        }
        self.session.selection.file_index = clamp_index(index, file_count);
        self.clamp_selection();
        if self.session.mode == ReviewerMode::Git {
            match self.selected_git_cursor_row() {
                Some(GitFileTreeRow::Dir { path, .. }) => {
                    if toggle_git_dirs {
                        let key = self.git_tree_dir_key(&path);
                        if !self.session.git_collapsed_dirs.remove(&key) {
                            self.session.git_collapsed_dirs.insert(key);
                        }
                        self.clamp_selection();
                    }
                    Ok(())
                }
                Some(GitFileTreeRow::File { file_index, .. }) => {
                    let next_key = self
                        .selected_git_repo()
                        .zip(self.selected_git_commit())
                        .and_then(|(repo, commit)| {
                            commit.files.get(file_index).map(|file| GitFileKey {
                                repo: repo.repo.clone(),
                                commit: commit.commit.clone(),
                                file_path: file.display_path.clone(),
                            })
                        });
                    if let Some(next_key) = next_key {
                        self.session.git_active_file = Some(next_key);
                    }
                    self.session.diff_state.diff_scroll = 0;
                    self.refresh_diff()
                }
                None => Ok(()),
            }
        } else {
            self.session.diff_state.diff_scroll = 0;
            self.refresh_diff()
        }
    }

    fn jump_first(&mut self) -> Result<()> {
        match self.session.active_column {
            ActiveColumn::ChangeGroups => self.select_group(0, GIT_COMMIT_PAGE_SIZE),
            ActiveColumn::Repos => self.select_repo(0, GIT_COMMIT_PAGE_SIZE),
            ActiveColumn::Files => self.move_file_cursor(0),
            ActiveColumn::Diff => {
                match self.session.diff_state.view_mode {
                    DiffViewMode::Diff => self.session.diff_state.diff_scroll = 0,
                    DiffViewMode::Full => self.session.diff_state.full_scroll = 0,
                }
                Ok(())
            }
        }
    }

    fn jump_last(&mut self, width: u16) -> Result<()> {
        match self.session.active_column {
            ActiveColumn::ChangeGroups => {
                let last = self.first_column_len().saturating_sub(1);
                self.select_group(last, GIT_COMMIT_PAGE_SIZE)
            }
            ActiveColumn::Repos => {
                let last = self.second_column_len().saturating_sub(1);
                self.select_repo(last, GIT_COMMIT_PAGE_SIZE)
            }
            ActiveColumn::Files => {
                let last = self.third_column_len().saturating_sub(1);
                self.move_file_cursor(last)
            }
            ActiveColumn::Diff => {
                let max_scroll = self.max_diff_scroll(self.diff_scroll_width(width));
                match self.session.diff_state.view_mode {
                    DiffViewMode::Diff => self.session.diff_state.diff_scroll = max_scroll,
                    DiffViewMode::Full => self.session.diff_state.full_scroll = max_scroll,
                }
                Ok(())
            }
        }
    }

    fn move_row(&mut self, delta: isize, width: u16) -> Result<()> {
        match self.session.active_column {
            ActiveColumn::ChangeGroups => {
                let next = offset_index(self.session.selection.change_row_index, delta);
                self.select_group(next, GIT_COMMIT_PAGE_SIZE)?;
                self.ensure_selected_row_visible(0, width);
                Ok(())
            }
            ActiveColumn::Repos => {
                let next = offset_index(self.session.selection.repo_index, delta);
                self.select_repo(next, GIT_COMMIT_PAGE_SIZE)?;
                self.ensure_selected_row_visible(1, width);
                Ok(())
            }
            ActiveColumn::Files => {
                let next = offset_index(self.session.selection.file_index, delta);
                self.move_file_cursor(next)?;
                self.ensure_selected_row_visible(2, width);
                Ok(())
            }
            ActiveColumn::Diff => {
                self.move_diff_cursor(delta, width);
                Ok(())
            }
        }
    }

    fn page_diff(&mut self, delta: isize, width: u16) {
        // Performance contract: scrolling only updates offsets. Do not add any
        // diff/full recomputation work on this path.
        let max_scroll = self.max_diff_scroll(self.diff_scroll_width(width));
        let jump = page_delta(10, delta);
        match self.session.diff_state.view_mode {
            DiffViewMode::Diff => {
                self.session.diff_state.diff_scroll =
                    scroll_offset(self.session.diff_state.diff_scroll, jump, max_scroll);
            }
            DiffViewMode::Full => {
                self.session.diff_state.full_scroll =
                    scroll_offset(self.session.diff_state.full_scroll, jump, max_scroll);
            }
        }
    }

    fn scroll_diff(&mut self, delta: isize, width: u16) {
        // Performance contract: scrolling only updates offsets. Keep heavy
        // rendering/preparation work out of this function.
        let max_scroll = self.max_diff_scroll(self.diff_scroll_width(width));
        match self.session.diff_state.view_mode {
            DiffViewMode::Diff => {
                self.session.diff_state.diff_scroll =
                    scroll_offset(self.session.diff_state.diff_scroll, delta, max_scroll);
            }
            DiffViewMode::Full => {
                self.session.diff_state.full_scroll =
                    scroll_offset(self.session.diff_state.full_scroll, delta, max_scroll);
            }
        }
    }

    fn move_diff_cursor(&mut self, delta: isize, width: u16) {
        let max_row = self.max_diff_cursor_row(width);
        let current = self.current_diff_cursor_row();
        let next = offset_index_with_limit(current, delta, max_row);
        self.set_diff_cursor_row(next, width);
    }

    fn set_diff_cursor_row(&mut self, row: usize, width: u16) {
        let row = row.min(self.max_diff_cursor_row(width));
        match self.session.diff_state.view_mode {
            DiffViewMode::Diff => {
                self.session.diff_state.diff_cursor_row = row;
                self.session.diff_state.diff_scroll = row;
            }
            DiffViewMode::Full => {
                self.session.diff_state.full_cursor_row = row;
                self.session.diff_state.full_scroll = row;
            }
        }
    }

    fn current_diff_cursor_row(&self) -> usize {
        match self.session.diff_state.view_mode {
            DiffViewMode::Diff => self.session.diff_state.diff_cursor_row,
            DiffViewMode::Full => self.session.diff_state.full_cursor_row,
        }
    }

    fn max_diff_cursor_row(&mut self, width: u16) -> usize {
        match self.session.diff_state.view_mode {
            DiffViewMode::Diff => self.diff_lines().len().saturating_sub(1),
            DiffViewMode::Full => self
                .full_render_lines(self.diff_scroll_width(width))
                .len()
                .saturating_sub(1),
        }
    }

    fn move_column(&mut self, delta: isize) {
        self.session.active_column = match delta.cmp(&0) {
            cmp::Ordering::Less => self.session.active_column.previous(),
            cmp::Ordering::Greater => self.session.active_column.next(),
            cmp::Ordering::Equal => self.session.active_column,
        };
    }

    fn set_active_column(&mut self, column: ActiveColumn) {
        self.session.active_column = column;
    }

    fn drill_in(&mut self) {
        self.session.active_column = match self.session.active_column {
            ActiveColumn::ChangeGroups => ActiveColumn::Repos,
            ActiveColumn::Repos => ActiveColumn::Files,
            ActiveColumn::Files => ActiveColumn::Diff,
            ActiveColumn::Diff => ActiveColumn::Diff,
        };
    }

    fn toggle_mode(&mut self) -> Result<()> {
        match self.session.mode {
            ReviewerMode::Gsd => self.session.mode = ReviewerMode::Git,
            ReviewerMode::Git if self.context.phase_id.is_some() => {
                self.session.mode = ReviewerMode::Gsd;
            }
            ReviewerMode::Git => return Ok(()),
        }

        self.session.active_column = ActiveColumn::ChangeGroups;
        self.session.selection = ReviewerSelection::default();
        self.session.hovered = None;
        self.clamp_selection();
        self.refresh_diff()
    }

    fn selection_key(&self) -> Option<SelectionKey> {
        let plan = self.selected_plan()?;
        let group = self.selected_group();
        let repo = self.selected_repo();
        let file = self.selected_file();
        Some(SelectionKey {
            plan_id: plan.id.clone(),
            task_index: group.map(|item| item.task_index),
            repo: repo.as_ref().and_then(|bucket| bucket.repo.clone()),
            unmatched: repo
                .as_ref()
                .map(|bucket| bucket.unmatched)
                .unwrap_or(false),
            file_path: file.map(|entry| entry.path),
        })
    }

    fn git_selection_key(&self) -> Option<GitSelectionKey> {
        let repo = self.selected_git_repo()?;
        Some(GitSelectionKey {
            repo: repo.repo.clone(),
            commit: self
                .selected_git_commit()
                .map(|commit| commit.commit.clone()),
            file_path: self
                .selected_git_file()
                .map(|file| file.display_path.clone()),
        })
    }

    fn selected_gsd_change_copy_text(&self) -> Option<String> {
        match self.selected_row()? {
            ChangeListRow::Plan { plan_index } => {
                let plan = self.session.phase.as_ref()?.plans.get(plan_index)?;
                if plan.title.trim().is_empty() {
                    Some(format!("block:{}", plan.id))
                } else {
                    Some(format!("block:{} {}", plan.id, plan.title))
                }
            }
            ChangeListRow::Task {
                plan_index,
                task_index,
                ..
            } => {
                let group = self
                    .session
                    .phase
                    .as_ref()?
                    .plans
                    .get(plan_index)?
                    .change_groups
                    .get(task_index)?;
                Some(format!(
                    "block:{} task:{} {}",
                    group.plan_id, group.task_index, group.task_name
                ))
            }
        }
    }

    fn selected_gsd_repo_copy_text(&self) -> Option<String> {
        self.selected_repo()
            .map(|repo| format_repo_copy_text(gsd_repo_label(repo.repo.as_deref())))
    }

    fn selected_gsd_file_copy_text(&self) -> Option<String> {
        let repo = self.selected_repo()?;
        let file = self.selected_file()?;
        let entry = file.entries.last()?;
        Some(file_copy_text(
            gsd_repo_label(repo.repo.as_deref()),
            short_diff_source_commit_label(&entry.diff_source),
            &file.path,
        ))
    }

    fn selected_git_repo_copy_text(&self) -> Option<String> {
        self.selected_git_repo()
            .map(|repo| format_repo_copy_text(git_repo_label(repo)))
    }

    fn selected_git_commit_copy_text(&self) -> Option<String> {
        let repo = self.selected_git_repo()?;
        let commit = self.selected_git_commit()?;
        Some(commit_copy_text(
            git_repo_label(repo),
            git_commit_label(commit),
        ))
    }

    fn selected_git_file_copy_text(&self) -> Option<String> {
        let repo = self.selected_git_repo()?;
        let commit = self.selected_git_commit()?;
        let base = commit_copy_text(git_repo_label(repo), git_commit_label(commit));
        match self.selected_git_cursor_row()? {
            GitFileTreeRow::Dir { path, .. } => Some(format!("{base} file:{path}")),
            GitFileTreeRow::File { file_index, .. } => self
                .selected_git_commit()
                .and_then(|commit| commit.files.get(file_index))
                .map(|file| format!("{base} file:{}", file.display_path)),
        }
    }

    fn selected_diff_file_entry(&self) -> Option<FileEntry> {
        match self.session.mode {
            ReviewerMode::Gsd => self
                .selected_file()
                .and_then(|file| file.entries.last().cloned()),
            ReviewerMode::Git => self.selected_git_file().map(|file| file.entry.clone()),
        }
    }

    fn selected_diff_copy_text(&self) -> Option<String> {
        let context = self.diff_copy_context()?;
        let line = match self.session.diff_state.view_mode {
            DiffViewMode::Diff => self
                .diff_lines()
                .get(self.session.diff_state.diff_cursor_row)
                .and_then(diff_line_target_line),
            DiffViewMode::Full => self
                .session
                .diff_state
                .full_render_cache
                .get(self.session.diff_state.full_cursor_row)
                .and_then(|line| parse_rendered_target_line(line)),
        }?;
        Some(single_diff_line_copy_text(&context, line))
    }

    fn diff_copy_context(&self) -> Option<DiffCopyContext> {
        let file = self.selected_diff_file_entry()?;
        let absolute_path = absolute_file_path(&self.context.project_dir, &file.path)
            .to_string_lossy()
            .to_string();
        Some(DiffCopyContext {
            commit: diff_source_commit_label(&file.diff_source),
            absolute_path,
        })
    }

    pub fn reload(&mut self) -> Result<()> {
        let gsd_key = (self.session.mode == ReviewerMode::Gsd)
            .then(|| self.selection_key())
            .flatten();
        let git_key = (self.session.mode == ReviewerMode::Git)
            .then(|| self.git_selection_key())
            .flatten();

        if let Some(phase_id) = self.context.phase_id.as_deref() {
            self.session.gsd_load_state = LoadState::Loading;
            match load_phase_provenance(
                &self.context.project_dir,
                phase_id,
                LoadPhaseOptions {
                    workstream: self.context.workstream.as_deref(),
                },
            ) {
                Ok(phase) => {
                    self.session.phase = Some(phase);
                    self.session.gsd_load_state = LoadState::Ready;
                }
                Err(error) => {
                    self.session.phase = None;
                    self.session.gsd_load_state = LoadState::Error(error.to_string());
                }
            }
        }

        self.session.git_load_state = LoadState::Loading;
        match load_git_review(&self.context.project_dir) {
            Ok(repos) => {
                self.session.git_repos = repos;
                self.session.git_load_state = LoadState::Ready;
            }
            Err(error) => {
                self.session.git_repos.clear();
                self.session.git_load_state = LoadState::Error(error.to_string());
            }
        }

        self.session.diff_state.diff_payload_cache.clear();
        match self.session.mode {
            ReviewerMode::Gsd => self.restore_selection(gsd_key),
            ReviewerMode::Git => self.restore_git_selection(git_key),
        }
        self.ensure_selected_git_commit_files(GIT_COMMIT_PAGE_SIZE)?;
        self.refresh_diff()?;
        self.rebuild_gui_state();
        Ok(())
    }

    fn restore_selection(&mut self, key: Option<SelectionKey>) {
        self.session.selection = ReviewerSelection::default();
        self.clamp_selection();

        let Some(key) = key else {
            return;
        };
        let Some(phase) = self.session.phase.as_ref() else {
            return;
        };

        let rows = self.change_list_rows();
        let target_row = rows.iter().position(|row| match row {
            ChangeListRow::Plan { plan_index } => {
                let plan = &phase.plans[*plan_index];
                key.task_index.is_none() && plan.id == key.plan_id
            }
            ChangeListRow::Task {
                plan_index,
                task_index,
                ..
            } => {
                let group = &phase.plans[*plan_index].change_groups[*task_index];
                key.task_index == Some(group.task_index) && group.plan_id == key.plan_id
            }
        });

        if let Some(row_index) = target_row {
            self.session.selection.change_row_index = row_index;
            let repo_buckets = self.current_repo_buckets();
            let repo_index = repo_buckets
                .iter()
                .position(|bucket| bucket.repo == key.repo && bucket.unmatched == key.unmatched);
            let file_index = repo_index.and_then(|repo_index| {
                key.file_path.as_ref().and_then(|file_path| {
                    repo_buckets[repo_index]
                        .files
                        .iter()
                        .position(|file| &file.path == file_path)
                })
            });

            if let Some(repo_index) = repo_index {
                self.session.selection.repo_index = repo_index;
            }
            if let Some(file_index) = file_index {
                self.session.selection.file_index = file_index;
            }
        }

        self.clamp_selection();
    }

    fn restore_git_selection(&mut self, key: Option<GitSelectionKey>) {
        self.session.selection = ReviewerSelection::default();
        self.clamp_selection();

        let Some(key) = key else {
            return;
        };

        let repo_index = self
            .session
            .git_repos
            .iter()
            .position(|repo| repo.repo == key.repo);
        if let Some(repo_index) = repo_index {
            self.session.selection.change_row_index = repo_index;
            let commit_index = self.session.git_repos[repo_index]
                .commits
                .iter()
                .position(|commit| Some(&commit.commit) == key.commit.as_ref());
            if let Some(commit_index) = commit_index {
                self.session.selection.repo_index = commit_index;
                if let Err(error) = self.ensure_selected_git_commit_files(GIT_COMMIT_PAGE_SIZE) {
                    self.session.git_load_state = LoadState::Error(error.to_string());
                    return;
                }
                if let Some(file_path) = key.file_path.as_ref() {
                    let commit = &self.session.git_repos[repo_index].commits[commit_index];
                    if commit
                        .files
                        .iter()
                        .any(|file| &file.display_path == file_path)
                    {
                        self.session.git_active_file = Some(GitFileKey {
                            repo: self.session.git_repos[repo_index].repo.clone(),
                            commit: commit.commit.clone(),
                            file_path: file_path.clone(),
                        });
                        if let Some(row_index) =
                            self.git_file_tree_rows().iter().position(|row| match row {
                                GitFileTreeRow::File { file_index, .. } => {
                                    commit.files.get(*file_index).map(|file| &file.display_path)
                                        == Some(file_path)
                                }
                                GitFileTreeRow::Dir { .. } => false,
                            })
                        {
                            self.session.selection.file_index = row_index;
                        }
                    }
                }
            }
        }

        self.clamp_selection();
        self.ensure_git_active_file();
    }

    fn diff_lines(&self) -> Vec<DiffLine> {
        match self.session.diff.as_ref().map(|payload| &payload.body) {
            Some(DiffBody::Lines(lines)) => lines.clone(),
            Some(DiffBody::Placeholder(body)) | Some(DiffBody::Error(body)) => body
                .lines()
                .map(|line| DiffLine {
                    kind: DiffLineKind::Context,
                    old_line: None,
                    new_line: None,
                    text: line.to_string(),
                    metadata: None,
                })
                .collect(),
            None => vec![DiffLine {
                kind: DiffLineKind::Context,
                old_line: None,
                new_line: None,
                text: EMPTY_DIFF_COPY.to_string(),
                metadata: None,
            }],
        }
    }

    fn ensure_diff_render_cache(&mut self, width: usize) {
        if self.session.diff_state.diff_render_cache_width == Some(width) {
            return;
        }
        self.session.diff_state.diff_render_cache = self
            .diff_lines()
            .into_iter()
            .flat_map(|line| {
                let prefix = match line.kind {
                    DiffLineKind::Context => CONTEXT_PREFIX,
                    DiffLineKind::Insert => INSERT_PREFIX,
                    DiffLineKind::Delete => DELETE_PREFIX,
                    DiffLineKind::Hunk => HUNK_PREFIX,
                    DiffLineKind::Metadata => CONTEXT_PREFIX,
                };
                let rendered = render_structured_diff_line(&line);
                wrap_text_to_width(&rendered, width.max(1))
                    .into_iter()
                    .map(move |wrapped| format!("{prefix}{wrapped}"))
            })
            .collect();
        self.session.diff_state.diff_render_cache_width = Some(width);
    }

    fn diff_render_lines(&mut self, width: usize) -> &[String] {
        self.ensure_diff_render_cache(width);
        &self.session.diff_state.diff_render_cache
    }

    fn ensure_full_render_cache(&mut self, width: usize) {
        if self.session.diff_state.full_render_cache_width == Some(width) {
            return;
        }

        if !self.session.diff_state.full_file_loaded {
            self.session.diff_state.full_render_cache_width = Some(width);
            self.session.diff_state.full_render_cache = vec!["Full file unavailable".to_string()];
            self.session.diff_state.full_jump_render_rows.clear();
            return;
        }

        let content_width = width.saturating_sub(7).max(1);
        let mut rendered = Vec::new();
        let mut jump_render_rows = Vec::new();
        let jump_targets = self.session.diff_state.full_jump_targets.clone();

        for (index, highlighted) in self.session.diff_state.full_lines.iter().enumerate() {
            let line_number = index + 1;
            if jump_targets.contains(&line_number) {
                jump_render_rows.push(rendered.len());
            }
            if let Some(deleted_lines) = self
                .session
                .diff_state
                .full_deleted_before
                .get(&line_number)
            {
                for deleted in deleted_lines {
                    let wrapped = wrap_text_to_width(deleted, content_width);
                    for (part_index, part) in wrapped.into_iter().enumerate() {
                        let raw = if part_index == 0 {
                            format!("    │ -{}", part)
                        } else {
                            format!("    │  {}", part)
                        };
                        rendered.push(format!("{DELETE_PREFIX}{raw}"));
                    }
                }
            }

            let wrapped = wrap_text_to_width(highlighted, content_width);
            let changed = self
                .session
                .diff_state
                .full_changed_lines
                .contains(&line_number);
            for (part_index, part) in wrapped.into_iter().enumerate() {
                let raw = if part_index == 0 {
                    if changed {
                        format!("{:>4}│ +{}", line_number, part)
                    } else {
                        format!("{:>4}│ {}", line_number, part)
                    }
                } else {
                    format!("    │ {}", part)
                };
                let prefix = if changed {
                    INSERT_PREFIX
                } else {
                    CONTEXT_PREFIX
                };
                rendered.push(format!("{prefix}{raw}"));
            }
        }

        let trailing_key = self.session.diff_state.full_lines.len() + 1;
        if let Some(deleted_lines) = self
            .session
            .diff_state
            .full_deleted_before
            .get(&trailing_key)
        {
            if jump_targets.contains(&trailing_key) {
                jump_render_rows.push(rendered.len());
            }
            for deleted in deleted_lines {
                let wrapped = wrap_text_to_width(deleted, content_width);
                for (part_index, part) in wrapped.into_iter().enumerate() {
                    let raw = if part_index == 0 {
                        format!("    │ -{}", part)
                    } else {
                        format!("    │  {}", part)
                    };
                    rendered.push(format!("{DELETE_PREFIX}{raw}"));
                }
            }
        }

        if rendered.is_empty() {
            rendered.push(format!("{CONTEXT_PREFIX}Full file is empty"));
        }
        self.session.diff_state.full_render_cache_width = Some(width);
        self.session.diff_state.full_jump_render_rows = jump_render_rows;
        self.session.diff_state.full_render_cache = rendered;
    }

    fn full_render_lines(&mut self, width: usize) -> &[String] {
        self.ensure_full_render_cache(width);
        &self.session.diff_state.full_render_cache
    }

    /// Returns the target line for the diff block containing the cursor.
    fn current_diff_cursor_block_anchor_line(&self) -> Option<usize> {
        if self.session.active_column != ActiveColumn::Diff {
            return None;
        }
        let lines = self.diff_lines();
        let row = self
            .session
            .diff_state
            .diff_cursor_row
            .min(lines.len().checked_sub(1)?);
        let start = lines[..=row]
            .iter()
            .rposition(|line| line.kind == DiffLineKind::Hunk)
            .unwrap_or(row);
        diff_line_target_line(lines.get(start)?).or_else(|| diff_line_target_line(lines.get(row)?))
    }

    /// Returns the block anchor nearest to the visible diff viewport top.
    fn current_visible_diff_block_anchor_line(&self) -> Option<usize> {
        let lines = self.diff_lines();
        if lines.is_empty() {
            return None;
        }
        let top = self
            .session
            .diff_state
            .diff_scroll
            .min(lines.len().saturating_sub(1));
        let previous = lines[..=top]
            .iter()
            .rposition(|line| line.kind == DiffLineKind::Hunk);
        let next = lines[top..]
            .iter()
            .position(|line| line.kind == DiffLineKind::Hunk)
            .map(|offset| top + offset);
        let row = match (previous, next) {
            (Some(previous), Some(next)) => {
                if top.saturating_sub(previous) <= next.saturating_sub(top) {
                    previous
                } else {
                    next
                }
            }
            (Some(previous), None) => previous,
            (None, Some(next)) => next,
            (None, None) => top,
        };
        diff_line_target_line(lines.get(row)?)
    }

    fn toggle_full_mode(&mut self, width: u16, height: u16) -> Result<()> {
        match self.session.diff_state.view_mode {
            DiffViewMode::Diff => {
                self.ensure_full_file_loaded()?;
                self.ensure_full_render_cache(self.diff_scroll_width(width));
                self.session.diff_state.diff_scroll_before_full =
                    Some(self.session.diff_state.diff_scroll);
                let trailing_anchor = self.session.diff_state.full_lines.len().saturating_add(1);
                let target_line = self
                    .current_diff_cursor_block_anchor_line()
                    .or_else(|| self.current_visible_diff_block_anchor_line())
                    .unwrap_or(1)
                    .clamp(1, trailing_anchor.max(1));
                self.session.diff_state.full_scroll = self
                    .session
                    .diff_state
                    .full_jump_targets
                    .iter()
                    .position(|line| *line == target_line)
                    .and_then(|index| {
                        self.session
                            .diff_state
                            .full_jump_render_rows
                            .get(index)
                            .copied()
                    })
                    .unwrap_or_else(|| target_line.saturating_sub(1));
                self.session.diff_state.view_mode = DiffViewMode::Full;
            }
            DiffViewMode::Full => {
                if let Some(previous) = self.session.diff_state.diff_scroll_before_full.take() {
                    self.session.diff_state.diff_scroll = previous;
                }
                self.session.diff_state.view_mode = DiffViewMode::Diff;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn full_jump_targets(&self) -> Vec<usize> {
        self.session.diff_state.full_jump_targets.clone()
    }

    fn jump_full_block(&mut self, reverse: bool) {
        if self.session.diff_state.view_mode != DiffViewMode::Full {
            return;
        }
        let rows = self.session.diff_state.full_jump_render_rows.clone();
        if rows.is_empty() {
            return;
        }

        let current_row = self.session.diff_state.full_scroll;
        let current_anchor = rows
            .iter()
            .copied()
            .take_while(|row| *row <= current_row)
            .last()
            .unwrap_or(0);
        let target = if reverse {
            rows.iter().copied().rev().find(|row| *row < current_anchor)
        } else {
            rows.iter().copied().find(|row| *row > current_anchor)
        };

        if let Some(target) = target {
            self.session.diff_state.full_scroll = target;
        }
    }

    fn can_jump_full_block(&self, reverse: bool) -> bool {
        if self.session.diff_state.view_mode != DiffViewMode::Full {
            return false;
        }
        let rows = &self.session.diff_state.full_jump_render_rows;
        if rows.is_empty() {
            return false;
        }
        let current_row = self.session.diff_state.full_scroll;
        let current_anchor = rows
            .iter()
            .copied()
            .take_while(|row| *row <= current_row)
            .last()
            .unwrap_or(0);
        if reverse {
            rows.iter().any(|row| *row < current_anchor)
        } else {
            rows.iter().any(|row| *row > current_anchor)
        }
    }

    fn max_diff_scroll(&mut self, width: usize) -> usize {
        match self.session.diff_state.view_mode {
            DiffViewMode::Diff => self.diff_render_lines(width).len().saturating_sub(1),
            DiffViewMode::Full => self.full_render_lines(width).len().saturating_sub(1),
        }
    }

    fn diff_scroll_width(&self, terminal_width: u16) -> usize {
        let empty: Vec<String> = Vec::new();
        compute_column_widths(terminal_width, &empty, &empty, &empty)[3].saturating_sub(2)
    }

    fn status_context(&self) -> String {
        match self.session.mode {
            ReviewerMode::Gsd => {
                let group = self.selected_group();
                let repo = self.selected_repo();
                let file = self.selected_file();

                format!(
                    "{} {} {} {}",
                    format!(
                        "group {}",
                        match self.selected_row() {
                            Some(ChangeListRow::Plan { plan_index }) => self
                                .session
                                .phase
                                .as_ref()
                                .map(|phase| format!("plan:{}", phase.plans[plan_index].id))
                                .unwrap_or_else(|| "none".to_string()),
                            Some(ChangeListRow::Task { .. }) => group
                                .map(|item| format!("{}:{}", item.task_index, item.task_name))
                                .unwrap_or_else(|| "none".to_string()),
                            None => "none".to_string(),
                        }
                    ),
                    format!(
                        "repo {}",
                        repo.map(|bucket| ui_repo_label(&bucket))
                            .unwrap_or_else(|| "none".to_string())
                    ),
                    format!(
                        "file {}",
                        file.map(|entry| entry.path)
                            .unwrap_or_else(|| "none".to_string())
                    ),
                    format!("diff {}", self.diff_kind_label())
                )
            }
            ReviewerMode::Git => format!(
                "mode git repo {} commit {} file {} diff {}",
                self.selected_git_repo()
                    .map(|repo| repo.label.clone())
                    .unwrap_or_else(|| "none".to_string()),
                self.selected_git_commit()
                    .map(|commit| {
                        format!(
                            "{}.{} {}",
                            commit.author, commit.short_hash, commit.display_time
                        )
                    })
                    .unwrap_or_else(|| "none".to_string()),
                self.selected_git_file()
                    .map(|file| file.display_path.clone())
                    .unwrap_or_else(|| "none".to_string()),
                self.diff_kind_label(),
            ),
        }
    }

    fn footer_text(&self) -> String {
        let mode_hint = match self.session.mode {
            ReviewerMode::Gsd => "g git-mode",
            ReviewerMode::Git if self.context.phase_id.is_some() => "g gsd-mode",
            ReviewerMode::Git => "g git-only",
        };
        let next = if self.can_jump_full_block(false) {
            "\u{1b}[32mn\u{1b}[0m"
        } else {
            "\u{1b}[90mn\u{1b}[0m"
        };
        let prev = if self.can_jump_full_block(true) {
            "\u{1b}[32mN\u{1b}[0m"
        } else {
            "\u{1b}[90mN\u{1b}[0m"
        };
        format!(
            "h/l move column  j/k move row  Shift+Left/Right horizontal  click select  {mode_hint}  Home first  f full/diff  {next}/{prev} jump block  PgUp/PgDn scroll diff  Ctrl+F2 help  r refresh  q {}",
            "quit"
        )
    }

    fn clamp_column_scrolls(&mut self, widths: &[usize; 4], rows: [&[String]; 3]) {
        for index in 0..3 {
            let max_scroll = max_line_width(rows[index]).saturating_sub(widths[index].max(1));
            self.session.selection.column_scrolls[index] =
                self.session.selection.column_scrolls[index].min(max_scroll);
        }
    }

    fn clamp_row_offsets(&mut self, height: usize, rows: [&[String]; 3]) {
        let visible_rows = height.saturating_sub(1);
        for index in 0..3 {
            let max_offset = rows[index].len().saturating_sub(visible_rows);
            self.session.selection.row_offsets[index] =
                self.session.selection.row_offsets[index].min(max_offset);
        }
    }

    fn ensure_selected_row_visible(&mut self, column: usize, terminal_width: u16) {
        if column >= 3 {
            return;
        }
        let height = 24_u16
            .saturating_sub(HEADER_ROWS)
            .saturating_sub(FOOTER_ROWS)
            .max(1) as usize;
        let visible_rows = height.saturating_sub(1).max(1);
        let selected = match column {
            0 => self.session.selection.change_row_index,
            1 => self.session.selection.repo_index,
            2 => self.session.selection.file_index,
            _ => 0,
        };
        let row_count = match column {
            0 => self.first_column_len(),
            1 => self.second_column_len(),
            2 => self.third_column_len(),
            _ => 0,
        };
        let max_offset = row_count.saturating_sub(visible_rows);
        let offset = &mut self.session.selection.row_offsets[column];
        if selected < *offset {
            *offset = selected;
        } else if selected >= offset.saturating_add(visible_rows) {
            *offset = selected.saturating_sub(visible_rows - 1);
        }
        *offset = (*offset).min(max_offset);
        let empty = Vec::new();
        let widths = compute_column_widths(terminal_width, &empty, &empty, &empty);
        let rows = [
            self.first_column_rows(),
            self.second_column_rows(),
            self.third_column_rows(),
        ];
        self.clamp_row_offsets(height, [&rows[0], &rows[1], &rows[2]]);
        self.clamp_column_scrolls(&widths, [&rows[0], &rows[1], &rows[2]]);
    }

    fn scroll_column_horizontal(&mut self, column: usize, delta: isize, terminal_width: u16) {
        if column >= 3 {
            return;
        }
        let rows = match column {
            0 => self.first_column_rows(),
            1 => self.second_column_rows(),
            2 => self.third_column_rows(),
            _ => Vec::new(),
        };
        let empty = Vec::new();
        let widths = compute_column_widths(terminal_width, &empty, &empty, &empty);
        let max_scroll = max_line_width(&rows).saturating_sub(widths[column].max(1));
        self.session.selection.column_scrolls[column] = offset_index_with_limit(
            self.session.selection.column_scrolls[column],
            delta,
            max_scroll,
        );
    }

    fn scroll_active_column_horizontal(&mut self, delta: isize, terminal_width: u16) {
        let column = match self.session.active_column {
            ActiveColumn::ChangeGroups => 0,
            ActiveColumn::Repos => 1,
            ActiveColumn::Files => 2,
            ActiveColumn::Diff => return,
        };
        self.scroll_column_horizontal(column, delta, terminal_width);
    }

    fn current_mode_load_state(&self) -> &LoadState {
        match self.session.mode {
            ReviewerMode::Gsd => &self.session.gsd_load_state,
            ReviewerMode::Git => &self.session.git_load_state,
        }
    }

    fn mode_label(&self) -> &'static str {
        match self.session.mode {
            ReviewerMode::Gsd => "gsd",
            ReviewerMode::Git => "git",
        }
    }

    fn diff_kind_label(&self) -> &'static str {
        match self.session.diff_state.view_mode {
            DiffViewMode::Full => "full",
            DiffViewMode::Diff => self
                .session
                .diff
                .as_ref()
                .map(|payload| match payload.body {
                    DiffBody::Lines(_) => "patch",
                    DiffBody::Placeholder(_) => "fallback",
                    DiffBody::Error(_) => "error",
                })
                .unwrap_or("empty"),
        }
    }

    fn header_text(&self) -> String {
        match self.session.mode {
            ReviewerMode::Gsd => format!(
                "gsdv gsd {} phase {} {}",
                self.context
                    .workstream
                    .as_deref()
                    .map(|ws| format!("ws:{ws}"))
                    .unwrap_or_else(|| "ws:-".to_string()),
                self.context.phase_id.as_deref().unwrap_or("-"),
                self.selected_group()
                    .map(|group| format!("task {}/{}", group.task_index, group.task_name))
                    .unwrap_or_else(|| "task -".to_string())
            ),
            ReviewerMode::Git => format!(
                "gsdv git root {} repo {} commit {}",
                self.context.project_dir.display(),
                self.selected_git_repo()
                    .map(|repo| repo.label.clone())
                    .unwrap_or_else(|| "-".to_string()),
                self.selected_git_commit()
                    .map(|commit| commit.short_hash.clone())
                    .unwrap_or_else(|| "-".to_string()),
            ),
        }
    }

    pub fn render_model(&mut self, width: u16, height: u16) -> RenderedReviewer {
        if width < MIN_REVIEWER_WIDTH {
            return RenderedReviewer {
                header: "Reviewer needs a wider terminal".to_string(),
                subheader: format!("Minimum width: {MIN_REVIEWER_WIDTH} columns"),
                columns: [vec![], vec![], vec![], vec![]],
                column_widths: [0; 4],
                content_height: 0,
                viewport_width: width as usize,
                row_hits: vec![],
                footer: self.footer_text(),
            };
        }

        if matches!(self.current_mode_load_state(), LoadState::Loading) {
            return RenderedReviewer {
                header: format!("Loading {} reviewer...", self.mode_label()),
                subheader: String::new(),
                columns: [vec![], vec![], vec![], vec![]],
                column_widths: [0; 4],
                content_height: 0,
                viewport_width: width as usize,
                row_hits: vec![],
                footer: self.footer_text(),
            };
        }

        if let LoadState::Error(message) = self.current_mode_load_state() {
            return RenderedReviewer {
                header: "Reviewer data could not be loaded.".to_string(),
                subheader: message.clone(),
                columns: [vec![], vec![], vec![], vec![]],
                column_widths: [0; 4],
                content_height: 0,
                viewport_width: width as usize,
                row_hits: vec![],
                footer: self.footer_text(),
            };
        }

        let content_height = height
            .saturating_sub(HEADER_ROWS)
            .saturating_sub(FOOTER_ROWS)
            .max(1) as usize;

        let raw_first = self.first_column_rows();
        let raw_second = self.second_column_rows();
        let raw_third = self.third_column_rows();
        let column_widths = compute_column_widths(width, &raw_first, &raw_second, &raw_third);
        self.clamp_row_offsets(content_height, [&raw_first, &raw_second, &raw_third]);
        self.clamp_column_scrolls(&column_widths, [&raw_first, &raw_second, &raw_third]);
        let (first, first_hits) = match self.session.mode {
            ReviewerMode::Gsd => self.render_change_groups(
                content_height,
                column_widths[0],
                self.session.selection.row_offsets[0],
            ),
            ReviewerMode::Git => self.render_simple_column(
                self.first_column_title(),
                &raw_first,
                0,
                content_height,
                column_widths[0],
                self.session.selection.row_offsets[0],
            ),
        };
        let (second, second_hits) = self.render_simple_column(
            self.second_column_title(),
            &raw_second,
            1,
            content_height,
            column_widths[1],
            self.session.selection.row_offsets[1],
        );
        let (third, third_hits) = self.render_simple_column(
            self.third_column_title(),
            &raw_third,
            2,
            content_height,
            column_widths[2],
            self.session.selection.row_offsets[2],
        );
        let row_hits = build_row_hits(content_height, &first_hits, &second_hits, &third_hits);
        RenderedReviewer {
            header: self.header_text(),
            subheader: self.status_context(),
            columns: [
                first,
                second,
                third,
                self.render_diff(content_height, column_widths[3]),
            ],
            column_widths,
            content_height,
            viewport_width: width as usize,
            row_hits,
            footer: self.footer_text(),
        }
    }

    fn render_change_groups(
        &self,
        height: usize,
        width: usize,
        row_offset: usize,
    ) -> (Vec<String>, Vec<Option<usize>>) {
        let rows = self.change_group_rows();
        if rows.is_empty() {
            return (
                fill_column_height(
                    vec!["CHANGE GROUPS".to_string(), "No change groups".to_string()],
                    height,
                ),
                fill_hit_height(vec![None, None], height),
            );
        }

        let content_width = width.max(1);
        let mut lines = vec!["CHANGE GROUPS".to_string()];
        let mut hits = vec![None];
        let scroll = self.session.selection.column_scrolls[0];
        let start = row_offset.min(rows.len().saturating_sub(1));
        for (index, row) in rows.into_iter().enumerate().skip(start) {
            let wrapped = wrap_change_group_row(&horizontal_slice(&row, scroll), content_width);
            for part in wrapped {
                if lines.len() >= height {
                    break;
                }
                lines.push(part);
                hits.push(Some(index));
            }
        }
        (
            fill_column_height(lines, height),
            fill_hit_height(hits, height),
        )
    }

    fn render_simple_column(
        &self,
        title: &str,
        rows: &[String],
        column_index: usize,
        height: usize,
        width: usize,
        row_offset: usize,
    ) -> (Vec<String>, Vec<Option<usize>>) {
        let mut lines = vec![title.to_string()];
        let mut hits = vec![None];
        let width = width.max(1);
        let scroll = self.session.selection.column_scrolls[column_index];
        let start = row_offset.min(rows.len().saturating_sub(1));
        for (index, row) in rows.iter().enumerate().skip(start) {
            if lines.len() >= height {
                break;
            }
            lines.push(fit_line_to_width(&horizontal_slice(row, scroll), width));
            hits.push(Some(index));
        }
        (
            fill_column_height(lines, height),
            fill_hit_height(hits, height),
        )
    }

    fn render_diff(&mut self, height: usize, width: usize) -> Vec<String> {
        let title = self.diff_title();
        let mut lines = vec![title];
        let prefix = if self.session.active_column == ActiveColumn::Diff {
            "> "
        } else {
            "  "
        };
        let message_lines = self.commit_message_render_lines(width.saturating_sub(2));
        let message_height = message_panel_height(height, message_lines.len());
        lines.extend(message_lines.into_iter().take(message_height));
        let remaining_height = height.saturating_sub(lines.len());

        match self.session.diff_state.view_mode {
            DiffViewMode::Diff => {
                let diff_scroll = self.session.diff_state.diff_scroll;
                let body_lines = self.diff_render_lines(width.saturating_sub(2));
                let start = diff_scroll.min(body_lines.len().saturating_sub(1));
                lines.extend(
                    body_lines
                        .iter()
                        .skip(start)
                        .take(remaining_height)
                        .map(|line| format_diff_line(prefix, line)),
                );
            }
            DiffViewMode::Full => {
                let full_scroll = self.session.diff_state.full_scroll;
                let body_lines = self.full_render_lines(width.saturating_sub(2));
                let start = full_scroll.min(body_lines.len().saturating_sub(1));
                lines.extend(
                    body_lines
                        .iter()
                        .skip(start)
                        .take(remaining_height)
                        .map(|line| format_diff_line(prefix, line)),
                );
            }
        }

        if lines.len() == 1 {
            lines.push(EMPTY_DIFF_COPY.to_string());
        }
        fill_column_height(lines, height)
    }

    fn diff_title(&self) -> String {
        match self.session.mode {
            ReviewerMode::Git => self
                .selected_git_commit()
                .map(|commit| format!("DIFF {}", commit.full_author))
                .unwrap_or_else(|| "DIFF".to_string()),
            ReviewerMode::Gsd => "DIFF".to_string(),
        }
    }

    fn commit_message_render_lines(&self, width: usize) -> Vec<String> {
        if self.session.mode != ReviewerMode::Git {
            return Vec::new();
        }
        let Some(commit) = self.selected_git_commit() else {
            return Vec::new();
        };
        if commit.message.trim().is_empty() {
            return Vec::new();
        }

        let mut lines = Vec::new();
        for line in commit.message.lines() {
            lines.extend(wrap_text_to_width(line, width.max(1)));
        }
        lines
    }

    fn scroll_column_under_mouse(&mut self, column: usize, delta: isize, width: u16) -> Result<()> {
        self.session.active_column = match column {
            0 => ActiveColumn::ChangeGroups,
            1 => ActiveColumn::Repos,
            2 => ActiveColumn::Files,
            _ => ActiveColumn::Diff,
        };

        match column {
            0 => {
                let next = offset_index(self.session.selection.change_row_index, delta);
                self.select_group(next, GIT_COMMIT_PAGE_SIZE)?;
                self.ensure_selected_row_visible(0, width);
                Ok(())
            }
            1 => {
                let next = offset_index(self.session.selection.repo_index, delta);
                self.select_repo(next, GIT_COMMIT_PAGE_SIZE)?;
                self.ensure_selected_row_visible(1, width);
                Ok(())
            }
            2 => {
                let next = offset_index(self.session.selection.file_index, delta);
                self.move_file_cursor(next)?;
                self.ensure_selected_row_visible(2, width);
                Ok(())
            }
            3 => {
                self.session.active_column = ActiveColumn::Diff;
                self.move_diff_cursor(delta, width);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub fn selected_repo_for_branch_action(&self) -> Option<(String, Option<String>, PathBuf)> {
        match self.session.mode {
            ReviewerMode::Git => {
                let repo = self.selected_git_repo()?;
                Some((
                    repo.label.clone(),
                    repo.repo.clone(),
                    repo_root(&self.context.project_dir, repo.repo.as_deref()),
                ))
            }
            ReviewerMode::Gsd => {
                let repo = self.selected_repo()?;
                Some((
                    ui_repo_label(&repo),
                    repo.repo.clone(),
                    repo_root(&self.context.project_dir, repo.repo.as_deref()),
                ))
            }
        }
    }

    #[cfg(test)]
    pub fn repo_script_target(
        &self,
        column: usize,
        row_index: usize,
    ) -> Option<(String, Option<String>, PathBuf)> {
        match (self.session.mode, column) {
            (ReviewerMode::Git, 0) => {
                let repo = self.session.git_repos.get(row_index)?;
                Some((repo.label.clone(), repo.repo.clone(), repo.root.clone()))
            }
            (ReviewerMode::Gsd, 1) => {
                let repo = self.current_repo_buckets().into_iter().nth(row_index)?;
                Some((
                    ui_repo_label(&repo),
                    repo.repo.clone(),
                    repo_root(&self.context.project_dir, repo.repo.as_deref()),
                ))
            }
            _ => None,
        }
    }

    fn selected_helix_target(&mut self, width: u16) -> Option<ReviewerHelixTarget> {
        match (self.session.mode, self.session.active_column) {
            (ReviewerMode::Gsd, ActiveColumn::ChangeGroups)
            | (ReviewerMode::Gsd, ActiveColumn::Repos) => {
                let repo = self.selected_repo()?;
                Some(repo_helix_target(
                    &self.context.project_dir,
                    repo.repo.as_deref(),
                    ui_repo_label(&repo),
                ))
            }
            (ReviewerMode::Gsd, ActiveColumn::Files) => {
                let repo = self.selected_repo()?;
                let file = self.selected_file()?;
                Some(file_helix_target(
                    &self.context.project_dir,
                    repo.repo.as_deref(),
                    file.path,
                    None,
                ))
            }
            (ReviewerMode::Gsd, ActiveColumn::Diff) => {
                let file = self.selected_diff_file_entry()?;
                let repo = diff_source_repo(&file.diff_source);
                let line = self.selected_diff_target_line(width);
                Some(file_helix_target(
                    &self.context.project_dir,
                    repo,
                    file.path,
                    line,
                ))
            }
            (ReviewerMode::Git, ActiveColumn::ChangeGroups)
            | (ReviewerMode::Git, ActiveColumn::Repos) => {
                let repo = self.selected_git_repo()?;
                Some(repo_helix_target(
                    &self.context.project_dir,
                    repo.repo.as_deref(),
                    repo.label.clone(),
                ))
            }
            (ReviewerMode::Git, ActiveColumn::Files) => {
                let repo = self.selected_git_repo()?;
                let file = self.selected_git_file()?;
                Some(file_helix_target(
                    &self.context.project_dir,
                    repo.repo.as_deref(),
                    file.entry.path.clone(),
                    None,
                ))
            }
            (ReviewerMode::Git, ActiveColumn::Diff) => {
                let file = self.selected_diff_file_entry()?;
                let repo = diff_source_repo(&file.diff_source)
                    .map(ToOwned::to_owned)
                    .or_else(|| self.selected_git_repo().and_then(|repo| repo.repo.clone()));
                let line = self.selected_diff_target_line(width);
                Some(file_helix_target(
                    &self.context.project_dir,
                    repo.as_deref(),
                    file.path,
                    line,
                ))
            }
        }
    }

    fn selected_diff_target_line(&mut self, width: u16) -> Option<usize> {
        match self.session.diff_state.view_mode {
            DiffViewMode::Diff => {
                let row = self.session.diff_state.diff_cursor_row;
                self.diff_lines().get(row).and_then(diff_line_target_line)
            }
            DiffViewMode::Full => {
                let row = self.session.diff_state.full_cursor_row;
                let render_width = self.diff_scroll_width(width);
                self.full_render_lines(render_width)
                    .get(row)
                    .and_then(|line| parse_rendered_target_line(line))
            }
        }
    }

    pub fn mode(&self) -> ReviewerMode {
        self.session.mode
    }
}

fn gui_clean_row_label(value: &str) -> String {
    let stripped = strip_all_ansi(value).replace(CLEAR_EOL_SENTINEL, "");
    for prefix in ["> ", "* ", "· ", "  "] {
        if let Some(rest) = stripped.strip_prefix(prefix) {
            return rest.to_string();
        }
    }
    stripped
}

fn gui_diff_line_from_rendered(
    value: &str,
    copy_context: Option<&DiffCopyContext>,
    block_info: Option<String>,
    selected: bool,
) -> GuiDiffLine {
    let (kind, body) = if let Some(rest) = value.strip_prefix(INSERT_PREFIX) {
        (GuiDiffLineKind::Insert, rest)
    } else if let Some(rest) = value.strip_prefix(DELETE_PREFIX) {
        (GuiDiffLineKind::Delete, rest)
    } else if let Some(rest) = value.strip_prefix(HUNK_PREFIX) {
        (GuiDiffLineKind::Hunk, rest)
    } else if let Some(rest) = value.strip_prefix(CONTEXT_PREFIX) {
        (GuiDiffLineKind::Context, rest)
    } else {
        (GuiDiffLineKind::Context, value)
    };
    let text = strip_all_ansi(body).replace(CLEAR_EOL_SENTINEL, "");
    let line_number = parse_display_line_number(&text);
    let single_click_copy = copy_context
        .zip(line_number)
        .map(|(context, line)| single_diff_line_copy_text(context, line));
    let block_copy = copy_context
        .zip(block_info.as_deref())
        .map(|(context, block)| diff_block_copy_text(context, block));
    let metadata_copy = copy_context
        .zip(block_info.as_deref())
        .map(|(context, block)| diff_block_metadata_copy_text(context, block));
    let double_click_copy = if matches!(
        kind,
        GuiDiffLineKind::Hunk | GuiDiffLineKind::Insert | GuiDiffLineKind::Delete
    ) {
        block_copy.or_else(|| single_click_copy.clone())
    } else {
        single_click_copy.clone()
    };
    GuiDiffLine {
        kind,
        text,
        selected,
        target_line: line_number,
        single_click_copy,
        double_click_copy,
        metadata_copy,
    }
}

fn gui_diff_line_from_diff_line(
    line: &DiffLine,
    copy_context: Option<&DiffCopyContext>,
    block_info: Option<String>,
    selected: bool,
) -> GuiDiffLine {
    let kind = match line.kind {
        DiffLineKind::Context => GuiDiffLineKind::Context,
        DiffLineKind::Insert => GuiDiffLineKind::Insert,
        DiffLineKind::Delete => GuiDiffLineKind::Delete,
        DiffLineKind::Hunk => GuiDiffLineKind::Hunk,
        DiffLineKind::Metadata => GuiDiffLineKind::Metadata,
    };
    let line_number = diff_line_target_line(line);
    let single_click_copy = copy_context
        .zip(line_number)
        .map(|(context, line)| single_diff_line_copy_text(context, line));
    let block_copy = copy_context
        .zip(block_info.as_deref())
        .map(|(context, block)| diff_block_copy_text(context, block));
    let metadata_copy = copy_context
        .zip(block_info.as_deref())
        .map(|(context, block)| diff_block_metadata_copy_text(context, block));
    let double_click_copy = if matches!(
        line.kind,
        DiffLineKind::Hunk | DiffLineKind::Insert | DiffLineKind::Delete
    ) {
        block_copy.or_else(|| single_click_copy.clone())
    } else {
        single_click_copy.clone()
    };
    GuiDiffLine {
        kind,
        text: render_structured_diff_line(line),
        selected,
        target_line: line_number,
        single_click_copy,
        double_click_copy,
        metadata_copy,
    }
}

fn single_diff_line_copy_text(context: &DiffCopyContext, line: usize) -> String {
    format!(
        "commit:{} {}:{}",
        context.commit, context.absolute_path, line
    )
}

fn diff_block_copy_text(context: &DiffCopyContext, block_info: &str) -> String {
    format!(
        "repo:{} commit:{}\n{}",
        context.absolute_path, context.commit, block_info
    )
}

/// Builds diff block metadata without copying changed code lines.
fn diff_block_metadata_copy_text(context: &DiffCopyContext, block_info: &str) -> String {
    let hunk_lines = block_info
        .lines()
        .filter(|line| line.contains("@@"))
        .collect::<Vec<_>>();
    if hunk_lines.is_empty() {
        format!("repo:{} commit:{}", context.absolute_path, context.commit)
    } else {
        format!(
            "repo:{} commit:{}\n{}",
            context.absolute_path,
            context.commit,
            hunk_lines.join("\n")
        )
    }
}

fn diff_source_commit_label(source: &DiffSource) -> String {
    match source {
        DiffSource::CommitBacked { commit, .. } => commit.clone(),
        DiffSource::WorkingTree { .. } => "uncommit".to_string(),
        DiffSource::NonCommitFallback { .. } => "unknown".to_string(),
    }
}

fn diff_source_repo(source: &DiffSource) -> Option<&str> {
    match source {
        DiffSource::CommitBacked { repo, .. } | DiffSource::WorkingTree { repo, .. } => {
            repo.as_deref()
        }
        DiffSource::NonCommitFallback { .. } => None,
    }
}

fn diff_line_target_line(line: &DiffLine) -> Option<usize> {
    match line.kind {
        DiffLineKind::Delete => line.old_line,
        DiffLineKind::Insert | DiffLineKind::Context | DiffLineKind::Hunk => {
            line.new_line.or(line.old_line)
        }
        DiffLineKind::Metadata => None,
    }
}

fn parse_rendered_target_line(line: &str) -> Option<usize> {
    let body = line
        .strip_prefix(INSERT_PREFIX)
        .or_else(|| line.strip_prefix(DELETE_PREFIX))
        .or_else(|| line.strip_prefix(HUNK_PREFIX))
        .or_else(|| line.strip_prefix(CONTEXT_PREFIX))
        .unwrap_or(line);
    parse_display_line_number(&strip_all_ansi(body).replace(CLEAR_EOL_SENTINEL, ""))
}

fn repo_helix_target(
    project_root: &Path,
    repo: Option<&str>,
    label: String,
) -> ReviewerHelixTarget {
    ReviewerHelixTarget {
        label,
        workdir: repo_root(project_root, repo),
        file: None,
        line: None,
    }
}

fn file_helix_target(
    project_root: &Path,
    repo: Option<&str>,
    path: String,
    line: Option<usize>,
) -> ReviewerHelixTarget {
    let file = helix_repo_relative_path(&path, repo);
    ReviewerHelixTarget {
        label: path,
        workdir: repo_root(project_root, repo),
        file: Some(PathBuf::from(file)),
        line,
    }
}

fn helix_repo_relative_path(path: &str, repo: Option<&str>) -> String {
    let Some(repo) = repo.filter(|repo| !repo.is_empty()) else {
        return path.to_string();
    };
    path.strip_prefix(repo)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(path)
        .to_string()
}

fn short_diff_source_commit_label(source: &DiffSource) -> String {
    match source {
        DiffSource::CommitBacked { commit, .. } => short_commit_label(commit),
        DiffSource::WorkingTree { .. } => "uncommit".to_string(),
        DiffSource::NonCommitFallback { .. } => "unknown".to_string(),
    }
}

fn short_commit_label(commit: &str) -> String {
    if commit == UNCOMMITTED_COMMIT_ID {
        "uncommit".to_string()
    } else {
        commit.chars().take(7).collect()
    }
}

fn git_commit_label(commit: &GitCommitReview) -> String {
    if commit.commit == UNCOMMITTED_COMMIT_ID {
        "uncommit".to_string()
    } else if commit.short_hash.is_empty() {
        short_commit_label(&commit.commit)
    } else {
        commit.short_hash.clone()
    }
}

fn git_repo_label(repo: &GitRepoReview) -> &str {
    repo.repo.as_deref().unwrap_or("root")
}

fn git_repo_has_uncommitted_changes(repo: &GitRepoReview) -> bool {
    repo.commits
        .first()
        .is_some_and(|commit| commit.commit == UNCOMMITTED_COMMIT_ID)
}

fn refresh_repo_dirty_commit(repo: &mut GitRepoReview) -> Result<()> {
    let dirty = load_git_dirty_commit(&repo.root, repo.repo.as_deref())?;
    repo.commits
        .retain(|commit| commit.commit != UNCOMMITTED_COMMIT_ID);
    if let Some(dirty) = dirty {
        repo.commits.insert(0, dirty);
    }
    Ok(())
}

fn gsd_repo_label(repo: Option<&str>) -> &str {
    repo.unwrap_or("root")
}

fn format_repo_copy_text(repo: &str) -> String {
    format!("repo:{repo}")
}

fn commit_copy_text(repo: &str, commit: String) -> String {
    format!("repo:{repo} commit:{commit}")
}

fn file_copy_text(repo: &str, commit: String, file: &str) -> String {
    format!("repo:{repo} commit:{commit} file:{file}")
}

fn absolute_file_path(project_root: &Path, file_path: &str) -> PathBuf {
    let path = Path::new(file_path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    }
}

fn diff_block_info(lines: &[DiffLine], index: usize) -> Option<String> {
    let line = lines.get(index)?;
    if !matches!(line.kind, DiffLineKind::Insert | DiffLineKind::Delete) {
        return None;
    }
    let start = lines[..=index]
        .iter()
        .rposition(|line| line.kind == DiffLineKind::Hunk)
        .unwrap_or(index);
    let end = lines[index + 1..]
        .iter()
        .position(|line| line.kind == DiffLineKind::Hunk)
        .map(|offset| index + 1 + offset)
        .unwrap_or(lines.len());
    let block = lines[start..end]
        .iter()
        .map(render_structured_diff_line)
        .collect::<Vec<_>>()
        .join("\n");
    (!block.is_empty()).then_some(block)
}

fn rendered_diff_block_info(lines: &[String], index: usize) -> Option<String> {
    let line = lines.get(index)?;
    let changed = line.starts_with(INSERT_PREFIX) || line.starts_with(DELETE_PREFIX);
    if !changed {
        return None;
    }
    let is_changed =
        |value: &str| value.starts_with(INSERT_PREFIX) || value.starts_with(DELETE_PREFIX);
    let mut start = index;
    while start > 0 && is_changed(&lines[start - 1]) {
        start -= 1;
    }
    let mut end = index + 1;
    while end < lines.len() && is_changed(&lines[end]) {
        end += 1;
    }
    let block = lines[start..end]
        .iter()
        .map(|line| gui_diff_line_from_rendered(line, None, None, false).text)
        .collect::<Vec<_>>()
        .join("\n");
    (!block.is_empty()).then_some(block)
}

fn selected_prefix(active: bool, selected: bool, hovered: bool) -> &'static str {
    if selected && active {
        ">"
    } else if hovered {
        "·"
    } else if selected {
        "*"
    } else {
        " "
    }
}

#[cfg(test)]
fn repo_label(bucket: &crate::reviewer::RepoBucket) -> String {
    if let Some(repo) = bucket.repo.as_deref() {
        repo.to_string()
    } else {
        "root".to_string()
    }
}

fn repo_root(project_root: &Path, repo: Option<&str>) -> PathBuf {
    repo.map(|repo| project_root.join(repo))
        .unwrap_or_else(|| project_root.to_path_buf())
}

pub(crate) fn ensure_clean_repo(repo_root: &Path) -> Result<()> {
    crate::reviewer::git_backend::ensure_clean_repo(repo_root)
}

pub(crate) fn load_branch_choices(repo_root: &Path) -> Result<(String, Vec<BranchInfo>)> {
    crate::reviewer::git_backend::load_branch_choices(repo_root)
}

pub(crate) fn checkout_branch(repo_root: &Path, checkout: &BranchCheckout) -> Result<()> {
    crate::reviewer::git_backend::checkout_branch(repo_root, checkout)
}

fn compact_task_name(value: &str) -> String {
    let trimmed = value.trim();
    if let Some(rest) = trimmed.strip_prefix("Task ") {
        if let Some((_, remainder)) = rest.split_once(':') {
            return remainder.trim().to_string();
        }
    }
    trimmed.to_string()
}

fn ui_repo_label(bucket: &UiRepoBucket) -> String {
    if let Some(repo) = bucket.repo.as_deref() {
        repo.to_string()
    } else {
        "root".to_string()
    }
}

fn display_ui_file_name(file: &UiFileEntry) -> String {
    Path::new(&file.path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&file.path)
        .to_string()
}

fn tree_basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn tree_prefix(depth: usize) -> String {
    if depth == 0 {
        String::new()
    } else {
        format!("{}|", " ".repeat(depth.saturating_sub(1)))
    }
}

fn aggregate_repo_buckets(groups: &[ChangeGroup]) -> Vec<UiRepoBucket> {
    let mut buckets: BTreeMap<(Option<String>, bool), BTreeMap<String, Vec<FileEntry>>> =
        BTreeMap::new();
    for group in groups {
        for bucket in &group.repo_buckets {
            let key = (bucket.repo.clone(), bucket.unmatched);
            let files = buckets.entry(key).or_default();
            for file in &bucket.files {
                files
                    .entry(file.path.clone())
                    .or_default()
                    .push(file.clone());
            }
        }
    }

    buckets
        .into_iter()
        .map(|((repo, unmatched), files)| UiRepoBucket {
            repo,
            unmatched,
            files: files
                .into_iter()
                .map(|(path, entries)| UiFileEntry { path, entries })
                .collect(),
        })
        .collect()
}

fn load_ui_diff(project_root: &Path, file: &UiFileEntry) -> Result<DiffPayload> {
    let debug_perf = env::var_os("GSDV_DEBUG_PERF").is_some();
    let started = Instant::now();
    if file.entries.len() == 1 {
        let payload = load_diff(project_root, &file.entries[0]);
        if debug_perf {
            eprintln!(
                "[perf] load_ui_diff path={} mode=single entries=1 ms={}",
                file.path,
                started.elapsed().as_millis()
            );
        }
        return payload;
    }

    let mut jump_targets = Vec::new();
    let mut parts = Vec::new();
    for entry in &file.entries {
        let payload = load_diff(project_root, entry)?;
        jump_targets.extend(payload.jump_targets);
        match payload.body {
            DiffBody::Lines(mut lines) => parts.append(&mut lines),
            DiffBody::Placeholder(body) | DiffBody::Error(body) => {
                parts.extend(body.lines().map(|line| DiffLine {
                    kind: DiffLineKind::Context,
                    old_line: None,
                    new_line: None,
                    text: line.to_string(),
                    metadata: None,
                }));
            }
        }
    }
    jump_targets.sort_unstable();
    jump_targets.dedup();

    let payload = DiffPayload {
        title: file.path.clone(),
        jump_targets,
        body: DiffBody::Lines(parts),
    };
    if debug_perf {
        eprintln!(
            "[perf] load_ui_diff path={} mode=aggregate entries={} ms={}",
            file.path,
            file.entries.len(),
            started.elapsed().as_millis()
        );
    }
    Ok(payload)
}

fn load_ui_full_file_payload(
    project_root: &Path,
    file: &UiFileEntry,
) -> Result<crate::reviewer::FullFilePayload> {
    if file.entries.len() == 1 {
        return load_full_file_payload(project_root, &file.entries[0]);
    }

    let payload = load_full_file_payload(
        project_root,
        file.entries.last().expect("aggregated file has entries"),
    )?;
    Ok(payload)
}

fn ui_file_cache_key(file: &UiFileEntry) -> String {
    let mut parts = vec![file.path.clone()];
    for entry in &file.entries {
        parts.push(entry.path.clone());
        match &entry.diff_source {
            crate::reviewer::DiffSource::CommitBacked { commit, repo } => {
                parts.push(format!("c:{commit}:{}", repo.clone().unwrap_or_default()));
            }
            crate::reviewer::DiffSource::WorkingTree {
                repo,
                staged,
                unstaged,
                untracked,
            } => {
                parts.push(format!(
                    "w:{}:{staged}:{unstaged}:{untracked}",
                    repo.clone().unwrap_or_default()
                ));
            }
            crate::reviewer::DiffSource::NonCommitFallback { hint } => {
                parts.push(format!("f:{hint}"));
            }
        }
    }
    parts.join("|")
}

fn git_file_cache_key(file: &GitFileReview) -> String {
    let mut parts = vec![file.display_path.clone(), file.entry.path.clone()];
    match &file.entry.diff_source {
        crate::reviewer::DiffSource::CommitBacked { commit, repo } => {
            parts.push(format!("c:{commit}:{}", repo.clone().unwrap_or_default()));
        }
        crate::reviewer::DiffSource::WorkingTree {
            repo,
            staged,
            unstaged,
            untracked,
        } => {
            parts.push(format!(
                "w:{}:{staged}:{unstaged}:{untracked}",
                repo.clone().unwrap_or_default()
            ));
        }
        crate::reviewer::DiffSource::NonCommitFallback { hint } => {
            parts.push(format!("f:{hint}"));
        }
    }
    parts.join("|")
}

fn git_working_tree_prefix(file: &GitFileReview) -> &'static str {
    match &file.entry.diff_source {
        crate::reviewer::DiffSource::WorkingTree {
            staged,
            unstaged,
            untracked,
            ..
        } => match (*staged, *unstaged, *untracked) {
            (_, _, true) => "[??] ",
            (true, true, false) => "[SM] ",
            (true, false, false) => "[S] ",
            (false, true, false) => "[M] ",
            _ => "[?] ",
        },
        _ => "",
    }
}

fn compute_column_widths(
    width: u16,
    _change_groups: &[String],
    _repos: &[String],
    _files: &[String],
) -> [usize; 4] {
    let available = width as usize - (COLUMN_COUNT - 1) * COLUMN_SEPARATOR.len();
    let groups = available * 15 / 100;
    let repos = available * 125 / 1000;
    let files = available * 125 / 1000;
    let diff = available.saturating_sub(groups + repos + files).max(1);
    [groups.max(1), repos.max(1), files.max(1), diff]
}

fn max_line_width(lines: &[String]) -> usize {
    lines
        .iter()
        .map(|line| {
            strip_all_ansi(line)
                .replace(CLEAR_EOL_SENTINEL, "")
                .chars()
                .count()
        })
        .max()
        .unwrap_or(0)
}

fn horizontal_slice(value: &str, scroll: usize) -> String {
    value.chars().skip(scroll).collect()
}

fn fit_line_to_width(value: &str, width: usize) -> String {
    value.chars().take(width).collect()
}

fn message_panel_height(total_height: usize, message_line_count: usize) -> usize {
    if total_height <= 1 || message_line_count == 0 {
        return 0;
    }
    let available = total_height.saturating_sub(1);
    let max_panel = (total_height / 5).max(1);
    message_line_count.min(available).min(max_panel)
}

fn fill_column_height(mut lines: Vec<String>, height: usize) -> Vec<String> {
    while lines.len() < height {
        lines.push(String::new());
    }
    lines.truncate(height);
    lines
}

fn build_row_hits(
    height: usize,
    change_hits: &[Option<usize>],
    repo_hits: &[Option<usize>],
    file_hits: &[Option<usize>],
) -> Vec<[Option<usize>; 4]> {
    (0..height)
        .map(|screen_row| {
            [
                change_hits.get(screen_row).copied().unwrap_or(None),
                repo_hits.get(screen_row).copied().unwrap_or(None),
                file_hits.get(screen_row).copied().unwrap_or(None),
                None,
            ]
        })
        .collect()
}

fn fill_hit_height(mut hits: Vec<Option<usize>>, height: usize) -> Vec<Option<usize>> {
    while hits.len() < height {
        hits.push(None);
    }
    hits.truncate(height);
    hits
}

fn wrap_plain_text_to_width(value: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let chars = value.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut start = 0usize;
    while start < chars.len() {
        let end = (start + width).min(chars.len());
        lines.push(chars[start..end].iter().collect::<String>());
        start = end;
    }
    lines
}

fn wrap_change_group_row(value: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }

    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= width {
        return vec![value.to_string()];
    }

    let indent = chars
        .iter()
        .position(|ch| ch.is_alphabetic())
        .or_else(|| chars.iter().position(|ch| !ch.is_whitespace()))
        .unwrap_or(0);
    let indent_spaces = " ".repeat(indent);
    let body = chars[indent..].iter().collect::<String>();

    let mut lines = Vec::new();
    lines.push(chars[..width.min(chars.len())].iter().collect::<String>());

    let available = width.saturating_sub(indent).max(1);
    let body_chars = body.chars().collect::<Vec<_>>();
    let mut start = width.min(chars.len()).saturating_sub(indent);
    while start < body_chars.len() {
        let end = (start + available).min(body_chars.len());
        let segment = body_chars[start..end].iter().collect::<String>();
        lines.push(format!("{indent_spaces}{segment}"));
        start = end;
    }

    lines
}

fn wrap_text_to_width(value: &str, width: usize) -> Vec<String> {
    wrap_ansi_text_to_width(&strip_unsafe_ansi(value), width)
}

fn strip_unsafe_ansi(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }

        if matches!(chars.peek(), Some('[')) {
            let seq = parse_csi_sequence(&mut chars);
            if seq.ends_with('m') {
                out.push_str(&seq);
            } else if seq.ends_with('K') {
                out.push(CLEAR_EOL_SENTINEL);
            }
        }
    }

    out
}

fn wrap_ansi_text_to_width(value: &str, width: usize) -> Vec<String> {
    if width == 0 || value.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    let mut visible = 0usize;
    let mut active_sgr = String::new();
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && matches!(chars.peek(), Some('[')) {
            let seq = parse_csi_sequence(&mut chars);
            current.push_str(&seq);
            if seq == "\u{1b}[0m" {
                active_sgr.clear();
            } else if seq.ends_with('m') {
                active_sgr.push_str(&seq);
            }
            continue;
        }

        if ch == CLEAR_EOL_SENTINEL {
            while visible < width {
                current.push(' ');
                visible += 1;
            }
            continue;
        }

        let char_width = if ch == '\t' {
            tab_display_width(visible)
        } else {
            display_char_width(ch)
        };
        if char_width > 0 && visible + char_width > width {
            if !active_sgr.is_empty() && !current.ends_with("\u{1b}[0m") {
                current.push_str("\u{1b}[0m");
            }
            lines.push(current);
            current = active_sgr.clone();
            visible = 0;
        }
        if ch == '\t' {
            let tab_width = tab_display_width(visible);
            current.push_str(&" ".repeat(tab_width));
            visible += tab_width;
        } else if char_width > 0 {
            current.push(ch);
            visible += char_width;
        }
    }

    if !current.is_empty() {
        if !active_sgr.is_empty() && !current.ends_with("\u{1b}[0m") {
            current.push_str("\u{1b}[0m");
        }
        lines.push(current);
    }

    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

fn display_char_width(ch: char) -> usize {
    match ch {
        '\t' => 4,
        '\n' | '\r' => 0,
        _ if ch.is_ascii() => 1,
        _ => 2,
    }
}

fn tab_display_width(visible: usize) -> usize {
    let tab_stop = 4;
    tab_stop - (visible % tab_stop)
}

fn parse_csi_sequence(chars: &mut Peekable<Chars<'_>>) -> String {
    let mut seq = String::from("\u{1b}[");
    chars.next();
    while let Some(next) = chars.next() {
        seq.push(next);
        if ('@'..='~').contains(&next) {
            break;
        }
    }
    seq
}

fn column_hit(column: u16, widths: &[usize; 4]) -> Option<usize> {
    let mut start = 0usize;
    let target = column as usize;
    for (index, width) in widths.iter().enumerate() {
        let end = start + *width;
        if (start..end).contains(&target) {
            return Some(index);
        }
        start = end + COLUMN_SEPARATOR.len();
    }
    None
}

fn hover_row_index(row: u16, content_height: usize) -> Option<usize> {
    let content_row = row.checked_sub(HEADER_ROWS)?;
    let index = content_row as usize;
    (index < content_height).then_some(index)
}

fn format_diff_line(prefix: &str, line: &str) -> String {
    if let Some(sgr) = leading_sgr(line) {
        format!("{sgr}{prefix}{line}\u{1b}[0m")
    } else {
        format!("{prefix}{line}")
    }
}

fn leading_sgr(value: &str) -> Option<&str> {
    if !value.starts_with('\u{1b}') {
        return None;
    }
    let end = value.find('m')?;
    value.get(..=end)
}

fn parse_display_line_number(line: &str) -> Option<usize> {
    let plain = strip_all_ansi(line);
    let plain = strip_diff_line_prefix(&plain);
    let sep = plain.find('│').or_else(|| plain.find(':'))?;
    let number = plain[..sep].trim();
    if number.is_empty() {
        None
    } else {
        number.parse().ok()
    }
}

fn top_visible_diff_anchor_line(lines: &[String]) -> Option<usize> {
    lines
        .iter()
        .find_map(|line| parse_visible_diff_anchor(line))
}

fn parse_visible_diff_anchor(line: &str) -> Option<usize> {
    parse_display_line_number(line).or_else(|| parse_hunk_header_display_start(line))
}

fn parse_hunk_header_display_start(line: &str) -> Option<usize> {
    let plain = strip_all_ansi(line);
    let plain = strip_diff_line_prefix(&plain);

    let leading_digits = plain
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if !leading_digits.is_empty() {
        let rest = &plain[leading_digits.len()..];
        if rest.starts_with(':') {
            return leading_digits.parse().ok();
        }
    }

    let marker = plain.find("@@ -")?;
    let rest = &plain[marker + "@@ -".len()..];
    let digits = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

#[cfg(test)]
fn parse_hunk_header_start(line: &str) -> Option<usize> {
    let plain = strip_all_ansi(line);
    let plain = strip_diff_line_prefix(&plain);

    let leading_digits = plain
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if !leading_digits.is_empty() {
        let rest = &plain[leading_digits.len()..];
        if rest.starts_with(':') || rest.starts_with('│') {
            return leading_digits.parse().ok();
        }
    }

    let marker = plain.find("@@ -")?;
    let rest = &plain[marker + 4..];
    let plus = rest.find('+')?;
    let after_plus = &rest[plus + 1..];
    let digits = after_plus
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

fn render_structured_diff_line(line: &DiffLine) -> String {
    match line.kind {
        DiffLineKind::Context | DiffLineKind::Hunk => line.text.clone(),
        DiffLineKind::Insert => {
            let number = line.new_line.unwrap_or_default();
            format!("{number:>4}│ +{}", line.text)
        }
        DiffLineKind::Delete => {
            let number = line.old_line.unwrap_or_default();
            format!("{number:>4}│ -{}", line.text)
        }
        DiffLineKind::Metadata => line
            .metadata
            .as_ref()
            .map(render_diff_metadata)
            .unwrap_or_default(),
    }
}

fn render_diff_metadata(metadata: &DiffLineMetadata) -> String {
    match metadata {
        DiffLineMetadata::Added { mode } => format!("new file mode {mode}"),
        DiffLineMetadata::Deleted { mode } => format!("deleted file mode {mode}"),
        DiffLineMetadata::ModeChanged { old_mode, new_mode } => {
            format!("old mode {old_mode} -> new mode {new_mode}")
        }
        DiffLineMetadata::Renamed {
            from,
            to,
            old_mode,
            new_mode,
        } => format!("rename {from} -> {to} ({old_mode} -> {new_mode})"),
    }
}

fn strip_diff_line_prefix<'a>(line: &'a str) -> &'a str {
    line.strip_prefix("> ")
        .or_else(|| line.strip_prefix("  "))
        .unwrap_or(line)
}

fn strip_all_ansi(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }

        if matches!(chars.peek(), Some('[')) {
            chars.next();
            while let Some(next) = chars.next() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
        }
    }

    out
}

#[cfg(test)]
fn display_width_without_ansi(value: &str) -> usize {
    let mut visible = 0usize;
    for ch in strip_all_ansi(value).chars() {
        let width = if ch == '\t' {
            tab_display_width(visible)
        } else {
            display_char_width(ch)
        };
        visible += width;
    }
    visible
}

fn clamp_index(index: usize, len: usize) -> usize {
    if len == 0 { 0 } else { index.min(len - 1) }
}

fn offset_index(index: usize, delta: isize) -> usize {
    if delta.is_negative() {
        index.saturating_sub(delta.unsigned_abs())
    } else {
        index.saturating_add(delta as usize)
    }
}

fn offset_index_with_limit(index: usize, delta: isize, limit: usize) -> usize {
    clamp_index(offset_index(index, delta), limit.saturating_add(1))
}

pub fn load_reviewer_git_data(request: ReviewerGitDataRequest) -> Result<ReviewerGitDataResult> {
    let mut commits = request.commits;
    let mut commits_loaded = request.commits_loaded;
    if !commits_loaded {
        let has_uncommit = commits
            .iter()
            .any(|commit| commit.commit == UNCOMMITTED_COMMIT_ID);
        let loaded_history = commits
            .iter()
            .filter(|commit| commit.commit != UNCOMMITTED_COMMIT_ID)
            .count();
        let target_history_count = if request.load_more {
            loaded_history.saturating_add(request.row_budget.max(1))
        } else {
            request
                .row_budget
                .saturating_sub(usize::from(has_uncommit))
                .max(1)
        };
        if loaded_history < target_history_count {
            let limit = target_history_count - loaded_history;
            let page = load_git_repo_commit_page(&request.repo_root, loaded_history, limit)?;
            for commit in page.iter().cloned() {
                if !commits
                    .iter()
                    .any(|existing| existing.commit == commit.commit)
                {
                    commits.push(commit);
                }
            }
            commits_loaded = page.len() < limit;
        }
    }

    let mut active_file_path = request.active_file_path;
    let mut diff = None;
    if let Some(commit) = commits.get_mut(request.commit_index) {
        if !commit.files_loaded {
            commit.files =
                load_git_commit_files(&request.repo_root, request.repo.as_deref(), &commit.commit)?;
            commit.files_loaded = true;
        }
        let active_file = active_file_path
            .as_deref()
            .and_then(|path| commit.files.iter().find(|file| file.display_path == path))
            .or_else(|| commit.files.first());
        if let Some(file) = active_file {
            active_file_path = Some(file.display_path.clone());
            diff = Some(load_diff(&request.project_dir, &file.entry)?);
        }
    }

    Ok(ReviewerGitDataResult {
        repo_index: request.repo_index,
        commit_index: request.commit_index,
        commits,
        commits_loaded,
        active_file_path,
        diff,
    })
}

#[cfg(test)]
#[path = "app_test.rs"]
mod app_test;

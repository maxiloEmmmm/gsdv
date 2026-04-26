use super::*;
use crate::reviewer::{
    CommitProvenance, DiffSource, FileEntry, PlanProvenance, ProvenanceStatus, RepoBucket,
};

#[test]
fn git_gui_file_column_exposes_tree_metadata_without_terminal_prefixes() {
    let runtime = ReviewerRuntime {
        context: ReviewerContext {
            project_dir: PathBuf::new(),
            phase_id: None,
            workstream: None,
        },
        session: ReviewerSession {
            active_column: ActiveColumn::Files,
            selection: ReviewerSelection::default(),
            diff_state: ReviewerDiffState::default(),
            gui_state: GuiReviewerPreparedState::default(),
            hovered: None,
            mode: ReviewerMode::Git,
            gsd_load_state: LoadState::Ready,
            git_load_state: LoadState::Ready,
            phase: None,
            git_repos: vec![GitRepoReview {
                label: ".".to_string(),
                repo: None,
                root: PathBuf::new(),
                commits: vec![GitCommitReview {
                    commit: "abc1234".to_string(),
                    short_hash: "abc1234".to_string(),
                    author: "tests".to_string(),
                    full_author: "tests.user".to_string(),
                    display_time: "04/25-12:00".to_string(),
                    message: "fixture".to_string(),
                    files: vec![git_file("src/main.rs"), git_file("src/lib.rs")],
                    files_loaded: true,
                }],
                commits_loaded: true,
            }],
            git_collapsed_dirs: BTreeSet::new(),
            git_active_file: None,
            mouse_capture_suspended: false,
            diff: None,
        },
        exit_mode: ReviewerExitMode::Quit,
    };

    let column = runtime.gui_column(2);

    assert_eq!(column.rows[0].label, "src");
    assert!(matches!(
        column.rows[0].tree,
        Some(GuiReviewerTreeRow::Dir {
            depth: 0,
            expanded: true
        })
    ));
    assert_eq!(column.rows[1].label, "main.rs");
    assert!(matches!(
        column.rows[1].tree,
        Some(GuiReviewerTreeRow::File { depth: 1 })
    ));
}

#[test]
fn git_gui_file_column_omits_working_tree_status_prefixes() {
    let runtime = ReviewerRuntime {
        context: ReviewerContext {
            project_dir: PathBuf::new(),
            phase_id: None,
            workstream: None,
        },
        session: ReviewerSession {
            active_column: ActiveColumn::Files,
            selection: ReviewerSelection::default(),
            diff_state: ReviewerDiffState::default(),
            gui_state: GuiReviewerPreparedState::default(),
            hovered: None,
            mode: ReviewerMode::Git,
            gsd_load_state: LoadState::Ready,
            git_load_state: LoadState::Ready,
            phase: None,
            git_repos: vec![GitRepoReview {
                label: ".".to_string(),
                repo: None,
                root: PathBuf::new(),
                commits: vec![GitCommitReview {
                    commit: "working-tree".to_string(),
                    short_hash: "worktree".to_string(),
                    author: "working tree".to_string(),
                    full_author: "working tree".to_string(),
                    display_time: "now".to_string(),
                    message: "Working tree".to_string(),
                    files: vec![working_tree_file("docs/new.md", true)],
                    files_loaded: true,
                }],
                commits_loaded: true,
            }],
            git_collapsed_dirs: BTreeSet::new(),
            git_active_file: None,
            mouse_capture_suspended: false,
            diff: None,
        },
        exit_mode: ReviewerExitMode::Quit,
    };

    let column = runtime.gui_column(2);

    assert_eq!(column.rows[0].label, "docs");
    assert_eq!(column.rows[1].label, "new.md");
}

#[test]
fn git_file_tree_collapse_to_first_level_opens_only_depth_zero_dirs() {
    let mut runtime = reviewer_runtime_with_git_files();
    runtime.session.git_repos[0].commits[0].files = vec![
        git_file("Cargo.toml"),
        git_file("crates/read_2d/src/lib.rs"),
        git_file("crates/read_3d/src/main.rs"),
    ];
    let crates_key = runtime.git_tree_dir_key("crates");
    runtime.session.git_collapsed_dirs.insert(crates_key);

    runtime.gui_collapse_file_tree_to_first_level();

    let rows = runtime.git_file_tree_rows();
    assert!(matches!(
        rows[1],
        GitFileTreeRow::Dir {
            ref path,
            depth: 0,
            expanded: true
        } if path == "crates"
    ));
    assert!(matches!(
        rows[2],
        GitFileTreeRow::Dir {
            ref path,
            depth: 1,
            expanded: false
        } if path == "crates/read_2d"
    ));
    assert!(matches!(
        rows[3],
        GitFileTreeRow::Dir {
            ref path,
            depth: 1,
            expanded: false
        } if path == "crates/read_3d"
    ));
    assert!(!rows.iter().any(|row| {
        matches!(
            row,
            GitFileTreeRow::Dir {
                path,
                depth: 2,
                ..
            } if path == "crates/read_2d/src"
        )
    }));
}

#[test]
fn full_gui_snapshot_exposes_jump_state_and_scroll_row() {
    let mut runtime = reviewer_runtime_with_full_diff();

    let snapshot = runtime.gui_snapshot();

    assert_eq!(snapshot.diff_view_mode, DiffViewMode::Full);
    assert_eq!(snapshot.diff_scroll_row, 0);
    assert_eq!(snapshot.current_diff_block, 1);
    assert_eq!(snapshot.diff_block_count, 2);
    assert!(!snapshot.can_jump_previous_block);
    assert!(snapshot.can_jump_next_block);

    runtime.gui_jump_full_block(false);
    let snapshot = runtime.gui_snapshot();

    assert_eq!(snapshot.diff_scroll_row, 2);
    assert_eq!(snapshot.current_diff_block, 2);
    assert_eq!(snapshot.diff_block_count, 2);
    assert!(snapshot.can_jump_previous_block);
    assert!(!snapshot.can_jump_next_block);
}

#[test]
fn gui_set_diff_scroll_row_clamps_full_scroll() {
    let mut runtime = reviewer_runtime_with_full_diff();

    runtime.gui_set_diff_scroll_row(999, 160);
    let snapshot = runtime.gui_snapshot();

    assert_eq!(snapshot.diff_scroll_row, 3);
}

#[test]
fn git_agent_paste_text_uses_repo_commit_and_file_for_file_tree_rows() {
    let mut runtime = reviewer_runtime_with_git_files();
    runtime.session.active_column = ActiveColumn::Files;
    runtime.session.selection.file_index = 1;

    assert_eq!(
        runtime.gui_selected_agent_paste_text(),
        Some("repo:root commit:abc1234 file:src/main.rs".to_string())
    );
}

#[test]
fn git_agent_paste_text_uses_repo_and_commit_for_commit_column() {
    let mut runtime = reviewer_runtime_with_git_files();
    runtime.session.active_column = ActiveColumn::Repos;

    assert_eq!(
        runtime.gui_selected_agent_paste_text(),
        Some("repo:root commit:abc1234".to_string())
    );
}

#[test]
fn git_agent_paste_text_uses_repo_for_repo_column() {
    let mut runtime = reviewer_runtime_with_git_files();
    runtime.session.active_column = ActiveColumn::ChangeGroups;

    assert_eq!(
        runtime.gui_selected_agent_paste_text(),
        Some("repo:root".to_string())
    );
}

#[test]
fn git_gui_repo_rows_mark_dirty_repos_without_text_prefix() {
    let mut runtime = reviewer_runtime_with_git_files();
    runtime.session.active_column = ActiveColumn::ChangeGroups;
    runtime.session.git_repos[0].label = "repos/zettos-ai-engine".to_string();
    runtime.session.git_repos[0].commits.insert(
        0,
        GitCommitReview {
            commit: UNCOMMITTED_COMMIT_ID.to_string(),
            short_hash: "uncommit".to_string(),
            author: String::new(),
            full_author: "uncommitted changes".to_string(),
            display_time: String::new(),
            message: "staged / unstaged / untracked changes".to_string(),
            files: Vec::new(),
            files_loaded: true,
        },
    );

    let column = runtime.gui_column(0);

    assert_eq!(column.rows[0].label, "repos/zettos-ai-engine");
    assert_eq!(column.rows[0].tone, GuiReviewerRowTone::Dirty);
}

#[test]
fn gsd_agent_paste_text_uses_repo_and_commit_file_for_file_column() {
    let runtime = reviewer_runtime_with_gsd_file();

    assert_eq!(
        runtime.gui_selected_agent_paste_text(),
        Some("repo:root commit:abcdef1 file:src/gui/app.rs".to_string())
    );
}

#[test]
fn gsd_agent_paste_text_uses_repo_for_repo_column() {
    let mut runtime = reviewer_runtime_with_gsd_file();
    runtime.session.active_column = ActiveColumn::Repos;

    assert_eq!(
        runtime.gui_selected_agent_paste_text(),
        Some("repo:root".to_string())
    );
}

#[test]
fn git_repo_column_helix_target_opens_repo_root_without_file() {
    let mut runtime = reviewer_runtime_with_git_files();
    runtime.context.project_dir = PathBuf::from("/tmp/project");
    runtime.session.active_column = ActiveColumn::ChangeGroups;

    let target = runtime.gui_selected_helix_target(160).unwrap();

    assert_eq!(target.workdir, PathBuf::from("/tmp/project"));
    assert_eq!(target.file, None);
    assert_eq!(target.line, None);
}

#[test]
fn git_file_column_helix_target_uses_repo_relative_file() {
    let mut runtime = reviewer_runtime_with_git_files();
    runtime.context.project_dir = PathBuf::from("/tmp/project");
    runtime.session.git_repos[0].repo = Some("crates/app".to_string());
    runtime.session.git_repos[0].root = PathBuf::from("/tmp/project/crates/app");
    runtime.session.git_repos[0].commits[0].files[0] = git_file("crates/app/src/main.rs");
    runtime.session.git_active_file = Some(GitFileKey {
        repo: Some("crates/app".to_string()),
        commit: "abc1234".to_string(),
        file_path: "crates/app/src/main.rs".to_string(),
    });
    runtime.session.active_column = ActiveColumn::Files;

    let target = runtime.gui_selected_helix_target(160).unwrap();

    assert_eq!(target.workdir, PathBuf::from("/tmp/project/crates/app"));
    assert_eq!(target.file, Some(PathBuf::from("src/main.rs")));
    assert_eq!(target.line, None);
}

#[test]
fn diff_column_helix_target_uses_selected_diff_line() {
    let mut runtime = reviewer_runtime_with_git_diff_lines();
    runtime.session.active_column = ActiveColumn::Diff;
    runtime.session.diff_state.diff_cursor_row = 1;

    let target = runtime.gui_selected_helix_target(160).unwrap();

    assert_eq!(target.file, Some(PathBuf::from("src/main.rs")));
    assert_eq!(target.line, Some(12));
}

#[test]
fn full_rendered_line_parses_helix_target_line() {
    assert_eq!(
        parse_rendered_target_line("I|  42│ +let value = true;"),
        Some(42)
    );
    assert_eq!(
        parse_rendered_target_line("D|    │ -removed before line"),
        None
    );
}

#[test]
fn gsd_agent_paste_text_uses_block_and_task_for_first_column() {
    let mut runtime = reviewer_runtime_with_gsd_file();
    runtime.session.active_column = ActiveColumn::ChangeGroups;
    runtime.session.selection.change_row_index = 1;

    assert_eq!(
        runtime.gui_selected_agent_paste_text(),
        Some("block:01-01 task:1 Copy selected reviewer row".to_string())
    );
}

#[test]
fn diff_line_single_click_copy_uses_commit_absolute_path_and_line() {
    let runtime = reviewer_runtime_with_git_diff_lines();

    let snapshot = runtime.gui_snapshot();

    assert_eq!(
        snapshot.diff_lines[1].single_click_copy.as_deref(),
        Some("commit:abc1234 /tmp/gsdv-project/src/main.rs:12")
    );
    assert_eq!(
        snapshot.diff_lines[2].single_click_copy.as_deref(),
        Some("commit:abc1234 /tmp/gsdv-project/src/main.rs:10")
    );
}

#[test]
fn diff_line_double_click_copy_uses_whole_changed_block() {
    let runtime = reviewer_runtime_with_git_diff_lines();

    let snapshot = runtime.gui_snapshot();
    let copied = snapshot.diff_lines[1]
        .double_click_copy
        .as_deref()
        .expect("copy text");

    assert!(copied.starts_with("repo:/tmp/gsdv-project/src/main.rs commit:abc1234\n"));
    assert!(copied.contains("@@ -10,2 +12,2 @@ HEAD~1"));
    assert!(copied.contains("  12│ +added"));
    assert!(copied.contains("  10│ -removed"));
}

/// 验证 `v` 复制 diff 元信息时不会带上代码正文。
#[test]
fn diff_metadata_copy_omits_changed_code_lines() {
    let mut runtime = reviewer_runtime_with_git_diff_lines();
    runtime.session.active_column = ActiveColumn::Diff;
    runtime.session.diff_state.diff_cursor_row = 1;

    let copied = runtime
        .gui_selected_diff_metadata_paste_text(160)
        .expect("diff metadata");

    assert_eq!(
        copied,
        "repo:/tmp/gsdv-project/src/main.rs commit:abc1234\n@@ -10,2 +12,2 @@ HEAD~1"
    );
    assert!(!copied.contains("+added"));
    assert!(!copied.contains("-removed"));
}

#[test]
fn git_repo_script_target_uses_first_column_row_repo() {
    let mut runtime = reviewer_runtime_with_git_files();
    runtime.session.git_repos[0].label = "crates/app".to_string();
    runtime.session.git_repos[0].repo = Some("crates/app".to_string());
    runtime.session.git_repos[0].root = PathBuf::from("/tmp/project/crates/app");

    let target = runtime.repo_script_target(0, 0).expect("script target");

    assert_eq!(target.0, "crates/app");
    assert_eq!(target.1.as_deref(), Some("crates/app"));
    assert_eq!(target.2, PathBuf::from("/tmp/project/crates/app"));
    assert!(runtime.repo_script_target(1, 0).is_none());
}

#[test]
fn gsd_repo_script_target_uses_second_column_row_repo() {
    let mut runtime = reviewer_runtime_with_gsd_file();
    runtime.context.project_dir = PathBuf::from("/tmp/project");

    let target = runtime.repo_script_target(1, 0).expect("script target");

    assert_eq!(target.0, "root");
    assert_eq!(target.1, None);
    assert_eq!(target.2, PathBuf::from("/tmp/project"));
    assert!(runtime.repo_script_target(0, 0).is_none());
}

fn reviewer_runtime_with_full_diff() -> ReviewerRuntime {
    let mut runtime = ReviewerRuntime {
        context: ReviewerContext {
            project_dir: PathBuf::new(),
            phase_id: None,
            workstream: None,
        },
        session: ReviewerSession {
            active_column: ActiveColumn::Diff,
            selection: ReviewerSelection::default(),
            diff_state: ReviewerDiffState {
                view_mode: DiffViewMode::Full,
                full_file_loaded: true,
                full_lines: vec![
                    "one".to_string(),
                    "two".to_string(),
                    "three".to_string(),
                    "four".to_string(),
                ],
                full_jump_targets: vec![1, 3],
                ..ReviewerDiffState::default()
            },
            gui_state: GuiReviewerPreparedState::default(),
            hovered: None,
            mode: ReviewerMode::Git,
            gsd_load_state: LoadState::Ready,
            git_load_state: LoadState::Ready,
            phase: None,
            git_repos: vec![GitRepoReview {
                label: ".".to_string(),
                repo: None,
                root: PathBuf::new(),
                commits: vec![GitCommitReview {
                    commit: "abc1234".to_string(),
                    short_hash: "abc1234".to_string(),
                    author: "tests".to_string(),
                    full_author: "tests.user".to_string(),
                    display_time: "04/25-12:00".to_string(),
                    message: "fixture".to_string(),
                    files: vec![git_file("src/main.rs")],
                    files_loaded: true,
                }],
                commits_loaded: true,
            }],
            git_collapsed_dirs: BTreeSet::new(),
            git_active_file: None,
            mouse_capture_suspended: false,
            diff: None,
        },
        exit_mode: ReviewerExitMode::Quit,
    };
    runtime.ensure_full_render_cache(80);
    runtime.rebuild_gui_state();
    runtime
}

fn reviewer_runtime_with_git_files() -> ReviewerRuntime {
    ReviewerRuntime {
        context: ReviewerContext {
            project_dir: PathBuf::new(),
            phase_id: None,
            workstream: None,
        },
        session: ReviewerSession {
            active_column: ActiveColumn::Files,
            selection: ReviewerSelection::default(),
            diff_state: ReviewerDiffState::default(),
            gui_state: GuiReviewerPreparedState::default(),
            hovered: None,
            mode: ReviewerMode::Git,
            gsd_load_state: LoadState::Ready,
            git_load_state: LoadState::Ready,
            phase: None,
            git_repos: vec![GitRepoReview {
                label: ".".to_string(),
                repo: None,
                root: PathBuf::new(),
                commits: vec![GitCommitReview {
                    commit: "abc1234".to_string(),
                    short_hash: "abc1234".to_string(),
                    author: "tests".to_string(),
                    full_author: "tests.user".to_string(),
                    display_time: "04/25-12:00".to_string(),
                    message: "fixture".to_string(),
                    files: vec![git_file("src/main.rs")],
                    files_loaded: true,
                }],
                commits_loaded: true,
            }],
            git_collapsed_dirs: BTreeSet::new(),
            git_active_file: None,
            mouse_capture_suspended: false,
            diff: None,
        },
        exit_mode: ReviewerExitMode::Quit,
    }
}

fn reviewer_runtime_with_git_diff_lines() -> ReviewerRuntime {
    let mut runtime = reviewer_runtime_with_git_files();
    runtime.context.project_dir = PathBuf::from("/tmp/gsdv-project");
    runtime.session.git_active_file = Some(GitFileKey {
        repo: None,
        commit: "abc1234".to_string(),
        file_path: "src/main.rs".to_string(),
    });
    runtime.session.git_repos[0].commits[0].files[0]
        .entry
        .diff_source = DiffSource::CommitBacked {
        commit: "abc1234".to_string(),
        repo: None,
    };
    runtime.session.diff = Some(DiffPayload {
        title: "src/main.rs".to_string(),
        jump_targets: vec![10],
        body: DiffBody::Lines(vec![
            DiffLine {
                kind: DiffLineKind::Hunk,
                old_line: Some(10),
                new_line: Some(12),
                text: "@@ -10,2 +12,2 @@ HEAD~1".to_string(),
                metadata: None,
            },
            DiffLine {
                kind: DiffLineKind::Insert,
                old_line: None,
                new_line: Some(12),
                text: "added".to_string(),
                metadata: None,
            },
            DiffLine {
                kind: DiffLineKind::Delete,
                old_line: Some(10),
                new_line: None,
                text: "removed".to_string(),
                metadata: None,
            },
            DiffLine {
                kind: DiffLineKind::Context,
                old_line: Some(11),
                new_line: Some(13),
                text: "context".to_string(),
                metadata: None,
            },
        ]),
    });
    runtime.rebuild_gui_state();
    runtime
}

fn reviewer_runtime_with_gsd_file() -> ReviewerRuntime {
    ReviewerRuntime {
        context: ReviewerContext {
            project_dir: PathBuf::new(),
            phase_id: Some("01".to_string()),
            workstream: None,
        },
        session: ReviewerSession {
            active_column: ActiveColumn::Files,
            selection: ReviewerSelection::default(),
            diff_state: ReviewerDiffState::default(),
            gui_state: GuiReviewerPreparedState::default(),
            hovered: None,
            mode: ReviewerMode::Gsd,
            gsd_load_state: LoadState::Ready,
            git_load_state: LoadState::Ready,
            phase: Some(PhaseProvenance {
                phase_number: "01".to_string(),
                phase_dir: PathBuf::new(),
                plans: vec![PlanProvenance {
                    id: "01-01".to_string(),
                    title: "Reviewer copy".to_string(),
                    change_groups: vec![ChangeGroup {
                        plan_id: "01-01".to_string(),
                        plan_title: "Reviewer copy".to_string(),
                        task_index: 1,
                        task_name: "Copy selected reviewer row".to_string(),
                        provenance_status: ProvenanceStatus::Ok,
                        summary_status: ProvenanceStatus::Ok,
                        commit_provenance: CommitProvenance::Missing,
                        file_hints: Vec::new(),
                        repo_buckets: vec![RepoBucket {
                            repo: None,
                            unmatched: false,
                            files: vec![FileEntry {
                                path: "src/gui/app.rs".to_string(),
                                diff_source: DiffSource::CommitBacked {
                                    commit: "abcdef1234567890".to_string(),
                                    repo: None,
                                },
                            }],
                        }],
                    }],
                }],
            }),
            git_repos: Vec::new(),
            git_collapsed_dirs: BTreeSet::new(),
            git_active_file: None,
            mouse_capture_suspended: false,
            diff: None,
        },
        exit_mode: ReviewerExitMode::Quit,
    }
}

fn git_file(path: &str) -> GitFileReview {
    GitFileReview {
        display_path: path.to_string(),
        entry: FileEntry {
            path: path.to_string(),
            diff_source: DiffSource::NonCommitFallback {
                hint: "fixture".to_string(),
            },
        },
    }
}

fn working_tree_file(path: &str, untracked: bool) -> GitFileReview {
    GitFileReview {
        display_path: path.to_string(),
        entry: FileEntry {
            path: path.to_string(),
            diff_source: DiffSource::WorkingTree {
                repo: None,
                staged: false,
                unstaged: !untracked,
                untracked,
            },
        },
    }
}

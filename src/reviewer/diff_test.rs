use super::*;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn loads_commit_backed_patch() {
    let root = git_fixture("loads_commit_backed_patch");
    commit_file(&root, "src/lib.rs", "fn before() {}\n");
    write_file(&root.join("src/lib.rs"), "fn after() {}\n");
    run_git(&root, &["add", "src/lib.rs"]);
    run_git(&root, &["commit", "-m", "after"]);
    let commit = git_stdout(&root, &["rev-parse", "--short", "HEAD"])
        .trim()
        .to_string();

    let payload = load_diff(
        &root,
        &FileEntry {
            path: "src/lib.rs".to_string(),
            diff_source: DiffSource::CommitBacked { commit, repo: None },
        },
    )
    .unwrap();

    assert!(matches!(
        payload.body,
        DiffBody::Lines(lines)
            if lines.iter().any(|line| line.kind == DiffLineKind::Delete && line.text == "fn before() {}")
                && lines.iter().any(|line| line.kind == DiffLineKind::Insert && line.text == "fn after() {}")
    ));
    assert_eq!(payload.jump_targets, vec![1]);
    cleanup(&root);
}

#[test]
fn renders_non_commit_placeholder() {
    let payload = load_diff(
        Path::new("/tmp/demo"),
        &FileEntry {
            path: "src/lib.rs".to_string(),
            diff_source: DiffSource::NonCommitFallback {
                hint: "plan/task file hint".to_string(),
            },
        },
    )
    .unwrap();

    assert_eq!(
        payload.body,
        DiffBody::Placeholder(
            "No commit-backed diff is available for this file.\nplan/task file hint".to_string()
        )
    );
    assert!(payload.jump_targets.is_empty());
}

#[test]
fn renders_untracked_working_tree_file_as_insertions() {
    let root = git_fixture("renders_untracked_working_tree_file_as_insertions");
    write_file(&root.join("src/new.rs"), "fn new() {}\n");

    let payload = load_diff(
        &root,
        &FileEntry {
            path: "src/new.rs".to_string(),
            diff_source: DiffSource::WorkingTree {
                repo: None,
                staged: false,
                unstaged: false,
                untracked: true,
            },
        },
    )
    .unwrap();

    assert!(matches!(
        payload.body,
        DiffBody::Lines(lines)
            if lines.iter().any(|line| line.kind == DiffLineKind::Insert && line.text == "fn new() {}")
    ));
    cleanup(&root);
}

#[test]
fn unavailable_commit_backed_diff_uses_placeholder() {
    let root = git_fixture("surfaces_git_lookup_errors");
    let payload = load_diff(
        &root,
        &FileEntry {
            path: "src/missing.rs".to_string(),
            diff_source: DiffSource::CommitBacked {
                commit: "deadbeef".to_string(),
                repo: None,
            },
        },
    )
    .unwrap();

    assert!(matches!(payload.body, DiffBody::Placeholder(body) if body.contains("deadbeef")));
    assert!(payload.jump_targets.is_empty());
    cleanup(&root);
}

#[test]
fn commit_backed_mode_only_change_uses_gix_metadata() {
    let root = git_fixture("commit_backed_mode_only_change_uses_gix_metadata");
    commit_file(&root, "scripts/run.sh", "#!/bin/sh\necho ok\n");
    run_git(&root, &["update-index", "--chmod=+x", "scripts/run.sh"]);
    run_git(&root, &["commit", "-m", "make executable"]);
    let commit = git_stdout(&root, &["rev-parse", "--short", "HEAD"])
        .trim()
        .to_string();

    let payload = load_diff(
        &root,
        &FileEntry {
            path: "scripts/run.sh".to_string(),
            diff_source: DiffSource::CommitBacked { commit, repo: None },
        },
    )
    .unwrap();

    assert!(matches!(
        payload.body,
        DiffBody::Lines(lines)
            if lines.iter().any(|line| matches!(
                line.metadata,
                Some(DiffLineMetadata::ModeChanged { .. })
            ))
    ));
    cleanup(&root);
}

#[test]
fn structured_overlay_tracks_insertions_and_deletions() {
    let lines = structured_diff_lines("src/main.rs", "same\nold\n", "same\nnew\n", "demo");
    let (jumps, changed, deleted_before) = structured_overlay(&lines);

    assert_eq!(jumps, vec![1]);
    assert!(changed.contains(&2));
    assert_eq!(deleted_before.get(&3), Some(&vec!["old".to_string()]));
}

#[test]
fn structured_diff_keeps_equal_context_instead_of_replacing_whole_file() {
    let lines = structured_diff_lines(
        "plan.md",
        "a\nb\nc\nd\ne\n",
        "a\nb\nchanged\nd\ne\n",
        "demo",
    );

    let deletes = lines
        .iter()
        .filter(|line| line.kind == DiffLineKind::Delete)
        .count();
    let inserts = lines
        .iter()
        .filter(|line| line.kind == DiffLineKind::Insert)
        .count();

    assert_eq!(deletes, 1);
    assert_eq!(inserts, 1);
    assert!(lines.iter().any(|line| {
        line.kind == DiffLineKind::Context
            && line.old_line == Some(2)
            && line.new_line == Some(2)
            && line.text == "b"
    }));
}

#[test]
fn structured_diff_ignores_line_ending_only_changes() {
    let lines = structured_diff_lines("setting.go", "a\nb\nc\n", "a\r\nb\r\nc\r\n", "worktree");

    assert!(lines.is_empty());
}

#[test]
fn structured_diff_does_not_replace_whole_file_when_line_endings_change() {
    let lines = structured_diff_lines(
        "setting.go",
        "a\nb\nc\nd\n",
        "a\r\nb\r\nchanged\r\nd\r\n",
        "worktree",
    );

    let deletes = lines
        .iter()
        .filter(|line| line.kind == DiffLineKind::Delete)
        .count();
    let inserts = lines
        .iter()
        .filter(|line| line.kind == DiffLineKind::Insert)
        .count();

    assert_eq!(deletes, 1);
    assert_eq!(inserts, 1);
}

fn git_fixture(name: &str) -> PathBuf {
    let root = test_root(name);
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "tests@example.com"]);
    run_git(&root, &["config", "user.name", "Tests"]);
    write_file(&root.join(".gitignore"), "target/\n");
    run_git(&root, &["add", ".gitignore"]);
    run_git(&root, &["commit", "-m", "init"]);
    root
}

fn commit_file(root: &Path, path: &str, content: &str) -> String {
    write_file(&root.join(path), content);
    run_git(root, &["add", path]);
    run_git(root, &["commit", "-m", &format!("add {path}")]);
    git_stdout(root, &["rev-parse", "--short", "HEAD"])
        .trim()
        .to_string()
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut file = File::create(path).unwrap();
    file.write_all(content.as_bytes()).unwrap();
}

fn run_git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {:?} failed", args);
}

fn git_stdout(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8(output.stdout).unwrap()
}

fn test_root(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("gsdv-diff-{name}-{unique}"));
    fs::create_dir_all(&root).unwrap();
    root
}

fn cleanup(root: &Path) {
    let _ = fs::remove_dir_all(root);
}

use super::*;
use std::fs::{self, File};
use std::io::Write;
use std::process::Command;

#[test]
fn loads_root_and_nested_git_repos() {
    let root = test_root("loads_root_and_nested_git_repos");
    init_git_repo(&root);
    commit_file(&root, "src/root.rs", "fn root() {}\n", "root");

    let nested = root.join("tools/helper");
    init_git_repo(&nested);
    commit_file(&nested, "src/helper.rs", "fn helper() {}\n", "helper");

    let reviews = load_git_review(&root).unwrap();
    let labels = reviews
        .iter()
        .map(|repo| repo.label.clone())
        .collect::<Vec<_>>();

    assert_eq!(labels, vec![".".to_string(), "tools/helper".to_string()]);
    let root_commits = load_git_repo_commits(&reviews[0].root).unwrap();
    let nested_commits = load_git_repo_commits(&reviews[1].root).unwrap();
    let root_files = load_git_commit_files(
        &reviews[0].root,
        reviews[0].repo.as_deref(),
        &root_commits[0].commit,
    )
    .unwrap();
    let nested_files = load_git_commit_files(
        &reviews[1].root,
        reviews[1].repo.as_deref(),
        &nested_commits[0].commit,
    )
    .unwrap();
    assert_eq!(root_files[0].display_path, "src/root.rs");
    assert_eq!(nested_files[0].entry.path, "tools/helper/src/helper.rs");
    cleanup(&root);
}

#[test]
fn does_not_treat_commit_message_body_as_changed_files() {
    let root = test_root("does_not_treat_commit_message_body_as_changed_files");
    init_git_repo(&root);
    write_file(&root.join("src/lib.rs"), "fn one() {}\n");
    run_git(&root, &["add", "src/lib.rs"]);
    run_git(
        &root,
        &[
            "commit",
            "-m",
            "feat: add lib",
            "-m",
            "- fake/path/from/message",
        ],
    );

    let reviews = load_git_review(&root).unwrap();
    let commits = load_git_repo_commits(&reviews[0].root).unwrap();
    let commit = &commits[0];
    let files = load_git_commit_files(&reviews[0].root, reviews[0].repo.as_deref(), &commit.commit)
        .unwrap();

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].display_path, "src/lib.rs");
    assert!(commit.message.contains("fake/path/from/message"));
    cleanup(&root);
}

#[test]
fn changed_files_do_not_expand_modified_directories() {
    let root = test_root("changed_files_do_not_expand_modified_directories");
    init_git_repo(&root);
    write_file(&root.join("src/changed.rs"), "fn changed() {}\n");
    write_file(&root.join("src/unchanged.rs"), "fn unchanged() {}\n");
    run_git(&root, &["add", "src/changed.rs", "src/unchanged.rs"]);
    run_git(&root, &["commit", "-m", "base"]);
    write_file(&root.join("src/changed.rs"), "fn changed_again() {}\n");
    run_git(&root, &["add", "src/changed.rs"]);
    run_git(&root, &["commit", "-m", "modify one file"]);

    let reviews = load_git_review(&root).unwrap();
    let commits = load_git_repo_commits(&reviews[0].root).unwrap();
    let commit = &commits[0];
    let files = load_git_commit_files(&reviews[0].root, reviews[0].repo.as_deref(), &commit.commit)
        .unwrap();
    let paths = files
        .iter()
        .map(|file| file.display_path.as_str())
        .collect::<Vec<_>>();

    assert_eq!(paths, vec!["src/changed.rs"]);
    cleanup(&root);
}

#[test]
fn uses_human_readable_commit_rows() {
    let root = test_root("uses_human_readable_commit_rows");
    init_git_repo(&root);
    commit_file(&root, "src/lib.rs", "fn one() {}\n", "first");

    let reviews = load_git_review(&root).unwrap();
    let commits = load_git_repo_commits(&reviews[0].root).unwrap();
    let commit = &commits[0];

    assert!(!commit.short_hash.is_empty());
    assert!(!commit.author.is_empty());
    assert!(!commit.display_time.is_empty());
    cleanup(&root);
}

#[test]
fn clean_repo_review_defers_commit_history_loading() {
    let root = test_root("clean_repo_review_defers_commit_history_loading");
    init_git_repo(&root);
    commit_file(&root, "src/lib.rs", "fn one() {}\n", "first");

    let reviews = load_git_review(&root).unwrap();

    assert!(reviews[0].commits.is_empty());
    assert!(!reviews[0].commits_loaded);

    let commits = load_git_repo_commits(&reviews[0].root).unwrap();
    assert!(!commits[0].short_hash.is_empty());
    cleanup(&root);
}

#[test]
fn dirty_files_are_loaded_as_uncommitted_pseudo_commit() {
    let root = test_root("dirty_files_are_loaded_as_uncommitted_pseudo_commit");
    init_git_repo(&root);
    commit_file(&root, "src/tracked.rs", "fn old() {}\n", "tracked");
    write_file(&root.join("src/tracked.rs"), "fn changed() {}\n");
    write_file(&root.join("src/staged.rs"), "fn staged() {}\n");
    run_git(&root, &["add", "src/staged.rs"]);
    write_file(&root.join("src/untracked.rs"), "fn untracked() {}\n");

    let reviews = load_git_review(&root).unwrap();
    let commit = &reviews[0].commits[0];

    assert_eq!(commit.commit, UNCOMMITTED_COMMIT_ID);
    assert_eq!(commit.short_hash, "uncommit");
    let paths = commit
        .files
        .iter()
        .map(|file| file.display_path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec!["src/staged.rs", "src/tracked.rs", "src/untracked.rs"]
    );
    assert!(matches!(
        commit.files[0].entry.diff_source,
        DiffSource::WorkingTree {
            staged: true,
            unstaged: false,
            untracked: false,
            ..
        }
    ));
    assert!(matches!(
        commit.files[1].entry.diff_source,
        DiffSource::WorkingTree {
            staged: false,
            unstaged: true,
            untracked: false,
            ..
        }
    ));
    assert!(matches!(
        commit.files[2].entry.diff_source,
        DiffSource::WorkingTree {
            untracked: true,
            ..
        }
    ));
    cleanup(&root);
}

#[test]
fn compacts_dot_separated_author_names_to_initials() {
    assert_eq!(format_commit_author("ab.cd"), "aC");
    assert_eq!(format_commit_author("pengcheng.mo"), "pM");
    assert_eq!(format_commit_author("weijian.zheng extra"), "wZ");
    assert_eq!(format_commit_author("single"), "single");
}

#[test]
fn compacts_pinyinish_nasal_names() {
    assert_eq!(format_commit_author("penggang"), "pG");
    assert_eq!(format_commit_author("jiangjie"), "jJie");
    assert_eq!(format_commit_author("dwd-dwdw-dw"), "dDD");
    assert_eq!(format_commit_author("ab.dwdw"), "aD");
}

fn init_git_repo(root: &Path) {
    fs::create_dir_all(root).unwrap();
    run_git(root, &["init"]);
    run_git(root, &["config", "user.email", "tests@example.com"]);
    run_git(root, &["config", "user.name", "Tests"]);
    write_file(&root.join(".gitignore"), "target/\n");
    run_git(root, &["add", ".gitignore"]);
    run_git(root, &["commit", "-m", "init"]);
}

fn commit_file(root: &Path, path: &str, content: &str, message: &str) {
    write_file(&root.join(path), content);
    run_git(root, &["add", path]);
    run_git(root, &["commit", "-m", message]);
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

fn test_root(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("gsdv-git-{name}-{unique}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn cleanup(root: &Path) {
    let _ = fs::remove_dir_all(root);
}

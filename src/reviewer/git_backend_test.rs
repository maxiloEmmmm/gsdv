use super::*;
use std::fs::{self, File};
use std::io::Write;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn branch_choices_include_local_branches() {
    let root = git_fixture("branch_choices_include_local_branches");
    run_git(&root, &["checkout", "-b", "feature/demo"]);

    let (current, branches) = load_branch_choices(&root).unwrap();

    assert_eq!(current, "feature/demo");
    assert!(branches.iter().any(|branch| branch.label == "feature/demo"));
    cleanup(&root);
}

#[test]
fn checkout_local_branch_updates_worktree() {
    let root = git_fixture("checkout_local_branch_updates_worktree");
    write_file(&root.join("src/lib.rs"), "main\n");
    run_git(&root, &["add", "src/lib.rs"]);
    run_git(&root, &["commit", "-m", "main"]);
    run_git(&root, &["checkout", "-b", "feature/demo"]);
    write_file(&root.join("src/lib.rs"), "feature\n");
    run_git(&root, &["add", "src/lib.rs"]);
    run_git(&root, &["commit", "-m", "feature"]);
    run_git(&root, &["checkout", "main"]);

    checkout_branch(&root, &BranchCheckout::Local("feature/demo".to_string())).unwrap();

    assert_eq!(
        fs::read_to_string(root.join("src/lib.rs"))
            .unwrap()
            .replace("\r\n", "\n"),
        "feature\n"
    );
    cleanup(&root);
}

fn git_fixture(name: &str) -> std::path::PathBuf {
    let root = test_root(name);
    run_git(&root, &["init", "-b", "main"]);
    run_git(&root, &["config", "user.email", "tests@example.com"]);
    run_git(&root, &["config", "user.name", "Tests"]);
    write_file(&root.join(".gitignore"), "target/\n");
    run_git(&root, &["add", ".gitignore"]);
    run_git(&root, &["commit", "-m", "init"]);
    root
}

fn test_root(name: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("gsdv-git-backend-{name}-{nonce}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
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
    assert!(status.success(), "git {args:?} failed");
}

fn cleanup(root: &Path) {
    let _ = fs::remove_dir_all(root);
}

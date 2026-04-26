use super::*;
use std::fs::{self, File};
use std::io::Write;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn phase_directory_resolution() {
    let root = test_root("phase_directory_resolution");
    write_file(
        &root.join(".planning/phases/01-reviewer/01-01-PLAN.md"),
        &plan_doc("01", "Alpha", &[task_xml("One", &["src/a.rs"])]),
    );
    write_file(
        &root.join(".planning/workstreams/ws-one/phases/01-reviewer/01-01-PLAN.md"),
        &plan_doc("01", "Alpha", &[task_xml("One", &["src/a.rs"])]),
    );

    let flat = resolve_phase_directory(&root, "01", None).unwrap();
    let workstream = resolve_phase_directory(&root, "1", Some("ws-one")).unwrap();

    assert!(flat.ends_with(".planning/phases/01-reviewer"));
    assert!(workstream.ends_with(".planning/workstreams/ws-one/phases/01-reviewer"));
    cleanup(&root);
}

#[test]
fn preserves_plan_task_order() {
    let root = basic_phase_fixture("preserves_plan_task_order");
    write_file(
        &phase_dir(&root).join("01-01-PLAN.md"),
        &plan_doc(
            "01",
            "First",
            &[
                task_xml("Repeat", &["src/a.rs"]),
                task_xml("Unique", &["src/b.rs"]),
            ],
        ),
    );
    write_file(
        &phase_dir(&root).join("01-02-PLAN.md"),
        &plan_doc("02", "Second", &[task_xml("Repeat", &["src/c.rs"])]),
    );

    let loaded = load_phase_provenance(&root, "01", LoadPhaseOptions::default()).unwrap();
    let ordered = loaded
        .plans
        .iter()
        .flat_map(|plan| {
            plan.change_groups
                .iter()
                .map(|group| format!("{}:{}", group.plan_id, group.task_name.clone()))
        })
        .collect::<Vec<_>>();

    assert_eq!(
        ordered,
        vec!["01-01:Repeat", "01-01:Unique", "01-02:Repeat"]
    );
    cleanup(&root);
}

#[test]
fn summary_is_optional() {
    let root = basic_phase_fixture("summary_is_optional");
    write_file(
        &phase_dir(&root).join("01-01-PLAN.md"),
        &plan_doc("01", "Only", &[task_xml("Solo", &["src/a.rs"])]),
    );

    let loaded = load_phase_provenance(&root, "01", LoadPhaseOptions::default()).unwrap();
    let group = &loaded.plans[0].change_groups[0];
    assert_eq!(group.task_name, "Solo");
    assert_eq!(group.commit_provenance.display(), "无");
    cleanup(&root);
}

#[test]
fn marks_bad_parse_state() {
    let root = basic_phase_fixture("marks_bad_parse_state");
    write_file(
        &phase_dir(&root).join("01-01-PLAN.md"),
        &format!(
            "{}\n<tasks>\n<task type=\"auto\"><name>Broken</name>\n",
            plan_frontmatter("01")
        ),
    );

    let loaded = load_phase_provenance(&root, "01", LoadPhaseOptions::default()).unwrap();
    assert_eq!(
        loaded.plans[0].change_groups[0].provenance_status.display(),
        "bad"
    );
    cleanup(&root);
}

#[test]
fn maps_task_commits() {
    let root = git_phase_fixture("maps_task_commits");
    let commit_one = commit_file(&root, "src/a.rs", "fn a() {}\n");
    let commit_two = commit_file(&root, "src/b.rs", "fn b() {}\n");
    write_file(
        &phase_dir(&root).join("01-01-PLAN.md"),
        &plan_doc(
            "01",
            "Commits",
            &[
                task_xml("One", &["src/a.rs"]),
                task_xml("Two", &["src/b.rs"]),
            ],
        ),
    );
    write_file(
        &phase_dir(&root).join("01-01-SUMMARY.md"),
        &summary_doc(&[
            format!("1. **Task 1: One** - `{commit_one}`"),
            format!("2. **Task 2: Two** - `{commit_two}`"),
        ]),
    );

    let loaded = load_phase_provenance(&root, "01", LoadPhaseOptions::default()).unwrap();
    assert_eq!(
        loaded.plans[0].change_groups[0].commit_provenance,
        CommitProvenance::Commits(vec![commit_one])
    );
    assert_eq!(
        loaded.plans[0].change_groups[1].commit_provenance,
        CommitProvenance::Commits(vec![commit_two])
    );
    cleanup(&root);
}

#[test]
fn uses_wu_for_missing_commits() {
    let root = basic_phase_fixture("uses_wu_for_missing_commits");
    write_file(
        &phase_dir(&root).join("01-01-PLAN.md"),
        &plan_doc("01", "Missing", &[task_xml("Solo", &["src/a.rs"])]),
    );
    write_file(
        &phase_dir(&root).join("01-01-SUMMARY.md"),
        &summary_doc(&Vec::<String>::new()),
    );

    let loaded = load_phase_provenance(&root, "01", LoadPhaseOptions::default()).unwrap();
    assert_eq!(
        loaded.plans[0].change_groups[0].commit_provenance.display(),
        "无"
    );
    cleanup(&root);
}

#[test]
fn marks_bad_commit_metadata() {
    let root = basic_phase_fixture("marks_bad_commit_metadata");
    write_file(
        &phase_dir(&root).join("01-01-PLAN.md"),
        &plan_doc("01", "Bad", &[task_xml("Solo", &["src/a.rs"])]),
    );
    write_file(
        &phase_dir(&root).join("01-01-SUMMARY.md"),
        &summary_doc(&["1. **Task 1: Solo** - `not-a-hash`".to_string()]),
    );

    let loaded = load_phase_provenance(&root, "01", LoadPhaseOptions::default()).unwrap();
    assert_eq!(
        loaded.plans[0].change_groups[0].commit_provenance.display(),
        "bad"
    );
    cleanup(&root);
}

#[test]
fn groups_files_by_subrepo() {
    let root = git_phase_fixture("groups_files_by_subrepo");
    write_json(
        &root.join(".planning/config.json"),
        r#"{"sub_repos":["frontend","backend"]}"#,
    );
    let frontend_commit = commit_file(&root, "frontend/src/app.ts", "console.log('a');\n");
    let backend_commit = commit_file(&root, "backend/src/lib.rs", "fn b() {}\n");
    write_file(
        &phase_dir(&root).join("01-01-PLAN.md"),
        &plan_doc(
            "01",
            "Repos",
            &[
                task_xml("Frontend", &["frontend/src/app.ts"]),
                task_xml("Backend", &["backend/src/lib.rs"]),
            ],
        ),
    );
    write_file(
        &phase_dir(&root).join("01-01-SUMMARY.md"),
        &summary_doc(&[
            format!("1. **Task 1: Frontend** - `{frontend_commit}`"),
            format!("2. **Task 2: Backend** - `{backend_commit}`"),
        ]),
    );

    let loaded = load_phase_provenance(&root, "01", LoadPhaseOptions::default()).unwrap();
    assert_eq!(
        loaded.plans[0].change_groups[0].repo_buckets[0]
            .repo
            .as_deref(),
        Some("frontend")
    );
    assert_eq!(
        loaded.plans[0].change_groups[1].repo_buckets[0]
            .repo
            .as_deref(),
        Some("backend")
    );
    cleanup(&root);
}

#[test]
fn keeps_unmatched_or_non_commit_fallback() {
    let root = basic_phase_fixture("keeps_unmatched_or_non_commit_fallback");
    write_json(
        &root.join(".planning/config.json"),
        r#"{"sub_repos":["frontend"]}"#,
    );
    write_file(
        &phase_dir(&root).join("01-01-PLAN.md"),
        &plan_doc("01", "Fallback", &[task_xml("Docs", &["docs/readme.md"])]),
    );

    let loaded = load_phase_provenance(&root, "01", LoadPhaseOptions::default()).unwrap();
    let bucket = &loaded.plans[0].change_groups[0].repo_buckets[0];
    assert!(bucket.unmatched);
    assert!(matches!(
        bucket.files[0].diff_source,
        DiffSource::NonCommitFallback { .. }
    ));
    cleanup(&root);
}

#[test]
fn reads_file_lists_from_commit_evidence() {
    let root = git_phase_fixture("reads_file_lists_from_commit_evidence");
    write_json(
        &root.join(".planning/config.json"),
        r#"{"sub_repos":["frontend"]}"#,
    );
    let commit = commit_files(
        &root,
        &[
            ("frontend/src/app.ts", "a\n"),
            ("frontend/src/panel.ts", "b\n"),
        ],
    );
    write_file(
        &phase_dir(&root).join("01-01-PLAN.md"),
        &plan_doc(
            "01",
            "Files",
            &[task_xml("Frontend", &["frontend/src/app.ts"])],
        ),
    );
    write_file(
        &phase_dir(&root).join("01-01-SUMMARY.md"),
        &summary_doc(&[format!("1. **Task 1: Frontend** - `{commit}`")]),
    );

    let loaded = load_phase_provenance(&root, "01", LoadPhaseOptions::default()).unwrap();
    let files = loaded.plans[0].change_groups[0].repo_buckets[0]
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    assert_eq!(files, vec!["frontend/src/app.ts", "frontend/src/panel.ts"]);
    cleanup(&root);
}

#[test]
fn marks_commit_backed_diff_sources() {
    let root = git_phase_fixture("marks_commit_backed_diff_sources");
    let commit = commit_file(&root, "src/a.rs", "fn a() {}\n");
    write_file(
        &phase_dir(&root).join("01-01-PLAN.md"),
        &plan_doc("01", "CommitBacked", &[task_xml("Task", &["src/a.rs"])]),
    );
    write_file(
        &phase_dir(&root).join("01-01-SUMMARY.md"),
        &summary_doc(&[format!("1. **Task 1: Task** - `{commit}`")]),
    );

    let loaded = load_phase_provenance(&root, "01", LoadPhaseOptions::default()).unwrap();
    assert!(matches!(
        loaded.plans[0].change_groups[0].repo_buckets[0].files[0].diff_source,
        DiffSource::CommitBacked { .. }
    ));
    cleanup(&root);
}

#[test]
fn marks_fallback_diff_sources() {
    let root = basic_phase_fixture("marks_fallback_diff_sources");
    write_file(
        &phase_dir(&root).join("01-01-PLAN.md"),
        &plan_doc("01", "Fallback", &[task_xml("Task", &["src/a.rs"])]),
    );

    let loaded = load_phase_provenance(&root, "01", LoadPhaseOptions::default()).unwrap();
    assert!(matches!(
        loaded.plans[0].change_groups[0].repo_buckets[0].files[0].diff_source,
        DiffSource::NonCommitFallback { .. }
    ));
    cleanup(&root);
}

#[test]
fn full_reviewer_provenance_suite() {
    let root = git_phase_fixture("full_reviewer_provenance_suite");
    write_json(
        &root.join(".planning/config.json"),
        r#"{"sub_repos":["frontend","backend"]}"#,
    );
    let frontend_commit = commit_file(&root, "frontend/src/app.ts", "console.log('a');\n");
    write_file(
        &phase_dir(&root).join("01-01-PLAN.md"),
        &plan_doc(
            "01",
            "Suite",
            &[
                task_xml("Frontend", &["frontend/src/app.ts"]),
                task_xml("Docs", &["docs/readme.md"]),
            ],
        ),
    );
    write_file(
        &phase_dir(&root).join("01-01-SUMMARY.md"),
        &summary_doc(&[format!("1. **Task 1: Frontend** - `{frontend_commit}`")]),
    );

    let loaded = load_phase_provenance(&root, "01", LoadPhaseOptions::default()).unwrap();
    assert_eq!(loaded.plans.len(), 1);
    assert_eq!(loaded.plans[0].change_groups.len(), 2);
    assert_eq!(
        loaded.plans[0].change_groups[0].repo_buckets[0]
            .repo
            .as_deref(),
        Some("frontend")
    );
    assert_eq!(
        loaded.plans[0].change_groups[1].commit_provenance.display(),
        "无"
    );
    cleanup(&root);
}

fn basic_phase_fixture(name: &str) -> PathBuf {
    let root = test_root(name);
    write_json(&root.join(".planning/config.json"), "{}");
    fs::create_dir_all(phase_dir(&root)).unwrap();
    root
}

fn git_phase_fixture(name: &str) -> PathBuf {
    let root = basic_phase_fixture(name);
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "tests@example.com"]);
    run_git(&root, &["config", "user.name", "Tests"]);
    write_file(&root.join(".gitignore"), "target/\n");
    run_git(&root, &["add", ".gitignore"]);
    run_git(&root, &["commit", "-m", "init"]);
    root
}

fn phase_dir(root: &Path) -> PathBuf {
    root.join(".planning/phases/01-reviewer-provenance-extraction")
}

fn plan_frontmatter(plan_number: &str) -> String {
    format!(
        "---\nphase: 01-reviewer-provenance-extraction\nplan: {plan_number}\nwave: 1\nfiles_modified:\n  - src/a.rs\n---\n# Phase 1 Plan {plan_number}\n"
    )
}

fn plan_doc(plan_number: &str, title: &str, tasks: &[String]) -> String {
    format!(
        "{}# {}\n\n<tasks>\n{}\n</tasks>\n",
        plan_frontmatter(plan_number),
        title,
        tasks.join("\n")
    )
}

fn task_xml(name: &str, files: &[&str]) -> String {
    format!(
        "<task type=\"auto\">\n  <name>{name}</name>\n  <files>{}</files>\n</task>",
        files.join(", ")
    )
}

fn summary_doc<T: AsRef<str>>(lines: &[T]) -> String {
    let body = lines
        .iter()
        .map(|line| line.as_ref().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "# Phase 1 Summary\n\n## Task Commits\n{}\n\n## Files Created/Modified\n- `src/main.rs` - wiring\n",
        body
    )
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut file = File::create(path).unwrap();
    file.write_all(content.as_bytes()).unwrap();
}

fn write_json(path: &Path, content: &str) {
    write_file(path, content);
}

fn commit_file(root: &Path, path: &str, content: &str) -> String {
    commit_files(root, &[(path, content)])
}

fn commit_files(root: &Path, files: &[(&str, &str)]) -> String {
    for (path, content) in files {
        write_file(&root.join(path), content);
        run_git(root, &["add", path]);
    }
    run_git(root, &["commit", "-m", "fixture"]);
    git_stdout(root, &["rev-parse", "--short", "HEAD"])
        .trim()
        .to_string()
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
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn test_root(name: &str) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("gsdv-{name}-{ts}"));
    fs::create_dir_all(&root).unwrap();
    root
}

fn cleanup(root: &Path) {
    let _ = fs::remove_dir_all(root);
}

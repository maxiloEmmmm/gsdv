use super::workflow::{
    WorkflowMutationRequest, WorkflowSaveRequest, apply_workflow_mutation, load_workflow_tree,
    save_workflow_step_editor, workflow_step_editor_from_node,
};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// 验证 workflow tree 会让叶子 step 继承根父 doc。
#[test]
fn workflow_leaf_step_inherits_root_doc() {
    let root = workflow_fixture("workflow-leaf-doc");

    let tree = load_workflow_tree(&root).unwrap();

    let task = &tree.projects[0].tasks[0];
    let leaf = &task.steps[0].children[0];
    let editor = workflow_step_editor_from_node(&task.path, leaf);
    assert_eq!(editor.doc_key, "root1");
    assert_eq!(editor.doc_text, "doc desc");
    assert_eq!(editor.step_text, "leaf desc");

    let _ = fs::remove_dir_all(root);
}

/// 验证保存 workflow 片段会同时写回 doc desc 和 step desc。
#[test]
fn workflow_save_updates_doc_and_step_desc() {
    let root = workflow_fixture("workflow-save");
    let task_path = PathBuf::from("gsdv-spec/ps/project1/task-a.md");

    save_workflow_step_editor(
        &root,
        WorkflowSaveRequest {
            task_path: task_path.clone(),
            step_path: vec![0, 0],
            doc_key: "root1".to_string(),
            doc_text: "next doc".to_string(),
            step_text: "next step".to_string(),
        },
    )
    .unwrap();

    let content = fs::read_to_string(root.join(task_path)).unwrap();
    assert!(content.contains("- root1\n  next doc"));
    assert!(content.contains("  - [ ] leaf1\n    next step"));

    let _ = fs::remove_dir_all(root);
}

/// 验证左侧 doc desc 清空时会移除 doc key。
#[test]
fn workflow_empty_doc_desc_removes_doc_key() {
    let root = workflow_fixture("workflow-empty-doc");
    let task_path = PathBuf::from("gsdv-spec/ps/project1/task-a.md");

    save_workflow_step_editor(
        &root,
        WorkflowSaveRequest {
            task_path: task_path.clone(),
            step_path: vec![0, 0],
            doc_key: "root1".to_string(),
            doc_text: String::new(),
            step_text: "leaf desc".to_string(),
        },
    )
    .unwrap();

    let content = fs::read_to_string(root.join(task_path)).unwrap();
    let doc_section = content.split("--doc--").nth(1).unwrap_or_default();
    assert!(!doc_section.contains("- root1\n"));
    assert!(content.contains("  - [ ] leaf1\n    leaf desc"));

    let _ = fs::remove_dir_all(root);
}

/// 验证新增 workflow task 会创建空 task 模板。
#[test]
fn workflow_add_task_creates_empty_template() {
    let root = workflow_fixture("workflow-add-task");

    apply_workflow_mutation(
        &root,
        WorkflowMutationRequest::AddTask {
            project_key: "project1".to_string(),
            task_key: "new.task".to_string(),
        },
    )
    .unwrap();

    let content = fs::read_to_string(root.join("gsdv-spec/ps/project1/task-new.task.md")).unwrap();
    assert_eq!(content, "--steps--\n---------\n\n--doc--\n--------\n");

    let _ = fs::remove_dir_all(root);
}

/// 验证新增同级 step 时不允许 key 重复。
#[test]
fn workflow_add_step_rejects_sibling_duplicate() {
    let root = workflow_fixture("workflow-add-duplicate");

    let error = apply_workflow_mutation(
        &root,
        WorkflowMutationRequest::AddStep {
            task_path: PathBuf::from("gsdv-spec/ps/project1/task-a.md"),
            parent_step_path: Some(vec![0]),
            key: "leaf1".to_string(),
            desc: String::new(),
        },
    )
    .unwrap_err();

    assert!(error.contains("already exists"));

    let _ = fs::remove_dir_all(root);
}

/// 验证新增子 step 会插入到父 step 子树末尾。
#[test]
fn workflow_add_child_step_appends_inside_parent_subtree() {
    let root = workflow_fixture("workflow-add-child");
    let task_path = PathBuf::from("gsdv-spec/ps/project1/task-a.md");

    apply_workflow_mutation(
        &root,
        WorkflowMutationRequest::AddStep {
            task_path: task_path.clone(),
            parent_step_path: Some(vec![0]),
            key: "leaf3".to_string(),
            desc: "new desc".to_string(),
        },
    )
    .unwrap();

    let content = fs::read_to_string(root.join(task_path)).unwrap();
    assert!(
        content.contains("  - [x] leaf2\n    done desc\n  - [ ] leaf3\n    new desc\n---------")
    );

    let _ = fs::remove_dir_all(root);
}

/// 验证删除顶级 step 会同时清理对应 doc 项。
#[test]
fn workflow_delete_root_step_removes_doc_item() {
    let root = workflow_fixture("workflow-delete-root");
    let task_path = PathBuf::from("gsdv-spec/ps/project1/task-a.md");

    apply_workflow_mutation(
        &root,
        WorkflowMutationRequest::DeleteStep {
            task_path: task_path.clone(),
            step_path: vec![0],
        },
    )
    .unwrap();

    let content = fs::read_to_string(root.join(task_path)).unwrap();
    assert!(!content.contains("- root1\n"));
    assert!(!content.contains("leaf1"));
    assert!(!content.contains("leaf2"));

    let _ = fs::remove_dir_all(root);
}

/// 验证删除嵌套非叶子 step 会清理同名 doc 项。
#[test]
fn workflow_delete_nested_parent_step_removes_doc_item() {
    let root = workflow_fixture("workflow-delete-nested-parent");
    let task_path = PathBuf::from("gsdv-spec/ps/project1/task-a.md");
    fs::write(
        root.join(&task_path),
        "--steps--\n- root1\n  - branch\n    - [ ] leaf1\n      leaf desc\n---------\n\n--doc--\n- root1\n  root doc\n- branch\n  branch doc\n--------\n",
    )
    .unwrap();

    apply_workflow_mutation(
        &root,
        WorkflowMutationRequest::DeleteStep {
            task_path: task_path.clone(),
            step_path: vec![0, 0],
        },
    )
    .unwrap();

    let content = fs::read_to_string(root.join(task_path)).unwrap();
    assert!(content.contains("- root1\n---------"));
    assert!(content.contains("- root1\n  root doc"));
    assert!(!content.contains("branch"));
    assert!(!content.contains("leaf1"));

    let _ = fs::remove_dir_all(root);
}

/// 验证重命名 project 会移动项目目录。
#[test]
fn workflow_rename_project_moves_project_directory() {
    let root = workflow_fixture("workflow-rename-project");

    apply_workflow_mutation(
        &root,
        WorkflowMutationRequest::RenameProject {
            project_key: "project1".to_string(),
            new_key: "project2".to_string(),
        },
    )
    .unwrap();

    assert!(!root.join("gsdv-spec/ps/project1").exists());
    assert!(root.join("gsdv-spec/ps/project2/root.md").is_file());

    let _ = fs::remove_dir_all(root);
}

/// 验证重命名 task 会移动 task 文件。
#[test]
fn workflow_rename_task_moves_task_file() {
    let root = workflow_fixture("workflow-rename-task");

    apply_workflow_mutation(
        &root,
        WorkflowMutationRequest::RenameTask {
            task_path: PathBuf::from("gsdv-spec/ps/project1/task-a.md"),
            new_key: "b".to_string(),
        },
    )
    .unwrap();

    assert!(!root.join("gsdv-spec/ps/project1/task-a.md").exists());
    assert!(root.join("gsdv-spec/ps/project1/task-b.md").is_file());

    let _ = fs::remove_dir_all(root);
}

/// 验证重命名非叶子 step 会同步 doc key 并保留 desc。
#[test]
fn workflow_rename_parent_step_updates_doc_key() {
    let root = workflow_fixture("workflow-rename-parent-step");
    let task_path = PathBuf::from("gsdv-spec/ps/project1/task-a.md");

    apply_workflow_mutation(
        &root,
        WorkflowMutationRequest::RenameStep {
            task_path: task_path.clone(),
            step_path: vec![0],
            new_key: "root2".to_string(),
        },
    )
    .unwrap();

    let content = fs::read_to_string(root.join(task_path)).unwrap();
    assert!(content.contains("- root2\n  - [ ] leaf1"));
    assert!(content.contains("--doc--\n- root2\n  doc desc\n--------"));
    assert!(!content.contains("- root1\n  doc desc"));

    let _ = fs::remove_dir_all(root);
}

/// 验证 rename key 不允许空白字符。
#[test]
fn workflow_rename_rejects_whitespace_key() {
    let root = workflow_fixture("workflow-rename-whitespace");

    let error = apply_workflow_mutation(
        &root,
        WorkflowMutationRequest::RenameTask {
            task_path: PathBuf::from("gsdv-spec/ps/project1/task-a.md"),
            new_key: "bad key".to_string(),
        },
    )
    .unwrap_err();

    assert!(error.contains("spaces or newlines"));

    let _ = fs::remove_dir_all(root);
}

/// 创建带有 gsdv-spec 的临时 workspace。
fn workflow_fixture(name: &str) -> PathBuf {
    let root = unique_temp_dir(name);
    let project = root.join("gsdv-spec/ps/project1");
    fs::create_dir_all(&project).unwrap();
    fs::write(root.join("gsdv-spec/root.md"), "# root\n").unwrap();
    fs::write(project.join("root.md"), "# project1\n").unwrap();
    fs::write(
        project.join("task-a.md"),
        "--steps--\n- root1\n  - [ ] leaf1\n    leaf desc\n  - [x] leaf2\n    done desc\n---------\n\n--doc--\n- root1\n  doc desc\n--------\n",
    )
    .unwrap();
    root
}

/// 生成不会和并发测试互相覆盖的临时目录。
fn unique_temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("gsdv-{name}-{nanos}"))
}

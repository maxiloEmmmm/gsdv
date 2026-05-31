use super::workflow::{
    WorkflowMutationRequest, WorkflowSaveRequest, apply_workflow_mutation, load_workflow_tree,
    save_workflow_step_editor, workflow_step_editor_from_node, workflow_task_editor_from_node,
};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// 验证 workflow task 会按二级 checkbox heading 解析扁平 step。
#[test]
fn workflow_task_parses_heading_steps() {
    let root = workflow_fixture("workflow-heading-steps");

    let tree = load_workflow_tree(&root).unwrap();

    let task = &tree.projects[0].tasks[0];
    assert_eq!(task.desc, "Task intro\n");
    assert_eq!(task.steps.len(), 2);
    assert_eq!(task.steps[0].path, vec![0]);
    assert_eq!(task.steps[0].title, "完成中文 step");
    assert!(task.steps[0].checked);
    assert_eq!(task.steps[0].desc, "doc desc\nstep desc");
    assert!(task.steps[0].children.is_empty());
    assert_eq!(task.steps[1].title, "next step with spaces");
    assert!(!task.steps[1].checked);

    let editor = workflow_step_editor_from_node(&task.path, &task.steps[0]);
    assert_eq!(editor.step_text, "doc desc\nstep desc");
    assert!(!editor.is_dirty());
    let task_editor = workflow_task_editor_from_node(task);
    assert_eq!(task_editor.task_text, "Task intro\n");
    assert!(!task_editor.is_dirty());

    let _ = fs::remove_dir_all(root);
}

/// 验证保存 workflow task 说明只写回首个 step 之前的内容。
#[test]
fn workflow_save_updates_task_desc() {
    let root = workflow_fixture("workflow-save-task-desc");
    let task_path = PathBuf::from("gsdv-spec/ps/project1/task-a.md");

    save_workflow_step_editor(
        &root,
        WorkflowSaveRequest {
            task_path: task_path.clone(),
            task_text: "Updated task intro\n".to_string(),
            step_path: None,
            step_text: None,
        },
    )
    .unwrap();

    let content = fs::read_to_string(root.join(task_path)).unwrap();
    assert!(content.starts_with("Updated task intro\n\n## [x] 完成中文 step\n"));
    assert!(content.contains("doc desc\nstep desc\n"));

    let _ = fs::remove_dir_all(root);
}

/// 验证保存 workflow step 只写回当前 heading 下的正文。
#[test]
fn workflow_save_updates_step_desc() {
    let root = workflow_fixture("workflow-save");
    let task_path = PathBuf::from("gsdv-spec/ps/project1/task-a.md");

    save_workflow_step_editor(
        &root,
        WorkflowSaveRequest {
            task_path: task_path.clone(),
            task_text: "Task intro\n".to_string(),
            step_path: Some(vec![0]),
            step_text: Some("next desc\nmore".to_string()),
        },
    )
    .unwrap();

    let content = fs::read_to_string(root.join(task_path)).unwrap();
    assert!(content.contains("## [x] 完成中文 step\nnext desc\nmore\n"));
    assert!(content.contains("## [ ] next step with spaces\npending desc\n"));

    let _ = fs::remove_dir_all(root);
}

/// 验证 task 说明不能保存新的 step heading。
#[test]
fn workflow_save_rejects_step_heading_inside_task_desc() {
    let root = workflow_fixture("workflow-save-heading-task-desc");
    let task_path = PathBuf::from("gsdv-spec/ps/project1/task-a.md");

    let error = save_workflow_step_editor(
        &root,
        WorkflowSaveRequest {
            task_path,
            task_text: "before\n## [ ] nested\n".to_string(),
            step_path: None,
            step_text: None,
        },
    )
    .unwrap_err();

    assert!(error.contains("## [ ]"));

    let _ = fs::remove_dir_all(root);
}

/// 验证 step 正文不能保存新的 step heading。
#[test]
fn workflow_save_rejects_step_heading_inside_desc() {
    let root = workflow_fixture("workflow-save-heading-desc");
    let task_path = PathBuf::from("gsdv-spec/ps/project1/task-a.md");

    let error = save_workflow_step_editor(
        &root,
        WorkflowSaveRequest {
            task_path,
            task_text: "Task intro\n".to_string(),
            step_path: Some(vec![0]),
            step_text: Some("before\n## [ ] nested\n".to_string()),
        },
    )
    .unwrap_err();

    assert!(error.contains("## [ ]"));

    let _ = fs::remove_dir_all(root);
}

/// 验证新增 workflow task 会创建空 Markdown 文件。
#[test]
fn workflow_add_task_creates_empty_file() {
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
    assert_eq!(content, "");

    let _ = fs::remove_dir_all(root);
}

/// 验证新增 step 会追加二级 checkbox heading 和正文。
#[test]
fn workflow_add_step_appends_heading() {
    let root = workflow_fixture("workflow-add-step");
    let task_path = PathBuf::from("gsdv-spec/ps/project1/task-a.md");

    apply_workflow_mutation(
        &root,
        WorkflowMutationRequest::AddStep {
            task_path: task_path.clone(),
            key: "新增 step title".to_string(),
            desc: "new desc".to_string(),
        },
    )
    .unwrap();

    let content = fs::read_to_string(root.join(task_path)).unwrap();
    assert!(content.ends_with("## [ ] 新增 step title\nnew desc\n"));

    let _ = fs::remove_dir_all(root);
}

/// 验证新增 step 不允许同 task 内标题重复。
#[test]
fn workflow_add_step_rejects_duplicate_title() {
    let root = workflow_fixture("workflow-add-duplicate");

    let error = apply_workflow_mutation(
        &root,
        WorkflowMutationRequest::AddStep {
            task_path: PathBuf::from("gsdv-spec/ps/project1/task-a.md"),
            key: "完成中文 step".to_string(),
            desc: String::new(),
        },
    )
    .unwrap_err();

    assert!(error.contains("already exists"));

    let _ = fs::remove_dir_all(root);
}

/// 验证 step 标题可以包含空格和中文。
#[test]
fn workflow_rename_step_allows_spaces_and_chinese() {
    let root = workflow_fixture("workflow-rename-step");
    let task_path = PathBuf::from("gsdv-spec/ps/project1/task-a.md");

    apply_workflow_mutation(
        &root,
        WorkflowMutationRequest::RenameStep {
            task_path: task_path.clone(),
            step_path: vec![1],
            new_key: "新的 step 名称".to_string(),
        },
    )
    .unwrap();

    let content = fs::read_to_string(root.join(task_path)).unwrap();
    assert!(content.contains("## [ ] 新的 step 名称\npending desc\n"));
    assert!(!content.contains("## [ ] next step with spaces"));

    let _ = fs::remove_dir_all(root);
}

/// 验证删除 step 会删除整个二级 heading 区块。
#[test]
fn workflow_delete_step_removes_heading_block() {
    let root = workflow_fixture("workflow-delete-step");
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
    assert!(!content.contains("完成中文 step"));
    assert!(!content.contains("doc desc"));
    assert!(content.contains("## [ ] next step with spaces\npending desc\n"));

    let _ = fs::remove_dir_all(root);
}

/// 验证 task/project key 仍不允许空白字符。
#[test]
fn workflow_rename_task_rejects_whitespace_key() {
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
        "Task intro\n\n## [x] 完成中文 step\ndoc desc\nstep desc\n\n## [ ] next step with spaces\npending desc\n",
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

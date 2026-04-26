//! gsdv-spec workflow 文档解析和片段写回。
//!
//! 本模块只处理文件内容和结构化数据，不直接修改 egui 渲染状态。

use std::fs;
use std::path::{Path, PathBuf};

/// workspace 根目录下的 workflow 规范目录。
const GSDV_SPEC_DIR: &str = "gsdv-spec";
/// 根项目索引文件名。
const ROOT_MD: &str = "root.md";
/// 子项目目录名。
const PROJECTS_DIR: &str = "ps";
/// task 文件名前缀。
const TASK_PREFIX: &str = "task-";
/// Markdown 文件扩展名。
const MARKDOWN_EXT: &str = "md";
/// steps 区块标记。
const STEPS_MARKER: &str = "--steps--";
/// doc 区块标记。
const DOC_MARKER: &str = "--doc--";

/// workflow 树加载结果。
#[derive(Debug, Clone, Default)]
pub(super) struct WorkflowTree {
    /// workspace 内相对的 gsdv-spec 目录。
    pub spec_path: PathBuf,
    /// 当前 workspace 的 workflow 项目列表。
    pub projects: Vec<WorkflowProjectNode>,
}

/// workflow 项目节点。
#[derive(Debug, Clone)]
pub(super) struct WorkflowProjectNode {
    /// 项目目录名，也是 tree 中的稳定 key。
    pub key: String,
    /// tree 中显示的项目名。
    pub label: String,
    /// 项目 root.md 相对 workspace 的路径。
    pub root_path: PathBuf,
    /// 项目下的 task 文档节点。
    pub tasks: Vec<WorkflowTaskNode>,
}

/// workflow task 节点。
#[derive(Debug, Clone)]
pub(super) struct WorkflowTaskNode {
    /// task 文件名去扩展名后的展示文本。
    pub label: String,
    /// task 文档相对 workspace 的路径。
    pub path: PathBuf,
    /// task 文档内的 step 树。
    pub steps: Vec<WorkflowStepNode>,
}

/// workflow step 节点。
#[derive(Debug, Clone)]
pub(super) struct WorkflowStepNode {
    /// step 在同级列表中的索引路径。
    pub path: Vec<usize>,
    /// step 标题，不包含 checkbox。
    pub title: String,
    /// step 是否已经完成。
    pub checked: bool,
    /// step 是否有 checkbox。
    pub checkable: bool,
    /// step 自己的 desc 文本，不包含 step 行。
    pub desc: String,
    /// 叶子 step 继承的根父 step 标题。
    pub root_doc_key: String,
    /// 叶子 step 继承的根父 doc desc。
    pub root_doc_text: String,
    /// 子 step 列表。
    pub children: Vec<WorkflowStepNode>,
}

/// workflow 选择目标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WorkflowSelectionTarget {
    /// 项目 root.md。
    Project { root_path: PathBuf },
    /// task 专属 workflow 界面。
    Task { task_path: PathBuf },
    /// task 文档内的某个 step。
    Step {
        task_path: PathBuf,
        step_path: Vec<usize>,
    },
}

/// workflow step 编辑器展示模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkflowStepEditorMode {
    /// 只编辑根 step 对应的 doc desc。
    DocOnly,
    /// 同时编辑 doc desc 和 step 自身 desc。
    DocAndStep,
}

/// workflow 片段编辑状态。
#[derive(Debug, Clone)]
pub(super) struct WorkflowStepEditor {
    /// 当前编辑的选择目标。
    pub target: WorkflowSelectionTarget,
    /// task 文档相对 workspace 的路径。
    pub task_path: PathBuf,
    /// step 在 task 文档中的索引路径。
    pub step_path: Vec<usize>,
    /// step 标题。
    pub step_title: String,
    /// 当前 step 编辑器展示模式。
    pub mode: WorkflowStepEditorMode,
    /// 左侧 doc editor 对应的根父 key。
    pub doc_key: String,
    /// 左侧 doc desc 当前文本。
    pub doc_text: String,
    /// 左侧 doc desc 已保存文本。
    pub saved_doc_text: String,
    /// 右侧叶子 step desc 当前文本。
    pub step_text: String,
    /// 右侧叶子 step desc 已保存文本。
    pub saved_step_text: String,
    /// 最近一次保存错误。
    pub save_error: Option<String>,
}

impl WorkflowStepEditor {
    /// 判断左右任意片段是否有未保存修改。
    pub(super) fn is_dirty(&self) -> bool {
        self.doc_text != self.saved_doc_text || self.step_text != self.saved_step_text
    }
}

/// workflow 片段保存请求。
#[derive(Debug, Clone)]
pub(super) struct WorkflowSaveRequest {
    /// task 文档相对 workspace 的路径。
    pub task_path: PathBuf,
    /// step 在 task 文档中的索引路径。
    pub step_path: Vec<usize>,
    /// 左侧 doc desc 对应的 key。
    pub doc_key: String,
    /// 需要写入的左侧 doc desc。
    pub doc_text: String,
    /// 需要写入的右侧 step desc。
    pub step_text: String,
}

/// workflow 片段保存成功结果。
#[derive(Debug, Clone)]
pub(super) struct WorkflowSaveSuccess {
    /// 保存后的左侧 doc desc。
    pub doc_text: String,
    /// 保存后的右侧 step desc。
    pub step_text: String,
}

/// workflow tree 右键菜单触发的文件修改请求。
#[derive(Debug, Clone)]
pub(super) enum WorkflowMutationRequest {
    /// 在项目目录下创建一个空 task Markdown 文件。
    AddTask {
        /// 项目目录名。
        project_key: String,
        /// task key，不包含 `task-` 前缀和 `.md` 后缀。
        task_key: String,
    },
    /// 在 task 的 steps 区块中新增 step。
    AddStep {
        /// task 文档相对 workspace 的路径。
        task_path: PathBuf,
        /// 父 step 路径；为空表示新增顶级 step。
        parent_step_path: Option<Vec<usize>>,
        /// 新 step key。
        key: String,
        /// 新 step desc。
        desc: String,
    },
    /// 重命名 workflow project 目录。
    RenameProject {
        /// 原项目目录名。
        project_key: String,
        /// 新项目目录名。
        new_key: String,
    },
    /// 重命名 workflow task 文件。
    RenameTask {
        /// task 文档相对 workspace 的路径。
        task_path: PathBuf,
        /// 新 task key，不包含 `task-` 前缀和 `.md` 后缀。
        new_key: String,
    },
    /// 重命名 task 文档里的 step。
    RenameStep {
        /// task 文档相对 workspace 的路径。
        task_path: PathBuf,
        /// 要重命名的 step 路径。
        step_path: Vec<usize>,
        /// 新 step key。
        new_key: String,
    },
    /// 删除整个 workflow project 目录。
    DeleteProject {
        /// 项目目录名。
        project_key: String,
    },
    /// 删除一个 task Markdown 文件。
    DeleteTask {
        /// task 文档相对 workspace 的路径。
        task_path: PathBuf,
    },
    /// 删除 task 文档内的一个 step 及其子树。
    DeleteStep {
        /// task 文档相对 workspace 的路径。
        task_path: PathBuf,
        /// 要删除的 step 路径。
        step_path: Vec<usize>,
    },
}

/// 从已解析的 step 节点创建片段编辑器状态。
pub(super) fn workflow_step_editor_from_node(
    task_path: &Path,
    node: &WorkflowStepNode,
) -> WorkflowStepEditor {
    let target = WorkflowSelectionTarget::Step {
        task_path: task_path.to_path_buf(),
        step_path: node.path.clone(),
    };
    WorkflowStepEditor {
        target,
        task_path: task_path.to_path_buf(),
        step_path: node.path.clone(),
        step_title: node.title.clone(),
        mode: workflow_step_editor_mode(node),
        doc_key: node.root_doc_key.clone(),
        doc_text: node.root_doc_text.clone(),
        saved_doc_text: node.root_doc_text.clone(),
        step_text: node.desc.clone(),
        saved_step_text: node.desc.clone(),
        save_error: None,
    }
}

/// 根据 step 层级和子节点决定 editor 展示模式。
fn workflow_step_editor_mode(node: &WorkflowStepNode) -> WorkflowStepEditorMode {
    if node.path.len() == 1 && !node.children.is_empty() {
        WorkflowStepEditorMode::DocOnly
    } else {
        WorkflowStepEditorMode::DocAndStep
    }
}

/// 从 workspace 根目录加载 workflow tree。
pub(super) fn load_workflow_tree(workspace_root: &Path) -> Result<WorkflowTree, String> {
    let spec_root = workspace_root.join(GSDV_SPEC_DIR);
    let root_md = spec_root.join(ROOT_MD);
    if !root_md.is_file() {
        return Err(format!("{} not found", root_md.display()));
    }
    let projects_root = spec_root.join(PROJECTS_DIR);
    let mut projects = Vec::new();
    for entry in sorted_dirs(&projects_root)? {
        let root_path = entry.join(ROOT_MD);
        if !root_path.is_file() {
            continue;
        }
        let Some(project_name) = entry.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let tasks = load_project_tasks(workspace_root, &entry)?;
        projects.push(WorkflowProjectNode {
            key: project_name.to_string(),
            label: project_name.to_string(),
            root_path: relative_to_workspace(workspace_root, &root_path),
            tasks,
        });
    }
    Ok(WorkflowTree {
        spec_path: PathBuf::from(GSDV_SPEC_DIR),
        projects,
    })
}

/// 从一个项目目录加载 task 文档列表。
fn load_project_tasks(
    workspace_root: &Path,
    project_dir: &Path,
) -> Result<Vec<WorkflowTaskNode>, String> {
    let mut tasks = Vec::new();
    for path in sorted_markdown_tasks(project_dir)? {
        let content = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let steps = parse_task_steps(&content);
        let label = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_else(|| "task".to_string());
        tasks.push(WorkflowTaskNode {
            label,
            path: relative_to_workspace(workspace_root, &path),
            steps,
        });
    }
    Ok(tasks)
}

/// 返回按文件名排序的直接子目录。
fn sorted_dirs(root: &Path) -> Result<Vec<PathBuf>, String> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut dirs = Vec::new();
    for entry in
        fs::read_dir(root).map_err(|error| format!("failed to read {}: {error}", root.display()))?
    {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs.sort_by(|left, right| file_name_string(left).cmp(&file_name_string(right)));
    Ok(dirs)
}

/// 返回按文件名排序的 task Markdown 文件。
fn sorted_markdown_tasks(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut tasks = Vec::new();
    for entry in
        fs::read_dir(root).map_err(|error| format!("failed to read {}: {error}", root.display()))?
    {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let is_task = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(TASK_PREFIX))
            && path
                .extension()
                .is_some_and(|extension| extension == MARKDOWN_EXT);
        if is_task {
            tasks.push(path);
        }
    }
    tasks.sort_by(|left, right| file_name_string(left).cmp(&file_name_string(right)));
    Ok(tasks)
}

/// 将绝对路径转成 workspace 相对路径。
fn relative_to_workspace(workspace_root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(workspace_root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf())
}

/// 获取稳定文件名排序键。
fn file_name_string(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// 为指定叶子 step 构建片段编辑器状态。
pub(super) fn build_workflow_step_editor(
    workspace_root: &Path,
    task_path: &Path,
    step_path: &[usize],
) -> Result<WorkflowStepEditor, String> {
    let absolute = workspace_root.join(task_path);
    let content = fs::read_to_string(&absolute)
        .map_err(|error| format!("failed to read {}: {error}", absolute.display()))?;
    let records = parse_step_records(&content);
    let Some(record) = records.iter().find(|record| record.path == step_path) else {
        return Err("step not found".to_string());
    };
    let docs = parse_doc_items(&content);
    let doc_text = docs
        .iter()
        .find(|doc| doc.key == record.root_doc_key)
        .map(|doc| doc.desc.clone())
        .unwrap_or_default();
    let target = WorkflowSelectionTarget::Step {
        task_path: task_path.to_path_buf(),
        step_path: step_path.to_vec(),
    };
    Ok(WorkflowStepEditor {
        target,
        task_path: task_path.to_path_buf(),
        step_path: step_path.to_vec(),
        step_title: record.title.clone(),
        mode: if record.path.len() == 1
            && records.iter().any(|candidate| {
                candidate.path.starts_with(&record.path) && candidate.path.len() > 1
            }) {
            WorkflowStepEditorMode::DocOnly
        } else {
            WorkflowStepEditorMode::DocAndStep
        },
        doc_key: record.root_doc_key.clone(),
        doc_text: doc_text.clone(),
        saved_doc_text: doc_text,
        step_text: record.desc.clone(),
        saved_step_text: record.desc.clone(),
        save_error: None,
    })
}

/// 保存 workflow 片段到同一个 task Markdown 文件。
pub(super) fn save_workflow_step_editor(
    workspace_root: &Path,
    request: WorkflowSaveRequest,
) -> Result<WorkflowSaveSuccess, String> {
    let absolute = workspace_root.join(&request.task_path);
    let content = fs::read_to_string(&absolute)
        .map_err(|error| format!("failed to read {}: {error}", absolute.display()))?;
    let mut lines = markdown_lines(&content);
    replace_step_desc(&mut lines, &request.step_path, &request.step_text)?;
    replace_doc_desc(&mut lines, &request.doc_key, &request.doc_text);
    let next_content = join_markdown_lines(&lines);
    fs::write(&absolute, next_content.as_bytes())
        .map_err(|error| format!("failed to write {}: {error}", absolute.display()))?;
    Ok(WorkflowSaveSuccess {
        doc_text: request.doc_text,
        step_text: request.step_text,
    })
}

/// 应用 workflow tree 右键菜单触发的文件修改。
pub(super) fn apply_workflow_mutation(
    workspace_root: &Path,
    request: WorkflowMutationRequest,
) -> Result<(), String> {
    match request {
        WorkflowMutationRequest::AddTask {
            project_key,
            task_key,
        } => add_workflow_task(workspace_root, &project_key, &task_key),
        WorkflowMutationRequest::AddStep {
            task_path,
            parent_step_path,
            key,
            desc,
        } => add_workflow_step(
            workspace_root,
            &task_path,
            parent_step_path.as_deref(),
            &key,
            &desc,
        ),
        WorkflowMutationRequest::RenameProject {
            project_key,
            new_key,
        } => rename_workflow_project(workspace_root, &project_key, &new_key),
        WorkflowMutationRequest::RenameTask { task_path, new_key } => {
            rename_workflow_task(workspace_root, &task_path, &new_key)
        }
        WorkflowMutationRequest::RenameStep {
            task_path,
            step_path,
            new_key,
        } => rename_workflow_step(workspace_root, &task_path, &step_path, &new_key),
        WorkflowMutationRequest::DeleteProject { project_key } => {
            delete_workflow_project(workspace_root, &project_key)
        }
        WorkflowMutationRequest::DeleteTask { task_path } => {
            delete_workflow_task(workspace_root, &task_path)
        }
        WorkflowMutationRequest::DeleteStep {
            task_path,
            step_path,
        } => delete_workflow_step(workspace_root, &task_path, &step_path),
    }
}

/// 校验 workflow key 是否能作为 task 或 step 的同级唯一 key。
pub(super) fn validate_workflow_key(key: &str) -> Result<&str, String> {
    if key.trim().is_empty() {
        return Err("Key is required".to_string());
    }
    if key != key.trim() || key.chars().any(char::is_whitespace) {
        return Err("Key cannot contain spaces or newlines".to_string());
    }
    if key.contains('/') || key.contains('\\') {
        return Err("Key cannot contain path separators".to_string());
    }
    Ok(key)
}

/// 在项目目录下创建空 task Markdown 文件。
fn add_workflow_task(
    workspace_root: &Path,
    project_key: &str,
    task_key: &str,
) -> Result<(), String> {
    let project_key = validate_workflow_key(project_key)?;
    let task_key = validate_workflow_key(task_key)?;
    let project_dir = workspace_root
        .join(GSDV_SPEC_DIR)
        .join(PROJECTS_DIR)
        .join(project_key);
    if !project_dir.is_dir() {
        return Err(format!("workflow project not found: {project_key}"));
    }
    let task_path = project_dir.join(format!("{TASK_PREFIX}{task_key}.{MARKDOWN_EXT}"));
    if task_path.exists() {
        return Err(format!("task already exists: {task_key}"));
    }
    let content = "--steps--\n---------\n\n--doc--\n--------\n";
    fs::write(&task_path, content.as_bytes())
        .map_err(|error| format!("failed to write {}: {error}", task_path.display()))
}

/// 在 task 文档中新增顶级 step 或子 step。
fn add_workflow_step(
    workspace_root: &Path,
    task_path: &Path,
    parent_step_path: Option<&[usize]>,
    key: &str,
    desc: &str,
) -> Result<(), String> {
    let key = validate_workflow_key(key)?;
    let absolute = workspace_root.join(task_path);
    let content = fs::read_to_string(&absolute)
        .map_err(|error| format!("failed to read {}: {error}", absolute.display()))?;
    let mut lines = markdown_lines(&content);
    let section = ensure_steps_section(&mut lines);
    let content = join_markdown_lines(&lines);
    let records = parse_step_records(&content);
    let (insert_at, indent) = if let Some(parent_path) = parent_step_path {
        // 触发条件：外部入口尝试给子 step 新增子节点。
        // 不能只靠 UI 隐藏菜单：mutation 也可能从其他入口调用。
        // 防止回归：workflow 退化成多级 step tree。
        if parent_path.len() > 1 {
            return Err("child step cannot have child steps".to_string());
        }
        let parent_index = records
            .iter()
            .position(|record| record.path == parent_path)
            .ok_or_else(|| "parent step not found".to_string())?;
        if records.iter().any(|record| {
            record.path.len() == parent_path.len() + 1
                && record.path.starts_with(parent_path)
                && record.title == key
        }) {
            return Err(format!("step already exists at this level: {key}"));
        }
        let parent = &records[parent_index];
        let insert_at = step_subtree_end(&records, parent_index, section.content_end);
        (insert_at, parent.indent + 2)
    } else {
        if records
            .iter()
            .any(|record| record.path.len() == 1 && record.title == key)
        {
            return Err(format!("step already exists at this level: {key}"));
        }
        (section.content_end, 0)
    };
    let mut replacement = vec![format!("{}- [ ] {key}", " ".repeat(indent))];
    replacement.extend(indent_text_lines(desc, indent + 2));
    lines.splice(insert_at..insert_at, replacement);
    let next_content = join_markdown_lines(&lines);
    fs::write(&absolute, next_content.as_bytes())
        .map_err(|error| format!("failed to write {}: {error}", absolute.display()))
}

/// 重命名 workflow project 目录。
fn rename_workflow_project(
    workspace_root: &Path,
    project_key: &str,
    new_key: &str,
) -> Result<(), String> {
    let project_key = validate_workflow_key(project_key)?;
    let new_key = validate_workflow_key(new_key)?;
    if project_key == new_key {
        return Ok(());
    }
    let projects_dir = workspace_root.join(GSDV_SPEC_DIR).join(PROJECTS_DIR);
    let old_dir = projects_dir.join(project_key);
    let new_dir = projects_dir.join(new_key);
    if !old_dir.is_dir() {
        return Err(format!("workflow project not found: {project_key}"));
    }
    if new_dir.exists() {
        return Err(format!("project already exists: {new_key}"));
    }
    fs::rename(&old_dir, &new_dir).map_err(|error| {
        format!(
            "failed to rename {} to {}: {error}",
            old_dir.display(),
            new_dir.display()
        )
    })
}

/// 重命名 workflow task Markdown 文件。
fn rename_workflow_task(
    workspace_root: &Path,
    task_path: &Path,
    new_key: &str,
) -> Result<(), String> {
    let new_key = validate_workflow_key(new_key)?;
    let absolute = workspace_root.join(task_path);
    if !absolute.is_file() {
        return Err(format!("workflow task not found: {}", task_path.display()));
    }
    let current_key = task_key_from_path(task_path)?;
    if current_key == new_key {
        return Ok(());
    }
    let parent = absolute
        .parent()
        .ok_or_else(|| format!("task parent not found: {}", task_path.display()))?;
    let target = parent.join(format!("{TASK_PREFIX}{new_key}.{MARKDOWN_EXT}"));
    if target.exists() {
        return Err(format!("task already exists at this level: {new_key}"));
    }
    fs::rename(&absolute, &target).map_err(|error| {
        format!(
            "failed to rename {} to {}: {error}",
            absolute.display(),
            target.display()
        )
    })
}

/// 重命名 workflow step，并同步非叶子 step 的 doc key。
fn rename_workflow_step(
    workspace_root: &Path,
    task_path: &Path,
    step_path: &[usize],
    new_key: &str,
) -> Result<(), String> {
    let new_key = validate_workflow_key(new_key)?;
    let absolute = workspace_root.join(task_path);
    let content = fs::read_to_string(&absolute)
        .map_err(|error| format!("failed to read {}: {error}", absolute.display()))?;
    let mut lines = markdown_lines(&content);
    let records = parse_step_records(&content);
    let index = records
        .iter()
        .position(|record| record.path == step_path)
        .ok_or_else(|| "step not found".to_string())?;
    let record = records[index].clone();
    if record.title == new_key {
        return Ok(());
    }
    if records.iter().any(|candidate| {
        candidate.path != record.path
            && candidate.path.len() == record.path.len()
            && candidate.path[..candidate.path.len().saturating_sub(1)]
                == record.path[..record.path.len().saturating_sub(1)]
            && candidate.title == new_key
    }) {
        return Err(format!("step already exists at this level: {new_key}"));
    }
    lines[record.line_index] = renamed_step_line(&record, new_key);
    if step_has_children(&records, index) {
        rename_doc_item_key_if_exists(&mut lines, &record.title, new_key)?;
    }
    let next_content = join_markdown_lines(&lines);
    fs::write(&absolute, next_content.as_bytes())
        .map_err(|error| format!("failed to write {}: {error}", absolute.display()))
}

/// 删除 workflow project 目录。
fn delete_workflow_project(workspace_root: &Path, project_key: &str) -> Result<(), String> {
    let project_key = validate_workflow_key(project_key)?;
    let project_dir = workspace_root
        .join(GSDV_SPEC_DIR)
        .join(PROJECTS_DIR)
        .join(project_key);
    if !project_dir.is_dir() {
        return Err(format!("workflow project not found: {project_key}"));
    }
    fs::remove_dir_all(&project_dir)
        .map_err(|error| format!("failed to delete {}: {error}", project_dir.display()))
}

/// 删除 workflow task Markdown 文件。
fn delete_workflow_task(workspace_root: &Path, task_path: &Path) -> Result<(), String> {
    let absolute = workspace_root.join(task_path);
    if !absolute.is_file() {
        return Err(format!("workflow task not found: {}", task_path.display()));
    }
    fs::remove_file(&absolute)
        .map_err(|error| format!("failed to delete {}: {error}", absolute.display()))
}

/// 删除 task 文档里的 step 子树。
fn delete_workflow_step(
    workspace_root: &Path,
    task_path: &Path,
    step_path: &[usize],
) -> Result<(), String> {
    let absolute = workspace_root.join(task_path);
    let content = fs::read_to_string(&absolute)
        .map_err(|error| format!("failed to read {}: {error}", absolute.display()))?;
    let mut lines = markdown_lines(&content);
    let section =
        find_section(&lines, STEPS_MARKER).ok_or_else(|| "steps section not found".to_string())?;
    let records = parse_step_records(&content);
    let index = records
        .iter()
        .position(|record| record.path == step_path)
        .ok_or_else(|| "step not found".to_string())?;
    let record = records[index].clone();
    let delete_end = step_subtree_end(&records, index, section.content_end);
    lines.splice(record.line_index..delete_end, Vec::<String>::new());
    if step_has_children(&records, index) {
        delete_doc_item_if_exists(&mut lines, &record.title);
    }
    let next_content = join_markdown_lines(&lines);
    fs::write(&absolute, next_content.as_bytes())
        .map_err(|error| format!("failed to write {}: {error}", absolute.display()))
}

/// 计算一个 step 及其子树结束行。
fn step_subtree_end(records: &[StepRecord], index: usize, fallback_end: usize) -> usize {
    let parent_indent = records[index].indent;
    records[index + 1..]
        .iter()
        .find(|record| record.indent <= parent_indent)
        .map(|record| record.line_index)
        .unwrap_or(fallback_end)
}

/// 解析 task 文档中的 step 树。
fn parse_task_steps(content: &str) -> Vec<WorkflowStepNode> {
    let records = parse_step_records(content);
    let docs = parse_doc_items(content);
    let mut roots = Vec::new();
    let mut stack: Vec<(usize, Vec<usize>)> = Vec::new();
    for record in records {
        while stack
            .last()
            .is_some_and(|(indent, _)| *indent >= record.indent)
        {
            stack.pop();
        }
        let mut path = stack
            .last()
            .map(|(_, path)| path.clone())
            .unwrap_or_default();
        let sibling_index = children_len_at_path(&roots, &path);
        path.push(sibling_index);
        let node = WorkflowStepNode {
            path: path.clone(),
            title: record.title,
            checked: record.checked,
            checkable: record.checkable,
            desc: record.desc,
            root_doc_text: docs
                .iter()
                .find(|doc| doc.key == record.root_doc_key)
                .map(|doc| doc.desc.clone())
                .unwrap_or_default(),
            root_doc_key: record.root_doc_key,
            children: Vec::new(),
        };
        push_step_node_at_path(&mut roots, &path, node);
        stack.push((record.indent, path));
    }
    roots
}

/// 统计指定路径下已有子节点数量。
fn children_len_at_path(nodes: &[WorkflowStepNode], parent_path: &[usize]) -> usize {
    let Some((first, rest)) = parent_path.split_first() else {
        return nodes.len();
    };
    let Some(node) = nodes.get(*first) else {
        return 0;
    };
    children_len_at_path(&node.children, rest)
}

/// 将 step 节点插入到指定路径。
fn push_step_node_at_path(
    nodes: &mut Vec<WorkflowStepNode>,
    path: &[usize],
    node: WorkflowStepNode,
) {
    if path.len() == 1 {
        nodes.push(node);
        return;
    }
    let Some((first, rest)) = path.split_first() else {
        return;
    };
    if let Some(parent) = nodes.get_mut(*first) {
        push_step_node_at_path(&mut parent.children, rest, node);
    }
}

/// 解析 task 文档中的扁平 step 记录。
fn parse_step_records(content: &str) -> Vec<StepRecord> {
    let lines = markdown_lines(content);
    let section = find_section(&lines, STEPS_MARKER).unwrap_or(Section {
        content_start: 0,
        content_end: 0,
        marker_line: None,
        end_line: 0,
    });
    let step_lines = collect_step_lines(&lines, &section);
    step_lines
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let desc_end = step_lines
                .get(index + 1)
                .map(|next| next.line_index)
                .unwrap_or(section.content_end);
            let flat_path = step_path_for_index(&step_lines, index);
            let root_doc_key = flat_path
                .first()
                .and_then(|root_index| step_lines.get(*root_index))
                .map(|root| root.title.clone())
                .unwrap_or_else(|| step.title.clone());
            StepRecord {
                path: flat_path
                    .iter()
                    .enumerate()
                    .map(|(depth, _)| sibling_index_for_flat_path(&step_lines, &flat_path, depth))
                    .collect(),
                line_index: step.line_index,
                indent: step.indent,
                title: step.title.clone(),
                checked: step.checked,
                checkable: step.checkable,
                desc_start: step.line_index + 1,
                desc_end,
                desc: unindent_lines(&lines[step.line_index + 1..desc_end], step.indent + 2),
                root_doc_key,
            }
        })
        .collect()
}

/// 收集 steps 区块中的 step 行。
fn collect_step_lines(lines: &[String], section: &Section) -> Vec<StepLine> {
    let mut steps = Vec::new();
    for (offset, line) in lines[section.content_start..section.content_end]
        .iter()
        .enumerate()
    {
        if let Some(parsed) = parse_step_line(line) {
            steps.push(StepLine {
                line_index: section.content_start + offset,
                indent: parsed.indent,
                title: parsed.title,
                checked: parsed.checked,
                checkable: parsed.checkable,
            });
        }
    }
    steps
}

/// 计算 step flat index 的祖先 flat index 路径。
fn step_path_for_index(steps: &[StepLine], index: usize) -> Vec<usize> {
    let mut path = Vec::new();
    let mut current_indent = steps[index].indent;
    path.push(index);
    for previous in (0..index).rev() {
        if steps[previous].indent < current_indent {
            path.push(previous);
            current_indent = steps[previous].indent;
        }
    }
    path.reverse();
    path
}

/// 将 flat path 的指定深度转换成同级索引。
fn sibling_index_for_flat_path(steps: &[StepLine], flat_path: &[usize], depth: usize) -> usize {
    let flat_index = flat_path[depth];
    let parent_flat_path = &flat_path[..depth];
    (0..flat_index)
        .filter(|candidate| {
            let candidate_path = step_path_for_index(steps, *candidate);
            candidate_path.len() == depth + 1
                && &candidate_path[..depth] == parent_flat_path
                && steps[*candidate].indent == steps[flat_index].indent
        })
        .count()
}

/// 解析单行 step。
fn parse_step_line(line: &str) -> Option<ParsedStepLine> {
    let indent = leading_spaces(line);
    let trimmed = line.get(indent..)?;
    let rest = trimmed.strip_prefix("- ")?;
    let (checkable, checked, title) = if let Some(title) = rest.strip_prefix("[ ] ") {
        (true, false, title)
    } else if let Some(title) = rest.strip_prefix("[x] ") {
        (true, true, title)
    } else if let Some(title) = rest.strip_prefix("[X] ") {
        (true, true, title)
    } else {
        (false, false, rest)
    };
    let title = title.trim().to_string();
    (!title.is_empty()).then_some(ParsedStepLine {
        indent,
        title,
        checked,
        checkable,
    })
}

/// 替换指定 step 的 desc 行。
fn replace_step_desc(
    lines: &mut Vec<String>,
    step_path: &[usize],
    next_text: &str,
) -> Result<(), String> {
    let content = join_markdown_lines(lines);
    let records = parse_step_records(&content);
    let Some(record) = records.iter().find(|record| record.path == step_path) else {
        return Err("step not found".to_string());
    };
    let replacement = indent_text_lines(next_text, record.indent + 2);
    lines.splice(record.desc_start..record.desc_end, replacement);
    Ok(())
}

/// 替换、创建或删除 doc desc。
fn replace_doc_desc(lines: &mut Vec<String>, key: &str, next_text: &str) {
    let section = ensure_doc_section(lines);
    let docs = parse_doc_items_from_lines(lines, &section);
    if let Some(doc) = docs.iter().find(|doc| doc.key == key) {
        if next_text.trim().is_empty() {
            lines.splice(doc.item_start..doc.item_end, Vec::<String>::new());
        } else {
            let replacement = indent_text_lines(next_text, 2);
            lines.splice(doc.desc_start..doc.desc_end, replacement);
        }
        return;
    }
    if next_text.trim().is_empty() {
        return;
    }
    let mut replacement = vec![format!("- {key}")];
    replacement.extend(indent_text_lines(next_text, 2));
    lines.splice(section.content_end..section.content_end, replacement);
}

/// 删除已有 doc 项；不存在时不创建 doc 区块。
fn delete_doc_item_if_exists(lines: &mut Vec<String>, key: &str) {
    let Some(section) = find_section(lines, DOC_MARKER) else {
        return;
    };
    let docs = parse_doc_items_from_lines(lines, &section);
    if let Some(doc) = docs.iter().find(|doc| doc.key == key) {
        lines.splice(doc.item_start..doc.item_end, Vec::<String>::new());
    }
}

/// 重命名已有 doc key；不存在时不创建 doc 项。
fn rename_doc_item_key_if_exists(
    lines: &mut [String],
    old_key: &str,
    new_key: &str,
) -> Result<(), String> {
    let Some(section) = find_section(lines, DOC_MARKER) else {
        return Ok(());
    };
    let docs = parse_doc_items_from_lines(lines, &section);
    let Some(doc) = docs.iter().find(|doc| doc.key == old_key) else {
        return Ok(());
    };
    if docs.iter().any(|doc| doc.key == new_key) {
        return Err(format!("doc key already exists: {new_key}"));
    }
    lines[doc.item_start] = format!("- {new_key}");
    Ok(())
}

/// 判断一个 step 记录是否存在子 step。
fn step_has_children(records: &[StepRecord], index: usize) -> bool {
    records
        .get(index + 1)
        .is_some_and(|next| next.indent > records[index].indent)
}

/// 生成重命名后的 step 行，保留 checkbox 状态。
fn renamed_step_line(record: &StepRecord, new_key: &str) -> String {
    let prefix = " ".repeat(record.indent);
    if record.checkable {
        let checkbox = if record.checked { "[x]" } else { "[ ]" };
        format!("{prefix}- {checkbox} {new_key}")
    } else {
        format!("{prefix}- {new_key}")
    }
}

/// 从 task 路径提取不带 `task-` 前缀的 key。
fn task_key_from_path(task_path: &Path) -> Result<String, String> {
    let stem = task_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .ok_or_else(|| format!("task key not found: {}", task_path.display()))?;
    Ok(stem
        .strip_prefix(TASK_PREFIX)
        .map(str::to_string)
        .unwrap_or(stem))
}

/// 确保文档存在 steps 区块。
fn ensure_steps_section(lines: &mut Vec<String>) -> Section {
    if let Some(section) = find_section(lines, STEPS_MARKER) {
        return section;
    }
    let mut prefix = vec![
        STEPS_MARKER.to_string(),
        "---------".to_string(),
        String::new(),
    ];
    prefix.append(lines);
    *lines = prefix;
    Section {
        marker_line: Some(0),
        content_start: 1,
        content_end: 1,
        end_line: 1,
    }
}

/// 确保文档存在 doc 区块。
fn ensure_doc_section(lines: &mut Vec<String>) -> Section {
    if let Some(section) = find_section(lines, DOC_MARKER) {
        return section;
    }
    if !lines.last().is_none_or(|line| line.trim().is_empty()) {
        lines.push(String::new());
    }
    let marker_line = lines.len();
    lines.push(DOC_MARKER.to_string());
    let end_line = lines.len() + 1;
    lines.push("--------".to_string());
    Section {
        marker_line: Some(marker_line),
        content_start: marker_line + 1,
        content_end: marker_line + 1,
        end_line,
    }
}

/// 解析 task 文档中的 doc 项。
fn parse_doc_items(content: &str) -> Vec<DocItem> {
    let lines = markdown_lines(content);
    let Some(section) = find_section(&lines, DOC_MARKER) else {
        return Vec::new();
    };
    parse_doc_items_from_lines(&lines, &section)
}

/// 从已切分行中解析 doc 项。
fn parse_doc_items_from_lines(lines: &[String], section: &Section) -> Vec<DocItem> {
    let mut item_lines = Vec::new();
    for (offset, line) in lines[section.content_start..section.content_end]
        .iter()
        .enumerate()
    {
        if leading_spaces(line) == 0
            && let Some(key) = line.trim_start().strip_prefix("- ")
        {
            let key = key.trim().to_string();
            if !key.is_empty() {
                item_lines.push((section.content_start + offset, key));
            }
        }
    }
    let mut docs = Vec::new();
    for (index, (line_index, key)) in item_lines.iter().enumerate() {
        let item_end = item_lines
            .get(index + 1)
            .map(|(next_line, _)| *next_line)
            .unwrap_or(section.content_end);
        docs.push(DocItem {
            key: key.clone(),
            item_start: *line_index,
            item_end,
            desc_start: *line_index + 1,
            desc_end: item_end,
            desc: unindent_lines(&lines[*line_index + 1..item_end], 2),
        });
    }
    docs
}

/// 查找一个标记区块。
fn find_section(lines: &[String], marker: &str) -> Option<Section> {
    let marker_line = lines.iter().position(|line| line.trim() == marker)?;
    let content_start = marker_line + 1;
    let end_line = lines[content_start..]
        .iter()
        .position(|line| is_section_separator(line))
        .map(|offset| content_start + offset)
        .unwrap_or(lines.len());
    Some(Section {
        marker_line: Some(marker_line),
        content_start,
        content_end: end_line,
        end_line,
    })
}

/// 判断一行是否是区块分隔线。
fn is_section_separator(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.len() >= 3 && trimmed.chars().all(|character| character == '-')
}

/// 将 Markdown 文本切成不带换行符的行。
fn markdown_lines(content: &str) -> Vec<String> {
    content.lines().map(str::to_string).collect()
}

/// 将行重新合并为 Markdown 文本。
fn join_markdown_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

/// 移除一组 desc 行的公共语义缩进。
fn unindent_lines(lines: &[String], indent: usize) -> String {
    lines
        .iter()
        .map(|line| strip_indent(line, indent))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end_matches('\n')
        .to_string()
}

/// 给用户输入的 desc 行补回 Markdown 缩进。
fn indent_text_lines(text: &str, indent: usize) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let prefix = " ".repeat(indent);
    text.lines()
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("{prefix}{line}")
            }
        })
        .collect()
}

/// 移除指定数量的前导空格。
fn strip_indent(line: &str, indent: usize) -> String {
    let mut remaining = indent;
    let mut byte_index = 0;
    for (index, character) in line.char_indices() {
        if remaining == 0 || character != ' ' {
            byte_index = index;
            break;
        }
        remaining -= 1;
        byte_index = index + character.len_utf8();
    }
    if remaining > 0 && byte_index >= line.len() {
        String::new()
    } else {
        line.get(byte_index..).unwrap_or_default().to_string()
    }
}

/// 统计行首空格数量。
fn leading_spaces(line: &str) -> usize {
    line.chars()
        .take_while(|character| *character == ' ')
        .count()
}

/// 一个 Markdown 区块的位置。
#[derive(Debug, Clone)]
struct Section {
    /// marker 所在行。
    marker_line: Option<usize>,
    /// 内容起始行。
    content_start: usize,
    /// 内容结束行，排除分隔线。
    content_end: usize,
    /// 分隔线所在行或文档末尾。
    end_line: usize,
}

/// 解析后的 step 行。
#[derive(Debug, Clone)]
struct StepLine {
    /// step 行号。
    line_index: usize,
    /// step 行缩进。
    indent: usize,
    /// step 标题。
    title: String,
    /// step 是否完成。
    checked: bool,
    /// step 是否带 checkbox。
    checkable: bool,
}

/// 带 range 的 step 记录。
#[derive(Debug, Clone)]
struct StepRecord {
    /// step 在同级列表中的索引路径。
    path: Vec<usize>,
    /// step 行号。
    line_index: usize,
    /// step 行缩进。
    indent: usize,
    /// step 标题。
    title: String,
    /// step 是否完成。
    checked: bool,
    /// step 是否带 checkbox。
    checkable: bool,
    /// desc 起始行。
    desc_start: usize,
    /// desc 结束行。
    desc_end: usize,
    /// 反缩进后的 desc。
    desc: String,
    /// 根父 step 对应的 doc key。
    root_doc_key: String,
}

/// 单行 step 的解析结果。
struct ParsedStepLine {
    /// step 行缩进。
    indent: usize,
    /// step 标题。
    title: String,
    /// step 是否完成。
    checked: bool,
    /// step 是否带 checkbox。
    checkable: bool,
}

/// doc 项及其文本范围。
#[derive(Debug, Clone)]
struct DocItem {
    /// doc key。
    key: String,
    /// doc 项起始行，包含 key 行。
    item_start: usize,
    /// doc 项结束行。
    item_end: usize,
    /// desc 起始行。
    desc_start: usize,
    /// desc 结束行。
    desc_end: usize,
    /// 反缩进后的 desc。
    desc: String,
}

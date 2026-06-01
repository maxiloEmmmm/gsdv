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

/// 判断路径是否属于 workspace 的 workflow 规范目录。
pub(super) fn path_is_workflow_spec_path(workspace_root: &Path, path: &Path) -> bool {
    let spec_root = workspace_root.join(GSDV_SPEC_DIR);
    path == spec_root || path.starts_with(spec_root)
}

/// 判断加载错误是否表示 workflow 根文件还没初始化。
pub(super) fn workflow_root_missing_error(workspace_root: &Path, error: &str) -> bool {
    error == format!("{} not found", workflow_root_path(workspace_root).display())
}

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
    /// 首个合法 step heading 之前的 task 说明。
    pub desc: String,
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

/// workflow task 说明编辑状态。
#[derive(Debug, Clone)]
pub(super) struct WorkflowTaskEditor {
    /// task 文档相对 workspace 的路径。
    pub task_path: PathBuf,
    /// task 说明当前文本。
    pub task_text: String,
    /// task 说明已保存文本。
    pub saved_task_text: String,
    /// 最近一次保存错误。
    pub save_error: Option<String>,
}

impl WorkflowTaskEditor {
    /// 判断 task 说明是否有未保存修改。
    pub(super) fn is_dirty(&self) -> bool {
        self.task_text != self.saved_task_text
    }
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
        self.step_text != self.saved_step_text
    }
}

/// workflow 片段保存请求。
#[derive(Debug, Clone)]
pub(super) struct WorkflowSaveRequest {
    /// task 文档相对 workspace 的路径。
    pub task_path: PathBuf,
    /// 需要写入的 task 说明。
    pub task_text: String,
    /// step 在 task 文档中的索引路径。
    pub step_path: Option<Vec<usize>>,
    /// 需要写入的右侧 step desc。
    pub step_text: Option<String>,
}

/// workflow 片段保存成功结果。
#[derive(Debug, Clone)]
pub(super) struct WorkflowSaveSuccess {
    /// 保存后的 task 说明。
    pub task_text: String,
    /// 保存后的右侧 step desc。
    pub step_text: Option<String>,
}

/// workflow tree 右键菜单触发的文件修改请求。
#[derive(Debug, Clone)]
pub(super) enum WorkflowMutationRequest {
    /// 初始化 workspace 级 workflow 根文件。
    InitRoot,
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

/// 从已解析的 task 节点创建 task 说明编辑器状态。
pub(super) fn workflow_task_editor_from_node(node: &WorkflowTaskNode) -> WorkflowTaskEditor {
    WorkflowTaskEditor {
        task_path: node.path.clone(),
        task_text: node.desc.clone(),
        saved_task_text: node.desc.clone(),
        save_error: None,
    }
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
        step_text: node.desc.clone(),
        saved_step_text: node.desc.clone(),
        save_error: None,
    }
}

/// 从 workspace 根目录加载 workflow tree。
pub(super) fn load_workflow_tree(workspace_root: &Path) -> Result<WorkflowTree, String> {
    let spec_root = workspace_root.join(GSDV_SPEC_DIR);
    let root_md = workflow_root_path(workspace_root);
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
            desc: parse_task_desc(&content),
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
    let target = WorkflowSelectionTarget::Step {
        task_path: task_path.to_path_buf(),
        step_path: step_path.to_vec(),
    };
    Ok(WorkflowStepEditor {
        target,
        task_path: task_path.to_path_buf(),
        step_path: step_path.to_vec(),
        step_title: record.title.clone(),
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
    validate_workflow_task_desc(&request.task_text)?;
    if let Some(step_text) = request.step_text.as_deref() {
        validate_workflow_step_desc(step_text)?;
    }
    let absolute = workspace_root.join(&request.task_path);
    let content = fs::read_to_string(&absolute)
        .map_err(|error| format!("failed to read {}: {error}", absolute.display()))?;
    let mut lines = markdown_lines(&content);
    replace_task_desc(&mut lines, &request.task_text);
    if let (Some(step_path), Some(step_text)) = (&request.step_path, request.step_text.as_deref()) {
        replace_step_desc(&mut lines, step_path, step_text)?;
    }
    let next_content = join_markdown_lines(&lines);
    fs::write(&absolute, next_content.as_bytes())
        .map_err(|error| format!("failed to write {}: {error}", absolute.display()))?;
    Ok(WorkflowSaveSuccess {
        task_text: request.task_text,
        step_text: request.step_text,
    })
}

/// 校验 task 说明，避免说明伪装成新的 step heading。
fn validate_workflow_task_desc(text: &str) -> Result<(), String> {
    if text.lines().any(|line| parse_step_line(line).is_some()) {
        return Err(
            "Task description cannot contain lines starting with `## [ ]` or `## [x]`".to_string(),
        );
    }
    Ok(())
}

/// 校验 step 正文，避免正文伪装成新的 step heading。
fn validate_workflow_step_desc(text: &str) -> Result<(), String> {
    if text.lines().any(|line| parse_step_line(line).is_some()) {
        return Err(
            "Step description cannot contain lines starting with `## [ ]` or `## [x]`".to_string(),
        );
    }
    Ok(())
}

/// 应用 workflow tree 右键菜单触发的文件修改。
pub(super) fn apply_workflow_mutation(
    workspace_root: &Path,
    request: WorkflowMutationRequest,
) -> Result<(), String> {
    match request {
        WorkflowMutationRequest::InitRoot => init_workflow_root(workspace_root),
        WorkflowMutationRequest::AddTask {
            project_key,
            task_key,
        } => add_workflow_task(workspace_root, &project_key, &task_key),
        WorkflowMutationRequest::AddStep {
            task_path,
            key,
            desc,
        } => add_workflow_step(workspace_root, &task_path, &key, &desc),
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

/// 校验 workflow step 标题是否能作为单行 Markdown heading。
pub(super) fn validate_workflow_step_title(title: &str) -> Result<&str, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("Step title is required".to_string());
    }
    if title.contains('\n') || title.contains('\r') {
        return Err("Step title must be one line".to_string());
    }
    Ok(title)
}

/// 初始化 workspace 级 workflow 根文件。
fn init_workflow_root(workspace_root: &Path) -> Result<(), String> {
    let root_path = workflow_root_path(workspace_root);
    if root_path.is_file() {
        return Ok(());
    }
    if root_path.exists() {
        return Err(format!(
            "workflow root is not a file: {}",
            root_path.display()
        ));
    }
    let Some(parent) = root_path.parent() else {
        return Err(format!(
            "invalid workflow root path: {}",
            root_path.display()
        ));
    };
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    fs::write(&root_path, [])
        .map_err(|error| format!("failed to write {}: {error}", root_path.display()))
}

/// 返回 workspace 级 workflow 根文件路径。
fn workflow_root_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(GSDV_SPEC_DIR).join(ROOT_MD)
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
    fs::write(&task_path, [])
        .map_err(|error| format!("failed to write {}: {error}", task_path.display()))
}

/// 在 task 文档中新增扁平 step。
fn add_workflow_step(
    workspace_root: &Path,
    task_path: &Path,
    key: &str,
    desc: &str,
) -> Result<(), String> {
    let key = validate_workflow_step_title(key)?;
    let absolute = workspace_root.join(task_path);
    let content = fs::read_to_string(&absolute)
        .map_err(|error| format!("failed to read {}: {error}", absolute.display()))?;
    let mut lines = markdown_lines(&content);
    if parse_step_records(&content)
        .iter()
        .any(|record| record.title == key)
    {
        return Err(format!("step already exists at this level: {key}"));
    }
    if !lines.is_empty() && lines.last().is_some_and(|line| !line.trim().is_empty()) {
        lines.push(String::new());
    }
    lines.push(format!("## [ ] {key}"));
    if !desc.trim().is_empty() {
        lines.extend(desc.lines().map(str::to_string));
    }
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
    let new_key = validate_workflow_step_title(new_key)?;
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
    if records
        .iter()
        .any(|candidate| candidate.path != record.path && candidate.title == new_key)
    {
        return Err(format!("step already exists at this level: {new_key}"));
    }
    lines[record.line_index] = renamed_step_line(&record, new_key);
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
    let records = parse_step_records(&content);
    let index = records
        .iter()
        .position(|record| record.path == step_path)
        .ok_or_else(|| "step not found".to_string())?;
    let record = records[index].clone();
    let delete_end = records
        .get(index + 1)
        .map(|record| record.line_index)
        .unwrap_or(lines.len());
    lines.splice(record.line_index..delete_end, Vec::<String>::new());
    let next_content = join_markdown_lines(&lines);
    fs::write(&absolute, next_content.as_bytes())
        .map_err(|error| format!("failed to write {}: {error}", absolute.display()))
}

/// 解析 task 文档首个 step 前的 task 说明。
fn parse_task_desc(content: &str) -> String {
    let lines = markdown_lines(content);
    let desc_end = lines
        .iter()
        .position(|line| parse_step_line(line).is_some())
        .unwrap_or(lines.len());
    lines[..desc_end].join("\n")
}

/// 解析 task 文档中的 step 树。
fn parse_task_steps(content: &str) -> Vec<WorkflowStepNode> {
    let records = parse_step_records(content);
    records
        .into_iter()
        .map(|record| WorkflowStepNode {
            path: record.path,
            title: record.title,
            checked: record.checked,
            checkable: record.checkable,
            desc: record.desc,
            children: Vec::new(),
        })
        .collect()
}

/// 解析 task 文档中的扁平 step 记录。
fn parse_step_records(content: &str) -> Vec<StepRecord> {
    let lines = markdown_lines(content);
    let step_lines = collect_step_lines(&lines);
    step_lines
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let desc_end = step_lines
                .get(index + 1)
                .map(|next| next.line_index)
                .unwrap_or(lines.len());
            StepRecord {
                path: vec![index],
                line_index: step.line_index,
                title: step.title.clone(),
                checked: step.checked,
                checkable: step.checkable,
                desc_start: step.line_index + 1,
                desc_end,
                desc: lines[step.line_index + 1..desc_end]
                    .join("\n")
                    .trim_end_matches('\n')
                    .to_string(),
            }
        })
        .collect()
}

/// 收集 task 文档中的二级 checkbox step 标题行。
fn collect_step_lines(lines: &[String]) -> Vec<StepLine> {
    let mut steps = Vec::new();
    for (line_index, line) in lines.iter().enumerate() {
        if let Some(parsed) = parse_step_line(line) {
            steps.push(StepLine {
                line_index,
                title: parsed.title,
                checked: parsed.checked,
                checkable: parsed.checkable,
            });
        }
    }
    steps
}

/// 解析单行 Markdown 二级 checkbox step 标题。
fn parse_step_line(line: &str) -> Option<ParsedStepLine> {
    let rest = line.strip_prefix("## ")?;
    let (checked, title) = if let Some(title) = rest.strip_prefix("[ ] ") {
        (false, title)
    } else if let Some(title) = rest.strip_prefix("[x] ") {
        (true, title)
    } else if let Some(title) = rest.strip_prefix("[X] ") {
        (true, title)
    } else {
        return None;
    };
    let title = title.trim().to_string();
    (!title.is_empty()).then_some(ParsedStepLine {
        title,
        checked,
        checkable: true,
    })
}

/// 替换首个合法 step heading 之前的 task 说明。
fn replace_task_desc(lines: &mut Vec<String>, next_text: &str) {
    let desc_end = lines
        .iter()
        .position(|line| parse_step_line(line).is_some())
        .unwrap_or(lines.len());
    let replacement = editor_text_lines(next_text);
    lines.splice(0..desc_end, replacement);
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
    let replacement = editor_text_lines(next_text);
    lines.splice(record.desc_start..record.desc_end, replacement);
    Ok(())
}

/// 生成重命名后的 step 行，保留 checkbox 状态。
fn renamed_step_line(record: &StepRecord, new_key: &str) -> String {
    let checkbox = if record.checked { "[x]" } else { "[ ]" };
    format!("## {checkbox} {new_key}")
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

/// 将 Markdown 文本切成不带换行符的行。
fn markdown_lines(content: &str) -> Vec<String> {
    content.lines().map(str::to_string).collect()
}

/// 将 editor 文本切成行，保留用户在末尾输入的空行。
fn editor_text_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').map(str::to_string).collect()
    }
}

/// 将行重新合并为 Markdown 文本。
fn join_markdown_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

/// 解析后的 step 行。
#[derive(Debug, Clone)]
struct StepLine {
    /// step 行号。
    line_index: usize,
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
}

/// 单行 step 的解析结果。
struct ParsedStepLine {
    /// step 标题。
    title: String,
    /// step 是否完成。
    checked: bool,
    /// step 是否带 checkbox。
    checkable: bool,
}

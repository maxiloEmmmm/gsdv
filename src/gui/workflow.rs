//! gsdv-spec workflow 文档解析和片段写回。
//!
//! 本模块只处理文件内容和结构化数据，不直接修改 egui 渲染状态。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

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
    let Ok(relative) = path.strip_prefix(workspace_root) else {
        return false;
    };
    relative
        .components()
        .any(|component| matches!(component, Component::Normal(value) if value == GSDV_SPEC_DIR))
}

/// 判断加载错误是否表示 workflow 根文件还没初始化。
pub(super) fn workflow_root_missing_error(workspace_root: &Path, error: &str) -> bool {
    error == format!("{} not found", workflow_root_path(workspace_root).display())
}

/// workflow 树加载结果。
#[derive(Debug, Clone, Default)]
pub(super) struct WorkflowTree {
    /// 当前 workflow 所属 repo 相对 workspace 的路径；主 repo 为空。
    pub repo_path: PathBuf,
    /// workspace 内相对的 gsdv-spec 目录。
    pub spec_path: PathBuf,
    /// workspace 级 workflow root.md 相对 workspace 的路径。
    pub root_path: PathBuf,
    /// 当前 workspace 的 workflow 项目列表。
    pub projects: Vec<WorkflowProjectNode>,
    /// workspace 内手动或首次扫描发现的子 repo workflow。
    pub sub_workflows: Vec<WorkflowTree>,
}

/// 遍历主 workflow 与已加载的子 repo workflow。
///
/// 适用场景：选择、校验和渲染需要统一查找所有 tree。
/// 例：`主树 + 2 个子树 -> 3 个元素`。
pub(super) fn workflow_trees(tree: &WorkflowTree) -> impl Iterator<Item = &WorkflowTree> {
    std::iter::once(tree).chain(tree.sub_workflows.iter())
}

/// 遍历主 workflow 与子 repo 中的全部项目。
///
/// 适用场景：按 root/task 路径定位所属项目。
/// 例：`2 棵树各 1 项目 -> 2 个项目`。
pub(super) fn workflow_projects(tree: &WorkflowTree) -> impl Iterator<Item = &WorkflowProjectNode> {
    workflow_trees(tree).flat_map(|tree| tree.projects.iter())
}

/// 遍历主 workflow 与子 repo 中的全部 task。
///
/// 适用场景：编辑器按 workspace 相对 task 路径恢复节点。
/// 例：`所有项目 -> 所有直接 task`。
pub(super) fn workflow_tasks(tree: &WorkflowTree) -> impl Iterator<Item = &WorkflowTaskNode> {
    workflow_projects(tree).flat_map(|project| project.tasks.iter())
}

/// 返回跨 repo 唯一的 project 折叠状态 key。
///
/// 适用场景：不同 repo 可以拥有同名 project。
/// 例：`services/api + main -> services/api/main`。
pub(super) fn workflow_project_state_key(tree: &WorkflowTree, project_key: &str) -> String {
    tree.repo_path
        .join(project_key)
        .to_string_lossy()
        .to_string()
}

/// 从 project 目录相对路径生成折叠状态 key。
///
/// 适用场景：project 重命名后的状态迁移。例：`repo/gsdv-spec/ps/main -> repo/main`。
pub(super) fn workflow_project_state_key_from_path(project_path: &Path) -> Option<String> {
    let project_key = project_path.file_name()?.to_string_lossy();
    let repo_path = project_path.parent()?.parent()?.parent()?;
    Some(
        repo_path
            .join(project_key.as_ref())
            .to_string_lossy()
            .to_string(),
    )
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
    /// workspace 级 root.md。
    WorkspaceRoot { root_path: PathBuf },
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
    /// 在 workflow 规范目录下创建一个项目 root.md。
    AddProject {
        /// 目标 repo 的 gsdv-spec 相对 workspace 的路径。
        spec_path: PathBuf,
        /// 项目目录名。
        project_key: String,
    },
    /// 在项目目录下创建一个空 task Markdown 文件。
    AddTask {
        /// 目标 repo 的 gsdv-spec 相对 workspace 的路径。
        spec_path: PathBuf,
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
        /// 原项目目录相对 workspace 的路径。
        project_path: PathBuf,
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
        /// 项目目录相对 workspace 的路径。
        project_path: PathBuf,
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
    /// 合并同一个 task 中连续选中的 step。
    MergeSteps {
        /// task 文档相对 workspace 的路径。
        task_path: PathBuf,
        /// 要合并的 step 路径列表。
        step_paths: Vec<Vec<usize>>,
        /// 合并后新 step 的标题。
        title: String,
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
    load_workflow_tree_at_repo(workspace_root, Path::new(""))
}

/// 加载主 workflow，并按需扫描或复用已发现的子 repo。
///
/// 适用场景：首次打开传 `scan_sub_workflows=true`，文件刷新复用缓存路径。
/// 例：`known=[services/api] -> 主树 + services/api 子树`。
pub(super) fn load_workflow_tree_with_sub_workflows(
    workspace_root: &Path,
    scan_sub_workflows: bool,
    known_repo_paths: &[PathBuf],
) -> Result<WorkflowTree, String> {
    let mut tree = load_workflow_tree(workspace_root)?;
    let repo_paths = if scan_sub_workflows {
        discover_nested_git_repo_paths(workspace_root)?
    } else {
        known_repo_paths.to_vec()
    };
    for repo_path in repo_paths {
        let root_path = workspace_root
            .join(&repo_path)
            .join(GSDV_SPEC_DIR)
            .join(ROOT_MD);
        if !root_path.is_file() {
            continue;
        }
        tree.sub_workflows
            .push(load_workflow_tree_at_repo(workspace_root, &repo_path)?);
    }
    tree.sub_workflows
        .sort_by(|left, right| left.repo_path.cmp(&right.repo_path));
    Ok(tree)
}

/// 从 workspace 内指定 repo 根加载一棵 workflow tree。
///
/// 适用场景：主 repo 使用空路径，子 repo 使用 workspace 相对路径。
/// 例：`services/api -> services/api/gsdv-spec/root.md`。
fn load_workflow_tree_at_repo(
    workspace_root: &Path,
    repo_path: &Path,
) -> Result<WorkflowTree, String> {
    let repo_root = workspace_root.join(repo_path);
    let spec_root = repo_root.join(GSDV_SPEC_DIR);
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
        repo_path: repo_path.to_path_buf(),
        spec_path: relative_to_workspace(workspace_root, &spec_root),
        root_path: relative_to_workspace(workspace_root, &root_md),
        projects,
        sub_workflows: Vec::new(),
    })
}

/// 扫描 workspace 下最近一层嵌套 Git repo 的 workflow 路径。
///
/// 触发条件：首次打开 workflow 或用户手动扫描。
/// 不能在命中 `.git` 后继续递归：一个 repo 可能包含大量依赖目录。
/// 防止回归：嵌套 repo 内部再次扫描造成重复树和额外 IO。
fn discover_nested_git_repo_paths(workspace_root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut repo_paths = Vec::new();
    discover_nested_git_repo_paths_in(workspace_root, Path::new(""), &mut repo_paths)?;
    repo_paths.sort();
    repo_paths.dedup();
    Ok(repo_paths)
}

/// 递归扫描尚未命中 Git 边界的目录。
///
/// 适用场景：workspace 根自身的 `.git` 不参与判断，只检查其子目录。
/// 例：`services/api/.git -> services/api`，并停止进入 `services/api`。
fn discover_nested_git_repo_paths_in(
    workspace_root: &Path,
    relative_dir: &Path,
    repo_paths: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let absolute_dir = workspace_root.join(relative_dir);
    let entries = fs::read_dir(&absolute_dir)
        .map_err(|error| format!("failed to read {}: {error}", absolute_dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let child_relative = relative_dir.join(&name);
        let child_absolute = entry.path();
        if fs::symlink_metadata(child_absolute.join(".git")).is_ok() {
            if child_absolute.join(GSDV_SPEC_DIR).join(ROOT_MD).is_file() {
                repo_paths.push(child_relative);
            }
            continue;
        }
        discover_nested_git_repo_paths_in(workspace_root, &child_relative, repo_paths)?;
    }
    Ok(())
}

/// 从已读取的 workflow 文档集合构建 tree。
///
/// 适用场景：Remote Workspace 通过 SSH 读取文件后复用本地解析规则。
/// 例：`gsdv-spec/root.md + task-a.md -> WorkflowTree`。
pub(super) fn load_workflow_tree_from_documents(
    workspace_root: &Path,
    documents: &BTreeMap<PathBuf, String>,
) -> Result<WorkflowTree, String> {
    let mut tree =
        load_workflow_tree_from_documents_at_repo(workspace_root, documents, Path::new(""))?;
    let mut repo_paths = documents
        .keys()
        .filter_map(|path| workflow_document_repo_path(path))
        .filter(|path| !path.as_os_str().is_empty())
        .collect::<Vec<_>>();
    repo_paths.sort();
    repo_paths.dedup();
    for repo_path in repo_paths {
        tree.sub_workflows
            .push(load_workflow_tree_from_documents_at_repo(
                workspace_root,
                documents,
                &repo_path,
            )?);
    }
    Ok(tree)
}

/// 从远端文档集合加载指定 repo 的 workflow tree。
///
/// 适用场景：一份 SSH inventory 同时携带主 repo 和多个子 repo 文档。
/// 例：`services/api + documents -> 子 WorkflowTree`。
fn load_workflow_tree_from_documents_at_repo(
    workspace_root: &Path,
    documents: &BTreeMap<PathBuf, String>,
    repo_path: &Path,
) -> Result<WorkflowTree, String> {
    let spec_path = repo_path.join(GSDV_SPEC_DIR);
    let root_path = spec_path.join(ROOT_MD);
    if !documents.contains_key(&root_path) {
        return Err(format!(
            "{} not found",
            workspace_root.join(&root_path).display()
        ));
    }

    let projects_root = spec_path.join(PROJECTS_DIR);
    let mut project_keys = documents
        .keys()
        .filter_map(|path| {
            let project_dir = path.parent()?;
            if path.file_name()? != ROOT_MD || project_dir.parent()? != projects_root {
                return None;
            }
            Some(project_dir.file_name()?.to_string_lossy().to_string())
        })
        .collect::<Vec<_>>();
    project_keys.sort();
    project_keys.dedup();

    let projects = project_keys
        .into_iter()
        .map(|project_key| {
            let project_dir = projects_root.join(&project_key);
            let mut task_paths = documents
                .keys()
                .filter(|path| path.parent() == Some(project_dir.as_path()))
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with(TASK_PREFIX))
                        && path
                            .extension()
                            .is_some_and(|extension| extension == MARKDOWN_EXT)
                })
                .cloned()
                .collect::<Vec<_>>();
            task_paths.sort_by(|left, right| file_name_string(left).cmp(&file_name_string(right)));
            let tasks = task_paths
                .into_iter()
                .filter_map(|path| {
                    let content = documents.get(&path)?;
                    Some(WorkflowTaskNode {
                        label: workflow_task_label_for_path(&path),
                        path,
                        desc: parse_task_desc(content),
                        steps: parse_task_steps(content),
                    })
                })
                .collect();
            WorkflowProjectNode {
                key: project_key.clone(),
                label: project_key,
                root_path: project_dir.join(ROOT_MD),
                tasks,
            }
        })
        .collect();

    Ok(WorkflowTree {
        repo_path: repo_path.to_path_buf(),
        spec_path,
        root_path,
        projects,
        sub_workflows: Vec::new(),
    })
}

/// 从一条远端 inventory 文档路径识别其 repo 相对路径。
///
/// 适用场景：`services/api/gsdv-spec/root.md -> services/api`。
/// 例：主 `gsdv-spec/root.md -> 空路径`。
fn workflow_document_repo_path(path: &Path) -> Option<PathBuf> {
    if path.file_name()? != ROOT_MD {
        return None;
    }
    let spec_path = path.parent()?;
    if spec_path.file_name()? != GSDV_SPEC_DIR {
        return None;
    }
    Some(
        spec_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default(),
    )
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
        let label = workflow_task_label_for_path(&path);
        tasks.push(WorkflowTaskNode {
            label,
            path: relative_to_workspace(workspace_root, &path),
            desc: parse_task_desc(&content),
            steps,
        });
    }
    Ok(tasks)
}

/// 返回 task tree 的展示名，保留文件命名规范但隐藏 task- 前缀。
fn workflow_task_label_for_path(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| "task".to_string());
    stem.strip_prefix(TASK_PREFIX)
        .map(str::to_string)
        .unwrap_or(stem)
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
    let absolute = workspace_root.join(&request.task_path);
    let content = fs::read_to_string(&absolute)
        .map_err(|error| format!("failed to read {}: {error}", absolute.display()))?;
    let (next_content, saved) = save_workflow_step_content(&content, &request)?;
    fs::write(&absolute, next_content.as_bytes())
        .map_err(|error| format!("failed to write {}: {error}", absolute.display()))?;
    Ok(saved)
}

/// 在内存中的 task Markdown 上应用片段保存。
///
/// 适用场景：本地与 Remote 存储层在写盘前共享同一套内容规则。
/// 例：`旧 task + 新 step desc -> (新 task, 保存结果)`。
pub(super) fn save_workflow_step_content(
    content: &str,
    request: &WorkflowSaveRequest,
) -> Result<(String, WorkflowSaveSuccess), String> {
    validate_workflow_task_desc(&request.task_text)?;
    if let Some(step_text) = request.step_text.as_deref() {
        validate_workflow_step_desc(step_text)?;
    }
    let mut lines = markdown_lines(&content);
    replace_task_desc(&mut lines, &request.task_text);
    if let (Some(step_path), Some(step_text)) = (&request.step_path, request.step_text.as_deref()) {
        replace_step_desc(&mut lines, step_path, step_text)?;
    }
    let next_content = join_markdown_lines(&lines);
    let saved = WorkflowSaveSuccess {
        task_text: request.task_text.clone(),
        step_text: request.step_text.clone(),
    };
    Ok((next_content, saved))
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
        WorkflowMutationRequest::AddProject {
            spec_path,
            project_key,
        } => add_workflow_project(workspace_root, &spec_path, &project_key),
        WorkflowMutationRequest::AddTask {
            spec_path,
            project_key,
            task_key,
        } => add_workflow_task(workspace_root, &spec_path, &project_key, &task_key),
        WorkflowMutationRequest::AddStep {
            task_path,
            key,
            desc,
        } => add_workflow_step(workspace_root, &task_path, &key, &desc),
        WorkflowMutationRequest::RenameProject {
            project_path,
            new_key,
        } => rename_workflow_project(workspace_root, &project_path, &new_key),
        WorkflowMutationRequest::RenameTask { task_path, new_key } => {
            rename_workflow_task(workspace_root, &task_path, &new_key)
        }
        WorkflowMutationRequest::RenameStep {
            task_path,
            step_path,
            new_key,
        } => rename_workflow_step(workspace_root, &task_path, &step_path, &new_key),
        WorkflowMutationRequest::DeleteProject { project_path } => {
            delete_workflow_project(workspace_root, &project_path)
        }
        WorkflowMutationRequest::DeleteTask { task_path } => {
            delete_workflow_task(workspace_root, &task_path)
        }
        WorkflowMutationRequest::DeleteStep {
            task_path,
            step_path,
        } => delete_workflow_step(workspace_root, &task_path, &step_path),
        WorkflowMutationRequest::MergeSteps {
            task_path,
            step_paths,
            title,
        } => merge_workflow_steps(workspace_root, &task_path, &step_paths, &title),
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

/// 校验 workflow 路径是 workspace 内含 gsdv-spec 的普通相对路径。
///
/// 适用场景：本地和 Remote mutation 共用路径边界检查。
/// 例：`services/api/gsdv-spec/ps/main -> Ok`，`../x -> Err`。
pub(super) fn validate_workflow_relative_path(path: &Path) -> Result<(), String> {
    let mut contains_spec = false;
    let mut has_component = false;
    for component in path.components() {
        let Component::Normal(value) = component else {
            return Err(format!("workflow path is unsafe: {}", path.display()));
        };
        has_component = true;
        if value == GSDV_SPEC_DIR {
            contains_spec = true;
        }
        if value.to_string_lossy().contains('\\') {
            return Err(format!("workflow path is unsafe: {}", path.display()));
        }
    }
    if !has_component || !contains_spec {
        return Err(format!(
            "workflow path is outside gsdv-spec: {}",
            path.display()
        ));
    }
    Ok(())
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

/// 创建 workflow project 根文件。
fn add_workflow_project(
    workspace_root: &Path,
    spec_path: &Path,
    project_key: &str,
) -> Result<(), String> {
    validate_workflow_relative_path(spec_path)?;
    let project_key = validate_workflow_key(project_key)?;
    let project_dir = workspace_root
        .join(spec_path)
        .join(PROJECTS_DIR)
        .join(project_key);
    let root_path = project_dir.join(ROOT_MD);
    if root_path.exists() {
        return Err(format!("workflow project already exists: {project_key}"));
    }
    fs::create_dir_all(&project_dir)
        .map_err(|error| format!("failed to create {}: {error}", project_dir.display()))?;
    fs::write(&root_path, [])
        .map_err(|error| format!("failed to write {}: {error}", root_path.display()))
}

/// 在项目目录下创建空 task Markdown 文件。
fn add_workflow_task(
    workspace_root: &Path,
    spec_path: &Path,
    project_key: &str,
    task_key: &str,
) -> Result<(), String> {
    validate_workflow_relative_path(spec_path)?;
    let project_key = validate_workflow_key(project_key)?;
    let task_key = validate_workflow_key(task_key)?;
    let project_dir = workspace_root
        .join(spec_path)
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
    rewrite_workflow_task(workspace_root, task_path, |content| {
        add_workflow_step_content(content, key, desc)
    })
}

/// 向内存中的 task Markdown 追加一个 step。
///
/// 适用场景：Remote 保存前不能直接调用本地文件 API。
/// 例：`空 task + build -> ## [ ] build`。
fn add_workflow_step_content(content: &str, key: &str, desc: &str) -> Result<String, String> {
    let key = validate_workflow_step_title(key)?;
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
    Ok(join_markdown_lines(&lines))
}

/// 重命名 workflow project 目录。
fn rename_workflow_project(
    workspace_root: &Path,
    project_path: &Path,
    new_key: &str,
) -> Result<(), String> {
    validate_workflow_relative_path(project_path)?;
    let project_key = project_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            format!(
                "workflow project path is invalid: {}",
                project_path.display()
            )
        })?;
    let project_key = validate_workflow_key(project_key)?;
    let new_key = validate_workflow_key(new_key)?;
    if project_key == new_key {
        return Ok(());
    }
    let old_dir = workspace_root.join(project_path);
    let projects_dir = old_dir.parent().ok_or_else(|| {
        format!(
            "workflow project parent not found: {}",
            project_path.display()
        )
    })?;
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
    rewrite_workflow_task(workspace_root, task_path, |content| {
        rename_workflow_step_content(content, step_path, new_key)
    })
}

/// 在内存中的 task Markdown 里重命名 step。
///
/// 适用场景：本地与 Remote 共用标题冲突和索引校验。
/// 例：`build -> compile`。
fn rename_workflow_step_content(
    content: &str,
    step_path: &[usize],
    new_key: &str,
) -> Result<String, String> {
    let new_key = validate_workflow_step_title(new_key)?;
    let mut lines = markdown_lines(&content);
    let records = parse_step_records(&content);
    let index = records
        .iter()
        .position(|record| record.path == step_path)
        .ok_or_else(|| "step not found".to_string())?;
    let record = records[index].clone();
    if record.title == new_key {
        return Ok(content.to_string());
    }
    if records
        .iter()
        .any(|candidate| candidate.path != record.path && candidate.title == new_key)
    {
        return Err(format!("step already exists at this level: {new_key}"));
    }
    lines[record.line_index] = renamed_step_line(&record, new_key);
    Ok(join_markdown_lines(&lines))
}

/// 删除 workflow project 目录。
fn delete_workflow_project(workspace_root: &Path, project_path: &Path) -> Result<(), String> {
    validate_workflow_relative_path(project_path)?;
    let project_dir = workspace_root.join(project_path);
    if !project_dir.is_dir() {
        return Err(format!(
            "workflow project not found: {}",
            project_path.display()
        ));
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
    rewrite_workflow_task(workspace_root, task_path, |content| {
        delete_workflow_step_content(content, step_path)
    })
}

/// 从内存中的 task Markdown 删除一个 step 子树。
///
/// 适用场景：Remote 删除必须先在本机按现有索引规则生成新文档。
/// 例：`[0] -> 删除首个 step 块`。
fn delete_workflow_step_content(content: &str, step_path: &[usize]) -> Result<String, String> {
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
    Ok(join_markdown_lines(&lines))
}

/// 合并 task 文档里连续的多个 step 块。
fn merge_workflow_steps(
    workspace_root: &Path,
    task_path: &Path,
    step_paths: &[Vec<usize>],
    title: &str,
) -> Result<(), String> {
    rewrite_workflow_task(workspace_root, task_path, |content| {
        merge_workflow_steps_content(content, step_paths, title)
    })
}

/// 在内存中的 task Markdown 合并连续 step。
///
/// 适用场景：Remote 与本地必须生成完全一致的合并结果。
/// 例：`[a,b] + merged -> 单个 merged step`。
fn merge_workflow_steps_content(
    content: &str,
    step_paths: &[Vec<usize>],
    title: &str,
) -> Result<String, String> {
    let title = validate_workflow_step_title(title)?;
    if step_paths.len() < 2 {
        return Err("Select at least two steps to merge".to_string());
    }
    let mut lines = markdown_lines(&content);
    let records = parse_step_records(&content);
    let selected_indices = workflow_step_indices_for_merge(&records, step_paths)?;
    let first_index = *selected_indices
        .first()
        .ok_or_else(|| "Select at least two steps to merge".to_string())?;
    if records
        .iter()
        .enumerate()
        .any(|(index, record)| !selected_indices.contains(&index) && record.title == title)
    {
        return Err(format!("step already exists at this level: {title}"));
    }
    let selected_records = selected_indices
        .iter()
        .map(|index| records[*index].clone())
        .collect::<Vec<_>>();
    let replacement = merged_workflow_step_lines(selected_records, title);
    for index in selected_indices.iter().skip(1).rev() {
        let delete_end = records
            .get(*index + 1)
            .map(|record| record.line_index)
            .unwrap_or(lines.len());
        lines.splice(records[*index].line_index..delete_end, Vec::<String>::new());
    }
    let first_end = records
        .get(first_index + 1)
        .map(|record| record.line_index)
        .unwrap_or(lines.len());
    lines.splice(records[first_index].line_index..first_end, replacement);
    Ok(join_markdown_lines(&lines))
}

/// 对需要改写 task 内容的 workflow mutation 执行纯内存转换。
///
/// 适用场景：Remote 先 SSH 读取 task，再复用本地 mutation 语义。
/// 例：`RenameStep + task text -> 新 task text`。
pub(super) fn apply_workflow_task_content_mutation(
    content: &str,
    request: &WorkflowMutationRequest,
) -> Result<String, String> {
    match request {
        WorkflowMutationRequest::AddStep { key, desc, .. } => {
            add_workflow_step_content(content, key, desc)
        }
        WorkflowMutationRequest::RenameStep {
            step_path, new_key, ..
        } => rename_workflow_step_content(content, step_path, new_key),
        WorkflowMutationRequest::DeleteStep { step_path, .. } => {
            delete_workflow_step_content(content, step_path)
        }
        WorkflowMutationRequest::MergeSteps {
            step_paths, title, ..
        } => merge_workflow_steps_content(content, step_paths, title),
        _ => Err("workflow mutation does not rewrite a task document".to_string()),
    }
}

/// 读取、转换并覆盖一个本地 workflow task。
///
/// 适用场景：文件存储层复用纯内容转换函数。
/// 例：`task.md + rename transform -> 覆盖 task.md`。
fn rewrite_workflow_task(
    workspace_root: &Path,
    task_path: &Path,
    transform: impl FnOnce(&str) -> Result<String, String>,
) -> Result<(), String> {
    let absolute = workspace_root.join(task_path);
    let content = fs::read_to_string(&absolute)
        .map_err(|error| format!("failed to read {}: {error}", absolute.display()))?;
    let next_content = transform(&content)?;
    fs::write(&absolute, next_content.as_bytes())
        .map_err(|error| format!("failed to write {}: {error}", absolute.display()))
}

/// 将待合并 step 路径映射成文档中的连续索引。
fn workflow_step_indices_for_merge(
    records: &[StepRecord],
    step_paths: &[Vec<usize>],
) -> Result<Vec<usize>, String> {
    let mut indices = Vec::with_capacity(step_paths.len());
    for path in step_paths {
        let index = records
            .iter()
            .position(|record| record.path == *path)
            .ok_or_else(|| "step not found".to_string())?;
        if !indices.contains(&index) {
            indices.push(index);
        }
    }
    indices.sort_unstable();
    Ok(indices)
}

/// 生成合并后的 step 行，只顺序保留原 step 正文。
fn merged_workflow_step_lines(records: Vec<StepRecord>, title: &str) -> Vec<String> {
    let checked = records.iter().all(|record| record.checked);
    let mut lines = vec![renamed_step_line_with_checked(title, checked)];
    let mut appended_desc_count = 0usize;
    for record in records {
        let desc = record.desc.trim_end();
        if desc.is_empty() {
            continue;
        }
        if appended_desc_count > 0 && lines.last().is_some_and(|line| !line.is_empty()) {
            lines.push(String::new());
        }
        lines.extend(editor_text_lines(desc));
        appended_desc_count += 1;
    }
    lines
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
    let mut replacement = editor_text_lines(next_text.trim_end());
    if desc_end < lines.len()
        && !replacement.is_empty()
        && replacement
            .last()
            .is_some_and(|line| !line.trim().is_empty())
    {
        replacement.push(String::new());
    }
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
    renamed_step_line_with_checked(new_key, record.checked)
}

/// 按指定 checkbox 状态生成 step 行。
fn renamed_step_line_with_checked(title: &str, checked: bool) -> String {
    let checkbox = if checked { "[x]" } else { "[ ]" };
    format!("## {checkbox} {title}")
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

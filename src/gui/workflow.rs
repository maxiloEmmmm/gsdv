//! gsdv-spec workflow 文档解析和片段写回。
//!
//! 本模块只处理文件内容和结构化数据，不直接修改 egui 渲染状态。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::gui::data::{
    SubagentViewData, WorkflowAgentTarget, WorkflowAgentTargetKind, WorkspaceActivity,
};

/// workspace 根目录下的 workflow 规范目录。
const GSDV_SPEC_DIR: &str = "gsdv-spec";
/// 历史根项目索引文件名，仅用于惰性迁移。
const LEGACY_ROOT_MD: &str = "root.md";
/// 子项目目录名。
const PROJECTS_DIR: &str = "ps";
/// task 文件名前缀。
const TASK_PREFIX: &str = "task-";
/// Markdown 文件扩展名。
const MARKDOWN_EXT: &str = "md";
/// Repo、Project 和 Task 的稳定短 ID 长度。
pub(super) const NODE_ID_LEN: usize = 4;
/// Step 的 task 内稳定短 ID 长度。
const STEP_ID_LEN: usize = 6;
/// workflow agent sidecar 的通用文件名。
pub(super) const WORKFLOW_AGENT_FILE: &str = ".gsdv-wf-agents";
/// workflow agent sidecar 的 task 文件后缀。
pub(super) const WORKFLOW_TASK_AGENT_SUFFIX: &str = ".gsdv-wf-agents";
/// gsdv-spec 下必须忽略的 workflow agent sidecar 规则。
const WORKFLOW_GITIGNORE_RULES: [&str; 2] = [".gsdv-wf-agents", ".*.gsdv-wf-agents"];

/// 判断字符串是否是固定长度的小写 base36 ID。
///
/// 适用场景：文件名和 step heading 的稳定身份校验。例：`a1b2, 4 -> true`。
pub(super) fn is_stable_id(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

/// 拆分 `<id>.<原名>`，只接受严格稳定 ID。
///
/// 适用场景：Project 目录和 Task 文件名。例：`a1b2.main -> (a1b2, main)`。
pub(super) fn split_stable_name(value: &str, id_len: usize) -> Option<(&str, &str)> {
    let (id, name) = value.split_once('.')?;
    (is_stable_id(id, id_len) && !name.is_empty()).then_some((id, name))
}

/// 生成在给定局部范围内不重复的小写 base36 ID。
///
/// 适用场景：Repo/Project/Task/Step 创建与迁移。例：已有 `a1b2` -> 新四位 ID。
pub(super) fn generate_short_id<'a>(len: usize, used: impl Iterator<Item = &'a str>) -> String {
    let used = used.collect::<std::collections::BTreeSet<_>>();
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    loop {
        let mut rng = rand::rng();
        let id = (0..len)
            .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
            .collect::<String>();
        if !used.contains(id.as_str()) {
            return id;
        }
    }
}

/// 解析 checkbox heading 的状态和紧随 `]` 的剩余文本。
///
/// 适用场景：严格新格式与旧格式迁移共享前缀识别。例：`## [ ]a1b2c3 x -> (false, a1b2c3 x)`。
fn checkbox_step_rest(line: &str) -> Option<(bool, &str)> {
    let rest = line.strip_prefix("## ")?;
    if let Some(rest) = rest.strip_prefix("[ ]") {
        Some((false, rest))
    } else if let Some(rest) = rest.strip_prefix("[x]") {
        Some((true, rest))
    } else if let Some(rest) = rest.strip_prefix("[X]") {
        Some((true, rest))
    } else {
        None
    }
}

/// 判断路径是否是 `<4位ID>.root.md`。
///
/// 适用场景：Repo 根文件发现。例：`a1b2.root.md -> true`。
pub(super) fn is_repo_root_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(id) = name.strip_suffix(".root.md") else {
        return false;
    };
    is_stable_id(id, NODE_ID_LEN)
}

/// 从 Repo 根文件名读取稳定 ID。
///
/// 适用场景：构建 WorkflowTree。例：`a1b2.root.md -> a1b2`。
fn repo_id_from_root_path(path: &Path) -> Result<String, String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid workflow root path: {}", path.display()))?;
    let id = name
        .strip_suffix(".root.md")
        .filter(|id| is_stable_id(id, NODE_ID_LEN))
        .ok_or_else(|| format!("invalid workflow repo id: {}", path.display()))?;
    Ok(id.to_string())
}

/// 查找 gsdv-spec 内唯一的稳定 Repo 根文件。
///
/// 适用场景：本地加载和嵌套 repo 发现。例：一个 `a1b2.root.md -> Some(path)`。
fn workflow_root_file(spec_root: &Path) -> Result<Option<PathBuf>, String> {
    if !spec_root.is_dir() {
        return Ok(None);
    }
    let mut roots = fs::read_dir(spec_root)
        .map_err(|error| format!("failed to read {}: {error}", spec_root.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_repo_root_path(path))
        .collect::<Vec<_>>();
    roots.sort();
    match roots.len() {
        0 => Ok(None),
        1 => Ok(roots.pop()),
        _ => Err(format!(
            "multiple workflow repo roots found in {}",
            spec_root.display()
        )),
    }
}

/// 判断规范目录是否含旧版或稳定 Repo 根文件。
///
/// 适用场景：惰性迁移前发现嵌套 repo；旧 `root.md` 也必须进入迁移。例：legacy root -> true。
fn workflow_repo_has_root(spec_root: &Path) -> Result<bool, String> {
    Ok(spec_root.join(LEGACY_ROOT_MD).is_file() || workflow_root_file(spec_root)?.is_some())
}

/// 为一个本地 repo 的旧 Workflow 结构惰性补齐全部稳定 ID。
///
/// 触发条件：打开 Workflow；只改 gsdv-spec 内受规范管理的文件。
/// 不能依赖 UI 索引：迁移后 Agent 关联必须由文件内 ID 稳定定位。
/// 例：`root.md + ps/main/task-a.md -> a1b2.root.md + ps/c3d4.main/e5f6.task-a.md`。
fn migrate_workflow_repo(repo_root: &Path) -> Result<(), String> {
    let spec_root = repo_root.join(GSDV_SPEC_DIR);
    if !spec_root.is_dir() {
        return Err(format!("{} not found", spec_root.display()));
    }
    ensure_workflow_gitignore(&spec_root)?;
    migrate_repo_root_file(&spec_root)?;
    let projects_root = spec_root.join(PROJECTS_DIR);
    let project_dirs = sorted_dirs(&projects_root)?
        .into_iter()
        .filter(|path| path.join(LEGACY_ROOT_MD).is_file())
        .collect::<Vec<_>>();
    let mut used_project_ids = Vec::new();
    for project_dir in &project_dirs {
        let Some(name) = project_dir.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if let Some((id, _)) = split_stable_name(name, NODE_ID_LEN) {
            if used_project_ids.iter().any(|existing| existing == id) {
                return Err(format!("duplicate workflow project id: {id}"));
            }
            used_project_ids.push(id.to_string());
        }
    }
    for project_dir in project_dirs {
        let name = project_dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("invalid workflow project path: {}", project_dir.display()))?;
        let final_dir = if split_stable_name(name, NODE_ID_LEN).is_some() {
            project_dir
        } else {
            let id = generate_short_id(NODE_ID_LEN, used_project_ids.iter().map(String::as_str));
            used_project_ids.push(id.clone());
            let target = projects_root.join(format!("{id}.{name}"));
            fs::rename(&project_dir, &target).map_err(|error| {
                format!(
                    "failed to migrate {} to {}: {error}",
                    project_dir.display(),
                    target.display()
                )
            })?;
            target
        };
        migrate_project_tasks(&final_dir)?;
    }
    Ok(())
}

/// 创建或增量补齐 gsdv-spec/.gitignore 的 sidecar 通配规则。
///
/// 适用场景：初始化和惰性迁移；保留用户已有内容。例：已有一条规则 -> 只追加缺失项。
fn ensure_workflow_gitignore(spec_root: &Path) -> Result<(), String> {
    let path = spec_root.join(".gitignore");
    let current = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("failed to read {}: {error}", path.display())),
    };
    let existing = current
        .lines()
        .map(str::trim)
        .collect::<std::collections::BTreeSet<_>>();
    let missing = WORKFLOW_GITIGNORE_RULES
        .iter()
        .filter(|rule| !existing.contains(**rule))
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    let mut next = current;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    for rule in missing {
        next.push_str(rule);
        next.push('\n');
    }
    atomic_write(&path, next.as_bytes())
}

/// 将旧 root.md 迁移为带四位 Repo ID 的根文件。
///
/// 适用场景：首次打开旧规范。例：`root.md -> a1b2.root.md`。
fn migrate_repo_root_file(spec_root: &Path) -> Result<(), String> {
    let stable = workflow_root_file(spec_root)?;
    let legacy = spec_root.join(LEGACY_ROOT_MD);
    match (stable, legacy.is_file()) {
        (Some(_), false) => Ok(()),
        (Some(_), true) => Err(format!(
            "legacy and stable workflow repo roots both exist in {}",
            spec_root.display()
        )),
        (None, true) => {
            let id = generate_short_id(NODE_ID_LEN, std::iter::empty());
            let target = spec_root.join(format!("{id}.root.md"));
            fs::rename(&legacy, &target).map_err(|error| {
                format!(
                    "failed to migrate {} to {}: {error}",
                    legacy.display(),
                    target.display()
                )
            })
        }
        (None, false) => Err(format!("{} not found", spec_root.display())),
    }
}

/// 迁移一个 Project 目录内的 Task 文件与 Step heading。
///
/// 适用场景：Project 已有稳定目录 ID。例：`task-a.md -> e5f6.task-a.md`。
fn migrate_project_tasks(project_dir: &Path) -> Result<(), String> {
    let mut task_paths = fs::read_dir(project_dir)
        .map_err(|error| format!("failed to read {}: {error}", project_dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.extension().is_some_and(|ext| ext == MARKDOWN_EXT)
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.starts_with(TASK_PREFIX)
                            || name.split_once('.').is_some_and(|(id, rest)| {
                                is_stable_id(id, NODE_ID_LEN) && rest.starts_with(TASK_PREFIX)
                            })
                    })
        })
        .collect::<Vec<_>>();
    task_paths.sort();
    let mut used_task_ids = Vec::new();
    for task_path in &task_paths {
        let Some(stem) = task_path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if let Some((id, _)) = split_stable_name(stem, NODE_ID_LEN) {
            if used_task_ids.iter().any(|existing| existing == id) {
                return Err(format!("duplicate workflow task id: {id}"));
            }
            used_task_ids.push(id.to_string());
        }
    }
    for task_path in task_paths {
        let content = fs::read_to_string(&task_path)
            .map_err(|error| format!("failed to read {}: {error}", task_path.display()))?;
        let next_content = migrate_legacy_step_content(&content)?;
        let stem = task_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| format!("invalid workflow task path: {}", task_path.display()))?;
        let final_path = if split_stable_name(stem, NODE_ID_LEN).is_some() {
            task_path.clone()
        } else {
            let id = generate_short_id(NODE_ID_LEN, used_task_ids.iter().map(String::as_str));
            used_task_ids.push(id.clone());
            let target = project_dir.join(format!("{id}.{stem}.{MARKDOWN_EXT}"));
            fs::rename(&task_path, &target).map_err(|error| {
                format!(
                    "failed to migrate {} to {}: {error}",
                    task_path.display(),
                    target.display()
                )
            })?;
            target
        };
        if next_content != content {
            atomic_write(&final_path, next_content.as_bytes())?;
        }
    }
    Ok(())
}

/// 把旧 `## [ ] title` 批量改为严格 `## [ ]<6位ID> title`。
///
/// 适用场景：Task 惰性迁移；格式疑似新格式但 ID 非法时硬错误。
/// 例：两个旧 step -> 两个 task 内不重复的六位 ID。
pub(super) fn migrate_legacy_step_content(content: &str) -> Result<String, String> {
    let mut lines = markdown_lines(content);
    let mut used = Vec::<String>::new();
    let mut changed = false;
    for line in &lines {
        if line.starts_with("## [") && checkbox_step_rest(line).is_none() {
            return Err(format!("invalid workflow checkbox heading: {line}"));
        }
        let Some((_, rest)) = checkbox_step_rest(line) else {
            continue;
        };
        if let Some(title) = rest.strip_prefix(' ') {
            if title.trim().is_empty() {
                return Err("workflow step title is empty".to_string());
            }
            continue;
        }
        if rest.len() <= STEP_ID_LEN || rest.as_bytes().get(STEP_ID_LEN) != Some(&b' ') {
            return Err(format!("invalid workflow step id heading: {line}"));
        }
        let id = &rest[..STEP_ID_LEN];
        if !is_stable_id(id, STEP_ID_LEN) || rest[STEP_ID_LEN + 1..].trim().is_empty() {
            return Err(format!("invalid workflow step id heading: {line}"));
        }
        if used.iter().any(|existing| existing == id) {
            return Err(format!("duplicate workflow step id: {id}"));
        }
        used.push(id.to_string());
    }
    for line in &mut lines {
        let Some((checked, rest)) = checkbox_step_rest(line) else {
            continue;
        };
        let Some(title) = rest.strip_prefix(' ') else {
            if line.starts_with("## [X]") {
                *line = line.replacen("## [X]", "## [x]", 1);
                changed = true;
            }
            continue;
        };
        let id = generate_short_id(STEP_ID_LEN, used.iter().map(String::as_str));
        used.push(id.clone());
        *line = step_line_with_checked(&id, title.trim(), checked);
        changed = true;
    }
    if changed {
        Ok(join_markdown_lines(&lines))
    } else {
        Ok(content.to_string())
    }
}

/// 使用同目录临时文件原子覆盖业务 sidecar 或 Markdown。
///
/// 适用场景：迁移中断不能留下半份 JSON/Markdown。例：`bytes -> temp -> rename`。
fn atomic_write(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("path parent not found: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    let temp = parent.join(format!(
        ".{}.gsdv-{:016x}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("workflow"),
        rand::random::<u64>()
    ));
    fs::write(&temp, content)
        .map_err(|error| format!("failed to write {}: {error}", temp.display()))?;
    fs::rename(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        format!(
            "failed to replace {} with {}: {error}",
            path.display(),
            temp.display()
        )
    })
}

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
    error == format!("{} not found", workspace_root.join(GSDV_SPEC_DIR).display())
}

/// workflow 树加载结果。
#[derive(Debug, Clone, Default)]
pub(super) struct WorkflowTree {
    /// 当前 repo 的四位稳定 ID。
    pub id: String,
    /// 当前 workflow 所属 repo 相对 workspace 的路径；主 repo 为空。
    pub repo_path: PathBuf,
    /// workspace 内相对的 gsdv-spec 目录。
    pub spec_path: PathBuf,
    /// workspace 级 workflow root.md 相对 workspace 的路径。
    pub root_path: PathBuf,
    /// workspace 级 root.md 的完整 Markdown 内容。
    pub content: String,
    /// 当前 workspace 的 workflow 项目列表。
    pub projects: Vec<WorkflowProjectNode>,
    /// 当前 repo 从分层 sidecar 加载的 Workflow Agent。
    pub agents: Vec<SubagentViewData>,
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
    /// Repo 内唯一的四位稳定 ID。
    pub id: String,
    /// 项目目录名，也是 tree 中的稳定 key。
    pub key: String,
    /// tree 中显示的项目名。
    pub label: String,
    /// 项目 root.md 相对 workspace 的路径。
    pub root_path: PathBuf,
    /// 项目 root.md 的完整 Markdown 内容。
    pub content: String,
    /// 项目下的 task 文档节点。
    pub tasks: Vec<WorkflowTaskNode>,
}

/// workflow task 节点。
#[derive(Debug, Clone)]
pub(super) struct WorkflowTaskNode {
    /// Project 内唯一的四位稳定 ID。
    pub id: String,
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
    /// Task 内唯一的六位稳定 ID。
    pub id: String,
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

/// Workflow Agent sidecar 的版本化载荷。
#[derive(Debug, Default, Deserialize, Serialize)]
struct WorkflowAgentFile {
    /// 文件格式版本。
    #[serde(default = "workflow_agent_file_version")]
    version: u8,
    /// 当前 sidecar 管理的 Agent 记录。
    #[serde(default)]
    agents: Vec<WorkflowAgentRecord>,
}

/// 一条目标与现有 Agent 配置的关联。
#[derive(Debug, Deserialize, Serialize)]
struct WorkflowAgentRecord {
    /// 稳定 Workflow 目标。
    target: WorkflowAgentTarget,
    /// 复用普通 Subagent 的运行配置与 session。
    agent: SubagentViewData,
}

/// 返回当前 Workflow Agent sidecar 版本。
///
/// 适用场景：旧 JSON 缺 version 时使用。例：缺字段 -> 1。
const fn workflow_agent_file_version() -> u8 {
    1
}

/// Workflow 目标解析出的 Agent 绑定信息。
#[derive(Debug, Clone)]
pub(super) struct WorkflowAgentBinding {
    /// 创建或打开 Agent 后需要保持的 UI 选择。
    pub selection: WorkflowSelectionTarget,
    /// 稳定层级身份。
    pub target: WorkflowAgentTarget,
    /// sidecar 相对 workspace 的路径。
    pub sidecar_path: PathBuf,
    /// 创建面板默认展示名。
    pub label: String,
}

/// 把当前树中的 UI 选择解析成稳定 Agent 目标与 sidecar。
///
/// 适用场景：右键 Agent、销毁与 Busy 冒泡。例：Step -> task 专属 sidecar + step ID。
pub(super) fn workflow_agent_binding(
    tree: &WorkflowTree,
    selected: &WorkflowSelectionTarget,
) -> Option<WorkflowAgentBinding> {
    for owner in workflow_trees(tree) {
        if let WorkflowSelectionTarget::WorkspaceRoot { root_path } = selected
            && root_path == &owner.root_path
        {
            return Some(WorkflowAgentBinding {
                selection: selected.clone(),
                target: WorkflowAgentTarget {
                    kind: WorkflowAgentTargetKind::Repo,
                    repo_path: owner.repo_path.clone(),
                    repo_id: owner.id.clone(),
                    project_id: None,
                    task_id: None,
                    step_id: None,
                },
                sidecar_path: owner.spec_path.join(WORKFLOW_AGENT_FILE),
                label: format!("Repo {}", owner.id),
            });
        }
        for project in &owner.projects {
            let project_dir = project.root_path.parent()?;
            if let WorkflowSelectionTarget::Project { root_path } = selected
                && root_path == &project.root_path
            {
                return Some(WorkflowAgentBinding {
                    selection: selected.clone(),
                    target: WorkflowAgentTarget {
                        kind: WorkflowAgentTargetKind::Project,
                        repo_path: owner.repo_path.clone(),
                        repo_id: owner.id.clone(),
                        project_id: Some(project.id.clone()),
                        task_id: None,
                        step_id: None,
                    },
                    sidecar_path: project_dir.join(WORKFLOW_AGENT_FILE),
                    label: project.label.clone(),
                });
            }
            for task in &project.tasks {
                let task_sidecar =
                    project_dir.join(format!(".{}{}", task.id, WORKFLOW_TASK_AGENT_SUFFIX));
                if let WorkflowSelectionTarget::Task { task_path } = selected
                    && task_path == &task.path
                {
                    return Some(WorkflowAgentBinding {
                        selection: selected.clone(),
                        target: WorkflowAgentTarget {
                            kind: WorkflowAgentTargetKind::Task,
                            repo_path: owner.repo_path.clone(),
                            repo_id: owner.id.clone(),
                            project_id: Some(project.id.clone()),
                            task_id: Some(task.id.clone()),
                            step_id: None,
                        },
                        sidecar_path: task_sidecar,
                        label: task.label.clone(),
                    });
                }
                if let WorkflowSelectionTarget::Step {
                    task_path,
                    step_path,
                } = selected
                    && task_path == &task.path
                    && let Some(step) = workflow_step_node_at_path(&task.steps, step_path)
                {
                    return Some(WorkflowAgentBinding {
                        selection: selected.clone(),
                        target: WorkflowAgentTarget {
                            kind: WorkflowAgentTargetKind::Step,
                            repo_path: owner.repo_path.clone(),
                            repo_id: owner.id.clone(),
                            project_id: Some(project.id.clone()),
                            task_id: Some(task.id.clone()),
                            step_id: Some(step.id.clone()),
                        },
                        sidecar_path: task_sidecar,
                        label: step.title.clone(),
                    });
                }
            }
        }
    }
    None
}

/// 按索引路径定位 step 节点。
///
/// 适用场景：选择仍使用 UI path，持久化改用稳定 ID。例：`[1] -> 第二个 step`。
fn workflow_step_node_at_path<'a>(
    nodes: &'a [WorkflowStepNode],
    path: &[usize],
) -> Option<&'a WorkflowStepNode> {
    let (first, rest) = path.split_first()?;
    let node = nodes.get(*first)?;
    if rest.is_empty() {
        Some(node)
    } else {
        workflow_step_node_at_path(&node.children, rest)
    }
}

/// 从本地分层 sidecar 加载当前 repo 的 Workflow Agent。
///
/// 适用场景：Workflow tree 后台加载；缺 sidecar 等同于无 Agent。例：Repo + 2 Task 文件 -> 合并 records。
fn load_local_workflow_agents(
    workspace_root: &Path,
    tree: &WorkflowTree,
) -> Result<Vec<SubagentViewData>, String> {
    let mut agents = Vec::new();
    let mut seen_targets = BTreeSet::new();
    for sidecar_path in workflow_agent_sidecar_paths(tree) {
        let absolute = workspace_root.join(&sidecar_path);
        let content = match fs::read_to_string(&absolute) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!("failed to read {}: {error}", absolute.display()));
            }
        };
        append_workflow_agent_file(
            &content,
            &sidecar_path,
            workspace_root,
            tree,
            &mut seen_targets,
            &mut agents,
        )?;
    }
    Ok(agents)
}

/// 返回一棵 repo tree 管理的全部 Workflow Agent sidecar 路径。
///
/// 适用场景：本地加载只读规范路径，不扫描无关隐藏文件。例：Repo + Project + Task 路径集合。
fn workflow_agent_sidecar_paths(tree: &WorkflowTree) -> Vec<PathBuf> {
    let mut paths = vec![tree.spec_path.join(WORKFLOW_AGENT_FILE)];
    for project in &tree.projects {
        let Some(project_dir) = project.root_path.parent() else {
            continue;
        };
        paths.push(project_dir.join(WORKFLOW_AGENT_FILE));
        paths.extend(
            project.tasks.iter().map(|task| {
                project_dir.join(format!(".{}{}", task.id, WORKFLOW_TASK_AGENT_SUFFIX))
            }),
        );
    }
    paths
}

/// 由稳定目标反查其唯一合法 sidecar 路径。
///
/// 适用场景：加载 JSON 时拒绝孤儿目标和层级错放。例：Step 记录只能位于所属 Task sidecar。
fn workflow_agent_target_sidecar(
    tree: &WorkflowTree,
    target: &WorkflowAgentTarget,
) -> Option<PathBuf> {
    if target.repo_path != tree.repo_path || target.repo_id != tree.id {
        return None;
    }
    if target.kind == WorkflowAgentTargetKind::Repo {
        return (target.project_id.is_none()
            && target.task_id.is_none()
            && target.step_id.is_none())
        .then(|| tree.spec_path.join(WORKFLOW_AGENT_FILE));
    }
    let project_id = target.project_id.as_deref()?;
    let project = tree
        .projects
        .iter()
        .find(|project| project.id == project_id)?;
    let project_dir = project.root_path.parent()?;
    if target.kind == WorkflowAgentTargetKind::Project {
        return (target.task_id.is_none() && target.step_id.is_none())
            .then(|| project_dir.join(WORKFLOW_AGENT_FILE));
    }
    let task_id = target.task_id.as_deref()?;
    let task = project.tasks.iter().find(|task| task.id == task_id)?;
    let task_sidecar = project_dir.join(format!(".{}{}", task.id, WORKFLOW_TASK_AGENT_SUFFIX));
    match target.kind {
        WorkflowAgentTargetKind::Task => target.step_id.is_none().then_some(task_sidecar),
        WorkflowAgentTargetKind::Step => {
            let step_id = target.step_id.as_deref()?;
            workflow_step_id_exists(&task.steps, step_id).then_some(task_sidecar)
        }
        WorkflowAgentTargetKind::Repo | WorkflowAgentTargetKind::Project => None,
    }
}

/// 递归判断 Task 内是否存在指定稳定 Step ID。
///
/// 适用场景：加载 Workflow Agent sidecar。例：子 Step ID 命中 -> true。
fn workflow_step_id_exists(steps: &[WorkflowStepNode], id: &str) -> bool {
    steps
        .iter()
        .any(|step| step.id == id || workflow_step_id_exists(&step.children, id))
}

/// 解析一个 sidecar，并把运行时目标与路径补回 Subagent。
///
/// 适用场景：本地与 Remote 共享 JSON 规则。例：重复 target -> 硬错误。
fn append_workflow_agent_file(
    content: &str,
    sidecar_path: &Path,
    workspace_scope: &Path,
    tree: &WorkflowTree,
    seen_targets: &mut BTreeSet<WorkflowAgentTarget>,
    agents: &mut Vec<SubagentViewData>,
) -> Result<(), String> {
    let file = serde_json::from_str::<WorkflowAgentFile>(content)
        .map_err(|error| format!("failed to parse {}: {error}", sidecar_path.display()))?;
    if file.version != workflow_agent_file_version() {
        return Err(format!(
            "unsupported workflow agent file version {} in {}",
            file.version,
            sidecar_path.display()
        ));
    }
    for mut record in file.agents {
        record.target.repo_path = tree.repo_path.clone();
        let Some(expected_sidecar) = workflow_agent_target_sidecar(tree, &record.target) else {
            // mutation 与 sidecar writer 是异步串行的；节点先删除时忽略短暂孤儿记录。
            continue;
        };
        if expected_sidecar != sidecar_path {
            return Err(format!(
                "workflow agent target does not belong in {}",
                sidecar_path.display()
            ));
        }
        if !seen_targets.insert(record.target.clone()) {
            return Err(format!(
                "duplicate workflow agent target in {}",
                sidecar_path.display()
            ));
        }
        let runtime_id = workflow_agent_runtime_id(workspace_scope, &record.target);
        record.agent.id = runtime_id.clone();
        record.agent.agent_id = runtime_id;
        record.agent.workflow_target = Some(record.target);
        record.agent.workflow_sidecar_path = Some(sidecar_path.to_path_buf());
        record.agent.activity = WorkspaceActivity::Unknown;
        agents.push(record.agent);
    }
    Ok(())
}

/// 返回一个目标稳定且不会进入普通 Agent 命名空间的运行时 ID。
///
/// 适用场景：新建或加载记录时隔离 workspace/repo 命名空间。例：不同路径 -> 不同 ID。
pub(super) fn workflow_agent_runtime_id(
    workspace_scope: &Path,
    target: &WorkflowAgentTarget,
) -> String {
    let repo_scope =
        crate::gui::data::workspace_store_key(&workspace_scope.join(&target.repo_path));
    let leaf = target
        .step_id
        .as_deref()
        .or(target.task_id.as_deref())
        .or(target.project_id.as_deref())
        .unwrap_or(target.repo_id.as_str());
    format!(
        "wf-{repo_scope}-{}-{}-{leaf}",
        target.repo_id,
        match target.kind {
            WorkflowAgentTargetKind::Repo => "repo",
            WorkflowAgentTargetKind::Project => "project",
            WorkflowAgentTargetKind::Task => "task",
            WorkflowAgentTargetKind::Step => "step",
        }
    )
}

/// 生成本地 Workspace 的 Workflow Agent 作用域。
///
/// 适用场景：同一路径稳定生成同一个 Agent ID。例：`/repo -> /repo`。
pub(super) fn workflow_agent_workspace_scope(workspace_path: &Path) -> PathBuf {
    workspace_path.to_path_buf()
}

/// 原子保存一个本地 Workflow Agent sidecar；空记录直接删除文件。
///
/// 适用场景：创建、配置更新与销毁。例：最后一个 Agent 删除 -> sidecar 消失。
pub(super) fn save_local_workflow_agent_sidecar(
    workspace_root: &Path,
    sidecar_path: &Path,
    agents: &[SubagentViewData],
) -> Result<(), String> {
    let absolute = workspace_root.join(sidecar_path);
    let Some(content) = workflow_agent_sidecar_content(sidecar_path, agents)? else {
        return match fs::remove_file(&absolute) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("failed to delete {}: {error}", absolute.display())),
        };
    };
    let parent = absolute
        .parent()
        .ok_or_else(|| format!("workflow agent parent not found: {}", absolute.display()))?;
    if !parent.is_dir() {
        return Err(format!(
            "workflow agent parent not found: {}",
            parent.display()
        ));
    }
    atomic_write(&absolute, &content)
}

/// 序列化指定路径管理的 Workflow Agent；无记录返回 None。
///
/// 适用场景：本地与 Remote writer 共用完全相同的 JSON。例：一个 Task Agent -> versioned bytes。
pub(super) fn workflow_agent_sidecar_content(
    sidecar_path: &Path,
    agents: &[SubagentViewData],
) -> Result<Option<Vec<u8>>, String> {
    let records = agents
        .iter()
        .filter(|agent| agent.workflow_sidecar_path.as_deref() == Some(sidecar_path))
        .filter_map(|agent| {
            Some(WorkflowAgentRecord {
                target: agent.workflow_target.clone()?,
                agent: agent.clone(),
            })
        })
        .collect::<Vec<_>>();
    if records.is_empty() {
        return Ok(None);
    }
    let content = serde_json::to_vec_pretty(&WorkflowAgentFile {
        version: workflow_agent_file_version(),
        agents: records,
    })
    .map_err(|error| format!("failed to serialize {}: {error}", sidecar_path.display()))?;
    Ok(Some(content))
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
    migrate_workflow_repo(workspace_root)?;
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
        let repo_root = workspace_root.join(&repo_path);
        if !workflow_repo_has_root(&repo_root.join(GSDV_SPEC_DIR))? {
            continue;
        }
        migrate_workflow_repo(&repo_root)?;
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
    let root_md = workflow_root_file(&spec_root)?
        .ok_or_else(|| format!("{} not found", spec_root.display()))?;
    let repo_id = repo_id_from_root_path(&root_md)?;
    let content = fs::read_to_string(&root_md)
        .map_err(|error| format!("failed to read {}: {error}", root_md.display()))?;
    let projects_root = spec_root.join(PROJECTS_DIR);
    let mut projects = Vec::new();
    for entry in sorted_dirs(&projects_root)? {
        let root_path = entry.join(LEGACY_ROOT_MD);
        if !root_path.is_file() {
            continue;
        }
        let Some(project_name) = entry.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let (project_id, project_label) = split_stable_name(project_name, NODE_ID_LEN)
            .ok_or_else(|| format!("invalid workflow project id: {project_name}"))?;
        let project_content = fs::read_to_string(&root_path)
            .map_err(|error| format!("failed to read {}: {error}", root_path.display()))?;
        let tasks = load_project_tasks(workspace_root, &entry)?;
        projects.push(WorkflowProjectNode {
            id: project_id.to_string(),
            key: project_name.to_string(),
            label: project_label.to_string(),
            root_path: relative_to_workspace(workspace_root, &root_path),
            content: project_content,
            tasks,
        });
    }
    let mut tree = WorkflowTree {
        id: repo_id,
        repo_path: repo_path.to_path_buf(),
        spec_path: relative_to_workspace(workspace_root, &spec_root),
        root_path: relative_to_workspace(workspace_root, &root_md),
        content,
        projects,
        agents: Vec::new(),
        sub_workflows: Vec::new(),
    };
    tree.agents = load_local_workflow_agents(workspace_root, &tree)?;
    Ok(tree)
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
    if !relative_dir.as_os_str().is_empty()
        && fs::symlink_metadata(absolute_dir.join(".git")).is_ok()
    {
        if workflow_repo_has_root(&absolute_dir.join(GSDV_SPEC_DIR))? {
            repo_paths.push(relative_dir.to_path_buf());
        }
        return Ok(());
    }
    let entries = match fs::read_dir(&absolute_dir) {
        Ok(entries) => entries,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            // 子 workflow 发现是可选能力。扫描期间目录消失或不可读时，
            // 主 workflow 仍必须可用，不能让一个旁支目录拖垮整棵 tree。
            return Ok(());
        }
        Err(error) => {
            return Err(format!(
                "failed to read {}: {error}",
                absolute_dir.display()
            ));
        }
    };
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let child_relative = relative_dir.join(&name);
        discover_nested_git_repo_paths_in(workspace_root, &child_relative, repo_paths)?;
    }
    Ok(())
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
        let (id, label) = task_identity_for_path(&path)?;
        tasks.push(WorkflowTaskNode {
            id,
            label,
            path: relative_to_workspace(workspace_root, &path),
            desc: parse_task_desc(&content),
            steps,
        });
    }
    Ok(tasks)
}

/// 返回 task tree 的展示名，保留文件命名规范但隐藏 task- 前缀。
fn task_identity_for_path(path: &Path) -> Result<(String, String), String> {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .ok_or_else(|| format!("task name not found: {}", path.display()))?;
    let (id, legacy_name) = split_stable_name(&stem, NODE_ID_LEN)
        .ok_or_else(|| format!("invalid workflow task id: {}", path.display()))?;
    let label = legacy_name
        .strip_prefix(TASK_PREFIX)
        .unwrap_or(legacy_name)
        .to_string();
    Ok((id.to_string(), label))
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
            .and_then(|name| name.split_once('.'))
            .is_some_and(|(id, name)| {
                is_stable_id(id, NODE_ID_LEN) && name.starts_with(TASK_PREFIX)
            })
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
    if text.lines().any(|line| line.starts_with("## [")) {
        return Err("Task description cannot contain level-2 checkbox headings".to_string());
    }
    Ok(())
}

/// 校验 step 正文，避免正文伪装成新的 step heading。
fn validate_workflow_step_desc(text: &str) -> Result<(), String> {
    if text.lines().any(|line| line.starts_with("## [")) {
        return Err("Step description cannot contain level-2 checkbox headings".to_string());
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
    let spec_root = workspace_root.join(GSDV_SPEC_DIR);
    fs::create_dir_all(&spec_root)
        .map_err(|error| format!("failed to create {}: {error}", spec_root.display()))?;
    ensure_workflow_gitignore(&spec_root)?;
    if workflow_root_file(&spec_root)?.is_some() {
        return Ok(());
    }
    let legacy = spec_root.join(LEGACY_ROOT_MD);
    if legacy.exists() {
        return migrate_repo_root_file(&spec_root);
    }
    let id = generate_short_id(NODE_ID_LEN, std::iter::empty());
    let root_path = spec_root.join(format!("{id}.root.md"));
    fs::write(&root_path, [])
        .map_err(|error| format!("failed to write {}: {error}", root_path.display()))
}

/// 创建 workflow project 根文件。
fn add_workflow_project(
    workspace_root: &Path,
    spec_path: &Path,
    project_key: &str,
) -> Result<(), String> {
    validate_workflow_relative_path(spec_path)?;
    let project_key = validate_workflow_key(project_key)?;
    let projects_dir = workspace_root.join(spec_path).join(PROJECTS_DIR);
    let identities = sorted_dirs(&projects_dir)?
        .into_iter()
        .filter(|path| path.join(LEGACY_ROOT_MD).is_file())
        .filter_map(|path| path.file_name()?.to_str().map(str::to_string))
        .filter_map(|name| {
            split_stable_name(&name, NODE_ID_LEN)
                .map(|(id, label)| (id.to_string(), label.to_string()))
        })
        .collect::<Vec<_>>();
    if identities.iter().any(|(_, label)| label == project_key) {
        return Err(format!("workflow project already exists: {project_key}"));
    }
    let used = identities
        .iter()
        .map(|(id, _)| id.as_str())
        .collect::<Vec<_>>();
    let project_id = generate_short_id(NODE_ID_LEN, used.into_iter());
    let project_dir = projects_dir.join(format!("{project_id}.{project_key}"));
    let root_path = project_dir.join(LEGACY_ROOT_MD);
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
    let identities = sorted_markdown_tasks(&project_dir)?
        .into_iter()
        .filter_map(|path| task_identity_for_path(&path).ok())
        .collect::<Vec<_>>();
    if identities.iter().any(|(_, label)| label == task_key) {
        return Err(format!("task already exists: {task_key}"));
    }
    let task_id = generate_short_id(NODE_ID_LEN, identities.iter().map(|(id, _)| id.as_str()));
    let task_path = project_dir.join(format!("{task_id}.{TASK_PREFIX}{task_key}.{MARKDOWN_EXT}"));
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
    let records = parse_step_records(content);
    if records.iter().any(|record| record.title == key) {
        return Err(format!("step already exists at this level: {key}"));
    }
    let step_id = generate_short_id(STEP_ID_LEN, records.iter().map(|record| record.id.as_str()));
    if !lines.is_empty() && lines.last().is_some_and(|line| !line.trim().is_empty()) {
        lines.push(String::new());
    }
    lines.push(format!("## [ ]{step_id} {key}"));
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
    let (project_id, current_label) = split_stable_name(project_key, NODE_ID_LEN)
        .ok_or_else(|| format!("invalid workflow project id: {project_key}"))?;
    let new_key = validate_workflow_key(new_key)?;
    if current_label == new_key {
        return Ok(());
    }
    let old_dir = workspace_root.join(project_path);
    let projects_dir = old_dir.parent().ok_or_else(|| {
        format!(
            "workflow project parent not found: {}",
            project_path.display()
        )
    })?;
    let new_dir = projects_dir.join(format!("{project_id}.{new_key}"));
    if !old_dir.is_dir() {
        return Err(format!("workflow project not found: {current_label}"));
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
    let task_id = task_identity_for_path(task_path)?.0;
    let target = parent.join(format!("{task_id}.{TASK_PREFIX}{new_key}.{MARKDOWN_EXT}"));
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
    let merged_id = generate_short_id(STEP_ID_LEN, records.iter().map(|record| record.id.as_str()));
    let replacement = merged_workflow_step_lines(selected_records, &merged_id, title);
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
fn merged_workflow_step_lines(
    records: Vec<StepRecord>,
    merged_id: &str,
    title: &str,
) -> Vec<String> {
    let checked = records.iter().all(|record| record.checked);
    let mut lines = vec![step_line_with_checked(merged_id, title, checked)];
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
            id: record.id,
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
                id: step.id.clone(),
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
                id: parsed.id,
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
    let (checked, rest) = checkbox_step_rest(line)?;
    if rest.len() <= STEP_ID_LEN || rest.as_bytes().get(STEP_ID_LEN) != Some(&b' ') {
        return None;
    }
    let id = &rest[..STEP_ID_LEN];
    if !is_stable_id(id, STEP_ID_LEN) {
        return None;
    }
    let title = rest[STEP_ID_LEN + 1..].trim().to_string();
    (!title.is_empty()).then_some(ParsedStepLine {
        id: id.to_string(),
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
    step_line_with_checked(&record.id, new_key, record.checked)
}

/// 按指定 checkbox 状态生成 step 行。
fn step_line_with_checked(id: &str, title: &str, checked: bool) -> String {
    let checkbox = if checked { "[x]" } else { "[ ]" };
    format!("## {checkbox}{id} {title}")
}

/// 从 task 路径提取不带 `task-` 前缀的 key。
fn task_key_from_path(task_path: &Path) -> Result<String, String> {
    Ok(task_identity_for_path(task_path)?.1)
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
    /// task 内唯一的六位稳定 ID。
    id: String,
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
    /// task 内唯一的六位稳定 ID。
    id: String,
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
    /// task 内唯一的六位稳定 ID。
    id: String,
    /// step 标题。
    title: String,
    /// step 是否完成。
    checked: bool,
    /// step 是否带 checkbox。
    checkable: bool,
}

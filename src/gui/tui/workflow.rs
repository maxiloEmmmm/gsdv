//! TUI 只读 Workflow 浏览器与轻量 Markdown 渲染。

use super::*;
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::borrow::Cow;
use std::collections::BTreeSet;

/// TUI Workflow 浏览器的持久帧状态。
///
/// 适用场景：在 Agent 与 Workflow 视图间切换。示例：重新打开同一 workspace -> 保留选择。
#[derive(Debug, Clone, Default)]
pub(super) struct WorkflowBrowser {
    /// 当前浏览的 workspace 索引。
    pub workspace_index: Option<usize>,
    /// 当前右侧预览目标。
    pub selected: Option<WorkflowSelectionTarget>,
    /// 当前折叠的 project 状态键。
    pub collapsed_projects: BTreeSet<String>,
    /// 左侧 tree 首个可见行。
    pub tree_scroll: usize,
    /// 右侧 Markdown 首个可见行。
    pub content_scroll: usize,
    /// task 说明 Markdown 首个可见行。
    pub task_scroll: usize,
    /// step tree 首个可见行。
    pub step_tree_scroll: usize,
    /// step 说明 Markdown 首个可见行。
    pub step_content_scroll: usize,
}

/// Workflow 左侧 tree 的一行。
///
/// 适用场景：绘制与鼠标命中共享同一份扁平列表。示例：project 行可折叠。
#[derive(Debug, Clone)]
pub(super) struct WorkflowBrowserRow {
    /// 相对左侧 tree 的缩进层级。
    pub depth: usize,
    /// 行内展示文本。
    pub label: String,
    /// 点击后可预览的目标；repo 分隔行没有目标。
    pub target: Option<WorkflowSelectionTarget>,
    /// project 行对应的折叠状态键。
    pub project_key: Option<String>,
    /// 右键复制的逻辑路径。
    pub copy_path: Option<String>,
    /// task 是否全部完成。
    pub done: bool,
    /// 是否为子 repo 分隔行。
    pub repo_header: bool,
}

/// Workflow task 下方 step tree 的一行。
///
/// 适用场景：task 工作台绘制、点击和滚动共用。示例：子 step 使用更深缩进。
#[derive(Debug, Clone)]
struct WorkflowStepRow {
    /// 相对 step tree 的缩进层级。
    depth: usize,
    /// 带完成状态的显示文本。
    label: String,
    /// 点击后预览的 step 目标。
    target: WorkflowSelectionTarget,
    /// 右键复制的逻辑路径。
    copy_path: String,
    /// step 是否完成。
    checked: bool,
}

/// Workflow 浏览器内的独立滚动区域。
///
/// 适用场景：鼠标滚轮根据指针位置只滚动命中的 panel。
#[derive(Debug, Clone, Copy)]
pub(super) enum WorkflowScrollRegion {
    /// 左侧 workspace/project/task tree。
    Tree,
    /// root 或 project 的完整 Markdown。
    Content,
    /// task 自身说明 Markdown。
    Task,
    /// task 下方的 step tree。
    Steps,
    /// 当前 step 的说明 Markdown。
    StepContent,
}

/// Markdown 行的语义样式。
///
/// 适用场景：把 CommonMark block 映射到 xterm 色彩。示例：Heading -> 蓝色粗体。
#[derive(Debug, Clone, Copy, Default)]
enum MarkdownLineStyle {
    /// 普通正文。
    #[default]
    Body,
    /// 标题及其级别。
    Heading(u8),
    /// 引用块。
    Quote,
    /// 代码块。
    Code,
    /// 分隔线。
    Rule,
}

/// 解析后的单条 Markdown 显示行。
///
/// 适用场景：右侧只读预览滚动。示例：长段落按终端宽度拆成多行。
#[derive(Debug, Clone, Default)]
struct MarkdownLine {
    /// 不含 ANSI 的纯文本。
    text: String,
    /// 整行语义样式。
    style: MarkdownLineStyle,
}

/// Markdown 解析期间的列表层级状态。
///
/// 适用场景：无序和有序列表生成稳定前缀。示例：有序列表从 3 开始。
#[derive(Debug, Clone, Copy)]
struct MarkdownListState {
    /// 有序列表下一项编号；None 表示无序列表。
    next_number: Option<u64>,
}

/// 让浏览器跟随当前 workspace，并在 tree 首次到达时选择 root。
///
/// 适用场景：异步 WorkflowTreeLoaded 到达。示例：None -> 选中 root.md。
pub(super) fn sync_browser(
    browser: &mut WorkflowBrowser,
    workspace_index: usize,
    tree: Option<&WorkflowTree>,
) {
    if browser.workspace_index != Some(workspace_index) {
        *browser = WorkflowBrowser {
            workspace_index: Some(workspace_index),
            ..WorkflowBrowser::default()
        };
    }
    let Some(tree) = tree else {
        return;
    };
    let selection_exists = selected_markdown(tree, browser.selected.as_ref()).is_some();
    if !selection_exists {
        browser.selected = Some(WorkflowSelectionTarget::WorkspaceRoot {
            root_path: tree.root_path.clone(),
        });
        browser.content_scroll = 0;
        browser.task_scroll = 0;
        browser.step_tree_scroll = 0;
        browser.step_content_scroll = 0;
    }
}

/// 生成可比较的 Workflow 浏览状态摘要。
///
/// 适用场景：后台 tree 刷新后标记 TUI dirty。示例：新增 task -> fingerprint 变化。
pub(super) fn state_fingerprint(state: Option<&WorkflowUiState>) -> String {
    let Some(state) = state else {
        return "missing".to_string();
    };
    let mut value = format!(
        "{}:{:?}:{}:{:?}",
        state.loading,
        state.load_error,
        state.tree.is_some(),
        state.active_agent_target,
    );
    if let Some(tree) = state.tree.as_ref() {
        append_tree_fingerprint(&mut value, tree);
    }
    value
}

/// 递归追加一棵 tree 的低成本内容摘要。
///
/// 适用场景：文件 watcher 更新正文但节点数量不变。示例：task 文本变更 -> 长度摘要变化。
fn append_tree_fingerprint(value: &mut String, tree: &WorkflowTree) {
    value.push_str(&format!(
        "|{}:{}:{}",
        tree.repo_path.display(),
        tree.content.len(),
        tree.projects.len()
    ));
    for project in &tree.projects {
        value.push_str(&format!(
            ":{}:{}:{}",
            project.key,
            project.content.len(),
            project.tasks.len()
        ));
        for task in &project.tasks {
            value.push_str(&format!(
                ":{}:{}:{}",
                task.path.display(),
                task.desc.len(),
                workflow_step_content_len(&task.steps)
            ));
        }
    }
    for child in &tree.sub_workflows {
        append_tree_fingerprint(value, child);
    }
}

/// 汇总 step tree 的正文长度用于刷新摘要。
///
/// 适用场景：正文变化但节点数量不变。示例：修改 step desc -> fingerprint 变化。
fn workflow_step_content_len(steps: &[WorkflowStepNode]) -> usize {
    steps
        .iter()
        .map(|step| step.title.len() + step.desc.len() + workflow_step_content_len(&step.children))
        .sum()
}

/// 把主 repo 与子 repo Workflow 展平为左侧可见行。
///
/// 适用场景：绘制、键盘移动和滚动共用。示例：折叠 project -> 隐藏 task 行。
pub(super) fn workflow_rows(
    tree: &WorkflowTree,
    collapsed_projects: &BTreeSet<String>,
) -> Vec<WorkflowBrowserRow> {
    let mut rows = Vec::new();
    append_tree_rows(&mut rows, tree, collapsed_projects, false);
    for child in &tree.sub_workflows {
        append_tree_rows(&mut rows, child, collapsed_projects, true);
    }
    rows
}

/// 追加单个 repo 的 root、project 和 task 行。
///
/// 适用场景：主 repo 与子 repo 使用相同层级规则。示例：子 repo 前插入路径分隔行。
fn append_tree_rows(
    rows: &mut Vec<WorkflowBrowserRow>,
    tree: &WorkflowTree,
    collapsed_projects: &BTreeSet<String>,
    child_repo: bool,
) {
    if child_repo {
        rows.push(WorkflowBrowserRow {
            depth: 0,
            label: display_path(&tree.repo_path),
            target: None,
            project_key: None,
            copy_path: None,
            done: false,
            repo_header: true,
        });
    }
    rows.push(WorkflowBrowserRow {
        depth: 0,
        label: "ROOT  root.md".to_string(),
        target: Some(WorkflowSelectionTarget::WorkspaceRoot {
            root_path: tree.root_path.clone(),
        }),
        project_key: None,
        copy_path: Some(super::super::app_shell_ui::workflow_root_copy_path(tree)),
        done: false,
        repo_header: false,
    });
    for project in &tree.projects {
        let project_key = workflow_project_state_key(tree, &project.key);
        let collapsed = collapsed_projects.contains(&project_key);
        rows.push(WorkflowBrowserRow {
            depth: 0,
            label: format!(
                "{} PROJECT  {}",
                if collapsed { "▸" } else { "▾" },
                project.label
            ),
            target: Some(WorkflowSelectionTarget::Project {
                root_path: project.root_path.clone(),
            }),
            project_key: Some(project_key),
            copy_path: Some(super::super::app_shell_ui::workflow_project_copy_path(
                tree, project,
            )),
            done: false,
            repo_header: false,
        });
        if collapsed {
            continue;
        }
        for task in &project.tasks {
            let done = task_done(task);
            rows.push(WorkflowBrowserRow {
                depth: 1,
                label: format!("{} {}", if done { "✓" } else { "·" }, task.label),
                target: Some(WorkflowSelectionTarget::Task {
                    task_path: task.path.clone(),
                }),
                project_key: None,
                copy_path: Some(super::super::app_shell_ui::workflow_task_copy_path(
                    tree, project, task,
                )),
                done,
                repo_header: false,
            });
        }
    }
}

/// 判断左侧主树行是否对应当前选择。
///
/// 适用场景：选中 step 时仍保持其父 task 高亮。示例：Task(a) + Step(a/0) -> true。
fn workflow_row_selected(
    row_target: Option<&WorkflowSelectionTarget>,
    selected: Option<&WorkflowSelectionTarget>,
) -> bool {
    match (row_target, selected) {
        (
            Some(WorkflowSelectionTarget::Task {
                task_path: row_path,
            }),
            Some(WorkflowSelectionTarget::Step { task_path, .. }),
        ) => row_path == task_path,
        (Some(row_target), Some(selected)) => row_target == selected,
        _ => false,
    }
}

/// 查找当前选择所属的 task 及其 project。
///
/// 适用场景：task 和 step 选择都进入同一三栏工作台。示例：Step(task-a/0) -> task-a。
fn selected_task_context<'a>(
    tree: &'a WorkflowTree,
    selected: Option<&WorkflowSelectionTarget>,
) -> Option<(
    &'a WorkflowTree,
    &'a WorkflowProjectNode,
    &'a WorkflowTaskNode,
)> {
    let task_path = match selected? {
        WorkflowSelectionTarget::Task { task_path }
        | WorkflowSelectionTarget::Step { task_path, .. } => task_path,
        WorkflowSelectionTarget::WorkspaceRoot { .. } | WorkflowSelectionTarget::Project { .. } => {
            return None;
        }
    };
    workflow_trees(tree).find_map(|current| {
        current.projects.iter().find_map(|project| {
            project
                .tasks
                .iter()
                .find(|task| &task.path == task_path)
                .map(|task| (current, project, task))
        })
    })
}

/// 把 task 的嵌套 step 展平为可绘制行。
///
/// 适用场景：TUI 保留 GUI step tree 的层级，同时支持连续键盘和鼠标命中。
fn workflow_step_rows(
    tree: &WorkflowTree,
    project: &WorkflowProjectNode,
    task: &WorkflowTaskNode,
) -> Vec<WorkflowStepRow> {
    let mut rows = Vec::new();
    let base_path = super::super::app_shell_ui::workflow_task_copy_path(tree, project, task);
    append_step_rows(&mut rows, task, &task.steps, 0, &base_path);
    rows
}

/// 递归追加 step tree 行。
///
/// 适用场景：父子 step 的缩进和复制路径必须同步生成。示例：build/test -> 两级路径。
fn append_step_rows(
    rows: &mut Vec<WorkflowStepRow>,
    task: &WorkflowTaskNode,
    steps: &[WorkflowStepNode],
    depth: usize,
    parent_copy_path: &str,
) {
    for step in steps {
        let copy_path = format!("{parent_copy_path} > {}", step.title);
        rows.push(WorkflowStepRow {
            depth,
            label: format!(
                "{} {}",
                if step.checkable {
                    if step.checked { "✓" } else { "□" }
                } else {
                    "·"
                },
                step.title
            ),
            target: WorkflowSelectionTarget::Step {
                task_path: task.path.clone(),
                step_path: step.path.clone(),
            },
            copy_path: copy_path.clone(),
            checked: step.checked,
        });
        append_step_rows(
            rows,
            task,
            &step.children,
            depth.saturating_add(1),
            &copy_path,
        );
    }
}

/// 查找当前选择对应的 step。
///
/// 适用场景：右下角只渲染选中 step 自己的 Markdown。示例：[1,0] -> 第二个父 step 的首个子 step。
fn selected_step<'a>(
    task: &'a WorkflowTaskNode,
    selected: Option<&WorkflowSelectionTarget>,
) -> Option<&'a WorkflowStepNode> {
    let WorkflowSelectionTarget::Step {
        task_path,
        step_path,
    } = selected?
    else {
        return None;
    };
    if task_path != &task.path {
        return None;
    }
    workflow_step_at_path(&task.steps, step_path)
}

/// 按索引路径递归查找 step。
///
/// 适用场景：解析后的 step tree 使用 Vec 索引作为稳定帧内定位。示例：[0,1] -> 首个父节点第二个子节点。
pub(super) fn workflow_step_at_path<'a>(
    steps: &'a [WorkflowStepNode],
    path: &[usize],
) -> Option<&'a WorkflowStepNode> {
    let (first, rest) = path.split_first()?;
    let step = steps.get(*first)?;
    if rest.is_empty() {
        Some(step)
    } else {
        workflow_step_at_path(&step.children, rest)
    }
}

/// 判断 task 下所有可勾选 step 是否完成。
///
/// 适用场景：左侧 task 状态颜色。示例：无 checkbox -> false。
pub(super) fn task_done(task: &WorkflowTaskNode) -> bool {
    let (has_checkable, all_checked) = steps_done(&task.steps);
    has_checkable && all_checked
}

/// 递归汇总 step checkbox。
///
/// 适用场景：嵌套 step 也影响 task 完成态。示例：父完成、子未完成 -> false。
fn steps_done(steps: &[WorkflowStepNode]) -> (bool, bool) {
    let mut has_checkable = false;
    let mut all_checked = true;
    for step in steps {
        if step.checkable {
            has_checkable = true;
            all_checked &= step.checked;
        }
        let (child_has, child_all) = steps_done(&step.children);
        has_checkable |= child_has;
        all_checked &= child_all;
    }
    (has_checkable, all_checked)
}

/// 返回当前选择对应的完整 Markdown 文档。
///
/// 适用场景：root、project 和 task 使用同一个只读预览。示例：task -> 原始 task Markdown。
pub(super) fn selected_markdown<'a>(
    tree: &'a WorkflowTree,
    selected: Option<&WorkflowSelectionTarget>,
) -> Option<Cow<'a, str>> {
    let selected = selected?;
    for current in workflow_trees(tree) {
        match selected {
            WorkflowSelectionTarget::WorkspaceRoot { root_path }
                if &current.root_path == root_path =>
            {
                return Some(Cow::Borrowed(&current.content));
            }
            WorkflowSelectionTarget::Project { root_path } => {
                if let Some(project) = current
                    .projects
                    .iter()
                    .find(|project| &project.root_path == root_path)
                {
                    return Some(Cow::Borrowed(&project.content));
                }
            }
            WorkflowSelectionTarget::Task { task_path }
            | WorkflowSelectionTarget::Step { task_path, .. } => {
                if let Some(task) = current
                    .projects
                    .iter()
                    .flat_map(|project| &project.tasks)
                    .find(|task| &task.path == task_path)
                {
                    return match selected {
                        WorkflowSelectionTarget::Task { .. } => {
                            Some(Cow::Borrowed(task.desc.as_str()))
                        }
                        WorkflowSelectionTarget::Step { .. } => selected_step(task, Some(selected))
                            .map(|step| Cow::Borrowed(step.desc.as_str())),
                        WorkflowSelectionTarget::WorkspaceRoot { .. }
                        | WorkflowSelectionTarget::Project { .. } => None,
                    };
                }
            }
            WorkflowSelectionTarget::WorkspaceRoot { .. } => {}
        }
    }
    None
}

/// 绘制 Workflow tree 与右侧 Markdown，并生成节点鼠标命中区。
///
/// 适用场景：TUI 主视图为 Workflow。示例：右键 task -> Open/Copy path。
pub(super) fn draw_browser(
    browser: &mut WorkflowBrowser,
    frame: &mut [RenderLine],
    size: (u16, u16),
    state: Option<&WorkflowUiState>,
    hit_regions: &mut Vec<HitRegion>,
) {
    let (width, height) = size;
    let body_height = height.saturating_sub(1);
    let tree_width = (width / 3).clamp(24, 42).min(width.saturating_sub(12));
    for y in 0..body_height {
        put_char(frame, y, tree_width, '│');
    }
    let Some(state) = state else {
        set_base_text(
            frame,
            0,
            1,
            "Workflow state unavailable",
            tree_width.saturating_sub(2),
        );
        return;
    };
    let Some(tree) = state.tree.as_ref() else {
        let message = if state.loading {
            "Loading workflow..."
        } else {
            state.load_error.as_deref().unwrap_or("No workflow")
        };
        set_base_text(frame, 0, 1, message, tree_width.saturating_sub(2));
        return;
    };

    let rows = workflow_rows(tree, &browser.collapsed_projects);
    let visible = usize::from(body_height);
    browser.tree_scroll = browser.tree_scroll.min(rows.len().saturating_sub(visible));
    for (screen_index, row) in rows
        .iter()
        .skip(browser.tree_scroll)
        .take(visible)
        .enumerate()
    {
        let y = screen_index as u16;
        let text = format!("{}{}", "  ".repeat(row.depth), row.label);
        let selected = workflow_row_selected(row.target.as_ref(), browser.selected.as_ref());
        let foreground = if row.repo_header {
            (148, 163, 184)
        } else if row.done {
            (74, 222, 128)
        } else if selected {
            (255, 255, 255)
        } else {
            (203, 213, 225)
        };
        styled_overlay_text(
            frame,
            y,
            0,
            &pad_display_text(&text, tree_width),
            foreground,
            if selected {
                (37, 99, 235)
            } else {
                (11, 16, 22)
            },
            selected || row.repo_header,
        );
        let Some(target) = row.target.clone() else {
            continue;
        };
        let left = row
            .project_key
            .clone()
            .map(HitAction::ToggleWorkflowProject)
            .unwrap_or_else(|| HitAction::SelectWorkflow(target.clone()));
        hit_regions.push(HitRegion {
            rect: TuiRect {
                x: 0,
                y,
                width: tree_width,
                height: 1,
            },
            left: Some(left),
            right: row.copy_path.clone().map(|copy_path| {
                HitAction::WorkflowMenu(WorkflowMenuContext { target, copy_path })
            }),
        });
    }

    if let Some((task_tree, project, task)) = selected_task_context(tree, browser.selected.as_ref())
    {
        draw_task_workspace(
            browser,
            frame,
            width,
            body_height,
            tree_width,
            task_tree,
            project,
            task,
            hit_regions,
        );
    } else {
        let content_x = tree_width.saturating_add(2);
        let content_width = width.saturating_sub(content_x);
        let markdown = selected_markdown(tree, browser.selected.as_ref()).unwrap_or_default();
        draw_markdown_region(
            frame,
            &markdown,
            TuiRect {
                x: content_x,
                y: 0,
                width: content_width,
                height: body_height,
            },
            &mut browser.content_scroll,
        );
    }
}

/// 绘制与 GUI 一致的 task 工作台。
///
/// 适用场景：上方 task Markdown；下方左侧 step tree、右侧 step Markdown。
#[allow(clippy::too_many_arguments)]
fn draw_task_workspace(
    browser: &mut WorkflowBrowser,
    frame: &mut [RenderLine],
    width: u16,
    body_height: u16,
    tree_width: u16,
    tree: &WorkflowTree,
    project: &WorkflowProjectNode,
    task: &WorkflowTaskNode,
    hit_regions: &mut Vec<HitRegion>,
) {
    let content_x = tree_width.saturating_add(2);
    let content_width = width.saturating_sub(content_x);
    let task_height = task_panel_height(body_height);
    let split_y = task_height;
    for x in content_x..width {
        put_char(frame, split_y, x, '─');
    }
    draw_markdown_region(
        frame,
        &task.desc,
        TuiRect {
            x: content_x,
            y: 0,
            width: content_width,
            height: task_height,
        },
        &mut browser.task_scroll,
    );

    let bottom_y = split_y.saturating_add(1);
    let bottom_height = body_height.saturating_sub(bottom_y);
    let step_width = step_panel_width(content_width);
    let step_separator_x = content_x.saturating_add(step_width);
    for y in bottom_y..body_height {
        put_char(frame, y, step_separator_x, '│');
    }
    let rows = workflow_step_rows(tree, project, task);
    browser.step_tree_scroll = browser
        .step_tree_scroll
        .min(rows.len().saturating_sub(usize::from(bottom_height)));
    if rows.is_empty() {
        styled_overlay_text(
            frame,
            bottom_y,
            content_x,
            "No steps",
            (100, 116, 139),
            (11, 16, 22),
            false,
        );
    }
    for (screen_index, row) in rows
        .iter()
        .skip(browser.step_tree_scroll)
        .take(usize::from(bottom_height))
        .enumerate()
    {
        let y = bottom_y.saturating_add(screen_index as u16);
        let selected = browser.selected.as_ref() == Some(&row.target);
        let text = format!("{}{}", "  ".repeat(row.depth), row.label);
        styled_overlay_text(
            frame,
            y,
            content_x,
            &pad_display_text(&text, step_width),
            if row.checked {
                (74, 222, 128)
            } else if selected {
                (255, 255, 255)
            } else {
                (203, 213, 225)
            },
            if selected {
                (37, 99, 235)
            } else {
                (11, 16, 22)
            },
            selected,
        );
        hit_regions.push(HitRegion {
            rect: TuiRect {
                x: content_x,
                y,
                width: step_width,
                height: 1,
            },
            left: Some(HitAction::SelectWorkflow(row.target.clone())),
            right: Some(HitAction::WorkflowMenu(WorkflowMenuContext {
                target: row.target.clone(),
                copy_path: row.copy_path.clone(),
            })),
        });
    }

    let step_content_x = step_separator_x.saturating_add(2);
    let step_content_width = width.saturating_sub(step_content_x);
    let step_markdown = selected_step(task, browser.selected.as_ref())
        .map(|step| step.desc.as_str())
        .unwrap_or_default();
    draw_markdown_region(
        frame,
        step_markdown,
        TuiRect {
            x: step_content_x,
            y: bottom_y,
            width: step_content_width,
            height: bottom_height,
        },
        &mut browser.step_content_scroll,
    );
}

/// 计算 task 说明区域高度。
///
/// 适用场景：复用 GUI 的约 28% 比例，同时给下方 step 工作台保留空间。
fn task_panel_height(body_height: u16) -> u16 {
    body_height
        .saturating_mul(28)
        .checked_div(100)
        .unwrap_or(0)
        .clamp(3, body_height.saturating_sub(5).max(3))
        .min(body_height.saturating_sub(1))
}

/// 计算 step tree 宽度。
///
/// 适用场景：复用 GUI 的约 32% 比例，并防止窄终端挤没右侧 Markdown。
fn step_panel_width(content_width: u16) -> u16 {
    content_width
        .saturating_mul(32)
        .checked_div(100)
        .unwrap_or(0)
        .clamp(16, 36)
        .min(content_width.saturating_sub(8))
}

/// 在指定矩形内绘制可独立滚动的 Markdown。
///
/// 适用场景：root/project、task 和 step 复用同一轻量渲染器。
pub(super) fn draw_markdown_region(
    frame: &mut [RenderLine],
    markdown: &str,
    rect: TuiRect,
    scroll: &mut usize,
) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let palette = tui_markdown_palette();
    let lines = markdown_lines(markdown, rect.width.max(1));
    *scroll = (*scroll).min(lines.len().saturating_sub(usize::from(rect.height)));
    if lines.is_empty() {
        styled_overlay_text(
            frame,
            rect.y,
            rect.x,
            "(empty)",
            palette.muted,
            palette.background,
            false,
        );
        return;
    }
    for (screen_index, line) in lines
        .iter()
        .skip(*scroll)
        .take(usize::from(rect.height))
        .enumerate()
    {
        let (foreground, bold) = markdown_style(line.style, palette);
        styled_overlay_text(
            frame,
            rect.y.saturating_add(screen_index as u16),
            rect.x,
            &clip_text(&line.text, rect.width),
            foreground,
            palette.background,
            bold,
        );
    }
}

/// 根据指针位置返回 Workflow 独立滚动区域。
///
/// 适用场景：TUI 鼠标滚轮不应同时滚动 task 和 step。示例：右下角 -> StepContent。
pub(super) fn scroll_region(
    browser: &WorkflowBrowser,
    tree: Option<&WorkflowTree>,
    size: (u16, u16),
    column: u16,
    row: u16,
) -> WorkflowScrollRegion {
    let (width, height) = size;
    let tree_width = (width / 3).clamp(24, 42).min(width.saturating_sub(12));
    if column <= tree_width {
        return WorkflowScrollRegion::Tree;
    }
    let Some(tree) = tree else {
        return WorkflowScrollRegion::Content;
    };
    if selected_task_context(tree, browser.selected.as_ref()).is_none() {
        return WorkflowScrollRegion::Content;
    }
    let body_height = height.saturating_sub(1);
    let task_height = task_panel_height(body_height);
    if row <= task_height {
        return WorkflowScrollRegion::Task;
    }
    let content_x = tree_width.saturating_add(2);
    let content_width = width.saturating_sub(content_x);
    let separator_x = content_x.saturating_add(step_panel_width(content_width));
    if column <= separator_x {
        WorkflowScrollRegion::Steps
    } else {
        WorkflowScrollRegion::StepContent
    }
}

/// 返回 Markdown 语义样式对应的 xterm RGB。
///
/// 适用场景：右侧预览。示例：代码 -> 低亮蓝灰色，引用 -> muted。
fn markdown_style(style: MarkdownLineStyle, palette: TuiMarkdownPalette) -> ((u8, u8, u8), bool) {
    match style {
        MarkdownLineStyle::Heading(level) => (
            if level <= 2 {
                palette.heading
            } else {
                palette.subheading
            },
            true,
        ),
        MarkdownLineStyle::Quote => (palette.muted, false),
        MarkdownLineStyle::Code => (palette.code, false),
        MarkdownLineStyle::Rule => (palette.rule, false),
        MarkdownLineStyle::Body => (palette.body, false),
    }
}

/// 使用 pulldown-cmark 将 Markdown 转成按终端宽度换行的显示行。
///
/// 适用场景：只读预览，不生成 HTML。示例：链接 -> 文本和目标地址。
fn markdown_lines(markdown: &str, width: u16) -> Vec<MarkdownLine> {
    let mut options = Options::empty();
    options
        .insert(Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut style = MarkdownLineStyle::Body;
    let mut lists = Vec::<MarkdownListState>::new();
    let mut links = Vec::<String>::new();
    let mut quote_depth = 0_usize;
    let mut in_code = false;

    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                flush_markdown_line(&mut lines, &mut current, style, width);
                style = MarkdownLineStyle::Heading(heading_level(level));
            }
            Event::End(TagEnd::Heading(_)) => {
                flush_markdown_line(&mut lines, &mut current, style, width);
                style = MarkdownLineStyle::Body;
            }
            Event::Start(Tag::Paragraph) => {
                if quote_depth > 0 && current.is_empty() {
                    current.push_str(&"│ ".repeat(quote_depth));
                    style = MarkdownLineStyle::Quote;
                }
            }
            Event::End(TagEnd::Paragraph) => {
                flush_markdown_line(&mut lines, &mut current, style, width);
                style = if in_code {
                    MarkdownLineStyle::Code
                } else {
                    MarkdownLineStyle::Body
                };
            }
            Event::Start(Tag::BlockQuote(_)) => quote_depth = quote_depth.saturating_add(1),
            Event::End(TagEnd::BlockQuote(_)) => quote_depth = quote_depth.saturating_sub(1),
            Event::Start(Tag::CodeBlock(_)) => {
                flush_markdown_line(&mut lines, &mut current, style, width);
                in_code = true;
                style = MarkdownLineStyle::Code;
            }
            Event::End(TagEnd::CodeBlock) => {
                flush_markdown_line(&mut lines, &mut current, style, width);
                in_code = false;
                style = MarkdownLineStyle::Body;
            }
            Event::Start(Tag::List(start)) => {
                lists.push(MarkdownListState { next_number: start });
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
                flush_markdown_line(&mut lines, &mut current, style, width);
            }
            Event::Start(Tag::Item) => {
                flush_markdown_line(&mut lines, &mut current, style, width);
                current.push_str(&"  ".repeat(lists.len().saturating_sub(1)));
                let marker = lists
                    .last_mut()
                    .and_then(|list| {
                        let number = list.next_number?;
                        list.next_number = Some(number.saturating_add(1));
                        Some(format!("{number}. "))
                    })
                    .unwrap_or_else(|| "• ".to_string());
                current.push_str(&marker);
            }
            Event::End(TagEnd::Item) => {
                flush_markdown_line(&mut lines, &mut current, style, width);
            }
            Event::Start(Tag::Link { dest_url, .. }) => links.push(dest_url.into_string()),
            Event::End(TagEnd::Link) => {
                if let Some(url) = links.pop()
                    && !url.is_empty()
                {
                    current.push_str(" (");
                    current.push_str(&url);
                    current.push(')');
                }
            }
            Event::Text(text) => {
                append_markdown_text(&mut lines, &mut current, &text, style, width, in_code)
            }
            Event::Code(text) => {
                current.push(char::from(96));
                current.push_str(&text);
                current.push(char::from(96));
            }
            Event::InlineMath(text) => {
                current.push('$');
                current.push_str(&text);
                current.push('$');
            }
            Event::DisplayMath(text) => {
                flush_markdown_line(&mut lines, &mut current, style, width);
                current.push_str(&text);
                flush_markdown_line(&mut lines, &mut current, MarkdownLineStyle::Code, width);
            }
            Event::SoftBreak | Event::HardBreak => {
                flush_markdown_line(&mut lines, &mut current, style, width);
            }
            Event::Rule => {
                flush_markdown_line(&mut lines, &mut current, style, width);
                lines.push(MarkdownLine {
                    text: "─".repeat(usize::from(width)),
                    style: MarkdownLineStyle::Rule,
                });
            }
            Event::TaskListMarker(checked) => {
                current.push_str(if checked { "[x] " } else { "[ ] " });
            }
            Event::FootnoteReference(label) => {
                current.push_str("[^");
                current.push_str(&label);
                current.push(']');
            }
            Event::Html(text) | Event::InlineHtml(text) => current.push_str(&text),
            Event::Start(_) | Event::End(_) => {}
        }
    }
    flush_markdown_line(&mut lines, &mut current, style, width);
    lines
}

/// 把 HeadingLevel 转成紧凑数字。
///
/// 适用场景：标题色阶。示例：H3 -> 3。
fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// 追加可能含换行的 Markdown 文本事件。
///
/// 适用场景：代码块 Text 可以一次携带多行。示例：两行文本 -> 两条 Code 行。
fn append_markdown_text(
    lines: &mut Vec<MarkdownLine>,
    current: &mut String,
    text: &str,
    style: MarkdownLineStyle,
    width: u16,
    preserve_newlines: bool,
) {
    if !preserve_newlines {
        current.push_str(text);
        return;
    }
    let mut parts = text.split('\n').peekable();
    while let Some(part) = parts.next() {
        current.push_str(part);
        if parts.peek().is_some() {
            flush_markdown_line(lines, current, style, width);
        }
    }
}

/// 把当前逻辑行按显示宽度换行并清空缓冲。
///
/// 适用场景：中文宽字符与 ASCII 混排。示例：80 列段落 -> 多个 80 列以内行。
fn flush_markdown_line(
    lines: &mut Vec<MarkdownLine>,
    current: &mut String,
    style: MarkdownLineStyle,
    width: u16,
) {
    if current.is_empty() {
        return;
    }
    for text in wrap_display_text(current, width.max(1)) {
        lines.push(MarkdownLine { text, style });
    }
    current.clear();
}

/// 按终端显示列宽拆分文本。
///
/// 适用场景：避免中文或宽字符越过右边界。示例：宽度 4 的中文文本自动折行。
fn wrap_display_text(text: &str, width: u16) -> Vec<String> {
    let limit = usize::from(width.max(1));
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut used = 0_usize;
    for ch in text.chars() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used > 0 && used.saturating_add(char_width) > limit {
            lines.push(current);
            current = String::new();
            used = 0;
        }
        current.push(ch);
        used = used.saturating_add(char_width);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// 返回 Path 的稳定终端文本。
///
/// 适用场景：子 repo 分隔行。示例：services/api 原样显示。
fn display_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

//! TUI Finder 风格 Column View。
//!
//! 导航列只从已有 outline/workflow 快照派生，不在绘制路径扫描文件系统。

use super::*;

/// 同步 Workflow 当前可见的 Markdown 工作台。
///
/// 适用场景：tree 刷新与选择变化后准备纯 render 状态。
pub(super) fn sync_workflow_workbenches(
    browser: &WorkflowBrowser,
    state: Option<&WorkflowUiState>,
    workbenches: &mut MarkdownWorkbenches,
    editable: bool,
    workspace_root: &Path,
) -> bool {
    let Some(tree) = state.and_then(|state| state.tree.as_ref()) else {
        return false;
    };
    let Some(target) = browser.selected.as_ref() else {
        return false;
    };
    let mut changed = false;
    let selected_tree = selected_workflow_tree(tree, Some(target)).unwrap_or(tree);
    if let Some(task) = selected_workflow_task(selected_tree, Some(target)) {
        let task_editor = state
            .and_then(|state| state.task_editor.as_ref())
            .filter(|editor| editor.task_path == task.path);
        changed |= workbenches.sync(
            "workflow-task".to_string(),
            MarkdownWorkbenchTarget::Workflow(WorkflowSelectionTarget::Task {
                task_path: task.path.clone(),
            }),
            task_editor.map_or(task.desc.as_str(), |editor| editor.task_text.as_str()),
            task_editor.map_or(task.desc.as_str(), |editor| editor.saved_task_text.as_str()),
            editable,
            Some(workspace_root.join(&task.path)),
            task_editor.and_then(|editor| editor.save_error.as_deref()),
        );
        if let WorkflowSelectionTarget::Step { step_path, .. } = target
            && let Some(step) = workflow::workflow_step_at_path(&task.steps, step_path)
        {
            let step_editor = state
                .and_then(|state| state.editor.as_ref())
                .filter(|editor| editor.task_path == task.path && editor.step_path == *step_path);
            changed |= workbenches.sync(
                "workflow-step".to_string(),
                MarkdownWorkbenchTarget::Workflow(target.clone()),
                step_editor.map_or(step.desc.as_str(), |editor| editor.step_text.as_str()),
                step_editor.map_or(step.desc.as_str(), |editor| editor.saved_step_text.as_str()),
                editable,
                Some(workspace_root.join(&task.path)),
                step_editor.and_then(|editor| editor.save_error.as_deref()),
            );
        }
        return changed;
    }
    let path = match target {
        WorkflowSelectionTarget::WorkspaceRoot { root_path }
        | WorkflowSelectionTarget::Project { root_path } => root_path.clone(),
        WorkflowSelectionTarget::Task { .. } | WorkflowSelectionTarget::Step { .. } => {
            return changed;
        }
    };
    let markdown = workflow::selected_markdown(tree, Some(target)).unwrap_or_default();
    changed |= workbenches.sync(
        "workflow-primary".to_string(),
        MarkdownWorkbenchTarget::Document(path.clone()),
        &markdown,
        &markdown,
        editable,
        Some(workspace_root.join(&path)),
        None,
    );
    changed
}

/// 同步 Dir 当前文件的 Markdown 工作台。
///
/// 适用场景：DocumentLoaded/DocumentSaved 到达后更新草稿基线。
pub(super) fn sync_directory_workbench(
    browser: &DirectoryBrowser,
    document: Option<&DocumentState>,
    workbenches: &mut MarkdownWorkbenches,
) -> bool {
    let Some(path) = browser.selected_file.as_ref() else {
        return false;
    };
    let target = MarkdownWorkbenchTarget::Document(path.clone());
    let Some(document) = document.filter(|document| document.path.as_ref() == Some(path)) else {
        return workbenches.remove_clean_target("directory", &target);
    };
    workbenches.sync(
        "directory".to_string(),
        target,
        &document.text,
        &document.saved_text,
        true,
        document.absolute_path.clone(),
        document.save_error.as_deref(),
    )
}

/// 目录 Column View 的帧间状态。
///
/// 适用场景：Dir 一级视图切换目录。示例：root -> docs -> guide.md。
#[derive(Debug, Clone, Default)]
pub(super) struct DirectoryBrowser {
    /// 当前状态所属 workspace。
    pub workspace_index: Option<usize>,
    /// 从 root 开始的已选目录 key。
    pub selected_dirs: Vec<PathBuf>,
    /// 当前打开的 Markdown 路径。
    pub selected_file: Option<PathBuf>,
    /// 各导航列首个可见行。
    pub column_scrolls: Vec<usize>,
    /// Markdown 工作台首个可见行。
    pub content_scroll: usize,
}

/// Column View 一行的完成统计。
///
/// 适用场景：Repo、Project、Task 行背景进度。示例：3/5 -> 60%。
#[derive(Debug, Clone, Copy, Default)]
struct Progress {
    /// 已完成 checkbox 数。
    done: usize,
    /// checkbox 总数。
    total: usize,
}

impl Progress {
    /// 返回 0 到 1 的完成比例。
    ///
    /// 适用场景：计算背景填充字符数。示例：1/4 -> 0.25。
    fn ratio(self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.done as f32 / self.total as f32
        }
    }

    /// 判断存在 checkbox 且全部完成。
    ///
    /// 适用场景：整行绿色。示例：4/4 -> true。
    fn complete(self) -> bool {
        self.total > 0 && self.done == self.total
    }
}

/// 绘制 Workflow 的 Repo|Project|Task|Step|工作台 Column View。
///
/// 适用场景：TUI Workflow 一级视图。示例：点击 task 后右侧显示上下工作台。
pub(super) fn draw_workflow_columns(
    browser: &mut WorkflowBrowser,
    workbenches: &mut MarkdownWorkbenches,
    frame: &mut [RenderLine],
    size: (u16, u16),
    state: Option<&WorkflowUiState>,
    workspace: Option<&WorkspaceViewData>,
    hit_regions: &mut Vec<HitRegion>,
    images: &local_image::LocalImageStore,
) {
    let body_height = size.1.saturating_sub(1);
    let Some(tree) = state.and_then(|state| state.tree.as_ref()) else {
        let message = state
            .and_then(|state| state.load_error.as_deref())
            .unwrap_or("Loading workflow...");
        set_base_text(frame, 0, 1, message, size.0.saturating_sub(2));
        return;
    };
    let selected_tree = selected_workflow_tree(tree, browser.selected.as_ref()).unwrap_or(tree);
    let selected_project = selected_workflow_project(selected_tree, browser.selected.as_ref());
    let selected_task = selected_workflow_task(selected_tree, browser.selected.as_ref());
    let column_width = workflow_column_width(size.0);
    let mut x = 0_u16;

    let repos = workflow_trees_for_display(tree)
        .into_iter()
        .map(|repo| {
            let label = if repo.repo_path.as_os_str().is_empty() {
                "Root".to_string()
            } else {
                repo.repo_path.to_string_lossy().replace('\\', "/")
            };
            (
                label,
                WorkflowSelectionTarget::WorkspaceRoot {
                    root_path: repo.root_path.clone(),
                },
                workflow_tree_progress(repo),
                repo.root_path.to_string_lossy().to_string(),
            )
        })
        .collect::<Vec<_>>();
    draw_workflow_column(
        frame,
        hit_regions,
        x,
        column_width,
        body_height,
        "Repo",
        &repos,
        browser.selected.as_ref(),
        tree,
        workspace,
    );
    x = x.saturating_add(column_width).saturating_add(1);

    let projects = workflow_projects_for_display(selected_tree)
        .into_iter()
        .map(|project| {
            (
                project.label.clone(),
                WorkflowSelectionTarget::Project {
                    root_path: project.root_path.clone(),
                },
                workflow_project_progress(project),
                project.root_path.to_string_lossy().to_string(),
            )
        })
        .collect::<Vec<_>>();
    draw_workflow_column(
        frame,
        hit_regions,
        x,
        column_width,
        body_height,
        "Project",
        &projects,
        browser.selected.as_ref(),
        tree,
        workspace,
    );
    x = x.saturating_add(column_width).saturating_add(1);

    if let Some(project) = selected_project {
        let tasks = workflow_tasks_for_display(project)
            .into_iter()
            .map(|task| {
                (
                    task.label.clone(),
                    WorkflowSelectionTarget::Task {
                        task_path: task.path.clone(),
                    },
                    workflow_task_progress(task),
                    task.path.to_string_lossy().to_string(),
                )
            })
            .collect::<Vec<_>>();
        draw_workflow_column(
            frame,
            hit_regions,
            x,
            column_width,
            body_height,
            "Task",
            &tasks,
            browser.selected.as_ref(),
            tree,
            workspace,
        );
    } else {
        draw_empty_column(frame, x, column_width, body_height, "Task");
    }
    x = x.saturating_add(column_width).saturating_add(1);

    if let Some(task) = selected_task {
        draw_step_column(
            frame,
            hit_regions,
            x,
            column_width,
            body_height,
            task,
            browser.selected.as_ref(),
            tree,
            workspace,
        );
    } else {
        draw_empty_column(frame, x, column_width, body_height, "Step");
    }
    x = x.saturating_add(column_width).saturating_add(1);
    draw_workflow_workbench(
        browser,
        workbenches,
        frame,
        selected_task,
        x,
        size.0,
        body_height,
        hit_regions,
        images,
    );
}

/// 返回 Workflow 四个导航列之后的 Agent/Markdown 工作台矩形。
///
/// 适用场景：TUI 需要把 Workflow Agent terminal 精确覆盖到既有工作台。例：120 列 -> 余下区域。
pub(super) fn workflow_workbench_rect(size: (u16, u16)) -> TuiRect {
    let column_width = workflow_column_width(size.0);
    let x = column_width.saturating_add(1).saturating_mul(4);
    TuiRect {
        x,
        y: 0,
        width: size.0.saturating_sub(x),
        height: size.1.saturating_sub(1),
    }
}

/// 绘制 Dir 的多层目录列和 Markdown 工作台。
///
/// 适用场景：TUI Dir 一级视图。示例：点击目录产生右侧下一列。
pub(super) fn draw_directory_columns(
    browser: &mut DirectoryBrowser,
    workbenches: &mut MarkdownWorkbenches,
    frame: &mut [RenderLine],
    size: (u16, u16),
    workspace_index: usize,
    workspace: Option<&WorkspaceViewData>,
    document: Option<&DocumentState>,
    hit_regions: &mut Vec<HitRegion>,
    images: &local_image::LocalImageStore,
) {
    if browser.workspace_index != Some(workspace_index) {
        *browser = DirectoryBrowser {
            workspace_index: Some(workspace_index),
            ..DirectoryBrowser::default()
        };
    }
    let body_height = size.1.saturating_sub(1);
    let Some(workspace) = workspace else {
        set_base_text(
            frame,
            0,
            1,
            "Workspace unavailable",
            size.0.saturating_sub(2),
        );
        return;
    };
    let column_width = directory_column_width(size.0);
    let mut columns = Vec::<&[OutlineNode]>::new();
    columns.push(&workspace.outline);
    let mut nodes = workspace.outline.as_slice();
    for selected in &browser.selected_dirs {
        let Some(node) = nodes
            .iter()
            .find(|node| outline_node_key(node) == Some(selected))
        else {
            break;
        };
        nodes = outline_node_children(node).unwrap_or_default();
        columns.push(nodes);
    }
    let max_columns = ((size.0.saturating_sub(36)) / column_width.max(1)).clamp(1, 5) as usize;
    let visible_columns = columns.len().min(max_columns);
    for (column_index, nodes) in columns.iter().take(visible_columns).enumerate() {
        let x = u16::try_from(column_index)
            .unwrap_or(u16::MAX)
            .saturating_mul(column_width.saturating_add(1));
        draw_directory_column(
            frame,
            hit_regions,
            x,
            column_width,
            body_height,
            column_index,
            nodes,
            browser,
        );
    }
    let content_x = u16::try_from(visible_columns)
        .unwrap_or(u16::MAX)
        .saturating_mul(column_width.saturating_add(1));
    let _ = document;
    draw_markdown_workbench(
        workbenches,
        "directory",
        frame,
        TuiRect {
            x: content_x,
            y: 0,
            width: size.0.saturating_sub(content_x),
            height: body_height,
        },
        hit_regions,
        images,
    );
}

/// 绘制单个 Workflow 导航列。
///
/// 适用场景：Repo、Project、Task 共用进度背景。示例：2/5 在行尾显示统计。
#[allow(clippy::too_many_arguments)]
fn draw_workflow_column(
    frame: &mut [RenderLine],
    hit_regions: &mut Vec<HitRegion>,
    x: u16,
    width: u16,
    height: u16,
    title: &str,
    rows: &[(String, WorkflowSelectionTarget, Progress, String)],
    selected: Option<&WorkflowSelectionTarget>,
    tree: &WorkflowTree,
    workspace: Option<&WorkspaceViewData>,
) {
    draw_column_chrome(frame, x, width, height, title);
    for (index, (label, target, progress, copy_path)) in rows
        .iter()
        .take(usize::from(height.saturating_sub(1)))
        .enumerate()
    {
        let y = index as u16 + 1;
        let is_selected = workflow_column_row_selected(tree, target, selected);
        draw_progress_row(
            frame,
            y,
            x,
            width,
            label,
            *progress,
            is_selected,
            workflow_selection_is_busy(tree, workspace, target),
        );
        hit_regions.push(HitRegion {
            rect: TuiRect {
                x,
                y,
                width,
                height: 1,
            },
            left: Some(HitAction::SelectWorkflow(target.clone())),
            right: Some(HitAction::WorkflowMenu(WorkflowMenuContext {
                target: target.clone(),
                copy_path: copy_path.clone(),
            })),
        });
    }
}

/// 绘制 Step 列，step 只表达自身完成态。
///
/// 适用场景：选中 task 后显示扁平 step 列。示例：完成项整行深绿。
fn draw_step_column(
    frame: &mut [RenderLine],
    hit_regions: &mut Vec<HitRegion>,
    x: u16,
    width: u16,
    height: u16,
    task: &WorkflowTaskNode,
    selected: Option<&WorkflowSelectionTarget>,
    tree: &WorkflowTree,
    workspace: Option<&WorkspaceViewData>,
) {
    draw_column_chrome(frame, x, width, height, "Step");
    let mut rows = Vec::new();
    append_step_rows(&mut rows, &task.path, &task.steps, 0);
    for (index, (label, target, checked, depth)) in rows
        .into_iter()
        .take(usize::from(height.saturating_sub(1)))
        .enumerate()
    {
        let y = index as u16 + 1;
        let text = format!(
            "{}{} {}",
            "  ".repeat(depth),
            if checked { "x" } else { "·" },
            label
        );
        let selected = selected == Some(&target);
        let text = if workflow_selection_is_busy(tree, workspace, &target) {
            format!("{} Busy", clip_text(&text, width.saturating_sub(5)))
        } else {
            text
        };
        styled_overlay_text(
            frame,
            y,
            x,
            &pad_display_text(&text, width),
            if selected {
                (255, 255, 255)
            } else {
                (203, 213, 225)
            },
            if selected {
                (37, 99, 235)
            } else if checked {
                (20, 83, 45)
            } else {
                (11, 16, 22)
            },
            selected || checked,
        );
        hit_regions.push(HitRegion {
            rect: TuiRect {
                x,
                y,
                width,
                height: 1,
            },
            left: Some(HitAction::SelectWorkflow(target.clone())),
            right: Some(HitAction::WorkflowMenu(WorkflowMenuContext {
                copy_path: workflow_target_copy_path(&target),
                target,
            })),
        });
    }
}

/// 递归展平 step 列。
///
/// 适用场景：嵌套 step 在单列中保留缩进。示例：父[0]子[1] -> path [0,1]。
fn append_step_rows(
    rows: &mut Vec<(String, WorkflowSelectionTarget, bool, usize)>,
    task_path: &Path,
    steps: &[WorkflowStepNode],
    depth: usize,
) {
    for step in workflow_steps_for_display(steps) {
        rows.push((
            step.title.clone(),
            WorkflowSelectionTarget::Step {
                task_path: task_path.to_path_buf(),
                step_path: step.path.clone(),
            },
            step.checked,
            depth,
        ));
        append_step_rows(rows, task_path, &step.children, depth + 1);
    }
}

/// 绘制 Workflow 最右 Markdown 工作台。
///
/// 适用场景：root/project 全屏预览；task 保持上下 task/step 布局。
fn draw_workflow_workbench(
    browser: &mut WorkflowBrowser,
    workbenches: &mut MarkdownWorkbenches,
    frame: &mut [RenderLine],
    task: Option<&WorkflowTaskNode>,
    x: u16,
    width: u16,
    height: u16,
    hit_regions: &mut Vec<HitRegion>,
    images: &local_image::LocalImageStore,
) {
    let region_width = width.saturating_sub(x);
    if let Some(task) = task {
        let top_height = height.saturating_mul(45) / 100;
        draw_markdown_workbench(
            workbenches,
            "workflow-task",
            frame,
            TuiRect {
                x,
                y: 0,
                width: region_width,
                height: top_height,
            },
            hit_regions,
            images,
        );
        let separator = top_height;
        for column in x..width {
            put_char(frame, separator, column, '─');
        }
        let step = match browser.selected.as_ref() {
            Some(WorkflowSelectionTarget::Step { step_path, .. }) => {
                workflow::workflow_step_at_path(&task.steps, step_path)
            }
            _ => None,
        };
        if step.is_some() {
            draw_markdown_workbench(
                workbenches,
                "workflow-step",
                frame,
                TuiRect {
                    x,
                    y: separator.saturating_add(1),
                    width: region_width,
                    height: height.saturating_sub(separator.saturating_add(1)),
                },
                hit_regions,
                images,
            );
        }
    } else {
        draw_markdown_workbench(
            workbenches,
            "workflow-primary",
            frame,
            TuiRect {
                x,
                y: 0,
                width: region_width,
                height,
            },
            hit_regions,
            images,
        );
    }
}

/// 绘制目录的一列。
///
/// 适用场景：root/folder/file 使用同一行命中逻辑。
#[allow(clippy::too_many_arguments)]
fn draw_directory_column(
    frame: &mut [RenderLine],
    hit_regions: &mut Vec<HitRegion>,
    x: u16,
    width: u16,
    height: u16,
    depth: usize,
    nodes: &[OutlineNode],
    browser: &DirectoryBrowser,
) {
    draw_column_chrome(
        frame,
        x,
        width,
        height,
        if depth == 0 { "Root" } else { "Folder" },
    );
    for (index, node) in nodes
        .iter()
        .take(usize::from(height.saturating_sub(1)))
        .enumerate()
    {
        let y = index as u16 + 1;
        let (label, selected, action) = match node {
            OutlineNode::Root { key, label, .. } | OutlineNode::Dir { key, label, .. } => (
                format!("{} >", label),
                browser.selected_dirs.get(depth) == Some(key),
                HitAction::SelectDirectory {
                    depth,
                    key: key.clone(),
                },
            ),
            OutlineNode::File { path, label } => (
                label.clone(),
                browser.selected_file.as_ref() == Some(path),
                HitAction::OpenDirectoryMarkdown(path.clone()),
            ),
        };
        styled_overlay_text(
            frame,
            y,
            x,
            &pad_display_text(&label, width),
            if selected {
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
                x,
                y,
                width,
                height: 1,
            },
            left: Some(action),
            right: None,
        });
    }
}

/// 绘制列标题和右侧分隔线。
///
/// 适用场景：所有 Column View 导航列。示例：标题使用蓝色强调。
fn draw_column_chrome(frame: &mut [RenderLine], x: u16, width: u16, height: u16, title: &str) {
    styled_overlay_text(
        frame,
        0,
        x,
        &pad_display_text(title, width),
        (147, 197, 253),
        (15, 23, 42),
        true,
    );
    let separator_x = x.saturating_add(width);
    for y in 0..height {
        put_char(frame, y, separator_x, '│');
    }
}

/// 绘制没有可选内容的列。
///
/// 适用场景：尚未选择上级节点。示例：未选 project -> 空 Task 列。
fn draw_empty_column(frame: &mut [RenderLine], x: u16, width: u16, height: u16, title: &str) {
    draw_column_chrome(frame, x, width, height, title);
    styled_overlay_text(
        frame,
        1,
        x,
        &pad_display_text("-", width),
        (71, 85, 105),
        (11, 16, 22),
        false,
    );
}

/// 绘制带背景进度的单行。
///
/// 适用场景：非 Step 导航项。示例：3/4 -> 前 75% 深绿背景。
fn draw_progress_row(
    frame: &mut [RenderLine],
    y: u16,
    x: u16,
    width: u16,
    label: &str,
    progress: Progress,
    selected: bool,
    busy: bool,
) {
    let count = if busy {
        Some("Busy".to_string())
    } else {
        (progress.total > 0 && !progress.complete())
            .then(|| format!("{}/{}", progress.done, progress.total))
    };
    let count_width = count.as_deref().map(display_width).unwrap_or_default();
    let label_width = usize::from(width).saturating_sub(count_width.saturating_add(1));
    let mut text = clip_text(label, label_width as u16);
    let used = display_width(&text);
    text.push_str(&" ".repeat(label_width.saturating_sub(used)));
    if let Some(count) = count {
        text.push(' ');
        text.push_str(&count);
    } else {
        text.push(' ');
    }
    let text = pad_display_text(&text, width);
    styled_overlay_text(
        frame,
        y,
        x,
        &text,
        if selected {
            (255, 255, 255)
        } else {
            (203, 213, 225)
        },
        if selected {
            (30, 64, 175)
        } else {
            (11, 16, 22)
        },
        selected,
    );
    let progress_width = if progress.complete() {
        width
    } else {
        (progress.ratio() * f32::from(width)).round() as u16
    };
    if progress_width > 0 && !selected {
        let prefix = clip_text(&text, progress_width);
        styled_overlay_text(
            frame,
            y,
            x,
            &prefix,
            if selected {
                (255, 255, 255)
            } else {
                (220, 252, 231)
            },
            if progress.complete() {
                (21, 128, 61)
            } else {
                (20, 83, 45)
            },
            true,
        );
    }
}

/// 判断 Repo、Project 或 Task 行是否位于当前选择路径上。
///
/// 适用场景：选中 Step 时父级三列仍清晰高亮。示例：Step(a/0) -> Repo、Project、Task(a) 都选中。
fn workflow_column_row_selected(
    root: &WorkflowTree,
    row: &WorkflowSelectionTarget,
    selected: Option<&WorkflowSelectionTarget>,
) -> bool {
    match row {
        WorkflowSelectionTarget::WorkspaceRoot { root_path } => {
            selected_workflow_tree(root, selected).is_some_and(|tree| &tree.root_path == root_path)
        }
        WorkflowSelectionTarget::Project { root_path } => workflow_trees(root)
            .find_map(|tree| selected_workflow_project(tree, selected))
            .is_some_and(|project| &project.root_path == root_path),
        WorkflowSelectionTarget::Task { task_path } => workflow_trees(root)
            .find_map(|tree| selected_workflow_task(tree, selected))
            .is_some_and(|task| &task.path == task_path),
        WorkflowSelectionTarget::Step { .. } => selected == Some(row),
    }
}

/// 判断 Workflow 目标自身或后代是否存在 Busy Agent。
///
/// 适用场景：TUI Repo/Project/Task/Step 行冒泡状态。例：Step Busy -> 上三级均显示 Busy。
fn workflow_selection_is_busy(
    tree: &WorkflowTree,
    workspace: Option<&WorkspaceViewData>,
    selection: &WorkflowSelectionTarget,
) -> bool {
    let Some(binding) = crate::gui::workflow::workflow_agent_binding(tree, selection) else {
        return false;
    };
    workspace
        .into_iter()
        .flat_map(|workspace| workspace.subagents.iter())
        .filter(|agent| agent.activity == WorkspaceActivity::Busy)
        .filter_map(|agent| agent.workflow_target.as_ref())
        .any(|busy| {
            if busy.repo_path != binding.target.repo_path || busy.repo_id != binding.target.repo_id
            {
                return false;
            }
            match binding.target.kind {
                data::WorkflowAgentTargetKind::Repo => true,
                data::WorkflowAgentTargetKind::Project => {
                    busy.project_id == binding.target.project_id
                }
                data::WorkflowAgentTargetKind::Task => {
                    busy.project_id == binding.target.project_id
                        && busy.task_id == binding.target.task_id
                }
                data::WorkflowAgentTargetKind::Step => busy == &binding.target,
            }
        })
}

/// 汇总一棵 repo workflow 的 checkbox 进度。
///
/// 适用场景：Repo 列。示例：跨 project 汇总所有 task step。
fn workflow_tree_progress(tree: &WorkflowTree) -> Progress {
    tree.projects
        .iter()
        .fold(Progress::default(), |mut total, project| {
            let value = workflow_project_progress(project);
            total.done += value.done;
            total.total += value.total;
            total
        })
}

/// 汇总 project 的 checkbox 进度。
///
/// 适用场景：Project 列。示例：多个 task 合并统计。
fn workflow_project_progress(project: &WorkflowProjectNode) -> Progress {
    project
        .tasks
        .iter()
        .fold(Progress::default(), |mut total, task| {
            let value = workflow_task_progress(task);
            total.done += value.done;
            total.total += value.total;
            total
        })
}

/// 汇总 task 的递归 checkbox 进度。
///
/// 适用场景：Task 列。示例：父子 step 都计入统计。
fn workflow_task_progress(task: &WorkflowTaskNode) -> Progress {
    workflow_steps_progress(&task.steps)
}

/// 递归汇总 step checkbox。
///
/// 适用场景：嵌套 step。示例：一个父项和两个子项 -> total 3。
fn workflow_steps_progress(steps: &[WorkflowStepNode]) -> Progress {
    let mut result = Progress::default();
    for step in steps {
        if step.checkable {
            result.total += 1;
            result.done += usize::from(step.checked);
        }
        let children = workflow_steps_progress(&step.children);
        result.done += children.done;
        result.total += children.total;
    }
    result
}

/// 按当前 target 定位所属 workflow tree。
///
/// 适用场景：选择子 repo 节点后保持正确列上下文。
fn selected_workflow_tree<'a>(
    root: &'a WorkflowTree,
    target: Option<&WorkflowSelectionTarget>,
) -> Option<&'a WorkflowTree> {
    let target = target?;
    workflow_trees(root).find(|tree| workflow_target_in_tree(tree, target))
}

/// 判断 target 是否属于指定 workflow tree。
///
/// 适用场景：跨 repo 路径定位。示例：task path 匹配当前 tree。
fn workflow_target_in_tree(tree: &WorkflowTree, target: &WorkflowSelectionTarget) -> bool {
    match target {
        WorkflowSelectionTarget::WorkspaceRoot { root_path } => &tree.root_path == root_path,
        WorkflowSelectionTarget::Project { root_path } => tree
            .projects
            .iter()
            .any(|project| &project.root_path == root_path),
        WorkflowSelectionTarget::Task { task_path }
        | WorkflowSelectionTarget::Step { task_path, .. } => tree
            .projects
            .iter()
            .flat_map(|project| &project.tasks)
            .any(|task| &task.path == task_path),
    }
}

/// 按当前 target 定位 project。
///
/// 适用场景：生成 Task 列。示例：Step target -> 所属 project。
fn selected_workflow_project<'a>(
    tree: &'a WorkflowTree,
    target: Option<&WorkflowSelectionTarget>,
) -> Option<&'a WorkflowProjectNode> {
    let target = target?;
    tree.projects.iter().find(|project| match target {
        WorkflowSelectionTarget::Project { root_path } => &project.root_path == root_path,
        WorkflowSelectionTarget::Task { task_path }
        | WorkflowSelectionTarget::Step { task_path, .. } => {
            project.tasks.iter().any(|task| &task.path == task_path)
        }
        WorkflowSelectionTarget::WorkspaceRoot { .. } => false,
    })
}

/// 按当前 target 定位 task。
///
/// 适用场景：生成 Step 列和上下工作台。示例：Step target -> 所属 task。
fn selected_workflow_task<'a>(
    tree: &'a WorkflowTree,
    target: Option<&WorkflowSelectionTarget>,
) -> Option<&'a WorkflowTaskNode> {
    let task_path = match target? {
        WorkflowSelectionTarget::Task { task_path }
        | WorkflowSelectionTarget::Step { task_path, .. } => task_path,
        WorkflowSelectionTarget::WorkspaceRoot { .. } | WorkflowSelectionTarget::Project { .. } => {
            return None;
        }
    };
    tree.projects
        .iter()
        .flat_map(|project| &project.tasks)
        .find(|task| &task.path == task_path)
}

/// 返回 outline node 的稳定 key。
///
/// 适用场景：Column View 选中目录。示例：File -> None。
fn outline_node_key(node: &OutlineNode) -> Option<&PathBuf> {
    match node {
        OutlineNode::Root { key, .. } | OutlineNode::Dir { key, .. } => Some(key),
        OutlineNode::File { .. } => None,
    }
}

/// 返回 outline 目录节点的已加载 children。
///
/// 适用场景：不在 render 中扫描目录。示例：File -> None。
fn outline_node_children(node: &OutlineNode) -> Option<&[OutlineNode]> {
    match node {
        OutlineNode::Root { children, .. } | OutlineNode::Dir { children, .. } => Some(children),
        OutlineNode::File { .. } => None,
    }
}

/// 返回 Workflow target 的可复制逻辑路径。
///
/// 适用场景：Step 右键菜单。示例：task.md#0.1。
fn workflow_target_copy_path(target: &WorkflowSelectionTarget) -> String {
    match target {
        WorkflowSelectionTarget::WorkspaceRoot { root_path }
        | WorkflowSelectionTarget::Project { root_path } => root_path.to_string_lossy().to_string(),
        WorkflowSelectionTarget::Task { task_path } => task_path.to_string_lossy().to_string(),
        WorkflowSelectionTarget::Step {
            task_path,
            step_path,
        } => format!(
            "{}#{}",
            task_path.display(),
            step_path
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(".")
        ),
    }
}

/// 根据终端宽度计算 Workflow 导航列宽。
///
/// 适用场景：保证四列后仍给工作台留空间。示例：160 -> 22。
fn workflow_column_width(width: u16) -> u16 {
    (width.saturating_sub(36) / 4).clamp(14, 24)
}

/// 根据终端宽度计算目录导航列宽。
///
/// 适用场景：多层目录最多显示五列。示例：120 -> 20。
fn directory_column_width(width: u16) -> u16 {
    (width.saturating_sub(36) / 4).clamp(14, 24)
}

use crate::gui::data::{self, AppLanguage, OutlineNode};
use crate::gui::i18n;
use crate::gui::theme;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, RichText, ScrollArea, Sense, Stroke, Ui, Vec2,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// outline 行为向 app 层返回的动作。
pub(crate) enum OutlineAction {
    /// 在编辑器中打开 Markdown。
    OpenEditor(PathBuf),
    /// 在预览中打开 Markdown。
    OpenPreview(PathBuf),
    /// 在目录下创建 Markdown。
    CreateMarkdown(PathBuf),
    /// 在目录下创建文件夹。
    CreateFolder(PathBuf),
    /// 重命名路径。
    Rename(PathBuf),
    /// 删除 Markdown 文件。
    DeleteMarkdown(PathBuf),
    /// 复制绝对路径。
    CopyAbsolute(PathBuf),
    /// 复制 outline 相对路径。
    CopyRelative(PathBuf),
    /// 在文件管理器中定位路径。
    Reveal(PathBuf),
    /// 切换 Markdown 收藏状态。
    ToggleFavorite(PathBuf),
    /// 请求刷新 outline。
    Refresh,
}

/// tree row 左侧 marker。
pub(crate) enum TreeRowMarker<'a> {
    /// 目录 marker，携带展开状态。
    Dir { expanded: bool },
    /// 文件 marker，携带可选 badge。
    File { badge: Option<&'a str> },
}

/// 绘制最近访问 Markdown modal 的 tree 内容。
pub(crate) fn recent_markdown_outline_dialog_content(
    ui: &mut Ui,
    nodes: &mut [OutlineNode],
    selected_file: Option<&Path>,
    workspace_favorites: &BTreeSet<PathBuf>,
    global_favorites: &BTreeSet<PathBuf>,
    action: &mut Option<OutlineAction>,
    language: AppLanguage,
) {
    if nodes.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(36.0);
            ui.label(
                RichText::new(i18n::text(language, "No recently viewed Markdown"))
                    .color(theme::muted()),
            );
        });
        return;
    }
    ScrollArea::both()
        .id_salt("recent-markdown-outline")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut tree_hovered = false;
            for node in nodes {
                render_outline_node(
                    ui,
                    node,
                    0,
                    selected_file,
                    workspace_favorites,
                    global_favorites,
                    action,
                    &mut tree_hovered,
                    language,
                );
            }
        });
}

/// 从完整 outline 中构造最近访问 Markdown tree。
pub(crate) fn recent_markdown_outline_nodes(
    nodes: &[OutlineNode],
    recent: &[data::RecentMarkdownEntry],
) -> Vec<OutlineNode> {
    let ranks = recent
        .iter()
        .map(|entry| (entry.path.clone(), entry.edited_at_ms))
        .collect::<BTreeMap<_, _>>();
    let mut ranked = nodes
        .iter()
        .filter_map(|node| recent_markdown_outline_node(node, &ranks))
        .collect::<Vec<_>>();
    sort_recent_outline_nodes(&mut ranked);
    ranked.into_iter().map(|(node, _)| node).collect()
}

/// 递归过滤一个 outline 节点，并返回该子树最近访问时间。
fn recent_markdown_outline_node(
    node: &OutlineNode,
    ranks: &BTreeMap<PathBuf, u64>,
) -> Option<(OutlineNode, u64)> {
    match node {
        OutlineNode::File { path, label } => ranks.get(path).map(|rank| {
            (
                OutlineNode::File {
                    path: path.clone(),
                    label: label.clone(),
                },
                *rank,
            )
        }),
        OutlineNode::Root {
            key,
            label,
            children,
            ..
        } => {
            let mut children = children
                .iter()
                .filter_map(|child| recent_markdown_outline_node(child, ranks))
                .collect::<Vec<_>>();
            if children.is_empty() {
                return None;
            }
            sort_recent_outline_nodes(&mut children);
            let newest = children
                .iter()
                .map(|(_, rank)| *rank)
                .max()
                .unwrap_or_default();
            Some((
                OutlineNode::Root {
                    key: key.clone(),
                    label: label.clone(),
                    expanded: true,
                    children: children.into_iter().map(|(node, _)| node).collect(),
                },
                newest,
            ))
        }
        OutlineNode::Dir {
            key,
            label,
            children,
            ..
        } => {
            let mut children = children
                .iter()
                .filter_map(|child| recent_markdown_outline_node(child, ranks))
                .collect::<Vec<_>>();
            if children.is_empty() {
                return None;
            }
            sort_recent_outline_nodes(&mut children);
            let newest = children
                .iter()
                .map(|(_, rank)| *rank)
                .max()
                .unwrap_or_default();
            Some((
                OutlineNode::Dir {
                    key: key.clone(),
                    label: label.clone(),
                    expanded: true,
                    children: children.into_iter().map(|(node, _)| node).collect(),
                },
                newest,
            ))
        }
    }
}

/// 按最近访问时间降序排序 outline 节点。
fn sort_recent_outline_nodes(nodes: &mut [(OutlineNode, u64)]) {
    nodes.sort_by(|(left, left_rank), (right, right_rank)| {
        right_rank
            .cmp(left_rank)
            .then_with(|| outline_node_label(left).cmp(outline_node_label(right)))
    });
}

/// 返回 outline 节点排序使用的稳定 label。
fn outline_node_label(node: &OutlineNode) -> &str {
    match node {
        OutlineNode::Root { label, .. }
        | OutlineNode::Dir { label, .. }
        | OutlineNode::File { label, .. } => label,
    }
}

/// 绘制普通 outline 节点。
pub(crate) fn render_outline_node(
    ui: &mut Ui,
    node: &mut OutlineNode,
    depth: usize,
    selected_file: Option<&Path>,
    workspace_favorites: &BTreeSet<PathBuf>,
    global_favorites: &BTreeSet<PathBuf>,
    action: &mut Option<OutlineAction>,
    tree_hovered: &mut bool,
    language: AppLanguage,
) {
    match node {
        OutlineNode::Root {
            key,
            label,
            expanded,
            children,
        } => {
            ui.push_id(key.as_path(), |ui| {
                let response = tree_row(ui, depth, *expanded, true, label, false);
                if response.clicked() {
                    let was_expanded = *expanded;
                    *expanded = !*expanded;
                    if !was_expanded {
                        *action = Some(OutlineAction::Refresh);
                    }
                }
                if response.hovered() {
                    *tree_hovered = true;
                }
                response.context_menu(|ui| outline_dir_menu(ui, key, action, language));
                if *expanded {
                    for child in children {
                        render_outline_node(
                            ui,
                            child,
                            depth + 1,
                            selected_file,
                            workspace_favorites,
                            global_favorites,
                            action,
                            tree_hovered,
                            language,
                        );
                    }
                }
            });
        }
        OutlineNode::Dir {
            key,
            label,
            expanded,
            children,
        } => {
            ui.push_id(key.as_path(), |ui| {
                let response = tree_row(ui, depth, *expanded, true, label, false);
                if response.clicked() {
                    let was_expanded = *expanded;
                    *expanded = !*expanded;
                    if !was_expanded {
                        *action = Some(OutlineAction::Refresh);
                    }
                }
                if response.hovered() {
                    *tree_hovered = true;
                }
                response.context_menu(|ui| outline_dir_menu(ui, key, action, language));
                if *expanded {
                    for child in children {
                        render_outline_node(
                            ui,
                            child,
                            depth + 1,
                            selected_file,
                            workspace_favorites,
                            global_favorites,
                            action,
                            tree_hovered,
                            language,
                        );
                    }
                }
            });
        }
        OutlineNode::File { path, label } => {
            let selected = selected_file.is_some_and(|selected| selected == path);
            let favorited = outline_path_is_favorite(path, workspace_favorites, global_favorites);
            let response = tree_row(ui, depth, false, false, label, selected);
            if response.clicked() {
                *action = Some(OutlineAction::OpenEditor(path.clone()));
            }
            if response.hovered() {
                *tree_hovered = true;
            }
            response.context_menu(|ui| outline_file_menu(ui, path, favorited, action, language));
        }
    }
}

/// 绘制收藏过滤后的 outline 节点。
pub(crate) fn render_favorite_outline_node(
    ui: &mut Ui,
    node: &mut OutlineNode,
    depth: usize,
    selected_file: Option<&Path>,
    workspace_favorites: &BTreeSet<PathBuf>,
    global_favorites: &BTreeSet<PathBuf>,
    action: &mut Option<OutlineAction>,
    tree_hovered: &mut bool,
    language: AppLanguage,
) -> bool {
    if !outline_node_has_favorite(node, workspace_favorites, global_favorites) {
        return false;
    }
    match node {
        OutlineNode::Root {
            key,
            label,
            expanded,
            children,
        } => {
            ui.push_id(key.as_path(), |ui| {
                let response = tree_row(ui, depth, *expanded, true, label, false);
                if response.clicked() {
                    *expanded = !*expanded;
                }
                if response.hovered() {
                    *tree_hovered = true;
                }
                response.context_menu(|ui| outline_dir_menu(ui, key, action, language));
                if *expanded {
                    for child in children {
                        render_favorite_outline_node(
                            ui,
                            child,
                            depth + 1,
                            selected_file,
                            workspace_favorites,
                            global_favorites,
                            action,
                            tree_hovered,
                            language,
                        );
                    }
                }
            });
        }
        OutlineNode::Dir {
            key,
            label,
            expanded,
            children,
        } => {
            ui.push_id(key.as_path(), |ui| {
                let response = tree_row(ui, depth, *expanded, true, label, false);
                if response.clicked() {
                    *expanded = !*expanded;
                }
                if response.hovered() {
                    *tree_hovered = true;
                }
                response.context_menu(|ui| outline_dir_menu(ui, key, action, language));
                if *expanded {
                    for child in children {
                        render_favorite_outline_node(
                            ui,
                            child,
                            depth + 1,
                            selected_file,
                            workspace_favorites,
                            global_favorites,
                            action,
                            tree_hovered,
                            language,
                        );
                    }
                }
            });
        }
        OutlineNode::File { path, label } => {
            let selected = selected_file.is_some_and(|selected| selected == path);
            let response = tree_row(ui, depth, false, false, label, selected);
            if response.clicked() {
                *action = Some(OutlineAction::OpenEditor(path.clone()));
            }
            if response.hovered() {
                *tree_hovered = true;
            }
            response.context_menu(|ui| outline_file_menu(ui, path, true, action, language));
        }
    }
    true
}

/// 判断 outline 子树是否包含收藏项。
fn outline_node_has_favorite(
    node: &OutlineNode,
    workspace_favorites: &BTreeSet<PathBuf>,
    global_favorites: &BTreeSet<PathBuf>,
) -> bool {
    match node {
        OutlineNode::Root { children, .. } | OutlineNode::Dir { children, .. } => children
            .iter()
            .any(|child| outline_node_has_favorite(child, workspace_favorites, global_favorites)),
        OutlineNode::File { path, .. } => {
            outline_path_is_favorite(path, workspace_favorites, global_favorites)
        }
    }
}

/// 将 outline 折叠到第一层。
pub(crate) fn collapse_outline_to_first_level(nodes: &mut [OutlineNode]) {
    for node in nodes {
        collapse_outline_node_to_first_level(node, 0);
    }
}

/// 递归执行第一层折叠。
fn collapse_outline_node_to_first_level(node: &mut OutlineNode, depth: usize) {
    match node {
        OutlineNode::Root {
            expanded, children, ..
        }
        | OutlineNode::Dir {
            expanded, children, ..
        } => {
            *expanded = depth == 0;
            for child in children {
                collapse_outline_node_to_first_level(child, depth + 1);
            }
        }
        OutlineNode::File { .. } => {}
    }
}

/// 判断 outline 路径是否属于 Home Root 的全局收藏区。
pub(crate) fn outline_path_is_global(path: &Path, absolute: &Path) -> bool {
    path.is_absolute()
        && crate::home::home_dir()
            .as_deref()
            .is_some_and(|home| absolute.starts_with(home))
}

/// 判断 outline 文件是否已收藏。
fn outline_path_is_favorite(
    path: &Path,
    workspace_favorites: &BTreeSet<PathBuf>,
    global_favorites: &BTreeSet<PathBuf>,
) -> bool {
    if outline_path_is_global(path, path) {
        global_favorites.contains(path)
    } else {
        workspace_favorites.contains(path)
    }
}

/// 切换路径集合成员，返回切换后的收藏状态。
pub(crate) fn toggle_path_in_set(paths: &mut BTreeSet<PathBuf>, path: PathBuf) -> bool {
    if paths.remove(&path) {
        false
    } else {
        paths.insert(path);
        true
    }
}

/// 绘制普通 tree row。
fn tree_row(
    ui: &mut Ui,
    depth: usize,
    expanded: bool,
    is_dir: bool,
    label: &str,
    selected: bool,
) -> egui::Response {
    let marker = if is_dir {
        TreeRowMarker::Dir { expanded }
    } else {
        TreeRowMarker::File { badge: Some("MD") }
    };
    compact_tree_row(ui, depth, marker, label, selected)
}

/// 绘制紧凑 tree row，供 outline 和 reviewer 文件树共用。
pub(crate) fn compact_tree_row(
    ui: &mut Ui,
    depth: usize,
    marker: TreeRowMarker<'_>,
    label: &str,
    selected: bool,
) -> egui::Response {
    let row_height = 22.0;
    let width = ui.available_width().max(80.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, row_height), Sense::click());
    let fill = if selected {
        theme::primary_soft()
    } else if response.hovered() {
        theme::hover()
    } else {
        theme::transparent()
    };
    if fill != theme::transparent() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme::RADIUS_SM), fill);
    }

    let center_y = rect.center().y;
    let indent = depth as f32 * 12.0;
    let marker_left = rect.left() + 6.0 + indent;
    let label_left = match marker {
        TreeRowMarker::Dir { expanded } => {
            paint_tree_chevron(
                ui,
                egui::pos2(marker_left + 5.5, center_y),
                expanded,
                theme::muted(),
            );
            marker_left + 18.0
        }
        TreeRowMarker::File { badge } => {
            if let Some(badge) = badge {
                let font = egui::TextStyle::Small.resolve(ui.style());
                ui.painter().text(
                    egui::pos2(marker_left, center_y),
                    Align2::LEFT_CENTER,
                    badge,
                    font,
                    theme::muted(),
                );
                marker_left + 24.0
            } else {
                marker_left + 14.0
            }
        }
    };

    ui.painter().text(
        egui::pos2(label_left, center_y),
        Align2::LEFT_CENTER,
        label,
        egui::TextStyle::Body.resolve(ui.style()),
        theme::list_text(),
    );

    if selected {
        ui.painter().circle_filled(
            egui::pos2(rect.right() - 14.0, center_y),
            3.5,
            theme::primary(),
        );
    }

    response
}

/// 计算 tree row 内容宽度，供布局回归测试使用。
#[cfg(test)]
pub(crate) fn tree_row_content_width_from_label_width(
    depth: usize,
    marker_width: f32,
    label_width: f32,
    selected: bool,
) -> f32 {
    let selection_width = if selected { 22.0 } else { 6.0 };
    6.0 + depth as f32 * 12.0 + marker_width + label_width + selection_width + 8.0
}

/// 绘制 tree 展开箭头。
fn paint_tree_chevron(ui: &Ui, center: egui::Pos2, expanded: bool, color: Color32) {
    let stroke = Stroke::new(1.5, color);
    if expanded {
        ui.painter().line_segment(
            [
                egui::pos2(center.x - 4.0, center.y - 2.0),
                egui::pos2(center.x, center.y + 3.0),
            ],
            stroke,
        );
        ui.painter().line_segment(
            [
                egui::pos2(center.x, center.y + 3.0),
                egui::pos2(center.x + 4.0, center.y - 2.0),
            ],
            stroke,
        );
    } else {
        ui.painter().line_segment(
            [
                egui::pos2(center.x - 2.0, center.y - 4.0),
                egui::pos2(center.x + 3.0, center.y),
            ],
            stroke,
        );
        ui.painter().line_segment(
            [
                egui::pos2(center.x + 3.0, center.y),
                egui::pos2(center.x - 2.0, center.y + 4.0),
            ],
            stroke,
        );
    }
}

/// 绘制目录右键菜单。
fn outline_dir_menu(
    ui: &mut Ui,
    dir: &Path,
    action: &mut Option<OutlineAction>,
    language: AppLanguage,
) {
    if ui.button(i18n::text(language, "New markdown")).clicked() {
        *action = Some(OutlineAction::CreateMarkdown(dir.to_path_buf()));
        ui.close_menu();
    }
    if ui.button(i18n::text(language, "New folder")).clicked() {
        *action = Some(OutlineAction::CreateFolder(dir.to_path_buf()));
        ui.close_menu();
    }
    if ui
        .button(i18n::text(language, "Reveal in Finder"))
        .clicked()
    {
        *action = Some(OutlineAction::Reveal(dir.to_path_buf()));
        ui.close_menu();
    }
    if ui
        .button(i18n::text(language, "Copy absolute path"))
        .clicked()
    {
        *action = Some(OutlineAction::CopyAbsolute(dir.to_path_buf()));
        ui.close_menu();
    }
    if ui
        .button(i18n::text(language, "Copy relative path"))
        .clicked()
    {
        *action = Some(OutlineAction::CopyRelative(dir.to_path_buf()));
        ui.close_menu();
    }
    if ui.button(i18n::text(language, "Refresh")).clicked() {
        *action = Some(OutlineAction::Refresh);
        ui.close_menu();
    }
}

/// 绘制 Markdown 文件右键菜单。
fn outline_file_menu(
    ui: &mut Ui,
    path: &Path,
    favorited: bool,
    action: &mut Option<OutlineAction>,
    language: AppLanguage,
) {
    if ui
        .button(i18n::text(
            language,
            if favorited { "Unfavorite" } else { "Favorite" },
        ))
        .clicked()
    {
        *action = Some(OutlineAction::ToggleFavorite(path.to_path_buf()));
        ui.close_menu();
    }
    ui.separator();
    if ui.button(i18n::text(language, "Open in Editor")).clicked() {
        *action = Some(OutlineAction::OpenEditor(path.to_path_buf()));
        ui.close_menu();
    }
    if ui.button(i18n::text(language, "Open in Preview")).clicked() {
        *action = Some(OutlineAction::OpenPreview(path.to_path_buf()));
        ui.close_menu();
    }
    if ui.button(i18n::text(language, "Rename")).clicked() {
        *action = Some(OutlineAction::Rename(path.to_path_buf()));
        ui.close_menu();
    }
    if ui
        .button(i18n::text(language, "Reveal in Finder"))
        .clicked()
    {
        *action = Some(OutlineAction::Reveal(path.to_path_buf()));
        ui.close_menu();
    }
    if ui
        .button(i18n::text(language, "Copy absolute path"))
        .clicked()
    {
        *action = Some(OutlineAction::CopyAbsolute(path.to_path_buf()));
        ui.close_menu();
    }
    if ui
        .button(i18n::text(language, "Copy relative path"))
        .clicked()
    {
        *action = Some(OutlineAction::CopyRelative(path.to_path_buf()));
        ui.close_menu();
    }
    if ui
        .button(RichText::new(i18n::text(language, "Delete markdown")).color(theme::danger()))
        .clicked()
    {
        *action = Some(OutlineAction::DeleteMarkdown(path.to_path_buf()));
        ui.close_menu();
    }
}

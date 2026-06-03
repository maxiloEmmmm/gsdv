//! workspace chrome 绘制与轻量 UI helper。
//!
//! 这里承接 rail、tab、状态点和 editor gutter 等无状态界面片段，
//! 避免 `app.rs` 同时承担顶层状态机和具体控件实现。

use super::*;

pub(super) fn panel_frame() -> Frame {
    Frame::new()
        .fill(theme::bg())
        .stroke(Stroke::new(1.0, theme::border()))
        .inner_margin(Margin::symmetric(10, 8))
}

pub(super) fn workspace_mode_tabs(
    ui: &mut Ui,
    current_mode: CenterMode,
    markdown_outline_collapsed: bool,
    language: AppLanguage,
    mut set_mode: impl FnMut(CenterMode),
) -> Option<AgentTabAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        for (label, mode, active) in [
            (
                "Agent",
                CenterMode::Agent,
                current_mode == CenterMode::Agent,
            ),
            (
                "Md",
                markdown_tab_mode(current_mode),
                matches!(current_mode, CenterMode::Editor | CenterMode::Preview),
            ),
        ] {
            let button = ui.add_sized(
                [82.0, 28.0],
                Button::new(RichText::new(label).color(if active {
                    theme::primary()
                } else {
                    theme::text()
                }))
                .fill(theme::bg())
                .stroke(Stroke::NONE),
            );
            if button.clicked() {
                set_mode(mode);
            }
            if matches!(mode, CenterMode::Editor | CenterMode::Preview) {
                button.context_menu(|ui| {
                    let (label, collapsed) = if markdown_outline_collapsed {
                        (i18n::text(language, "Show outline"), false)
                    } else {
                        (i18n::text(language, "Hide outline"), true)
                    };
                    if ui.button(label).clicked() {
                        action = Some(AgentTabAction::SetMarkdownOutlineCollapsed(collapsed));
                        ui.close_menu();
                    }
                });
            }
            if active {
                let rect = button.rect;
                ui.painter().line_segment(
                    [rect.left_bottom(), rect.right_bottom()],
                    Stroke::new(2.0, theme::primary()),
                );
            }
        }
    });
    action
}

pub(super) fn markdown_tab_mode(current_mode: CenterMode) -> CenterMode {
    match current_mode {
        CenterMode::Preview => CenterMode::Preview,
        _ => CenterMode::Editor,
    }
}

/// Returns all agent slots that belong to one workspace.
pub(super) fn agent_slots_for_workspace(workspace: &WorkspaceViewData) -> Vec<AgentSlotId> {
    std::iter::once(AgentSlotId::Main)
        .chain(
            workspace
                .subagents
                .iter()
                .map(|subagent| AgentSlotId::Subagent(subagent.id.clone())),
        )
        .collect()
}

/// Renders one main/subagent tab inside the Agent surface.
pub(super) fn agent_slot_tab_button(
    ui: &mut Ui,
    label: &str,
    activity: WorkspaceActivity,
    active: bool,
) -> egui::Response {
    let (dot, _) = activity_style(activity);
    let response = ui.add_sized(
        [label.chars().count().max(4) as f32 * 8.0 + 32.0, 24.0],
        Button::new(RichText::new(label).color(if active {
            theme::primary()
        } else {
            theme::text()
        }))
        .fill(if active {
            theme::primary_soft()
        } else {
            theme::bg()
        })
        .stroke(Stroke::new(
            1.0,
            if active {
                theme::primary()
            } else {
                theme::border()
            },
        )),
    );
    let center = egui::pos2(response.rect.left() + 10.0, response.rect.center().y);
    paint_activity_dot(ui, center, dot, activity, true, Duration::from_millis(500));
    response
}

/// 绘制 main/subagent 槽位右键菜单，动作始终绑定被点击的槽位。
pub(super) fn agent_slot_context_menu(
    ui: &mut Ui,
    slot: AgentSlotId,
    current_agent_kind: AgentKind,
    current_agent_model: Option<&str>,
    current_agent_effort: Option<&str>,
    current_agent_fast_mode: Option<bool>,
    current_agent_work_dir: Option<&Path>,
    current_session_id: Option<&str>,
    action: &mut Option<AgentTabAction>,
    language: AppLanguage,
) {
    if let Some(session_id) = current_session_id {
        if ui.button(i18n::text(language, "Copy session ID")).clicked() {
            *action = Some(AgentTabAction::CopySessionId(session_id.to_string()));
            ui.close_menu();
        }
    } else {
        ui.add_enabled(
            false,
            Button::new(i18n::text(language, "Copy session ID (after chat)")),
        );
    }
    if ui.button(i18n::text(language, "Restart agent")).clicked() {
        *action = Some(AgentTabAction::Restart(slot.clone()));
        ui.close_menu();
    }
    if ui.button(i18n::text(language, "Set model")).clicked() {
        *action = Some(AgentTabAction::SetModel {
            slot: slot.clone(),
            model: current_agent_model.unwrap_or_default().to_string(),
        });
        ui.close_menu();
    }
    if ui.button(i18n::text(language, "Set work-dir")).clicked() {
        *action = Some(AgentTabAction::SetWorkDir {
            slot: slot.clone(),
            work_dir: current_agent_work_dir
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default(),
        });
        ui.close_menu();
    }
    ui.menu_button(i18n::text(language, "Effort"), |ui| {
        let current = current_agent_effort.unwrap_or_default();
        if ui
            .selectable_label(current.is_empty(), i18n::text(language, "Default effort"))
            .clicked()
        {
            *action = Some(AgentTabAction::SetEffort {
                slot: slot.clone(),
                effort: None,
            });
            ui.close_menu();
        }
        for effort in current_agent_kind.effort_levels() {
            if ui.selectable_label(current == *effort, *effort).clicked() {
                *action = Some(AgentTabAction::SetEffort {
                    slot: slot.clone(),
                    effort: Some((*effort).to_string()),
                });
                ui.close_menu();
            }
        }
    });
    if current_agent_kind.supports_fast_mode() {
        ui.menu_button(i18n::text(language, "Fast mode"), |ui| {
            if ui
                .selectable_label(
                    current_agent_fast_mode.is_none(),
                    i18n::text(language, "Default fast mode"),
                )
                .clicked()
            {
                *action = Some(AgentTabAction::SetFastMode {
                    slot: slot.clone(),
                    fast_mode: None,
                });
                ui.close_menu();
            }
            if ui
                .selectable_label(current_agent_fast_mode == Some(true), "On")
                .clicked()
            {
                *action = Some(AgentTabAction::SetFastMode {
                    slot: slot.clone(),
                    fast_mode: Some(true),
                });
                ui.close_menu();
            }
            if ui
                .selectable_label(current_agent_fast_mode == Some(false), "Off")
                .clicked()
            {
                *action = Some(AgentTabAction::SetFastMode {
                    slot: slot.clone(),
                    fast_mode: Some(false),
                });
                ui.close_menu();
            }
        });
    }
    ui.menu_button(i18n::text(language, "Switch agent"), |ui| {
        for agent_kind in AgentKind::all() {
            if agent_kind == current_agent_kind {
                continue;
            }
            if ui.button(agent_kind.title()).clicked() {
                *action = Some(AgentTabAction::Switch {
                    slot: slot.clone(),
                    next_kind: agent_kind,
                });
                ui.close_menu();
            }
        }
    });
}

/// Collapses main and subagent state into one workspace-level activity.
pub(super) fn workspace_effective_activity(workspace: &WorkspaceViewData) -> WorkspaceActivity {
    if workspace.activity == WorkspaceActivity::Busy
        || workspace
            .subagents
            .iter()
            .any(|subagent| subagent.activity == WorkspaceActivity::Busy)
    {
        return WorkspaceActivity::Busy;
    }
    if workspace.activity == WorkspaceActivity::Idle
        || workspace
            .subagents
            .iter()
            .any(|subagent| subagent.activity == WorkspaceActivity::Idle)
    {
        return WorkspaceActivity::Idle;
    }
    WorkspaceActivity::Unknown
}

/// 根据当前源码行数计算编辑器行号 gutter 宽度。
pub(super) fn editor_line_number_gutter_width(text: &str) -> f32 {
    let digits = text.split('\n').count().max(1).to_string().len();
    (digits as f32 * 9.0 + 18.0).max(34.0)
}

/// 为手绘行号 gutter 预留横向空间。
pub(super) fn reserve_editor_line_number_gutter(ui: &mut Ui, width: f32) -> egui::Rect {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
    rect
}

/// 基于 TextEdit 实际折行布局绘制源码行号。
pub(super) fn paint_editor_line_number_gutter(
    ui: &mut Ui,
    gutter_rect: egui::Rect,
    output: &TextEditOutput,
    font: egui::FontId,
) {
    let clip_rect = egui::Rect::from_min_max(
        egui::pos2(gutter_rect.left(), output.text_clip_rect.top()),
        egui::pos2(gutter_rect.right(), output.text_clip_rect.bottom()),
    );
    let painter = ui.painter().with_clip_rect(clip_rect);
    let mut source_line = 1usize;
    let mut first_visual_row = true;
    for row in &output.galley.rows {
        if first_visual_row {
            // 触发条件：一条源码行被 TextEdit 自动折成多条视觉行。
            // 不能按 '\n' 拼 Label：Label 不知道 TextEdit 的折行位置。
            // 防止回归：行号居中、截断或与折行后的文本错位。
            let pos = egui::pos2(
                gutter_rect.right() - 6.0,
                output.galley_pos.y + row.rect.top(),
            );
            painter.text(
                pos,
                Align2::RIGHT_TOP,
                source_line.to_string(),
                font.clone(),
                theme::muted(),
            );
        }
        if row.ends_with_newline {
            source_line += 1;
            first_visual_row = true;
        } else {
            first_visual_row = false;
        }
    }
}

pub(super) fn agent_markdown_toggle_modes(
    current_mode: CenterMode,
    previous_mode: CenterMode,
) -> (CenterMode, CenterMode) {
    match current_mode {
        CenterMode::Editor | CenterMode::Preview => (CenterMode::Agent, current_mode),
        CenterMode::Agent | CenterMode::Terminal => {
            let markdown_mode = match previous_mode {
                CenterMode::Editor | CenterMode::Preview => previous_mode,
                CenterMode::Agent | CenterMode::Terminal => CenterMode::Editor,
            };
            (markdown_mode, previous_mode)
        }
    }
}

pub(super) fn default_terminal_input_target(
    workspace: &WorkspaceViewData,
    workspace_terminal_open: bool,
    reviewer_helix_open: bool,
) -> Option<TerminalSurfaceKind> {
    if reviewer_helix_open {
        return Some(TerminalSurfaceKind::Helix);
    }
    if workspace_terminal_open {
        return Some(TerminalSurfaceKind::Workspace);
    }
    if workspace.route == Route::Reviewer {
        return None;
    }
    match workspace.center_mode {
        CenterMode::Agent | CenterMode::Terminal => Some(TerminalSurfaceKind::Agent),
        CenterMode::Editor | CenterMode::Preview => None,
    }
}

/// 返回当前 Unix 毫秒，适用于轻量 UI 排序时间戳。
pub(super) fn current_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

/// Chooses the Helix workdir for a terminal-detected file.
///
/// 触发条件：Agent 终端里的 path:line 被点击后打开 Helix。
/// 不能直接用 workspace dir：monorepo 内嵌 git repo 需要 hx LSP/搜索
/// 以文件所在 repo 为根。
/// 防止回归：文件不在 git repo 时仍能回退到当前 workspace。
pub(super) fn terminal_file_helix_workdir(workspace_dir: &Path, file_path: &Path) -> PathBuf {
    let start = file_path.parent().unwrap_or(file_path);
    start
        .ancestors()
        .take_while(|candidate| *candidate == workspace_dir || candidate.starts_with(workspace_dir))
        .find(|candidate| candidate.join(".git").exists())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| workspace_dir.to_path_buf())
}

/// Resolves a terminal `path:line` path against the active workspace.
pub(super) fn resolve_terminal_file_line_path(workspace_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_dir.join(path)
    }
}

pub(super) fn network_settings_changed_since(
    baseline: Option<&NetworkSettings>,
    current: &NetworkSettings,
) -> bool {
    baseline.is_some_and(|baseline| baseline != current)
}

pub(super) fn bottom_bar_frame() -> Frame {
    Frame::new()
        .fill(theme::bg())
        .stroke(Stroke::new(1.0, theme::border()))
        .inner_margin(Margin::symmetric(8, 0))
}

pub(super) fn theme_mode_switch(
    ui: &mut Ui,
    mode: theme::ThemeMode,
    language: AppLanguage,
) -> egui::Response {
    let size = Vec2::new(52.0, 24.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let active_left = mode == theme::ThemeMode::Light;
    let active_rect = if active_left {
        egui::Rect::from_min_size(rect.min + Vec2::splat(2.0), Vec2::new(24.0, 20.0))
    } else {
        egui::Rect::from_min_size(
            egui::pos2(rect.right() - 26.0, rect.top() + 2.0),
            Vec2::new(24.0, 20.0),
        )
    };

    ui.painter().rect_filled(
        rect,
        CornerRadius::same(theme::RADIUS_SM),
        theme::surface_elevated(),
    );
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(theme::RADIUS_SM),
        Stroke::new(1.0, theme::border()),
        egui::StrokeKind::Inside,
    );
    ui.painter().rect_filled(
        active_rect,
        CornerRadius::same(theme::RADIUS_SM),
        theme::primary_soft(),
    );

    let sun_color = if active_left {
        theme::primary()
    } else {
        theme::muted()
    };
    let moon_color = if active_left {
        theme::muted()
    } else {
        theme::primary()
    };
    ui.painter().text(
        egui::pos2(rect.left() + 14.0, rect.center().y),
        Align2::CENTER_CENTER,
        "☀",
        egui::TextStyle::Body.resolve(ui.style()),
        sun_color,
    );
    ui.painter().text(
        egui::pos2(rect.right() - 14.0, rect.center().y),
        Align2::CENTER_CENTER,
        "☾",
        egui::TextStyle::Body.resolve(ui.style()),
        moon_color,
    );
    response.on_hover_text(i18n::text(language, "Toggle light/dark mode"))
}

pub(super) fn rail_header_add_button(ui: &mut Ui) -> egui::Response {
    rail_header_icon_button(ui, "Add", |ui, center, color| {
        paint_plus_icon(ui, center, 5.0, color, 1.4);
    })
}

pub(super) fn rail_header_back_button(ui: &mut Ui) -> egui::Response {
    rail_header_icon_button(ui, "Back", |ui, center, color| {
        paint_back_icon(ui, center, color);
    })
}

pub(super) fn rail_header_collapse_button(ui: &mut Ui, expanded: bool) -> egui::Response {
    let hover_text = if expanded {
        "Collapse workspace rail"
    } else {
        "Expand workspace rail"
    };
    rail_header_icon_button(ui, hover_text, |ui, center, color| {
        paint_sidebar_toggle_icon(ui, center, color, expanded);
    })
}

pub(super) fn rail_header_icon_button(
    ui: &mut Ui,
    hover_text: &'static str,
    paint_icon: impl FnOnce(&Ui, egui::Pos2, Color32),
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
    let fill = if response.hovered() {
        theme::surface()
    } else {
        theme::bg()
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(theme::RADIUS_SM), fill);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(theme::RADIUS_SM),
        Stroke::new(1.0, theme::muted()),
        egui::StrokeKind::Inside,
    );
    paint_icon(ui, rect.center(), theme::muted());
    response.on_hover_text(hover_text)
}

pub(super) fn compact_workspace_rail_row(
    ui: &mut Ui,
    workspace: &WorkspaceViewData,
    active: bool,
    repaint_after: Duration,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(38.0, 38.0), Sense::click());
    let fill = if active {
        theme::primary_soft()
    } else if response.hovered() {
        theme::surface()
    } else {
        theme::transparent()
    };
    if fill != theme::transparent() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme::RADIUS_MD), fill);
    }
    if active {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(theme::RADIUS_MD),
            Stroke::new(1.0, theme::primary_border()),
            egui::StrokeKind::Inside,
        );
    }

    let activity = workspace_effective_activity(workspace);
    let (dot_color, status) = activity_style(activity);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        workspace_initial(&workspace.name),
        egui::FontId::proportional(15.0),
        theme::text(),
    );
    paint_activity_dot(
        ui,
        egui::pos2(rect.right() - 7.0, rect.bottom() - 7.0),
        dot_color,
        activity,
        true,
        repaint_after,
    );
    response.on_hover_text(format!("{}\n{}", workspace.name, status))
}

pub(super) fn workspace_rail_row(
    ui: &mut Ui,
    workspace: &WorkspaceViewData,
    active: bool,
    repaint_after: Duration,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 52.0), Sense::click());
    let paint_rect = rect.shrink2(Vec2::new(RAIL_EDGE_INSET, 0.0));
    let fill = if active {
        theme::primary_soft()
    } else if response.hovered() {
        theme::surface()
    } else {
        theme::transparent()
    };
    if fill != theme::transparent() {
        ui.painter()
            .rect_filled(paint_rect, CornerRadius::same(theme::RADIUS_MD), fill);
    }
    if active {
        ui.painter().rect_stroke(
            paint_rect,
            CornerRadius::same(theme::RADIUS_MD),
            Stroke::new(1.0, theme::primary_border()),
            egui::StrokeKind::Inside,
        );
    }

    let text_x = paint_rect.left() + 12.0;
    ui.painter().text(
        egui::pos2(text_x, paint_rect.top() + 18.0),
        Align2::LEFT_CENTER,
        truncate_middle(&workspace.name, 20),
        egui::FontId::proportional(13.0),
        theme::text(),
    );

    let activity = workspace_effective_activity(workspace);
    let (dot_color, status) = activity_style(activity);
    paint_activity_dot(
        ui,
        egui::pos2(text_x + 3.0, paint_rect.top() + 35.0),
        dot_color,
        activity,
        false,
        repaint_after,
    );
    ui.painter().text(
        egui::pos2(text_x + 15.0, paint_rect.top() + 35.0),
        Align2::LEFT_CENTER,
        status,
        egui::FontId::proportional(12.0),
        if activity == WorkspaceActivity::Busy {
            theme::primary()
        } else {
            theme::muted()
        },
    );

    ui.painter().text(
        egui::pos2(paint_rect.right() - 18.0, paint_rect.center().y),
        Align2::CENTER_CENTER,
        "...",
        egui::FontId::proportional(13.0),
        theme::muted(),
    );
    response
}

pub(super) fn compact_rail_nav_button(
    ui: &mut Ui,
    hover_text: &'static str,
    paint_icon: impl FnOnce(&Ui, egui::Pos2, Color32),
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(34.0, 28.0), Sense::click());
    let fill = if response.hovered() {
        theme::surface()
    } else {
        theme::transparent()
    };
    if fill != theme::transparent() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme::RADIUS_SM), fill);
    }
    paint_icon(ui, rect.center(), theme::muted());
    response.on_hover_text(hover_text)
}

pub(super) fn rail_nav_row(ui: &mut Ui, icon: &str, label: &str) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::click());
    let fill = if response.hovered() {
        theme::surface()
    } else {
        theme::transparent()
    };
    if fill != theme::transparent() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme::RADIUS_SM), fill);
    }
    let icon_center = egui::pos2(rect.left() + 17.0, rect.center().y);
    if icon == "gear" {
        paint_gear_icon(ui, icon_center, theme::muted());
    } else {
        paint_plus_icon(ui, icon_center, 6.0, theme::muted(), 1.4);
    }
    ui.painter().text(
        egui::pos2(rect.left() + 36.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        egui::TextStyle::Body.resolve(ui.style()),
        theme::text(),
    );
    response
}

pub(super) fn paint_plus_icon(
    ui: &Ui,
    center: egui::Pos2,
    radius: f32,
    color: Color32,
    width: f32,
) {
    let stroke = Stroke::new(width, color);
    ui.painter().line_segment(
        [
            egui::pos2(center.x - radius, center.y),
            egui::pos2(center.x + radius, center.y),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            egui::pos2(center.x, center.y - radius),
            egui::pos2(center.x, center.y + radius),
        ],
        stroke,
    );
}

pub(super) fn paint_back_icon(ui: &Ui, center: egui::Pos2, color: Color32) {
    let stroke = Stroke::new(1.4, color);
    ui.painter().line_segment(
        [
            egui::pos2(center.x + 5.0, center.y),
            egui::pos2(center.x - 5.0, center.y),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            egui::pos2(center.x - 5.0, center.y),
            egui::pos2(center.x - 1.0, center.y - 4.0),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            egui::pos2(center.x - 5.0, center.y),
            egui::pos2(center.x - 1.0, center.y + 4.0),
        ],
        stroke,
    );
}

pub(super) fn paint_sidebar_toggle_icon(
    ui: &Ui,
    center: egui::Pos2,
    color: Color32,
    expanded: bool,
) {
    let stroke = Stroke::new(1.35, color);
    let panel = egui::Rect::from_center_size(center, Vec2::new(14.0, 14.0));
    ui.painter().rect_stroke(
        panel,
        CornerRadius::same(4),
        stroke,
        egui::StrokeKind::Inside,
    );
    ui.painter().line_segment(
        [
            egui::pos2(panel.left() + 4.5, panel.top() + 2.5),
            egui::pos2(panel.left() + 4.5, panel.bottom() - 2.5),
        ],
        stroke,
    );

    let arrow_x = if expanded {
        panel.left() + 9.5
    } else {
        panel.left() + 7.0
    };
    let direction = if expanded { -1.0 } else { 1.0 };
    ui.painter().line_segment(
        [
            egui::pos2(arrow_x, center.y),
            egui::pos2(arrow_x + direction * 4.0, center.y),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            egui::pos2(arrow_x + direction * 4.0, center.y),
            egui::pos2(arrow_x + direction * 1.5, center.y - 2.5),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            egui::pos2(arrow_x + direction * 4.0, center.y),
            egui::pos2(arrow_x + direction * 1.5, center.y + 2.5),
        ],
        stroke,
    );
}

pub(super) fn paint_gear_icon(ui: &Ui, center: egui::Pos2, color: Color32) {
    let stroke = Stroke::new(1.4, color);
    ui.painter().circle_stroke(center, 5.0, stroke);
    for (dx, dy) in [(0.0, -8.0), (0.0, 8.0), (-8.0, 0.0), (8.0, 0.0)] {
        ui.painter().line_segment(
            [
                egui::pos2(center.x + dx * 0.62, center.y + dy * 0.62),
                egui::pos2(center.x + dx, center.y + dy),
            ],
            stroke,
        );
    }
}

pub(super) fn activity_style(activity: WorkspaceActivity) -> (Color32, &'static str) {
    match activity {
        WorkspaceActivity::Busy => (theme::primary(), "Busy"),
        WorkspaceActivity::Idle => (theme::success(), "Idle"),
        WorkspaceActivity::Unknown => (theme::muted(), "Unknown"),
    }
}

pub(super) fn paint_activity_dot(
    ui: &Ui,
    center: egui::Pos2,
    color: Color32,
    activity: WorkspaceActivity,
    compact: bool,
    repaint_after: Duration,
) {
    if activity == WorkspaceActivity::Busy {
        paint_busy_activity_indicator(ui, center, compact, repaint_after);
    } else {
        ui.painter().circle_filled(center, 3.2, color);
    }
}

pub(super) fn paint_busy_activity_indicator(
    ui: &Ui,
    center: egui::Pos2,
    compact: bool,
    repaint_after: Duration,
) {
    ui.ctx()
        .request_repaint_after(Duration::from_millis(80).max(repaint_after));

    let time = ui.input(|input| input.time) as f32;
    let colors = [theme::primary(), theme::warning(), theme::danger()];
    let scale = if compact { 0.78 } else { 1.0 };
    let bar_width = 2.1 * scale;
    let gap = 1.8 * scale;
    let max_height = 10.0 * scale;
    let min_height = 3.5 * scale;
    let total_width = bar_width * 3.0 + gap * 2.0;
    let left = center.x - total_width * 0.5;
    let baseline = center.y + max_height * 0.45;

    for (index, color) in colors.into_iter().enumerate() {
        let phase = time * 8.5 + index as f32 * 1.7;
        let beat = (phase.sin() + 1.0) * 0.5;
        let height = min_height + beat * (max_height - min_height);
        let x = left + index as f32 * (bar_width + gap);
        let rect = egui::Rect::from_min_max(
            egui::pos2(x, baseline - height),
            egui::pos2(x + bar_width, baseline),
        );
        ui.painter()
            .rect_filled(rect, CornerRadius::same(2), color.gamma_multiply(1.08));
    }
}

pub(super) fn workspace_initial(name: &str) -> String {
    name.chars()
        .find(|ch| ch.is_alphanumeric())
        .map(|ch| ch.to_uppercase().collect())
        .unwrap_or_else(|| "W".to_string())
}

pub(super) fn tiny_status_dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

pub(super) fn should_show_outline_panel(workspace: Option<&WorkspaceViewData>) -> bool {
    workspace.is_some_and(|workspace| workspace.route == Route::Workspace)
}

use super::*;
use crate::gui::data::{OutlineRootKind, SubagentViewData};
use crate::reviewer::app::GuiReviewerRow;

fn test_workspace() -> WorkspaceViewData {
    WorkspaceViewData {
        name: "demo".to_string(),
        path: PathBuf::from("/tmp/demo"),
        agent_kind: AgentKind::Codex,
        agent_model: None,
        agent_model_provider: None,
        agent_effort: None,
        agent_fast_mode: None,
        agent_work_dir: None,
        agent_id: "agent".to_string(),
        session_id: None,
        subagents: Vec::new(),
        activity: WorkspaceActivity::Unknown,
        center_mode: CenterMode::Agent,
        previous_center_mode: CenterMode::Agent,
        route: Route::Workspace,
        reviewer_mode: ReviewerMode::Git,
        selected_file: None,
        outline: Vec::new(),
        outline_favorites: BTreeSet::new(),
        attached_outline_dirs: Vec::new(),
        recent_markdowns: Vec::new(),
        markdown_outline_collapsed: false,
        memo: String::new(),
    }
}

fn key_input(key: egui::Key, modifiers: egui::Modifiers) -> egui::InputState {
    let mut input = egui::InputState::default();
    input.modifiers = modifiers;
    input.events.push(egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers,
    });
    input
}

/// Returns a per-test temporary directory path.
fn app_test_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("gsdv_app_test_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// 构造 logical/physical key 不一致的快捷键输入。
fn physical_key_input(
    logical_key: egui::Key,
    physical_key: egui::Key,
    modifiers: egui::Modifiers,
) -> egui::InputState {
    let mut input = egui::InputState::default();
    input.modifiers = modifiers;
    input.events.push(egui::Event::Key {
        key: logical_key,
        physical_key: Some(physical_key),
        pressed: true,
        repeat: false,
        modifiers,
    });
    input
}

#[test]
fn normalize_rename_name_preserves_markdown_extension() {
    assert_eq!(
        normalize_rename_name(Path::new("release-plan.md"), "notes"),
        "notes.md"
    );
    assert_eq!(
        normalize_rename_name(Path::new("release-plan.md"), "notes.md"),
        "notes.md"
    );
    assert_eq!(
        normalize_rename_name(Path::new("release-plan.md"), "   "),
        "release-plan.md"
    );
}

/// Verifies Markdown outline extraction keeps real ATX headings only.
#[test]
fn markdown_outline_entries_skip_code_fences_and_plain_hashes() {
    let entries = markdown_outline_entries(
        "# Title\n\
         text\n\
         ### Deep title ###\n\
         ```\n\
         # Not a heading\n\
         ```\n\
         ####### Too deep\n\
         #NoSpace\n",
    );

    assert_eq!(
        entries,
        vec![
            MarkdownOutlineEntry {
                level: 1,
                line: 0,
                title: "Title".to_string(),
            },
            MarkdownOutlineEntry {
                level: 3,
                line: 2,
                title: "Deep title".to_string(),
            },
        ]
    );
}

/// Verifies outline scroll targets move downward with source lines.
#[test]
fn markdown_outline_scroll_y_uses_source_line() {
    assert_eq!(markdown_outline_scroll_y(0, None), 0.0);
    assert_eq!(markdown_outline_scroll_y(2, None), 0.0);
    assert!(markdown_outline_scroll_y(12, None) > markdown_outline_scroll_y(2, None));
    assert_eq!(markdown_outline_scroll_y(40, Some(&72.0)), 24.0);
    assert_eq!(markdown_scroll_max_y(900.0, 240.0), 660.0);
    assert_eq!(markdown_scroll_max_y(120.0, 240.0), 0.0);
}

/// Verifies the active outline entry follows the heading closest to the viewport.
#[test]
fn markdown_outline_active_index_uses_nearest_heading_to_scroll() {
    let entries = markdown_outline_entries("# One\nbody\n## Two\nbody\n## Three\n");

    assert_eq!(markdown_outline_active_index(&entries, None, 0.0), Some(0));
    assert_eq!(markdown_outline_active_index(&entries, None, 23.0), Some(0));
    assert_eq!(markdown_outline_active_index(&entries, None, 25.0), Some(1));
    assert_eq!(markdown_outline_active_index(&entries, None, 47.0), Some(1));
    assert_eq!(markdown_outline_active_index(&entries, None, 73.0), Some(2));
    assert_eq!(
        markdown_outline_active_index(&entries, None, 120.0),
        Some(2)
    );
    assert_eq!(
        markdown_outline_active_index(&entries, Some(&[0.0, 300.0, 360.0]), 280.0),
        Some(1)
    );
}

#[test]
fn app_icon_data_loads_embedded_png() {
    let icon = app_icon_data();

    assert_eq!(icon.width, 1024);
    assert_eq!(icon.height, 1024);
    assert_eq!(icon.rgba.len(), 1024 * 1024 * 4);
}

#[test]
fn about_metadata_strings_are_populated() {
    assert_eq!(APP_NAME, "gsdv");
    assert!(!APP_VERSION.is_empty());
    assert!(APP_DESCRIPTION.contains("egui"));
    assert!(APP_DESCRIPTION.contains("workspace"));
    assert!(APP_COPYRIGHT.contains("gsdv"));
}

/// Verifies terminal surfaces inherit the global default font size.
#[test]
fn terminal_font_size_for_kind_resolves_default_fonts() {
    let default_fonts = data::FontSurfaceSettings {
        family: data::FontFamilySetting::System,
        size: 17.0,
        system_name: Some("Primary".to_string()),
        system_path: Some("/tmp/primary.ttf".to_string()),
        ..data::FontSurfaceSettings::default()
    };
    let settings = FontSettings {
        default_fonts: default_fonts.clone(),
        agent: data::FontSurfaceSettings {
            family: data::FontFamilySetting::Default,
            ..data::FontSurfaceSettings::default()
        },
        terminal: data::FontSurfaceSettings {
            family: data::FontFamilySetting::Default,
            ..data::FontSurfaceSettings::default()
        },
        editor: data::FontSurfaceSettings::default(),
    };

    assert_eq!(
        terminal_font_size_for_kind(&settings, TerminalSurfaceKind::Agent),
        default_fonts.size
    );
    assert_eq!(
        terminal_font_size_for_kind(&settings, TerminalSurfaceKind::Workspace),
        default_fonts.size
    );
    assert_eq!(
        terminal_font_size_for_kind(&settings, TerminalSurfaceKind::Helix),
        default_fonts.size
    );
}

/// 验证 Markdown editor 继承默认字体链时仍使用自己的字号。
#[test]
fn effective_editor_font_id_resolves_default_fonts() {
    let settings = FontSettings {
        default_fonts: data::FontSurfaceSettings {
            family: data::FontFamilySetting::System,
            size: 18.0,
            system_name: Some("Primary".to_string()),
            system_path: Some("/tmp/primary.ttf".to_string()),
            ..data::FontSurfaceSettings::default()
        },
        editor: data::FontSurfaceSettings {
            family: data::FontFamilySetting::Default,
            size: 15.0,
            ..data::FontSurfaceSettings::default()
        },
        agent: data::FontSurfaceSettings::default(),
        terminal: data::FontSurfaceSettings::default(),
    };

    let font = effective_editor_font_id(&settings);

    assert_eq!(font.size, 15.0);
    assert_eq!(font.family, theme::editor_system_font_family());
}

/// 验证 surface 会继承全局 fallback 字体。
#[test]
fn effective_font_surface_settings_inherits_default_fallback() {
    let settings = FontSettings {
        default_fonts: data::FontSurfaceSettings {
            family: data::FontFamilySetting::System,
            size: 17.0,
            system_name: Some("Primary".to_string()),
            system_path: Some("/tmp/primary.ttf".to_string()),
            fallback_system_name: Some("Fallback".to_string()),
            fallback_system_path: Some("/tmp/fallback.ttf".to_string()),
        },
        agent: data::FontSurfaceSettings {
            family: data::FontFamilySetting::System,
            size: 15.0,
            system_name: Some("Agent".to_string()),
            system_path: Some("/tmp/agent.ttf".to_string()),
            ..data::FontSurfaceSettings::default()
        },
        terminal: data::FontSurfaceSettings::default(),
        editor: data::FontSurfaceSettings::default(),
    };

    let resolved = effective_font_surface_settings(&settings, &settings.agent);

    assert_eq!(resolved.system_path.as_deref(), Some("/tmp/agent.ttf"));
    assert_eq!(
        resolved.fallback_system_path.as_deref(),
        Some("/tmp/fallback.ttf")
    );
}

/// Verifies first-run font settings select primary and CJK fallback defaults.
#[test]
fn initial_font_settings_selects_primary_and_cjk_fallback() {
    let fonts = vec![
        SystemFontEntry {
            name: "NotoSansCJK VF".to_string(),
            path: PathBuf::from("/usr/share/fonts/NotoSansCJK-VF.ttc"),
            size_bytes: 120_000_000,
            search_key: "notosanscjk vf".to_string(),
        },
        SystemFontEntry {
            name: "FiraCode Regular".to_string(),
            path: PathBuf::from("/usr/share/fonts/FiraCode-Regular.ttf"),
            size_bytes: 2_000_000,
            search_key: "firacode regular".to_string(),
        },
    ];

    let settings = initial_font_settings(&fonts);

    assert_eq!(
        settings.default_fonts.family,
        data::FontFamilySetting::System
    );
    assert_eq!(
        settings.default_fonts.system_path.as_deref(),
        Some("/usr/share/fonts/FiraCode-Regular.ttf")
    );
    assert_eq!(
        settings.default_fonts.fallback_system_path.as_deref(),
        Some("/usr/share/fonts/NotoSansCJK-VF.ttc")
    );
    assert_eq!(settings.agent.family, data::FontFamilySetting::Default);
    assert_eq!(settings.terminal.family, data::FontFamilySetting::Default);
    assert_eq!(settings.editor.family, data::FontFamilySetting::Default);
}

/// 验证 CJK fallback 沿用旧 fallback 的集合字体优先级。
#[test]
fn preferred_fallback_font_matches_old_fallback_priority() {
    let fonts = vec![
        SystemFontEntry {
            name: "NotoSansCJK VF".to_string(),
            path: PathBuf::from("/usr/share/fonts/NotoSansCJK-VF.ttc"),
            size_bytes: 120_000_000,
            search_key: "notosanscjk vf".to_string(),
        },
        SystemFontEntry {
            name: "NotoSansSC Regular".to_string(),
            path: PathBuf::from("/usr/share/fonts/NotoSansSC-Regular.otf"),
            size_bytes: 12_000_000,
            search_key: "notosanssc regular".to_string(),
        },
        SystemFontEntry {
            name: "FiraCode Regular".to_string(),
            path: PathBuf::from("/usr/share/fonts/FiraCode-Regular.ttf"),
            size_bytes: 2_000_000,
            search_key: "firacode regular".to_string(),
        },
    ];

    let selected = preferred_fallback_font(&fonts).unwrap();

    assert_eq!(selected.name, "NotoSansCJK VF");
}

/// 验证旧 fallback 逻辑不会在同关键字里按文件大小重排。
#[test]
fn preferred_fallback_font_keeps_first_match_for_same_keyword() {
    let fonts = vec![
        SystemFontEntry {
            name: "NotoSansCJK Large".to_string(),
            path: PathBuf::from("/usr/share/fonts/NotoSansCJK-Large.ttc"),
            size_bytes: 120_000_000,
            search_key: "notosanscjk large".to_string(),
        },
        SystemFontEntry {
            name: "NotoSansCJK Small".to_string(),
            path: PathBuf::from("/usr/share/fonts/NotoSansCJK-Small.ttc"),
            size_bytes: 12_000_000,
            search_key: "notosanscjk small".to_string(),
        },
    ];

    let selected = preferred_fallback_font(&fonts).unwrap();

    assert_eq!(selected.name, "NotoSansCJK Large");
}

/// 验证 macOS 历史 fallback 字体优先于无关 Noto 字体。
#[test]
fn preferred_fallback_font_includes_macos_history_names() {
    let fonts = vec![
        SystemFontEntry {
            name: "NotoSansSiddham Regular".to_string(),
            path: PathBuf::from("/System/Library/Fonts/Supplemental/NotoSansSiddham-Regular.otf"),
            size_bytes: 2_000_000,
            search_key: "notosanssiddham regular".to_string(),
        },
        SystemFontEntry {
            name: "Hiragino Sans GB".to_string(),
            path: PathBuf::from("/System/Library/Fonts/Hiragino Sans GB.ttc"),
            size_bytes: 20_000_000,
            search_key: "hiragino sans gb".to_string(),
        },
        SystemFontEntry {
            name: "Arial Unicode".to_string(),
            path: PathBuf::from("/System/Library/Fonts/Supplemental/Arial Unicode.ttf"),
            size_bytes: 23_000_000,
            search_key: "arial unicode".to_string(),
        },
    ];

    let selected = preferred_fallback_font(&fonts).unwrap();

    assert_eq!(selected.name, "Arial Unicode");
}

/// 验证 normalize 不覆盖已经明确选择的 system 字体。
#[test]
fn normalize_font_settings_preserves_selected_system_font() {
    let fonts = vec![SystemFontEntry {
        name: "NotoSansSC Regular".to_string(),
        path: PathBuf::from("/usr/share/fonts/NotoSansSC-Regular.otf"),
        size_bytes: 12_000_000,
        search_key: "notosanssc regular".to_string(),
    }];
    let mut settings = FontSettings {
        default_fonts: data::FontSurfaceSettings {
            family: data::FontFamilySetting::System,
            system_name: Some("NotoSansSC Regular".to_string()),
            system_path: Some("/usr/share/fonts/NotoSansSC-Regular.otf".to_string()),
            ..data::FontSurfaceSettings::default()
        },
        agent: data::FontSurfaceSettings {
            family: data::FontFamilySetting::Default,
            ..data::FontSurfaceSettings::default()
        },
        terminal: data::FontSurfaceSettings {
            family: data::FontFamilySetting::Default,
            ..data::FontSurfaceSettings::default()
        },
        editor: data::FontSurfaceSettings {
            family: data::FontFamilySetting::Default,
            ..data::FontSurfaceSettings::default()
        },
    };

    assert!(normalize_font_settings(&mut settings, &fonts));

    assert_eq!(
        settings.default_fonts.system_path.as_deref(),
        Some("/usr/share/fonts/NotoSansSC-Regular.otf")
    );
    assert_eq!(
        settings.default_fonts.fallback_system_path.as_deref(),
        Some("/usr/share/fonts/NotoSansSC-Regular.otf")
    );
}

/// 验证已有 store 缺 fallback 时会自动补上 CJK fallback。
#[test]
fn normalize_font_settings_adds_missing_default_fallback() {
    let fonts = vec![SystemFontEntry {
        name: "Arial Unicode".to_string(),
        path: PathBuf::from("/System/Library/Fonts/Supplemental/Arial Unicode.ttf"),
        size_bytes: 23_000_000,
        search_key: "arial unicode".to_string(),
    }];
    let mut settings = FontSettings {
        default_fonts: data::FontSurfaceSettings {
            family: data::FontFamilySetting::Monospace,
            ..data::FontSurfaceSettings::default()
        },
        agent: data::FontSurfaceSettings {
            family: data::FontFamilySetting::Default,
            ..data::FontSurfaceSettings::default()
        },
        terminal: data::FontSurfaceSettings {
            family: data::FontFamilySetting::Default,
            ..data::FontSurfaceSettings::default()
        },
        editor: data::FontSurfaceSettings {
            family: data::FontFamilySetting::Default,
            ..data::FontSurfaceSettings::default()
        },
    };

    assert!(normalize_font_settings(&mut settings, &fonts));

    assert_eq!(
        settings.default_fonts.fallback_system_path.as_deref(),
        Some("/System/Library/Fonts/Supplemental/Arial Unicode.ttf")
    );
}

/// 验证跨平台旧 fallback 路径不可用时会换成当前系统字体。
#[test]
fn normalize_font_settings_replaces_unavailable_default_fallback() {
    let fonts = vec![SystemFontEntry {
        name: "msyh".to_string(),
        path: PathBuf::from("C:/Windows/Fonts/msyh.ttc"),
        size_bytes: 18_000_000,
        search_key: "msyh".to_string(),
    }];
    let mut settings = FontSettings {
        default_fonts: data::FontSurfaceSettings {
            family: data::FontFamilySetting::Monospace,
            fallback_system_name: Some("PingFang".to_string()),
            fallback_system_path: Some("/System/Library/Fonts/PingFang.ttc".to_string()),
            ..data::FontSurfaceSettings::default()
        },
        agent: data::FontSurfaceSettings {
            family: data::FontFamilySetting::Default,
            ..data::FontSurfaceSettings::default()
        },
        terminal: data::FontSurfaceSettings {
            family: data::FontFamilySetting::Default,
            ..data::FontSurfaceSettings::default()
        },
        editor: data::FontSurfaceSettings {
            family: data::FontFamilySetting::Default,
            ..data::FontSurfaceSettings::default()
        },
    };

    assert!(normalize_font_settings(&mut settings, &fonts));

    assert_eq!(
        settings.default_fonts.fallback_system_name.as_deref(),
        Some("msyh")
    );
    assert_eq!(
        settings.default_fonts.fallback_system_path.as_deref(),
        Some("C:/Windows/Fonts/msyh.ttc")
    );
}

/// 验证 default surface 会清理不该保留的字体路径。
#[test]
fn normalize_font_settings_clears_stale_default_surface_paths() {
    let fonts = Vec::new();
    let mut settings = FontSettings {
        default_fonts: data::FontSurfaceSettings::default(),
        agent: data::FontSurfaceSettings {
            family: data::FontFamilySetting::Default,
            system_name: Some("Old".to_string()),
            system_path: Some("/tmp/old.ttf".to_string()),
            fallback_system_name: Some("Fallback".to_string()),
            fallback_system_path: Some("/tmp/fallback.ttf".to_string()),
            ..data::FontSurfaceSettings::default()
        },
        terminal: data::FontSurfaceSettings::default(),
        editor: data::FontSurfaceSettings::default(),
    };

    assert!(normalize_font_settings(&mut settings, &fonts));

    assert_eq!(settings.agent.system_path, None);
    assert_eq!(settings.agent.fallback_system_path, None);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_about_metadata_can_be_configured() {
    configure_platform_about_metadata();
}

#[test]
fn normalize_rename_name_does_not_force_extension_for_directories() {
    assert_eq!(
        normalize_rename_name(Path::new("docs"), "archive"),
        "archive"
    );
}

#[test]
fn filtered_reviewer_rows_keeps_original_indices_after_filtering() {
    let rows = vec![
        GuiReviewerRow {
            label: "docs".to_string(),
            selected: false,
            tone: crate::reviewer::app::GuiReviewerRowTone::Normal,
            tree: None,
            script_target: None,
        },
        GuiReviewerRow {
            label: "release-plan.md".to_string(),
            selected: true,
            tone: crate::reviewer::app::GuiReviewerRowTone::Normal,
            tree: None,
            script_target: None,
        },
        GuiReviewerRow {
            label: "README.md".to_string(),
            selected: false,
            tone: crate::reviewer::app::GuiReviewerRowTone::Normal,
            tree: None,
            script_target: None,
        },
    ];

    let filtered = filtered_reviewer_rows(&rows, "release");

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].0, 1);
    assert_eq!(filtered[0].1.label, "release-plan.md");
    assert!(filtered[0].1.selected);
}

#[test]
fn filtered_reviewer_rows_is_case_insensitive_and_trims_filter() {
    let rows = vec![
        GuiReviewerRow {
            label: "README.md".to_string(),
            selected: false,
            tone: crate::reviewer::app::GuiReviewerRowTone::Normal,
            tree: None,
            script_target: None,
        },
        GuiReviewerRow {
            label: "vision.md".to_string(),
            selected: false,
            tone: crate::reviewer::app::GuiReviewerRowTone::Normal,
            tree: None,
            script_target: None,
        },
    ];

    let filtered = filtered_reviewer_rows(&rows, " read ");

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].0, 0);
}

#[test]
fn opening_document_preserves_preview_mode_only() {
    assert_eq!(document_open_mode(CenterMode::Preview), CenterMode::Preview);
    assert_eq!(document_open_mode(CenterMode::Editor), CenterMode::Editor);
    assert_eq!(document_open_mode(CenterMode::Agent), CenterMode::Editor);
    assert_eq!(document_open_mode(CenterMode::Terminal), CenterMode::Editor);
}

#[test]
fn agent_markdown_toggle_preserves_last_markdown_mode() {
    assert_eq!(
        agent_markdown_toggle_modes(CenterMode::Preview, CenterMode::Agent),
        (CenterMode::Agent, CenterMode::Preview)
    );
    assert_eq!(
        agent_markdown_toggle_modes(CenterMode::Agent, CenterMode::Preview),
        (CenterMode::Preview, CenterMode::Preview)
    );
    assert_eq!(
        agent_markdown_toggle_modes(CenterMode::Agent, CenterMode::Agent),
        (CenterMode::Editor, CenterMode::Agent)
    );
}

#[test]
fn preview_scroll_capture_offset_parses_and_clamps() {
    assert_eq!(
        preview_scroll_capture_offset("preview-scroll:1200"),
        Some(1200.0)
    );
    assert_eq!(
        preview_scroll_capture_offset("preview-scroll: 480.5"),
        Some(480.5)
    );
    assert_eq!(
        preview_scroll_capture_offset("preview-scroll:-20"),
        Some(0.0)
    );
    assert_eq!(preview_scroll_capture_offset("preview"), None);
    assert_eq!(preview_scroll_capture_offset("preview-scroll:nope"), None);
}

/// 验证工作预警哈基米复用休息模式的弹跳移动。
#[test]
fn pomodoro_warning_cat_uses_rest_bounce_motion() {
    let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(800.0, 600.0));
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![test_workspace()],
        rail_collapsed: false,
    });
    let start = Instant::now();
    app.pomodoro.cat_pos = egui::pos2(200.0, 160.0);
    app.pomodoro.cat_velocity = Vec2::new(100.0, 50.0);
    app.pomodoro.last_animation_at = start;

    app.animate_pomodoro_cat(screen, start + Duration::from_millis(50));

    assert!(app.pomodoro.cat_pos.x > 200.0);
    assert!(app.pomodoro.cat_pos.y > 160.0);
}

/// 验证哈基米预警阈值按剩余百分比换算。
#[test]
fn pomodoro_warning_progress_uses_remaining_percent() {
    let mut settings = RuntimeSettings::default();
    settings.pomodoro_warning_remaining_percent = 20;

    assert_eq!(pomodoro_warning_progress(&settings), 0.8);

    settings.pomodoro_warning_remaining_percent = 35;
    assert_eq!(pomodoro_warning_progress(&settings), 0.65);
}

/// 验证工作末段提示文字围绕哈基米旋转。
#[test]
fn pomodoro_peek_orbit_text_positions_spin_around_cat() {
    let cat_rect = Rect::from_min_size(egui::pos2(100.0, 100.0), POMODORO_CAT_SIZE);

    let first = pomodoro_peek_orbit_text_positions(cat_rect, Duration::from_millis(100));
    let second = pomodoro_peek_orbit_text_positions(cat_rect, Duration::from_millis(900));

    assert_eq!(first.len(), POMODORO_PEEK_ORBIT_TEXT.chars().count());
    assert_eq!(second.len(), POMODORO_PEEK_ORBIT_TEXT.chars().count());
    assert_ne!(first, second);
}

/// 验证工作自动到点后直接开始休息。
#[test]
fn pomodoro_work_timeout_starts_rest_directly() {
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![test_workspace()],
        rail_collapsed: false,
    });
    app.pomodoro.phase_started_at =
        Instant::now() - pomodoro_work_duration(&app.runtime_settings) - Duration::from_secs(1);

    app.process_pomodoro_state(&egui::Context::default());

    assert_eq!(app.pomodoro.phase, PomodoroPhase::Resting);
}

/// 验证休息前冷静期结束后才重置并开始休息。
#[test]
fn pomodoro_quiet_wait_starts_rest_after_ten_silent_seconds() {
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![test_workspace()],
        rail_collapsed: false,
    });
    app.pomodoro
        .wait_for_rest_quiet(Instant::now() - POMODORO_REST_QUIET_DURATION);

    app.process_pomodoro_state(&egui::Context::default());

    assert_eq!(app.pomodoro.phase, PomodoroPhase::Resting);
    assert!(app.pomodoro.phase_started_at.elapsed() < Duration::from_secs(1));
}

/// 验证休息期间任意输入会回到等待安静。
#[test]
fn pomodoro_resting_input_returns_to_quiet_wait() {
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![test_workspace()],
        rail_collapsed: false,
    });
    app.pomodoro
        .start_resting(Instant::now() - Duration::from_secs(30));
    let ctx = egui::Context::default();
    ctx.input_mut(|input| {
        input
            .events
            .push(egui::Event::PointerMoved(egui::pos2(1.0, 1.0)));
    });

    let mut request = app
        .input_runtime_request(&ctx, ctx.input(Clone::clone))
        .unwrap();
    request.terminal_surface_owns_input = false;
    let events = process_input_runtime_request(request);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::PomodoroInputDetected))
    );
    for event in events {
        app.handle_app_event(&ctx, event);
    }

    assert_eq!(app.pomodoro.phase, PomodoroPhase::WaitingForRestQuiet);
    assert!(app.pomodoro.phase_started_at.elapsed() < Duration::from_secs(1));
}

/// 验证 Cmd+B 只在 Agent 主界面打开外置工具。
#[test]
fn extra_tools_shortcut_only_opens_from_agent_workspace() {
    let mut workspace = test_workspace();
    workspace.center_mode = CenterMode::Agent;
    workspace.route = Route::Workspace;
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![workspace],
        rail_collapsed: false,
    });
    let ctx = egui::Context::default();
    ctx.input_mut(|input| {
        *input = key_input(egui::Key::B, egui::Modifiers::COMMAND);
    });

    let mut request = app
        .input_runtime_request(&ctx, ctx.input(Clone::clone))
        .unwrap();
    request.terminal_surface_owns_input = false;
    let events = process_input_runtime_request(request);
    assert!(
        events.iter().any(|event| {
            matches!(event, AppEvent::InputUiCommand(UiCommand::ToggleExtraTools))
        })
    );

    app.workspaces[0].center_mode = CenterMode::Editor;
    let mut request = app
        .input_runtime_request(&ctx, ctx.input(Clone::clone))
        .unwrap();
    request.terminal_surface_owns_input = false;
    let events = process_input_runtime_request(request);
    assert!(
        !events.iter().any(|event| {
            matches!(event, AppEvent::InputUiCommand(UiCommand::ToggleExtraTools))
        })
    );

    app.workspaces[0].center_mode = CenterMode::Agent;
    app.workspace_terminal_drawers[0] = true;
    let mut request = app
        .input_runtime_request(&ctx, ctx.input(Clone::clone))
        .unwrap();
    request.terminal_surface_owns_input = false;
    let events = process_input_runtime_request(request);
    assert!(
        !events.iter().any(|event| {
            matches!(event, AppEvent::InputUiCommand(UiCommand::ToggleExtraTools))
        })
    );
}

/// 验证 Extra Tools 抽屉保留 T/W 两个抽屉级快捷键。
#[test]
fn extra_tools_drawer_keeps_terminal_and_agent_shortcuts() {
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![test_workspace()],
        rail_collapsed: false,
    });
    app.extra_tools.open = true;
    let ctx = egui::Context::default();

    ctx.input_mut(|input| {
        *input = key_input(egui::Key::T, egui::Modifiers::COMMAND);
    });
    let request = app
        .input_runtime_request(&ctx, ctx.input(Clone::clone))
        .unwrap();
    let events = process_input_runtime_request(request);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            AppEvent::InputUiCommand(UiCommand::ToggleWorkspaceTerminal)
        )
    }));

    ctx.input_mut(|input| {
        *input = key_input(egui::Key::W, egui::Modifiers::COMMAND);
    });
    let request = app
        .input_runtime_request(&ctx, ctx.input(Clone::clone))
        .unwrap();
    let events = process_input_runtime_request(request);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            AppEvent::InputUiCommand(UiCommand::AgentMarkdownShortcut)
        )
    }));
}

/// 验证外置工具 metadata 协议字段可按文本解析。
#[test]
fn extra_tool_metadata_parses_protocol_text() {
    let metadata = parse_extra_tool_metadata(
        "type: card\n\
         action.1.key: stop\n\
         input_need: true\n\
         input_value: seed\n\
         input_rows: 3\n\
         refresh: 2.5\n\
         action.0.key: start\n",
    )
    .unwrap();

    assert_eq!(metadata.tool_type, ExtraToolType::Card);
    assert_eq!(
        metadata.actions,
        vec!["start".to_string(), "stop".to_string()]
    );
    assert!(metadata.input_need);
    assert_eq!(metadata.input_value, "seed");
    assert_eq!(metadata.input_rows, 3);
    assert_eq!(metadata.refresh, 2.5);
}

/// 验证 switch 类型的 metadata 可解析。
#[test]
fn extra_tool_metadata_parses_switch_type() {
    let metadata = parse_extra_tool_metadata(
        "type: switch\n\
         input_need: false\n\
         refresh: 10\n",
    )
    .unwrap();

    assert_eq!(metadata.tool_type, ExtraToolType::Switch);
    assert_eq!(metadata.actions, Vec::<String>::new());
    assert!(!metadata.input_need);
    assert_eq!(metadata.refresh, 10.0);
}

/// 验证冷静期不会打断哈基米的弹跳移动。
#[test]
fn pomodoro_quiet_wait_keeps_cat_moving() {
    let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(800.0, 600.0));
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![test_workspace()],
        rail_collapsed: false,
    });
    let start = Instant::now();
    app.pomodoro.start_resting(start);
    app.pomodoro.cat_pos = egui::pos2(200.0, 160.0);
    app.pomodoro.cat_velocity = Vec2::new(100.0, 50.0);
    app.pomodoro.last_animation_at = start;
    app.pomodoro
        .wait_for_rest_quiet(start + Duration::from_millis(10));

    app.animate_pomodoro_cat(screen, start + Duration::from_millis(60));

    assert!(app.pomodoro.cat_pos.x > 200.0);
    assert!(app.pomodoro.cat_pos.y > 160.0);
}

/// 验证冷静期输入只重置倒计时，不重置问号动画。
#[test]
fn pomodoro_quiet_wait_input_keeps_question_animation_clock() {
    let mut state = PomodoroState::new(Instant::now());
    let start = Instant::now();
    state.start_resting(start);
    state.wait_for_rest_quiet(start + Duration::from_millis(10));
    let animation_started_at = state.rest_quiet_animation_started_at;

    state.wait_for_rest_quiet(start + Duration::from_millis(600));

    assert_eq!(state.phase, PomodoroPhase::WaitingForRestQuiet);
    assert_eq!(state.phase_started_at, start + Duration::from_millis(600));
    assert_eq!(state.rest_quiet_animation_started_at, animation_started_at);
}

/// 验证等待安静阶段直接显示 5 个旋转问号。
#[test]
fn pomodoro_rest_quiet_questions_are_five_and_spin() {
    let cat_rect = Rect::from_min_size(egui::pos2(100.0, 100.0), POMODORO_CAT_SIZE);

    let first = pomodoro_rest_quiet_question_positions(cat_rect, Duration::from_millis(100));
    let second = pomodoro_rest_quiet_question_positions(cat_rect, Duration::from_millis(900));

    assert_eq!(first.len(), POMODORO_REST_QUIET_QUESTION_COUNT);
    assert_eq!(second.len(), POMODORO_REST_QUIET_QUESTION_COUNT);
    assert_ne!(first, second);
}

/// 验证休息结束后输入会先进入 3 秒退场动画。
#[test]
fn pomodoro_ready_input_starts_return_animation_before_working() {
    let now = Instant::now();
    let mut state = PomodoroState::new(now);
    state.wait_for_work_input(now);

    state.start_returning_to_work(now + Duration::from_millis(10));

    assert_eq!(state.phase, PomodoroPhase::ReturningToWork);
    assert!(state.meows.is_empty());
}

/// 验证退场动画前段最多爬到 5 个问号。
#[test]
fn pomodoro_return_question_count_ramps_to_five() {
    assert_eq!(pomodoro_return_question_count(Duration::ZERO), 1);
    assert_eq!(
        pomodoro_return_question_count(Duration::from_millis(300)),
        2
    );
    assert_eq!(
        pomodoro_return_question_count(Duration::from_millis(600)),
        3
    );
    assert_eq!(
        pomodoro_return_question_count(Duration::from_millis(900)),
        4
    );
    assert_eq!(
        pomodoro_return_question_count(POMODORO_RETURN_QUESTION_RAMP),
        POMODORO_RETURN_QUESTION_COUNT
    );
    assert_eq!(
        pomodoro_return_question_count(POMODORO_RETURN_TO_WORK_DURATION),
        POMODORO_RETURN_QUESTION_COUNT
    );
}

/// 验证退场动画后段 5 个问号围成一圈并随时间旋转。
#[test]
fn pomodoro_return_question_positions_spin_after_ramp() {
    let cat_rect = Rect::from_min_size(egui::pos2(100.0, 100.0), POMODORO_CAT_SIZE);
    let first = pomodoro_return_question_positions(cat_rect, Duration::from_millis(1_300));
    let second = pomodoro_return_question_positions(cat_rect, Duration::from_millis(1_650));

    assert_eq!(first.len(), POMODORO_RETURN_QUESTION_COUNT);
    assert_eq!(second.len(), POMODORO_RETURN_QUESTION_COUNT);
    assert_ne!(first, second);
}

#[test]
fn reviewer_shortcuts_open_from_workspace_and_r_shortcut_exits_from_reviewer() {
    assert_eq!(
        reviewer_shortcut_action(true, false, false, true, false),
        Some(ReviewerShortcutAction::Open)
    );
    assert_eq!(
        reviewer_shortcut_action(true, false, true, false, false),
        Some(ReviewerShortcutAction::Open)
    );
    assert_eq!(
        reviewer_shortcut_action(false, true, true, false, false),
        Some(ReviewerShortcutAction::Open)
    );
    assert_eq!(
        reviewer_shortcut_action(true, false, true, false, true),
        Some(ReviewerShortcutAction::Exit)
    );
    assert_eq!(
        reviewer_shortcut_action(false, true, true, false, true),
        Some(ReviewerShortcutAction::Exit)
    );
    assert_eq!(
        reviewer_shortcut_action(false, false, true, false, false),
        None
    );
    assert_eq!(
        reviewer_shortcut_action(false, true, false, true, false),
        None
    );
}

#[test]
fn help_shortcut_accepts_command_or_alt_period_only() {
    assert!(help_shortcut_action(true, false, true));
    assert!(help_shortcut_action(false, true, true));
    assert!(!help_shortcut_action(false, false, true));
    assert!(!help_shortcut_action(true, false, false));
    assert!(!help_shortcut_action(false, true, false));
}

#[test]
fn notification_center_keeps_latest_two_thousand_lines() {
    let mut notifications = NotificationCenter::default();

    for index in 0..(NOTIFICATION_MAX_LINES + 3) {
        notifications.push_line(format!("line {index}"));
    }

    assert_eq!(notifications.lines.len(), NOTIFICATION_MAX_LINES);
    assert_eq!(notifications.lines.front().unwrap(), "line 3");
    assert_eq!(
        notifications.lines.back().unwrap(),
        &format!("line {}", NOTIFICATION_MAX_LINES + 2)
    );
    assert!(notifications.scroll_to_bottom);
}

#[test]
fn notification_toggle_opens_at_bottom() {
    let mut notifications = NotificationCenter::default();

    notifications.toggle();

    assert!(notifications.open);
    assert!(notifications.scroll_to_bottom);
    notifications.close();
    assert!(!notifications.open);
}

#[test]
fn ui_command_reader_keeps_event_consumption_separate_from_routing() {
    let mut modifiers = egui::Modifiers::NONE;
    modifiers.alt = true;
    let input = key_input(egui::Key::T, modifiers);

    assert_eq!(
        read_ui_command(&input, false, false, false, false, false),
        Some(UiCommand::ToggleWorkspaceTerminal)
    );
}

#[test]
fn ui_command_reader_accepts_editor_preview_toggle_shortcut() {
    let mut modifiers = egui::Modifiers::NONE;
    modifiers.alt = true;
    let input = key_input(egui::Key::E, modifiers);

    assert_eq!(
        read_ui_command(&input, false, false, false, false, false),
        Some(UiCommand::ToggleMarkdownEditorPreview)
    );
}

#[test]
fn editor_preview_toggle_shortcut_uses_event_modifiers_before_text_routing() {
    let mut modifiers = egui::Modifiers::NONE;
    modifiers.alt = true;
    let mut input = egui::InputState::default();
    input.events.push(egui::Event::Key {
        key: egui::Key::E,
        physical_key: Some(egui::Key::E),
        pressed: true,
        repeat: false,
        modifiers,
    });
    input.events.push(egui::Event::Text("e".to_string()));

    assert_eq!(
        read_ui_command(&input, false, false, false, false, false),
        Some(UiCommand::ToggleMarkdownEditorPreview)
    );
}

#[test]
fn ui_command_reader_does_not_consume_escape_without_closeable_layer() {
    let input = key_input(egui::Key::Escape, egui::Modifiers::NONE);

    assert_eq!(
        read_ui_command(&input, false, false, false, false, false),
        None
    );
    assert_eq!(
        read_ui_command(&input, false, true, false, false, false),
        Some(UiCommand::CloseTopLayer)
    );
}

#[test]
fn helix_shortcut_uses_key_chord_not_text_cut_event() {
    let mut modifiers = egui::Modifiers::NONE;
    modifiers.command = true;
    let input = key_input(egui::Key::X, modifiers);

    assert_eq!(
        read_ui_command(&input, false, false, false, false, true),
        Some(UiCommand::ToggleReviewerHelix)
    );

    let mut cut_input = egui::InputState::default();
    cut_input.events.push(egui::Event::Cut);
    assert_eq!(
        read_ui_command(&cut_input, false, false, false, false, true),
        None
    );

    let mut cut_key_input = key_input(egui::Key::X, modifiers);
    cut_key_input.events.insert(0, egui::Event::Cut);
    assert_eq!(
        read_ui_command(&cut_key_input, false, false, false, false, true),
        Some(UiCommand::ToggleReviewerHelix)
    );

    let mut cut_only_input = egui::InputState::default();
    cut_only_input.modifiers = modifiers;
    cut_only_input.events.push(egui::Event::Cut);
    assert_eq!(
        read_ui_command(&cut_only_input, false, false, false, false, true),
        Some(UiCommand::ToggleReviewerHelix)
    );

    let mut mac_cmd_modifiers = egui::Modifiers::NONE;
    mac_cmd_modifiers.mac_cmd = true;
    let mac_cmd_input = key_input(egui::Key::X, mac_cmd_modifiers);
    assert_eq!(
        read_ui_command(&mac_cmd_input, false, false, false, false, true),
        Some(UiCommand::ToggleReviewerHelix)
    );
}

#[test]
fn text_edit_pre_global_consumes_cut_before_route_shortcuts() {
    let mut modifiers = egui::Modifiers::NONE;
    modifiers.command = true;
    let input = key_input(egui::Key::X, modifiers);
    let mut ctrl_modifiers = egui::Modifiers::NONE;
    ctrl_modifiers.ctrl = true;
    let ctrl_input = key_input(egui::Key::X, ctrl_modifiers);
    let undo_input = key_input(egui::Key::Z, modifiers);
    let mut alt_modifiers = egui::Modifiers::NONE;
    alt_modifiers.alt = true;
    let alt_undo_input = key_input(egui::Key::Z, alt_modifiers);

    assert!(text_edit_pre_global_shortcut_consumed(&input, true));
    assert!(text_edit_pre_global_shortcut_consumed(&ctrl_input, true));
    assert!(text_edit_pre_global_shortcut_consumed(&undo_input, true));
    assert!(!text_edit_pre_global_shortcut_consumed(
        &alt_undo_input,
        true
    ));
    assert!(!text_edit_pre_global_shortcut_consumed(&input, false));
}

#[test]
fn text_edit_pre_global_does_not_consume_alt_x_helix_shortcut() {
    let mut modifiers = egui::Modifiers::NONE;
    modifiers.alt = true;
    let input = key_input(egui::Key::X, modifiers);

    assert!(!text_edit_pre_global_shortcut_consumed(&input, true));
}

/// Verifies base route shortcuts are handled before Agent tab fallback.
#[test]
fn base_route_consumes_route_switching_shortcuts_before_agent_tab() {
    let app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![test_workspace()],
        rail_collapsed: false,
    });
    for (key, command) in [
        (egui::Key::T, UiCommand::ToggleWorkspaceTerminal),
        (egui::Key::W, UiCommand::AgentMarkdownShortcut),
        (egui::Key::Z, UiCommand::ToggleOutlineWorkflowTab),
        (egui::Key::K, UiCommand::ToggleNotifications),
        (egui::Key::X, UiCommand::ToggleReviewerHelix),
        (egui::Key::R, UiCommand::OpenReviewerRoute),
    ] {
        let mut modifiers = egui::Modifiers::NONE;
        modifiers.alt = true;
        let input = key_input(key, modifiers);

        assert_eq!(app.read_base_route_command(&input), Some(command));
    }

    let mut input = egui::InputState::default();
    input.events.push(egui::Event::Cut);
    assert_eq!(app.read_base_route_command(&input), None);
}

/// 验证 Cmd/Alt+Z 可以切换 Outline 和 Work-flow tab。
#[test]
fn outline_workflow_shortcut_toggles_tab_and_center_surface() {
    let mut workspace = test_workspace();
    workspace.center_mode = CenterMode::Agent;
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![workspace],
        rail_collapsed: false,
    });
    let task_path = PathBuf::from("gsdv-spec/ps/project1/task-a.md");
    app.workflow_states[0].tree = Some(WorkflowTree {
        spec_path: PathBuf::from("gsdv-spec"),
        root_path: PathBuf::from("gsdv-spec/root.md"),
        projects: vec![WorkflowProjectNode {
            key: "project1".to_string(),
            label: "project1".to_string(),
            root_path: PathBuf::from("gsdv-spec/ps/project1/root.md"),
            tasks: vec![WorkflowTaskNode {
                label: "a".to_string(),
                path: task_path.clone(),
                desc: "Task intro\n".to_string(),
                steps: vec![WorkflowStepNode {
                    path: vec![0],
                    title: "step one".to_string(),
                    checked: false,
                    checkable: true,
                    desc: "step desc".to_string(),
                    children: Vec::new(),
                }],
            }],
        }],
    });
    app.workflow_states[0].last_task_surface_target = Some(WorkflowSelectionTarget::Step {
        task_path: task_path.clone(),
        step_path: vec![0],
    });
    let ctx = egui::Context::default();

    assert_eq!(app.outline_panel_tabs[0], OutlinePanelTab::Outline);
    app.dispatch_ui_command(&ctx, UiCommand::ToggleOutlineWorkflowTab);
    assert_eq!(app.outline_panel_tabs[0], OutlinePanelTab::Workflow);
    assert_eq!(app.workspaces[0].center_mode, CenterMode::Editor);
    assert_eq!(
        app.workflow_states[0].selected,
        Some(WorkflowSelectionTarget::Step {
            task_path: task_path.clone(),
            step_path: vec![0],
        })
    );
    assert!(app.workflow_states[0].editor.is_some());
    assert!(app.workflow_task_surface_visible());

    app.dispatch_ui_command(&ctx, UiCommand::ToggleOutlineWorkflowTab);
    assert_eq!(app.outline_panel_tabs[0], OutlinePanelTab::Outline);
    assert_eq!(app.workspaces[0].center_mode, CenterMode::Agent);

    app.dispatch_ui_command(&ctx, UiCommand::ToggleOutlineWorkflowTab);
    assert_eq!(
        app.workflow_states[0].selected,
        Some(WorkflowSelectionTarget::Step {
            task_path,
            step_path: vec![0],
        })
    );
}

/// 验证编辑器焦点下 Cmd+Z 仍归文本编辑器撤销处理。
#[test]
fn outline_workflow_command_z_does_not_override_editor_undo() {
    let mut workspace = test_workspace();
    workspace.center_mode = CenterMode::Editor;
    let app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![workspace],
        rail_collapsed: false,
    });
    let ctx = egui::Context::default();
    ctx.input_mut(|input| {
        *input = key_input(egui::Key::Z, egui::Modifiers::COMMAND);
    });
    let mut request = app
        .input_runtime_request(&ctx, ctx.input(Clone::clone))
        .unwrap();
    request.wants_keyboard_input = true;

    let events = process_input_runtime_request(request);

    assert!(!events.iter().any(|event| {
        matches!(
            event,
            AppEvent::InputUiCommand(UiCommand::ToggleOutlineWorkflowTab)
        )
    }));
}

/// 验证 Cmd/Alt+1 会按物理数字键切换 active workspace。
#[test]
fn base_route_switches_active_workspace_from_physical_num1() {
    let app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![test_workspace()],
        rail_collapsed: false,
    });

    for modifiers in [egui::Modifiers::COMMAND, egui::Modifiers::ALT] {
        let input = physical_key_input(egui::Key::Exclamationmark, egui::Key::Num1, modifiers);

        assert_eq!(
            app.read_base_route_command(&input),
            Some(UiCommand::SwitchActiveWorkspace)
        );
    }
}

/// 验证最近编辑 Markdown tree 会保留目录并按编辑时间排序。
#[test]
fn recent_markdown_outline_tree_sorts_by_edit_time() {
    let nodes = vec![OutlineNode::Root {
        root_kind: OutlineRootKind::Workspace,
        key: PathBuf::new(),
        label: "root".to_string(),
        expanded: true,
        children: vec![
            OutlineNode::Dir {
                key: PathBuf::from("docs"),
                label: "docs".to_string(),
                expanded: false,
                children: vec![
                    OutlineNode::File {
                        path: PathBuf::from("docs/a.md"),
                        label: "a.md".to_string(),
                    },
                    OutlineNode::File {
                        path: PathBuf::from("docs/b.md"),
                        label: "b.md".to_string(),
                    },
                ],
            },
            OutlineNode::File {
                path: PathBuf::from("root.md"),
                label: "root.md".to_string(),
            },
        ],
    }];
    let recent = vec![
        data::RecentMarkdownEntry {
            path: PathBuf::from("docs/a.md"),
            edited_at_ms: 10,
        },
        data::RecentMarkdownEntry {
            path: PathBuf::from("root.md"),
            edited_at_ms: 20,
        },
        data::RecentMarkdownEntry {
            path: PathBuf::from("docs/b.md"),
            edited_at_ms: 30,
        },
    ];

    let tree = recent_markdown_outline_nodes(&nodes, &recent);

    let OutlineNode::Root { children, .. } = &tree[0] else {
        panic!("recent tree root missing");
    };
    assert!(matches!(&children[0], OutlineNode::Dir { label, .. } if label == "docs"));
    assert!(matches!(&children[1], OutlineNode::File { label, .. } if label == "root.md"));
    let OutlineNode::Dir {
        children: docs_children,
        expanded,
        ..
    } = &children[0]
    else {
        panic!("docs dir missing");
    };
    assert!(*expanded);
    assert!(matches!(&docs_children[0], OutlineNode::File { label, .. } if label == "b.md"));
    assert!(matches!(&docs_children[1], OutlineNode::File { label, .. } if label == "a.md"));
}

/// 验证 F1 只在基础 outline 可见时投递最近编辑 modal 命令。
#[test]
fn f1_recent_markdown_shortcut_requires_visible_outline() {
    let mut workspace = test_workspace();
    workspace.route = Route::Workspace;
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![workspace],
        rail_collapsed: false,
    });
    let ctx = egui::Context::default();
    ctx.input_mut(|input| {
        *input = key_input(egui::Key::F1, egui::Modifiers::NONE);
    });

    let request = app
        .input_runtime_request(&ctx, ctx.input(Clone::clone))
        .unwrap();
    let events = process_input_runtime_request(request);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            AppEvent::InputUiCommand(UiCommand::ToggleRecentMarkdownOutline)
        )
    }));

    app.workspaces[0].route = Route::Reviewer;
    let request = app
        .input_runtime_request(&ctx, ctx.input(Clone::clone))
        .unwrap();
    let events = process_input_runtime_request(request);
    assert!(!events.iter().any(|event| {
        matches!(
            event,
            AppEvent::InputUiCommand(UiCommand::ToggleRecentMarkdownOutline)
        )
    }));
}

/// 验证 reviewer route 不抢 diff viewer 的 d。
#[test]
fn reviewer_leaves_plain_d_to_diff_viewer() {
    let plain = key_input(egui::Key::D, egui::Modifiers::NONE);
    assert_eq!(read_reviewer_command(&plain), None);

    for modifiers in [
        egui::Modifiers::COMMAND,
        egui::Modifiers::CTRL,
        egui::Modifiers::ALT,
    ] {
        let modified = key_input(egui::Key::D, modifiers);
        assert_eq!(read_reviewer_command(&modified), None);
    }
}

/// 验证 reviewer 忙时重复 notify 刷新只排一个补刷任务。
#[test]
fn reviewer_refresh_uncommitted_queue_coalesces_duplicates() {
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![test_workspace()],
        rail_collapsed: false,
    });

    app.queue_reviewer_adapter_task(
        0,
        ReviewerAdapterTask::RefreshUncommitted,
        ReviewerAdapterTaskEffect::None,
    );
    app.queue_reviewer_adapter_task(
        0,
        ReviewerAdapterTask::RefreshUncommitted,
        ReviewerAdapterTaskEffect::None,
    );

    assert_eq!(
        app.pop_pending_reviewer_adapter_task(0),
        Some((
            ReviewerAdapterTask::RefreshUncommitted,
            ReviewerAdapterTaskEffect::None
        ))
    );
    assert_eq!(app.pop_pending_reviewer_adapter_task(0), None);
}

/// Verifies Agent tab local handling does not steal base route shortcuts.
#[test]
fn agent_tab_does_not_consume_route_switching_shortcuts_locally() {
    for key in [egui::Key::T, egui::Key::W, egui::Key::K, egui::Key::X] {
        let mut modifiers = egui::Modifiers::NONE;
        modifiers.alt = true;
        let input = key_input(key, modifiers);

        assert!(!agent_tab_own_shortcut_pressed(&input));
    }
}

/// 验证关闭 active workspace 后会切到左侧 workspace。
#[test]
fn close_active_workspace_selects_left_neighbor() {
    let mut left = test_workspace();
    left.name = "left".to_string();
    left.path = PathBuf::from("/tmp/left");
    let mut middle = test_workspace();
    middle.name = "middle".to_string();
    middle.path = PathBuf::from("/tmp/middle");
    let mut right = test_workspace();
    right.name = "right".to_string();
    right.path = PathBuf::from("/tmp/right");
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 1,
        workspaces: vec![left, middle, right],
        rail_collapsed: false,
    });

    app.apply_workspace_close_sidecar_result(
        &egui::Context::default(),
        1,
        PathBuf::from("/tmp/middle"),
        Ok(()),
    );

    assert_eq!(app.workspaces.len(), 2);
    assert_eq!(app.active_workspace, 0);
    assert_eq!(app.workspaces[app.active_workspace].name, "left");
    assert_eq!(app.documents.len(), app.workspaces.len());
    assert_eq!(app.terminal_hosts.len(), app.workspaces.len());
}

/// 验证关闭左侧非 active workspace 后 active 索引左移。
#[test]
fn close_left_workspace_shifts_active_index() {
    let mut left = test_workspace();
    left.name = "left".to_string();
    left.path = PathBuf::from("/tmp/left");
    let mut right = test_workspace();
    right.name = "right".to_string();
    right.path = PathBuf::from("/tmp/right");
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 1,
        workspaces: vec![left, right],
        rail_collapsed: false,
    });

    app.apply_workspace_close_sidecar_result(
        &egui::Context::default(),
        0,
        PathBuf::from("/tmp/left"),
        Ok(()),
    );

    assert_eq!(app.workspaces.len(), 1);
    assert_eq!(app.active_workspace, 0);
    assert_eq!(app.workspaces[0].name, "right");
    assert_eq!(app.app_dialogs.len(), app.workspaces.len());
    assert_eq!(app.reviewer_adapters.len(), app.workspaces.len());
}

/// 验证终端文件 Helix workdir 优先使用文件最近的 git 根目录。
#[test]
fn terminal_file_helix_workdir_uses_nearest_git_root() {
    let root = std::env::temp_dir().join(format!(
        "gsdv-terminal-file-helix-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let workspace = root.join("workspace");
    let nested = workspace.join("apps/demo");
    let file = nested.join("src/main.rs");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(nested.join(".git"), "gitdir: ../../.git/modules/demo").unwrap();

    assert_eq!(terminal_file_helix_workdir(&workspace, &file), nested);

    let _ = std::fs::remove_dir_all(root);
}

/// 验证终端文件 Helix workdir 在没有 git 根时回退 workspace。
#[test]
fn terminal_file_helix_workdir_falls_back_to_workspace_without_git() {
    let root = std::env::temp_dir().join(format!(
        "gsdv-terminal-file-helix-fallback-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let workspace = root.join("workspace");
    let file = workspace.join("src/main.rs");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();

    assert_eq!(terminal_file_helix_workdir(&workspace, &file), workspace);

    let _ = std::fs::remove_dir_all(root);
}

/// Verifies Agent tab still lets normal paste flow to the terminal.
#[test]
fn agent_tab_does_not_consume_plain_paste_events() {
    let mut input = egui::InputState::default();
    input.events.push(egui::Event::Paste("hello".to_string()));

    assert!(!agent_tab_own_shortcut_pressed(&input));
}

/// 验证中文 IME commit 会进入 Markdown editor fallback。
#[test]
fn markdown_editor_ime_commit_events_are_collected() {
    let events = vec![egui::Event::Ime(egui::ImeEvent::Commit(
        "中文，了".to_string(),
    ))];

    assert_eq!(
        markdown_editor_ime_commit_texts_from_events(&events),
        vec!["中文，了".to_string()]
    );
}

/// 验证已有 Text 事件时 IME fallback 不会重复插入。
#[test]
fn markdown_editor_ime_commit_events_skip_matching_text_event() {
    let events = vec![
        egui::Event::Ime(egui::ImeEvent::Commit("中文".to_string())),
        egui::Event::Text("中文".to_string()),
    ];

    assert!(markdown_editor_ime_commit_texts_from_events(&events).is_empty());
}

/// 验证 notify 后干净的当前 Markdown 文档会从磁盘重载。
#[test]
fn fs_watch_reload_clean_selected_markdown_document() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!("gsdv-md-reload-{unique}"));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("note.md"), "new text\n").unwrap();
    let mut workspace = test_workspace();
    workspace.path = root.clone();
    workspace.selected_file = Some(PathBuf::from("note.md"));
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![workspace],
        rail_collapsed: false,
    });
    app.documents[0].path = Some(PathBuf::from("note.md"));
    app.documents[0].text = "old text\n".to_string();
    app.documents[0].saved_text = "old text\n".to_string();
    let ctx = egui::Context::default();
    let dirty_workspaces = BTreeSet::from([0]);

    app.reload_clean_selected_documents(&ctx, &dirty_workspaces);
    for _ in 0..50 {
        app.drain_app_events(&ctx, Instant::now(), Duration::from_millis(20));
        if app.documents[0].text == "new text\n" {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(app.documents[0].text, "new text\n");
    assert_eq!(app.documents[0].saved_text, "new text\n");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn workspace_terminal_shortcut_closes_notifications_before_opening_drawer() {
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![test_workspace()],
        rail_collapsed: false,
    });
    app.open_notifications();

    app.route_to_workspace_terminal_drawer();

    assert!(!app.notifications.open);
    assert!(app.workspace_terminal_drawer_is_open());
    assert!(app.notification_return_context.is_none());
}

#[test]
fn editor_preview_toggle_only_changes_markdown_modes() {
    let mut workspace = test_workspace();
    workspace.center_mode = CenterMode::Editor;
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![workspace],
        rail_collapsed: false,
    });

    app.toggle_markdown_editor_preview();
    assert_eq!(
        app.current_workspace().unwrap().center_mode,
        CenterMode::Preview
    );
    app.toggle_markdown_editor_preview();
    assert_eq!(
        app.current_workspace().unwrap().center_mode,
        CenterMode::Editor
    );

    app.current_workspace_mut().unwrap().center_mode = CenterMode::Agent;
    app.toggle_markdown_editor_preview();
    assert_eq!(
        app.current_workspace().unwrap().center_mode,
        CenterMode::Agent
    );
}

#[test]
fn agent_markdown_shortcut_routes_to_agent_from_overlays_and_reviewer() {
    let mut workspace = test_workspace();
    workspace.route = Route::Reviewer;
    workspace.center_mode = CenterMode::Editor;
    workspace.previous_center_mode = CenterMode::Preview;
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![workspace],
        rail_collapsed: false,
    });
    app.open_notifications();
    app.workspace_terminal_drawers[0] = true;
    app.reviewer_helix_drawers[0] = true;

    app.route_agent_markdown_shortcut();

    let workspace = app.current_workspace().unwrap();
    assert_eq!(workspace.route, Route::Workspace);
    assert_eq!(workspace.center_mode, CenterMode::Agent);
    assert!(!app.notifications.open);
    assert!(!app.workspace_terminal_drawer_is_open());
    assert!(!app.reviewer_helix_drawer_is_open());
}

#[test]
fn closing_notifications_restores_context_from_when_it_opened() {
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![test_workspace()],
        rail_collapsed: false,
    });
    app.open_notifications();
    {
        let workspace = app.current_workspace_mut().unwrap();
        workspace.route = Route::Reviewer;
        workspace.center_mode = CenterMode::Preview;
    }

    app.close_notifications_restoring_route();

    let workspace = app.current_workspace().unwrap();
    assert_eq!(workspace.route, Route::Workspace);
    assert_eq!(workspace.center_mode, CenterMode::Agent);
    assert!(app.notification_return_context.is_none());
}

#[test]
fn closing_notifications_restores_reviewer_when_opened_from_reviewer() {
    let mut workspace = test_workspace();
    workspace.route = Route::Reviewer;
    workspace.center_mode = CenterMode::Editor;
    workspace.previous_center_mode = CenterMode::Preview;
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![workspace],
        rail_collapsed: false,
    });
    app.workspace_terminal_drawers[0] = true;
    app.open_notifications();
    {
        let workspace = app.current_workspace_mut().unwrap();
        workspace.route = Route::Workspace;
        workspace.center_mode = CenterMode::Agent;
    }
    app.workspace_terminal_drawers[0] = false;

    app.close_notifications_restoring_route();

    let workspace = app.current_workspace().unwrap();
    assert_eq!(workspace.route, Route::Reviewer);
    assert_eq!(workspace.center_mode, CenterMode::Editor);
    assert_eq!(workspace.previous_center_mode, CenterMode::Preview);
    assert!(app.workspace_terminal_drawer_is_open());
}

#[test]
fn trim_line_ending_removes_lf_and_crlf_only() {
    let mut line = b"ok\r\n".to_vec();
    trim_line_ending(&mut line);
    assert_eq!(line, b"ok");

    let mut line = b"ok".to_vec();
    trim_line_ending(&mut line);
    assert_eq!(line, b"ok");
}

#[test]
fn tree_row_content_width_grows_with_depth_and_label() {
    let shallow = tree_row_content_width_from_label_width(0, 18.0, 40.0, false);
    let deep = tree_row_content_width_from_label_width(4, 18.0, 40.0, false);
    let long_label = tree_row_content_width_from_label_width(0, 18.0, 180.0, false);

    assert!(deep > shallow);
    assert!(long_label > shallow);
}

#[test]
fn collapse_outline_to_first_level_opens_only_depth_zero_dirs() {
    let mut nodes = vec![OutlineNode::Dir {
        key: PathBuf::from("crates"),
        label: "crates".to_string(),
        expanded: false,
        children: vec![OutlineNode::Dir {
            key: PathBuf::from("crates/read_2d"),
            label: "read_2d".to_string(),
            expanded: true,
            children: vec![OutlineNode::Dir {
                key: PathBuf::from("crates/read_2d/src"),
                label: "src".to_string(),
                expanded: true,
                children: Vec::new(),
            }],
        }],
    }];

    collapse_outline_to_first_level(&mut nodes);

    let OutlineNode::Dir {
        expanded, children, ..
    } = &nodes[0]
    else {
        panic!("expected dir");
    };
    assert!(*expanded);
    let OutlineNode::Dir {
        expanded, children, ..
    } = &children[0]
    else {
        panic!("expected child dir");
    };
    assert!(!*expanded);
    let OutlineNode::Dir { expanded, .. } = &children[0] else {
        panic!("expected nested dir");
    };
    assert!(!*expanded);
}

#[test]
fn help_shortcut_event_accepts_physical_period() {
    let event = egui::Event::Key {
        key: egui::Key::Period,
        physical_key: Some(egui::Key::Period),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            command: true,
            ..Default::default()
        },
    };

    assert!(help_shortcut_event_pressed(&event, false));
}

#[test]
fn help_shortcut_event_uses_current_modifiers_when_event_modifiers_lag() {
    let event = egui::Event::Key {
        key: egui::Key::Period,
        physical_key: Some(egui::Key::Period),
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };

    assert!(help_shortcut_event_pressed(&event, true));
}

#[test]
fn reviewer_route_hides_outline_panel() {
    let mut workspace = WorkspaceViewData {
        name: "demo".to_string(),
        path: PathBuf::from("/tmp/demo"),
        agent_kind: AgentKind::Codex,
        agent_model: None,
        agent_model_provider: None,
        agent_effort: None,
        agent_fast_mode: None,
        agent_work_dir: None,
        agent_id: "agent".to_string(),
        session_id: None,
        subagents: Vec::new(),
        activity: WorkspaceActivity::Unknown,
        center_mode: CenterMode::Agent,
        previous_center_mode: CenterMode::Agent,
        route: Route::Workspace,
        reviewer_mode: ReviewerMode::Git,
        selected_file: None,
        outline: Vec::new(),
        outline_favorites: BTreeSet::new(),
        attached_outline_dirs: Vec::new(),
        recent_markdowns: Vec::new(),
        markdown_outline_collapsed: false,
        memo: String::new(),
    };

    assert!(should_show_outline_panel(Some(&workspace)));

    workspace.route = Route::Reviewer;

    assert!(!should_show_outline_panel(Some(&workspace)));
    assert!(!should_show_outline_panel(None));
}

#[test]
fn workspace_terminal_drawer_can_open_over_reviewer_route() {
    let workspace = WorkspaceViewData {
        name: "demo".to_string(),
        path: PathBuf::from("/tmp/demo"),
        agent_kind: AgentKind::Codex,
        agent_model: None,
        agent_model_provider: None,
        agent_effort: None,
        agent_fast_mode: None,
        agent_work_dir: None,
        agent_id: "agent".to_string(),
        session_id: None,
        subagents: Vec::new(),
        activity: WorkspaceActivity::Unknown,
        center_mode: CenterMode::Agent,
        previous_center_mode: CenterMode::Agent,
        route: Route::Reviewer,
        reviewer_mode: ReviewerMode::Git,
        selected_file: None,
        outline: Vec::new(),
        outline_favorites: BTreeSet::new(),
        attached_outline_dirs: Vec::new(),
        recent_markdowns: Vec::new(),
        markdown_outline_collapsed: false,
        memo: String::new(),
    };
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![workspace],
        rail_collapsed: false,
    });
    app.workspace_terminal_drawers[0] = true;

    assert!(app.workspace_terminal_drawer_is_open());
}

/// Verifies generic Helix launch uses the main agent work-dir override.
#[test]
fn workspace_helix_workdir_uses_active_main_agent_work_dir() {
    let root = app_test_root("main_agent_helix_workdir");
    let agent_dir = root.join("agent-dir");
    std::fs::create_dir_all(&agent_dir).unwrap();
    let mut workspace = test_workspace();
    workspace.path = root;
    workspace.agent_work_dir = Some(agent_dir.clone());
    let app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![workspace],
        rail_collapsed: false,
    });

    assert_eq!(app.active_agent_or_workspace_work_dir(), Some(agent_dir));
}

/// Verifies generic Helix launch follows the selected subagent work-dir.
#[test]
fn workspace_helix_workdir_uses_active_subagent_work_dir() {
    let root = app_test_root("subagent_helix_workdir");
    let main_dir = root.join("main-dir");
    let subagent_dir = root.join("subagent-dir");
    std::fs::create_dir_all(&main_dir).unwrap();
    std::fs::create_dir_all(&subagent_dir).unwrap();
    let mut workspace = test_workspace();
    workspace.path = root;
    workspace.agent_work_dir = Some(main_dir);
    workspace.subagents.push(SubagentViewData {
        id: "subagent-one".to_string(),
        name: "one".to_string(),
        agent_kind: AgentKind::Codex,
        agent_model: None,
        agent_model_provider: None,
        agent_effort: None,
        agent_fast_mode: None,
        agent_work_dir: Some(subagent_dir.clone()),
        agent_id: "subagent-one-agent".to_string(),
        session_id: None,
        activity: WorkspaceActivity::Unknown,
    });
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![workspace],
        rail_collapsed: false,
    });
    app.set_active_agent_slot(0, AgentSlotId::Subagent("subagent-one".to_string()));

    assert_eq!(app.active_agent_or_workspace_work_dir(), Some(subagent_dir));
}

/// Verifies covered center surfaces cannot keep consuming keyboard text.
#[test]
fn center_surface_input_is_blocked_by_top_keyboard_routes() {
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![test_workspace()],
        rail_collapsed: false,
    });

    assert!(app.center_surface_accepts_keyboard_input());

    app.workspace_terminal_drawers[0] = true;
    assert!(!app.center_surface_accepts_keyboard_input());

    app.workspace_terminal_drawers[0] = false;
    app.notifications.open();
    assert!(!app.center_surface_accepts_keyboard_input());

    app.notifications.close();
    app.set_active_app_dialog(Some(AppDialog::Help));
    assert!(!app.center_surface_accepts_keyboard_input());
}

/// 验证 modal dialog 不会被 terminal/Helix 抽屉盖住。
#[test]
fn modal_dialog_order_stays_above_terminal_overlays() {
    assert_eq!(modal_dialog_order(), egui::Order::Foreground);
}

#[test]
fn default_terminal_input_routes_to_workspace_terminal_when_drawer_is_open() {
    let mut workspace = test_workspace();

    assert_eq!(
        default_terminal_input_target(&workspace, false, false),
        Some(TerminalSurfaceKind::Agent)
    );
    assert_eq!(
        default_terminal_input_target(&workspace, true, false),
        Some(TerminalSurfaceKind::Workspace)
    );
    assert_eq!(
        default_terminal_input_target(&workspace, false, true),
        Some(TerminalSurfaceKind::Helix)
    );
    workspace.center_mode = CenterMode::Editor;
    assert_eq!(
        default_terminal_input_target(&workspace, false, false),
        None
    );
}

/// 验证 workspace terminal 抽屉只把 T/W 留给 app 快捷键。
#[test]
fn workspace_terminal_drawer_forwards_non_terminal_shortcuts() {
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![test_workspace()],
        rail_collapsed: false,
    });
    app.workspace_terminal_drawers[0] = true;
    let ctx = egui::Context::default();
    ctx.input_mut(|input| {
        *input = key_input(
            egui::Key::X,
            egui::Modifiers {
                alt: true,
                ..egui::Modifiers::NONE
            },
        );
    });

    let mut request = app
        .input_runtime_request(&ctx, ctx.input(Clone::clone))
        .unwrap();
    request.terminal_surface_owns_input = false;
    let events = process_input_runtime_request(request);

    assert!(!events.iter().any(|event| {
        matches!(
            event,
            AppEvent::InputUiCommand(UiCommand::ToggleReviewerHelix)
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            AppEvent::InputTerminalBytes {
                target: TerminalSurfaceKind::Workspace,
                bytes,
                ..
            } if bytes == b"\x1bx"
        )
    }));
}

/// 验证 input runtime 在 kitty 协议下不会把 Esc 降级成裸字节。
#[test]
fn input_runtime_forwards_escape_as_csi_u_when_terminal_uses_kitty() {
    let app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![test_workspace()],
        rail_collapsed: false,
    });
    let ctx = egui::Context::default();
    ctx.input_mut(|input| {
        *input = key_input(egui::Key::Escape, egui::Modifiers::NONE);
    });

    let mut request = app
        .input_runtime_request(&ctx, ctx.input(Clone::clone))
        .unwrap();
    request.terminal_kitty_keyboard_protocol = true;
    request.terminal_surface_owns_input = false;
    let events = process_input_runtime_request(request);

    assert!(events.iter().any(|event| {
        matches!(
            event,
            AppEvent::InputTerminalBytes {
                target: TerminalSurfaceKind::Agent,
                bytes,
                ..
            } if bytes == b"\x1b[27u"
        )
    }));
}

/// 验证可见 terminal 接管输入时，input runtime 不再重复写 PTY。
#[test]
fn input_runtime_skips_terminal_bytes_when_surface_owns_input() {
    let app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![test_workspace()],
        rail_collapsed: false,
    });
    let ctx = egui::Context::default();
    ctx.input_mut(|input| {
        *input = key_input(egui::Key::Escape, egui::Modifiers::NONE);
    });

    let mut request = app
        .input_runtime_request(&ctx, ctx.input(Clone::clone))
        .unwrap();
    request.terminal_kitty_keyboard_protocol = true;
    request.terminal_surface_owns_input = true;
    let events = process_input_runtime_request(request);

    assert!(!events.iter().any(|event| {
        matches!(
            event,
            AppEvent::InputTerminalBytes {
                target: TerminalSurfaceKind::Agent,
                ..
            }
        )
    }));
}

#[test]
fn network_settings_change_tip_tracks_dialog_open_baseline() {
    let baseline = NetworkSettings {
        proxy: "http://proxy-a.local:8080".to_string(),
        no_proxy: "example.com".to_string(),
        mirror_proxy_protocol: false,
    };
    let changed = NetworkSettings {
        proxy: "http://proxy-b.local:8080".to_string(),
        no_proxy: "example.com".to_string(),
        mirror_proxy_protocol: false,
    };

    assert!(!network_settings_changed_since(None, &changed));
    assert!(!network_settings_changed_since(Some(&baseline), &baseline));
    assert!(network_settings_changed_since(Some(&baseline), &changed));
    assert!(!network_settings_changed_since(Some(&baseline), &baseline));
}

/// 验证 Codex Auth token 请求会接受 Settings 中的 HTTP 代理。
#[test]
fn codex_auth_client_accepts_http_network_proxy() {
    let settings = NetworkSettings {
        proxy: "http://127.0.0.1:8080".to_string(),
        no_proxy: "localhost".to_string(),
        mirror_proxy_protocol: false,
    };

    assert!(codex_auth_client(&settings).is_ok());
}

/// 验证 Codex Auth token 请求会接受 Settings 中的 SOCKS 代理。
#[test]
fn codex_auth_client_accepts_socks_network_proxy() {
    let settings = NetworkSettings {
        proxy: "socks5://127.0.0.1:1080".to_string(),
        no_proxy: "localhost".to_string(),
        mirror_proxy_protocol: false,
    };

    assert!(codex_auth_client(&settings).is_ok());
}

#[test]
fn reviewer_helix_drawer_is_workspace_scoped_outside_reviewer_route() {
    let workspace = WorkspaceViewData {
        name: "demo".to_string(),
        path: PathBuf::from("/tmp/demo"),
        agent_kind: AgentKind::Codex,
        agent_model: None,
        agent_model_provider: None,
        agent_effort: None,
        agent_fast_mode: None,
        agent_work_dir: None,
        agent_id: "agent".to_string(),
        session_id: None,
        subagents: Vec::new(),
        activity: WorkspaceActivity::Unknown,
        center_mode: CenterMode::Agent,
        previous_center_mode: CenterMode::Agent,
        route: Route::Workspace,
        reviewer_mode: ReviewerMode::Git,
        selected_file: None,
        outline: Vec::new(),
        outline_favorites: BTreeSet::new(),
        attached_outline_dirs: Vec::new(),
        recent_markdowns: Vec::new(),
        markdown_outline_collapsed: false,
        memo: String::new(),
    };
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![workspace],
        rail_collapsed: false,
    });
    app.reviewer_helix_drawers[0] = true;

    assert!(app.reviewer_helix_drawer_is_open());
}

#[test]
fn reviewer_helix_toggle_closes_outside_reviewer_route() {
    let workspace = WorkspaceViewData {
        name: "demo".to_string(),
        path: PathBuf::from("/tmp/demo"),
        agent_kind: AgentKind::Codex,
        agent_model: None,
        agent_model_provider: None,
        agent_effort: None,
        agent_fast_mode: None,
        agent_work_dir: None,
        agent_id: "agent".to_string(),
        session_id: None,
        subagents: Vec::new(),
        activity: WorkspaceActivity::Unknown,
        center_mode: CenterMode::Agent,
        previous_center_mode: CenterMode::Agent,
        route: Route::Workspace,
        reviewer_mode: ReviewerMode::Git,
        selected_file: None,
        outline: Vec::new(),
        outline_favorites: BTreeSet::new(),
        attached_outline_dirs: Vec::new(),
        recent_markdowns: Vec::new(),
        markdown_outline_collapsed: false,
        memo: String::new(),
    };
    let mut app = GsdvGuiApp::from(InitialGuiData {
        active_workspace: 0,
        workspaces: vec![workspace],
        rail_collapsed: false,
    });
    app.reviewer_helix_drawers[0] = true;

    app.toggle_reviewer_helix_drawer(&egui::Context::default());

    assert!(!app.reviewer_helix_drawer_is_open());
}

#[test]
fn reviewer_script_loader_sorts_shell_scripts_and_ignores_other_files() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!("gsdv-reviewer-scripts-{unique}"));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("z-last.sh"), "#!/bin/sh\n").unwrap();
    fs::write(root.join("a-first.sh"), "#!/bin/sh\n").unwrap();
    fs::write(root.join("notes.txt"), "ignore").unwrap();
    fs::create_dir(root.join("nested.sh")).unwrap();

    let scripts = load_reviewer_scripts_from_dir(&root).unwrap();

    assert_eq!(
        scripts
            .iter()
            .map(|script| script.label.as_str())
            .collect::<Vec<_>>(),
        vec!["a-first.sh", "z-last.sh"]
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn reviewer_script_loader_reads_optional_tip_for_confirm_scripts() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!("gsdv-reviewer-script-tip-{unique}"));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("clear-confirm.sh"),
        "#!/bin/sh\n#tip: danger action\n",
    )
    .unwrap();
    fs::write(root.join("plain.sh"), "#!/bin/sh\n").unwrap();

    let scripts = load_reviewer_scripts_from_dir(&root).unwrap();
    let confirm_script = scripts
        .iter()
        .find(|script| script.label == "clear-confirm.sh")
        .unwrap();
    let plain_script = scripts
        .iter()
        .find(|script| script.label == "plain.sh")
        .unwrap();

    assert!(reviewer_script_requires_confirm(confirm_script));
    assert_eq!(confirm_script.tip.as_deref(), Some("danger action"));
    assert!(!reviewer_script_requires_confirm(plain_script));
    assert_eq!(plain_script.tip, None);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn reviewer_script_reason_line_explains_why_and_what_runs() {
    let line = reviewer_script_reason_line("check.sh", "root repo", Path::new("/tmp/project"));

    assert!(line.contains("because reviewer repo action was requested for root repo"));
    assert!(line.contains("running check.sh"));
    assert!(line.contains("/tmp/project"));
}

#[test]
fn reviewer_script_process_receives_network_env() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!("gsdv-reviewer-script-env-{unique}"));
    fs::create_dir_all(&root).unwrap();
    let output = root.join("env.txt");
    let script = root.join("script.sh");
    let output_path = output.to_string_lossy().replace('"', "\\\"");
    fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s|%s|%s' \"$http_proxy\" \"$https_proxy\" \"$no_proxy\" > \"{output_path}\"\n",
            ),
        )
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let network_settings = NetworkSettings {
        proxy: "http://127.0.0.1:7890".to_string(),
        no_proxy: "example.com".to_string(),
        mirror_proxy_protocol: false,
    };

    run_reviewer_script_process(
        tx,
        None,
        repaint_gate::RepaintController::new(),
        "script.sh".to_string(),
        script,
        "repo".to_string(),
        root.clone(),
        root.clone(),
        "repo".to_string(),
        network_settings,
    );

    assert_eq!(
        fs::read_to_string(&output).unwrap(),
        "http://127.0.0.1:7890|http://127.0.0.1:7890|192.168.0.0/16,127.0.0.1/8,example.com"
    );
    assert!(rx.try_iter().any(|event| {
        matches!(event, AppEvent::Notification(line) if line.contains("[exit] script.sh"))
    }));
    let _ = fs::remove_dir_all(root);
}

/// 校验各 terminal surface 使用自己的启动中提示。
#[test]
fn terminal_pending_message_matches_surface_kind() {
    assert_eq!(
        super::app_terminal_ui::terminal_pending_message_for_kind(TerminalSurfaceKind::Agent),
        "Starting Agent terminal..."
    );
    assert_eq!(
        super::app_terminal_ui::terminal_pending_message_for_kind(TerminalSurfaceKind::Workspace),
        "Starting workspace terminal..."
    );
    assert_eq!(
        super::app_terminal_ui::terminal_pending_message_for_kind(TerminalSurfaceKind::Helix),
        "Starting Helix..."
    );
}

/// 校验 Helix 失败时不会复用 Codex/shell 的排查文案。
#[test]
fn terminal_error_hint_matches_surface_kind() {
    assert_eq!(
        super::app_terminal_ui::terminal_error_hint_for_kind(TerminalSurfaceKind::Agent),
        "Check that the configured shell or Codex command is available."
    );
    assert_eq!(
        super::app_terminal_ui::terminal_error_hint_for_kind(TerminalSurfaceKind::Workspace),
        "Check that the configured shell is available."
    );
    assert_eq!(
        super::app_terminal_ui::terminal_error_hint_for_kind(TerminalSurfaceKind::Helix),
        "Check that the Helix executable `hx` is available."
    );
}

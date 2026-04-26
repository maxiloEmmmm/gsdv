use super::*;

#[test]
fn configure_defaults_to_dark_mode() {
    egui::__run_test_ctx(|ctx| {
        configure(&ctx);
        assert_eq!(current_mode(), ThemeMode::Dark);
        assert!(ctx.style().visuals.dark_mode);
    });
}

#[test]
fn markdown_style_uses_colored_dark_tokens() {
    let style = markdown_style(ThemeMode::Dark);

    assert!(style.visuals.dark_mode);
    assert_eq!(style.wrap_mode, Some(egui::TextWrapMode::Wrap));
    assert_eq!(style.visuals.hyperlink_color, DARK.markdown_link);
    assert_eq!(style.visuals.code_bg_color, DARK.code_bg);
    assert_eq!(
        style.visuals.widgets.active.fg_stroke.color,
        DARK.markdown_heading
    );
}

#[test]
fn dark_terminal_and_notification_text_avoid_pure_white() {
    assert_ne!(terminal_text_for(ThemeMode::Dark), Color32::WHITE);
    assert_ne!(terminal_white_for(ThemeMode::Dark), Color32::WHITE);
    assert_ne!(terminal_bright_white_for(ThemeMode::Dark), Color32::WHITE);
    assert_ne!(notification_text_for(ThemeMode::Dark), Color32::WHITE);
    assert_ne!(markdown_text_for(ThemeMode::Dark), Color32::WHITE);
    assert_ne!(list_text_for(ThemeMode::Dark), Color32::WHITE);
    assert!(
        terminal_bright_white_for(ThemeMode::Dark).r() > terminal_white_for(ThemeMode::Dark).r()
    );
    assert!(text_for(ThemeMode::Dark).r() > notification_text_for(ThemeMode::Dark).r());
    assert!(text_for(ThemeMode::Dark).r() > list_text_for(ThemeMode::Dark).r());
}

#[test]
fn configure_registers_markdown_strong_font_families() {
    let ctx = egui::Context::default();
    configure(&ctx);

    let _ = ctx.run(Default::default(), |ctx| {
        let families = ctx.fonts(|fonts| fonts.families());
        assert!(families.contains(&markdown_strong_font_family()));
        assert!(families.contains(&markdown_strong_monospace_font_family()));
    });
}

#[test]
fn surface_system_font_uses_selected_primary_only() {
    let mut fonts = FontDefinitions::default();
    let mut registered_system_fonts = BTreeMap::new();
    let path = Path::new(file!());

    register_surface_font_family(
        &mut fonts,
        agent_system_font_family(),
        FontFamilySetting::System,
        "primary",
        Some(path),
        "fallback",
        None,
        &mut registered_system_fonts,
    );

    let family_fonts = fonts
        .families
        .get(&agent_system_font_family())
        .expect("agent family is registered");
    assert_eq!(family_fonts.first().map(String::as_str), Some("primary"));
    assert_eq!(family_fonts.len(), 1);
}

/// Covers reuse of large font files across custom surface families.
#[test]
fn system_font_registration_reuses_existing_path_entry() {
    let mut fonts = FontDefinitions::default();
    let mut registered_system_fonts = BTreeMap::new();
    let path = Path::new(file!());

    register_surface_font_family(
        &mut fonts,
        agent_system_font_family(),
        FontFamilySetting::System,
        "primary",
        Some(path),
        "fallback",
        None,
        &mut registered_system_fonts,
    );
    register_surface_font_family(
        &mut fonts,
        terminal_system_font_family(),
        FontFamilySetting::System,
        "same-primary",
        Some(path),
        "same-fallback",
        None,
        &mut registered_system_fonts,
    );

    assert!(fonts.font_data.contains_key("primary"));
    assert!(!fonts.font_data.contains_key("same-primary"));
    let terminal_fonts = fonts
        .families
        .get(&terminal_system_font_family())
        .expect("terminal family is registered");
    assert_eq!(terminal_fonts.first().map(String::as_str), Some("primary"));
}

#[test]
fn surface_system_font_inserts_selected_fallback_after_primary() {
    let mut fonts = FontDefinitions::default();
    let mut registered_system_fonts = BTreeMap::new();
    let primary = Path::new(file!());
    let fallback = Path::new("Cargo.toml");

    register_surface_font_family(
        &mut fonts,
        agent_system_font_family(),
        FontFamilySetting::System,
        "primary",
        Some(primary),
        "fallback",
        Some(fallback),
        &mut registered_system_fonts,
    );

    let family_fonts = fonts
        .families
        .get(&agent_system_font_family())
        .expect("agent family is registered");
    assert_eq!(family_fonts.first().map(String::as_str), Some("primary"));
    assert_eq!(family_fonts.get(1).map(String::as_str), Some("fallback"));
}

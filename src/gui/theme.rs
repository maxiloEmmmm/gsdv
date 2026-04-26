use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke,
    Style, TextStyle, Vec2, Visuals,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::gui::data::FontFamilySetting;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    Light,
    Dark,
}

#[derive(Clone, Copy)]
struct Palette {
    bg: Color32,
    surface: Color32,
    surface_elevated: Color32,
    border: Color32,
    text: Color32,
    muted: Color32,
    primary: Color32,
    primary_soft: Color32,
    on_primary: Color32,
    success: Color32,
    success_soft: Color32,
    success_border: Color32,
    warning: Color32,
    warning_soft: Color32,
    warning_border: Color32,
    danger: Color32,
    danger_soft: Color32,
    danger_border: Color32,
    hover: Color32,
    primary_border: Color32,
    terminal_text: Color32,
    terminal_white: Color32,
    terminal_bright_white: Color32,
    notification_text: Color32,
    list_text: Color32,
    diff_insert_bg: Color32,
    diff_delete_bg: Color32,
    diff_hunk_bg: Color32,
    diff_metadata_bg: Color32,
    code_bg: Color32,
    markdown_text: Color32,
    markdown_heading: Color32,
    markdown_link: Color32,
}

const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    surface: Color32::from_rgb(0xF6, 0xF7, 0xF9),
    surface_elevated: Color32::from_rgb(0xFB, 0xFC, 0xFD),
    border: Color32::from_rgb(0xE5, 0xE7, 0xEB),
    text: Color32::from_rgb(0x1F, 0x23, 0x28),
    muted: Color32::from_rgb(0x6B, 0x72, 0x80),
    primary: Color32::from_rgb(0x25, 0x63, 0xEB),
    primary_soft: Color32::from_rgb(0xDB, 0xE8, 0xFF),
    on_primary: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    success: Color32::from_rgb(0x16, 0xA3, 0x4A),
    success_soft: Color32::from_rgb(0xE8, 0xF8, 0xEE),
    success_border: Color32::from_rgb(0x9F, 0xDD, 0xB8),
    warning: Color32::from_rgb(0xD9, 0x77, 0x06),
    warning_soft: Color32::from_rgb(0xFF, 0xF4, 0xD6),
    warning_border: Color32::from_rgb(0xF6, 0xD3, 0x8E),
    danger: Color32::from_rgb(0xDC, 0x26, 0x26),
    danger_soft: Color32::from_rgb(0xFE, 0xF0, 0xF2),
    danger_border: Color32::from_rgb(0xF0, 0xB0, 0xB8),
    hover: Color32::from_rgb(0xF2, 0xF5, 0xFB),
    primary_border: Color32::from_rgb(0xBF, 0xD4, 0xFF),
    terminal_text: Color32::from_rgb(0x1F, 0x23, 0x28),
    terminal_white: Color32::from_rgb(0x1F, 0x23, 0x28),
    terminal_bright_white: Color32::from_rgb(0x11, 0x18, 0x27),
    notification_text: Color32::from_rgb(0x1F, 0x23, 0x28),
    list_text: Color32::from_rgb(0x1F, 0x23, 0x28),
    diff_insert_bg: Color32::from_rgb(0xEC, 0xF8, 0xF0),
    diff_delete_bg: Color32::from_rgb(0xFE, 0xF0, 0xF2),
    diff_hunk_bg: Color32::from_rgb(0xF3, 0xF6, 0xFC),
    diff_metadata_bg: Color32::from_rgb(0xF8, 0xFA, 0xFC),
    code_bg: Color32::from_rgb(0xF1, 0xF5, 0xF9),
    markdown_text: Color32::from_rgb(0x20, 0x2A, 0x37),
    markdown_heading: Color32::from_rgb(0x1D, 0x4E, 0x89),
    markdown_link: Color32::from_rgb(0x25, 0x63, 0xEB),
};

const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x0B, 0x10, 0x16),
    surface: Color32::from_rgb(0x11, 0x18, 0x21),
    surface_elevated: Color32::from_rgb(0x16, 0x20, 0x2B),
    border: Color32::from_rgb(0x29, 0x36, 0x46),
    text: Color32::from_rgb(0xE6, 0xED, 0xF3),
    muted: Color32::from_rgb(0x91, 0xA4, 0xB7),
    primary: Color32::from_rgb(0x60, 0xA5, 0xFA),
    primary_soft: Color32::from_rgb(0x17, 0x2C, 0x4A),
    on_primary: Color32::from_rgb(0x08, 0x12, 0x1E),
    success: Color32::from_rgb(0x34, 0xD3, 0x99),
    success_soft: Color32::from_rgb(0x0E, 0x2B, 0x24),
    success_border: Color32::from_rgb(0x1D, 0x73, 0x59),
    warning: Color32::from_rgb(0xF5, 0xB8, 0x4B),
    warning_soft: Color32::from_rgb(0x32, 0x25, 0x12),
    warning_border: Color32::from_rgb(0x8E, 0x64, 0x1F),
    danger: Color32::from_rgb(0xFB, 0x71, 0x85),
    danger_soft: Color32::from_rgb(0x35, 0x16, 0x21),
    danger_border: Color32::from_rgb(0x9A, 0x3B, 0x4B),
    hover: Color32::from_rgb(0x1D, 0x2A, 0x38),
    primary_border: Color32::from_rgb(0x2C, 0x5F, 0x9F),
    terminal_text: Color32::from_rgb(0xB8, 0xC2, 0xCC),
    terminal_white: Color32::from_rgb(0xAE, 0xB8, 0xC4),
    terminal_bright_white: Color32::from_rgb(0xCB, 0xD5, 0xE1),
    notification_text: Color32::from_rgb(0xB8, 0xC2, 0xCC),
    list_text: Color32::from_rgb(0xB8, 0xC2, 0xCC),
    diff_insert_bg: Color32::from_rgb(0x0D, 0x2A, 0x20),
    diff_delete_bg: Color32::from_rgb(0x35, 0x16, 0x21),
    diff_hunk_bg: Color32::from_rgb(0x13, 0x24, 0x3A),
    diff_metadata_bg: Color32::from_rgb(0x12, 0x1B, 0x25),
    code_bg: Color32::from_rgb(0x13, 0x1D, 0x29),
    markdown_text: Color32::from_rgb(0xB8, 0xC2, 0xCC),
    markdown_heading: Color32::from_rgb(0x93, 0xC5, 0xFD),
    markdown_link: Color32::from_rgb(0x67, 0xE8, 0xF9),
};

static DARK_MODE: AtomicBool = AtomicBool::new(true);

const MARKDOWN_STRONG_FAMILY: &str = "gsdv_markdown_strong";
const MARKDOWN_STRONG_MONO_FAMILY: &str = "gsdv_markdown_strong_mono";
const AGENT_SYSTEM_FAMILY: &str = "gsdv_agent_system";
const TERMINAL_SYSTEM_FAMILY: &str = "gsdv_terminal_system";
const EDITOR_SYSTEM_FAMILY: &str = "gsdv_editor_system";

pub const RADIUS_SM: u8 = 6;
pub const RADIUS_MD: u8 = 8;
pub const RADIUS_LG: u8 = 8;

pub fn configure(ctx: &egui::Context) {
    configure_fonts(
        ctx,
        None,
        FontFamilySetting::Monospace,
        None,
        None,
        FontFamilySetting::Monospace,
        None,
        None,
        FontFamilySetting::Monospace,
        None,
        None,
    );
    set_mode(ctx, ThemeMode::Dark);
}

/// Applies runtime-selected primary and fallback fonts to egui.
pub fn configure_runtime_fonts(
    ctx: &egui::Context,
    ui_fallback_font_path: Option<&Path>,
    agent_family: FontFamilySetting,
    agent_font_path: Option<&Path>,
    agent_fallback_font_path: Option<&Path>,
    terminal_family: FontFamilySetting,
    terminal_font_path: Option<&Path>,
    terminal_fallback_font_path: Option<&Path>,
    editor_family: FontFamilySetting,
    editor_font_path: Option<&Path>,
    editor_fallback_font_path: Option<&Path>,
) {
    ctx.set_fonts(runtime_font_definitions(
        ui_fallback_font_path,
        agent_family,
        agent_font_path,
        agent_fallback_font_path,
        terminal_family,
        terminal_font_path,
        terminal_fallback_font_path,
        editor_family,
        editor_font_path,
        editor_fallback_font_path,
    ));
}

/// 构建运行时字体定义，适用于后台提前读取字体文件。
pub fn runtime_font_definitions(
    ui_fallback_font_path: Option<&Path>,
    agent_family: FontFamilySetting,
    agent_font_path: Option<&Path>,
    agent_fallback_font_path: Option<&Path>,
    terminal_family: FontFamilySetting,
    terminal_font_path: Option<&Path>,
    terminal_fallback_font_path: Option<&Path>,
    editor_family: FontFamilySetting,
    editor_font_path: Option<&Path>,
    editor_fallback_font_path: Option<&Path>,
) -> FontDefinitions {
    build_font_definitions(
        ui_fallback_font_path,
        agent_family,
        agent_font_path,
        agent_fallback_font_path,
        terminal_family,
        terminal_font_path,
        terminal_fallback_font_path,
        editor_family,
        editor_font_path,
        editor_fallback_font_path,
    )
}

/// 应用后台构建好的字体定义。
pub fn apply_runtime_font_definitions(ctx: &egui::Context, fonts: FontDefinitions) {
    ctx.set_fonts(fonts);
}

/// Registers app font families with selected primary and fallback fonts.
fn configure_fonts(
    ctx: &egui::Context,
    ui_fallback_font_path: Option<&Path>,
    agent_family: FontFamilySetting,
    agent_font_path: Option<&Path>,
    agent_fallback_font_path: Option<&Path>,
    terminal_family: FontFamilySetting,
    terminal_font_path: Option<&Path>,
    terminal_fallback_font_path: Option<&Path>,
    editor_family: FontFamilySetting,
    editor_font_path: Option<&Path>,
    editor_fallback_font_path: Option<&Path>,
) {
    ctx.set_fonts(build_font_definitions(
        ui_fallback_font_path,
        agent_family,
        agent_font_path,
        agent_fallback_font_path,
        terminal_family,
        terminal_font_path,
        terminal_fallback_font_path,
        editor_family,
        editor_font_path,
        editor_fallback_font_path,
    ));
}

/// 组装 egui 字体表；调用者决定在哪个线程读取字体文件。
fn build_font_definitions(
    ui_fallback_font_path: Option<&Path>,
    agent_family: FontFamilySetting,
    agent_font_path: Option<&Path>,
    agent_fallback_font_path: Option<&Path>,
    terminal_family: FontFamilySetting,
    terminal_font_path: Option<&Path>,
    terminal_fallback_font_path: Option<&Path>,
    editor_family: FontFamilySetting,
    editor_font_path: Option<&Path>,
    editor_fallback_font_path: Option<&Path>,
) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let mut registered_system_fonts = BTreeMap::new();

    register_builtin_ui_fallback(
        &mut fonts,
        ui_fallback_font_path,
        &mut registered_system_fonts,
    );
    register_surface_font_family(
        &mut fonts,
        agent_system_font_family(),
        agent_family,
        "gsdv_agent_system_font",
        agent_font_path,
        "gsdv_agent_fallback_system_font",
        agent_fallback_font_path,
        &mut registered_system_fonts,
    );
    register_surface_font_family(
        &mut fonts,
        terminal_system_font_family(),
        terminal_family,
        "gsdv_terminal_system_font",
        terminal_font_path,
        "gsdv_terminal_fallback_system_font",
        terminal_fallback_font_path,
        &mut registered_system_fonts,
    );
    register_surface_font_family(
        &mut fonts,
        editor_system_font_family(),
        editor_family,
        "gsdv_editor_system_font",
        editor_font_path,
        "gsdv_editor_fallback_system_font",
        editor_fallback_font_path,
        &mut registered_system_fonts,
    );
    register_markdown_strong_family(
        &mut fonts,
        editor_system_font_family(),
        markdown_strong_font_family(),
        None,
    );
    register_markdown_strong_family(
        &mut fonts,
        terminal_system_font_family(),
        markdown_strong_monospace_font_family(),
        None,
    );

    fonts
}

/// 给 egui 内建 UI 字体族补 CJK fallback，适用于设置页和弹窗。
fn register_builtin_ui_fallback(
    fonts: &mut FontDefinitions,
    fallback_path: Option<&Path>,
    registered_system_fonts: &mut BTreeMap<(PathBuf, u32), String>,
) {
    let Some(fallback_name) = register_optional_system_font(
        fonts,
        registered_system_fonts,
        "gsdv_ui_fallback_system_font",
        fallback_path,
        None,
    ) else {
        return;
    };
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let mut family_fonts = base_family_fonts(fonts, family.clone());
        insert_after_primary(&mut family_fonts, fallback_name.clone());
        fonts.families.insert(family, family_fonts);
    }
}

/// Creates a surface family from the selected primary and fallback fonts.
fn register_surface_font_family(
    fonts: &mut FontDefinitions,
    family: FontFamily,
    selected_family: FontFamilySetting,
    font_name: &str,
    path: Option<&Path>,
    fallback_font_name: &str,
    fallback_path: Option<&Path>,
    registered_system_fonts: &mut BTreeMap<(PathBuf, u32), String>,
) {
    let mut family_fonts = match selected_family {
        FontFamilySetting::Default => base_family_fonts(fonts, FontFamily::Monospace),
        FontFamilySetting::Monospace => base_family_fonts(fonts, FontFamily::Monospace),
        FontFamilySetting::Proportional => base_family_fonts(fonts, FontFamily::Proportional),
        FontFamilySetting::System => {
            register_primary_system_font(fonts, registered_system_fonts, font_name, path)
                .map(|font_name| vec![font_name])
                .unwrap_or_else(|| base_family_fonts(fonts, FontFamily::Monospace))
        }
    };

    let primary_path = if selected_family == FontFamilySetting::System {
        path
    } else {
        None
    };
    if let Some(fallback_name) = register_optional_system_font(
        fonts,
        registered_system_fonts,
        fallback_font_name,
        fallback_path,
        primary_path,
    ) {
        insert_after_primary(&mut family_fonts, fallback_name);
    }
    fonts.families.insert(family, family_fonts);
}

/// Returns the current built-in family chain for a surface custom family.
fn base_family_fonts(fonts: &FontDefinitions, family: FontFamily) -> Vec<String> {
    fonts.families.get(&family).cloned().unwrap_or_default()
}

/// Loads the primary system font selected for a surface.
fn register_primary_system_font(
    fonts: &mut FontDefinitions,
    registered_system_fonts: &mut BTreeMap<(PathBuf, u32), String>,
    font_name: &str,
    path: Option<&Path>,
) -> Option<String> {
    register_system_font(fonts, registered_system_fonts, font_name, path?)
}

/// Loads an optional fallback font when it differs from the primary font.
fn register_optional_system_font(
    fonts: &mut FontDefinitions,
    registered_system_fonts: &mut BTreeMap<(PathBuf, u32), String>,
    font_name: &str,
    path: Option<&Path>,
    primary_path: Option<&Path>,
) -> Option<String> {
    let path = path.filter(|path| Some(*path) != primary_path)?;
    register_system_font(fonts, registered_system_fonts, font_name, path)
}

/// Registers a system font once per path and face index.
fn register_system_font(
    fonts: &mut FontDefinitions,
    registered_system_fonts: &mut BTreeMap<(PathBuf, u32), String>,
    font_name: &str,
    path: &Path,
) -> Option<String> {
    let face_index = preferred_font_face_index(path);
    let key = (path.to_path_buf(), face_index);
    if let Some(existing_name) = registered_system_fonts.get(&key) {
        return Some(existing_name.clone());
    }
    let bytes = fs::read(path).ok()?;
    let mut font_data = FontData::from_owned(bytes);
    font_data.index = face_index;
    let font_name = font_name.to_string();
    fonts
        .font_data
        .insert(font_name.clone(), Arc::new(font_data));
    registered_system_fonts.insert(key, font_name.clone());
    Some(font_name)
}

/// Chooses a stable face inside known TTC font collections.
fn preferred_font_face_index(path: &Path) -> u32 {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return 0;
    };
    // Special logic:
    // Trigger: font settings store only the file path for a TTC collection.
    // Why: egui FontData needs a face index, and index 0 in Noto CJK is JP.
    // Prevents: Chinese fallback selecting the JP face and rendering wrong glyphs.
    if file_name.starts_with("NotoSansCJK") {
        2
    } else {
        0
    }
}

/// Keeps the fallback immediately after the primary font candidate.
fn insert_after_primary(family_fonts: &mut Vec<String>, fallback_name: String) {
    family_fonts.retain(|name| name != &fallback_name);
    if family_fonts.is_empty() {
        family_fonts.push(fallback_name);
    } else {
        family_fonts.insert(1, fallback_name);
    }
}

fn register_markdown_strong_family(
    fonts: &mut FontDefinitions,
    fallback_family: FontFamily,
    strong_family: FontFamily,
    strong_font: Option<(String, Vec<u8>)>,
) {
    let mut family_fonts = fonts
        .families
        .get(&fallback_family)
        .cloned()
        .unwrap_or_default();

    if let Some((name, bytes)) = strong_font {
        fonts
            .font_data
            .insert(name.clone(), Arc::new(FontData::from_owned(bytes)));
        family_fonts.insert(0, name);
    }

    fonts.families.insert(strong_family, family_fonts);
}

pub fn markdown_strong_font_family() -> FontFamily {
    FontFamily::Name(MARKDOWN_STRONG_FAMILY.into())
}

pub fn markdown_strong_monospace_font_family() -> FontFamily {
    FontFamily::Name(MARKDOWN_STRONG_MONO_FAMILY.into())
}

pub fn markdown_monospace_font_family() -> FontFamily {
    terminal_system_font_family()
}

pub fn agent_system_font_family() -> FontFamily {
    FontFamily::Name(AGENT_SYSTEM_FAMILY.into())
}

pub fn terminal_system_font_family() -> FontFamily {
    FontFamily::Name(TERMINAL_SYSTEM_FAMILY.into())
}

pub fn editor_system_font_family() -> FontFamily {
    FontFamily::Name(EDITOR_SYSTEM_FAMILY.into())
}

pub fn set_mode(ctx: &egui::Context, mode: ThemeMode) {
    DARK_MODE.store(mode == ThemeMode::Dark, Ordering::Relaxed);
    ctx.set_style(style(mode));
}

pub fn current_mode() -> ThemeMode {
    if DARK_MODE.load(Ordering::Relaxed) {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    }
}

pub fn is_dark() -> bool {
    current_mode() == ThemeMode::Dark
}

pub fn toggle_mode(ctx: &egui::Context) -> ThemeMode {
    let mode = if is_dark() {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    };
    set_mode(ctx, mode);
    mode
}

pub fn bg() -> Color32 {
    palette().bg
}
pub fn surface() -> Color32 {
    palette().surface
}
pub fn surface_elevated() -> Color32 {
    palette().surface_elevated
}
pub fn border() -> Color32 {
    palette().border
}
pub fn text() -> Color32 {
    palette().text
}
pub fn muted() -> Color32 {
    palette().muted
}
pub fn primary() -> Color32 {
    palette().primary
}
pub fn primary_soft() -> Color32 {
    palette().primary_soft
}
pub fn success() -> Color32 {
    palette().success
}
pub fn warning() -> Color32 {
    palette().warning
}
pub fn danger() -> Color32 {
    palette().danger
}
pub fn hover() -> Color32 {
    palette().hover
}
pub fn primary_border() -> Color32 {
    palette().primary_border
}
pub fn on_primary() -> Color32 {
    palette().on_primary
}
pub fn diff_insert_bg() -> Color32 {
    palette().diff_insert_bg
}
pub fn diff_delete_bg() -> Color32 {
    palette().diff_delete_bg
}
pub fn diff_hunk_bg() -> Color32 {
    palette().diff_hunk_bg
}
pub fn diff_metadata_bg() -> Color32 {
    palette().diff_metadata_bg
}
pub fn danger_soft() -> Color32 {
    palette().danger_soft
}
pub fn danger_border() -> Color32 {
    palette().danger_border
}
pub fn accent_soft(color: Color32) -> Color32 {
    let p = palette();
    if color == p.primary {
        p.primary_soft
    } else if color == p.success {
        p.success_soft
    } else if color == p.warning {
        p.warning_soft
    } else if color == p.danger {
        p.danger_soft
    } else {
        p.surface
    }
}
pub fn accent_border(color: Color32) -> Color32 {
    let p = palette();
    if color == p.primary {
        p.primary_border
    } else if color == p.success {
        p.success_border
    } else if color == p.warning {
        p.warning_border
    } else if color == p.danger {
        p.danger_border
    } else {
        p.border
    }
}
pub fn transparent() -> Color32 {
    Color32::TRANSPARENT
}
pub fn terminal_bright_white() -> Color32 {
    palette().terminal_bright_white
}
pub fn notification_text() -> Color32 {
    palette().notification_text
}
pub fn markdown_text() -> Color32 {
    palette().markdown_text
}
pub fn list_text() -> Color32 {
    palette().list_text
}

pub fn bg_for(mode: ThemeMode) -> Color32 {
    palette_for(mode).bg
}
pub fn text_for(mode: ThemeMode) -> Color32 {
    palette_for(mode).text
}
pub fn muted_for(mode: ThemeMode) -> Color32 {
    palette_for(mode).muted
}
pub fn primary_for(mode: ThemeMode) -> Color32 {
    palette_for(mode).primary
}
pub fn success_for(mode: ThemeMode) -> Color32 {
    palette_for(mode).success
}
pub fn warning_for(mode: ThemeMode) -> Color32 {
    palette_for(mode).warning
}
pub fn danger_for(mode: ThemeMode) -> Color32 {
    palette_for(mode).danger
}
pub fn terminal_bright_white_for(mode: ThemeMode) -> Color32 {
    palette_for(mode).terminal_bright_white
}
pub fn terminal_text_for(mode: ThemeMode) -> Color32 {
    palette_for(mode).terminal_text
}
pub fn terminal_white_for(mode: ThemeMode) -> Color32 {
    palette_for(mode).terminal_white
}
pub fn notification_text_for(mode: ThemeMode) -> Color32 {
    palette_for(mode).notification_text
}
pub fn markdown_text_for(mode: ThemeMode) -> Color32 {
    palette_for(mode).markdown_text
}
pub fn list_text_for(mode: ThemeMode) -> Color32 {
    palette_for(mode).list_text
}

pub fn markdown_style(mode: ThemeMode) -> Style {
    let mut style = style(mode);
    let p = palette_for(mode);

    style.text_styles = markdown_text_styles();
    style.wrap_mode = Some(egui::TextWrapMode::Wrap);
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(6.0, 4.0);
    style.visuals.override_text_color = None;
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.markdown_text);
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, p.markdown_text);
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, p.markdown_heading);
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.0, p.markdown_heading);
    style.visuals.hyperlink_color = p.markdown_link;
    style.visuals.faint_bg_color = p.surface;
    style.visuals.extreme_bg_color = p.code_bg;
    style.visuals.code_bg_color = p.code_bg;
    style.visuals.selection.bg_fill = p.primary_soft;
    style.visuals.selection.stroke = Stroke::new(1.0, p.primary);
    style
}

/// 返回 Markdown 预览专用字号，并挂到 editor/terminal 字体链。
fn markdown_text_styles() -> BTreeMap<TextStyle, FontId> {
    [
        (
            TextStyle::Heading,
            FontId::new(20.0, editor_system_font_family()),
        ),
        (
            TextStyle::Name("Title".into()),
            FontId::new(17.0, editor_system_font_family()),
        ),
        (
            TextStyle::Body,
            FontId::new(13.0, editor_system_font_family()),
        ),
        (
            TextStyle::Button,
            FontId::new(13.0, editor_system_font_family()),
        ),
        (
            TextStyle::Small,
            FontId::new(11.0, editor_system_font_family()),
        ),
        (
            TextStyle::Monospace,
            FontId::new(12.0, markdown_monospace_font_family()),
        ),
    ]
    .into()
}

fn style(mode: ThemeMode) -> Style {
    let mut style = Style::default();
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(10.0, 6.0);
    style.spacing.indent = 16.0;
    style.spacing.interact_size = Vec2::new(0.0, 32.0);
    style.spacing.window_margin = Margin::same(12);

    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(20.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Name("Title".into()),
            FontId::new(17.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(13.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(12.0, FontFamily::Monospace),
        ),
    ]
    .into();

    style.visuals = visuals(mode);
    style
}

fn visuals(mode: ThemeMode) -> Visuals {
    let p = palette_for(mode);
    let mut visuals = match mode {
        ThemeMode::Light => Visuals::light(),
        ThemeMode::Dark => Visuals::dark(),
    };
    visuals.window_fill = p.bg;
    visuals.panel_fill = p.bg;
    visuals.faint_bg_color = p.surface;
    visuals.extreme_bg_color = p.surface;
    visuals.code_bg_color = p.code_bg;
    visuals.override_text_color = Some(p.text);
    visuals.widgets.noninteractive.bg_fill = p.bg;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(RADIUS_MD);
    visuals.widgets.inactive.bg_fill = p.surface_elevated;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(RADIUS_SM);
    visuals.widgets.hovered.bg_fill = p.hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, p.border);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(RADIUS_SM);
    visuals.widgets.active.bg_fill = p.primary_soft;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, p.primary);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, p.primary);
    visuals.widgets.active.corner_radius = CornerRadius::same(RADIUS_SM);
    visuals.selection.bg_fill = p.primary_soft;
    visuals.selection.stroke = Stroke::new(1.0, p.primary);
    visuals.hyperlink_color = p.markdown_link;
    visuals.warn_fg_color = p.warning;
    visuals.error_fg_color = p.danger;
    visuals.window_corner_radius = CornerRadius::same(RADIUS_LG);
    visuals.window_stroke = Stroke::new(1.0, p.border);
    visuals.menu_corner_radius = CornerRadius::same(RADIUS_MD);
    visuals
}

fn palette() -> Palette {
    palette_for(current_mode())
}

fn palette_for(mode: ThemeMode) -> Palette {
    match mode {
        ThemeMode::Light => LIGHT,
        ThemeMode::Dark => DARK,
    }
}

#[cfg(test)]
#[path = "theme_test.rs"]
mod theme_test;

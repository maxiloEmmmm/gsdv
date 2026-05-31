//! 字体发现、字体设置解析和运行时字体应用。
//!
//! 该模块只处理字体相关业务：扫描系统字体、规范化 store 字体配置，
//! 以及把最终字体链注入 egui/theme。

use super::*;

pub(super) fn editor_font_id(settings: &data::FontSurfaceSettings) -> egui::FontId {
    surface_font_id(settings, FontSurfaceTarget::Editor)
}

/// 返回继承默认配置后的 Markdown editor 字体。
pub(super) fn effective_editor_font_id(settings: &FontSettings) -> egui::FontId {
    editor_font_id(&effective_font_surface_settings(settings, &settings.editor))
}

pub(super) fn notification_font_id(settings: &data::FontSurfaceSettings) -> egui::FontId {
    surface_font_id(settings, FontSurfaceTarget::Terminal)
}

/// Returns the effective terminal font size without cloning full font settings.
pub(super) fn terminal_font_size_for_kind(
    settings: &FontSettings,
    kind: TerminalSurfaceKind,
) -> f32 {
    match kind {
        TerminalSurfaceKind::Agent => effective_surface_font_size(settings, &settings.agent),
        TerminalSurfaceKind::Workspace | TerminalSurfaceKind::Helix => {
            effective_surface_font_size(settings, &settings.terminal)
        }
    }
}

#[derive(Clone, Copy)]
enum FontSurfaceTarget {
    Terminal,
    Editor,
}

fn surface_font_id(
    settings: &data::FontSurfaceSettings,
    target: FontSurfaceTarget,
) -> egui::FontId {
    egui::FontId::new(
        settings.size.clamp(9.0, 28.0),
        match target {
            FontSurfaceTarget::Terminal => theme::terminal_system_font_family(),
            FontSurfaceTarget::Editor => theme::editor_system_font_family(),
        },
    )
}

/// 让内容差异面板跟随 Markdown editor 字体链。
pub(super) fn apply_editor_content_font_style(ui: &mut Ui, settings: &FontSettings) {
    let font = effective_editor_font_id(settings);
    let small_size = (font.size - 2.0).clamp(9.0, 28.0);
    let mut style = ui.style().as_ref().clone();
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::new(font.size, font.family.clone()),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::new(font.size, font.family.clone()),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        egui::FontId::new(small_size, font.family.clone()),
    );
    style.text_styles.insert(egui::TextStyle::Monospace, font);
    ui.set_style(style);
}

/// Resolves only the inherited font size for a surface.
pub(super) fn effective_surface_font_size(
    settings: &FontSettings,
    surface: &data::FontSurfaceSettings,
) -> f32 {
    if surface.family == data::FontFamilySetting::Default {
        settings.default_fonts.size
    } else {
        surface.size
    }
}

pub(super) fn apply_runtime_fonts(ctx: &egui::Context, settings: &FontSettings) {
    let fonts = build_runtime_font_definitions(settings);
    theme::apply_runtime_font_definitions(ctx, fonts);
}

/// 构建运行时字体定义，适用于后台任务先完成字体文件读取。
pub(super) fn build_runtime_font_definitions(settings: &FontSettings) -> egui::FontDefinitions {
    let ui_fallback_font_path = fallback_system_font_path(&settings.default_fonts);
    let agent = effective_font_surface_settings(settings, &settings.agent);
    let terminal = effective_font_surface_settings(settings, &settings.terminal);
    let editor = effective_font_surface_settings(settings, &settings.editor);
    let agent_font_path = system_font_path(&agent);
    let agent_fallback_font_path = fallback_system_font_path(&agent);
    let terminal_font_path = system_font_path(&terminal);
    let terminal_fallback_font_path = fallback_system_font_path(&terminal);
    let editor_font_path = system_font_path(&editor);
    let editor_fallback_font_path = fallback_system_font_path(&editor);
    theme::runtime_font_definitions(
        ui_fallback_font_path.as_deref(),
        agent.family,
        agent_font_path.as_deref(),
        agent_fallback_font_path.as_deref(),
        terminal.family,
        terminal_font_path.as_deref(),
        terminal_fallback_font_path.as_deref(),
        editor.family,
        editor_font_path.as_deref(),
        editor_fallback_font_path.as_deref(),
    )
}

/// Resolves a surface font against the global default chain.
pub(super) fn effective_font_surface_settings(
    settings: &FontSettings,
    surface: &data::FontSurfaceSettings,
) -> data::FontSurfaceSettings {
    if surface.family == data::FontFamilySetting::Default {
        settings.default_fonts.clone()
    } else {
        let mut resolved = surface.clone();
        if resolved.fallback_system_path.is_none() {
            resolved.fallback_system_name = settings.default_fonts.fallback_system_name.clone();
            resolved.fallback_system_path = settings.default_fonts.fallback_system_path.clone();
        }
        resolved
    }
}

/// Returns the selected primary system font path when it can be loaded.
pub(super) fn system_font_path(settings: &data::FontSurfaceSettings) -> Option<PathBuf> {
    if settings.family != data::FontFamilySetting::System {
        return None;
    }
    settings
        .system_path
        .as_deref()
        .map(PathBuf::from)
        .filter(|path| path.is_file())
}

/// Returns the selected fallback system font path when it can be loaded.
pub(super) fn fallback_system_font_path(settings: &data::FontSurfaceSettings) -> Option<PathBuf> {
    settings
        .fallback_system_path
        .as_deref()
        .map(PathBuf::from)
        .filter(|path| path.is_file())
}

pub(super) fn scan_system_fonts() -> Vec<SystemFontEntry> {
    let mut fonts = Vec::new();
    for root in system_font_roots() {
        collect_system_fonts(&root, &mut fonts);
    }
    fonts.sort_by_key(|font| font.name.to_lowercase());
    fonts.dedup_by(|a, b| a.path == b.path);
    fonts
}

/// Builds the first-run font defaults from the discovered system font list.
pub(super) fn initial_font_settings(system_fonts: &[SystemFontEntry]) -> FontSettings {
    let primary = preferred_primary_font(system_fonts);
    let fallback = preferred_fallback_font(system_fonts);
    FontSettings {
        default_fonts: data::FontSurfaceSettings {
            family: if primary.is_some() {
                data::FontFamilySetting::System
            } else {
                data::FontFamilySetting::Monospace
            },
            size: 14.0,
            system_name: primary.map(|font| font.name.clone()),
            system_path: primary.map(|font| font.path.to_string_lossy().to_string()),
            fallback_system_name: fallback.map(|font| font.name.clone()),
            fallback_system_path: fallback.map(|font| font.path.to_string_lossy().to_string()),
        },
        agent: default_inherited_font_surface_settings(),
        terminal: default_inherited_font_surface_settings(),
        editor: default_inherited_font_surface_settings(),
    }
}

/// 归一化已加载字体设置，只清理默认继承里的脏字段。
pub(super) fn normalize_font_settings(
    settings: &mut FontSettings,
    system_fonts: &[SystemFontEntry],
) -> bool {
    let surface_changed = [
        (&mut settings.default_fonts, false),
        (&mut settings.agent, true),
        (&mut settings.terminal, true),
        (&mut settings.editor, true),
    ]
    .into_iter()
    .fold(false, |changed, (surface, allow_default)| {
        normalize_font_surface_primary(surface, system_fonts, allow_default) || changed
    });
    ensure_default_fallback_font(&mut settings.default_fonts, system_fonts) || surface_changed
}

/// 确保已有 store 里的默认字体链也能补上 CJK fallback。
pub(super) fn ensure_default_fallback_font(
    settings: &mut data::FontSurfaceSettings,
    system_fonts: &[SystemFontEntry],
) -> bool {
    if settings
        .fallback_system_path
        .as_deref()
        .is_some_and(|path| font_path_is_available(path, system_fonts))
    {
        return false;
    }
    let Some(fallback) = preferred_fallback_font(system_fonts) else {
        return false;
    };
    settings.fallback_system_name = Some(fallback.name.clone());
    settings.fallback_system_path = Some(fallback.path.to_string_lossy().to_string());
    true
}

/// 判断字体路径当前是否可用，适用于跨平台迁移后的旧配置校正。
fn font_path_is_available(path: &str, system_fonts: &[SystemFontEntry]) -> bool {
    Path::new(path).is_file() || system_fonts.iter().any(|font| font.path == Path::new(path))
}

/// 清理旧配置里和默认继承冲突的字段。
pub(super) fn normalize_font_surface_primary(
    settings: &mut data::FontSurfaceSettings,
    _system_fonts: &[SystemFontEntry],
    allow_default: bool,
) -> bool {
    if allow_default && settings.family == data::FontFamilySetting::Default {
        let mut changed = false;
        changed |= settings.system_name.take().is_some();
        changed |= settings.system_path.take().is_some();
        changed |= settings.fallback_system_name.take().is_some();
        changed |= settings.fallback_system_path.take().is_some();
        if changed {
            return true;
        }
        return false;
    }
    if !allow_default && settings.family == data::FontFamilySetting::Default {
        settings.family = data::FontFamilySetting::Monospace;
        return true;
    }
    false
}

/// Finds a code-friendly primary font for first-run defaults.
pub(super) fn preferred_primary_font(system_fonts: &[SystemFontEntry]) -> Option<&SystemFontEntry> {
    preferred_font_by_keywords(
        system_fonts,
        &[
            "firacoderegular",
            "firacode",
            "jetbrainsmonoregular",
            "jetbrainsmono",
            "cascadiacode",
            "cascadiamono",
            "dejavusansmono",
            "notosansmono",
            "consola",
            "menlo",
        ],
    )
}

/// Finds a CJK-capable fallback font for first-run defaults.
pub(super) fn preferred_fallback_font(
    system_fonts: &[SystemFontEntry],
) -> Option<&SystemFontEntry> {
    preferred_font_by_keywords(
        system_fonts,
        &[
            "droidsansfallbackfull",
            "arialunicode",
            "hiraginosansgb",
            "stheitimedium",
            "pingfang",
            "notosanscjk",
            "notosanssc",
            "sourcehansanssc",
            "wqyzenhei",
            "wenquanyi",
            "microsoftyahei",
            "msyh",
            "simsun",
        ],
    )
}

/// 按旧逻辑选择第一个命中关键字的系统字体。
pub(super) fn preferred_font_by_keywords<'a>(
    system_fonts: &'a [SystemFontEntry],
    keywords: &[&str],
) -> Option<&'a SystemFontEntry> {
    keywords.iter().find_map(|keyword| {
        system_fonts
            .iter()
            .find(|font| system_font_matches_keyword(font, keyword))
    })
}

/// 用归一化关键字检查系统字体。
pub(super) fn system_font_matches_keyword(font: &SystemFontEntry, keyword: &str) -> bool {
    normalized_font_search_text(font).contains(keyword)
}

/// 从展示名和文件名构建归一化搜索文本。
pub(super) fn normalized_font_search_text(font: &SystemFontEntry) -> String {
    let file = font
        .path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .unwrap_or_default();
    format!(
        "{}{}",
        normalized_font_match_text(&font.name),
        normalized_font_match_text(file)
    )
}

/// 归一化字体名和文件名，供偏好匹配使用。
pub(super) fn normalized_font_match_text(text: &str) -> String {
    text.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Builds inherited per-surface font settings for first-run defaults.
pub(super) fn default_inherited_font_surface_settings() -> data::FontSurfaceSettings {
    data::FontSurfaceSettings {
        family: data::FontFamilySetting::Default,
        ..data::FontSurfaceSettings::default()
    }
}

pub(super) fn collect_system_fonts(root: &Path, fonts: &mut Vec<SystemFontEntry>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_system_fonts(&path, fonts);
        } else if is_font_file(&path) {
            let name = system_font_display_name(&path);
            let search_key = name.to_lowercase();
            let size_bytes = entry
                .metadata()
                .map(|metadata| metadata.len())
                .unwrap_or(u64::MAX);
            fonts.push(SystemFontEntry {
                name,
                path,
                size_bytes,
                search_key,
            });
        }
    }
}

pub(super) fn system_font_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(target_os = "windows")]
    {
        if let Some(windir) = env::var_os("WINDIR") {
            roots.push(PathBuf::from(windir).join("Fonts"));
        }
        if let Some(system_root) = env::var_os("SystemRoot") {
            roots.push(PathBuf::from(system_root).join("Fonts"));
        }
        if let Some(local) = env::var_os("LOCALAPPDATA") {
            roots.push(
                PathBuf::from(local)
                    .join("Microsoft")
                    .join("Windows")
                    .join("Fonts"),
            );
        }
    }
    #[cfg(target_os = "macos")]
    {
        roots.push(PathBuf::from("/System/Library/Fonts"));
        roots.push(PathBuf::from("/Library/Fonts"));
        if let Some(home) = crate::home::home_dir() {
            roots.push(home.join("Library").join("Fonts"));
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        roots.push(PathBuf::from("/usr/share/fonts"));
        roots.push(PathBuf::from("/usr/local/share/fonts"));
        if let Some(home) = crate::home::home_dir() {
            roots.push(home.join(".local").join("share").join("fonts"));
            roots.push(home.join(".fonts"));
        }
    }
    roots
}

pub(super) fn is_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc" | "otc"
            )
        })
        .unwrap_or(false)
}

pub(super) fn system_font_display_name(path: &Path) -> String {
    path.file_stem()
        .map(|name| name.to_string_lossy().replace(['_', '-'], " "))
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

pub(super) fn font_matches_filter_key(font: &SystemFontEntry, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    font.search_key.contains(filter) || fuzzy_subsequence_match(&font.search_key, filter)
}

pub(super) fn fuzzy_subsequence_match(value: &str, filter: &str) -> bool {
    let mut chars = value.chars();
    filter.chars().all(|needle| chars.any(|ch| ch == needle))
}

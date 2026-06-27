//! App 子模块共享的轻量 helper。
//!
//! 这里放路径、截图、脚本、快捷键和小型 UI 控件 helper。它们不拥有
//! app 状态，也不能直接触发后台工作。

use super::*;
use crate::gui::repaint_gate;
use std::thread;

/// 判断 workspace watcher 是否应该忽略这个路径。
///
/// 触发条件：notify 上报构建产物或缓存目录里的改动。
/// 不能只在 outline 层过滤：watcher 仍会制造无意义刷新事件。
/// 防止回归：dist/.nx 等目录频繁变化时不会打断当前 UI 状态。
pub(super) fn should_skip_workspace_watch_path(workspace_root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(workspace_root).unwrap_or(path);
    relative.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        WORKSPACE_WATCH_IGNORED_DIRS.contains(&name.as_ref())
    })
}

pub(super) fn text_event_matches(event: &egui::Event, value: &str) -> bool {
    matches!(event, egui::Event::Text(text) if text == value)
}

/// Extracts IME commits that still need manual text insertion.
pub(super) fn markdown_editor_ime_commit_texts_from_events(events: &[egui::Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            egui::Event::Ime(egui::ImeEvent::Commit(text)) if !text.is_empty() => {
                Some(text.clone())
            }
            _ => None,
        })
        .filter(|commit| !events.iter().any(|event| text_event_matches(event, commit)))
        .collect()
}

/// 将 TextEdit 的 IME 区域稳定到当前光标附近。
pub(super) fn stabilize_text_edit_ime_output(
    ui: &Ui,
    output: &TextEditOutput,
    font_id: &egui::FontId,
) {
    if !output.response.has_focus() {
        return;
    }
    let Some(cursor_range) = output.cursor_range else {
        return;
    };
    let row_height = ui.fonts(|fonts| fonts.row_height(font_id));
    let cursor_rect = egui::text_selection::text_cursor_state::cursor_rect(
        &output.galley,
        &cursor_range.primary,
        row_height,
    )
    .translate(output.galley_pos.to_vec2());
    let to_global = ui
        .ctx()
        .layer_transform_to_global(ui.layer_id())
        .unwrap_or_default();
    let cursor_rect = to_global * cursor_rect;
    // 触发条件：egui-winit 0.31 将 IMEOutput.rect 传给系统候选窗。
    // 不能使用 TextEdit 默认整块编辑区：大文档滚动时候选窗会远离光标。
    // 防止回归：中文输入法候选栏漂移或偶尔不出现。
    ui.ctx().output_mut(|platform_output| {
        platform_output.ime = Some(egui::output::IMEOutput {
            rect: cursor_rect,
            cursor_rect,
        });
    });
}

/// 构建 Codex OAuth token 请求客户端，适用于 Settings 里的代理配置。
pub(super) fn codex_auth_client(settings: &NetworkSettings) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder();
    let proxy = settings.proxy.trim();
    if settings.proxy_enabled && !proxy.is_empty() {
        let no_proxy = reqwest::NoProxy::from_string(&settings.effective_no_proxy());
        let proxy = reqwest::Proxy::all(proxy)
            .map_err(|error| format!("invalid Codex auth proxy: {error}"))?
            .no_proxy(no_proxy);
        builder = builder.proxy(proxy);
    }
    builder
        .build()
        .map_err(|error| format!("failed to build Codex auth HTTP client: {error}"))
}

/// 返回 modal dialog 的绘制层级，适用于覆盖 terminal 抽屉的窗口。
pub(super) fn modal_dialog_order() -> egui::Order {
    // 触发条件：workspace terminal/Helix 抽屉使用 Foreground overlay。
    // 不能走默认 Middle：Settings 会被抽屉盖住但仍拦截键盘输入。
    // 防止回归：隐藏 dialog 让终端和导航同时看起来无响应。
    egui::Order::Foreground
}

pub(super) fn theme_mode_label(mode: theme::ThemeMode) -> &'static str {
    match mode {
        theme::ThemeMode::Light => "light",
        theme::ThemeMode::Dark => "dark",
    }
}

pub(super) fn document_open_mode(current: CenterMode) -> CenterMode {
    match current {
        CenterMode::Preview => CenterMode::Preview,
        _ => CenterMode::Editor,
    }
}

/// Converts the reviewer terminal-style minimum width into UI pixels.
pub(super) fn reviewer_min_width_px() -> f32 {
    f32::from(crate::reviewer::MIN_REVIEWER_WIDTH) * 8.5
}

/// Returns whether optional screenshot request file polling is enabled.
pub(super) fn screenshot_request_poll_enabled() -> bool {
    env::var_os(SCREENSHOT_REQUEST_POLL_ENV)
        .and_then(|value| value.into_string().ok())
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
}

/// Normalizes paths for watcher registration and event matching.
pub(super) fn comparable_watch_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

/// Finds an existing path suitable for registering a filesystem watcher.
pub(super) fn nearest_existing_watch_path(path: &Path) -> PathBuf {
    let mut candidate = comparable_watch_path(path);
    loop {
        if candidate.exists() {
            return candidate;
        }
        if !candidate.pop() {
            return PathBuf::from(".");
        }
    }
}

/// Returns the minimum interval enforced between app-scheduled frames.
pub(super) fn max_frame_rate_interval(max_frame_rate: u16) -> Duration {
    let max_frame_rate = max_frame_rate.clamp(data::MIN_MAX_FRAME_RATE, data::MAX_MAX_FRAME_RATE);
    Duration::from_secs_f64(1.0 / f64::from(max_frame_rate))
}

/// 返回 Busy Agent 无输出自动继续的等待时间。
pub(super) fn agent_busy_auto_go_delay(settings: &RuntimeSettings) -> Duration {
    let minutes = settings.agent_busy_auto_go_minutes.clamp(
        data::MIN_AGENT_BUSY_AUTO_GO_MINUTES,
        data::MAX_AGENT_BUSY_AUTO_GO_MINUTES,
    );
    Duration::from_secs(u64::from(minutes) * 60)
}

/// Returns custom Agent quick replies as trimmed non-empty lines.
pub(super) fn agent_custom_quick_reply_lines(value: &str) -> Vec<&str> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

/// Returns how long remains before a periodic task is due.
pub(super) fn duration_until_due(last: Instant, interval: Duration) -> Duration {
    interval.saturating_sub(last.elapsed())
}

/// Returns the earliest available duration while allowing absent timers.
pub(super) fn min_optional_duration(
    left: Option<Duration>,
    right: Option<Duration>,
) -> Option<Duration> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

pub(super) fn toast_card(ui: &mut Ui, toast: &Toast) {
    Frame::new()
        .fill(theme::surface_elevated())
        .stroke(Stroke::new(1.0, theme::accent_border(toast.color)))
        .corner_radius(CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(280.0);
            ui.horizontal(|ui| {
                status_dot(ui, toast.color);
                ui.label(RichText::new(&toast.message).strong().color(theme::text()));
            });
        });
}

pub(super) fn terminal_error_panel(
    ui: &mut Ui,
    title: &'static str,
    message: &str,
    hint: &'static str,
    language: AppLanguage,
) -> bool {
    let mut retry = false;
    let message = if message == "No backend error was reported." {
        i18n::text(language, "No backend error was reported.")
    } else {
        message
    };
    Frame::new()
        .fill(theme::surface_elevated())
        .stroke(Stroke::new(1.0, theme::danger_border()))
        .corner_radius(CornerRadius::same(theme::RADIUS_LG))
        .inner_margin(Margin::same(20))
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.label(RichText::new("!").size(28.0).color(theme::danger()));
                ui.vertical(|ui| {
                    ui.colored_label(theme::danger(), i18n::text(language, title));
                    ui.label(muted(message));
                    ui.label(muted(i18n::text(language, hint)));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    retry = secondary_action(ui, i18n::text(language, "Retry")).clicked();
                });
            });
        });
    retry
}

pub(super) fn terminal_page_header(
    ui: &mut Ui,
    icon: &str,
    title: &str,
    description: &str,
    workspace_name: &str,
    workspace_path: &str,
    detail: &str,
    status: (&str, Color32),
) {
    Frame::new()
        .fill(theme::surface_elevated())
        .stroke(Stroke::new(1.0, theme::border()))
        .corner_radius(CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                pill_icon(ui, icon, theme::primary());
                header_info_block(ui, title, description, 210.0);
                header_info_block(ui, "Workspace", &truncate_middle(workspace_name, 28), 160.0);
                header_info_block(ui, "Root", &truncate_middle(workspace_path, 56), 280.0);
                header_info_block(ui, "Session", &truncate_middle(detail, 34), 190.0);
                badge(ui, status.0, status.1);
            });
        });
}

pub(super) fn header_info_block(ui: &mut Ui, label: &str, value: &str, width: f32) {
    ui.allocate_ui_with_layout(
        Vec2::new(width.min(ui.available_width()), 46.0),
        Layout::top_down(Align::Min),
        |ui| {
            ui.label(RichText::new(label).size(11.0).color(theme::muted()));
            ui.add_sized(
                [ui.available_width(), 22.0],
                egui::Label::new(
                    RichText::new(value)
                        .strong()
                        .size(13.0)
                        .color(theme::text()),
                )
                .truncate(),
            );
        },
    );
}

/// Rebuilds branch dialog visibility when the user edits the filter.
pub(super) fn filtered_branch_indices(branches: &[BranchInfo], filter: &str) -> Vec<usize> {
    let needle = filter.trim().to_lowercase();
    branches
        .iter()
        .enumerate()
        .filter_map(|(index, branch)| {
            (needle.is_empty() || branch.label.to_lowercase().contains(&needle)).then_some(index)
        })
        .collect()
}

pub(super) fn display_path(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

pub(super) fn workspace_title_path(workspace: &WorkspaceViewData, path: &Path) -> String {
    path.strip_prefix(&workspace.path)
        .ok()
        .map(display_path)
        .unwrap_or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| display_path(path))
        })
}

pub(super) fn truncate_middle(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars || max_chars < 8 {
        return value.to_string();
    }
    let left_count = (max_chars - 3) / 2;
    let right_count = max_chars - 3 - left_count;
    let left: String = value.chars().take(left_count).collect();
    let right: String = value
        .chars()
        .rev()
        .take(right_count)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{left}...{right}")
}

pub(super) fn resolve_workspace_path(workspace_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    }
}

/// 在系统文件管理器中定位路径，适用于 outline 和终端路径点击。
pub(super) fn reveal_path_command(absolute: &Path) -> Result<(), String> {
    let result = if cfg!(target_os = "macos") {
        Command::new("open").arg("-R").arg(absolute).status()
    } else if cfg!(target_os = "windows") {
        Command::new("explorer")
            .arg("/select,")
            .arg(absolute)
            .status()
    } else {
        // 触发条件：Linux 文件管理器没有统一的 reveal-file 参数。
        // 不能对目录也打开 parent：目录点击要直接定位到该目录。
        // 防止回归：终端目录路径点击只打开上级目录。
        let target = if absolute.is_dir() {
            absolute
        } else {
            absolute.parent().unwrap_or(absolute)
        };
        Command::new("xdg-open").arg(target).status()
    };
    result.map(|_| ()).map_err(|error| error.to_string())
}

pub(super) fn load_reviewer_scripts() -> io::Result<Vec<ReviewerScript>> {
    let Some(dir) = reviewer_script_dir() else {
        return Ok(Vec::new());
    };
    load_reviewer_scripts_from_dir(&dir)
}

pub(super) fn load_reviewer_scripts_from_dir(dir: &Path) -> io::Result<Vec<ReviewerScript>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut scripts = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("sh") {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let tip = reviewer_script_tip(&path)?;
        scripts.push(ReviewerScript {
            label: file_name.to_string(),
            path,
            tip,
        });
    }
    scripts.sort_by(|left, right| left.label.cmp(&right.label));
    Ok(scripts)
}

pub(super) fn reviewer_script_requires_confirm(script: &ReviewerScript) -> bool {
    script.label.ends_with("-confirm.sh")
}

pub(super) fn reviewer_script_tip(path: &Path) -> io::Result<Option<String>> {
    let file = fs::File::open(path)?;
    let mut lines = BufReader::new(file).lines();
    let _ = lines.next();
    let Some(line) = lines.next().transpose()? else {
        return Ok(None);
    };
    let Some(tip) = line.strip_prefix("#tip:") else {
        return Ok(None);
    };
    let tip = tip.trim();
    if tip.is_empty() {
        Ok(None)
    } else {
        Ok(Some(tip.to_string()))
    }
}

pub(super) fn reviewer_script_dir() -> Option<PathBuf> {
    env::var_os("GSDV_REVIEWER_SCRIPT_DIR")
        .map(PathBuf::from)
        .or_else(|| crate::home::home_dir().map(|home| home.join(".gsdv/reviewer")))
}

pub(super) fn run_reviewer_script_process(
    tx: Sender<AppEvent>,
    repaint_ctx: Option<egui::Context>,
    repaint_controller: repaint_gate::RepaintController,
    script_label: String,
    script_path: PathBuf,
    target_label: String,
    target_root: PathBuf,
    project_dir: PathBuf,
    repo_name: String,
    network_settings: NetworkSettings,
) {
    let mut command = Command::new("sh");
    if let Some(gws) = data::ensure_workspace_store_dir(&project_dir) {
        command.env("GWS", gws);
    }
    let spawn_result = command
        .arg(&script_path)
        .arg(&target_root)
        .arg(&project_dir)
        .arg(&target_label)
        .current_dir(&target_root)
        .envs(network_settings.env_vars())
        .env("GSDV_REVIEWER_SCRIPT", &script_path)
        .env("GSDV_REVIEWER_PROJECT_DIR", &project_dir)
        .env("GSDV_REVIEWER_REPO_ROOT", &target_root)
        .env("GSDV_REVIEWER_REPO_LABEL", &target_label)
        .env("GSDV_REVIEWER_REPO", &repo_name)
        .env("GSDV_WORKSPACE_DIR", &project_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let mut child = match spawn_result {
        Ok(child) => child,
        Err(error) => {
            send_notification(
                &tx,
                repaint_ctx.as_ref(),
                &repaint_controller,
                format!("[script] failed to start {script_label} for {target_label}: {error}"),
            );
            return;
        }
    };

    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        readers.push(spawn_script_pipe_reader(
            stdout,
            "stdout",
            tx.clone(),
            repaint_ctx.clone(),
            repaint_controller.clone(),
        ));
    }
    if let Some(stderr) = child.stderr.take() {
        readers.push(spawn_script_pipe_reader(
            stderr,
            "stderr",
            tx.clone(),
            repaint_ctx.clone(),
            repaint_controller.clone(),
        ));
    }

    let wait_result = child.wait();
    for reader in readers {
        let _ = reader.join();
    }

    match wait_result {
        Ok(status) => {
            let exit = status
                .code()
                .map(|code| format!("code {code}"))
                .unwrap_or_else(|| status.to_string());
            send_notification(
                &tx,
                repaint_ctx.as_ref(),
                &repaint_controller,
                format!("[exit] {script_label} for {target_label}: {exit}"),
            );
        }
        Err(error) => send_notification(
            &tx,
            repaint_ctx.as_ref(),
            &repaint_controller,
            format!("[exit] {script_label} for {target_label}: wait failed: {error}"),
        ),
    }
}

pub(super) fn reviewer_script_reason_line(
    script_label: &str,
    target_label: &str,
    target_root: &Path,
) -> String {
    format!(
        "[script] because reviewer repo action was requested for {target_label}, running {script_label} in {}",
        target_root.display()
    )
}

pub(super) fn spawn_script_pipe_reader<R>(
    reader: R,
    stream: &'static str,
    tx: Sender<AppEvent>,
    repaint_ctx: Option<egui::Context>,
    repaint_controller: repaint_gate::RepaintController,
) -> thread::JoinHandle<()>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut bytes = Vec::new();
        loop {
            bytes.clear();
            match reader.read_until(b'\n', &mut bytes) {
                Ok(0) => break,
                Ok(_) => {
                    trim_line_ending(&mut bytes);
                    let text = String::from_utf8_lossy(&bytes);
                    send_notification(
                        &tx,
                        repaint_ctx.as_ref(),
                        &repaint_controller,
                        format!("[{stream}] {text}"),
                    );
                }
                Err(error) => {
                    send_notification(
                        &tx,
                        repaint_ctx.as_ref(),
                        &repaint_controller,
                        format!("[{stream}] read failed: {error}"),
                    );
                    break;
                }
            }
        }
    })
}

pub(super) fn trim_line_ending(bytes: &mut Vec<u8>) {
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
}

/// 投递 reviewer script 输出，并唤醒通知抽屉刷新。
pub(super) fn send_notification(
    tx: &Sender<AppEvent>,
    repaint_ctx: Option<&egui::Context>,
    repaint_controller: &repaint_gate::RepaintController,
    line: String,
) {
    if tx.send(AppEvent::Notification(line)).is_ok()
        && let Some(ctx) = repaint_ctx
    {
        repaint_controller.request_repaint(ctx);
    }
}

pub(super) fn screenshot_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("target")
        .join("gsdv-screenshots")
}

pub(super) fn screenshot_request_path() -> PathBuf {
    screenshot_dir().join(SCREENSHOT_REQUEST_FILE)
}

pub(super) fn screenshot_latest_path() -> PathBuf {
    screenshot_dir().join("latest.txt")
}

pub(super) fn save_color_image_png(
    path: &Path,
    color_image: &egui::ColorImage,
) -> Result<(), String> {
    let mut rgba = Vec::with_capacity(color_image.pixels.len() * 4);
    for pixel in &color_image.pixels {
        rgba.extend_from_slice(&[pixel.r(), pixel.g(), pixel.b(), pixel.a()]);
    }
    save_rgba_image_png(
        path,
        color_image.size[0] as u32,
        color_image.size[1] as u32,
        &rgba,
    )
}

pub(super) fn save_rgba_image_png(
    path: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    image::save_buffer(path, rgba, width, height, image::ColorType::Rgba8)
        .map_err(|error| error.to_string())
}

pub(super) fn section_label(text: &str) -> RichText {
    RichText::new(text)
        .strong()
        .size(11.0)
        .color(theme::muted())
}

pub(super) fn muted(text: &str) -> RichText {
    RichText::new(text).color(theme::muted())
}

pub(super) fn primary_action(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add_sized(
        [action_button_width(text, 76.0, 156.0), 32.0],
        Button::new(RichText::new(text).color(theme::on_primary()))
            .fill(theme::primary())
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(theme::RADIUS_SM)),
    )
}

pub(super) fn secondary_action(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add_sized(
        [action_button_width(text, 86.0, 168.0), 32.0],
        Button::new(RichText::new(text).color(theme::text()))
            .fill(theme::surface_elevated())
            .stroke(Stroke::new(1.0, theme::border()))
            .corner_radius(CornerRadius::same(theme::RADIUS_SM)),
    )
}

pub(super) fn markdown_name_suggestion_button(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(
        Button::new(RichText::new(text).size(12.0).color(theme::text()))
            .small()
            .fill(theme::surface_elevated())
            .stroke(Stroke::new(1.0, theme::border()))
            .corner_radius(CornerRadius::same(theme::RADIUS_SM)),
    )
}

pub(super) fn help_entry_button(ui: &mut Ui, language: AppLanguage) -> egui::Response {
    let text = i18n::text(language, "Help");
    ui.add_sized(
        [action_button_width(text, 86.0, 132.0), 26.0],
        Button::new(RichText::new(text).strong().color(theme::primary()))
            .fill(theme::primary_soft())
            .stroke(Stroke::new(1.0, theme::primary_border()))
            .corner_radius(CornerRadius::same(theme::RADIUS_SM)),
    )
    .on_hover_text(i18n::text(language, "Open keyboard help"))
}

/// 构造右上角手动休息按钮。
pub(super) fn rest_entry_button(
    ui: &mut Ui,
    enabled: bool,
    language: AppLanguage,
) -> egui::Response {
    let text = i18n::text(language, "Rest");
    ui.add_enabled_ui(enabled, |ui| {
        ui.add_sized(
            [action_button_width(text, 86.0, 132.0), 26.0],
            Button::new(RichText::new(text).strong().color(theme::warning()))
                .fill(theme::surface_elevated())
                .stroke(Stroke::new(1.0, theme::border()))
                .corner_radius(CornerRadius::same(theme::RADIUS_SM)),
        )
    })
    .inner
    .on_hover_text(if enabled {
        i18n::text(language, "Start rest now")
    } else {
        i18n::text(language, "Pomodoro is disabled in settings")
    })
}

/// 构造右上角手动工作按钮。
pub(super) fn work_entry_button(
    ui: &mut Ui,
    enabled: bool,
    language: AppLanguage,
) -> egui::Response {
    let text = i18n::text(language, "Work");
    ui.add_enabled_ui(enabled, |ui| {
        ui.add_sized(
            [action_button_width(text, 86.0, 132.0), 26.0],
            Button::new(RichText::new(text).strong().color(theme::primary()))
                .fill(theme::surface_elevated())
                .stroke(Stroke::new(1.0, theme::border()))
                .corner_radius(CornerRadius::same(theme::RADIUS_SM)),
        )
    })
    .inner
    .on_hover_text(if enabled {
        i18n::text(language, "Start work now")
    } else {
        i18n::text(language, "Pomodoro is disabled in settings")
    })
}

/// 返回配置里的工作时长。
pub(super) fn pomodoro_work_duration(settings: &RuntimeSettings) -> Duration {
    Duration::from_secs(u64::from(settings.pomodoro_work_minutes.max(1)) * 60)
}

/// 返回配置里的休息时长。
pub(super) fn pomodoro_rest_duration(settings: &RuntimeSettings) -> Duration {
    Duration::from_secs(u64::from(settings.pomodoro_rest_minutes.max(1)) * 60)
}

/// 返回休息哈基米标签需要显示的剩余时间。
pub(super) fn pomodoro_rest_remaining(
    settings: &RuntimeSettings,
    state: &PomodoroState,
    now: Instant,
) -> Duration {
    pomodoro_rest_duration(settings).saturating_sub(now.duration_since(state.phase_started_at))
}

/// 把时长格式化成紧凑计时标签。
pub(super) fn format_minutes_seconds(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    format!("{:02}:{:02}", total_seconds / 60, total_seconds % 60)
}

/// 返回顶部计时条使用的工作完成进度。
pub(super) fn pomodoro_work_progress(settings: &RuntimeSettings, state: &PomodoroState) -> f32 {
    let total = pomodoro_work_duration(settings).as_secs_f32().max(1.0);
    (Instant::now()
        .duration_since(state.phase_started_at)
        .as_secs_f32()
        / total)
        .clamp(0.0, 1.0)
}

/// 返回哈基米开始预警时的工作进度。
pub(super) fn pomodoro_warning_progress(settings: &RuntimeSettings) -> f32 {
    let remaining = settings.pomodoro_warning_remaining_percent.clamp(
        data::MIN_POMODORO_WARNING_REMAINING_PERCENT,
        data::MAX_POMODORO_WARNING_REMAINING_PERCENT,
    );
    1.0 - f32::from(remaining) / 100.0
}

/// 检测能让哈基米离开待工作状态的用户输入。
pub(super) fn pomodoro_ready_input_detected(input: &egui::InputState) -> bool {
    input.events.iter().any(|event| match event {
        egui::Event::PointerMoved(_) => true,
        egui::Event::PointerButton { pressed: true, .. } => true,
        egui::Event::Key { pressed: true, .. } => true,
        _ => false,
    })
}

/// 解码内置哈基米图片。
pub(super) fn hajimi_color_image() -> Option<egui::ColorImage> {
    let image = image::load_from_memory(include_bytes!("../../assets/hajimi.png")).ok()?;
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(
        size,
        rgba.as_raw(),
    ))
}

/// 绘制带轻微状态动画的哈基米。
pub(super) fn draw_hajimi_cat(
    ui: &mut Ui,
    rect: Rect,
    texture: &egui::TextureHandle,
    phase: PomodoroPhase,
    phase_elapsed: Duration,
) {
    let painter = ui.painter();
    let t = phase_elapsed.as_secs_f32();
    let bounce = (t * 8.0).sin() * 3.0;
    let sway = (t * 3.2).sin() * 4.0;
    let scale = 1.0 + (t * 5.4).sin() * 0.012;
    let animated = Rect::from_center_size(rect.center(), rect.size() * scale);
    let base = animated.translate(Vec2::new(sway, bounce));
    painter.image(
        texture.id(),
        base,
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    match phase {
        PomodoroPhase::WaitingForRestQuiet => {
            draw_pomodoro_rest_quiet_questions(painter, base, phase_elapsed);
        }
        PomodoroPhase::ReadyToWork => {
            painter.text(
                egui::pos2(base.center().x, base.top() - 8.0),
                Align2::CENTER_CENTER,
                "?",
                egui::FontId::proportional(30.0),
                theme::warning(),
            );
        }
        PomodoroPhase::ReturningToWork => {
            draw_pomodoro_return_questions(painter, base, phase_elapsed);
        }
        PomodoroPhase::Working | PomodoroPhase::Resting => {}
    }
}

/// 绘制等待安静阶段的 5 个旋转问号。
pub(super) fn draw_pomodoro_rest_quiet_questions(
    painter: &egui::Painter,
    cat_rect: Rect,
    elapsed: Duration,
) {
    let positions = pomodoro_rest_quiet_question_positions(cat_rect, elapsed);
    let font = egui::FontId::proportional(26.0);
    for position in positions {
        painter.text(
            position,
            Align2::CENTER_CENTER,
            "?",
            font.clone(),
            theme::warning(),
        );
    }
}

/// 计算等待安静阶段的问号圆环位置。
pub(super) fn pomodoro_rest_quiet_question_positions(
    cat_rect: Rect,
    elapsed: Duration,
) -> Vec<egui::Pos2> {
    let center = egui::pos2(cat_rect.center().x, cat_rect.top() + 18.0);
    let base_angle = elapsed.as_secs_f32() * std::f32::consts::TAU * 0.9;
    let radius = 30.0;
    (0..POMODORO_REST_QUIET_QUESTION_COUNT)
        .map(|index| {
            let angle = base_angle
                + index as f32 / POMODORO_REST_QUIET_QUESTION_COUNT as f32 * std::f32::consts::TAU;
            center + Vec2::new(angle.cos() * radius, angle.sin() * radius)
        })
        .collect()
}

/// 绘制哈基米退出休息模式时的问号。
pub(super) fn draw_pomodoro_return_questions(
    painter: &egui::Painter,
    cat_rect: Rect,
    elapsed: Duration,
) {
    let positions = pomodoro_return_question_positions(cat_rect, elapsed);
    let font = egui::FontId::proportional(26.0);
    for position in positions {
        painter.text(
            position,
            Align2::CENTER_CENTER,
            "?",
            font.clone(),
            theme::warning(),
        );
    }
}

/// 计算休息转工作退场动画里的问号位置。
pub(super) fn pomodoro_return_question_positions(
    cat_rect: Rect,
    elapsed: Duration,
) -> Vec<egui::Pos2> {
    let count = pomodoro_return_question_count(elapsed);
    if count == 0 {
        return Vec::new();
    }
    let center = egui::pos2(cat_rect.center().x, cat_rect.top() + 18.0);
    if elapsed < POMODORO_RETURN_QUESTION_RAMP {
        return (0..count)
            .map(|index| {
                let offset = (index as f32 - (count.saturating_sub(1)) as f32 * 0.5) * 18.0;
                center + Vec2::new(offset, 0.0)
            })
            .collect();
    }
    let elapsed_after_ramp = elapsed.saturating_sub(POMODORO_RETURN_QUESTION_RAMP);
    let spin_progress = elapsed_after_ramp.as_secs_f32()
        / POMODORO_RETURN_TO_WORK_DURATION
            .saturating_sub(POMODORO_RETURN_QUESTION_RAMP)
            .as_secs_f32()
            .max(0.001);
    let base_angle = spin_progress * std::f32::consts::TAU * 2.0;
    let radius = 30.0;
    (0..POMODORO_RETURN_QUESTION_COUNT)
        .map(|index| {
            let angle = base_angle
                + index as f32 / POMODORO_RETURN_QUESTION_COUNT as f32 * std::f32::consts::TAU;
            center + Vec2::new(angle.cos() * radius, angle.sin() * radius)
        })
        .collect()
}

/// 返回退场动画当前可见的问号数量。
pub(super) fn pomodoro_return_question_count(elapsed: Duration) -> usize {
    if elapsed >= POMODORO_RETURN_QUESTION_RAMP {
        return POMODORO_RETURN_QUESTION_COUNT;
    }
    let progress = elapsed.as_secs_f32() / POMODORO_RETURN_QUESTION_RAMP.as_secs_f32().max(0.001);
    ((progress * POMODORO_RETURN_QUESTION_COUNT as f32).floor() as usize + 1)
        .min(POMODORO_RETURN_QUESTION_COUNT)
}

/// 按休息模式的身体动画绘制工作末段哈基米。
pub(super) fn draw_hajimi_work_warning_cat(
    ui: &mut Ui,
    rect: Rect,
    texture: &egui::TextureHandle,
    reveal: f32,
    elapsed: Duration,
) {
    let reveal = reveal.clamp(0.0, 1.0);
    let t = elapsed.as_secs_f32();
    let bounce = (t * 8.0).sin() * 3.0;
    let sway = (t * 3.2).sin() * 4.0;
    let scale = 1.0 + (t * 5.4).sin() * 0.012;
    let animated = Rect::from_center_size(rect.center(), rect.size() * scale);
    let base = animated.translate(Vec2::new(sway, bounce));
    let blur = (1.0 - reveal) * 14.0;
    let tint =
        Color32::from_rgba_unmultiplied(255, 255, 255, ((0.20 + reveal * 0.80) * 255.0) as u8);
    let offsets = [
        Vec2::new(-blur, 0.0),
        Vec2::new(blur, 0.0),
        Vec2::new(0.0, -blur),
        Vec2::new(0.0, blur),
    ];
    for offset in offsets {
        ui.painter().image(
            texture.id(),
            base.translate(offset),
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::from_rgba_unmultiplied(255, 255, 255, ((1.0 - reveal) * 72.0) as u8),
        );
    }
    ui.painter().image(
        texture.id(),
        base,
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        tint,
    );
    draw_pomodoro_peek_orbit_text(ui.painter(), base, elapsed);
}

/// 绘制工作末段围绕哈基米旋转的提示文字。
pub(super) fn draw_pomodoro_peek_orbit_text(
    painter: &egui::Painter,
    cat_rect: Rect,
    elapsed: Duration,
) {
    let font = egui::FontId::new(18.0, theme::editor_system_font_family());
    let color = Color32::from_rgb(0xD9, 0x77, 0x06);
    for (ch, pos) in pomodoro_peek_orbit_text_positions(cat_rect, elapsed) {
        painter.text(pos, Align2::CENTER_CENTER, ch, font.clone(), color);
    }
}

/// 计算围绕哈基米旋转的每个提示字位置。
pub(super) fn pomodoro_peek_orbit_text_positions(
    cat_rect: Rect,
    elapsed: Duration,
) -> Vec<(char, egui::Pos2)> {
    let chars = POMODORO_PEEK_ORBIT_TEXT.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return Vec::new();
    }
    let center = cat_rect.center() + Vec2::new(0.0, -8.0);
    let radius_x = cat_rect.width() * 0.52;
    let radius_y = cat_rect.height() * 0.45;
    let base_angle = elapsed.as_secs_f32() * 1.65;
    chars
        .iter()
        .enumerate()
        .map(|(index, ch)| {
            let angle = base_angle + index as f32 / chars.len() as f32 * std::f32::consts::TAU;
            let pos = center + Vec2::new(angle.cos() * radius_x, angle.sin() * radius_y);
            (*ch, pos)
        })
        .collect()
}

pub(super) fn action_button_width(text: &str, min: f32, max: f32) -> f32 {
    (text.chars().count() as f32 * 7.0 + 28.0).clamp(min, max)
}

pub(super) fn subtle_icon_button(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add_sized(
        [32.0, 32.0],
        Button::new(RichText::new(icon_label(text)).color(theme::muted()))
            .fill(theme::transparent())
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(theme::RADIUS_SM)),
    )
    .on_hover_text(text)
}

pub(super) fn icon_label(name: &str) -> &'static str {
    match name {
        "Copy" => "C",
        "Add" => "+",
        "Panels" => "P",
        "Screenshot" => "S",
        "Filter" => "F",
        "More" => "...",
        "History" => "R",
        "Expand" => "v",
        _ => "?",
    }
}

pub(super) fn badge(ui: &mut Ui, text: &str, color: Color32) {
    Frame::new()
        .fill(theme::accent_soft(color))
        .stroke(Stroke::new(1.0, theme::accent_border(color)))
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color).size(12.0).strong());
        });
}

pub(super) fn pill_icon(ui: &mut Ui, text: &str, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(theme::RADIUS_SM), color);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(13.0),
        theme::bg(),
    );
}

#[cfg(test)]
pub(super) fn filtered_reviewer_rows(
    rows: &[crate::reviewer::app::GuiReviewerRow],
    filter: &str,
) -> Vec<(usize, crate::reviewer::app::GuiReviewerRow)> {
    let needle = filter.trim().to_lowercase();
    rows.iter()
        .cloned()
        .enumerate()
        .filter(|(_, row)| needle.is_empty() || row.label.to_lowercase().contains(&needle))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReviewerShortcutAction {
    Open,
    Exit,
}

pub(super) fn reviewer_shortcut_action(
    command: bool,
    alt: bool,
    r: bool,
    enter: bool,
    in_reviewer_route: bool,
) -> Option<ReviewerShortcutAction> {
    if (command || alt) && r {
        return Some(if in_reviewer_route {
            ReviewerShortcutAction::Exit
        } else {
            ReviewerShortcutAction::Open
        });
    }
    if command && enter {
        return Some(ReviewerShortcutAction::Open);
    }
    None
}

pub(super) fn help_shortcut_pressed(input: &egui::InputState) -> bool {
    let shortcut_modifier_down = command_or_alt_shortcut_modifier(input.modifiers);
    input
        .events
        .iter()
        .any(|event| help_shortcut_event_pressed(event, shortcut_modifier_down))
        || (input.key_pressed(egui::Key::Period)
            && command_or_alt_shortcut_modifier(input.modifiers))
}

pub(super) fn help_shortcut_action(command: bool, alt: bool, period: bool) -> bool {
    period && (command || alt)
}

pub(super) fn notification_shortcut_pressed(input: &egui::InputState) -> bool {
    input.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key {
                key: egui::Key::K,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } if command_or_alt_shortcut_modifier(*modifiers)
        ) || matches!(
            event,
            egui::Event::Key {
                physical_key: Some(egui::Key::K),
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } if command_or_alt_shortcut_modifier(*modifiers)
        )
    }) || (input.key_pressed(egui::Key::K) && command_or_alt_shortcut_modifier(input.modifiers))
}

pub(super) fn editor_preview_shortcut_pressed(input: &egui::InputState) -> bool {
    input.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key {
                key: egui::Key::E,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } if command_or_alt_shortcut_modifier(*modifiers)
        ) || matches!(
            event,
            egui::Event::Key {
                physical_key: Some(egui::Key::E),
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } if command_or_alt_shortcut_modifier(*modifiers)
        )
    }) || (input.key_pressed(egui::Key::E) && command_or_alt_shortcut_modifier(input.modifiers))
}

pub(super) fn tree_collapse_shortcut_pressed(input: &egui::InputState) -> bool {
    input.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key {
                key: egui::Key::W,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } if !modifiers.command && !modifiers.mac_cmd && !modifiers.ctrl && !modifiers.alt
        ) || matches!(
            event,
            egui::Event::Key {
                physical_key: Some(egui::Key::W),
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } if !modifiers.command && !modifiers.mac_cmd && !modifiers.ctrl && !modifiers.alt
        )
    }) || (input.key_pressed(egui::Key::W)
        && !input.modifiers.command
        && !input.modifiers.mac_cmd
        && !input.modifiers.ctrl
        && !input.modifiers.alt)
}

pub(super) fn outline_favorite_filter_shortcut_pressed(input: &egui::InputState) -> bool {
    input.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key {
                key: egui::Key::V,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } if !modifiers.command && !modifiers.mac_cmd && !modifiers.ctrl && !modifiers.alt
        ) || matches!(
            event,
            egui::Event::Key {
                physical_key: Some(egui::Key::V),
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } if !modifiers.command && !modifiers.mac_cmd && !modifiers.ctrl && !modifiers.alt
        )
    }) || (input.key_pressed(egui::Key::V)
        && !input.modifiers.command
        && !input.modifiers.mac_cmd
        && !input.modifiers.ctrl
        && !input.modifiers.alt)
}

/// Returns whether a text edit route should keep a shortcut from globals.
pub(super) fn text_edit_pre_global_shortcut_consumed(
    input: &egui::InputState,
    text_edit_focused: bool,
) -> bool {
    text_edit_focused
        && (text_edit_cut_shortcut_pressed(input)
            || text_edit_copy_shortcut_pressed(input)
            || text_edit_undo_shortcut_pressed(input))
}

/// Detects cut commands generated by platform text editing shortcuts.
pub(super) fn text_edit_cut_shortcut_pressed(input: &egui::InputState) -> bool {
    input.events.iter().any(|event| match event {
        egui::Event::Cut => true,
        egui::Event::Key {
            key,
            physical_key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } => {
            (*key == egui::Key::X || *physical_key == Some(egui::Key::X))
                && text_edit_cut_modifiers(*modifiers)
        }
        _ => false,
    }) || (input.key_pressed(egui::Key::X) && text_edit_cut_modifiers(input.modifiers))
}

/// Checks cut modifiers while a text editor owns the focused route.
pub(super) fn text_edit_cut_modifiers(modifiers: egui::Modifiers) -> bool {
    modifiers.command || modifiers.mac_cmd || modifiers.ctrl
}

/// 检测文本编辑器内部的复制快捷键。
pub(super) fn text_edit_copy_shortcut_pressed(input: &egui::InputState) -> bool {
    input.events.iter().any(|event| match event {
        egui::Event::Copy => true,
        egui::Event::Key {
            key,
            physical_key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } => {
            (*key == egui::Key::C || *physical_key == Some(egui::Key::C))
                && text_edit_copy_modifiers(*modifiers)
        }
        _ => false,
    }) || (input.key_pressed(egui::Key::C) && text_edit_copy_modifiers(input.modifiers))
}

/// 判断当前修饰键是否是文本复制快捷键组合。
pub(super) fn text_edit_copy_modifiers(modifiers: egui::Modifiers) -> bool {
    (modifiers.command || modifiers.mac_cmd || modifiers.ctrl) && !modifiers.alt && !modifiers.shift
}

/// 检测文本编辑器内部的撤销/重做快捷键。
pub(super) fn text_edit_undo_shortcut_pressed(input: &egui::InputState) -> bool {
    input.events.iter().any(|event| match event {
        egui::Event::Key {
            key,
            physical_key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } => {
            (*key == egui::Key::Z || *physical_key == Some(egui::Key::Z))
                && text_edit_undo_modifiers(*modifiers)
        }
        _ => false,
    }) || (input.key_pressed(egui::Key::Z) && text_edit_undo_modifiers(input.modifiers))
}

/// 判断当前修饰键是否是文本撤销/重做快捷键组合。
pub(super) fn text_edit_undo_modifiers(modifiers: egui::Modifiers) -> bool {
    (modifiers.command || modifiers.mac_cmd || modifiers.ctrl) && !modifiers.alt
}

pub(super) fn preview_scroll_capture_offset(command: &str) -> Option<f32> {
    command
        .strip_prefix("preview-scroll:")
        .and_then(|value| value.trim().parse::<f32>().ok())
        .map(|value| value.max(0.0))
}

pub(super) fn help_shortcut_event_pressed(
    event: &egui::Event,
    shortcut_modifier_down: bool,
) -> bool {
    let egui::Event::Key {
        key,
        physical_key,
        pressed: true,
        repeat: false,
        modifiers,
        ..
    } = event
    else {
        return false;
    };
    (shortcut_modifier_down || command_or_alt_shortcut_modifier(*modifiers))
        && (matches!(key, egui::Key::Period) || *physical_key == Some(egui::Key::Period))
}

/// Checks app-level Cmd/Alt shortcuts without accepting egui's Ctrl-as-command alias.
pub(super) fn command_or_alt_shortcut_modifier(modifiers: egui::Modifiers) -> bool {
    !modifiers.ctrl && (modifiers.alt || modifiers.mac_cmd || modifiers.command)
}

#[cfg(target_os = "macos")]
pub(super) fn help_shortcut_label() -> &'static str {
    "Cmd+. or Alt+."
}

#[cfg(not(target_os = "macos"))]
pub(super) fn help_shortcut_label() -> &'static str {
    "Alt+."
}

#[cfg(target_os = "macos")]
pub(super) fn help_shortcut_keys() -> &'static [&'static str] {
    &["Cmd+.", "Alt+."]
}

#[cfg(not(target_os = "macos"))]
pub(super) fn help_shortcut_keys() -> &'static [&'static str] {
    &["Alt+."]
}

pub(super) fn reviewer_helix_shortcut_pressed(input: &egui::InputState) -> bool {
    input.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key {
                key: egui::Key::X,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } if command_or_alt_shortcut_modifier(*modifiers)
        ) || matches!(
            event,
            egui::Event::Key {
                physical_key: Some(egui::Key::X),
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } if command_or_alt_shortcut_modifier(*modifiers)
        )
    }) || (input.key_pressed(egui::Key::X) && command_or_alt_shortcut_modifier(input.modifiers))
        || (input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::Cut))
            && command_or_alt_shortcut_modifier(input.modifiers))
}

pub(super) fn status_dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

pub(super) fn separator_dot(ui: &mut Ui) {
    ui.label(muted("•"));
}

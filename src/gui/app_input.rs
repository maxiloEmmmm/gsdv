//! egui 原生输入到 AppEvent 的轻量解析。
//!
//! input runtime 只读取快照并产出具体 UI 事件，不能直接修改 App 状态，
//! 也不能消费 terminal、fs、store 等业务队列。

use super::*;

/// 解析一帧 egui 原生输入，产出可进入 AppEvent drain 的具体事件。
pub(super) fn process_input_runtime_request(request: InputRuntimeRequest) -> Vec<AppEvent> {
    let mut events = Vec::new();
    let mut terminal_input_suppressed = request.active_app_dialog_open
        || request.active_reviewer_dialog_open
        || request.extra_tools_open;
    let agent_escape_pressed = request.active_agent_busy
        && request.terminal_input_target == Some(TerminalSurfaceKind::Agent)
        && request.input.key_pressed(egui::Key::Escape);
    let agent_escape_debug = std::env::var_os("GSDV_AGENT_ESC_DEBUG").is_some();
    if agent_escape_debug && agent_escape_pressed {
        eprintln!(
            "[gsdv][agent-esc][input-snapshot] workspace={} slot={:?} wants_keyboard_input={} app_dialog={} reviewer_dialog={} extra_tools={} notifications={} target={:?} kitty={}",
            request.active_workspace,
            request.active_agent_slot,
            request.wants_keyboard_input,
            request.active_app_dialog_open,
            request.active_reviewer_dialog_open,
            request.extra_tools_open,
            request.notifications_open,
            request.terminal_input_target,
            request.terminal_kitty_keyboard_protocol,
        );
    }

    if request.pomodoro_enabled
        && matches!(
            request.pomodoro_phase,
            PomodoroPhase::WaitingForRestQuiet
                | PomodoroPhase::Resting
                | PomodoroPhase::ReadyToWork
        )
        && pomodoro_ready_input_detected(&request.input)
    {
        events.push(AppEvent::PomodoroInputDetected);
    }

    for event in &request.input.events {
        if let egui::Event::Screenshot {
            user_data, image, ..
        } = event
        {
            let purpose = user_data
                .data
                .as_ref()
                .and_then(|data| data.downcast_ref::<ScreenshotPurpose>())
                .cloned();
            events.push(AppEvent::ScreenshotCaptured {
                purpose,
                image: image.clone(),
            });
        }
    }

    match input_runtime_keyboard_action(&request) {
        InputRuntimeKeyboardAction::Command(command) => {
            terminal_input_suppressed = true;
            if agent_escape_debug && agent_escape_pressed {
                eprintln!(
                    "[gsdv][agent-esc][input-action] command={:?} suppress_terminal=true",
                    command
                );
            }
            events.push(AppEvent::InputUiCommand(command));
        }
        InputRuntimeKeyboardAction::ReviewerDiff(action) => {
            terminal_input_suppressed = true;
            events.push(AppEvent::InputReviewerDiffAction(action));
        }
        InputRuntimeKeyboardAction::SuppressTerminalInput => {
            terminal_input_suppressed = true;
            if agent_escape_debug && agent_escape_pressed {
                eprintln!("[gsdv][agent-esc][input-action] suppress_terminal=true");
            }
        }
        InputRuntimeKeyboardAction::None => {}
    }

    if !terminal_input_suppressed
        && !request.terminal_surface_owns_input
        && !request.notifications_open
        && !request.reviewer_helix_open
        && !request.wants_keyboard_input
        && let Some(target) = request.terminal_input_target
    {
        let bytes = agent_input_bytes_from_events_with_kitty_protocol(
            &request.input.events,
            request.input.modifiers,
            /*copy_event_can_interrupt*/ true,
            request.terminal_kitty_keyboard_protocol,
            None,
        );
        if !bytes.is_empty() {
            if target == TerminalSurfaceKind::Agent
                && request.active_agent_busy
                && bytes.contains(&0x1b)
                && agent_escape_debug
            {
                eprintln!(
                    "[gsdv][agent-esc][input-runtime] workspace={} slot={:?} target={:?} busy={} kitty={} bytes={:?}",
                    request.active_workspace,
                    request.active_agent_slot,
                    target,
                    request.active_agent_busy,
                    request.terminal_kitty_keyboard_protocol,
                    bytes
                );
            }
            events.push(AppEvent::InputTerminalBytes {
                workspace_index: request.active_workspace,
                target,
                agent_slot: request.active_agent_slot,
                bytes,
            });
        }
    }

    events
}

/// input runtime 的键盘路由结果。
enum InputRuntimeKeyboardAction {
    /// 解析出 app UI 命令。
    Command(UiCommand),
    /// 解析出 reviewer diff 动作。
    ReviewerDiff(crate::gui::diff_viewer::DiffViewerAction),
    /// 输入已被当前 route 消费，只需要阻止 terminal 默认输入。
    SuppressTerminalInput,
    /// 当前输入不属于 app 快捷键。
    None,
}

/// 按当前 UI route 快照解析键盘输入。
fn input_runtime_keyboard_action(request: &InputRuntimeRequest) -> InputRuntimeKeyboardAction {
    let input = &request.input;
    let recent_markdown_shortcut = input.key_pressed(egui::Key::F1);
    let recent_agent_helix_targets_shortcut =
        command_or_alt_shortcut_modifier(input.modifiers) && input.key_pressed(egui::Key::D);
    if request.recent_markdown_dialog_open && recent_markdown_shortcut {
        return InputRuntimeKeyboardAction::Command(UiCommand::ToggleRecentMarkdownOutline);
    }
    if request.recent_agent_helix_targets_dialog_open && recent_agent_helix_targets_shortcut {
        return InputRuntimeKeyboardAction::Command(UiCommand::ToggleRecentAgentHelixTargets);
    }
    if request.active_app_dialog_open || request.active_reviewer_dialog_open {
        if request.workflow_quick_dialog_open
            && !request.wants_keyboard_input
            && workflow_quick_copy_shortcut_pressed(input)
        {
            return InputRuntimeKeyboardAction::Command(UiCommand::CopyWorkflowPath);
        }
        if request.workflow_quick_dialog_open
            && command_or_alt_shortcut_modifier(input.modifiers)
            && shortcut_key_pressed(input, egui::Key::Z)
        {
            return InputRuntimeKeyboardAction::Command(UiCommand::ToggleWorkflowQuickModal);
        }
        if request.agent_translation_dialog_open
            && let Some(command) = read_agent_translation_dialog_command(input)
        {
            return InputRuntimeKeyboardAction::Command(command);
        }
        if input.key_pressed(egui::Key::Escape) {
            return InputRuntimeKeyboardAction::Command(UiCommand::CloseTopLayer);
        }
        return InputRuntimeKeyboardAction::SuppressTerminalInput;
    }
    if request.extra_tools_open {
        if input.key_pressed(egui::Key::Escape) {
            return InputRuntimeKeyboardAction::Command(UiCommand::CloseTopLayer);
        }
        if let Some(command) =
            read_terminal_drawer_command(input, TerminalDrawerCommandScope::Workspace)
        {
            return InputRuntimeKeyboardAction::Command(command);
        }
        return InputRuntimeKeyboardAction::SuppressTerminalInput;
    }
    if request.outline_visible && recent_markdown_shortcut {
        return InputRuntimeKeyboardAction::Command(UiCommand::ToggleRecentMarkdownOutline);
    }
    if request.notifications_open {
        if text_edit_pre_global_shortcut_consumed(input, request.wants_keyboard_input) {
            return InputRuntimeKeyboardAction::SuppressTerminalInput;
        }
        if input.key_pressed(egui::Key::Escape) {
            return InputRuntimeKeyboardAction::Command(UiCommand::CloseTopLayer);
        }
        return read_notification_drawer_command(input)
            .map(InputRuntimeKeyboardAction::Command)
            .unwrap_or(InputRuntimeKeyboardAction::SuppressTerminalInput);
    }
    if request.reviewer_helix_open {
        return read_terminal_drawer_command(input, TerminalDrawerCommandScope::Helix)
            .map(InputRuntimeKeyboardAction::Command)
            .unwrap_or(InputRuntimeKeyboardAction::SuppressTerminalInput);
    }
    if request.workspace_terminal_open
        && let Some(command) =
            read_terminal_drawer_command(input, TerminalDrawerCommandScope::Workspace)
    {
        return InputRuntimeKeyboardAction::Command(command);
    }
    if request.workspace_terminal_open {
        // 触发条件：workspace terminal 抽屉打开时，egui 仍能看到快捷键。
        // 不能继续走全局快捷键：终端界面只归 app 处理 T/W。
        // 防止回归：M/N/X/数字切换等快捷键抢走终端输入。
        return InputRuntimeKeyboardAction::None;
    }
    if matches!(request.route, Route::Workspace)
        && matches!(request.center_mode, CenterMode::Editor)
        && text_edit_pre_global_shortcut_consumed(input, request.wants_keyboard_input)
    {
        return InputRuntimeKeyboardAction::SuppressTerminalInput;
    }
    if let Some(command) = read_base_route_command_for_input(
        input,
        request.route == Route::Reviewer,
        request.app_fullscreen,
    ) {
        return InputRuntimeKeyboardAction::Command(command);
    }
    match request.route {
        Route::Workspace => input_runtime_workspace_route_action(request),
        Route::Reviewer => input_runtime_reviewer_route_action(request),
    }
}

/// 解析 workspace route 内部输入。
fn input_runtime_workspace_route_action(
    request: &InputRuntimeRequest,
) -> InputRuntimeKeyboardAction {
    let input = &request.input;
    match request.center_mode {
        CenterMode::Agent | CenterMode::Terminal => {
            if let Some(direction) = read_agent_focus_move(input) {
                return InputRuntimeKeyboardAction::Command(UiCommand::MoveAgentFocus(direction));
            }
            if request.center_mode == CenterMode::Agent
                && extra_tools_shortcut_pressed(input)
                && !request.workspace_terminal_open
                && !request.reviewer_helix_open
                && !request.notifications_open
                && !request.active_app_dialog_open
                && !request.active_reviewer_dialog_open
            {
                return InputRuntimeKeyboardAction::Command(UiCommand::ToggleExtraTools);
            }
            if request.outline_tree_rect.is_some_and(|rect| {
                input
                    .pointer
                    .hover_pos()
                    .is_some_and(|pos| rect.contains(pos))
            }) || agent_tab_own_shortcut_pressed(input)
            {
                InputRuntimeKeyboardAction::SuppressTerminalInput
            } else {
                InputRuntimeKeyboardAction::None
            }
        }
        CenterMode::Editor | CenterMode::Preview => {
            if let Some(command) = read_workspace_route_command(
                input,
                request.keyboard_layer_can_close_with_escape,
                request.app_fullscreen,
            ) {
                return InputRuntimeKeyboardAction::Command(command);
            }
            if request.wants_keyboard_input {
                return InputRuntimeKeyboardAction::SuppressTerminalInput;
            }
            read_workspace_command(input, false)
                .map(InputRuntimeKeyboardAction::Command)
                .unwrap_or(InputRuntimeKeyboardAction::None)
        }
    }
}

/// Reads Ctrl+Arrow focus movement for the Agent grid.
///
/// Example: `Ctrl+Right` -> `Some(AgentFocusMove::Right)`.
fn read_agent_focus_move(input: &egui::InputState) -> Option<AgentFocusMove> {
    if !input.modifiers.ctrl {
        return None;
    }
    if input.key_pressed(egui::Key::ArrowLeft) {
        return Some(AgentFocusMove::Left);
    }
    if input.key_pressed(egui::Key::ArrowRight) {
        return Some(AgentFocusMove::Right);
    }
    if input.key_pressed(egui::Key::ArrowUp) {
        return Some(AgentFocusMove::Up);
    }
    if input.key_pressed(egui::Key::ArrowDown) {
        return Some(AgentFocusMove::Down);
    }
    None
}

/// 解析 reviewer route 内部输入。
fn input_runtime_reviewer_route_action(
    request: &InputRuntimeRequest,
) -> InputRuntimeKeyboardAction {
    let input = &request.input;
    if let Some(command) = read_reviewer_route_command(input, true, request.app_fullscreen) {
        return InputRuntimeKeyboardAction::Command(command);
    }
    let action = crate::gui::diff_viewer::read_diff_viewer_keyboard_action(
        input,
        request.selected_reviewer_diff_row,
        true,
        request.selected_reviewer_diff_row.is_some(),
    );
    if action != crate::gui::diff_viewer::DiffViewerAction::None {
        return InputRuntimeKeyboardAction::ReviewerDiff(action);
    }
    read_reviewer_command(input)
        .map(|command| InputRuntimeKeyboardAction::Command(UiCommand::Reviewer(command)))
        .unwrap_or(InputRuntimeKeyboardAction::None)
}

/// 解析 base route 快捷键，不读取 app 实例。
pub(super) fn read_base_route_command_for_input(
    input: &egui::InputState,
    in_reviewer_route: bool,
    app_fullscreen: bool,
) -> Option<UiCommand> {
    let command_or_alt = command_or_alt_shortcut_modifier(input.modifiers);
    let agent_slot_modifier = agent_slot_shortcut_modifier(input.modifiers);
    if active_workspace_shortcut_pressed(input) {
        return Some(UiCommand::SwitchActiveWorkspace);
    }
    if inactive_workspace_shortcut_pressed(input) {
        return Some(UiCommand::SwitchInactiveWorkspace);
    }
    if command_or_alt && input.key_pressed(egui::Key::T) {
        return Some(UiCommand::ToggleWorkspaceTerminal);
    }
    if notification_shortcut_pressed(input) {
        return Some(UiCommand::ToggleNotifications);
    }
    if command_or_alt && input.key_pressed(egui::Key::W) {
        return Some(UiCommand::AgentMarkdownShortcut);
    }
    if command_or_alt && shortcut_key_pressed(input, egui::Key::Z) {
        return Some(workflow_z_command(app_fullscreen));
    }
    if command_or_alt && input.key_pressed(egui::Key::M) {
        return Some(UiCommand::TranslateAgentInput);
    }
    if command_or_alt && input.key_pressed(egui::Key::N) {
        return Some(UiCommand::ApplyAgentInputTranslation);
    }
    if command_or_alt && input.key_pressed(egui::Key::D) {
        return Some(UiCommand::ToggleRecentAgentHelixTargets);
    }
    if agent_slot_modifier && shortcut_key_pressed(input, egui::Key::Num4) {
        return Some(UiCommand::PasteRecentMarkdownDiffsToAgent);
    }
    if agent_slot_modifier && shortcut_key_pressed(input, egui::Key::Num1) {
        return Some(UiCommand::SelectAgentSlot(0));
    }
    if agent_slot_modifier && shortcut_key_pressed(input, egui::Key::Num2) {
        return Some(UiCommand::SelectAgentSlot(1));
    }
    if agent_slot_modifier && shortcut_key_pressed(input, egui::Key::Num3) {
        return Some(UiCommand::SelectAgentSlot(2));
    }
    if reviewer_helix_shortcut_pressed(input) {
        return Some(UiCommand::ToggleReviewerHelix);
    }
    if in_reviewer_route && let Some(command) = reviewer_jump_shortcut_command(input) {
        return Some(UiCommand::Reviewer(command));
    }
    reviewer_shortcut_action(
        input.modifiers.command,
        input.modifiers.alt,
        input.key_pressed(egui::Key::R),
        input.key_pressed(egui::Key::Enter),
        in_reviewer_route,
    )
    .map(|action| match action {
        ReviewerShortcutAction::Open => UiCommand::OpenReviewerRoute,
        ReviewerShortcutAction::Exit => UiCommand::ExitReviewerRoute,
    })
}

/// Maps Cmd/Alt+Z to the route-specific workflow surface.
fn workflow_z_command(app_fullscreen: bool) -> UiCommand {
    if app_fullscreen {
        UiCommand::ToggleWorkflowQuickModal
    } else {
        UiCommand::ToggleOutlineWorkflowTab
    }
}

/// 解析 Agent 翻译弹窗打开时仍允许的快捷键。
fn read_agent_translation_dialog_command(input: &egui::InputState) -> Option<UiCommand> {
    let command_or_alt = command_or_alt_shortcut_modifier(input.modifiers);
    if command_or_alt && input.key_pressed(egui::Key::M) {
        return Some(UiCommand::TranslateAgentInput);
    }
    if command_or_alt && input.key_pressed(egui::Key::N) {
        return Some(UiCommand::ApplyAgentInputTranslation);
    }
    if command_or_alt && input.key_pressed(egui::Key::D) {
        return Some(UiCommand::ToggleRecentAgentHelixTargets);
    }
    None
}

/// Detects copy shortcuts owned by the fullscreen workflow quick modal.
fn workflow_quick_copy_shortcut_pressed(input: &egui::InputState) -> bool {
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
                && workflow_quick_copy_modifiers(*modifiers)
        }
        _ => false,
    }) || (input.key_pressed(egui::Key::C) && workflow_quick_copy_modifiers(input.modifiers))
}

/// Matches platform copy chords for quick workflow tree selection.
fn workflow_quick_copy_modifiers(modifiers: egui::Modifiers) -> bool {
    (modifiers.mac_cmd || modifiers.command) && !modifiers.alt && !modifiers.ctrl
        || modifiers.ctrl && !modifiers.alt && !modifiers.mac_cmd && !modifiers.command
}

pub(super) fn read_ui_command(
    input: &egui::InputState,
    in_reviewer_route: bool,
    has_closeable_keyboard_layer: bool,
    notification_drawer_open: bool,
    terminal_drawer_open: bool,
    helix_shortcut_allowed: bool,
    app_fullscreen: bool,
) -> Option<UiCommand> {
    let command = input.modifiers.command;
    let command_or_alt = command_or_alt_shortcut_modifier(input.modifiers);
    let agent_slot_modifier = agent_slot_shortcut_modifier(input.modifiers);

    if has_closeable_keyboard_layer && input.key_pressed(egui::Key::Escape) {
        return Some(UiCommand::CloseTopLayer);
    }
    if notification_drawer_open {
        // Trigger: the notification drawer is open, possibly with the
        // workspace memo editor focused.
        // The normal global shortcut path would also see the key chord and
        // can navigate routes while the user is typing in the drawer.
        // Keep only the drawer toggle here to prevent app/reviewer actions
        // from leaking through this overlay.
        return read_notification_drawer_command(input);
    }
    if terminal_drawer_open {
        // Trigger: a workspace terminal or Helix drawer is open.
        // The normal global shortcut path is too broad here because the
        // drawer embeds a child terminal while egui still sees key events.
        // Limit handling to drawer-owned shortcuts to prevent unrelated
        // reviewer/app actions and terminal text leaks from the same chord.
        return read_terminal_drawer_command(
            input,
            if helix_shortcut_allowed {
                TerminalDrawerCommandScope::Helix
            } else {
                TerminalDrawerCommandScope::Workspace
            },
        );
    }
    if help_shortcut_pressed(input) {
        return Some(UiCommand::OpenHelp);
    }
    if command && input.key_pressed(egui::Key::S) {
        return Some(UiCommand::SaveDocument);
    }
    if !in_reviewer_route && text_edit_copy_shortcut_pressed(input) {
        return Some(UiCommand::CopyWorkflowPath);
    }
    if command && input.modifiers.shift && input.key_pressed(egui::Key::P) {
        return Some(UiCommand::CaptureScreenshot);
    }
    if command_or_alt && input.key_pressed(egui::Key::T) {
        return Some(UiCommand::ToggleWorkspaceTerminal);
    }
    if notification_shortcut_pressed(input) {
        return Some(UiCommand::ToggleNotifications);
    }
    if command_or_alt && input.key_pressed(egui::Key::W) {
        return Some(UiCommand::AgentMarkdownShortcut);
    }
    if command_or_alt && shortcut_key_pressed(input, egui::Key::Z) {
        return Some(workflow_z_command(app_fullscreen));
    }
    if command_or_alt && input.key_pressed(egui::Key::M) {
        return Some(UiCommand::TranslateAgentInput);
    }
    if command_or_alt && input.key_pressed(egui::Key::N) {
        return Some(UiCommand::ApplyAgentInputTranslation);
    }
    if agent_slot_modifier && shortcut_key_pressed(input, egui::Key::Num4) {
        return Some(UiCommand::PasteRecentMarkdownDiffsToAgent);
    }
    if editor_preview_shortcut_pressed(input) {
        return Some(UiCommand::ToggleMarkdownEditorPreview);
    }
    if active_workspace_shortcut_pressed(input) {
        return Some(UiCommand::SwitchActiveWorkspace);
    }
    if inactive_workspace_shortcut_pressed(input) {
        return Some(UiCommand::SwitchInactiveWorkspace);
    }
    if agent_slot_modifier && shortcut_key_pressed(input, egui::Key::Num1) {
        return Some(UiCommand::SelectAgentSlot(0));
    }
    if agent_slot_modifier && shortcut_key_pressed(input, egui::Key::Num2) {
        return Some(UiCommand::SelectAgentSlot(1));
    }
    if agent_slot_modifier && shortcut_key_pressed(input, egui::Key::Num3) {
        return Some(UiCommand::SelectAgentSlot(2));
    }
    if helix_shortcut_allowed && reviewer_helix_shortcut_pressed(input) {
        return Some(UiCommand::ToggleReviewerHelix);
    }
    reviewer_shortcut_action(
        input.modifiers.command,
        input.modifiers.alt,
        input.key_pressed(egui::Key::R),
        input.key_pressed(egui::Key::Enter),
        in_reviewer_route,
    )
    .map(|action| match action {
        ReviewerShortcutAction::Open => UiCommand::OpenReviewerRoute,
        ReviewerShortcutAction::Exit => UiCommand::ExitReviewerRoute,
    })
}

/// Reads only shortcuts owned by terminal drawers.
fn read_terminal_drawer_command(
    input: &egui::InputState,
    scope: TerminalDrawerCommandScope,
) -> Option<UiCommand> {
    let command_or_alt = command_or_alt_shortcut_modifier(input.modifiers);
    if command_or_alt && input.key_pressed(egui::Key::T) {
        return Some(UiCommand::ToggleWorkspaceTerminal);
    }
    if command_or_alt && input.key_pressed(egui::Key::W) {
        return Some(UiCommand::AgentMarkdownShortcut);
    }
    if scope == TerminalDrawerCommandScope::Workspace
        && command_or_alt
        && input.key_pressed(egui::Key::D)
    {
        return Some(UiCommand::ToggleRecentAgentHelixTargets);
    }
    if scope == TerminalDrawerCommandScope::Helix && reviewer_helix_shortcut_pressed(input) {
        return Some(UiCommand::ToggleReviewerHelix);
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalDrawerCommandScope {
    Workspace,
    Helix,
}

/// Detects app-owned shortcuts that Agent tab must consume without dispatching.
fn read_workspace_route_command(
    input: &egui::InputState,
    has_closeable_keyboard_layer: bool,
    app_fullscreen: bool,
) -> Option<UiCommand> {
    read_ui_command(
        input,
        false,
        has_closeable_keyboard_layer,
        false,
        false,
        true,
        app_fullscreen,
    )
}

fn read_reviewer_route_command(
    input: &egui::InputState,
    has_closeable_keyboard_layer: bool,
    app_fullscreen: bool,
) -> Option<UiCommand> {
    read_ui_command(
        input,
        true,
        has_closeable_keyboard_layer,
        false,
        false,
        true,
        app_fullscreen,
    )
}

/// Detects gsdv-owned shortcuts that Agent tab must swallow locally.
pub(super) fn agent_tab_own_shortcut_pressed(input: &egui::InputState) -> bool {
    read_workspace_command(input, false).is_some()
}

/// Reads only shortcuts owned by the notification drawer.
fn read_notification_drawer_command(input: &egui::InputState) -> Option<UiCommand> {
    if notification_shortcut_pressed(input) {
        return Some(UiCommand::ToggleNotifications);
    }
    let command_or_alt = command_or_alt_shortcut_modifier(input.modifiers);
    if command_or_alt && input.key_pressed(egui::Key::W) {
        return Some(UiCommand::AgentMarkdownShortcut);
    }
    None
}

fn read_workspace_command(input: &egui::InputState, in_reviewer_route: bool) -> Option<UiCommand> {
    let command = input.modifiers.command;
    if command && input.key_pressed(egui::Key::O) {
        return Some(UiCommand::AddWorkspace);
    }
    if command && input.key_pressed(egui::Key::Comma) {
        return Some(UiCommand::OpenSettings);
    }
    if in_reviewer_route {
        return read_reviewer_command(input).map(UiCommand::Reviewer);
    }
    None
}

/// 检测 Agent 主界面的外置工具快捷键。
fn extra_tools_shortcut_pressed(input: &egui::InputState) -> bool {
    command_or_alt_shortcut_modifier(input.modifiers) && input.key_pressed(egui::Key::B)
}

pub(super) fn read_reviewer_command(input: &egui::InputState) -> Option<ReviewerCommand> {
    if let Some(command) = reviewer_jump_shortcut_command(input) {
        return Some(command);
    }
    if input.key_pressed(egui::Key::ArrowLeft)
        || (input.key_pressed(egui::Key::Tab) && input.modifiers.shift)
    {
        return Some(ReviewerCommand::PreviousColumn);
    }
    if input.key_pressed(egui::Key::ArrowRight) || input.key_pressed(egui::Key::Tab) {
        return Some(ReviewerCommand::NextColumn);
    }
    if input.key_pressed(egui::Key::ArrowUp) {
        return Some(ReviewerCommand::PreviousItem);
    }
    if input.key_pressed(egui::Key::ArrowDown) {
        return Some(ReviewerCommand::NextItem);
    }
    if input.key_pressed(egui::Key::R) {
        return Some(ReviewerCommand::Reload);
    }
    if input.key_pressed(egui::Key::B) {
        return Some(ReviewerCommand::OpenBranchDialog);
    }
    None
}

/// Reads Reviewer route jump shortcuts that do not require diff focus.
fn reviewer_jump_shortcut_command(input: &egui::InputState) -> Option<ReviewerCommand> {
    if input.key_pressed(egui::Key::N)
        && !input.modifiers.command
        && !input.modifiers.mac_cmd
        && !input.modifiers.ctrl
        && !input.modifiers.alt
    {
        if input.modifiers.shift {
            return Some(ReviewerCommand::JumpPreviousBlock);
        }
        return Some(ReviewerCommand::JumpNextBlock);
    }
    None
}

/// Checks shortcut keys using both logical and physical egui key data.
///
/// 触发条件：数字行快捷键叠加 Alt/Cmd 后，部分平台把 logical key
/// 报成符号，但 physical key 仍是数字键。
/// 不能只用 `InputState::key_pressed`：它只覆盖 logical key 路径。
/// 防止回归：Cmd/Alt+1 在不同键盘布局下都能切换 agent slot。
fn shortcut_key_pressed(input: &egui::InputState, key: egui::Key) -> bool {
    input.key_pressed(key)
        || input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: event_key,
                    physical_key,
                    pressed: true,
                    ..
                } if *event_key == key || *physical_key == Some(key)
            )
        })
}

/// Checks the Ctrl+1 shortcut for cycling non-Busy workspaces.
fn inactive_workspace_shortcut_pressed(input: &egui::InputState) -> bool {
    ctrl_workspace_shortcut_modifier(input.modifiers)
        && shortcut_key_pressed(input, egui::Key::Num1)
}

/// Checks the Ctrl+` shortcut for cycling Busy workspaces.
fn active_workspace_shortcut_pressed(input: &egui::InputState) -> bool {
    let physical_num1_symbol = command_or_alt_shortcut_modifier(input.modifiers)
        && physical_symbol_key_pressed(input, egui::Key::Num1);
    (ctrl_workspace_shortcut_modifier(input.modifiers)
        && (shortcut_key_pressed(input, egui::Key::Backtick)
            || input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Text(text) if text == "`"
                )
            })))
        || physical_num1_symbol
}

/// Checks Cmd/Alt shortcuts without accepting Ctrl-as-command fallback.
fn agent_slot_shortcut_modifier(modifiers: egui::Modifiers) -> bool {
    command_or_alt_shortcut_modifier(modifiers)
}

/// 检测物理数字键产生符号的快捷键，适配不同键盘布局。
fn physical_symbol_key_pressed(input: &egui::InputState, key: egui::Key) -> bool {
    input.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key {
                key: event_key,
                physical_key: Some(physical_key),
                pressed: true,
                ..
            } if *physical_key == key && *event_key != key
        )
    })
}

/// Checks Ctrl-only workspace switching shortcuts.
fn ctrl_workspace_shortcut_modifier(modifiers: egui::Modifiers) -> bool {
    (modifiers.ctrl || modifiers.command)
        && !modifiers.shift
        && !modifiers.alt
        && !modifiers.mac_cmd
}

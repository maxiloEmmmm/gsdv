use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

/// Verifies that platform paste events reach the PTY even while Cmd is held.
#[test]
fn command_paste_event_is_forwarded_to_terminal() {
    let events = vec![Event::Paste("hello".to_string())];
    let modifiers = Modifiers {
        command: true,
        ..Modifiers::default()
    };

    assert_eq!(
        agent_input_bytes_from_events(&events, modifiers, true),
        b"hello"
    );
}

/// Verifies that shortcut text from Cmd key chords is not leaked to the PTY.
#[test]
fn mac_command_text_event_is_suppressed_for_terminal_shortcuts() {
    let events = vec![Event::Text("v".to_string())];
    let modifiers = Modifiers {
        mac_cmd: true,
        command: true,
        ..Modifiers::default()
    };

    assert!(agent_input_bytes_from_events(&events, modifiers, true).is_empty());
}

/// Verifies that kitty protocol encodes Cmd+[ as terminal Super+[.
#[test]
fn kitty_command_open_bracket_is_forwarded_as_super_csi_u() {
    let modifiers = Modifiers {
        mac_cmd: true,
        command: true,
        ..Modifiers::default()
    };
    let events = vec![Event::Key {
        key: Key::OpenBracket,
        physical_key: Some(Key::OpenBracket),
        pressed: true,
        repeat: false,
        modifiers,
    }];

    assert_eq!(
        agent_input_bytes_from_events_with_kitty_protocol(&events, modifiers, true, true, None),
        b"\x1b[91;9u"
    );
}

/// Verifies that kitty protocol encodes Cmd+] as terminal Super+].
#[test]
fn kitty_command_close_bracket_is_forwarded_as_super_csi_u() {
    let modifiers = Modifiers {
        mac_cmd: true,
        command: true,
        ..Modifiers::default()
    };
    let events = vec![Event::Key {
        key: Key::CloseBracket,
        physical_key: Some(Key::CloseBracket),
        pressed: true,
        repeat: false,
        modifiers,
    }];

    assert_eq!(
        agent_input_bytes_from_events_with_kitty_protocol(&events, modifiers, true, true, None),
        b"\x1b[93;9u"
    );
}

/// Verifies that Cmd+[ stays suppressed before kitty protocol negotiation.
#[test]
fn command_open_bracket_without_kitty_protocol_is_suppressed() {
    let modifiers = Modifiers {
        mac_cmd: true,
        command: true,
        ..Modifiers::default()
    };
    let events = vec![Event::Key {
        key: Key::OpenBracket,
        physical_key: Some(Key::OpenBracket),
        pressed: true,
        repeat: false,
        modifiers,
    }];

    assert!(
        agent_input_bytes_from_events_with_kitty_protocol(&events, modifiers, true, false, None)
            .is_empty()
    );
}

/// kitty 协议下，普通 Enter 需要用 CSI-u 避免被当成 Ctrl+M。
#[test]
fn kitty_plain_enter_is_forwarded_as_csi_u() {
    let modifiers = Modifiers::default();
    let events = vec![Event::Key {
        key: Key::Enter,
        physical_key: Some(Key::Enter),
        pressed: true,
        repeat: false,
        modifiers,
    }];

    assert_eq!(
        agent_input_bytes_from_events_with_kitty_protocol(&events, modifiers, true, true, None),
        b"\x1b[13u"
    );
}

/// kitty 协议下，普通 Esc 需要继续送到 Codex 的中断快捷键。
#[test]
fn kitty_plain_escape_is_forwarded_as_csi_u() {
    let modifiers = Modifiers::default();
    let events = vec![Event::Key {
        key: Key::Escape,
        physical_key: Some(Key::Escape),
        pressed: true,
        repeat: false,
        modifiers,
    }];

    assert_eq!(
        agent_input_bytes_from_events_with_kitty_protocol(&events, modifiers, true, true, None),
        b"\x1b[27u"
    );
}

/// Verifies that embedded terminals accept kitty keyboard protocol requests.
#[test]
fn terminal_config_enables_kitty_keyboard_protocol() {
    assert!(terminal_config().kitty_keyboard);
}

/// Verifies that platform copy events become terminal Ctrl+C without selection.
#[test]
fn command_copy_event_is_forwarded_as_interrupt_without_selection() {
    let events = vec![Event::Copy];
    let modifiers = Modifiers {
        command: true,
        ..Modifiers::default()
    };

    assert_eq!(
        agent_input_bytes_from_events(&events, modifiers, true),
        b"\x03"
    );
}

/// Verifies that platform copy events preserve terminal selection copy.
#[test]
fn command_copy_event_is_not_forwarded_as_interrupt_with_selection() {
    let events = vec![Event::Copy];
    let modifiers = Modifiers {
        command: true,
        ..Modifiers::default()
    };

    assert!(agent_input_bytes_from_events(&events, modifiers, false).is_empty());
}

/// Verifies that Ctrl+C survives egui's command alias on Linux and Windows.
#[test]
fn control_c_copy_event_with_command_alias_is_forwarded_as_interrupt() {
    let events = vec![Event::Copy];
    let modifiers = Modifiers {
        ctrl: true,
        command: true,
        ..Modifiers::default()
    };

    assert_eq!(
        agent_input_bytes_from_events(&events, modifiers, true),
        b"\x03"
    );
}

/// Verifies that Ctrl+Shift+C keeps terminal copy semantics.
#[test]
fn control_shift_c_copy_event_is_not_forwarded_as_interrupt() {
    let events = vec![Event::Copy];
    let modifiers = Modifiers {
        ctrl: true,
        shift: true,
        command: true,
        ..Modifiers::default()
    };

    assert!(agent_input_bytes_from_events(&events, modifiers, true).is_empty());
}

/// Verifies that real Ctrl+C still reaches the terminal as an interrupt.
#[test]
fn control_c_key_event_is_forwarded_as_interrupt() {
    let modifiers = Modifiers {
        ctrl: true,
        ..Modifiers::default()
    };
    let events = vec![Event::Key {
        key: Key::C,
        physical_key: Some(Key::C),
        pressed: true,
        repeat: false,
        modifiers,
    }];

    assert_eq!(
        agent_input_bytes_from_events(&events, modifiers, true),
        b"\x03"
    );
}

/// Verifies that Ctrl key events survive egui's command alias.
#[test]
fn control_key_event_with_command_alias_is_forwarded_as_control_byte() {
    let modifiers = Modifiers {
        ctrl: true,
        command: true,
        ..Modifiers::default()
    };
    let events = vec![Event::Key {
        key: Key::A,
        physical_key: Some(Key::A),
        pressed: true,
        repeat: false,
        modifiers,
    }];

    assert_eq!(
        agent_input_bytes_from_events(&events, modifiers, true),
        b"\x01"
    );
}

/// Verifies that Ctrl+Shift+C is reserved for terminal copy.
#[test]
fn control_shift_c_key_event_is_not_forwarded_as_interrupt() {
    let modifiers = Modifiers {
        ctrl: true,
        shift: true,
        ..Modifiers::default()
    };
    let events = vec![Event::Key {
        key: Key::C,
        physical_key: Some(Key::C),
        pressed: true,
        repeat: false,
        modifiers,
    }];

    assert!(agent_input_bytes_from_events(&events, modifiers, true).is_empty());
}

/// Verifies that macOS Cmd+C key events clear Codex input without requiring Event::Copy modifiers.
#[test]
fn command_c_key_event_is_forwarded_as_interrupt() {
    let modifiers = Modifiers {
        mac_cmd: true,
        command: true,
        ..Modifiers::default()
    };
    let events = vec![Event::Key {
        key: Key::C,
        physical_key: Some(Key::C),
        pressed: true,
        repeat: false,
        modifiers,
    }];

    assert_eq!(
        agent_input_bytes_from_events(&events, Modifiers::default(), true),
        b"\x03"
    );
}

/// Verifies that Cmd+C still copies selected terminal text instead of interrupting.
#[test]
fn command_c_key_event_is_not_forwarded_as_interrupt_with_selection() {
    let modifiers = Modifiers {
        mac_cmd: true,
        command: true,
        ..Modifiers::default()
    };
    let events = vec![Event::Key {
        key: Key::C,
        physical_key: Some(Key::C),
        pressed: true,
        repeat: false,
        modifiers,
    }];

    assert!(agent_input_bytes_from_events(&events, modifiers, false).is_empty());
}

/// Verifies that Alt shortcuts use the terminal Meta encoding.
#[test]
fn alt_printable_key_event_is_forwarded_as_escape_prefixed_terminal_input() {
    let modifiers = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    let events = vec![Event::Key {
        key: Key::W,
        physical_key: Some(Key::W),
        pressed: true,
        repeat: false,
        modifiers,
    }];

    assert_eq!(
        agent_input_bytes_from_events(&events, modifiers, true),
        b"\x1bw"
    );
}

/// 验证 workspace terminal 抽屉不会把 Alt+X 当成 app 保留键。
#[test]
fn workspace_terminal_drawer_forwards_alt_x_to_terminal() {
    let modifiers = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    let events = vec![Event::Key {
        key: Key::X,
        physical_key: Some(Key::X),
        pressed: true,
        repeat: false,
        modifiers,
    }];

    assert_eq!(
        agent_input_bytes_from_events_with_kitty_protocol(
            &events,
            modifiers,
            true,
            false,
            Some(TerminalInputShortcutScope::WorkspaceDrawerSurface),
        ),
        b"\x1bx"
    );
}

/// Verifies that PgUp reaches Codex pager handling unchanged.
#[test]
fn page_up_key_event_is_forwarded_to_terminal() {
    let modifiers = Modifiers::default();
    let events = vec![Event::Key {
        key: Key::PageUp,
        physical_key: Some(Key::PageUp),
        pressed: true,
        repeat: false,
        modifiers,
    }];

    assert_eq!(
        agent_input_bytes_from_events(&events, modifiers, true),
        b"\x1b[5~"
    );
}

/// Verifies that PgDn reaches Codex pager handling unchanged.
#[test]
fn page_down_key_event_is_forwarded_to_terminal() {
    let modifiers = Modifiers::default();
    let events = vec![Event::Key {
        key: Key::PageDown,
        physical_key: Some(Key::PageDown),
        pressed: true,
        repeat: false,
        modifiers,
    }];

    assert_eq!(
        agent_input_bytes_from_events(&events, modifiers, true),
        b"\x1b[6~"
    );
}

/// Verifies Ctrl-click parsing ignores a leading bullet before a path.
#[test]
fn file_line_token_at_column_ignores_leading_bullet() {
    let click = file_line_token_at_column("  - repos/app/src/main.rs:53", 4).unwrap();

    assert_eq!(click.path, PathBuf::from("repos/app/src/main.rs"));
    assert_eq!(click.line, 53);
}

/// Verifies file-line parsing works when clicking inside the line number.
#[test]
fn file_line_token_at_column_accepts_line_number_clicks() {
    let click = file_line_token_at_column("repos/app/src/main.rs:53", 23).unwrap();

    assert_eq!(click.path, PathBuf::from("repos/app/src/main.rs"));
    assert_eq!(click.line, 53);
    assert_eq!(click.end_line, None);
}

/// 验证文件行 range 会定位 start 并保留 end。
#[test]
fn file_line_token_at_column_accepts_line_ranges() {
    let click = file_line_token_at_column("repos/app/src/main.rs:53-59", 26).unwrap();

    assert_eq!(click.path, PathBuf::from("repos/app/src/main.rs"));
    assert_eq!(click.line, 53);
    assert_eq!(click.end_line, Some(59));
}

/// Verifies ordinary non-path text does not produce a file click.
#[test]
fn file_line_token_at_column_ignores_non_paths() {
    assert!(file_line_token_at_column("no file here: today", 4).is_none());
}

/// 验证终端目录路径点击会走文件管理器定位。
#[test]
fn terminal_output_click_for_directory_reveals_path() {
    let root = std::env::temp_dir().join(format!(
        "gsdv-terminal-dir-click-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();

    let click =
        terminal_output_click_for_resolved_path(root.clone(), 1, None).map(|click| match click {
            TerminalOutputClick::PathCandidate(click) => classify_terminal_output_path_click(click),
            click => click,
        });

    assert_eq!(click, Some(TerminalOutputClick::RevealPath(root.clone())));
    let _ = std::fs::remove_dir_all(root);
}

/// 验证终端图片路径点击会走文件管理器定位。
#[test]
fn terminal_output_click_for_image_reveals_path() {
    let root = std::env::temp_dir().join(format!(
        "gsdv-terminal-image-click-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let image = root.join("preview.png");
    std::fs::write(&image, b"not-a-real-png").unwrap();

    let click =
        terminal_output_click_for_resolved_path(image.clone(), 1, None).map(|click| match click {
            TerminalOutputClick::PathCandidate(click) => classify_terminal_output_path_click(click),
            click => click,
        });

    assert_eq!(click, Some(TerminalOutputClick::RevealPath(image)));
    let _ = std::fs::remove_dir_all(root);
}

/// 验证 http URL 可以在终端输出里被点击识别。
#[test]
fn url_match_at_column_accepts_http_urls() {
    let matched = url_match_at_column("open http://localhost:3000/app", 10, 0).unwrap();

    assert_eq!(matched.url, "http://localhost:3000/app");
    assert_eq!(matched.start_column, 5);
}

/// 验证 https URL 会裁掉常见结尾标点。
#[test]
fn url_match_at_column_trims_trailing_punctuation() {
    let matched = url_match_at_column("see (https://example.com/a),", 9, 0).unwrap();

    assert_eq!(matched.url, "https://example.com/a");
}

/// Verifies that xterm cube color queries are answered.
#[test]
fn terminal_query_rgb_resolves_xterm_cube_color() {
    let colors = terminal_color_table::Colors::default();

    assert_eq!(
        terminal_query_rgb(16, &colors, gui_theme::ThemeMode::Light),
        Some(Rgb { r: 0, g: 0, b: 0 })
    );
}

/// Verifies that dynamic color table entries override theme defaults.
#[test]
fn terminal_query_rgb_prefers_dynamic_color_table() {
    let mut colors = terminal_color_table::Colors::default();
    colors[TERMINAL_BACKGROUND_COLOR_INDEX] = Some(Rgb { r: 1, g: 2, b: 3 });

    assert_eq!(
        terminal_query_rgb(
            TERMINAL_BACKGROUND_COLOR_INDEX,
            &colors,
            gui_theme::ThemeMode::Light
        ),
        Some(Rgb { r: 1, g: 2, b: 3 })
    );
}

/// Verifies that terminal size queries use the current grid metrics.
#[test]
fn terminal_size_converts_to_alacritty_window_size() {
    let size = TerminalSize {
        cell_width: 7.6,
        cell_height: 15.2,
        cols: 101,
        lines: 37,
        layout_size: Vec2::new(768.0, 562.0),
    };
    let window_size = WindowSize::from(size);

    assert_eq!(window_size.num_cols, 101);
    assert_eq!(window_size.num_lines, 37);
    assert_eq!(window_size.cell_width, 8);
    assert_eq!(window_size.cell_height, 15);
}

/// 验证终端 cell 尺寸会给 CJK fallback 留出空间。
#[test]
fn terminal_cell_measure_includes_cjk_fallback_metrics() {
    let size = terminal_cell_measure_from_metrics(
        TerminalFontMetrics {
            ascii_sample_width: 28.0,
            ascii_sample_chars: 4,
            space_width: 4.0,
            row_height: 14.0,
        },
        CjkTerminalFontMetrics {
            sample_width: 72.0,
            sample_chars: 4,
            sample_height: 19.0,
        },
    );

    assert_eq!(size, Vec2::new(7.0, 19.0));
}

/// 验证未处理 PTY 日志只记录事件类型。
#[test]
fn pty_event_kind_omits_title_payload() {
    assert_eq!(pty_event_kind(&PtyEvent::Title("one".to_string())), "Title");
    assert_eq!(pty_event_kind(&PtyEvent::Title("two".to_string())), "Title");
    assert_eq!(pty_event_kind(&PtyEvent::Bell), "Bell");
}

/// 验证 bell flash 会随剩余时间衰减。
#[test]
fn terminal_bell_flash_alpha_decays_with_remaining_duration() {
    assert_eq!(terminal_bell_flash_alpha(TERMINAL_BELL_FLASH_DURATION), 48);
    assert_eq!(terminal_bell_flash_alpha(Duration::ZERO), 0);
}

/// 验证恢复终端历史时换行会回到第 0 列。
#[test]
fn restored_terminal_history_uses_crlf_line_breaks() {
    assert_eq!(
        restored_terminal_history_bytes("first\nsecond\n"),
        "first\r\nsecond\r\n"
    );
}

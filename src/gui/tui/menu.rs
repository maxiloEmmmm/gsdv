//! TUI 右键菜单树和二级菜单渲染。

use super::*;

/// 构造直接动作菜单项。
///
/// 适用场景：重复菜单声明。示例：Stop -> Queue。
pub(super) fn menu_queue(label: &str, action: TuiAppAction) -> MenuItem {
    MenuItem {
        label: label.to_string(),
        action: Some(MenuAction::Queue(action)),
    }
}

/// 构造危险动作确认菜单项。
///
/// 适用场景：关闭和重启。示例：Close row -> Confirm。
pub(super) fn menu_confirm(label: &str, message: &str, action: TuiAppAction) -> MenuItem {
    MenuItem {
        label: label.to_string(),
        action: Some(MenuAction::Confirm {
            message: message.to_string(),
            action,
        }),
    }
}

/// 构造一个保持父菜单可见的二级菜单入口。
///
/// 适用场景：专注等级、Effort、Fast mode 等分类。示例：`Effort -> [low, high]`。
pub(super) fn menu_submenu(label: &str, items: Vec<MenuItem>) -> MenuItem {
    MenuItem {
        label: label.to_string(),
        action: Some(MenuAction::Submenu(items)),
    }
}

/// 构造菜单分隔线，键盘导航会自动跳过。
///
/// 适用场景：把进程、配置和布局动作分组。示例：两个动作组之间 -> 不可选横线。
pub(super) fn menu_separator() -> MenuItem {
    MenuItem {
        label: String::new(),
        action: None,
    }
}

/// 返回菜单包含边框、选择符和子菜单箭头后的显示宽度。
///
/// 适用场景：根菜单与二级菜单定位。示例：最长 10 列 -> 15 列菜单。
pub(super) fn menu_width_from_items(items: &[MenuItem]) -> u16 {
    u16::try_from(
        items
            .iter()
            .map(|item| display_width(&item.label))
            .max()
            .unwrap_or(8)
            .saturating_add(5)
            .min(42),
    )
    .unwrap_or(42)
}

/// 递归绘制菜单链，父菜单保持在二级菜单后方。
///
/// 适用场景：二级右键菜单。示例：根菜单 + Spotlight registration 子菜单。
pub(super) fn draw_context_menu_chain(
    frame: &mut [RenderLine],
    menu: &mut ContextMenu,
    size: (u16, u16),
    active: bool,
) {
    if let Some(parent) = menu.parent.as_mut() {
        draw_context_menu_chain(frame, parent, size, false);
    }
    draw_context_menu(frame, menu, size, active);
}

/// 绘制单层彩色菜单，并在当前终端边界内调整位置。
///
/// 适用场景：根菜单或二级菜单。示例：选中项使用蓝底，子菜单项显示 `›`。
pub(super) fn draw_context_menu(
    frame: &mut [RenderLine],
    menu: &mut ContextMenu,
    size: (u16, u16),
    active: bool,
) {
    let range = menu_visible_range(menu, size.1);
    let width = menu_width_from_items(&menu.items);
    menu.x = menu.x.min(size.0.saturating_sub(width));
    menu.y = menu.y.min(size.1.saturating_sub(range.len() as u16 + 2));
    styled_overlay_text(
        frame,
        menu.y,
        menu.x,
        &format!("┌{}┐", "─".repeat(width.saturating_sub(2) as usize)),
        (100, 116, 139),
        (15, 23, 42),
        false,
    );
    let visible_count = range.len();
    for (visible_index, item_index) in range.enumerate() {
        let item = &menu.items[item_index];
        let selected = item_index == menu.selected;
        let y = menu.y + 1 + visible_index as u16;
        if item.action.is_none() {
            styled_overlay_text(
                frame,
                y,
                menu.x,
                &format!("│ {} │", "─".repeat(width.saturating_sub(4) as usize)),
                (71, 85, 105),
                (15, 23, 42),
                false,
            );
            continue;
        }
        let marker = if selected { "›" } else { " " };
        let suffix = if matches!(item.action.as_ref(), Some(MenuAction::Submenu(_))) {
            "›"
        } else {
            " "
        };
        let content = clip_text(&item.label, width.saturating_sub(5));
        let padding = width
            .saturating_sub(5)
            .saturating_sub(display_width(&content) as u16);
        let (fg, bg, bold) = if selected && active {
            ((255, 255, 255), (37, 99, 235), true)
        } else if selected {
            ((226, 232, 240), (51, 65, 85), true)
        } else {
            ((203, 213, 225), (15, 23, 42), false)
        };
        styled_overlay_text(
            frame,
            y,
            menu.x,
            &format!(
                "│{marker} {content}{}{suffix}│",
                " ".repeat(padding as usize)
            ),
            fg,
            bg,
            bold,
        );
    }
    styled_overlay_text(
        frame,
        menu.y + 1 + visible_count as u16,
        menu.x,
        &format!("└{}┘", "─".repeat(width.saturating_sub(2) as usize)),
        (100, 116, 139),
        (15, 23, 42),
        false,
    );
}

/// 返回菜单当前可见范围。
///
/// 适用场景：超高 Agent 菜单。示例：24 行终端 -> 至多 20 项。
pub(super) fn menu_visible_range(menu: &ContextMenu, height: u16) -> std::ops::Range<usize> {
    let count = usize::from(height.saturating_sub(4).max(1));
    menu.scroll..(menu.scroll + count).min(menu.items.len())
}

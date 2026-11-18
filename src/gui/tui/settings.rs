//! TUI 全局设置表单。

use super::*;

/// 构造与 GUI 共享同一份持久化数据的网络代理表单。
///
/// 适用场景：底栏 Settings。示例：已启用 SOCKS 代理 -> 表单回显 On 与原 URL。
pub(super) fn network_settings_form(app: &GsdvGuiApp) -> FormDialog {
    FormDialog {
        title: "Network Proxy".to_string(),
        fields: vec![
            FormField::Choice {
                label: "Enable proxy".to_string(),
                options: vec!["Off".to_string(), "On".to_string()],
                selected: usize::from(app.network_settings.proxy_enabled),
            },
            FormField::Text {
                label: "Proxy URL".to_string(),
                value: app.network_settings.proxy.clone(),
            },
            FormField::Choice {
                label: "Mirror protocol".to_string(),
                options: vec!["Off".to_string(), "On".to_string()],
                selected: usize::from(app.network_settings.mirror_proxy_protocol),
            },
            FormField::Text {
                label: "Additional no_proxy".to_string(),
                value: app.network_settings.no_proxy.clone(),
            },
        ],
        active: 0,
        submit: FormSubmit::NetworkSettings,
        error: None,
    }
}

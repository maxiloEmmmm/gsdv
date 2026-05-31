use std::ffi::OsString;
use std::path::PathBuf;

/// 返回当前用户 home 目录，适用于跨平台用户级配置路径。
pub(crate) fn home_dir() -> Option<PathBuf> {
    home_dir_from_env_values(
        std::env::var_os("HOME"),
        std::env::var_os("USERPROFILE"),
        std::env::var_os("HOMEDRIVE"),
        std::env::var_os("HOMEPATH"),
    )
}

/// 从环境变量解析 home 目录，适用于测试 Windows 缺少 HOME 的场景。
pub(crate) fn home_dir_from_env_values(
    home: Option<OsString>,
    user_profile: Option<OsString>,
    home_drive: Option<OsString>,
    home_path: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(home) = non_empty_env_path(home) {
        return Some(PathBuf::from(home));
    }
    if let Some(user_profile) = non_empty_env_path(user_profile) {
        return Some(PathBuf::from(user_profile));
    }
    let drive = non_empty_env_path(home_drive)?;
    let path = non_empty_env_path(home_path)?;
    Some(PathBuf::from(format!(
        "{}{}",
        drive.to_string_lossy(),
        path.to_string_lossy()
    )))
}

/// 过滤空环境变量，避免把空值误当成当前目录。
fn non_empty_env_path(value: Option<OsString>) -> Option<OsString> {
    value.filter(|value| !value.as_os_str().is_empty())
}

#[cfg(test)]
#[path = "home_test.rs"]
mod home_test;

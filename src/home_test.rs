use super::*;

/// 验证 HOME 优先于其它 home 环境变量。
#[test]
fn home_dir_prefers_home() {
    let home = home_dir_from_env_values(
        Some("/home/test".into()),
        Some("C:\\Users\\test".into()),
        Some("D:".into()),
        Some("\\Users\\other".into()),
    );

    assert_eq!(home, Some(PathBuf::from("/home/test")));
}

/// 验证 Windows USERPROFILE 可在 HOME 缺失时作为 home。
#[test]
fn home_dir_uses_userprofile_without_home() {
    let home = home_dir_from_env_values(
        None,
        Some("C:\\Users\\test".into()),
        Some("D:".into()),
        Some("\\Users\\other".into()),
    );

    assert_eq!(home, Some(PathBuf::from("C:\\Users\\test")));
}

/// 验证 Windows HOMEDRIVE 和 HOMEPATH 可作为最后兜底。
#[test]
fn home_dir_uses_home_drive_and_path() {
    let home = home_dir_from_env_values(
        Some("".into()),
        Some("".into()),
        Some("C:".into()),
        Some("\\Users\\test".into()),
    );

    assert_eq!(home, Some(PathBuf::from("C:\\Users\\test")));
}

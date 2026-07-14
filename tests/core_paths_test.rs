//! 核心路径转义、展开与安全校验测试。

use petal_link_lib::core::cache_paths::{
    cloud_tree_cache_file, escape_mount_path, sync_state_cache_file,
};
use petal_link_lib::core::paths::{expand_tilde, validate_path_segment, validate_relative_path};

/// 验证挂载路径转义为安全文件名。
#[test]
fn test_escape_mount_path() {
    assert_eq!(
        escape_mount_path("/Users/me/hwcloud-drive"),
        "_Users_me_hwcloud-drive"
    );
}

/// 验证安全字符在转义时保持不变。
#[test]
fn test_escape_keeps_safe_chars() {
    assert_eq!(
        escape_mount_path("/Users/a.b-c_d/data"),
        "_Users_a.b-c_d_data"
    );
}

/// 验证缓存文件名包含转义后的挂载路径。
#[test]
fn test_cache_file_naming() {
    let f = sync_state_cache_file("/Users/me/drive").unwrap();
    assert!(f
        .to_string_lossy()
        .ends_with("syncstate__Users_me_drive.json"));
    let f = cloud_tree_cache_file("/Users/me/drive").unwrap();
    assert!(f
        .to_string_lossy()
        .ends_with("cloudtree__Users_me_drive.json"));
}

/// 验证波浪号路径使用 HOME 展开。
#[test]
fn test_expand_tilde_with_home() {
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() {
        assert_eq!(expand_tilde("~/drive"), format!("{home}/drive"));
        assert_eq!(expand_tilde("~/"), format!("{home}/"));
    }
}

/// 验证不含波浪号的路径原样返回。
#[test]
fn test_expand_tilde_no_tilde() {
    assert_eq!(expand_tilde("/absolute/path"), "/absolute/path");
    assert_eq!(expand_tilde("relative/path"), "relative/path");
    assert_eq!(expand_tilde(""), "");
}

/// 验证越界或绝对相对路径被拒绝。
#[test]
fn test_validate_relative_path_rejects_escape() {
    for bad in [
        "/tmp/file",
        "../file",
        "a/../file",
        "./file",
        "a//b",
        "a\\b",
        "",
    ] {
        assert!(
            validate_relative_path(bad, false).is_err(),
            "应拒绝不安全相对路径：{bad}"
        );
    }
}

/// 验证正常嵌套相对路径通过校验。
#[test]
fn test_validate_relative_path_accepts_normal_nested_path() {
    assert!(validate_relative_path("docs/report.txt", false).is_ok());
    assert!(validate_relative_path("", true).is_ok());
}

/// 验证分隔符及点路径片段被拒绝。
#[test]
fn test_validate_path_segment_rejects_separator_or_dot() {
    for bad in ["", ".", "..", "a/b", "a\\b"] {
        assert!(
            validate_path_segment(bad).is_err(),
            "应拒绝不安全路径片段：{bad}"
        );
    }
}

//! Drive 下载临时路径构造测试。

use std::path::PathBuf;

use petal_link_lib::drive::download_api::tmp_path;

/// 验证下载临时后缀追加到完整目标路径。
#[test]
fn test_tmp_path_appends_suffix() {
    let dest = PathBuf::from("/tmp/report.txt");
    let tmp = tmp_path(&dest);
    assert_eq!(tmp, PathBuf::from("/tmp/report.txt.tmp"));
}

/// 验证多扩展名路径不会丢失原扩展名。
#[test]
fn test_tmp_path_multiple_extensions() {
    let dest = PathBuf::from("/tmp/archive.tar.gz");
    let tmp = tmp_path(&dest);
    assert_eq!(tmp, PathBuf::from("/tmp/archive.tar.gz.tmp"));
}

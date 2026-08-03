//! 拖拽导入复制的核心合同测试：递归复制、原子落盘、拒绝覆盖、跳过规则与自我复制防护。

use std::fs;
use std::path::Path;

use petal_link_lib::mount::import_copy::copy_into_mount;
use petal_link_lib::mount::skip::SkipMatcher;
use tempfile::tempdir;

/// 收集目录下全部相对路径（`/`-分隔），便于断言复制产物的完整结构。
fn collect_relative_paths(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push(rel);
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    out.sort();
    out
}

/// 目录递归复制必须保持结构与内容，且完成后不残留 `.tmp` 临时文件。
#[test]
fn copies_directory_recursively_without_tmp_residue() {
    let source_root = tempdir().unwrap();
    let mount_root = tempdir().unwrap();
    fs::create_dir_all(source_root.path().join("docs/inner")).unwrap();
    fs::write(source_root.path().join("docs/a.txt"), "alpha").unwrap();
    fs::write(source_root.path().join("docs/inner/b.txt"), "beta").unwrap();

    let skip = SkipMatcher::new(&[]);
    let stats =
        copy_into_mount(&source_root.path().join("docs"), mount_root.path(), &skip).unwrap();

    assert_eq!(stats.copied_files, 2);
    assert_eq!(stats.copied_dirs, 2);
    assert_eq!(stats.skipped, 0);
    assert!(stats.conflicts.is_empty());
    assert_eq!(
        fs::read_to_string(mount_root.path().join("docs/a.txt")).unwrap(),
        "alpha"
    );
    assert_eq!(
        fs::read_to_string(mount_root.path().join("docs/inner/b.txt")).unwrap(),
        "beta"
    );
    // 复制完成后任何层级都不得残留 .tmp 临时文件。
    assert!(!collect_relative_paths(mount_root.path())
        .iter()
        .any(|rel| rel.ends_with(".tmp")));
}

/// 目标已存在的同名文件必须拒绝覆盖并计入冲突，既有内容保持不动。
#[test]
fn existing_target_is_reported_as_conflict_without_overwrite() {
    let source_root = tempdir().unwrap();
    let mount_root = tempdir().unwrap();
    fs::write(source_root.path().join("a.txt"), "new-content").unwrap();
    fs::write(mount_root.path().join("a.txt"), "cloud-mirror").unwrap();

    let skip = SkipMatcher::new(&[]);
    let stats =
        copy_into_mount(&source_root.path().join("a.txt"), mount_root.path(), &skip).unwrap();

    assert_eq!(stats.copied_files, 0);
    assert_eq!(stats.conflicts, vec!["a.txt".to_string()]);
    assert_eq!(
        fs::read_to_string(mount_root.path().join("a.txt")).unwrap(),
        "cloud-mirror"
    );
}

/// 命中内建规则与用户 skipPatterns 的条目必须整体跳过，不参与落盘。
#[test]
fn skipped_entries_are_not_copied() {
    let source_root = tempdir().unwrap();
    let mount_root = tempdir().unwrap();
    let docs = source_root.path().join("docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(docs.join("keep.txt"), "keep").unwrap();
    fs::write(docs.join(".hwcloud_cache.json"), "cache").unwrap();
    fs::write(docs.join("draft.tmp"), "half").unwrap();
    fs::write(docs.join(".DS_Store"), "ds").unwrap();

    let skip = SkipMatcher::new(&[".DS_Store".to_string()]);
    let stats = copy_into_mount(&docs, mount_root.path(), &skip).unwrap();

    assert_eq!(stats.copied_files, 1);
    assert_eq!(stats.skipped, 3);
    assert_eq!(
        collect_relative_paths(mount_root.path()),
        vec!["docs".to_string(), "docs/keep.txt".to_string()]
    );
}

/// 源是目标目录的祖先时（复制进自身）必须整体拒绝，防止无限递归或自截断。
#[test]
fn copying_into_own_descendant_is_rejected() {
    let source_root = tempdir().unwrap();
    let nested = source_root.path().join("outer/inner");
    fs::create_dir_all(&nested).unwrap();

    let skip = SkipMatcher::new(&[]);
    let result = copy_into_mount(&source_root.path().join("outer"), &nested, &skip);

    assert!(result.is_err());
    // 拒绝前不得创建任何目标目录。
    assert_eq!(fs::read_dir(&nested).unwrap().count(), 0);
}

/// 符号链接源必须拒绝；目录内的符号链接条目按跳过处理，不把挂载外目标链入镜像。
#[test]
fn symlinks_are_rejected_or_skipped() {
    let source_root = tempdir().unwrap();
    let mount_root = tempdir().unwrap();
    let docs = source_root.path().join("docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(docs.join("real.txt"), "real").unwrap();
    std::os::unix::fs::symlink(docs.join("real.txt"), docs.join("link.txt")).unwrap();
    std::os::unix::fs::symlink(&docs, source_root.path().join("docs-link")).unwrap();

    let skip = SkipMatcher::new(&[]);
    // 顶层符号链接源：整体拒绝。
    assert!(copy_into_mount(
        &source_root.path().join("docs-link"),
        mount_root.path(),
        &skip
    )
    .is_err());
    // 目录内符号链接条目：跳过，真实文件正常复制。
    let stats = copy_into_mount(&docs, mount_root.path(), &skip).unwrap();
    assert_eq!(stats.copied_files, 1);
    assert_eq!(stats.skipped, 1);
    assert_eq!(
        collect_relative_paths(mount_root.path()),
        vec!["docs".to_string(), "docs/real.txt".to_string()]
    );
}

/// 命中跳过规则的顶层导入源必须报错而不是静默成功，让调用方能归入失败列表。
#[test]
fn skipped_root_source_is_an_error() {
    let source_root = tempdir().unwrap();
    let mount_root = tempdir().unwrap();
    fs::write(source_root.path().join("draft.tmp"), "half").unwrap();

    let skip = SkipMatcher::new(&[]);
    let result = copy_into_mount(
        &source_root.path().join("draft.tmp"),
        mount_root.path(),
        &skip,
    );

    assert!(result.is_err());
    assert!(fs::read_dir(mount_root.path()).unwrap().next().is_none());
}

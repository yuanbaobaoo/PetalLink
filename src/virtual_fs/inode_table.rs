//! 会话内稳定 inode 分配。
//!
//! backing 文件会在下载完成时被原子替换，所以不能把底层 `st_ino` 暴露给 FUSE。
//! 本表以相对路径为身份，在一次挂载会话内只增不减、从不复用 inode。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use fuser::INodeNo;

#[derive(Debug)]
struct Inner {
    next_inode: u64,
    by_inode: HashMap<INodeNo, PathBuf>,
    by_path: HashMap<PathBuf, INodeNo>,
}

/// 路径和 FUSE inode 的双向表。
#[derive(Debug)]
pub(super) struct InodeTable {
    inner: RwLock<Inner>,
}

impl InodeTable {
    /// 创建只含根 inode 的新会话表。
    pub(super) fn new() -> Self {
        let root = PathBuf::new();
        let mut by_inode = HashMap::new();
        by_inode.insert(INodeNo::ROOT, root.clone());
        let mut by_path = HashMap::new();
        by_path.insert(root, INodeNo::ROOT);
        Self {
            inner: RwLock::new(Inner {
                next_inode: INodeNo::ROOT.0 + 1,
                by_inode,
                by_path,
            }),
        }
    }

    /// 返回路径已有 inode；首次出现时分配一个本会话内永久稳定的新 inode。
    pub(super) fn get_or_insert(&self, relative_path: &Path) -> INodeNo {
        if let Some(inode) = self
            .inner
            .read()
            .expect("inode 表读锁中毒")
            .by_path
            .get(relative_path)
            .copied()
        {
            return inode;
        }

        let mut inner = self.inner.write().expect("inode 表写锁中毒");
        if let Some(inode) = inner.by_path.get(relative_path).copied() {
            return inode;
        }

        let inode = INodeNo(inner.next_inode);
        inner.next_inode = inner
            .next_inode
            .checked_add(1)
            .expect("FUSE inode 空间耗尽");
        let relative_path = relative_path.to_path_buf();
        inner.by_path.insert(relative_path.clone(), inode);
        inner.by_inode.insert(inode, relative_path);
        inode
    }

    /// 根据 inode 返回相对 backing 根目录的路径。
    pub(super) fn path(&self, inode: INodeNo) -> Option<PathBuf> {
        self.inner
            .read()
            .expect("inode 表读锁中毒")
            .by_inode
            .get(&inode)
            .cloned()
    }

    /// 删除某个路径及其全部后代的路径映射。
    ///
    /// inode 数字永不复用；已经打开的底层文件句柄仍可继续使用，但后续对原路径的
    /// lookup 会获得一个全新的 inode，符合 unlink 后重建同名文件的语义。
    pub(super) fn remove_subtree(&self, relative_path: &Path) {
        if relative_path.as_os_str().is_empty() {
            return;
        }

        let mut inner = self.inner.write().expect("inode 表写锁中毒");
        let removed: Vec<(PathBuf, INodeNo)> = inner
            .by_path
            .iter()
            .filter(|(path, _)| path.as_path() == relative_path || path.starts_with(relative_path))
            .map(|(path, inode)| (path.clone(), *inode))
            .collect();
        for (path, inode) in removed {
            inner.by_path.remove(&path);
            inner.by_inode.remove(&inode);
        }
    }

    /// 把源路径及其全部已知后代原子重映射到新路径。
    ///
    /// 若目标原来已有映射（rename 覆盖），先使目标映射失效；源对象及后代保留原
    /// inode，因此打开目录或已被 lookup 的文件在移动后仍保持身份稳定。
    pub(super) fn rename_subtree(&self, source: &Path, destination: &Path) {
        if source == destination
            || source.as_os_str().is_empty()
            || destination.as_os_str().is_empty()
        {
            return;
        }

        let mut inner = self.inner.write().expect("inode 表写锁中毒");
        let source_entries: Vec<(PathBuf, INodeNo)> = inner
            .by_path
            .iter()
            .filter(|(path, _)| path.as_path() == source || path.starts_with(source))
            .map(|(path, inode)| (path.clone(), *inode))
            .collect();
        let destination_entries: Vec<(PathBuf, INodeNo)> = inner
            .by_path
            .iter()
            .filter(|(path, _)| path.as_path() == destination || path.starts_with(destination))
            .map(|(path, inode)| (path.clone(), *inode))
            .collect();

        for (path, inode) in destination_entries {
            inner.by_path.remove(&path);
            inner.by_inode.remove(&inode);
        }
        for (path, _) in &source_entries {
            inner.by_path.remove(path);
        }
        for (old_path, inode) in source_entries {
            let suffix = old_path
                .strip_prefix(source)
                .expect("已筛选的 inode 路径必须有源前缀");
            let new_path = if suffix.as_os_str().is_empty() {
                destination.to_path_buf()
            } else {
                destination.join(suffix)
            };
            inner.by_path.insert(new_path.clone(), inode);
            inner.by_inode.insert(inode, new_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::InodeTable;
    use fuser::INodeNo;
    use std::path::Path;

    #[test]
    fn root_is_fuse_root_inode() {
        let table = InodeTable::new();
        assert_eq!(table.get_or_insert(Path::new("")), INodeNo::ROOT);
        assert_eq!(table.path(INodeNo::ROOT).as_deref(), Some(Path::new("")));
    }

    #[test]
    fn relative_path_keeps_same_inode_for_session() {
        let table = InodeTable::new();
        let first = table.get_or_insert(Path::new("docs/report.pdf"));
        let other = table.get_or_insert(Path::new("docs/other.pdf"));
        let second = table.get_or_insert(Path::new("docs/report.pdf"));

        assert_eq!(first, second);
        assert_ne!(first, other);
        assert_eq!(
            table.path(first).as_deref(),
            Some(Path::new("docs/report.pdf"))
        );
    }

    #[test]
    fn rename_preserves_source_subtree_and_invalidates_replaced_destination() {
        let table = InodeTable::new();
        let source = table.get_or_insert(Path::new("draft"));
        let child = table.get_or_insert(Path::new("draft/chapter.txt"));
        let replaced = table.get_or_insert(Path::new("published"));

        table.rename_subtree(Path::new("draft"), Path::new("published"));

        assert_eq!(table.path(source).as_deref(), Some(Path::new("published")));
        assert_eq!(
            table.path(child).as_deref(),
            Some(Path::new("published/chapter.txt"))
        );
        assert_eq!(table.path(replaced), None);
        assert_eq!(
            table.get_or_insert(Path::new("published/chapter.txt")),
            child
        );
    }

    #[test]
    fn unlink_invalidates_path_without_reusing_inode() {
        let table = InodeTable::new();
        let removed = table.get_or_insert(Path::new("notes.txt"));
        table.remove_subtree(Path::new("notes.txt"));
        let recreated = table.get_or_insert(Path::new("notes.txt"));

        assert_ne!(removed, recreated);
        assert_eq!(table.path(removed), None);
        assert_eq!(
            table.path(recreated).as_deref(),
            Some(Path::new("notes.txt"))
        );
    }
}

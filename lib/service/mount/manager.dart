/// 本地镜像目录管理器 —— 占位符 + 本地扫描 + Finder 灰标。
///
/// 严格对齐 Rust 原版 `src/mount/manager.rs` 与 `src/commands/sync_status.rs`。
///
/// # 占位符策略（v2, Files-On-Demand-lite）
/// - 占位文件使用**真实文件名**（无后缀），0 字节。
/// - 状态通过 xattr state 键追踪（inode 方案，docs/design/10 §2.1：
///   fileId/size xattr 已废弃，文件身份由 local_inode_map 承担）。
/// - Finder 灰标（label index 7）= 未下载；无标签 = 已下载。
/// - xattr 是数据源头（source of truth），Finder label 仅视觉反馈。
/// - 0 字节且非占位 → 拒绝删除（保护用户空文件如 .gitkeep）
library;

import 'dart:async';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'package:intl/intl.dart';
import 'package:path/path.dart' as p;
import 'package:sqflite/sqflite.dart';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/entity/sync_item.dart';
import 'package:petal_link/service/mount/mount_path.dart';
import 'package:petal_link/service/mount/skip.dart';
import 'package:petal_link/service/mount/xattr_service.dart';
import 'package:petal_link/service/sync/identity/inode_identity.dart';
import 'package:petal_link/types/enums.dart';

/// xattr 键：云端文件 ID
/// 旧版 fileId xattr 键（docs/design/10 §6.5：不再写入，仅在迁移兼容
/// 路径读取——owner 核验回退与旧 free-up 现场恢复；后续版本移除）
const String xattrFileId = 'com.hwcloud.fileId';

/// xattr 键：占位状态（placeholder / downloaded）
const String xattrState = 'com.hwcloud.state';

/// xattr 键：文件大小

/// 释放空间事务暂存文件携带的原始相对路径，用于进程退出后的恢复。
const String xattrFreeUpRelativePath = 'com.hwcloud.freeUpRelativePath';

/// xattr 值：占位符
const String statePlaceholder = 'placeholder';

/// xattr 值：文件内容已完整落地
const String stateDownloaded = 'downloaded';

/// Finder 灰标 xattr 键
const String finderInfoXattr = 'com.apple.FinderInfo';

/// FinderInfo byte[9] 的灰标值（label index 7 = 灰；实测 byte[9]=0x02，
/// 与 osascript `set label index to 7` 结果一致，kMDItemFSLabel=1）。
const int grayLabelByte = 0x02;

/// 释放空间暂存文件名前缀（对齐 Rust `.hwcloud_freeup-`）
const String freeUpStagingPrefix = '.hwcloud_freeup-';

/// 生成 16 位随机十六进制串（对齐 Rust `{:016x}` 的 u64 随机数格式）。
String randomHex64(Random random) {
  final bytes = List<int>.generate(8, (_) => random.nextInt(256));
  return bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();
}

/// 本地文件条目（scanLocal 返回）
class LocalFileEntry {
  /// 绝对路径
  final String absolutePath;

  /// 相对挂载目录的路径
  final String relativePath;

  /// 文件大小（字节）
  final int size;

  /// 修改时间（毫秒 epoch）
  final int mtime;

  /// 是否文件夹
  final bool isFolder;

  /// 是否占位符（0 字节且 xattr state=placeholder）
  final bool isPlaceholder;

  /// 文件系统 inode（inode 身份方案，docs/design/10；
  /// 由 [MountManager.inodeBatchProvider] 批量填充，未注入时为 null）
  final int? inode;

  const LocalFileEntry({
    required this.absolutePath,
    required this.relativePath,
    required this.size,
    required this.mtime,
    required this.isFolder,
    required this.isPlaceholder,
    this.inode,
  });

  /// 附带 inode 的副本（扫描后批量填充用）
  LocalFileEntry withInode(int inode) => LocalFileEntry(
        absolutePath: absolutePath,
        relativePath: relativePath,
        size: size,
        mtime: mtime,
        isFolder: isFolder,
        isPlaceholder: isPlaceholder,
        inode: inode,
      );

  @override
  String toString() =>
      'LocalFileEntry($relativePath, size=$size, folder=$isFolder, placeholder=$isPlaceholder)';
}

/// 本地镜像目录管理器。
///
/// [xattr] 可注入 fake（测试）；默认走 [ChannelXattrService] 原生通道。
/// [db] 仅 [checkFileLocalStatus] / [batchFileLocalStatus] 需要。
class MountManager {
  /// 挂载根目录（绝对路径）
  final String mountDir;

  /// 同步基线数据库（仅 [checkFileLocalStatus] / [batchFileLocalStatus] 需要）
  final DatabaseService? db;

  final XattrService _xattr;

  /// 批量 inode 查询（生产：原生 lstat 批量通道；测试注入 fake）。
  ///
  /// 为 null 时扫描不填充 inode（行为与注入前一致）。
  final Future<Map<String, int>> Function(List<String> paths)?
      inodeBatchProvider;

  /// inode 身份映射（docs/design/10；引擎装配时注入共享实例）
  InodeIdentityStore identity = SqfliteInodeIdentityStore(DatabaseService.instance);

  MountManager(
    this.mountDir, {
    XattrService? xattr,
    this.db,
    this.inodeBatchProvider,
    InodeIdentityStore? identity,
  })  : _xattr = xattr ?? ChannelXattrService(),
        identity = identity ??
            SqfliteInodeIdentityStore(DatabaseService.instance);

  /// 经批量通道取本地文件 inode 的身份（无 provider/无记录 → null）。
  Future<String?> inodeOwnerOf(String localPath) async {
    final provider = inodeBatchProvider;
    if (provider == null) return null;
    final inodes = await provider([localPath]);
    final inode = inodes[localPath];
    if (inode == null) return null;
    return (await identity.lookup(inode))?.fileId;
  }

  /// 程序自身创建/落盘文件后的身份记账（docs/design/10 §4.1 upsertMapping）。
  Future<void> recordInodeIdentity(String localPath, String fileId) async {
    final provider = inodeBatchProvider;
    if (provider == null) return;
    try {
      final inodes = await provider([localPath]);
      final inode = inodes[localPath];
      if (inode != null) {
        final rel = MountPath.relativePathFromMount(mountDir, localPath);
        await identity.upsert(inode, rel, fileId);
      }
    } catch (e) {
      AppLogger.w('inode 身份记账失败（忽略）: $e');
    }
  }

  /// xattr 读写入口（供 free_up 等同层模块复用）
  XattrService get xattr => _xattr;

  /// 确保挂载目录存在（初始化时调用）。
  Future<void> ensureMountDir() async {
    if (!await Directory(mountDir).exists()) {
      try {
        await Directory(mountDir).create(recursive: true);
      } catch (e) {
        throw AppError.generic('创建挂载目录失败：$e');
      }
    }
  }

  /// 确保文件夹存在（递归创建），返回完整路径。
  Future<String> ensureFolder(String relPath) async {
    final full = MountPath.safeJoinUnder(mountDir, relPath, allowEmpty: true);
    if (!await Directory(full).exists()) {
      try {
        await Directory(full).create(recursive: true);
      } catch (e) {
        throw AppError.generic('创建目录失败：$e');
      }
    }
    return full;
  }

  /// 为云端文件创建本地占位符（创建即打 Finder 灰标）。
  /// 对齐 Rust `create_placeholder_if_needed`：
  /// - 若文件已存在且 xattrState=downloaded → skip
  /// - 若文件已存在且 xattrState=placeholder → skip
  /// - 若文件已存在但无 xattr → 拒绝（用户文件，永远不转为占位符）
  /// - 否则：确保父目录 → 排他新建 0 字节文件 → 写 3 个状态 xattr + Finder 灰标
  Future<void> createPlaceholderIfNeeded(
    String fileName,
    String fileId,
    int size,
  ) async {
    final localPath = MountPath.safeJoinUnder(mountDir, fileName);

    // 目标检查必须失败即停；普通存在性判断会隐藏权限或 I/O 错误。
    final type = await FileSystemEntity.type(localPath, followLinks: false);
    if (type != FileSystemEntityType.notFound) {
      if (type != FileSystemEntityType.file) {
        throw AppError.generic('占位目标已存在且不是普通文件');
      }
      final state = await _xattr.get(localPath, xattrState);
      // 属主核验：inode 映射优先，xattr 为过渡期回退（阶段 5 移除回退）
      var owner = await inodeOwnerOf(localPath);
      owner ??= await _xattr.get(localPath, xattrFileId);
      if ((state == stateDownloaded || state == statePlaceholder) &&
          owner == fileId) {
        return;
      }
      throw AppError.generic('占位目标已有用户内容或属于其他 fileId，拒绝覆盖');
    }

    await _createPlaceholderExclusive(localPath, fileName, fileId, size);
  }

  /// 仅在目标仍不存在时创建占位符。
  ///
  /// 破坏性流程在原文件完成原子暂存后使用此严格入口；已有用户文件绝不视为成功，
  /// 任一必要 xattr 写入失败时会清理未完成的 placeholder。
  Future<void> createPlaceholderStrict(
    String fileName,
    String fileId,
    int size,
  ) async {
    final localPath = MountPath.safeJoinUnder(mountDir, fileName);
    await _createPlaceholderExclusive(localPath, fileName, fileId, size);
  }

  /// 排他新建 0 字节占位符并写状态 xattr + Finder 灰标。
  Future<void> _createPlaceholderExclusive(
    String localPath,
    String fileName,
    String fileId,
    int size,
  ) async {
    // 确保父目录存在
    try {
      await Directory(p.dirname(localPath)).create(recursive: true);
    } catch (e) {
      throw AppError.generic('创建占位父目录失败：$e');
    }
    // 排他新建消除检查到创建的竞争窗口，且不会截断已有路径。
    final file = File(localPath);
    try {
      await file.create(exclusive: true);
    } catch (e) {
      throw AppError.generic('创建占位符失败：$e');
    }
    // 写 3 个状态 xattr（占位即打标，含批量 BFS）
    try {
      await _writePlaceholderXattrs(localPath, fileId, size);
      // 持久化占位符（对齐 Rust file.sync_all）
      final raf = await file.open(mode: FileMode.write);
      try {
        await raf.flush();
      } finally {
        await raf.close();
      }
    } catch (e) {
      try {
        await file.delete();
      } catch (_) {
        // 清理失败不掩盖原始错误
      }
      rethrow;
    }
    await setFinderLabel(localPath, true); // 灰标失败不阻断（仅 Finder 无灰标）
    // 身份记账（docs/design/10 §4.9：占位创建 → DB 写 inode 映射）
    await recordInodeIdentity(localPath, fileId);
  }

  /// 写占位的 state xattr（inode 方案唯一占位键，docs/design/10 §4.9；
  /// 文件身份经 [recordInodeIdentity] 写入 local_inode_map）。
  Future<void> _writePlaceholderXattrs(
      String localPath, String fileId, int size) async {
    try {
      await _xattr.set(localPath, xattrState, statePlaceholder);
    } catch (e) {
      throw AppError.generic('写 xattr state 失败：$e');
    }
  }

  /// 标记文件为已下载（更新 xattr + 清除灰标）。
  /// 对齐 Rust `mark_downloaded`。
  Future<void> markDownloaded(String localPath) async {
    try {
      await _xattr.set(localPath, xattrState, stateDownloaded);
    } catch (e) {
      throw AppError.generic('更新 xattr 失败：$e');
    }
    await setFinderLabel(localPath, false); // 清除灰标（尽力）
  }

  /// 下载前处理可能被用户修改过的占位文件
  /// （对齐 Rust `backup_modified_placeholder_if_needed`）。
  ///
  /// - 不存在 / 非 placeholder / 0 字节未修改 → 返回 null（调用方直接下载覆盖/删除）
  /// - state=placeholder 且 size>0（用户写入了内容）→ **改名**保留到
  ///   `<basename>.local-<yyyyMMdd-HHmmss>.<ext>`（撞名加序号），清掉备份的占位 xattr
  ///   （避免被 sync 当成新占位），返回备份路径。下载再写到原路径。
  Future<String?> backupModifiedPlaceholderIfNeeded(String localPath) async {
    if (await FileSystemEntity.type(localPath, followLinks: false) ==
        FileSystemEntityType.notFound) {
      return null;
    }
    // 必须是占位（state=placeholder）才走备份逻辑
    final state = await _xattr.get(localPath, xattrState);
    if (state != statePlaceholder) {
      return null;
    }
    // 占位创建时 0 字节，size>0 即被用户写入了内容
    final stat = await FileStat.stat(localPath);
    if (stat.size == 0) {
      return null;
    }
    // 改名保留：<base>.local-<stamp>.<ext>
    final stamp = DateFormat('yyyyMMdd-HHmmss').format(DateTime.now());
    final dir = p.dirname(localPath);
    final basename = p.basenameWithoutExtension(localPath);
    final ext = p.extension(localPath);
    var backup = p.join(dir, '$basename.local-$stamp$ext');
    var seq = 1;
    while (await FileSystemEntity.type(backup, followLinks: false) !=
        FileSystemEntityType.notFound) {
      backup = p.join(dir, '$basename.local-$stamp.$seq$ext');
      seq++;
    }
    await File(localPath).rename(backup);
    // 清掉备份的占位 xattr，避免被 sync 当新占位（尽力，对齐 Rust `let _ =`）
    try {
      await clearPlaceholderXattr(backup);
    } catch (_) {
      // 尽力清理
    }
    AppLogger.i('占位被修改过，已备份：$localPath → $backup');
    return backup;
  }

  /// 清除文件上的占位 xattr（fileId/state/size/FinderInfo）。
  ///
  /// 备份副本改名后调用：让副本被视为全新本地文件（对齐 Rust
  /// `clear_placeholder_xattr`；单项移除失败不阻断）。
  Future<void> clearPlaceholderXattr(String localPath) async {
    // state + FinderInfo 为现行键；fileId 为旧版键（存在即清，迁移兼容）
    for (final key in [xattrState, finderInfoXattr, xattrFileId]) {
      try {
        await _xattr.remove(localPath, key);
      } catch (_) {
        // 尽力清理
      }
    }
  }

  /// 通过 xattr 判断是否为占位符（state=placeholder）。
  Future<bool> isPlaceholderFile(String path) async {
    final state = await _xattr.get(path, xattrState);
    return state == statePlaceholder;
  }

  /// 设置/清除 Finder 灰色标签：直接读写 com.apple.FinderInfo xattr，无 fork。
  /// - gray=true：byte[9]=0x02（灰标）
  /// - gray=false：byte[9]=0x00（清除；若整块全 0 则删 xattr，对齐 osascript label 0）
  ///
  /// 用直接 xattr 写而非 osascript，避免批量文件 fork 进程风暴；
  /// 读改写保留其它 FinderInfo 字段。失败不阻断（对齐 Rust 调用方 `let _ =`）。
  Future<void> setFinderLabel(String path, bool gray) async {
    try {
      var buf = await _xattr.getBytes(path, finderInfoXattr) ?? Uint8List(0);
      if (buf.length < 32) {
        final grown = Uint8List(32);
        grown.setRange(0, buf.length, buf);
        buf = grown;
      }
      buf[9] = gray ? grayLabelByte : 0x00;
      if (!gray && buf.every((b) => b == 0)) {
        await _xattr.remove(path, finderInfoXattr);
      } else {
        await _xattr.setBytes(path, finderInfoXattr, buf);
      }
    } catch (e) {
      AppLogger.d('设置 Finder 灰标失败（忽略）：$path：$e');
    }
  }

  /// 扫描挂载目录，返回全部非跳过文件的条目。
  /// 对齐 Rust `scan_local`：跳过内部项和符号链接。
  Future<List<LocalFileEntry>> scanLocal(List<String> skipPatterns) async {
    // ★ 挂载目录为空时跳过扫描，返回空列表（避免误扫根目录或判断"本地无"误删云端）
    if (mountDir.isEmpty) {
      AppLogger.w('scanLocal 跳过：挂载目录未配置');
      return [];
    }
    final entries = <LocalFileEntry>[];
    try {
      await _scanRecursive(mountDir, skipPatterns, entries);
    } catch (e) {
      throw AppError.generic('扫描目录失败：$e');
    }
    // 两阶段扫描（docs/design/10 §4.2）：遍历后批量填充 inode
    final provider = inodeBatchProvider;
    if (provider != null && entries.isNotEmpty) {
      try {
        final inodes =
            await provider([for (final e in entries) e.absolutePath]);
        return [
          for (final e in entries)
            inodes[e.absolutePath] != null
                ? e.withInode(inodes[e.absolutePath]!)
                : e,
        ];
      } catch (e) {
        // inode 采集失败不阻断扫描（本轮无 inode 数据，退化为路径配对）
        AppLogger.w('批量 inode 查询失败（本轮无 inode 数据）: $e');
      }
    }
    return entries;
  }

  /// 递归扫描普通文件与目录，跳过内部项和符号链接。
  Future<void> _scanRecursive(
    String current,
    List<String> skipPatterns,
    List<LocalFileEntry> out,
  ) async {
    final stream = Directory(current).list(followLinks: false);
    await for (final entity in stream) {
      final name = p.basename(entity.path);

      // 跳过内部文件
      if (MountSkip.shouldSkip(name, skipPatterns)) {
        continue;
      }

      final type =
          await FileSystemEntity.type(entity.path, followLinks: false);
      // 符号链接整体跳过（对齐 Rust file_type 不跟随链接的 is_dir/is_file 判定）
      if (type == FileSystemEntityType.link) {
        continue;
      }
      final rel = p.relative(entity.path, from: mountDir);
      final stat = await FileStat.stat(entity.path);
      final mtime = stat.modified.millisecondsSinceEpoch;

      if (type == FileSystemEntityType.directory) {
        out.add(LocalFileEntry(
          absolutePath: entity.path,
          relativePath: rel,
          size: 0,
          mtime: mtime,
          isFolder: true,
          isPlaceholder: false,
        ));
        // 递归进入子目录
        await _scanRecursive(entity.path, skipPatterns, out);
      } else if (type == FileSystemEntityType.file) {
        final size = stat.size;
        // 占位符判断用 xattr state，而非 0 字节（用户空文件如 .gitkeep 不是占位符）
        final isPlaceholder = size == 0 && await isPlaceholderFile(entity.path);
        out.add(LocalFileEntry(
          absolutePath: entity.path,
          relativePath: rel,
          size: size,
          mtime: mtime,
          isFolder: false,
          isPlaceholder: isPlaceholder,
        ));
      }
    }
  }

  /// 删除本地文件（安全：0 字节文件若非占位符则拒绝删除，返回但跳过）。
  /// 对齐 Rust `delete_local`。
  Future<void> deleteLocal(String localPath) async {
    MountPath.relativePathFromMount(mountDir, localPath);
    final type = await FileSystemEntity.type(localPath, followLinks: false);
    if (type == FileSystemEntityType.notFound) {
      return;
    }
    if (type == FileSystemEntityType.directory) {
      // 红线（ai/coding-rules.md §十）：递归删除前扫描符号链接
      await _assertNoSymlinks(localPath);
      try {
        await Directory(localPath).delete(recursive: true);
      } catch (e) {
        throw AppError.generic('删除目录失败：$e');
      }
      return;
    }
    // 0 字节文件：必须是占位符才删；否则保留（用户文件如 .gitkeep）
    final stat = await FileStat.stat(localPath);
    if (stat.size == 0 && !(await isPlaceholderFile(localPath))) {
      AppLogger.d('保留非占位 0 字节文件：$localPath');
      return;
    }
    try {
      await File(localPath).delete();
    } catch (e) {
      throw AppError.generic('删除文件失败：$e');
    }
    // 清理旧版占位符
    await _removeLegacyPlaceholder(localPath);
  }

  /// 删除一个已经由同步执行器完成远端与本地版本复核的路径。
  ///
  /// 与面向普通调用方的 [deleteLocal] 不同，此入口允许删除真实的 0 字节文件；
  /// 调用方必须先证明它仍与持久化同步基线一致。路径边界仍在此处再次校验。
  /// （对齐 Rust `delete_local_confirmed`，仅供同步执行器使用。）
  Future<void> deleteLocalConfirmed(String localPath) async {
    MountPath.relativePathFromMount(mountDir, localPath);
    final type = await FileSystemEntity.type(localPath, followLinks: false);
    switch (type) {
      case FileSystemEntityType.notFound:
        return;
      case FileSystemEntityType.link:
        throw AppError.generic('拒绝删除符号链接');
      case FileSystemEntityType.directory:
        await _assertNoSymlinks(localPath);
        try {
          await Directory(localPath).delete(recursive: true);
        } catch (e) {
          throw AppError.generic('删除目录失败：$e');
        }
      case FileSystemEntityType.file:
        try {
          await File(localPath).delete();
        } catch (e) {
          throw AppError.generic('删除文件失败：$e');
        }
      default:
        throw AppError.generic('拒绝删除非普通文件类型');
    }
    await _removeLegacyPlaceholder(localPath);
  }

  /// 清理旧版占位符（尽力）。
  Future<void> _removeLegacyPlaceholder(String localPath) async {
    final legacy = File(localPath + MountSkip.legacyPlaceholderSuffix);
    try {
      if (await legacy.exists()) {
        await legacy.delete();
      }
    } catch (e) {
      AppLogger.w('清理旧版占位符失败：${legacy.path}：$e');
    }
  }

  /// 递归删除前断言目录子树不含符号链接（文件系统安全红线）。
  Future<void> _assertNoSymlinks(String dir) async {
    final stream = Directory(dir).list(recursive: true, followLinks: false);
    await for (final entity in stream) {
      if (entity is Link) {
        throw AppError.generic('拒绝递归删除含符号链接的目录：${entity.path}');
      }
    }
  }

  // ============================================================
  // 本地同步状态判定（对齐 Rust src/commands/sync_status.rs）
  // ============================================================

  /// 查询文件本地同步状态（供前端删除确认用）。
  /// 返回 "folder" | "synced" | "placeholder" | "not_synced"。
  ///
  /// 占位状态只以 xattr 为准；真实的 0 字节文件不能按长度误判成占位符。
  Future<String> checkFileLocalStatus(String fileId) async {
    final rawDb = await _requireDb().database;
    final record = await findByFileId(rawDb, fileId);
    if (record == null) {
      return 'not_synced';
    }
    if (record.isFolder) {
      return 'folder';
    }
    final absPath = p.join(mountDir, record.localPath);
    try {
      final stat = await FileStat.stat(absPath);
      // dart:io FileStat.stat 对缺失文件不抛异常而是返回 notFound 类型
      if (stat.type == FileSystemEntityType.notFound) {
        return 'not_synced';
      }
    } catch (e) {
      throw AppError.generic('读取本地同步状态失败：$e');
    }
    if (await isPlaceholderFile(absPath)) {
      return 'placeholder';
    }
    return 'synced';
  }

  /// 批量查询文件同步状态（供前端文件列表状态列展示用）。
  /// 返回 fileId → "folder" | "synced" | "placeholder" | "not_synced" 映射。
  /// 未配置同步目录时回退到仅 DB 状态判断。
  Future<Map<String, String>> batchFileLocalStatus(
      List<String> fileIds) async {
    final rawDb = await _requireDb().database;
    final hasMount = mountDir.isNotEmpty;
    final result = <String, String>{};

    for (final fileId in fileIds) {
      final record = await findByFileId(rawDb, fileId);
      final String status;
      if (record == null) {
        status = 'not_synced';
      } else if (record.isFolder) {
        status = 'folder';
      } else if (hasMount) {
        final absPath = p.join(mountDir, record.localPath);
        String probed;
        try {
          final stat = await FileStat.stat(absPath);
          if (stat.type == FileSystemEntityType.notFound) {
            probed = 'not_synced';
          } else {
            probed =
                await isPlaceholderFile(absPath) ? 'placeholder' : 'synced';
          }
        } catch (e) {
          throw AppError.generic('读取本地同步状态失败：$e');
        }
        status = probed;
      } else {
        // 未配置挂载目录：仅从 DB 状态判定
        status =
            record.status == SyncItemStatus.synced ? 'synced' : 'not_synced';
      }
      result[fileId] = status;
    }

    return result;
  }

  // ============================================================
  // 中断的释放空间恢复（对齐 Rust recover_interrupted_free_up）
  // ============================================================

  /// 收敛上次进程在“原文件暂存 → 占位符/DB 结算”之间退出留下的文件。
  /// 数据库已提交且占位符身份匹配时完成释放；其余情况优先恢复原内容。
  ///
  /// 恢复记录来源（docs/design/10 §4.8）：`free_up_staging` 表为主，
  /// 旧版 xattr 暂存文件（迁移前现场）为兼容回退。
  Future<int> recoverInterruptedFreeUp(Database db) async {
    var recovered = 0;
    final handledStagingPaths = <String>{};

    // 1. DB 恢复记录（新来源）
    final rows = await db.query('free_up_staging');
    for (final row in rows) {
      final stagingName = row['staging_name'] as String? ?? '';
      final relativePath = row['relative_path'] as String? ?? '';
      final fileId = row['file_id'] as String? ?? '';
      if (stagingName.isEmpty || relativePath.isEmpty || fileId.isEmpty) {
        await db.delete('free_up_staging',
            where: 'staging_name = ?', whereArgs: [stagingName]);
        continue;
      }
      final stagingPath = p.join(
          p.dirname(p.join(mountDir, relativePath)), stagingName);
      handledStagingPaths.add(stagingPath);
      await _recoverOneFreeUpStaging(db, stagingPath, relativePath, fileId);
      recovered++;
      await db.delete('free_up_staging',
          where: 'staging_name = ?', whereArgs: [stagingName]);
    }

    // 2. 旧版 xattr 现场（迁移兼容一轮：仅处理 DB 未覆盖的暂存文件）
    final stagingPaths = <String>[];
    await _collectFreeUpStaging(mountDir, stagingPaths);
    for (final stagingPath in stagingPaths) {
      if (handledStagingPaths.contains(stagingPath)) continue;
      final relativePath =
          await _xattr.get(stagingPath, xattrFreeUpRelativePath);
      final fileId = await _xattr.get(stagingPath, xattrFileId);
      if (relativePath == null || fileId == null) {
        await _surfaceFreeUpRecovery(stagingPath);
        recovered++;
        continue;
      }
      await _recoverOneFreeUpStaging(db, stagingPath, relativePath, fileId);
      recovered++;
    }
    return recovered;
  }

  /// 收敛单个暂存项（DB 记录与旧 xattr 现场共用恢复逻辑）。
  Future<void> _recoverOneFreeUpStaging(
    Database db,
    String stagingPath,
    String relativePath,
    String fileId,
  ) async {
    // 暂存文件已不存在（清理窗口崩溃）→ 无需恢复，记录由调用方清理
    if (await FileSystemEntity.type(stagingPath, followLinks: false) ==
        FileSystemEntityType.notFound) {
      return;
    }
    final String target;
    try {
      target = MountPath.safeJoinUnder(mountDir, relativePath);
      if (p.dirname(target) != p.dirname(stagingPath)) {
        throw AppError.config('释放空间恢复目标不在原目录');
      }
    } catch (_) {
      await _surfaceFreeUpRecovery(stagingPath);
      return;
    }
    final record = await findByFileId(db, fileId);
    final baseline =
        (record != null && record.localPath == relativePath) ? record : null;
    final targetType =
        await FileSystemEntity.type(target, followLinks: false);
    if (targetType != FileSystemEntityType.notFound &&
        await isPlaceholderFile(target)) {
      // 属主核验：inode 映射优先，xattr 过渡回退（docs/design/10 §4.8）
      var owner = await inodeOwnerOf(target);
      owner ??= await _xattr.get(target, xattrFileId);
      final committed = owner == fileId &&
          baseline != null &&
          baseline.status == SyncItemStatus.cloudOnly &&
          baseline.localSize == 0;
      if (committed) {
        try {
          await File(stagingPath).delete();
        } catch (e) {
          throw AppError.generic('完成释放空间恢复清理失败：$e');
        }
      } else {
        try {
          await File(target).delete();
        } catch (e) {
          throw AppError.generic('移除未提交占位符失败：$e');
        }
        await _restoreFreeUpStaging(
            db, stagingPath, target, fileId, relativePath);
      }
    } else if (targetType == FileSystemEntityType.notFound) {
      await _restoreFreeUpStaging(
          db, stagingPath, target, fileId, relativePath);
    } else {
      // 原路径已有用户内容或无法可靠读取时，不覆盖；把旧内容显式恢复为副本。
      await _surfaceFreeUpRecovery(stagingPath);
    }
  }

  /// 递归收集释放空间暂存项，并跳过符号链接目录。
  Future<void> _collectFreeUpStaging(
      String current, List<String> output) async {
    try {
      await for (final entity
          in Directory(current).list(followLinks: false)) {
        final type =
            await FileSystemEntity.type(entity.path, followLinks: false);
        if (type == FileSystemEntityType.link) {
          continue;
        }
        final name = p.basename(entity.path);
        if (name.startsWith(freeUpStagingPrefix)) {
          output.add(entity.path);
        } else if (type == FileSystemEntityType.directory) {
          await _collectFreeUpStaging(entity.path, output);
        }
      }
    } catch (e) {
      throw AppError.generic('扫描释放空间恢复项失败：$e');
    }
  }

  /// 将暂存原文件恢复到已核验目标，并同步修复数据库基线。
  Future<void> _restoreFreeUpStaging(
    Database db,
    String stagingPath,
    String target,
    String fileId,
    String relativePath,
  ) async {
    final FileStat metadata;
    try {
      metadata = await FileStat.stat(stagingPath);
    } catch (e) {
      throw AppError.generic('读取待恢复原文件失败：$e');
    }
    final localMtime = metadata.modified.millisecondsSinceEpoch;
    try {
      await File(stagingPath).rename(target);
    } catch (e) {
      throw AppError.generic('恢复释放空间原文件失败：$e');
    }
    try {
      await _xattr.remove(target, xattrFreeUpRelativePath);
    } catch (_) {
      // 尽力移除恢复标记
    }
    try {
      await db.update(
        'sync_items',
        {
          'status': SyncItemStatus.synced.code,
          'local_size': metadata.size,
          'local_mtime': localMtime,
          'error_message': null,
        },
        where: 'file_id = ? AND local_path = ?',
        whereArgs: [fileId, relativePath],
      );
    } catch (e) {
      throw AppError.generic('恢复释放空间同步基线失败：$e');
    }
    AppLogger.w('检测到中断的释放空间操作，已恢复原文件：$target');
  }

  /// 无法安全覆盖原路径时，把暂存内容改名为可见恢复副本。
  Future<String> _surfaceFreeUpRecovery(String stagingPath) async {
    final parent = p.dirname(stagingPath);
    final random = Random.secure();
    for (var i = 0; i < 16; i++) {
      final target = p.join(parent, '释放空间恢复-${randomHex64(random)}');
      if (await FileSystemEntity.type(target, followLinks: false) !=
          FileSystemEntityType.notFound) {
        continue;
      }
      try {
        await File(stagingPath).rename(target);
      } catch (e) {
        throw AppError.generic('显式恢复暂存内容失败：$e');
      }
      for (final key in [
        xattrFreeUpRelativePath,
        xattrFileId,
        xattrState,
      ]) {
        try {
          await _xattr.remove(target, key);
        } catch (_) {
          // 尽力移除
        }
      }
      AppLogger.w('释放空间恢复无法覆盖原路径，已保留为可见副本：$target');
      return target;
    }
    throw AppError.generic('无法分配释放空间恢复副本路径');
  }

  // ============================================================
  // DB 辅助
  // ============================================================

  DatabaseService _requireDb() {
    final service = db;
    if (service == null) {
      throw AppError.config('未注入 DatabaseService，无法查询同步基线');
    }
    return service;
  }

  /// 按 fileId 查询单条同步记录；多条歧义基线拒绝使用
  /// （对齐 Rust repository::find_by_file_id 的 LIMIT 2 歧义检测）。
  static Future<SyncItem?> findByFileId(Database db, String fileId) async {
    final rows = await db.query(
      'sync_items',
      where: 'file_id = ?',
      whereArgs: [fileId],
      limit: 2,
    );
    if (rows.isEmpty) return null;
    if (rows.length > 1) {
      throw AppError.generic('fileId $fileId 对应多条本地路径，拒绝使用歧义同步基线');
    }
    return SyncItem.fromRow(rows.first);
  }
}

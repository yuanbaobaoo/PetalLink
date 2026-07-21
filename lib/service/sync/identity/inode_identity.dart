/// 基于 inode 的文件身份识别（对标 CMP `sync/identity/InodeIdentity.kt`，
/// 设计见 `docs/design/10-基于inode的文件身份识别方案.md`）。
///
/// 取代 fileId xattr 机制：inode 在 mv 改名时不变、cp 复制时产生新编号，
/// 因此"同一身份出现在多处"在结构上不可能发生。身份查询只读 DB，
/// 不碰文件 xattr，不涉及任何"补写自愈"。
library;

import 'package:petal_link/core/storage/database_service.dart';

/// inode 映射记录：一个本地 inode ↔ 云端身份的对应关系。
/// 对应数据库表 `local_inode_map`（docs/design/04 §5.5）。
class InodeRecord {
  /// 文件系统 inode 编号（`stat().st_ino`；macOS u64，SQLite INTEGER 可容纳）
  final int inode;

  /// 相对挂载根的路径（与 `sync_items.local_path` 同语义）
  final String relativePath;

  /// 云端文件 ID（与 `sync_items.file_id` 对应）
  final String fileId;

  /// 上次扫描到该 inode 的时间戳（毫秒 epoch），用于清理陈旧记录
  final int scannedAt;

  const InodeRecord({
    required this.inode,
    required this.relativePath,
    required this.fileId,
    required this.scannedAt,
  });
}

/// inode 身份存储接口（数据层/测试 fake 实现）。
///
/// 所有身份查询都走此接口——只读 DB 操作，不碰文件 xattr。
abstract class InodeIdentityStore {
  /// 查询某 inode 对应的云端身份（扫描时识别移动：
  /// 同 inode 出现在新路径 = 移动）。
  Future<InodeRecord?> lookup(int inode);

  /// 下载/释放空间/占位创建完成后主动更新映射
  /// （程序自己操作文件时的确定性记账；替代旧方案的 xattr 补写——
  /// 要么成功要么回滚，不再静默丢失）。
  Future<void> upsert(int inode, String relativePath, String fileId);

  /// 扫描结束后，根据本轮见到的 inode 集合清理陈旧记录。
  ///
  /// 整表可安全清空重建：丢失只让本轮移动检测退化为删+增，
  /// 下一轮自动恢复（docs/design/10 §6.3）。
  Future<void> purgeMissing(Set<int> seenInodes);
}

/// 基于 sqflite 的生产实现（`local_inode_map` 表，v6 迁移创建）。
class SqfliteInodeIdentityStore implements InodeIdentityStore {
  /// 数据库服务
  final DatabaseService _db;

  /// 时钟（测试可注入假时钟）
  final int Function() _nowMs;

  SqfliteInodeIdentityStore(this._db, {int Function()? nowMs})
      : _nowMs = nowMs ?? _defaultNow;

  static int _defaultNow() => DateTime.now().millisecondsSinceEpoch;

  @override
  Future<InodeRecord?> lookup(int inode) async {
    final db = await _db.database;
    final rows = await db.query(
      'local_inode_map',
      where: 'inode = ?',
      whereArgs: [inode],
      limit: 1,
    );
    if (rows.isEmpty) return null;
    final row = rows.first;
    return InodeRecord(
      inode: row['inode'] as int,
      relativePath: row['relative_path'] as String,
      fileId: row['file_id'] as String,
      scannedAt: row['scanned_at'] as int,
    );
  }

  @override
  Future<void> upsert(int inode, String relativePath, String fileId) async {
    final db = await _db.database;
    await db.rawInsert(
      'INSERT OR REPLACE INTO local_inode_map '
      '(inode, relative_path, file_id, scanned_at) VALUES (?, ?, ?, ?)',
      [inode, relativePath, fileId, _nowMs()],
    );
  }

  @override
  Future<void> purgeMissing(Set<int> seenInodes) async {
    final db = await _db.database;
    if (seenInodes.isEmpty) {
      await db.delete('local_inode_map');
      return;
    }
    final placeholders = List.filled(seenInodes.length, '?').join(',');
    await db.rawDelete(
      'DELETE FROM local_inode_map WHERE inode NOT IN ($placeholders)',
      seenInodes.toList(),
    );
  }
}

/// 内存实现（测试 fake，契约与 sqflite 实现一致）。
class MemoryInodeIdentityStore implements InodeIdentityStore {
  final Map<int, InodeRecord> _map = {};
  int _clock = 1;

  /// 测试观测口：当前全部映射
  Map<int, InodeRecord> get debugAll => Map.unmodifiable(_map);

  @override
  Future<InodeRecord?> lookup(int inode) async => _map[inode];

  @override
  Future<void> upsert(int inode, String relativePath, String fileId) async {
    _map[inode] = InodeRecord(
      inode: inode,
      relativePath: relativePath,
      fileId: fileId,
      scannedAt: _clock++,
    );
  }

  @override
  Future<void> purgeMissing(Set<int> seenInodes) async {
    _map.removeWhere((inode, _) => !seenInodes.contains(inode));
  }
}

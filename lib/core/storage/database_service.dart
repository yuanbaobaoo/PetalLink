import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:sqflite/sqflite.dart';

import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/core/storage/app_paths.dart';

/// 本地 SQLite 数据库服务（单例）。
///
/// 严格对齐 Rust 原版 `src/data/migrations.rs`（schemaVersion=5）：
/// - `sync_items`：file_id + local_path 复合 PK
/// - `transfer_queue`：9 态状态机字段（state / operation / resume_offset /
///   session_url / attempt_count / next_retry_at / error_kind / state_revision 等）
///
/// 迁移策略（对齐 Rust run_with_mount）：
/// - 全新数据库：直接建 v5 终态，避免先建旧结构再 ALTER
/// - 旧数据库（Rust v1-v4 结构）：逐步 ALTER 升级
/// - 骨架期 v1 数据库（列结构不兼容）：重建（仅开发期产物，无用户数据）
///
/// 除 Rust 两张表外保留 `config` 键值表（Flutter 侧配置持久化，
/// 对应 Rust 的 config.json，见 ai/coding-rules.md §八）。
class DatabaseService {
  DatabaseService._internal();

  /// 单例
  factory DatabaseService() => _instance;
  static final DatabaseService _instance = DatabaseService._internal();

  /// 单例实例（等价于 [DatabaseService()] 构造）
  static DatabaseService get instance => _instance;

  /// 数据库结构版本（对齐 Rust SCHEMA_VERSION）
  static const int schemaVersion = 7;

  /// 数据库文件名（对齐 Rust DB_FILE_NAME）
  static const String dbFileName = 'petal_link.db';

  /// 测试用：覆盖数据库文件路径
  @visibleForTesting
  static String? debugDatabasePath;

  Database? _database;

  /// 获取数据库实例（懒初始化）。
  ///
  /// 首次访问时自动创建数据库及表结构。
  Future<Database> get database async {
    if (_database != null) return _database!;
    _database = await _initDatabase();
    return _database!;
  }

  /// 关闭数据库连接。
  Future<void> close() async {
    final db = _database;
    if (db != null && db.isOpen) {
      await db.close();
      _database = null;
      AppLogger.i('数据库已关闭');
    }
  }

  /// 清空全部业务表（对齐 Rust `delete_all` + `delete_all_transfers`；
  /// config 表对应 Rust 删除 config.json 的语义）
  Future<void> deleteAllData() async {
    final db = await database;
    await db.delete('sync_items');
    await db.delete('transfer_queue');
    await db.delete('config');
    AppLogger.i('数据库业务表已清空');
  }

  /// 删除数据库文件（含 WAL/SHM 伴随文件；对齐 Rust app_clear_cache
  /// 删除 db 文件）。先关闭连接，删除后下次访问自动重建。
  Future<void> deleteDatabaseFile() async {
    final path = debugDatabasePath ?? await AppPaths.databasePath();
    await close();
    for (final suffix in ['', '-wal', '-shm']) {
      final file = File('$path$suffix');
      if (await file.exists()) {
        await file.delete();
      }
    }
    AppLogger.i('数据库文件已删除: $path');
  }

  /// 初始化数据库：确定路径 → 打开/创建 → 迁移
  Future<Database> _initDatabase() async {
    final dbPath = debugDatabasePath ?? await AppPaths.databasePath();

    AppLogger.i('数据库路径: $dbPath');

    return openDatabase(
      dbPath,
      version: schemaVersion,
      onCreate: _onCreate,
      onUpgrade: _onUpgrade,
      onConfigure: (db) async {
        // 启用 WAL 模式，提升并发读性能
        // 注意：PRAGMA journal_mode 返回结果行，sqflite_darwin 的
        // execute() 对带返回值的语句报 Code=0 错误，必须用 rawQuery
        await db.rawQuery('PRAGMA journal_mode=WAL');
        // 启用外键约束
        await db.rawQuery('PRAGMA foreign_keys=ON');
      },
    );
  }

  // ============================================================
  // 建表（v5 终态）
  // ============================================================

  /// 新库直接创建为 v5 终态结构（对齐 Rust create_all）。
  Future<void> _onCreate(Database db, int version) async {
    AppLogger.i('创建数据库表 (version $version)');
    await _createAll(db);
  }

  /// 创建全部表与索引。
  Future<void> _createAll(Database db) async {
    // 同步条目（对齐 Rust sync_items）
    await db.execute('''
      CREATE TABLE IF NOT EXISTS sync_items (
          file_id           TEXT    NOT NULL,
          local_path        TEXT    NOT NULL,
          parent_folder_id  TEXT,
          name              TEXT    NOT NULL,
          is_folder         INTEGER NOT NULL DEFAULT 0,
          size              INTEGER NOT NULL DEFAULT 0,
          local_size        INTEGER,
          sha256            TEXT,
          local_mtime       INTEGER,
          cloud_edited_time INTEGER,
          last_sync_time    INTEGER,
          status            INTEGER NOT NULL DEFAULT 0,
          error_message     TEXT,
          PRIMARY KEY (file_id, local_path)
      )
    ''');

    // 传输队列（对齐 Rust transfer_queue，9 态状态机）
    await db.execute('''
      CREATE TABLE IF NOT EXISTS transfer_queue (
          id            INTEGER PRIMARY KEY AUTOINCREMENT,
          direction     INTEGER NOT NULL,
          file_id       TEXT,
          local_path    TEXT,
          name          TEXT    NOT NULL,
          total_size    INTEGER NOT NULL DEFAULT 0,
          transferred   INTEGER NOT NULL DEFAULT 0,
          state         INTEGER NOT NULL DEFAULT 0,
          error_message TEXT,
          created_at    INTEGER NOT NULL,
          finished_at   INTEGER,
          server_id     TEXT,
          upload_id     TEXT,
          resume_offset INTEGER NOT NULL DEFAULT 0,
          session_url   TEXT,
          relative_path TEXT,
          parent_file_id TEXT,
          operation INTEGER,
          source_mtime INTEGER,
          source_size INTEGER,
          expected_cloud_edited_time INTEGER,
          attempt_count INTEGER NOT NULL DEFAULT 0,
          next_retry_at INTEGER,
          error_kind INTEGER,
          remote_result_file_id TEXT,
          state_revision INTEGER NOT NULL DEFAULT 0
      )
    ''');

    // 配置键值（Flutter 侧配置持久化，对应 Rust config.json）
    await db.execute('''
      CREATE TABLE IF NOT EXISTS config (
          key   TEXT PRIMARY KEY,
          value TEXT
      )
    ''');

    await _createIndexes(db);
    await _createV6Tables(db);
  }

  /// v6：inode 身份识别两张表（docs/design/10 §3；纯增量，不回填）
  Future<void> _createV6Tables(Database db) async {
    // 文件身份映射（替代 com.hwcloud.fileId xattr）
    await db.execute('''
      CREATE TABLE IF NOT EXISTS local_inode_map (
          inode         INTEGER NOT NULL,
          relative_path TEXT    NOT NULL,
          file_id       TEXT    NOT NULL,
          scanned_at    INTEGER NOT NULL,
          PRIMARY KEY (inode)
      )
    ''');
    await db.execute(
        'CREATE INDEX IF NOT EXISTS idx_inode_map_path ON local_inode_map(relative_path)');
    await db.execute(
        'CREATE INDEX IF NOT EXISTS idx_inode_map_fid ON local_inode_map(file_id)');

    // 释放空间事务（替代 com.hwcloud.freeUpRelativePath xattr）
    await db.execute('''
      CREATE TABLE IF NOT EXISTS free_up_staging (
          staging_name   TEXT    NOT NULL PRIMARY KEY,
          relative_path  TEXT    NOT NULL,
          file_id        TEXT    NOT NULL,
          source_mtime   INTEGER,
          source_size    INTEGER,
          created_at     INTEGER NOT NULL
      )
    ''');
  }

  /// 创建全部索引（对齐 Rust create_all 与 v5 索引）。
  Future<void> _createIndexes(Database db) async {
    await db.execute(
        'CREATE INDEX IF NOT EXISTS idx_sync_items_file_id ON sync_items(file_id)');
    await db.execute(
        'CREATE INDEX IF NOT EXISTS idx_sync_items_status ON sync_items(status)');
    await db.execute(
        'CREATE INDEX IF NOT EXISTS idx_transfer_state ON transfer_queue(state)');
    await db.execute(
        'CREATE INDEX IF NOT EXISTS idx_transfer_state_retry ON transfer_queue(state, next_retry_at)');
    await db.execute(
        'CREATE INDEX IF NOT EXISTS idx_transfer_relative_state ON transfer_queue(relative_path, state)');
  }

  // ============================================================
  // 迁移（对齐 Rust 分步 upgrade_to_v2..v5）
  // ============================================================

  /// 旧库逐步升级；sqflite 已在事务内执行 onUpgrade。
  Future<void> _onUpgrade(Database db, int oldVersion, int newVersion) async {
    AppLogger.i('数据库迁移: v$oldVersion → v$newVersion');

    // 骨架期 v1 数据库列结构与 Rust v1 完全不兼容（旧表含 file_name/status TEXT 等），
    // 属于开发期产物，无真实用户数据，直接重建。
    if (await _isLegacySkeletonSchema(db)) {
      AppLogger.w('检测到骨架期不兼容数据库结构，重建全部表');
      await db.execute('DROP TABLE IF EXISTS transfer_queue');
      await db.execute('DROP TABLE IF EXISTS sync_items');
      await db.execute('DROP TABLE IF EXISTS sync_state');
      await db.execute('DROP TABLE IF EXISTS cloud_tree_cache');
      await _createAll(db);
      return;
    }

    if (oldVersion < 2) {
      await _upgradeToV2(db);
    }
    if (oldVersion < 3) {
      await _upgradeToV3(db);
    }
    if (oldVersion < 4) {
      await _upgradeToV4(db);
    }
    if (oldVersion < 5) {
      await _upgradeToV5(db);
    }
    if (oldVersion < 6) {
      // v6 只建表，不动旧数据（docs/design/10 §3.3：
      // 不回填历史 inode，首次扫描自动填充；不删除旧 xattr）
      await _createV6Tables(db);
    }
    if (oldVersion < 7) {
      // v7 兜底自愈：开发期曾存在另一套 v6 schema（sync_cursor 无
      // config/inode 表），与 inode v6 版本号相撞导致旧库永远跳过
      // 迁移。全部 CREATE IF NOT EXISTS，幂等补齐。
      await _createV6Tables(db);
      await db.execute('''
        CREATE TABLE IF NOT EXISTS config (
            key   TEXT PRIMARY KEY,
            value TEXT
        )
      ''');
    }

    // config 表为 Flutter 侧新增，旧库升级时补齐
    await db.execute('''
      CREATE TABLE IF NOT EXISTS config (
          key   TEXT PRIMARY KEY,
          value TEXT
      )
    ''');
  }

  /// 判断是否为骨架期 v1 的不兼容结构：
  /// 旧 transfer_queue 含 `file_name` 列且缺少 Rust v1 的 `name` 列。
  Future<bool> _isLegacySkeletonSchema(Database db) async {
    final columns = await db.rawQuery('PRAGMA table_info(transfer_queue)');
    if (columns.isEmpty) return false;
    final names = columns.map((c) => c['name'] as String).toSet();
    return names.contains('file_name') && !names.contains('name');
  }

  /// v2: TransferQueue 加分片续传断点字段（幂等安全）。
  Future<void> _upgradeToV2(Database db) async {
    await _addColumnIfMissing(db, 'transfer_queue', 'server_id', 'TEXT');
    await _addColumnIfMissing(db, 'transfer_queue', 'upload_id', 'TEXT');
    await _addColumnIfMissing(
        db, 'transfer_queue', 'resume_offset', 'INTEGER NOT NULL DEFAULT 0');
  }

  /// v3: SyncItems 加 localSize（本地变更检测，避免 mtime 精度不足漏判）。
  Future<void> _upgradeToV3(Database db) async {
    await _addColumnIfMissing(db, 'sync_items', 'local_size', 'INTEGER');
  }

  /// v4: TransferQueue 加 session_url（华为 resume 上传的 Location 头会话 URL）。
  Future<void> _upgradeToV4(Database db) async {
    await _addColumnIfMissing(db, 'transfer_queue', 'session_url', 'TEXT');
  }

  /// v5：补充持久化任务上下文并归一化旧生命周期值。
  Future<void> _upgradeToV5(Database db) async {
    await _addColumnIfMissing(db, 'transfer_queue', 'relative_path', 'TEXT');
    await _addColumnIfMissing(db, 'transfer_queue', 'parent_file_id', 'TEXT');
    await _addColumnIfMissing(db, 'transfer_queue', 'operation', 'INTEGER');
    await _addColumnIfMissing(db, 'transfer_queue', 'source_mtime', 'INTEGER');
    await _addColumnIfMissing(db, 'transfer_queue', 'source_size', 'INTEGER');
    await _addColumnIfMissing(
        db, 'transfer_queue', 'expected_cloud_edited_time', 'INTEGER');
    await _addColumnIfMissing(
        db, 'transfer_queue', 'attempt_count', 'INTEGER NOT NULL DEFAULT 0');
    await _addColumnIfMissing(db, 'transfer_queue', 'next_retry_at', 'INTEGER');
    await _addColumnIfMissing(db, 'transfer_queue', 'error_kind', 'INTEGER');
    await _addColumnIfMissing(
        db, 'transfer_queue', 'remote_result_file_id', 'TEXT');
    await _addColumnIfMissing(
        db, 'transfer_queue', 'state_revision', 'INTEGER NOT NULL DEFAULT 0');

    await _recoverLegacyRelativePaths(db);

    // v1-v4 的旧 FAILED=4 没有结构化错误分类
    await db.execute(
      'UPDATE transfer_queue SET error_kind=? WHERE state=4 AND error_kind IS NULL',
      [_transferErrorKindUnknown],
    );

    // 旧 PENDING/RUNNING/PAUSED 保守地从 Pending 重启；终态历史在新数值表示中保留原语义
    await db.execute(
      '''UPDATE transfer_queue
         SET state = CASE state
            WHEN 0 THEN ?
            WHEN 1 THEN ?
            WHEN 2 THEN ?
            WHEN 3 THEN ?
            WHEN 4 THEN ?
            WHEN 5 THEN ?
            ELSE state
         END''',
      [
        _transferStatePending,
        _transferStatePending,
        _transferStatePending,
        _transferStateCompleted,
        _transferStateFailed,
        _transferStateCanceled,
      ],
    );

    await db.execute(
        'CREATE INDEX IF NOT EXISTS idx_transfer_state_retry ON transfer_queue(state, next_retry_at)');
    await db.execute(
        'CREATE INDEX IF NOT EXISTS idx_transfer_relative_state ON transfer_queue(relative_path, state)');
  }

  /// 回填旧任务的相对路径；无法安全恢复的活动任务标记为验证失败。
  ///
  /// Flutter 侧迁移阶段拿不到可信同步目录（对齐 Rust mount_root=None 分支），
  /// 因此所有旧活动任务（state 0-2）都无法推导相对路径，统一标记失败。
  Future<void> _recoverLegacyRelativePaths(Database db) async {
    final rows = await db
        .rawQuery('SELECT id, state, local_path FROM transfer_queue');
    for (final row in rows) {
      final taskId = row['id'] as int;
      final legacyState = row['state'] as int? ?? 0;
      // mount_root 缺失 → 推导必然失败；仅活动任务（旧 state 0-2）需标记
      if (legacyState >= 0 && legacyState <= 2) {
        await db.execute(
          '''UPDATE transfer_queue
             SET state=?, error_kind=?, error_message=?
             WHERE id=?''',
          [
            _transferStateFailed,
            _transferErrorKindValidation,
            '旧传输任务无法安全恢复：未配置同步目录',
            taskId,
          ],
        );
      }
    }
  }

  /// 幂等加列：列已存在时跳过（SQLite ALTER TABLE 不支持 IF NOT EXISTS）。
  Future<void> _addColumnIfMissing(
    Database db,
    String table,
    String column,
    String definition,
  ) async {
    final columns = await db.rawQuery('PRAGMA table_info($table)');
    final exists = columns.any((c) => c['name'] == column);
    if (!exists) {
      await db.execute('ALTER TABLE $table ADD COLUMN $column $definition');
    }
  }

  // ===== 迁移用状态常量（对齐 Rust TransferState / TransferErrorKind 数值）=====

  /// TransferState::Pending
  static const int _transferStatePending = 0;

  /// TransferState::Completed
  static const int _transferStateCompleted = 6;

  /// TransferState::Failed
  static const int _transferStateFailed = 7;

  /// TransferState::Canceled
  static const int _transferStateCanceled = 8;

  /// TransferErrorKind::Validation
  static const int _transferErrorKindValidation = 7;

  /// TransferErrorKind::Unknown
  static const int _transferErrorKindUnknown = 11;
}

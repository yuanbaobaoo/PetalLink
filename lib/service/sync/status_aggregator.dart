/// 状态聚合器 —— TaskRunner 快照 + sync_items 计数 → SyncGlobalState。
///
/// 严格对齐 Rust 原版 `src/sync/status_aggregator.rs` + `src/sync/state.rs`：
/// - 一条 SQL 取 7 个计数（total/failed/conflict/uploading/downloading/
///   waiting_network/transfer_failed）
/// - failed_items 按 local_path 字典序 LIMIT 20
/// - completed = total - failed - conflict（saturating）
/// - revision 进程级单调（跨引擎替换仍递增；首快照 revision=1）
/// - publication 互斥锁串行化「版本分配 + 发布」
library;

import 'dart:async';

import 'package:sqflite/sqflite.dart';

import 'package:petal_link/entity/sync_state.dart';
import 'package:petal_link/types/enums.dart';

/// 引擎运行时状态（无 SQLite 持久化来源，只能由引擎运行时注入，
/// 对齐 Rust `RuntimeStatus`）。
class RuntimeStatus {
  /// 被暂停编辑的文件数
  int editing;

  /// 引擎是否正在运行
  bool isRunning;

  /// 上次同步时间（毫秒 epoch）
  int? lastSyncTime;

  /// 是否正在索引云端目录
  bool isIndexing;

  /// 已扫描的文件夹数（索引用）
  int indexingScannedFolders;

  /// 已发现的条目数（索引用）
  int indexingDiscoveredItems;

  /// 是否有目录结构变更
  bool contentChanged;

  /// 当前同步阶段（null = 空闲）
  SyncPhase? syncPhase;

  RuntimeStatus({
    this.editing = 0,
    this.isRunning = false,
    this.lastSyncTime,
    this.isIndexing = false,
    this.indexingScannedFolders = 0,
    this.indexingDiscoveredItems = 0,
    this.contentChanged = false,
    this.syncPhase,
  });

  /// 从完整快照提取运行时字段（对齐 Rust `From<&SyncGlobalState>`）。
  factory RuntimeStatus.from(SyncGlobalState state) {
    return RuntimeStatus(
      editing: state.editing,
      isRunning: state.isRunning,
      lastSyncTime: state.lastSyncTime,
      isIndexing: state.isIndexing,
      indexingScannedFolders: state.indexingScannedFolders,
      indexingDiscoveredItems: state.indexingDiscoveredItems,
      contentChanged: state.contentChanged,
      syncPhase: state.syncPhase,
    );
  }

  /// 拷贝运行时字段到另一实例。
  void copyFrom(RuntimeStatus other) {
    editing = other.editing;
    isRunning = other.isRunning;
    lastSyncTime = other.lastSyncTime;
    isIndexing = other.isIndexing;
    indexingScannedFolders = other.indexingScannedFolders;
    indexingDiscoveredItems = other.indexingDiscoveredItems;
    contentChanged = other.contentChanged;
    syncPhase = other.syncPhase;
  }
}

/// 状态聚合器（对齐 Rust `StatusAggregator`，进程级单例语义）。
///
/// revision 为全局单调计数器：每次 [snapshot] 分配 +1，
/// 与是否广播无关；跨引擎实例共享同一进程级单例时仍保持单调。
class StatusAggregator {
  /// 进程级共享实例（对齐 Rust 全局 STATUS_AGGREGATOR）。
  static final StatusAggregator process = StatusAggregator._();

  StatusAggregator._();

  /// 测试用独立实例（revision 从 0 起）。
  factory StatusAggregator.independent() => StatusAggregator._();

  /// 单调版本计数器
  int _nextRevision = 0;

  /// 发布互斥锁（串行化版本分配 + 发布的 Future 链）
  Future<void> _publication = Future<void>.value();

  /// 在发布屏障内执行 [body]（对齐 Rust `lock_publication`）。
  ///
  /// 同一时刻只有一个发布者能分配 revision 并广播，保证快照全序。
  Future<T> lockPublication<T>(Future<T> Function() body) {
    final completer = Completer<T>();
    _publication = _publication.then((_) async {
      try {
        final result = await body();
        completer.complete(result);
      } catch (e, st) {
        completer.completeError(e, st);
      }
    });
    return completer.future;
  }

  /// 分配下一个单调版本号。
  int nextRevision() => ++_nextRevision;

  /// 聚合 DB 计数与运行时状态，产出权威快照（对齐 Rust `snapshot`）。
  ///
  /// 调用方必须已持有发布锁（经 [lockPublication] 调用）。
  Future<SyncGlobalState> snapshot(Database db, RuntimeStatus runtime) async {
    final rows = await db.rawQuery(
      'SELECT '
      '(SELECT COUNT(*) FROM sync_items) AS total, '
      '(SELECT COUNT(*) FROM sync_items WHERE status = ?) AS failed, '
      '(SELECT COUNT(*) FROM sync_items WHERE status = ?) AS conflict, '
      '(SELECT COUNT(*) FROM transfer_queue WHERE state = ? AND direction = ?) '
      'AS uploading, '
      '(SELECT COUNT(*) FROM transfer_queue WHERE state = ? '
      'AND direction IN (?, ?)) AS downloading, '
      '(SELECT COUNT(*) FROM transfer_queue WHERE state = ?) '
      'AS waiting_network, '
      '(SELECT COUNT(*) FROM transfer_queue WHERE state = ?) '
      'AS transfer_failed',
      [
        SyncItemStatus.failed.code,
        SyncItemStatus.conflict.code,
        TransferState.running.code,
        TransferDirection.upload.code,
        TransferState.running.code,
        TransferDirection.download.code,
        TransferDirection.downloadUpdate.code,
        TransferState.waitingForNetwork.code,
        TransferState.failed.code,
      ],
    );
    final row = rows.first;
    int count(String key) {
      final v = row[key];
      if (v is int) return v;
      return int.tryParse('$v') ?? 0;
    }

    // 失败项详情：按路径字典序排序后截断前 20
    final failedRows = await db.query(
      'sync_items',
      columns: ['local_path', 'error_message'],
      where: 'status = ?',
      whereArgs: [SyncItemStatus.failed.code],
      orderBy: 'local_path ASC',
      limit: SyncGlobalState.maxFailedItems,
    );
    final failedItems = failedRows
        .map((r) => FailedItem(
              relativePath: r['local_path'] as String? ?? '',
              errorMessage: r['error_message'] as String?,
            ))
        .toList();

    final total = count('total');
    final failed = count('failed');
    final conflict = count('conflict');
    final completed = total - failed - conflict;

    return SyncGlobalState(
      revision: nextRevision(),
      total: total,
      completed: completed < 0 ? 0 : completed,
      uploading: count('uploading'),
      downloading: count('downloading'),
      waitingNetwork: count('waiting_network'),
      failed: failed,
      transferFailed: count('transfer_failed'),
      failedItems: failedItems,
      conflict: conflict,
      editing: runtime.editing,
      isRunning: runtime.isRunning,
      lastSyncTime: runtime.lastSyncTime,
      isIndexing: runtime.isIndexing,
      indexingScannedFolders: runtime.indexingScannedFolders,
      indexingDiscoveredItems: runtime.indexingDiscoveredItems,
      contentChanged: runtime.contentChanged,
      syncPhase: runtime.syncPhase,
    );
  }
}

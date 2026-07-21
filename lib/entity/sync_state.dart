import '../types/enums.dart';

/// copyWith 的「保持原值」哨兵（区分「不传」与「显式置 null」）
const Object _keep = Object();

/// 失败项详情（对齐 Rust `FailedItem`，前端失败项弹窗用）
class FailedItem {
  /// 相对路径（取自 sync_items.local_path）
  final String relativePath;

  /// 错误信息
  final String? errorMessage;

  const FailedItem({
    required this.relativePath,
    this.errorMessage,
  });

  /// 从 JSON 构造（snake_case 键）
  factory FailedItem.fromJson(Map<String, dynamic> json) {
    return FailedItem(
      relativePath: json['relative_path'] as String? ?? '',
      errorMessage: json['error_message'] as String?,
    );
  }

  /// 序列化为 JSON（snake_case 键，对齐 Rust serde）
  Map<String, dynamic> toJson() {
    return {
      'relative_path': relativePath,
      'error_message': errorMessage,
    };
  }

  /// 深拷贝并替换指定字段
  FailedItem copyWith({
    String? relativePath,
    Object? errorMessage = _keep,
  }) {
    return FailedItem(
      relativePath: relativePath ?? this.relativePath,
      errorMessage: identical(errorMessage, _keep)
          ? this.errorMessage
          : errorMessage as String?,
    );
  }
}

/// 同步全局状态（对齐 Rust `SyncGlobalState`，供 UI 透传）。
class SyncGlobalState {
  /// 失败项详情列表最大长度（对齐 Rust 注释：最多 20 条）
  static const int maxFailedItems = 20;

  /// 权威快照的进程内单调版本
  final int revision;

  /// 总操作数
  final int total;

  /// 已完成操作数
  final int completed;

  /// 上传中任务数
  final int uploading;

  /// 下载中任务数
  final int downloading;

  /// 因网络不可用而等待恢复的传输任务数（不属于永久失败）
  final int waitingNetwork;

  /// 当前同步失败数
  final int failed;

  /// 传输队列中永久失败的历史任务数（与当前同步失败分开统计）
  final int transferFailed;

  /// 失败项详情（供失败项弹窗，最多 [maxFailedItems] 条）
  final List<FailedItem> failedItems;

  /// 冲突数
  final int conflict;

  /// 被暂停编辑的文件数（F-MOUNT-11）
  final int editing;

  /// 引擎是否正在运行
  final bool isRunning;

  /// 上次同步时间（毫秒 epoch）
  final int? lastSyncTime;

  /// 是否正在索引云端目录
  final bool isIndexing;

  /// 已扫描的文件夹数（索引用）
  final int indexingScannedFolders;

  /// 已发现的文件总数（索引用）
  final int indexingDiscoveredItems;

  /// 是否有目录结构变更（触发前端目录重拉）
  final bool contentChanged;

  /// 当前同步阶段（供前端状态条精确显示；null = 空闲）
  final SyncPhase? syncPhase;

  const SyncGlobalState({
    this.revision = 0,
    this.total = 0,
    this.completed = 0,
    this.uploading = 0,
    this.downloading = 0,
    this.waitingNetwork = 0,
    this.failed = 0,
    this.transferFailed = 0,
    this.failedItems = const [],
    this.conflict = 0,
    this.editing = 0,
    this.isRunning = false,
    this.lastSyncTime,
    this.isIndexing = false,
    this.indexingScannedFolders = 0,
    this.indexingDiscoveredItems = 0,
    this.contentChanged = false,
    this.syncPhase,
  });

  /// 同步完成度 0.0 ~ 1.0（total 为 0 时返回 1.0，对齐 Rust `progress`）
  double get progress {
    if (total == 0) return 1.0;
    return completed / total;
  }

  /// 从 JSON 构造（snake_case 键，对齐 Rust serde 输出）
  factory SyncGlobalState.fromJson(Map<String, dynamic> json) {
    final rawItems = json['failed_items'];
    final items = <FailedItem>[];
    if (rawItems is List) {
      for (final e in rawItems) {
        if (e is Map<String, dynamic>) items.add(FailedItem.fromJson(e));
      }
    }

    return SyncGlobalState(
      revision: _tolerantInt(json['revision']) ?? 0,
      total: _tolerantInt(json['total']) ?? 0,
      completed: _tolerantInt(json['completed']) ?? 0,
      uploading: _tolerantInt(json['uploading']) ?? 0,
      downloading: _tolerantInt(json['downloading']) ?? 0,
      waitingNetwork: _tolerantInt(json['waiting_network']) ?? 0,
      failed: _tolerantInt(json['failed']) ?? 0,
      transferFailed: _tolerantInt(json['transfer_failed']) ?? 0,
      failedItems: items,
      conflict: _tolerantInt(json['conflict']) ?? 0,
      editing: _tolerantInt(json['editing']) ?? 0,
      isRunning: json['is_running'] == true,
      lastSyncTime: _tolerantInt(json['last_sync_time']),
      isIndexing: json['is_indexing'] == true,
      indexingScannedFolders:
          _tolerantInt(json['indexing_scanned_folders']) ?? 0,
      indexingDiscoveredItems:
          _tolerantInt(json['indexing_discovered_items']) ?? 0,
      contentChanged: json['content_changed'] == true,
      syncPhase: SyncPhase.fromWireName(json['sync_phase'] as String?),
    );
  }

  /// 序列化为 JSON（snake_case 键，对齐 Rust serde；
  /// syncPhase 为 null 时不输出，对齐 `skip_serializing_if`）
  Map<String, dynamic> toJson() {
    return {
      'revision': revision,
      'total': total,
      'completed': completed,
      'uploading': uploading,
      'downloading': downloading,
      'waiting_network': waitingNetwork,
      'failed': failed,
      'transfer_failed': transferFailed,
      'failed_items': failedItems.map((e) => e.toJson()).toList(),
      'conflict': conflict,
      'editing': editing,
      'is_running': isRunning,
      'last_sync_time': lastSyncTime,
      'is_indexing': isIndexing,
      'indexing_scanned_folders': indexingScannedFolders,
      'indexing_discovered_items': indexingDiscoveredItems,
      'content_changed': contentChanged,
      if (syncPhase != null) 'sync_phase': syncPhase!.wireName,
    };
  }

  /// 深拷贝并替换指定字段（可空字段传 null 显式清空）
  SyncGlobalState copyWith({
    int? revision,
    int? total,
    int? completed,
    int? uploading,
    int? downloading,
    int? waitingNetwork,
    int? failed,
    int? transferFailed,
    List<FailedItem>? failedItems,
    int? conflict,
    int? editing,
    bool? isRunning,
    Object? lastSyncTime = _keep,
    bool? isIndexing,
    int? indexingScannedFolders,
    int? indexingDiscoveredItems,
    bool? contentChanged,
    Object? syncPhase = _keep,
  }) {
    return SyncGlobalState(
      revision: revision ?? this.revision,
      total: total ?? this.total,
      completed: completed ?? this.completed,
      uploading: uploading ?? this.uploading,
      downloading: downloading ?? this.downloading,
      waitingNetwork: waitingNetwork ?? this.waitingNetwork,
      failed: failed ?? this.failed,
      transferFailed: transferFailed ?? this.transferFailed,
      failedItems: failedItems ?? this.failedItems,
      conflict: conflict ?? this.conflict,
      editing: editing ?? this.editing,
      isRunning: isRunning ?? this.isRunning,
      lastSyncTime: identical(lastSyncTime, _keep)
          ? this.lastSyncTime
          : lastSyncTime as int?,
      isIndexing: isIndexing ?? this.isIndexing,
      indexingScannedFolders:
          indexingScannedFolders ?? this.indexingScannedFolders,
      indexingDiscoveredItems:
          indexingDiscoveredItems ?? this.indexingDiscoveredItems,
      contentChanged: contentChanged ?? this.contentChanged,
      syncPhase: identical(syncPhase, _keep)
          ? this.syncPhase
          : syncPhase as SyncPhase?,
    );
  }
}

/// 容忍解析 int：接受 int / num / String（int 字段兼容 "123"）。
int? _tolerantInt(Object? v) {
  if (v is int) return v;
  if (v is num) return v.toInt();
  if (v is String) return int.tryParse(v.trim());
  return null;
}

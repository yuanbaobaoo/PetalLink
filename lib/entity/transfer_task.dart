import '../types/enums.dart';

/// copyWith 的「保持原值」哨兵（区分「不传」与「显式置 null」）
const Object _keep = Object();

/// 传输任务：transfer_queue 表一行（对齐 Rust `TransferTask` 实体，26 列）。
///
/// 持久化传输任务的完整生命周期上下文：
/// 断点续传、远端核验、网络恢复续跑、退避重试都依赖此模型的持久性。
/// 时间字段统一为毫秒 epoch（int），与 SQLite INTEGER 列一一对应。
class TransferTask {
  /// 自增主键（0 表示尚未入库，insert 时由 SQLite AUTOINCREMENT 分配）
  final int id;

  /// 传输方向
  final TransferDirection direction;

  /// 关联的 SyncItem fileId（可空，手动传输无对应项）
  final String? fileId;

  /// 本地路径（可空）
  final String? localPath;

  /// 文件名
  final String name;

  /// 总大小（字节）
  final int totalSize;

  /// 已传输（字节）
  final int transferred;

  /// 传输状态（九态状态机）
  final TransferState state;

  /// 失败原因
  final String? errorMessage;

  /// 入队时间（毫秒 epoch）
  final int createdAt;

  /// 完成时间（毫秒 epoch）
  final int? finishedAt;

  /// 华为 resume 上传会话标识（v2）
  final String? serverId;

  /// 华为 uploadId（v2）
  final String? uploadId;

  /// 已上传字节偏移（断点续传恢复点，v2）
  final int resumeOffset;

  /// 华为 resume 上传 Location 头返回的会话 URL（v4）。
  ///
  /// 新 API 不再在 body 返回 serverId/uploadId，分片 PUT 必须直接用此 URL。
  final String? sessionUrl;

  /// 相对挂载根的规范 UTF-8 路径（绝不替代 absolute localPath）
  final String? relativePath;

  /// 规划时的云端父目录 fileId
  final String? parentFileId;

  /// 持久化操作类型
  final TransferOperation? operation;

  /// 入队时本地源 mtime 快照（毫秒 epoch）
  final int? sourceMtime;

  /// 入队时本地源大小快照
  final int? sourceSize;

  /// 规划时观察到的云端 editedTime（毫秒 epoch）
  final int? expectedCloudEditedTime;

  /// 已消耗的持久化尝试次数
  final int attemptCount;

  /// 下一次允许重试的时间戳（毫秒 epoch）
  final int? nextRetryAt;

  /// 结构化错误类型
  final TransferErrorKind? errorKind;

  /// 远端结果复核确认的资源 fileId
  final String? remoteResultFileId;

  /// 乐观并发状态版本（CAS）
  final int stateRevision;

  const TransferTask({
    this.id = 0,
    this.direction = TransferDirection.Upload,
    this.fileId,
    this.localPath,
    required this.name,
    this.totalSize = 0,
    this.transferred = 0,
    this.state = TransferState.Pending,
    this.errorMessage,
    required this.createdAt,
    this.finishedAt,
    this.serverId,
    this.uploadId,
    this.resumeOffset = 0,
    this.sessionUrl,
    this.relativePath,
    this.parentFileId,
    this.operation,
    this.sourceMtime,
    this.sourceSize,
    this.expectedCloudEditedTime,
    this.attemptCount = 0,
    this.nextRetryAt,
    this.errorKind,
    this.remoteResultFileId,
    this.stateRevision = 0,
  });

  /// 传输进度（0.0 ~ 1.0；totalSize 为 0 时返回 0.0 避免除零）
  double get progress {
    if (totalSize <= 0) return 0.0;
    return (transferred / totalSize).clamp(0.0, 1.0);
  }

  /// 是否为终态（不再参与调度）
  bool get isTerminal => state.isTerminal;

  /// 是否为活跃态（占用传输槽位）
  bool get isActive => state.isActive;

  /// 校验 `state → to` 是否为合法生命周期转移（对齐 Rust `can_transition`）
  bool canTransitionTo(TransferState to) => state.canTransition(to);

  // ═══════════════════════════════════════════════════════════════════
  // SQLite 行映射
  // ═══════════════════════════════════════════════════════════════════

  /// 从 SQLite 查询结果构造（Map key 为列名，容忍 String 数字）。
  ///
  /// 枚举列未知值按安全默认处理：state→Pending（落库重跑），
  /// direction→Upload，operation/errorKind→null/Unknown。
  factory TransferTask.fromRow(Map<String, dynamic> row) {
    return TransferTask(
      id: _tolerantInt(row['id']) ?? 0,
      direction:
          TransferDirection.fromCode(_tolerantInt(row['direction']) ?? 0) ??
              TransferDirection.Upload,
      fileId: row['file_id'] as String?,
      localPath: row['local_path'] as String?,
      name: row['name'] as String? ?? '',
      totalSize: _tolerantInt(row['total_size']) ?? 0,
      transferred: _tolerantInt(row['transferred']) ?? 0,
      state: TransferState.fromCode(_tolerantInt(row['state']) ?? 0) ??
          TransferState.Pending,
      errorMessage: row['error_message'] as String?,
      createdAt: _tolerantInt(row['created_at']) ?? 0,
      finishedAt: _tolerantInt(row['finished_at']),
      serverId: row['server_id'] as String?,
      uploadId: row['upload_id'] as String?,
      resumeOffset: _tolerantInt(row['resume_offset']) ?? 0,
      sessionUrl: row['session_url'] as String?,
      relativePath: row['relative_path'] as String?,
      parentFileId: row['parent_file_id'] as String?,
      operation: _nullableEnum(row['operation'], TransferOperation.fromCode),
      sourceMtime: _tolerantInt(row['source_mtime']),
      sourceSize: _tolerantInt(row['source_size']),
      expectedCloudEditedTime:
          _tolerantInt(row['expected_cloud_edited_time']),
      attemptCount: _tolerantInt(row['attempt_count']) ?? 0,
      nextRetryAt: _tolerantInt(row['next_retry_at']),
      errorKind: _nullableEnum(row['error_kind'], TransferErrorKind.fromCode),
      remoteResultFileId: row['remote_result_file_id'] as String?,
      stateRevision: _tolerantInt(row['state_revision']) ?? 0,
    );
  }

  /// 转为 SQLite 插入/更新用的行 Map（列名 snake_case）。
  ///
  /// [id] 为 0 时不输出 id 列，交由 AUTOINCREMENT 分配。
  Map<String, dynamic> toRow() {
    return {
      if (id > 0) 'id': id,
      'direction': direction.code,
      'file_id': fileId,
      'local_path': localPath,
      'name': name,
      'total_size': totalSize,
      'transferred': transferred,
      'state': state.code,
      'error_message': errorMessage,
      'created_at': createdAt,
      'finished_at': finishedAt,
      'server_id': serverId,
      'upload_id': uploadId,
      'resume_offset': resumeOffset,
      'session_url': sessionUrl,
      'relative_path': relativePath,
      'parent_file_id': parentFileId,
      'operation': operation?.code,
      'source_mtime': sourceMtime,
      'source_size': sourceSize,
      'expected_cloud_edited_time': expectedCloudEditedTime,
      'attempt_count': attemptCount,
      'next_retry_at': nextRetryAt,
      'error_kind': errorKind?.code,
      'remote_result_file_id': remoteResultFileId,
      'state_revision': stateRevision,
    };
  }

  /// 深拷贝并替换指定字段（可空字段传 null 显式清空）
  TransferTask copyWith({
    int? id,
    TransferDirection? direction,
    Object? fileId = _keep,
    Object? localPath = _keep,
    String? name,
    int? totalSize,
    int? transferred,
    TransferState? state,
    Object? errorMessage = _keep,
    int? createdAt,
    Object? finishedAt = _keep,
    Object? serverId = _keep,
    Object? uploadId = _keep,
    int? resumeOffset,
    Object? sessionUrl = _keep,
    Object? relativePath = _keep,
    Object? parentFileId = _keep,
    Object? operation = _keep,
    Object? sourceMtime = _keep,
    Object? sourceSize = _keep,
    Object? expectedCloudEditedTime = _keep,
    int? attemptCount,
    Object? nextRetryAt = _keep,
    Object? errorKind = _keep,
    Object? remoteResultFileId = _keep,
    int? stateRevision,
  }) {
    return TransferTask(
      id: id ?? this.id,
      direction: direction ?? this.direction,
      fileId: identical(fileId, _keep) ? this.fileId : fileId as String?,
      localPath:
          identical(localPath, _keep) ? this.localPath : localPath as String?,
      name: name ?? this.name,
      totalSize: totalSize ?? this.totalSize,
      transferred: transferred ?? this.transferred,
      state: state ?? this.state,
      errorMessage: identical(errorMessage, _keep)
          ? this.errorMessage
          : errorMessage as String?,
      createdAt: createdAt ?? this.createdAt,
      finishedAt: identical(finishedAt, _keep)
          ? this.finishedAt
          : finishedAt as int?,
      serverId:
          identical(serverId, _keep) ? this.serverId : serverId as String?,
      uploadId:
          identical(uploadId, _keep) ? this.uploadId : uploadId as String?,
      resumeOffset: resumeOffset ?? this.resumeOffset,
      sessionUrl: identical(sessionUrl, _keep)
          ? this.sessionUrl
          : sessionUrl as String?,
      relativePath: identical(relativePath, _keep)
          ? this.relativePath
          : relativePath as String?,
      parentFileId: identical(parentFileId, _keep)
          ? this.parentFileId
          : parentFileId as String?,
      operation: identical(operation, _keep)
          ? this.operation
          : operation as TransferOperation?,
      sourceMtime: identical(sourceMtime, _keep)
          ? this.sourceMtime
          : sourceMtime as int?,
      sourceSize: identical(sourceSize, _keep)
          ? this.sourceSize
          : sourceSize as int?,
      expectedCloudEditedTime: identical(expectedCloudEditedTime, _keep)
          ? this.expectedCloudEditedTime
          : expectedCloudEditedTime as int?,
      attemptCount: attemptCount ?? this.attemptCount,
      nextRetryAt: identical(nextRetryAt, _keep)
          ? this.nextRetryAt
          : nextRetryAt as int?,
      errorKind: identical(errorKind, _keep)
          ? this.errorKind
          : errorKind as TransferErrorKind?,
      remoteResultFileId: identical(remoteResultFileId, _keep)
          ? this.remoteResultFileId
          : remoteResultFileId as String?,
      stateRevision: stateRevision ?? this.stateRevision,
    );
  }

  /// 可空枚举列解析：null 保持 null，未知码返回 null（对齐 Rust Option 语义）
  static T? _nullableEnum<T>(Object? raw, T? Function(int) fromCode) {
    final code = _tolerantInt(raw);
    if (code == null) return null;
    return fromCode(code);
  }
}

/// 容忍解析 int：接受 int / num / String（int 字段兼容 "123"）。
int? _tolerantInt(Object? v) {
  if (v is int) return v;
  if (v is num) return v.toInt();
  if (v is String) return int.tryParse(v.trim());
  return null;
}

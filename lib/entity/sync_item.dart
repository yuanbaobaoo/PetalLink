import '../types/enums.dart';

/// copyWith 的「保持原值」哨兵（区分「不传」与「显式置 null」）
const Object _keep = Object();

/// 新增上传失败的占位 fileId 前缀（对齐 Rust `PENDING_FILE_ID_PREFIX`）。
///
/// 新增文件上传时云端无真实 fileId，失败时用此前缀 + 相对路径生成占位
/// fileId 写入 sync_items，让 retry_failed 能找到失败项。
/// 成功上传后由真实 fileId 覆盖（先清占位行）。
/// planner 据此前缀判断「待上传占位项」→ 重新 Upload，绝不删本地。
const String pendingFileIdPrefix = 'pending:';

/// 同步项：sync_items 表一行（对齐 Rust `SyncItem` 实体，13 列）。
///
/// 本地文件 ↔ 云端文件的映射基线记录，
/// 作为 3-way diff 的第三方参照（本地 × 云端 × DB 基线）。
/// 复合主键：(fileId, localPath)。时间字段统一为毫秒 epoch（int）。
class SyncItem {
  /// 云端文件 ID（主键之一；待上传占位项以 [pendingFileIdPrefix] 开头）
  final String fileId;

  /// 相对挂载根的规范 UTF-8 路径（主键之二）
  final String localPath;

  /// 父目录 fileId
  final String? parentFolderId;

  /// 文件名
  final String name;

  /// 是否文件夹
  final bool isFolder;

  /// 云端大小（字节）
  final int size;

  /// 本地大小（字节，v3，变更检测用）
  final int? localSize;

  /// 本地 SHA256
  final String? sha256;

  /// 本地 mtime（毫秒 epoch）
  final int? localMtime;

  /// 云端 editedTime（毫秒 epoch）
  final int? cloudEditedTime;

  /// 最后成功同步时间（毫秒 epoch）
  final int? lastSyncTime;

  /// 同步状态
  final SyncItemStatus status;

  /// 失败/冲突原因
  final String? errorMessage;

  const SyncItem({
    required this.fileId,
    required this.localPath,
    this.parentFolderId,
    required this.name,
    this.isFolder = false,
    this.size = 0,
    this.localSize,
    this.sha256,
    this.localMtime,
    this.cloudEditedTime,
    this.lastSyncTime,
    this.status = SyncItemStatus.Synced,
    this.errorMessage,
  });

  /// 是否为待上传占位项（新增上传失败，云端尚无真实 fileId）
  bool get isPendingUpload => fileId.startsWith(pendingFileIdPrefix);

  // ═══════════════════════════════════════════════════════════════════
  // SQLite 行映射
  // ═══════════════════════════════════════════════════════════════════

  /// 从 SQLite 查询结果构造（Map key 为列名，容忍 String 数字）。
  ///
  /// status 未知值回退 [SyncItemStatus.Synced]（已同步基线，最安全默认）。
  factory SyncItem.fromRow(Map<String, dynamic> row) {
    return SyncItem(
      fileId: row['file_id'] as String? ?? '',
      localPath: row['local_path'] as String? ?? '',
      parentFolderId: row['parent_folder_id'] as String?,
      name: row['name'] as String? ?? '',
      isFolder: _tolerantInt(row['is_folder']) == 1,
      size: _tolerantInt(row['size']) ?? 0,
      localSize: _tolerantInt(row['local_size']),
      sha256: row['sha256'] as String?,
      localMtime: _tolerantInt(row['local_mtime']),
      cloudEditedTime: _tolerantInt(row['cloud_edited_time']),
      lastSyncTime: _tolerantInt(row['last_sync_time']),
      status: SyncItemStatus.fromCode(_tolerantInt(row['status']) ?? 0) ??
          SyncItemStatus.Synced,
      errorMessage: row['error_message'] as String?,
    );
  }

  /// 转为 SQLite 插入/更新用的行 Map（列名 snake_case）
  Map<String, dynamic> toRow() {
    return {
      'file_id': fileId,
      'local_path': localPath,
      'parent_folder_id': parentFolderId,
      'name': name,
      'is_folder': isFolder ? 1 : 0,
      'size': size,
      'local_size': localSize,
      'sha256': sha256,
      'local_mtime': localMtime,
      'cloud_edited_time': cloudEditedTime,
      'last_sync_time': lastSyncTime,
      'status': status.code,
      'error_message': errorMessage,
    };
  }

  // ═══════════════════════════════════════════════════════════════════
  // JSON 序列化（用于缓存 / API 传输）
  // ═══════════════════════════════════════════════════════════════════

  /// 从 JSON 构造（camelCase 键）
  factory SyncItem.fromJson(Map<String, dynamic> json) {
    return SyncItem(
      fileId: json['fileId'] as String? ?? '',
      localPath: json['localPath'] as String? ?? '',
      parentFolderId: json['parentFolderId'] as String?,
      name: json['name'] as String? ?? '',
      isFolder: json['isFolder'] == true,
      size: _tolerantInt(json['size']) ?? 0,
      localSize: _tolerantInt(json['localSize']),
      sha256: json['sha256'] as String?,
      localMtime: _tolerantInt(json['localMtime']),
      cloudEditedTime: _tolerantInt(json['cloudEditedTime']),
      lastSyncTime: _tolerantInt(json['lastSyncTime']),
      status: SyncItemStatus.fromCode(_tolerantInt(json['status']) ?? 0) ??
          SyncItemStatus.Synced,
      errorMessage: json['errorMessage'] as String?,
    );
  }

  /// 序列化为 JSON（camelCase 键）
  Map<String, dynamic> toJson() {
    return {
      'fileId': fileId,
      'localPath': localPath,
      'parentFolderId': parentFolderId,
      'name': name,
      'isFolder': isFolder,
      'size': size,
      'localSize': localSize,
      'sha256': sha256,
      'localMtime': localMtime,
      'cloudEditedTime': cloudEditedTime,
      'lastSyncTime': lastSyncTime,
      'status': status.code,
      'errorMessage': errorMessage,
    };
  }

  /// 深拷贝并替换指定字段（可空字段传 null 显式清空）
  SyncItem copyWith({
    String? fileId,
    String? localPath,
    Object? parentFolderId = _keep,
    String? name,
    bool? isFolder,
    int? size,
    Object? localSize = _keep,
    Object? sha256 = _keep,
    Object? localMtime = _keep,
    Object? cloudEditedTime = _keep,
    Object? lastSyncTime = _keep,
    SyncItemStatus? status,
    Object? errorMessage = _keep,
  }) {
    return SyncItem(
      fileId: fileId ?? this.fileId,
      localPath: localPath ?? this.localPath,
      parentFolderId: identical(parentFolderId, _keep)
          ? this.parentFolderId
          : parentFolderId as String?,
      name: name ?? this.name,
      isFolder: isFolder ?? this.isFolder,
      size: size ?? this.size,
      localSize:
          identical(localSize, _keep) ? this.localSize : localSize as int?,
      sha256: identical(sha256, _keep) ? this.sha256 : sha256 as String?,
      localMtime:
          identical(localMtime, _keep) ? this.localMtime : localMtime as int?,
      cloudEditedTime: identical(cloudEditedTime, _keep)
          ? this.cloudEditedTime
          : cloudEditedTime as int?,
      lastSyncTime: identical(lastSyncTime, _keep)
          ? this.lastSyncTime
          : lastSyncTime as int?,
      status: status ?? this.status,
      errorMessage: identical(errorMessage, _keep)
          ? this.errorMessage
          : errorMessage as String?,
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

/// Drive 数据模型 —— DriveFile / FileCategory / DriveAbout / FileListResult。
///
/// 严格对齐 Rust 原版 `src/drive/models.rs`。
library;

/// copyWith 的「保持原值」哨兵（区分「不传」与「显式置 null」）
const Object _keep = Object();

/// 文件分类（对齐 Rust `FileCategory`）。
///
/// 华为的 category 字段对所有资源都返回 "drive#file"（无类型信息），
/// 真正的类型在 mimeType。文件夹：`application/vnd.huawei-apps.folder`。
enum FileCategory {
  /// 文件夹
  folder,

  /// 音频
  audio,

  /// 视频
  video,

  /// 图片
  image,

  /// 文档
  document,

  /// 安装包
  package,

  /// 压缩包
  archive,

  /// 可执行文件
  executable,

  /// 未分类
  unknown;

  /// 基于 mimeType 判断文件分类（对齐 Rust `FileCategory::from_mime_type`）。
  static FileCategory fromMimeType(String? mimeType) {
    final m = mimeType?.toLowerCase();
    if (m == null) return FileCategory.unknown;

    // 文件夹（华为/Google Drive 兼容的四种写法）
    if (m == 'application/vnd.huawei-apps.folder' ||
        m == 'application/vnd.huawei-app.folder' ||
        m == 'application/vnd.google-apps.folder' ||
        m == 'application/x-folder') {
      return FileCategory.folder;
    }
    if (m.startsWith('image/')) return FileCategory.image;
    if (m.startsWith('video/')) return FileCategory.video;
    if (m.startsWith('audio/')) return FileCategory.audio;
    // 文档类
    if (m.startsWith('text/') ||
        m.contains('pdf') ||
        m.contains('word') ||
        m.contains('msword') ||
        m.contains('officedocument.wordprocessing') ||
        m.contains('spreadsheet') ||
        m.contains('excel') ||
        m.contains('presentation') ||
        m.contains('powerpoint')) {
      return FileCategory.document;
    }
    // 压缩包
    if (m.contains('zip') ||
        m.contains('rar') ||
        m.contains('7z') ||
        m.contains('tar') ||
        m.contains('gzip') ||
        m.contains('x-tar')) {
      return FileCategory.archive;
    }
    // 安装包
    if (m.contains('apk') ||
        m.contains('dmg') ||
        m.contains('pkg') ||
        m.contains('debian') ||
        m.contains('rpm')) {
      return FileCategory.package;
    }
    // 可执行
    if (m.contains('executable') ||
        m.contains('x-msdownload') ||
        m.endsWith('x-mach-binary')) {
      return FileCategory.executable;
    }
    return FileCategory.unknown;
  }
}

/// Drive 文件（对应华为云盘 File 资源，对齐 Rust `DriveFile`）。
class DriveFile {
  /// 云端文件 ID
  final String id;

  /// 文件名（JSON 键为 fileName，兼容 name）
  final String name;

  /// 文件分类（由 mimeType 派生）
  final FileCategory category;

  /// 文件大小（字节）
  final int size;

  /// 云端父目录 ID 列表
  final List<String>? parentFolder;

  /// 文件描述
  final String? description;

  /// 创建时间（UTC）
  final DateTime? createdTime;

  /// 最后修改时间（UTC）
  final DateTime? editedTime;

  /// MIME 类型
  final String? mimeType;

  /// 云端内容 hash（md5/sha256，字段名兼容多种）。
  ///
  /// 若华为返回则为内容指纹，用于精确变更检测；为 null 时降级用 editedTime。
  final String? contentHash;

  /// 缩略图 URL
  final String? thumbnailLink;

  const DriveFile({
    required this.id,
    required this.name,
    this.category = FileCategory.unknown,
    this.size = 0,
    this.parentFolder,
    this.description,
    this.createdTime,
    this.editedTime,
    this.mimeType,
    this.contentHash,
    this.thumbnailLink,
  });

  /// 是否文件夹（对齐 Rust `is_folder`）
  bool get isFolder => category == FileCategory.folder;

  /// 云端父目录 ID（取 [parentFolder] 第一个；无父目录时为 null）
  String? get parentId =>
      parentFolder != null && parentFolder!.isNotEmpty
          ? parentFolder!.first
          : null;

  /// 从华为 JSON 响应构造（对齐 Rust `from_json`）。
  ///
  /// 兼容点：
  /// - 文件名：华为用 fileName，标准用 name
  /// - size：容忍 int / num / String
  /// - contentHash：兼容 sha256/md5/md5Checksum/fileSha256/hash/contentHash 六种别名
  factory DriveFile.fromJson(Map<String, dynamic> json) {
    final id = json['id'];
    // 华为用 fileName，标准用 name
    final rawName = json['fileName'] is String
        ? json['fileName'] as String
        : json['name'];
    final mimeType = json['mimeType'];

    // 父目录列表（过滤非字符串元素）
    final rawParent = json['parentFolder'];
    final List<String>? parentFolder;
    if (rawParent is List) {
      parentFolder =
          rawParent.whereType<String>().map((s) => s.toString()).toList();
    } else {
      parentFolder = null;
    }

    return DriveFile(
      id: id is String ? id : id?.toString() ?? '',
      name: rawName is String ? rawName : '',
      category: FileCategory.fromMimeType(mimeType as String?),
      size: _tolerantInt(json['size']) ?? 0,
      parentFolder: parentFolder,
      description: json['description'] as String?,
      createdTime: _parseTime(json['createdTime']),
      editedTime: _parseTime(json['editedTime']),
      mimeType: mimeType is String ? mimeType : null,
      // 内容 hash：兼容华为多种字段名
      contentHash: _pickFirst(json, const [
        'sha256',
        'md5',
        'md5Checksum',
        'fileSha256',
        'hash',
        'contentHash',
      ]),
      thumbnailLink: json['thumbnailLink'] as String?,
    );
  }

  /// 严格解析：id 非字符串时返回 null（对齐 Rust `from_json` 的 Option 语义，
  /// 供 [FileListResult.fromJson] 过滤无效条目）
  static DriveFile? tryFromJson(Map<String, dynamic> json) {
    if (json['id'] is! String) return null;
    return DriveFile.fromJson(json);
  }

  /// 序列化为华为 JSON（用于云端树缓存持久化，对齐 Rust `to_json`）：
  /// size 仅在大于 0 时输出；contentHash 统一序列化为 sha256。
  Map<String, dynamic> toJson() {
    return {
      'id': id,
      'fileName': name,
      if (size > 0) 'size': size,
      if (parentFolder != null) 'parentFolder': parentFolder,
      if (description != null) 'description': description,
      if (createdTime != null)
        'createdTime': createdTime!.toUtc().toIso8601String(),
      if (editedTime != null)
        'editedTime': editedTime!.toUtc().toIso8601String(),
      if (mimeType != null) 'mimeType': mimeType,
      if (contentHash != null) 'sha256': contentHash,
    };
  }

  /// 深拷贝并替换指定字段（可空字段传 null 显式清空）
  DriveFile copyWith({
    String? id,
    String? name,
    FileCategory? category,
    int? size,
    Object? parentFolder = _keep,
    Object? description = _keep,
    Object? createdTime = _keep,
    Object? editedTime = _keep,
    Object? mimeType = _keep,
    Object? contentHash = _keep,
    Object? thumbnailLink = _keep,
  }) {
    return DriveFile(
      id: id ?? this.id,
      name: name ?? this.name,
      category: category ?? this.category,
      size: size ?? this.size,
      parentFolder: identical(parentFolder, _keep)
          ? this.parentFolder
          : parentFolder as List<String>?,
      description: identical(description, _keep)
          ? this.description
          : description as String?,
      createdTime: identical(createdTime, _keep)
          ? this.createdTime
          : createdTime as DateTime?,
      editedTime: identical(editedTime, _keep)
          ? this.editedTime
          : editedTime as DateTime?,
      mimeType:
          identical(mimeType, _keep) ? this.mimeType : mimeType as String?,
      contentHash: identical(contentHash, _keep)
          ? this.contentHash
          : contentHash as String?,
      thumbnailLink: identical(thumbnailLink, _keep)
          ? this.thumbnailLink
          : thumbnailLink as String?,
    );
  }

  /// 解析 ISO8601 时间字符串（对齐 Rust `parse_time`）
  static DateTime? _parseTime(Object? v) {
    if (v is! String || v.isEmpty) return null;
    return DateTime.tryParse(v)?.toUtc();
  }

  /// 按别名顺序取首个字符串值（对齐 Rust content_hash 别名链）
  static String? _pickFirst(Map<String, dynamic> json, List<String> keys) {
    for (final k in keys) {
      final v = json[k];
      if (v is String) return v;
    }
    return null;
  }
}

/// Drive 配额信息（对齐 Rust `DriveAbout`）。
class DriveAbout {
  /// 总容量（字节）
  final int userCapacity;

  /// 已用空间（字节）
  final int usedSpace;

  /// 用户展示名（嵌套 user.displayName）
  final String? userDisplayName;

  const DriveAbout({
    this.userCapacity = 0,
    this.usedSpace = 0,
    this.userDisplayName,
  });

  /// 剩余空间（对齐 Rust `remaining_space`）
  int get remainingSpace => userCapacity - usedSpace;

  /// 是否能容纳 n 字节（对齐 Rust `can_fit`）
  bool canFit(int n) => remainingSpace >= n;

  /// 从华为 JSON 构造（对齐 Rust `from_json`）。
  ///
  /// 配额字段在 `storageQuota` 子对象下（缺失时回退顶层），
  /// 且华为返回为 String（容忍解析）；用户名在 `user.displayName` 嵌套。
  factory DriveAbout.fromJson(Map<String, dynamic> json) {
    final quota = json['storageQuota'];
    final quotaMap = quota is Map<String, dynamic> ? quota : json;

    final user = json['user'];
    final displayName =
        user is Map<String, dynamic> ? user['displayName'] : null;

    return DriveAbout(
      userCapacity: _tolerantInt(quotaMap['userCapacity']) ?? 0,
      usedSpace: _tolerantInt(quotaMap['usedSpace']) ?? 0,
      userDisplayName: displayName is String ? displayName : null,
    );
  }

  /// 序列化为 JSON（snake_case 键，对齐 Rust serde）
  Map<String, dynamic> toJson() {
    return {
      'user_capacity': userCapacity,
      'used_space': usedSpace,
      'user_display_name': userDisplayName,
    };
  }

  /// 深拷贝并替换指定字段
  DriveAbout copyWith({
    int? userCapacity,
    int? usedSpace,
    Object? userDisplayName = _keep,
  }) {
    return DriveAbout(
      userCapacity: userCapacity ?? this.userCapacity,
      usedSpace: usedSpace ?? this.usedSpace,
      userDisplayName: identical(userDisplayName, _keep)
          ? this.userDisplayName
          : userDisplayName as String?,
    );
  }
}

/// 文件列表结果（对齐 Rust `FileListResult`）。
class FileListResult {
  /// 文件列表（已过滤无 id 的无效条目）
  final List<DriveFile> files;

  /// 下一页游标（null 或空串表示没有下一页）
  final String? nextCursor;

  const FileListResult({
    this.files = const [],
    this.nextCursor,
  });

  /// 是否还有下一页（对齐 Rust `has_next`）
  bool get hasNext => nextCursor != null && nextCursor!.isNotEmpty;

  /// 从华为 list 响应构造（对齐 Rust `from_json`）。
  ///
  /// 游标兼容 nextCursor / cursor 两种键；无效文件条目（缺 id）被跳过。
  factory FileListResult.fromJson(Map<String, dynamic> json) {
    final rawFiles = json['files'];
    final files = <DriveFile>[];
    if (rawFiles is List) {
      for (final e in rawFiles) {
        if (e is Map<String, dynamic>) {
          final file = DriveFile.tryFromJson(e);
          if (file != null) files.add(file);
        }
      }
    }

    String? cursor;
    for (final key in const ['nextCursor', 'cursor']) {
      final v = json[key];
      if (v is String && v.isNotEmpty) {
        cursor = v;
        break;
      }
    }

    return FileListResult(files: files, nextCursor: cursor);
  }

  /// 序列化为 JSON（snake_case 键，对齐 Rust serde）
  Map<String, dynamic> toJson() {
    return {
      'files': files.map((f) => f.toJson()).toList(),
      'next_cursor': nextCursor,
    };
  }
}

/// 容忍解析 int：接受 int / num / String（华为配额与大小字段可能返回 String）。
/// 对齐 Rust `tolerant_parse_int`。
int? _tolerantInt(Object? v) {
  if (v is int) return v;
  if (v is num) return v.toInt();
  if (v is String) return int.tryParse(v.trim());
  return null;
}

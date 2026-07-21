/// 全局枚举定义。
///
/// 持久化枚举（[TransferState] / [TransferDirection] / [TransferOperation] /
/// [TransferErrorKind] / [SyncItemStatus]）的数值协议严格对齐 Rust 原版
/// `src/sync/transfer_state.rs` 与 `src/data/repository.rs`，
/// 与 SQLite transfer_queue / sync_items 表的 INTEGER 列一一对应。
library;

/// 传输任务生命周期九态状态机（对齐 Rust `TransferState`，持久化数值 0-8）。
///
/// 每次状态变更必须通过 [canTransition] 合法转移表校验，
/// 并由 CAS（state_revision）乐观锁落库。
/// [Completed] 与 [Canceled] 为终态，无任何出边（[Failed] 可重试，非终态出边受限）。
enum TransferState {
  /// 等待调度
  Pending(0),

  /// 正在传输（上传 / 下载）
  Running(1),

  /// 等待网络恢复
  WaitingForNetwork(2),

  /// 退避重试（等待 next_retry_at 到来）
  BackingOff(3),

  /// 远端校验（传输完成后核验远端结果）
  VerifyingRemote(4),

  /// 需重新开始（不能原样重试，需回 planner 重新规划）
  RestartRequired(5),

  /// 已完成（终态）
  Completed(6),

  /// 永久失败（终态，可经 Pending / RestartRequired 重试）
  Failed(7),

  /// 已取消（终态）
  Canceled(8);

  /// 持久化数值（transfer_queue.state 列）
  final int code;

  const TransferState(this.code);

  /// 从持久化数值解析；未知值返回 null（对齐 Rust TryFrom 的 InvalidStoredValue 语义）
  static TransferState? fromCode(int code) {
    for (final s in values) {
      if (s.code == code) return s;
    }
    return null;
  }

  /// 合法转移边表（对齐 Rust `can_transition` 的 matches! 表）
  static const Set<(TransferState, TransferState)> _edges = {
    // Pending → Running / WaitingForNetwork / RestartRequired / Failed / Canceled
    (Pending, Running),
    (Pending, WaitingForNetwork),
    (Pending, RestartRequired),
    (Pending, Failed),
    (Pending, Canceled),
    // Running → WaitingForNetwork / BackingOff / VerifyingRemote /
    //           RestartRequired / Completed / Failed / Canceled
    (Running, WaitingForNetwork),
    (Running, BackingOff),
    (Running, VerifyingRemote),
    (Running, RestartRequired),
    (Running, Completed),
    (Running, Failed),
    (Running, Canceled),
    // WaitingForNetwork → Running / RestartRequired / Failed / Canceled
    (WaitingForNetwork, Running),
    (WaitingForNetwork, RestartRequired),
    (WaitingForNetwork, Failed),
    (WaitingForNetwork, Canceled),
    // BackingOff → Running / RestartRequired / Failed / Canceled
    (BackingOff, Running),
    (BackingOff, RestartRequired),
    (BackingOff, Failed),
    (BackingOff, Canceled),
    // VerifyingRemote → Running / WaitingForNetwork / BackingOff /
    //                   RestartRequired / Completed / Failed / Canceled
    (VerifyingRemote, Running),
    (VerifyingRemote, WaitingForNetwork),
    (VerifyingRemote, BackingOff),
    (VerifyingRemote, RestartRequired),
    (VerifyingRemote, Completed),
    (VerifyingRemote, Failed),
    (VerifyingRemote, Canceled),
    // RestartRequired → Pending / VerifyingRemote / Failed / Canceled
    (RestartRequired, Pending),
    (RestartRequired, VerifyingRemote),
    (RestartRequired, Failed),
    (RestartRequired, Canceled),
    // Failed → Pending / RestartRequired / Canceled
    (Failed, Pending),
    (Failed, RestartRequired),
    (Failed, Canceled),
    // Completed / Canceled：终态，无出边
  };

  /// 判断 `this → to` 是否为合法生命周期转移（对齐 Rust `can_transition`）
  bool canTransition(TransferState to) => _edges.contains((this, to));

  /// 是否为终态（不再参与调度；Failed 虽可重试但仍计入终态统计，对齐旧 UI 语义）
  bool get isTerminal =>
      this == Completed || this == Failed || this == Canceled;

  /// 是否为活跃态（占用传输槽位）
  bool get isActive => this == Running || this == VerifyingRemote;

  /// 中文展示标签
  String get displayName => switch (this) {
        Pending => '等待中',
        Running => '传输中',
        WaitingForNetwork => '等待网络',
        BackingOff => '退避重试',
        VerifyingRemote => '远端校验',
        RestartRequired => '需重新开始',
        Completed => '已完成',
        Failed => '失败',
        Canceled => '已取消',
      };
}

/// 传输方向（对齐 Rust `transfer_direction` 常量，持久化数值 0-3）
enum TransferDirection {
  /// 上传到云端
  Upload(0),

  /// 首次从云端下载
  Download(1),

  /// 删除目标资源
  Delete(2),

  /// 云端新版本覆盖本地已有文件（语义为「更新」，与 Download 共享下载执行路径）
  DownloadUpdate(3);

  /// 持久化数值（transfer_queue.direction 列）
  final int code;

  const TransferDirection(this.code);

  /// 从持久化数值解析；未知值返回 null
  static TransferDirection? fromCode(int code) {
    for (final d in values) {
      if (d.code == code) return d;
    }
    return null;
  }

  /// 是否为下载类方向（Download / DownloadUpdate 共享下载执行路径）
  bool get isDownload => this == Download || this == DownloadUpdate;

  /// 中文展示标签
  String get displayName => switch (this) {
        Upload => '上传',
        Download => '下载',
        Delete => '删除',
        DownloadUpdate => '更新下载',
      };
}

/// 持久传输任务代表的文件操作（对齐 Rust `TransferOperation`，持久化数值 0-7）
enum TransferOperation {
  /// 新建上传
  Create(0),

  /// 更新上传（覆盖云端已有文件）
  Update(1),

  /// 首次下载
  Download(2),

  /// 更新下载（覆盖本地已有文件）
  DownloadUpdate(3),

  /// 删除
  Delete(4),

  /// 移动
  Move(5),

  /// 重命名
  Rename(6),

  /// 新建文件夹
  CreateFolder(7);

  /// 持久化数值（transfer_queue.operation 列）
  final int code;

  const TransferOperation(this.code);

  /// 从持久化数值解析；未知值返回 null
  static TransferOperation? fromCode(int code) {
    for (final op in values) {
      if (op.code == code) return op;
    }
    return null;
  }
}

/// 可持久的结构化传输失败类型（对齐 Rust `TransferErrorKind`，持久化数值 0-11）
enum TransferErrorKind {
  /// 网络连接失败
  Network(0),

  /// 超时
  Timeout(1),

  /// 认证失败（401 刷新后仍失败）
  Auth(2),

  /// 服务端限流（429 / Retry-After）
  RateLimit(3),

  /// 服务端错误（5xx）
  Server(4),

  /// 云盘配额不足
  Quota(5),

  /// 权限不足
  Permission(6),

  /// 参数校验失败（本地可判定，不应重试）
  Validation(7),

  /// 断点上传会话已失效
  SessionExpired(8),

  /// 远端结果不确定（写入可能已到达服务端，需复核）
  RemoteAmbiguous(9),

  /// 本地源已变更（入队快照失效）
  LocalChanged(10),

  /// 未分类
  Unknown(11);

  /// 持久化数值（transfer_queue.error_kind 列）
  final int code;

  const TransferErrorKind(this.code);

  /// 从持久化数值解析；未知值返回 null
  static TransferErrorKind? fromCode(int code) {
    for (final k in values) {
      if (k.code == code) return k;
    }
    return null;
  }
}

/// 同步项状态（对齐 Rust `sync_status` 常量，sync_items.status 列）
///
/// 注意：持久化数值不连续（6 空缺，[Deleted] = 7）。
enum SyncItemStatus {
  /// 已完成双向同步
  Synced(0),

  /// 仅云端存在
  CloudOnly(1),

  /// 仅本地存在
  LocalOnly(2),

  /// 正在同步
  Syncing(3),

  /// 最近同步失败
  Failed(4),

  /// 本地与云端发生冲突
  Conflict(5),

  /// 用户已主动删除（tombstone：防云端重建）
  Deleted(7);

  /// 持久化数值（sync_items.status 列）
  final int code;

  const SyncItemStatus(this.code);

  /// 从持久化数值解析；未知值返回 null
  static SyncItemStatus? fromCode(int code) {
    for (final s in values) {
      if (s.code == code) return s;
    }
    return null;
  }
}

/// 当前同步阶段（对齐 Rust `SyncGlobalState.sync_phase` 的字符串协议）。
///
/// 供前端状态条精确显示；null 表示空闲。
enum SyncPhase {
  /// 启动时全量索引
  IndexingStartup('indexing-startup'),

  /// 手动触发全量索引
  IndexingManual('indexing-manual'),

  /// 定时自动全量索引
  IndexingAutoFull('indexing-auto-full'),

  /// 查询云端增量变更
  QueryingChanges('querying-changes'),

  /// 自动增量同步
  SyncingAutoIncremental('syncing-auto-incremental'),

  /// 本地变更触发的同步
  SyncingLocal('syncing-local'),

  /// 手动触发同步
  SyncingManual('syncing-manual'),

  /// 失败重试同步
  SyncingRetry('syncing-retry'),

  /// 启动时恢复同步
  SyncingStartup('syncing-startup');

  /// 线上字符串值（kebab-case，对齐 Rust sync_phase 协议）
  final String wireName;

  const SyncPhase(this.wireName);

  /// 从线上字符串解析；未知值返回 null
  static SyncPhase? fromWireName(String? name) {
    if (name == null) return null;
    for (final p in values) {
      if (p.wireName == name) return p;
    }
    return null;
  }

  /// 是否为索引阶段
  bool get isIndexing =>
      this == IndexingStartup ||
      this == IndexingManual ||
      this == IndexingAutoFull;
}

/// 全局同步 UI 状态（宏观阶段，仅 UI 层使用，不持久化）
enum SyncStatus {
  /// 空闲
  Idle,

  /// 扫描中
  Scanning,

  /// 同步中
  Syncing,

  /// 离线
  Offline,

  /// 错误
  Error,
}

/// 认证状态（OAuth 流程阶段，仅 UI 层使用）
enum AuthStatus {
  /// 初始化（未开始认证）
  Init,

  /// 正在授权（浏览器 / OAuth 流程进行中）
  Authorizing,

  /// 已授权（持有有效 token）
  Authorized,

  /// 未授权（token 无效或已登出）
  Unauthorized,

  /// 认证出错
  Error,
}

/// 应用页面路由
enum AppPage {
  /// 登录页
  Login,

  /// 文件浏览页
  Files,

  /// 设置页
  Settings,

  /// 日志查看页
  Logs,

  /// 更新页
  Update,
}

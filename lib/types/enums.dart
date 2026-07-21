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
/// [completed] 与 [canceled] 为终态，无任何出边（[failed] 可重试，非终态出边受限）。
enum TransferState {
  /// 等待调度
  pending(0),

  /// 正在传输（上传 / 下载）
  running(1),

  /// 等待网络恢复
  waitingForNetwork(2),

  /// 退避重试（等待 next_retry_at 到来）
  backingOff(3),

  /// 远端校验（传输完成后核验远端结果）
  verifyingRemote(4),

  /// 需重新开始（不能原样重试，需回 planner 重新规划）
  restartRequired(5),

  /// 已完成（终态）
  completed(6),

  /// 永久失败（终态，可经 pending / restartRequired 重试）
  failed(7),

  /// 已取消（终态）
  canceled(8);

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
    // pending → running / waitingForNetwork / restartRequired / failed / canceled
    (pending, running),
    (pending, waitingForNetwork),
    (pending, restartRequired),
    (pending, failed),
    (pending, canceled),
    // running → waitingForNetwork / backingOff / verifyingRemote /
    //           restartRequired / completed / failed / canceled
    (running, waitingForNetwork),
    (running, backingOff),
    (running, verifyingRemote),
    (running, restartRequired),
    (running, completed),
    (running, failed),
    (running, canceled),
    // waitingForNetwork → running / restartRequired / failed / canceled
    (waitingForNetwork, running),
    (waitingForNetwork, restartRequired),
    (waitingForNetwork, failed),
    (waitingForNetwork, canceled),
    // backingOff → running / restartRequired / failed / canceled
    (backingOff, running),
    (backingOff, restartRequired),
    (backingOff, failed),
    (backingOff, canceled),
    // verifyingRemote → running / waitingForNetwork / backingOff /
    //                   restartRequired / completed / failed / canceled
    (verifyingRemote, running),
    (verifyingRemote, waitingForNetwork),
    (verifyingRemote, backingOff),
    (verifyingRemote, restartRequired),
    (verifyingRemote, completed),
    (verifyingRemote, failed),
    (verifyingRemote, canceled),
    // restartRequired → pending / verifyingRemote / failed / canceled
    (restartRequired, pending),
    (restartRequired, verifyingRemote),
    (restartRequired, failed),
    (restartRequired, canceled),
    // failed → pending / restartRequired / canceled
    (failed, pending),
    (failed, restartRequired),
    (failed, canceled),
    // completed / canceled：终态，无出边
  };

  /// 判断 `this → to` 是否为合法生命周期转移（对齐 Rust `can_transition`）
  bool canTransition(TransferState to) => _edges.contains((this, to));

  /// 是否为终态（不再参与调度；failed 虽可重试但仍计入终态统计，对齐旧 UI 语义）
  bool get isTerminal =>
      this == completed || this == failed || this == canceled;

  /// 是否为活跃态（占用传输槽位）
  bool get isActive => this == running || this == verifyingRemote;

  /// 中文展示标签
  String get displayName => switch (this) {
        pending => '等待中',
        running => '传输中',
        waitingForNetwork => '等待网络',
        backingOff => '退避重试',
        verifyingRemote => '远端校验',
        restartRequired => '需重新开始',
        completed => '已完成',
        failed => '失败',
        canceled => '已取消',
      };
}

/// 传输方向（对齐 Rust `transfer_direction` 常量，持久化数值 0-3）
enum TransferDirection {
  /// 上传到云端
  upload(0),

  /// 首次从云端下载
  download(1),

  /// 删除目标资源
  delete(2),

  /// 云端新版本覆盖本地已有文件（语义为「更新」，与 download 共享下载执行路径）
  downloadUpdate(3);

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

  /// 是否为下载类方向（download / downloadUpdate 共享下载执行路径）
  bool get isDownload => this == download || this == downloadUpdate;

  /// 中文展示标签
  String get displayName => switch (this) {
        upload => '上传',
        download => '下载',
        delete => '删除',
        downloadUpdate => '更新下载',
      };
}

/// 持久传输任务代表的文件操作（对齐 Rust `TransferOperation`，持久化数值 0-7）
enum TransferOperation {
  /// 新建上传
  create(0),

  /// 更新上传（覆盖云端已有文件）
  update(1),

  /// 首次下载
  download(2),

  /// 更新下载（覆盖本地已有文件）
  downloadUpdate(3),

  /// 删除
  delete(4),

  /// 移动
  move(5),

  /// 重命名
  rename(6),

  /// 新建文件夹
  createFolder(7);

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
  network(0),

  /// 超时
  timeout(1),

  /// 认证失败（401 刷新后仍失败）
  auth(2),

  /// 服务端限流（429 / Retry-After）
  rateLimit(3),

  /// 服务端错误（5xx）
  server(4),

  /// 云盘配额不足
  quota(5),

  /// 权限不足
  permission(6),

  /// 参数校验失败（本地可判定，不应重试）
  validation(7),

  /// 断点上传会话已失效
  sessionExpired(8),

  /// 远端结果不确定（写入可能已到达服务端，需复核）
  remoteAmbiguous(9),

  /// 本地源已变更（入队快照失效）
  localChanged(10),

  /// 未分类
  unknown(11);

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
/// 注意：持久化数值不连续（6 空缺，[deleted] = 7）。
enum SyncItemStatus {
  /// 已完成双向同步
  synced(0),

  /// 仅云端存在
  cloudOnly(1),

  /// 仅本地存在
  localOnly(2),

  /// 正在同步
  syncing(3),

  /// 最近同步失败
  failed(4),

  /// 本地与云端发生冲突
  conflict(5),

  /// 用户已主动删除（tombstone：防云端重建）
  deleted(7);

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
  indexingStartup('indexing-startup'),

  /// 手动触发全量索引
  indexingManual('indexing-manual'),

  /// 定时自动全量索引
  indexingAutoFull('indexing-auto-full'),

  /// 查询云端增量变更
  queryingChanges('querying-changes'),

  /// 自动增量同步
  syncingAutoIncremental('syncing-auto-incremental'),

  /// 本地变更触发的同步
  syncingLocal('syncing-local'),

  /// 手动触发同步
  syncingManual('syncing-manual'),

  /// 失败重试同步
  syncingRetry('syncing-retry'),

  /// 启动时恢复同步
  syncingStartup('syncing-startup');

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
      this == indexingStartup ||
      this == indexingManual ||
      this == indexingAutoFull;
}

/// 全局同步 UI 状态（宏观阶段，仅 UI 层使用，不持久化）
enum SyncStatus {
  /// 空闲
  idle,

  /// 扫描中
  scanning,

  /// 同步中
  syncing,

  /// 离线
  offline,

  /// 错误
  error,
}

/// 认证状态（OAuth 流程阶段，仅 UI 层使用）
enum AuthStatus {
  /// 初始化（未开始认证）
  init,

  /// 正在授权（浏览器 / OAuth 流程进行中）
  authorizing,

  /// 已授权（持有有效 token）
  authorized,

  /// 未授权（token 无效或已登出）
  unauthorized,

  /// 认证出错
  error,
}

/// 应用页面路由
enum AppPage {
  /// 登录页
  login,

  /// 文件浏览页
  files,

  /// 设置页
  settings,

  /// 日志查看页
  logs,

  /// 更新页
  update,
}

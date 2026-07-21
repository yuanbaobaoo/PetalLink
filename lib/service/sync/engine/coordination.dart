/// 引擎协调原语 —— 周期请求位 / 周期协调器 / 活动追踪器。
///
/// 严格对齐 Rust 原版 `src/sync/engine/coordination.rs`：
/// - CycleRequest 7 个触发位，sticky 位或合并，单调序列号
/// - CycleCoordinator 唯一 owner drain：正在跑时新触发合并进 pending，
///   不丢弃也不排队等待执行；失败记录 (first, last, msg) 区间（上限 128）
/// - ActivityTracker 活动门：普通/排他路径租约，close 后拒绝新活动，
///   waitIdle 等待全部登记活动释放
library;

import 'dart:async';

import 'package:petal_link/core/error/app_error.dart';

/// 周期请求位（对齐 Rust `CycleRequest`）。
class CycleRequest {
  /// 本地重扫
  static const int localRescan = 1 << 0;

  /// 云端增量
  static const int cloudIncremental = 1 << 1;

  /// 云端全量
  static const int cloudFull = 1 << 2;

  /// 在线恢复
  static const int onlineRecovery = 1 << 3;

  /// 启动恢复
  static const int startup = 1 << 4;

  /// 全局重试
  static const int retry = 1 << 5;

  /// 重规划
  static const int replan = 1 << 6;

  /// 位掩码
  final int bits;

  const CycleRequest(this.bits);

  /// 组合多个位的工厂
  factory CycleRequest.of(Iterable<int> parts) {
    var bits = 0;
    for (final part in parts) {
      bits |= part;
    }
    return CycleRequest(bits);
  }

  /// 是否为空请求
  bool get isEmpty => bits == 0;

  /// 是否包含 [other] 的全部位
  bool contains(int other) => bits & other == other;

  /// 位或合并
  CycleRequest operator |(CycleRequest other) =>
      CycleRequest(bits | other.bits);

  @override
  String toString() => 'CycleRequest(0x${bits.toRadixString(2)})';
}

/// 周期协调器（对齐 Rust `CycleCoordinator`）。
///
/// 防重入语义：正在跑时新触发以位或方式合并进 pending（sticky），
/// 并分配单调递增序列号；owner 的 drain 循环每轮取走全部位。
/// Dart 单 isolate 下 owner 锁用 Future 链互斥实现。
class CycleCoordinator {
  /// 失败历史记录上限
  static const int failuresCap = 128;

  int _pending = 0;
  int _requested = 0;
  int _completed = 0;
  int _expiredResultThrough = 0;

  /// 失败区间（first, last, message）
  final List<(int, int, String)> _failures = [];

  /// owner 互斥锁（Future 链）
  Future<void> _ownerLock = Future<void>.value();

  /// owner 锁是否被持有
  bool _ownerHeld = false;

  /// 合并请求位并分配序列号（序列号从 1 开始，0 表示「无」）。
  int request(CycleRequest req) {
    _requested = (_requested + 1) & 0x7FFFFFFFFFFFFFFF;
    if (_requested < 1) _requested = 1;
    _pending |= req.bits;
    return _requested;
  }

  /// 取 owner 锁（返回释放闭包；同一时刻仅一个 owner 可 drain）。
  Future<void Function()> lockOwner() {
    final completer = Completer<void>();
    final previous = _ownerLock;
    var released = false;
    _ownerLock = previous.then((_) => completer.future);
    return previous.then((_) {
      _ownerHeld = true;
      return () {
        if (released) return;
        released = true;
        _ownerHeld = false;
        completer.complete();
      };
    });
  }

  /// 取走全部 pending 位并返回当前序列号。
  (CycleRequest, int) takePendingWithSequence() {
    final bits = _pending;
    _pending = 0;
    return (CycleRequest(bits), _requested);
  }

  /// 把请求位或回 pending（门控保留语义：shutdown 丢弃、
  /// folderSyncing/离线/云刷新失败/不可信则保留）。
  void restore(CycleRequest req) {
    _pending |= req.bits;
  }

  /// 标记序列号区间已结算；失败时记录区间（超出上限时把挤出区间的
  /// 最大 end 并入 expiredResultThrough）。
  void complete(int through, [Object? error]) {
    if (through > _completed) _completed = through;
    if (error == null) return;
    final first = through; // 单序列区间（drain 每轮一个序列）
    _failures.add((first, through, '$error'));
    while (_failures.length > failuresCap) {
      final evicted = _failures.removeAt(0);
      if (evicted.$2 > _expiredResultThrough) {
        _expiredResultThrough = evicted.$2;
      }
    }
  }

  /// 查询序列号是否已结算（对齐 Rust `result_if_completed`）。
  ///
  /// 返回 null = 未结算；否则返回错误（null 错误 = 成功）。
  /// 历史已过期抛 [StateError]。
  ({bool settled, Object? error}) resultIfCompleted(int sequence) {
    if (_completed < sequence) return (settled: false, error: null);
    if (sequence <= _expiredResultThrough) {
      return (
        settled: true,
        error: AppError.generic('同步周期结果历史已过期'),
      );
    }
    for (final (first, last, message) in _failures) {
      if (sequence >= first && sequence <= last) {
        return (settled: true, error: AppError.generic(message));
      }
    }
    return (settled: true, error: null);
  }

  /// 是否有 pending 请求
  bool hasPending() => _pending != 0;

  /// 是否有未结算请求
  bool hasUncompletedRequest() => _requested > _completed;

  /// 是否空闲（无 pending 且 owner 锁未被持有）
  bool isIdle() => _pending == 0 && !_ownerHeld;
}

/// 路径是否重叠（相等或互为祖先，对齐 Rust `sync_paths_overlap`）。
bool syncPathsOverlap(String left, String right) {
  if (left == right) return true;
  if (left.isEmpty || right.isEmpty) return false;
  return left.startsWith('$right/') || right.startsWith('$left/');
}

/// 活动守卫（RAII 语义；[close] 幂等）。
class ActivityGuard {
  final void Function() _release;
  bool _released = false;

  ActivityGuard(this._release);

  /// 释放活动登记。
  void close() {
    if (_released) return;
    _released = true;
    _release();
  }
}

/// 活动追踪器（对齐 Rust `ActivityTracker`）。
///
/// 普通活动：路径与任一排他租约重叠即拒绝；
/// 排他活动：与任一活动或排他路径重叠即拒绝。
/// [close] 后拒绝新活动（已登记活动仍可结算释放）。
class ActivityTracker {
  bool _accepting = true;
  int _count = 0;
  final Map<String, int> _activePaths = {};
  final Set<String> _exclusivePaths = {};
  final List<Completer<void>> _idleWaiters = [];

  /// 是否仍接受新活动
  bool get isAccepting => _accepting;

  /// 当前登记活动数
  int get count => _count;

  /// 登记一个普通活动（可携带相对路径）。
  ActivityGuard begin([String? path]) {
    if (!_accepting) {
      throw AppError.generic('同步引擎正在停止，拒绝新传输活动');
    }
    if (path != null) {
      for (final exclusive in _exclusivePaths) {
        if (syncPathsOverlap(path, exclusive)) {
          throw AppError.generic('该路径正在执行破坏性操作，请稍后重试');
        }
      }
      _activePaths[path] = (_activePaths[path] ?? 0) + 1;
    }
    _count++;
    return ActivityGuard(() {
      _count--;
      if (path != null) {
        final remaining = (_activePaths[path] ?? 1) - 1;
        if (remaining <= 0) {
          _activePaths.remove(path);
        } else {
          _activePaths[path] = remaining;
        }
      }
      if (_count == 0) _notifyIdle();
    });
  }

  /// 登记一个排他路径活动（破坏性操作用）。
  ActivityGuard beginExclusive(String path) {
    if (!_accepting) {
      throw AppError.generic('同步引擎正在停止，拒绝新传输活动');
    }
    for (final active in _activePaths.keys) {
      if (syncPathsOverlap(path, active)) {
        throw AppError.generic('该路径或其子树存在活动任务，请稍后重试');
      }
    }
    for (final exclusive in _exclusivePaths) {
      if (syncPathsOverlap(path, exclusive)) {
        throw AppError.generic('该路径或其子树存在活动任务，请稍后重试');
      }
    }
    _exclusivePaths.add(path);
    _count++;
    return ActivityGuard(() {
      _count--;
      _exclusivePaths.remove(path);
      if (_count == 0) _notifyIdle();
    });
  }

  /// 拒绝新活动（已登记活动仍可释放）。
  void close() {
    _accepting = false;
    if (_count == 0) _notifyIdle();
  }

  /// 等待全部登记活动释放。
  Future<void> waitIdle() {
    if (_count == 0) return Future<void>.value();
    final completer = Completer<void>();
    _idleWaiters.add(completer);
    return completer.future;
  }

  void _notifyIdle() {
    final waiters = List<Completer<void>>.of(_idleWaiters);
    _idleWaiters.clear();
    for (final waiter in waiters) {
      if (!waiter.isCompleted) waiter.complete();
    }
  }
}

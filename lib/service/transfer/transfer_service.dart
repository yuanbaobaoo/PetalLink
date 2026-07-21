import 'package:sqflite/sqflite.dart';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/transfer/transfer_patch.dart';
import 'package:petal_link/types/enums.dart';

/// 传输队列服务
///
/// 管理上传/下载任务队列，基于 SQLite transfer_queue 表（schema v5）持久化。
/// 使用 CAS（Compare-And-Swap）乐观锁保证并发安全：
/// - 状态转移：先经 [TransferState.canTransition] 合法转移表校验，
///   再 UPDATE...WHERE id=? AND state=? 并递增 state_revision，检查 affectedRows
/// - 进度更新：UPDATE...WHERE id=? AND transferred < ?，检查 affectedRows（防倒退）
class TransferService {
  final DatabaseService _db;

  TransferService(this._db);

  /// 入队新传输任务
  ///
  /// task.id 为 0 时由 SQLite AUTOINCREMENT 分配，返回携带真实 id 的任务。
  Future<AppResult<TransferTask>> enqueue(TransferTask task) async {
    try {
      final db = await _db.database;
      final id = await db.insert('transfer_queue', task.toRow());
      final stored = task.copyWith(id: id);
      AppLogger.i('传输任务入队: ${task.name} (id=$id)');
      return Ok(stored);
    } catch (e, st) {
      AppLogger.e('enqueue 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }

  /// 更新任务进度（对齐 Rust `update_running_transfer`）
  ///
  /// 进度补丁不递增 state_revision；门禁仅 (id, state_revision, Running)，
  /// 生命周期收束后的迟到回调无法落库（对齐 Rust repository/transfer_queue.rs）。
  /// 最后写入胜出：进度回调由 TaskRunner 顺序化（throttle + 单调值），无需防倒退。
  /// [expectedRevision] 非空时追加 `state_revision=? AND state=Running` 门禁。
  Future<AppResult<void>> updateProgress(
    int taskId,
    int transferred, {
    int? resumeOffset,
    int? expectedRevision,
  }) async {
    try {
      final db = await _db.database;

      final count = await db.update(
        'transfer_queue',
        {
          'transferred': transferred,
          'resume_offset': ?resumeOffset,
        },
        where: 'id = ?'
            '${expectedRevision != null ? ' AND state_revision = ? AND state = ?' : ''}',
        whereArgs: [
          taskId,
          if (expectedRevision != null) ...[
            expectedRevision,
            TransferState.Running.code,
          ],
        ],
      );

      if (count == 0) {
        AppLogger.d('updateProgress 门禁未通过: $taskId (生命周期已收束或修订过期)');
      }
      return const Ok(null);
    } catch (e, st) {
      AppLogger.e('updateProgress 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }

  /// 持久化上传会话身份与断点偏移（对齐 Rust TaskProgressReporter.update_resume）。
  ///
  /// 供上传执行器的断点回调将 serverId/uploadId/sessionUrl/resumeOffset
  /// 写入任务行，崩溃后可凭会话 URL 续传。会话 URL 可能随 308/状态查询轮换，
  /// 即使偏移未推进也必须落库，因此不按 transferred CAS 防倒退。
  /// [expectedRevision] 非空时追加 `state_revision=? AND state=Running` 门禁，
  /// 拒绝生命周期收束后的迟到会话写（对齐 Rust update_running_transfer）。
  Future<AppResult<void>> updateResumeSession(
    int taskId, {
    required String serverId,
    required String uploadId,
    required int resumeOffset,
    required String sessionUrl,
    int? expectedRevision,
  }) async {
    if (resumeOffset > 0 && sessionUrl.trim().isEmpty) {
      return Err(const ConfigError(message: '非零断点缺少 session_url'));
    }
    try {
      final db = await _db.database;
      await db.update(
        'transfer_queue',
        {
          'transferred': resumeOffset,
          'resume_offset': resumeOffset,
          'server_id': serverId,
          'upload_id': uploadId,
          'session_url': sessionUrl,
        },
        where: 'id = ?'
            '${expectedRevision != null ? ' AND state_revision = ? AND state = ?' : ''}',
        whereArgs: [
          taskId,
          if (expectedRevision != null) ...[
            expectedRevision,
            TransferState.Running.code,
          ],
        ],
      );
      return const Ok(null);
    } catch (e, st) {
      AppLogger.e('updateResumeSession 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }

  /// 状态转移（CAS：合法转移表校验 + 当前状态检查 + state_revision 递增）
  ///
  /// 对齐 Rust `transition_transfer_in_transaction`：
  /// 1. [TransferState.canTransition] 校验 from → to 合法性
  /// 2. UPDATE ... WHERE id=? AND state=? [AND state_revision=?]，
  ///    affectedRows=0 表示并发冲突（返回 Ok(null)）
  /// 3. 可变字段按 [TransferPatch] 三态语义应用；state_revision 恒递增
  /// 4. patch.clearUploadSession 置位时原子失效断点上传身份
  ///   （对齐 Rust `transition_transfer_clearing_upload_session`，
  ///   仅当远端复核确认目标写入不存在后方可使用）
  Future<AppResult<TransferTask?>> transition(
    int taskId,
    TransferState from,
    TransferState to, {
    TransferPatch patch = const TransferPatch(),
    int? expectedRevision,
  }) async {
    // 合法转移表校验（对齐 Rust can_transition）
    if (!from.canTransition(to)) {
      return Err(ConfigError(
          message: '非法传输状态转移：${from.name} → ${to.name}'));
    }

    try {
      final db = await _db.database;
      final transferPatch =
          patch.clearUploadSession ? _withClearedUploadSession(patch) : patch;

      final count = await db.rawUpdate(
        'UPDATE transfer_queue SET '
        'state = ?, '
        '${_patchAssignments(transferPatch)}, '
        'state_revision = state_revision + 1 '
        'WHERE id = ? AND state = ?'
        '${expectedRevision != null ? ' AND state_revision = ?' : ''}',
        [
          to.code,
          ..._patchArgs(transferPatch),
          taskId,
          from.code,
          ?expectedRevision,
        ],
      );

      if (count == 0) {
        AppLogger.d('transition CAS 失败: $taskId (状态已变更: ${from.name})');
        return const Ok(null);
      }

      AppLogger.d('状态转移: $taskId ${from.name} → ${to.name}');
      final updated = await _loadTask(db, taskId);
      return Ok(updated);
    } catch (e, st) {
      AppLogger.e('transition 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }

  /// 在生命周期不变时更新错误与重试事实（对齐 Rust `patch_transfer_in_state`）。
  ///
  /// CAS 校验 id + state + state_revision，state_revision 恒递增；
  /// 仅用于同状态事实更新（如 VerifyingRemote 停留、Failed 拒绝重试后改写错误）。
  /// 生命周期变化必须使用 [transition]。
  Future<AppResult<TransferTask?>> patchInState(
    int taskId,
    TransferState expectedState,
    int expectedRevision, {
    TransferPatch patch = const TransferPatch(),
  }) async {
    try {
      final db = await _db.database;
      final count = await db.rawUpdate(
        'UPDATE transfer_queue SET '
        '${_patchAssignments(patch)}, '
        'state_revision = state_revision + 1 '
        'WHERE id = ? AND state = ? AND state_revision = ?',
        [..._patchArgs(patch), taskId, expectedState.code, expectedRevision],
      );

      if (count == 0) {
        AppLogger.d('patchInState CAS 失败: $taskId (${expectedState.name})');
        return const Ok(null);
      }
      final updated = await _loadTask(db, taskId);
      return Ok(updated);
    } catch (e, st) {
      AppLogger.e('patchInState 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }

  /// 按 id 加载传输任务（不存在返回 Ok(null)）
  Future<AppResult<TransferTask?>> getTaskById(int taskId) async {
    try {
      final db = await _db.database;
      return Ok(await _loadTask(db, taskId));
    } catch (e, st) {
      AppLogger.e('getTaskById 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }

  /// 统计 Pending + Running 任务数（对齐 Rust `transfer_has_active` 命令）
  Future<AppResult<int>> countPendingOrRunning() async {
    try {
      final db = await _db.database;
      final rows = await db.rawQuery(
        'SELECT COUNT(*) AS c FROM transfer_queue WHERE state IN (?, ?)',
        [TransferState.Pending.code, TransferState.Running.code],
      );
      final count = rows.first['c'];
      return Ok(count is int ? count : int.tryParse('$count') ?? 0);
    } catch (e, st) {
      AppLogger.e('countPendingOrRunning 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }

  /// 构造清会话补丁（对齐 Rust transition_transfer_clearing_upload_session：
  /// session_url=NULL、transferred=0、resume_offset=0；server/upload id 在
  /// SQL 层随 clearUploadSession 一并置 NULL）。
  TransferPatch _withClearedUploadSession(TransferPatch patch) {
    return TransferPatch(
      errorKind: patch.errorKind,
      errorMessage: patch.errorMessage,
      nextRetryAt: patch.nextRetryAt,
      finishedAt: patch.finishedAt,
      remoteResultFileId: patch.remoteResultFileId,
      sessionUrl: const ClearPatch(),
      transferred: 0,
      resumeOffset: 0,
      attemptCount: patch.attemptCount,
      clearUploadSession: true,
    );
  }

  /// 将补丁编码为 SQL 赋值片段（CASE 三态，对齐 Rust nullable_patch）。
  String _patchAssignments(TransferPatch patch) {
    return [
      _nullableAssignment('error_kind'),
      _nullableAssignment('error_message'),
      _nullableAssignment('next_retry_at'),
      _nullableAssignment('finished_at'),
      _nullableAssignment('remote_result_file_id'),
      _nullableAssignment('session_url'),
      if (patch.clearUploadSession) ...[
        'server_id = NULL',
        'upload_id = NULL',
      ] else ...[
        'server_id = server_id',
        'upload_id = upload_id',
      ],
      'transferred = CASE WHEN ? IS NULL THEN transferred ELSE ? END',
      'resume_offset = CASE WHEN ? IS NULL THEN resume_offset ELSE ? END',
      'attempt_count = CASE WHEN ? IS NULL THEN attempt_count ELSE ? END',
    ].join(', ');
  }

  /// 单个可空列的 CASE 赋值片段（mode: 0 保留 / 1 设值 / 2 清空）。
  String _nullableAssignment(String column) {
    return '$column = CASE ? WHEN 0 THEN $column WHEN 1 THEN ? ELSE NULL END';
  }

  /// 与 [_patchAssignments] 对应的参数序列。
  List<Object?> _patchArgs(TransferPatch patch) {
    return [
      ..._nullableArgs(patch.errorKind, (v) => v.code),
      ..._nullableArgs(patch.errorMessage, (v) => v),
      ..._nullableArgs(patch.nextRetryAt, (v) => v),
      ..._nullableArgs(patch.finishedAt, (v) => v),
      ..._nullableArgs(patch.remoteResultFileId, (v) => v),
      ..._nullableArgs(patch.sessionUrl, (v) => v),
      patch.transferred,
      patch.transferred,
      patch.resumeOffset,
      patch.resumeOffset,
      patch.attemptCount,
      patch.attemptCount,
    ];
  }

  /// 三态补丁 → (mode, value) 参数对。
  List<Object?> _nullableArgs<T>(ColumnPatch<T> patch, Object? Function(T) map) {
    return switch (patch) {
      KeepPatch<T>() => [0, null],
      SetPatch<T>(:final value) => [1, map(value)],
      ClearPatch<T>() => [2, null],
    };
  }

  /// 按 id 读取单行（供迁移后返回最新任务快照）。
  Future<TransferTask?> _loadTask(Database db, int taskId) async {
    final rows = await db.query(
      'transfer_queue',
      where: 'id = ?',
      whereArgs: [taskId],
      limit: 1,
    );
    if (rows.isEmpty) return null;
    return TransferTask.fromRow(rows.first);
  }

  /// 获取所有活跃任务（非终态）
  Future<AppResult<List<TransferTask>>> getActiveTasks() async {
    try {
      final db = await _db.database;
      final terminalCodes = TransferState.values
          .where((s) => s.isTerminal)
          .map((s) => s.code)
          .toList();

      // 构建 NOT IN 查询
      final placeholders = terminalCodes.map((_) => '?').join(',');
      final rows = await db.rawQuery(
        'SELECT * FROM transfer_queue WHERE state NOT IN ($placeholders) ORDER BY created_at ASC',
        terminalCodes,
      );

      final tasks = rows.map((row) => TransferTask.fromRow(row)).toList();
      return Ok(tasks);
    } catch (e, st) {
      AppLogger.e('getActiveTasks 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }

  /// 获取所有任务
  Future<AppResult<List<TransferTask>>> getAllTasks() async {
    try {
      final db = await _db.database;
      final rows = await db.query('transfer_queue', orderBy: 'created_at DESC');
      final tasks = rows.map((row) => TransferTask.fromRow(row)).toList();
      return Ok(tasks);
    } catch (e, st) {
      AppLogger.e('getAllTasks 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }

  /// 获取指定相对路径的全部任务（同路径仲裁用，created_at 升序）。
  Future<AppResult<List<TransferTask>>> getTasksByRelativePath(
    String relativePath,
  ) async {
    try {
      final db = await _db.database;
      final rows = await db.query('transfer_queue',
          where: 'relative_path = ?',
          whereArgs: [relativePath],
          orderBy: 'created_at ASC');
      return Ok(rows.map(TransferTask.fromRow).toList());
    } catch (e, st) {
      AppLogger.e('getTasksByRelativePath 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }

  /// 获取全部 Failed 任务（全局重试用，created_at 升序）。
  Future<AppResult<List<TransferTask>>> getFailedTasks() async {
    try {
      final db = await _db.database;
      final rows = await db.query('transfer_queue',
          where: 'state = ?',
          whereArgs: [TransferState.Failed.code],
          orderBy: 'created_at ASC');
      return Ok(rows.map(TransferTask.fromRow).toList());
    } catch (e, st) {
      AppLogger.e('getFailedTasks 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }

  /// 用新意图全量覆写重规划任务的意图列（对齐 Rust `replan_task` 的裸 SQL；
  /// CAS 门禁 state=Pending + state_revision，changed != 1 返回 null）。
  Future<AppResult<TransferTask?>> overwriteReplanIntent(
    TransferTask current,
    TransferTask replacement,
  ) async {
    try {
      final db = await _db.database;
      final count = await db.rawUpdate(
        'UPDATE transfer_queue SET '
        'direction = ?, file_id = ?, local_path = ?, name = ?, '
        'total_size = ?, transferred = ?, created_at = ?, server_id = ?, '
        'upload_id = ?, resume_offset = ?, session_url = ?, '
        'relative_path = ?, parent_file_id = ?, operation = ?, '
        'source_mtime = ?, source_size = ?, expected_cloud_edited_time = ?, '
        'attempt_count = ? '
        'WHERE id = ? AND state = ? AND state_revision = ?',
        [
          replacement.direction.code,
          replacement.fileId,
          replacement.localPath,
          replacement.name,
          replacement.totalSize,
          replacement.transferred,
          replacement.createdAt,
          replacement.serverId,
          replacement.uploadId,
          replacement.resumeOffset,
          replacement.sessionUrl,
          replacement.relativePath,
          replacement.parentFileId,
          replacement.operation?.code,
          replacement.sourceMtime,
          replacement.sourceSize,
          replacement.expectedCloudEditedTime,
          replacement.attemptCount,
          current.id,
          TransferState.Pending.code,
          current.stateRevision,
        ],
      );
      if (count != 1) {
        AppLogger.d('overwriteReplanIntent CAS 失败: ${current.id}');
        return const Ok(null);
      }
      return Ok(await _loadTask(db, current.id));
    } catch (e, st) {
      AppLogger.e('overwriteReplanIntent 异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }

  /// 清除已完成的任务（对齐 Rust `transfer_clear_completed`：仅删 Completed）。
  ///
  /// Failed 行是自动任务的路径屏障（保留可见错误供显式重试），
  /// Canceled 行由历史修剪统一处理，均不在本命令清除范围内。
  /// 返回清除的任务数量。
  Future<AppResult<int>> clearCompleted() {
    return _clearByStates(const [TransferState.Completed], '已完成');
  }

  /// 清除已失败的任务（对齐 Rust `transfer_clear_failed`：仅删 Failed）。
  Future<AppResult<int>> clearFailed() {
    return _clearByStates(const [TransferState.Failed], '已失败');
  }

  /// 清除已结束的任务（对齐 Rust `transfer_clear_finished`：Completed + Failed）。
  Future<AppResult<int>> clearFinished() {
    return _clearByStates(
        const [TransferState.Completed, TransferState.Failed], '已结束');
  }

  /// 按状态集合删除传输历史。
  Future<AppResult<int>> _clearByStates(
      List<TransferState> states, String label) async {
    try {
      final db = await _db.database;
      final codes = states.map((s) => s.code).toList();
      final placeholders = codes.map((_) => '?').join(',');
      final count = await db.rawDelete(
        'DELETE FROM transfer_queue WHERE state IN ($placeholders)',
        codes,
      );

      AppLogger.i('清除$label任务: $count 条');
      return Ok(count);
    } catch (e, st) {
      AppLogger.e('清除$label任务异常', e, st);
      return Err(GenericError(message: e.toString()));
    }
  }
}

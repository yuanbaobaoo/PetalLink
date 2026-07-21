/// 同步动作与结果类型（对齐 Rust `src/sync/state.rs`）。
///
/// planner 逐路径产出 [SyncAction]，executor 执行后回写 [ActionResult]，
/// results 层按动作类型结算 sync_items 基线与云树缓存。
library;

import 'package:petal_link/entity/drive_file.dart';

/// 同步动作类型（对齐 Rust `SyncActionType`，serde camelCase 协议）。
enum SyncActionType {
  /// 上传（新增或覆盖）
  upload('upload'),

  /// 创建本地占位符
  createPlaceholder('createPlaceholder'),

  /// 下载
  download('download'),

  /// 从云端删除
  deleteFromCloud('deleteFromCloud'),

  /// 从本地删除
  deleteFromLocal('deleteFromLocal'),

  /// 创建冲突副本
  createConflictCopy('createConflictCopy'),

  /// 跳过（仅信息性；携带 cloudFile 的 Skip 是 pending 占位收敛动作）
  skip('skip'),

  /// 创建文件夹（本地或云端）
  createFolder('createFolder'),

  /// 云端移动/改名
  moveInCloud('moveInCloud'),

  /// 云端删除前备份本地已修改内容
  backupBeforeCloudDelete('backupBeforeCloudDelete');

  /// 线上字符串值（camelCase，对齐 Rust serde）
  final String wireName;

  const SyncActionType(this.wireName);
}

/// 单路径同步动作（对齐 Rust `SyncAction`）。
class SyncAction {
  /// 动作类型
  SyncActionType actionType;

  /// 相对挂载根的规范路径
  String? relativePath;

  /// 关联云端 fileId
  String? fileId;

  /// 目标父目录 fileId（CreateFolder/MoveInCloud 用）
  String? parentFileId;

  /// 本地绝对路径
  String? localPath;

  /// 云端元数据（createPlaceholder/createFolder/download 用）
  DriveFile? cloudFile;

  /// 决策原因（日志/UI 展示）
  String? reason;

  SyncAction({
    required this.actionType,
    this.relativePath,
    this.fileId,
    this.parentFileId,
    this.localPath,
    this.cloudFile,
    this.reason,
  });

  @override
  String toString() =>
      'SyncAction(${actionType.wireName}, $relativePath, fileId=$fileId)';
}

/// 动作执行结果（对齐 Rust `ActionResult`）。
class ActionResult {
  /// 是否成功
  final bool success;

  /// 错误信息（成功时可为信息性 reason）
  final String? errorMessage;

  /// 是否延迟结算（稳定性未过/用户编辑中/引擎已停；
  /// 取消不是同步失败，不生成 FAILED 基线）
  final bool deferred;

  /// 执行产出的云端元数据（上传/建目录/移动成功时携带）
  final DriveFile? cloudFile;

  const ActionResult({
    required this.success,
    this.errorMessage,
    this.deferred = false,
    this.cloudFile,
  });

  /// 成功结果
  const ActionResult.ok({this.errorMessage, this.cloudFile})
      : success = true,
        deferred = false;

  /// 失败结果（非延迟：写入 FAILED 基线）
  const ActionResult.fail(String message)
      : success = false,
        errorMessage = message,
        deferred = false,
        cloudFile = null;

  /// 延迟结果（不写 FAILED 基线，等待重新规划）
  const ActionResult.defer(String message)
      : success = false,
        errorMessage = message,
        deferred = true,
        cloudFile = null;
}

/// 释放空间安全检查结果（对齐 Rust `FreeUpCheckResult`）。
enum FreeUpCheckResult {
  /// 可安全释放
  safe,

  /// 云端不存在该文件
  notInCloud,

  /// 未同步（不可释放）
  notSynced,
}

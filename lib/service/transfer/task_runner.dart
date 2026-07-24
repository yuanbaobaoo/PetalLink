/// 持久化传输任务执行器（对齐 Rust `src/sync/task_runner.rs` + `task_runner/`）。
///
/// 职责：
/// - 驱动 9 态持久化状态机：Pending→Running→(WaitingForNetwork/BackingOff/
///   VerifyingRemote)→Completed/Failed/Canceled；RestartRequired 回 planner；
///   全部状态变迁走 TransferService.transition（CAS + state_revision 递增）
/// - 并发调度：按 created_at FIFO 准入，槽位数 = AppConfig.concurrency（1-20，默认 6）
/// - 网络门控：离线时执行链将任务转入 WaitingForNetwork；恢复在线后
///   按「核验 → 等待 → 到期退避 → 新任务」重新调度；请求级网络失败边沿上报 NetGuard
/// - 退避重试：按 retry_policy 分类决定可重试性、attempt_count、next_retry_at
/// - 崩溃恢复：启动时收敛非终态任务行（Running 上传→核验、下载→断点续跑）
///
/// 引擎编排（planner/云树/基线结算）属后续任务；本类只暴露执行与命令面。
///
/// 本文件为组合入口：基类 [TaskRunnerBase] 集中跨 mixin 共享的字段、helper、
/// 生命周期/命令面/pump loop/持久化/发布原语；三大区域由 mixin 提供：
/// - [TaskRunnerAdmission]：同路径仲裁入队
/// - [TaskRunnerExecutionSettlement]：执行主链 + 结果结算
/// - [TaskRunnerRecovery]：引擎恢复接缝 + 崩溃恢复 + 远端核验
library;

import 'package:petal_link/service/transfer/task_runner_admission.dart';
import 'package:petal_link/service/transfer/task_runner_base.dart';
import 'package:petal_link/service/transfer/task_runner_execution_settlement.dart';
import 'package:petal_link/service/transfer/task_runner_recovery.dart';

export 'package:petal_link/service/transfer/task_runner_base.dart'
    show TaskRunnerBase;

class TaskRunner extends TaskRunnerBase
    with
        TaskRunnerAdmission,
        TaskRunnerExecutionSettlement,
        TaskRunnerRecovery {
  TaskRunner({
    required super.transferService,
    required super.operations,
    super.isOnline,
    super.netTransitions,
    super.onRequestNetworkFailure,
    super.nowMs,
    super.jitterMs,
    super.concurrencyProvider,
    super.mountRootProvider,
    super.isPlaceholder,
    super.tickInterval,
    super.maxAttempts,
  });
}

import 'package:flutter/material.dart';
import 'package:intl/intl.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/entity/sync_state.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/types/enums.dart';
import 'package:petal_link/widgets/index.dart';

/// 同步状态条（对标 CMP SyncStatusBar.kt；v2：design/02-main.html、
/// 03-sync-states.html 的 .sync-bar）。
///
/// minHeight 44、padding 6/20（内容超高可换行）；
/// 左：状态指示（活动态 spin sync 图标 brand；空闲 failed>0 红色 8×8 圆点；
/// 空闲正常绿色 8×8 圆点）+ statusText（14sp medium，9 种 syncPhase 文案）
/// + 上次同步时间（13.5sp text-secondary）；
/// 右：标签区右对齐换行（MateTag chip：上传/下载 primary、
/// 等待网络/编辑中/冲突 warning、同步失败 error 可点）。
/// 失败弹窗列出 failedItems(path+error)。底部 MateHDivider 分隔线。
///
/// 数据源直接取 [SyncGlobalState] 权威快照（SyncUIState 共享文件未携带
/// uploading/downloading 等分桶计数，页面另行订阅 stateStream 获得）。
class FilesSyncStatusBar extends StatelessWidget {
  /// 完整同步快照
  final SyncGlobalState sync;

  /// 传输任务列表（空闲细分文案：核验中/等待重试/等待重规划/等待传输）
  final List<TransferTask> transfers;

  const FilesSyncStatusBar({
    super.key,
    required this.sync,
    this.transfers = const [],
  });

  /// 是否有活跃传输（上传/下载/等待网络任一 >0，对齐 CMP hasActiveTransfer）
  bool get _hasActiveTransfer =>
      sync.uploading > 0 || sync.downloading > 0 || sync.waitingNetwork > 0;

  /// 是否空闲（无活跃传输、非索引、非运行，对齐 CMP isIdle）
  bool get _isIdle => !_hasActiveTransfer && !sync.isIndexing && !sync.isRunning;

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final typography = MateTheme.typographyOf(context);
    final metrics = MateTheme.metricsOf(context).statusBar;
    final statusText = _statusTextFor(sync, transfers);

    return Column(
      children: [
        Container(
          width: double.infinity,
          constraints: BoxConstraints(minHeight: metrics.minimumHeight),
          color: colors.bgContainer,
          padding: EdgeInsets.symmetric(
            horizontal: metrics.horizontalPadding,
            vertical: metrics.verticalPadding,
          ),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.center,
            children: [
              // 左侧：状态指示 + 文案 + 时间（v2 .sync-bar__left：flex:1，gap 10）
              Expanded(
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.center,
                  children: [
                    if (!_isIdle)
                      // 活动态：spin 的 sync 图标（v2 场景 4）
                      MateIcon(
                        name: 'sync',
                        size: metrics.syncingIconSize,
                        tint: colors.brand,
                        spin: true,
                      )
                    else
                      // 空闲态：8×8 状态圆点（failed>0 红色，否则绿色）
                      Container(
                        width: metrics.idleIndicatorSize,
                        height: metrics.idleIndicatorSize,
                        decoration: BoxDecoration(
                          shape: BoxShape.circle,
                          color: sync.failed > 0 ? colors.error : colors.success,
                        ),
                      ),
                    SizedBox(width: metrics.statusContentSpacing),
                    Flexible(
                      child: Text(
                        statusText,
                        style: typography.statusBar.currentStatus.copyWith(
                          color: colors.textPrimary,
                        ),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                    if (_isIdle &&
                        sync.lastSyncTime != null &&
                        sync.lastSyncTime! > 0) ...[
                      SizedBox(width: metrics.statusContentSpacing),
                      Text(
                        '· 上次同步 ${_formatLastSync(sync.lastSyncTime!)}',
                        style: typography.statusBar.lastSyncTime.copyWith(
                          color: colors.textSecondary,
                        ),
                      ),
                    ],
                  ],
                ),
              ),

              // 右侧标签区（v2 .sync-bar__tags：右对齐、wrap 换行、gap 6）
              Wrap(
                spacing: metrics.actionHorizontalSpacing,
                runSpacing: metrics.actionVerticalSpacing,
                alignment: WrapAlignment.end,
                children: [
                  if (sync.uploading > 0)
                    MateTag(
                      label: '上传 ${sync.uploading}',
                      theme: MateTagTheme.primary,
                      size: MateTagSize.small,
                    ),
                  if (sync.downloading > 0)
                    MateTag(
                      label: '下载 ${sync.downloading}',
                      theme: MateTagTheme.primary,
                      size: MateTagSize.small,
                    ),
                  if (sync.waitingNetwork > 0)
                    MateTag(
                      label: '等待网络 ${sync.waitingNetwork}',
                      theme: MateTagTheme.warning,
                      size: MateTagSize.small,
                    ),
                  if (sync.editing > 0)
                    MateTag(
                      label: '编辑中 ${sync.editing}',
                      theme: MateTagTheme.warning,
                      size: MateTagSize.small,
                    ),
                  if (sync.conflict > 0)
                    MateTag(
                      label: '冲突 ${sync.conflict}',
                      theme: MateTagTheme.warning,
                      size: MateTagSize.small,
                    ),
                  if (sync.failed > 0)
                    MateTag(
                      label: '同步失败 ${sync.failed}',
                      theme: MateTagTheme.error,
                      size: MateTagSize.small,
                      onClick: () => _showFailedItems(context),
                    ),
                ],
              ),
            ],
          ),
        ),
        // 底分隔线（v2 .sync-bar border-bottom）
        const MateHDivider(),
      ],
    );
  }

  /// 上次同步时间（HH:mm，本地时区；对齐 CMP SimpleDateFormat("HH:mm")）
  static String _formatLastSync(int millis) {
    return DateFormat('HH:mm')
        .format(DateTime.fromMillisecondsSinceEpoch(millis));
  }

  /// 失败项弹窗：列出 failedItems(path+error)（对标 CMP 失败弹窗）
  void _showFailedItems(BuildContext context) {
    final content = sync.failedItems.map((it) {
      final err = it.errorMessage != null ? '\n${it.errorMessage}' : '';
      return '${it.relativePath}$err';
    }).join('\n\n');

    MateDialog.open(MateDialogOptions(
      title: '同步失败项 (${sync.failedItems.length})',
      titleIcon: 'alert',
      danger: true,
      confirmText: '关闭',
      content: content.isEmpty ? '暂无失败项详情' : content,
    ));
  }

  /// 9 种 syncPhase 文案 + 空闲细分（对标 CMP statusTextFor，含 transfer 细分）
  static String _statusTextFor(
    SyncGlobalState sync,
    List<TransferTask> transfers,
  ) {
    switch (sync.syncPhase?.wireName) {
      case 'indexing-startup':
        return '正在读取云端索引（首次）…';
      case 'indexing-manual':
        return '正在读取云端索引…';
      case 'indexing-auto-full':
        return '正在读取云端索引（全量纠偏）…';
      case 'querying-changes':
        return '正在查询云端变更…';
      case 'syncing-auto-incremental':
        return '正在同步云端变更…';
      case 'syncing-local':
        return '正在同步本地变更…';
      case 'syncing-manual':
        return '正在同步…';
      case 'syncing-retry':
        return '正在重试失败项…';
      case 'syncing-startup':
        return '正在同步（启动恢复）…';
    }
    // 空闲细分
    if (sync.uploading > 0 || sync.downloading > 0) return '同步中';
    if (transfers.any((t) => t.state == TransferState.verifyingRemote)) {
      return '正在核验远端…';
    }
    if (transfers.any((t) => t.state == TransferState.backingOff)) {
      return '等待下次重试…';
    }
    if (transfers.any((t) => t.state == TransferState.restartRequired)) {
      return '等待重新规划…';
    }
    if (transfers.any((t) => t.state == TransferState.pending)) {
      return '等待传输…';
    }
    if (sync.waitingNetwork > 0) return '等待网络恢复…';
    if (sync.failed > 0) return '同步存在失败项';
    return '同步完成';
  }
}

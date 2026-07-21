import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:intl/intl.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/pages/logs/controller/logs_controller.dart';
import 'package:petal_link/widgets/index.dart';

/// 日志查看页（对标 CMP LogViewerScreen.kt 独立模式 / design/v2/07-logs.html）。
///
/// AppBar（返回 + 同步日志）+ 工具栏（ALL/INFO/WARN/ERROR 级别过滤标签
/// + 导出/清空图标按钮）+ 白色 panel 日志列表（级别 tag + 消息 + 元信息）。
/// 列表数据由 [LogsController] 2s 轮询自动刷新。
class LogsPage extends StatefulWidget {
  const LogsPage({super.key});

  @override
  State<LogsPage> createState() => _LogsPageState();
}

class _LogsPageState extends State<LogsPage> {
  /// 页面控制器（xe-cloud 惯例：页面持有，dispose 时释放）
  final LogsController notifier = Get.put(LogsController());

  @override
  void initState() {
    super.initState();
    // 进入页面启动 2s 轮询（首次加载已在控制器 onInit 完成）
    Future.microtask(notifier.startPolling);
  }

  @override
  void dispose() {
    Get.delete<LogsController>();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).logViewer;
    final typography = MateTheme.typographyOf(context).logViewer;

    return Scaffold(
      body: Column(
        children: [
          // 独立 AppBar（56px + 底分隔线 + 返回箭头 rotate 180°）
          SizedBox(
            height: metrics.inlineHeaderHeight,
            child: Padding(
              padding: EdgeInsets.symmetric(
                horizontal: metrics.inlineHeaderHorizontalPadding,
              ),
              child: Row(
                children: [
                  Transform.rotate(
                    angle: 3.141592653589793, // 180°：arrow 图标朝左
                    child: MateButton(
                      variant: MateButtonVariant.icon,
                      icon: 'arrow',
                      onClick: () => Get.back(),
                    ),
                  ),
                  SizedBox(width: metrics.inlineHeaderContentSpacing),
                  Text(
                    '同步日志',
                    style: typography.pageTitle.copyWith(
                      color: colors.textPrimary,
                    ),
                  ),
                ],
              ),
            ),
          ),
          const MateHDivider(),

          // 内容区（工具栏 + 列表随状态刷新）
          Expanded(
            child: Obx(() {
              final state = notifier.state.value;
              return Column(
                children: [
                  _buildToolbar(state),
                  Expanded(child: _buildContent(state)),
                ],
              );
            }),
          ),
        ],
      ),
    );
  }

  /// 工具栏（v2 log-toolbar：级别过滤 chip + 右侧导出/清空）
  Widget _buildToolbar(LogsState state) {
    final metrics = MateTheme.metricsOf(context).logViewer;

    return Padding(
      padding: EdgeInsets.symmetric(
        horizontal: metrics.standaloneHeaderHorizontalPadding,
        vertical: metrics.headerVerticalPadding,
      ),
      child: Row(
        children: [
          for (final lv in LevelFilter.values) ...[
            MateTag(
              label: lv.name.toUpperCase(),
              theme: state.filter == lv
                  ? _filterTheme(lv)
                  : MateTagTheme.normal,
              onClick: () => notifier.setFilter(lv),
            ),
            if (lv != LevelFilter.values.last)
              SizedBox(width: metrics.headerContentSpacing),
          ],
          const Spacer(),
          MateButton(
            variant: MateButtonVariant.icon,
            icon: 'download',
            onClick: notifier.exportLogs,
            disabled: state.loading,
          ),
          SizedBox(width: metrics.headerContentSpacing),
          MateButton(
            variant: MateButtonVariant.icon,
            icon: 'trash',
            onClick: notifier.clearLogs,
            disabled: state.loading,
          ),
        ],
      ),
    );
  }

  /// 内容区：loading spinner / 空态 / 白色 panel 日志列表
  Widget _buildContent(LogsState state) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).logViewer;

    if (state.loading) {
      return Center(child: MateCircularProgress(size: metrics.loadingSize));
    }

    final filtered = state.filteredRecords;
    if (filtered.isEmpty) {
      return const MateEmpty(title: '暂无日志', icon: 'list');
    }

    // v2 log-list：白色 panel（bgContainer radius 10 + 0.5px 细边，外边距 0/20/20）
    return Padding(
      padding: EdgeInsets.only(
        left: metrics.standaloneContentPadding,
        right: metrics.standaloneContentPadding,
        bottom: metrics.standaloneContentPadding,
      ),
      child: Container(
        decoration: BoxDecoration(
          color: colors.bgContainer,
          borderRadius: BorderRadius.circular(metrics.listRadius),
          border: Border.all(
            color: colors.border,
            width: metrics.listBorderWidth,
          ),
        ),
        child: ClipRRect(
          borderRadius: BorderRadius.circular(metrics.listRadius),
          child: ListView.separated(
            itemCount: filtered.length,
            separatorBuilder: (_, _) => const MateHDivider(),
            itemBuilder: (context, index) =>
                _LogRecordRow(record: filtered[index]),
          ),
        ),
      ),
    );
  }

  /// 过滤标签选中态主题（对标 CMP：ERROR→error / WARN→warning / INFO→primary / ALL→default）
  MateTagTheme _filterTheme(LevelFilter lv) {
    return switch (lv) {
      LevelFilter.error => MateTagTheme.error,
      LevelFilter.warn => MateTagTheme.warning,
      LevelFilter.info => MateTagTheme.primary,
      LevelFilter.all => MateTagTheme.normal,
    };
  }
}

/// 日志记录行（v2 log-item：级别 tag + 消息 + 等宽元信息）。
class _LogRecordRow extends StatelessWidget {
  final LogRecordDisplay record;

  const _LogRecordRow({required this.record});

  static final DateFormat _timeFormat = DateFormat('yyyy-MM-dd HH:mm:ss');

  /// 级别 → tag 主题映射（对标 CMP levelTheme）
  static MateTagTheme _levelTheme(AppLogLevel level) {
    return switch (level) {
      AppLogLevel.error => MateTagTheme.error,
      AppLogLevel.warn => MateTagTheme.warning,
      AppLogLevel.info => MateTagTheme.primary,
      _ => MateTagTheme.normal,
    };
  }

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).logViewer;
    final typography = MateTheme.typographyOf(context).logViewer;

    final timeStr = _timeFormat.format(
      DateTime.fromMillisecondsSinceEpoch(record.timestampMs),
    );

    return Padding(
      padding: EdgeInsets.symmetric(
        horizontal: metrics.recordHorizontalPadding,
        vertical: metrics.recordVerticalPadding,
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          MateTag(
            label: record.level.label,
            theme: _levelTheme(record.level),
            size: MateTagSize.small,
          ),
          SizedBox(width: metrics.recordContentSpacing),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  record.message,
                  style: typography.recordMessage.copyWith(
                    color: colors.textPrimary,
                  ),
                ),
                Padding(
                  padding: EdgeInsets.only(top: metrics.metadataTopPadding),
                  // meta：时间 · logger（对标 CMP fmtTime · target，v2 等宽字体）
                  child: Text(
                    '$timeStr · ${record.target}',
                    style: typography.recordMetadata.copyWith(
                      color: colors.textSecondary,
                      fontFamily: 'Menlo',
                      fontFamilyFallback: const ['monospace'],
                    ),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

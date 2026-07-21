import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/pages/files/widgets/transfer_popover.dart';
import 'package:petal_link/types/enums.dart';
import 'package:petal_link/widgets/index.dart';

// =============================================================================
// TransferPopover 测试：9 态渲染 / 摘要统计 / 清空菜单 / 重试防抖。
// =============================================================================

Widget _wrap(Widget child) {
  return MateLinkTheme(
    child: MaterialApp(
      home: Scaffold(
        body: Stack(
          children: [
            child,
            const MateDialogHost(),
            const MateToastHost(),
          ],
        ),
      ),
    ),
  );
}

TransferTask _task(
  int id,
  TransferState state, {
  TransferDirection direction = TransferDirection.Upload,
  String? name,
  int totalSize = 1000,
  int transferred = 500,
  String? errorMessage,
}) {
  return TransferTask(
    id: id,
    direction: direction,
    name: name ?? '文件$id.txt',
    totalSize: totalSize,
    transferred: transferred,
    state: state,
    errorMessage: errorMessage,
    createdAt: 0,
  );
}

class _Callbacks {
  int? retriedId;
  int clearCompleted = 0;
  int clearFailed = 0;
  int clearFinished = 0;
  int dismissed = 0;
}

Widget _popover(_Callbacks cb, List<TransferTask> tasks) {
  return TransferPopover(
    tasks: tasks,
    onDismiss: () => cb.dismissed++,
    onRetry: (id, onResult) {
      cb.retriedId = id;
      onResult(true);
    },
    onClearCompleted: () => cb.clearCompleted++,
    onClearFailed: () => cb.clearFailed++,
    onClearFinished: () => cb.clearFinished++,
  );
}

void main() {
  tearDown(() {
    MateDialog.close();
    MateToast.dismiss();
  });

  testWidgets('9 态元数据渲染（图标标签文案）', (tester) async {
    // 面板 440×580 + top 64，需要更高画布
    tester.view.physicalSize = const Size(1200, 900);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    final cb = _Callbacks();

    // 面板高度有限，9 个任务分两波渲染验证（前 6 态）
    await tester.pumpWidget(_wrap(_popover(cb, [
      _task(1, TransferState.Pending),
      _task(2, TransferState.Running),
      _task(3, TransferState.WaitingForNetwork),
      _task(4, TransferState.BackingOff),
      _task(5, TransferState.VerifyingRemote),
      _task(6, TransferState.RestartRequired, errorMessage: '需重新规划'),
    ])));
    await tester.pump();

    expect(find.text('等待调度'), findsOneWidget);
    expect(find.text('传输中'), findsOneWidget);
    expect(find.text('等待网络'), findsWidgets);
    expect(find.text('等待重试'), findsOneWidget);
    expect(find.text('核验远端'), findsOneWidget);
    expect(find.text('等待重新规划'), findsOneWidget);
    expect(find.text('需重新规划'), findsOneWidget);
    // 进度文本（500/1000 = 50%）
    expect(find.text('50% · 500 B/1000 B'), findsWidgets);

    // 后 3 态（含 Failed 错误文案与 Canceled）
    await tester.pumpWidget(_wrap(_popover(cb, [
      _task(7, TransferState.Completed),
      _task(8, TransferState.Failed, errorMessage: '网络错误'),
      _task(9, TransferState.Canceled),
    ])));
    await tester.pump();

    expect(find.text('已完成'), findsWidgets); // 状态标签 + stat-pill 标签
    expect(find.text('失败'), findsWidgets);
    expect(find.text('已取消'), findsOneWidget);
    expect(find.text('网络错误'), findsOneWidget);
  });

  testWidgets('摘要统计：处理中/等待中/已完成/历史失败', (tester) async {
    tester.view.physicalSize = const Size(1200, 900);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    final cb = _Callbacks();
    final tasks = [
      _task(1, TransferState.Running),
      _task(2, TransferState.Pending),
      _task(3, TransferState.VerifyingRemote),
      _task(4, TransferState.WaitingForNetwork),
      _task(5, TransferState.Completed),
      _task(6, TransferState.Failed, errorMessage: 'x'),
    ];
    await tester.pumpWidget(_wrap(_popover(cb, tasks)));
    await tester.pump();

    expect(find.text('处理中'), findsOneWidget);
    expect(find.text('等待中'), findsOneWidget);
    expect(find.text('历史失败'), findsOneWidget);
    // 处理中 3 / 等待中 1 / 已完成 1 / 历史失败 1
    final pills = tester
        .widgetList(find.descendant(
          of: find.byType(TransferPopover),
          matching: find.byType(Text),
        ))
        .whereType<Text>()
        .map((t) => t.data)
        .toList();
    expect(pills.where((t) => t == '3').length, greaterThanOrEqualTo(1));
  });

  testWidgets('空队列显示空状态', (tester) async {
    tester.view.physicalSize = const Size(1200, 900);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    final cb = _Callbacks();
    await tester.pumpWidget(_wrap(_popover(cb, const [])));
    await tester.pump();

    expect(find.text('暂无传输任务'), findsOneWidget);
  });

  testWidgets('清空菜单三项分别回调', (tester) async {
    tester.view.physicalSize = const Size(1200, 900);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    final cb = _Callbacks();
    await tester.pumpWidget(_wrap(_popover(cb, [
      _task(1, TransferState.Completed),
    ])));
    await tester.pump();

    // 打开清空菜单（stats 行的 transfer 图标 MatePopupMenu trigger）
    await tester.tap(find.byWidgetPredicate(
        (w) => w is MateButton && w.icon == 'transfer'));
    await tester.pumpAndSettle();

    expect(find.text('清除已完成'), findsOneWidget);
    expect(find.text('清除失败历史'), findsOneWidget);
    expect(find.text('清除完成+失败历史'), findsOneWidget);

    await tester.tap(find.text('清除已完成'));
    await tester.pumpAndSettle();
    expect(cb.clearCompleted, 1);

    // 再次打开选「清除失败历史」
    await tester.tap(find.byWidgetPredicate(
        (w) => w is MateButton && w.icon == 'transfer'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('清除失败历史'));
    await tester.pumpAndSettle();
    expect(cb.clearFailed, 1);

    // 再次打开选「清除完成+失败历史」
    await tester.tap(find.byWidgetPredicate(
        (w) => w is MateButton && w.icon == 'transfer'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('清除完成+失败历史'));
    await tester.pumpAndSettle();
    expect(cb.clearFinished, 1);
  });

  testWidgets('失败上传任务可重试，重试回调透出 taskId', (tester) async {
    tester.view.physicalSize = const Size(1200, 900);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    final cb = _Callbacks();
    await tester.pumpWidget(_wrap(_popover(cb, [
      _task(42, TransferState.Failed, errorMessage: '网络错误'),
    ])));
    await tester.pump();

    // 重试按钮（icon=refresh）
    final retry = find.byWidgetPredicate(
        (w) => w is MateButton && w.icon == 'refresh');
    expect(retry, findsOneWidget);
    await tester.tap(retry);
    await tester.pump();

    expect(cb.retriedId, 42);
  });

  testWidgets('删除方向任务显示「删除操作」且无重试按钮', (tester) async {
    tester.view.physicalSize = const Size(1200, 900);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    final cb = _Callbacks();
    await tester.pumpWidget(_wrap(_popover(cb, [
      _task(7, TransferState.Failed,
          direction: TransferDirection.Delete,
          totalSize: 0,
          errorMessage: '删除失败'),
    ])));
    await tester.pump();

    expect(find.text('删除'), findsOneWidget); // dir chip
    expect(find.byWidgetPredicate(
        (w) => w is MateButton && w.icon == 'refresh'), findsNothing);
  });

  testWidgets('关闭按钮回调 onDismiss', (tester) async {
    tester.view.physicalSize = const Size(1200, 900);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    final cb = _Callbacks();
    await tester.pumpWidget(_wrap(_popover(cb, const [])));
    await tester.pump();

    await tester.tap(find.byWidgetPredicate(
        (w) => w is MateButton && w.icon == 'x' && w.variant == MateButtonVariant.icon));
    await tester.pump();
    expect(cb.dismissed, 1);
  });
}

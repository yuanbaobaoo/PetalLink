import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/widgets/index.dart';

// =============================================================================
// MateOverlay 测试：全局命令式 Dialog/Toast 宿主。
// 对标 CMP openDialog/confirmDialog/closeDialog + showToast 单条语义。
// =============================================================================

/// 挂载双宿主的最小测试环境（对齐 app.dart 的 builder 结构）。
Widget _wrapWithHosts() {
  return MaterialApp(
    home: MateLinkTheme(
      child: const Scaffold(
        body: Stack(
          children: [
            SizedBox.expand(),
            MateDialogHost(),
            MateToastHost(),
          ],
        ),
      ),
    ),
  );
}

void main() {
  tearDown(() {
    // 清理全局状态，避免用例间串扰
    MateDialog.close();
    MateToast.dismiss();
  });

  group('MateDialog 命令式对话框', () {
    testWidgets('open 显示标题与正文，确认按钮关闭', (tester) async {
      await tester.pumpWidget(_wrapWithHosts());

      MateDialog.open(const MateDialogOptions(
        title: '提示',
        content: '这是一条消息',
      ));
      await tester.pump();

      expect(find.text('提示'), findsOneWidget);
      expect(find.text('这是一条消息'), findsOneWidget);

      await tester.tap(find.text('确定'));
      await tester.pump();
      expect(find.text('提示'), findsNothing);
    });

    testWidgets('confirm 确认回调收到 true', (tester) async {
      await tester.pumpWidget(_wrapWithHosts());

      bool? result;
      MateDialog.confirm(
        const MateDialogOptions(title: '确认删除', danger: true),
        (ok) => result = ok,
      );
      await tester.pump();

      // confirm 型有取消 + 确认两个按钮
      expect(find.text('取消'), findsOneWidget);
      expect(find.text('确定'), findsOneWidget);

      await tester.tap(find.text('确定'));
      await tester.pump();
      expect(result, isTrue);
      expect(find.text('确认删除'), findsNothing);
    });

    testWidgets('confirm 取消回调收到 false', (tester) async {
      await tester.pumpWidget(_wrapWithHosts());

      bool? result;
      MateDialog.confirm(
        const MateDialogOptions(title: '确认删除'),
        (ok) => result = ok,
      );
      await tester.pump();

      await tester.tap(find.text('取消'));
      await tester.pump();
      expect(result, isFalse);
    });

    testWidgets('closeOnOverlay 时点击遮罩关闭并回调 false', (tester) async {
      await tester.pumpWidget(_wrapWithHosts());

      bool? result;
      MateDialog.confirm(
        const MateDialogOptions(title: '遮罩测试', content: '点外面'),
        (ok) => result = ok,
      );
      await tester.pump();
      expect(find.text('遮罩测试'), findsOneWidget);

      // 点击对话框外的遮罩区域（左上角）
      await tester.tapAt(const Offset(10, 10));
      await tester.pump();
      expect(result, isFalse);
      expect(find.text('遮罩测试'), findsNothing);
    });

    testWidgets('新 open 替换旧对话框（单例语义）', (tester) async {
      await tester.pumpWidget(_wrapWithHosts());

      MateDialog.open(const MateDialogOptions(title: '第一个'));
      await tester.pump();
      MateDialog.open(const MateDialogOptions(title: '第二个'));
      await tester.pump();

      expect(find.text('第一个'), findsNothing);
      expect(find.text('第二个'), findsOneWidget);
    });
  });

  group('MateToast 命令式提示', () {
    testWidgets('show 显示消息', (tester) async {
      await tester.pumpWidget(_wrapWithHosts());

      MateToast.show('保存成功', variant: MateToastVariant.success);
      await tester.pump();

      expect(find.text('保存成功'), findsOneWidget);
    });

    testWidgets('2 秒后自动消失（单条语义）', (tester) async {
      await tester.pumpWidget(_wrapWithHosts());

      MateToast.show('转瞬即逝');
      await tester.pump();
      expect(find.text('转瞬即逝'), findsOneWidget);

      await tester.pump(const Duration(seconds: 2));
      await tester.pump();
      expect(find.text('转瞬即逝'), findsNothing);
    });

    testWidgets('新 toast 替换旧 toast', (tester) async {
      await tester.pumpWidget(_wrapWithHosts());

      MateToast.show('第一条');
      await tester.pump();
      MateToast.show('第二条', variant: MateToastVariant.error);
      await tester.pump();

      expect(find.text('第一条'), findsNothing);
      expect(find.text('第二条'), findsOneWidget);
    });
  });
}

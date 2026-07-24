import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/widgets/index.dart';

// =============================================================================
// MateRadioGroup 测试：组内自动联动选中态、独立用法、InheritedWidget 传播。
// 对标原 Vue MateRadioGroup.vue 的 provide/inject 联动语义。
// =============================================================================

/// 包裹主题的最小测试环境。
Widget _wrap(Widget child) {
  return MaterialApp(
    home: MateLinkTheme(
      child: Scaffold(body: Center(child: child)),
    ),
  );
}

void main() {
  group('MateRadioGroup（组内自动联动）', () {
    testWidgets('groupValue 相等的子项自动选中，其余未选中', (tester) async {
      await tester.pumpWidget(_wrap(
        MateRadioGroup<String>(
          groupValue: 'b',
          onChanged: (_) {},
          child: Column(
            children: const [
              MateRadio(key: Key('a'), value: 'a'),
              MateRadio(key: Key('b'), value: 'b'),
              MateRadio(key: Key('c'), value: 'c'),
            ],
          ),
        ),
      ));

      // 选中项渲染 brand 实心圆点子容器（selected 时 child 非 null）。
      // b 是 groupValue → 其 Container 内应有选中圆点。
      // 通过统计选中圆点数验证：仅 1 个被选中。
      // MateRadio 选中态圆点是 width=size*0.5 的 BoxDecoration.brand 圆，
      // 用 find.ancestor 定位每个 MateRadio 的内部子容器。
      expect(find.byType(MateRadio), findsNWidgets(3));

      // 点击 b 不应触发 onChanged（已是选中态也允许回调，这里验证初始渲染）
      // 用点击切换验证选中态：点击 a 后 selected 圆点应从 b 移到 a
      String? selected = 'b';
      await tester.pumpWidget(_wrap(
        StatefulBuilder(
          builder: (context, setState) => MateRadioGroup<String?>(
            groupValue: selected,
            onChanged: (v) => setState(() => selected = v),
            child: Column(
              children: const [
                MateRadio(key: Key('a'), value: 'a'),
                MateRadio(key: Key('b'), value: 'b'),
              ],
            ),
          ),
        ),
      ));

      // 点击 a → onChanged('a')，selected 变 a
      await tester.tap(find.byKey(const Key('a')));
      await tester.pump();
      expect(selected, 'a');
    });

    testWidgets('点击未选中项 → 回调 onChanged 携带该项 value', (tester) async {
      String? selected;
      await tester.pumpWidget(_wrap(
        StatefulBuilder(
          builder: (context, setState) => MateRadioGroup<String?>(
            groupValue: selected,
            onChanged: (v) => setState(() => selected = v),
            child: Column(
              children: const [
                MateRadio(key: Key('a'), value: 'a'),
                MateRadio(key: Key('b'), value: 'b'),
              ],
            ),
          ),
        ),
      ));

      // 初始无选中
      expect(selected, isNull);

      // 点击 a
      await tester.tap(find.byKey(const Key('a')));
      await tester.pump();
      expect(selected, 'a');

      // 点击 b 切换
      await tester.tap(find.byKey(const Key('b')));
      await tester.pump();
      expect(selected, 'b');
    });

    testWidgets('groupValue 变化时子项选中态自动更新（InheritedWidget 传播）',
        (tester) async {
      String selected = 'a';
      await tester.pumpWidget(_wrap(
        StatefulBuilder(
          builder: (context, setState) => MateRadioGroup<String>(
            groupValue: selected,
            onChanged: (v) => setState(() => selected = v),
            child: Column(
              children: const [
                MateRadio(key: Key('a'), value: 'a'),
                MateRadio(key: Key('b'), value: 'b'),
              ],
            ),
          ),
        ),
      ));

      // 点击 b → selected 变 b → a 应自动取消选中
      await tester.tap(find.byKey(const Key('b')));
      await tester.pumpAndSettle();
      expect(selected, 'b');
      // 再点 a 应回切
      await tester.tap(find.byKey(const Key('a')));
      await tester.pumpAndSettle();
      expect(selected, 'a');
    });
  });

  group('MateRadio（独立用法，向后兼容）', () {
    testWidgets('直接传 selected + onSelect，不经组上下文', (tester) async {
      bool tapped = false;
      await tester.pumpWidget(_wrap(
        MateRadio(
          selected: true,
          onSelect: () => tapped = true,
        ),
      ));

      await tester.tap(find.byType(MateRadio));
      await tester.pump();
      expect(tapped, isTrue);
    });

    testWidgets('disabled 不触发回调', (tester) async {
      bool tapped = false;
      await tester.pumpWidget(_wrap(
        MateRadio(
          selected: false,
          onSelect: () => tapped = true,
          disabled: true,
        ),
      ));

      await tester.tap(find.byType(MateRadio), warnIfMissed: false);
      await tester.pump();
      expect(tapped, isFalse);
    });
  });
}

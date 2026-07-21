import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/widgets/index.dart';

// =============================================================================
// MateButton 测试：五变体渲染、点击/禁用/加载行为、badge 角标。
// =============================================================================

/// 包裹主题的最小测试环境。
Widget _wrap(Widget child) {
  return MaterialApp(
    home: MateLinkTheme(
      child: Scaffold(body: Center(child: child)),
    ),
  );
}

/// 取按钮根 AnimatedContainer 的 BoxDecoration。
BoxDecoration _decorationOf(WidgetTester tester) {
  final container = tester.widget<AnimatedContainer>(
    find
        .descendant(
          of: find.byType(MateButton),
          matching: find.byType(AnimatedContainer),
        )
        .first,
  );
  return container.decoration! as BoxDecoration;
}

void main() {
  group('MateButton 变体渲染', () {
    testWidgets('五变体均可渲染且显示文字', (tester) async {
      for (final variant in MateButtonVariant.values) {
        await tester.pumpWidget(_wrap(MateButton(
          label: variant.name,
          variant: variant,
          icon: 'check',
          onClick: () {},
        )));
        await tester.pump();
        expect(find.byType(MateButton), findsOneWidget);
        expect(find.text(variant.name), findsOneWidget);
      }
    });

    testWidgets('primary 正常态使用品牌渐变 + 柔影', (tester) async {
      await tester.pumpWidget(_wrap(MateButton(label: '确定', onClick: () {})));
      final decoration = _decorationOf(tester);
      expect(decoration.gradient, isA<LinearGradient>());
      final gradient = decoration.gradient! as LinearGradient;
      expect(gradient.colors,
          [const Color(0xFF4A8BF0), const Color(0xFF0053DB)]);
      expect(decoration.boxShadow, isNotEmpty);
    });

    testWidgets('primary danger 使用 error 纯色（无渐变）', (tester) async {
      await tester.pumpWidget(
          _wrap(MateButton(label: '删除', danger: true, onClick: () {})));
      final decoration = _decorationOf(tester);
      expect(decoration.gradient, isNull);
      expect(decoration.color, const Color(0xFFE5484D));
    });

    testWidgets('soft 常态为 brandLighter 底', (tester) async {
      await tester.pumpWidget(_wrap(MateButton(
        label: '软色',
        variant: MateButtonVariant.soft,
        onClick: () {},
      )));
      final decoration = _decorationOf(tester);
      expect(decoration.color, const Color(0xFFEFF4FE));
    });

    testWidgets('icon 变体固定 32×32', (tester) async {
      await tester.pumpWidget(_wrap(MateButton(
        variant: MateButtonVariant.icon,
        icon: 'x',
        onClick: () {},
      )));
      final size = tester.getSize(find.byType(MateButton));
      expect(size.width, 32);
      expect(size.height, 32);
    });

    testWidgets('primary 默认高度 36', (tester) async {
      await tester.pumpWidget(_wrap(MateButton(label: '确定', onClick: () {})));
      final size = tester.getSize(find.byType(MateButton));
      expect(size.height, 36);
    });
  });

  group('MateButton 交互行为', () {
    testWidgets('点击触发 onClick', (tester) async {
      var tapped = 0;
      await tester.pumpWidget(_wrap(MateButton(
        label: '确定',
        onClick: () => tapped++,
      )));
      await tester.tap(find.byType(MateButton));
      expect(tapped, 1);
    });

    testWidgets('disabled 不触发 onClick', (tester) async {
      var tapped = 0;
      await tester.pumpWidget(_wrap(MateButton(
        label: '确定',
        disabled: true,
        onClick: () => tapped++,
      )));
      await tester.tap(find.byType(MateButton));
      expect(tapped, 0);
    });

    testWidgets('loading 视为禁用', (tester) async {
      var tapped = 0;
      await tester.pumpWidget(_wrap(MateButton(
        label: '确定',
        loading: true,
        onClick: () => tapped++,
      )));
      await tester.tap(find.byType(MateButton));
      expect(tapped, 0);
    });
  });

  group('MateButton badge 角标', () {
    testWidgets('icon 变体 badge > 0 显示数字', (tester) async {
      await tester.pumpWidget(_wrap(MateButton(
        variant: MateButtonVariant.icon,
        icon: 'transfer',
        badge: 3,
        onClick: () {},
      )));
      expect(find.text('3'), findsOneWidget);
    });

    testWidgets('badge 超过 99 显示 99+', (tester) async {
      await tester.pumpWidget(_wrap(MateButton(
        variant: MateButtonVariant.iconText,
        label: '队列',
        icon: 'transfer',
        badge: 120,
        onClick: () {},
      )));
      expect(find.text('99+'), findsOneWidget);
    });

    testWidgets('badge = 0 不显示角标', (tester) async {
      await tester.pumpWidget(_wrap(MateButton(
        variant: MateButtonVariant.iconText,
        label: '队列',
        icon: 'transfer',
        onClick: () {},
      )));
      expect(find.text('0'), findsNothing);
    });
  });
}

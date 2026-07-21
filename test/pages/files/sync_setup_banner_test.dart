import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/pages/files/widgets/sync_setup_banner.dart';
import 'package:petal_link/widgets/index.dart';

// =============================================================================
// FilesSyncSetupBanner 测试：三态互斥（error 优先 > needsSetup > needsFirstSync）
// + active/loading 不显示。
// =============================================================================

Widget _wrap(Widget child) {
  return MateLinkTheme(
    child: MaterialApp(
      home: Scaffold(body: Column(children: [child])),
    ),
  );
}

void main() {
  testWidgets('needsSetup：info 引导 + 选择目录按钮', (tester) async {
    var selected = 0;
    await tester.pumpWidget(_wrap(FilesSyncSetupBanner(
      setupPhase: SetupPhase.needsSetup,
      mountDir: '',
      onSelectDir: () => selected++,
      onFirstSync: () {},
      onRetry: () {},
    )));
    await tester.pump();

    expect(find.text('尚未配置同步目录，选择一个空目录开始同步'), findsOneWidget);
    await tester.tap(find.text('选择目录'));
    await tester.pump();
    expect(selected, 1);
  });

  testWidgets('needsFirstSync：warning 引导（含挂载目录）+ 同步索引按钮', (tester) async {
    var firstSync = 0;
    await tester.pumpWidget(_wrap(FilesSyncSetupBanner(
      setupPhase: SetupPhase.needsFirstSync,
      mountDir: '/Users/test/PetalLink',
      onSelectDir: () {},
      onFirstSync: () => firstSync++,
      onRetry: () {},
    )));
    await tester.pump();

    expect(find.textContaining('/Users/test/PetalLink'), findsOneWidget);
    expect(find.textContaining('点击「同步索引」拉取云端索引'), findsOneWidget);
    await tester.tap(find.text('同步索引'));
    await tester.pump();
    expect(firstSync, 1);
  });

  testWidgets('error 优先于 setupPhase：错误横幅 + 重试按钮', (tester) async {
    var retried = 0;
    await tester.pumpWidget(_wrap(FilesSyncSetupBanner(
      setupPhase: SetupPhase.needsSetup,
      mountDir: '',
      errorMessage: '同步引擎启动失败',
      onSelectDir: () {},
      onFirstSync: () {},
      onRetry: () => retried++,
    )));
    await tester.pump();

    // error 态优先，needsSetup 引导不显示
    expect(find.text('同步引擎启动失败'), findsOneWidget);
    expect(find.text('尚未配置同步目录，选择一个空目录开始同步'), findsNothing);

    await tester.tap(find.text('重试'));
    await tester.pump();
    expect(retried, 1);
  });

  testWidgets('active / loading 不显示引导条', (tester) async {
    await tester.pumpWidget(_wrap(FilesSyncSetupBanner(
      setupPhase: SetupPhase.active,
      mountDir: '/x',
      onSelectDir: () {},
      onFirstSync: () {},
      onRetry: () {},
    )));
    await tester.pump();
    expect(find.byType(MateInfoBanner), findsNothing);

    await tester.pumpWidget(_wrap(FilesSyncSetupBanner(
      setupPhase: SetupPhase.loading,
      mountDir: '',
      onSelectDir: () {},
      onFirstSync: () {},
      onRetry: () {},
    )));
    await tester.pump();
    expect(find.byType(MateInfoBanner), findsNothing);
  });
}

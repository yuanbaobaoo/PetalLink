import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/entity/sync_state.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/pages/files/widgets/sync_status_bar.dart';
import 'package:petal_link/types/enums.dart';
import 'package:petal_link/widgets/index.dart';

// =============================================================================
// FilesSyncStatusBar 测试：9 种 syncPhase 文案 / 空闲细分 / 统计标签 / 失败项弹层。
// =============================================================================

Widget _wrap(Widget child) {
  return MateLinkTheme(
    child: MaterialApp(
      home: Scaffold(
        body: Stack(
          children: [
            Column(children: [child]),
            const MateDialogHost(),
            const MateToastHost(),
          ],
        ),
      ),
    ),
  );
}

TransferTask _transfer(TransferState state) => TransferTask(
      id: 1,
      name: 'a.txt',
      state: state,
      createdAt: 0,
    );

void main() {
  tearDown(() {
    MateDialog.close();
    MateToast.dismiss();
  });

  group('9 种 syncPhase 文案', () {
    final cases = <SyncPhase, String>{
      SyncPhase.IndexingStartup: '正在读取云端索引（首次）…',
      SyncPhase.IndexingManual: '正在读取云端索引…',
      SyncPhase.IndexingAutoFull: '正在读取云端索引（全量纠偏）…',
      SyncPhase.QueryingChanges: '正在查询云端变更…',
      SyncPhase.SyncingAutoIncremental: '正在同步云端变更…',
      SyncPhase.SyncingLocal: '正在同步本地变更…',
      SyncPhase.SyncingManual: '正在同步…',
      SyncPhase.SyncingRetry: '正在重试失败项…',
      SyncPhase.SyncingStartup: '正在同步（启动恢复）…',
    };

    for (final entry in cases.entries) {
      testWidgets('${entry.key.wireName} → ${entry.value}', (tester) async {
        await tester.pumpWidget(_wrap(FilesSyncStatusBar(
          sync: SyncGlobalState(
            isRunning: true,
            syncPhase: entry.key,
          ),
        )));
        await tester.pump();

        expect(find.text(entry.value), findsOneWidget);
      });
    }
  });

  group('空闲态细分文案', () {
    testWidgets('无活跃 → 同步完成 + 上次同步时间', (tester) async {
      await tester.pumpWidget(_wrap(FilesSyncStatusBar(
        sync: SyncGlobalState(
          lastSyncTime: DateTime(2026, 7, 20, 14, 32).millisecondsSinceEpoch,
        ),
      )));
      await tester.pump();

      expect(find.text('同步完成'), findsOneWidget);
      expect(find.textContaining('上次同步 14:32'), findsOneWidget);
    });

    testWidgets('上传/下载中 → 同步中', (tester) async {
      await tester.pumpWidget(_wrap(const FilesSyncStatusBar(
        sync: SyncGlobalState(uploading: 2),
      )));
      await tester.pump();

      expect(find.text('同步中'), findsOneWidget);
      expect(find.text('上传 2'), findsOneWidget);
    });

    testWidgets('传输细分：核验远端/退避/重规划/等待传输', (tester) async {
      final cases = <TransferState, String>{
        TransferState.VerifyingRemote: '正在核验远端…',
        TransferState.BackingOff: '等待下次重试…',
        TransferState.RestartRequired: '等待重新规划…',
        TransferState.Pending: '等待传输…',
      };
      for (final entry in cases.entries) {
        await tester.pumpWidget(_wrap(FilesSyncStatusBar(
          sync: const SyncGlobalState(),
          transfers: [_transfer(entry.key)],
        )));
        await tester.pump();
        expect(find.text(entry.value), findsOneWidget,
            reason: '${entry.key} 应显示 ${entry.value}');
      }
    });

    testWidgets('等待网络 → 等待网络恢复… + 警告标签', (tester) async {
      await tester.pumpWidget(_wrap(const FilesSyncStatusBar(
        sync: SyncGlobalState(waitingNetwork: 3),
      )));
      await tester.pump();

      expect(find.text('等待网络恢复…'), findsOneWidget);
      expect(find.text('等待网络 3'), findsOneWidget);
    });

    testWidgets('失败 > 0 → 同步存在失败项 + 可点错误标签', (tester) async {
      await tester.pumpWidget(_wrap(const FilesSyncStatusBar(
        sync: SyncGlobalState(
          failed: 2,
          failedItems: [
            FailedItem(relativePath: '文档/a.txt', errorMessage: '网络错误'),
            FailedItem(relativePath: '图片/b.png'),
          ],
        ),
      )));
      await tester.pump();

      expect(find.text('同步存在失败项'), findsOneWidget);
      expect(find.text('同步失败 2'), findsOneWidget);

      // 点击失败标签 → 失败项弹窗
      await tester.tap(find.text('同步失败 2'));
      await tester.pumpAndSettle();

      expect(find.text('同步失败项 (2)'), findsOneWidget);
      expect(find.textContaining('文档/a.txt'), findsOneWidget);
      expect(find.textContaining('图片/b.png'), findsOneWidget);

      await tester.tap(find.text('关闭'));
      await tester.pumpAndSettle();
      expect(find.text('同步失败项 (2)'), findsNothing);
    });

    testWidgets('编辑中/冲突标签', (tester) async {
      await tester.pumpWidget(_wrap(const FilesSyncStatusBar(
        sync: SyncGlobalState(editing: 1, conflict: 2),
      )));
      await tester.pump();

      expect(find.text('编辑中 1'), findsOneWidget);
      expect(find.text('冲突 2'), findsOneWidget);
    });
  });
}

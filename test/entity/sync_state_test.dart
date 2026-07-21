import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/entity/sync_state.dart';
import 'package:petal_link/types/enums.dart';

void main() {
  group('FailedItem', () {
    test('fromJson / toJson 往返（snake_case 键）', () {
      const item = FailedItem(
        relativePath: 'dir/a.txt',
        errorMessage: '网络超时',
      );

      final json = item.toJson();
      expect(json['relative_path'], 'dir/a.txt');
      expect(json['error_message'], '网络超时');

      final restored = FailedItem.fromJson(json);
      expect(restored.relativePath, item.relativePath);
      expect(restored.errorMessage, item.errorMessage);
    });

    test('errorMessage 可空', () {
      const item = FailedItem(relativePath: 'a.txt');

      expect(FailedItem.fromJson(item.toJson()).errorMessage, isNull);
    });
  });

  group('SyncGlobalState', () {
    test('默认值对齐 Rust Default（全零 / 空闲）', () {
      const state = SyncGlobalState();

      expect(state.revision, 0);
      expect(state.total, 0);
      expect(state.completed, 0);
      expect(state.uploading, 0);
      expect(state.downloading, 0);
      expect(state.waitingNetwork, 0);
      expect(state.failed, 0);
      expect(state.transferFailed, 0);
      expect(state.failedItems, isEmpty);
      expect(state.conflict, 0);
      expect(state.editing, 0);
      expect(state.isRunning, isFalse);
      expect(state.lastSyncTime, isNull);
      expect(state.isIndexing, isFalse);
      expect(state.indexingScannedFolders, 0);
      expect(state.indexingDiscoveredItems, 0);
      expect(state.contentChanged, isFalse);
      expect(state.syncPhase, isNull);
    });

    test('maxFailedItems 常量 = 20（对齐 Rust 注释）', () {
      expect(SyncGlobalState.maxFailedItems, 20);
    });

    group('progress', () {
      test('total 为 0 时返回 1.0（对齐 Rust）', () {
        const state = SyncGlobalState();

        expect(state.progress, 1.0);
      });

      test('completed / total', () {
        const state = SyncGlobalState(total: 10, completed: 3);

        expect(state.progress, closeTo(0.3, 1e-9));
      });
    });

    group('fromJson / toJson', () {
      test('全字段往返保持一致（snake_case 键）', () {
        const state = SyncGlobalState(
          revision: 42,
          total: 100,
          completed: 60,
          uploading: 2,
          downloading: 3,
          waitingNetwork: 1,
          failed: 5,
          transferFailed: 7,
          failedItems: [
            FailedItem(relativePath: 'a.txt', errorMessage: '超时'),
            FailedItem(relativePath: 'b.txt'),
          ],
          conflict: 1,
          editing: 2,
          isRunning: true,
          lastSyncTime: 1750000000000,
          isIndexing: true,
          indexingScannedFolders: 10,
          indexingDiscoveredItems: 200,
          contentChanged: true,
          syncPhase: SyncPhase.SyncingManual,
        );

        final json = state.toJson();
        expect(json['revision'], 42);
        expect(json['waiting_network'], 1);
        expect(json['transfer_failed'], 7);
        expect(json['is_running'], true);
        expect(json['last_sync_time'], 1750000000000);
        expect(json['indexing_scanned_folders'], 10);
        expect(json['sync_phase'], 'syncing-manual');
        expect((json['failed_items'] as List).length, 2);

        final restored = SyncGlobalState.fromJson(json);
        expect(restored.revision, 42);
        expect(restored.total, 100);
        expect(restored.completed, 60);
        expect(restored.uploading, 2);
        expect(restored.downloading, 3);
        expect(restored.waitingNetwork, 1);
        expect(restored.failed, 5);
        expect(restored.transferFailed, 7);
        expect(restored.failedItems.length, 2);
        expect(restored.failedItems[0].relativePath, 'a.txt');
        expect(restored.failedItems[0].errorMessage, '超时');
        expect(restored.failedItems[1].errorMessage, isNull);
        expect(restored.conflict, 1);
        expect(restored.editing, 2);
        expect(restored.isRunning, isTrue);
        expect(restored.lastSyncTime, 1750000000000);
        expect(restored.isIndexing, isTrue);
        expect(restored.indexingScannedFolders, 10);
        expect(restored.indexingDiscoveredItems, 200);
        expect(restored.contentChanged, isTrue);
        expect(restored.syncPhase, SyncPhase.SyncingManual);
      });

      test('syncPhase 为 null 时不输出键（对齐 skip_serializing_if）', () {
        const state = SyncGlobalState();

        expect(state.toJson().containsKey('sync_phase'), isFalse);
      });

      test('fromJson 未知 sync_phase 字符串 → null', () {
        final state = SyncGlobalState.fromJson({'sync_phase': 'unknown'});

        expect(state.syncPhase, isNull);
      });

      test('fromJson 容忍 String 数字', () {
        final state = SyncGlobalState.fromJson({
          'revision': '42',
          'total': '100',
          'last_sync_time': '1750000000000',
        });

        expect(state.revision, 42);
        expect(state.total, 100);
        expect(state.lastSyncTime, 1750000000000);
      });
    });

    group('copyWith', () {
      test('替换字段并可显式清空可空字段', () {
        const state = SyncGlobalState(
          revision: 1,
          lastSyncTime: 1000,
          syncPhase: SyncPhase.IndexingStartup,
        );

        final copy = state.copyWith(revision: 2, total: 10);
        expect(copy.revision, 2);
        expect(copy.total, 10);
        expect(copy.lastSyncTime, 1000);
        expect(copy.syncPhase, SyncPhase.IndexingStartup);

        final cleared = state.copyWith(lastSyncTime: null, syncPhase: null);
        expect(cleared.lastSyncTime, isNull);
        expect(cleared.syncPhase, isNull);
      });
    });
  });
}

import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/types/enums.dart';

void main() {
  group('TransferState', () {
    test('has exactly 9 values', () {
      expect(TransferState.values.length, 9);
    });

    test('persistent codes match Rust TransferState (0-8)', () {
      final expected = {
        TransferState.pending: 0,
        TransferState.running: 1,
        TransferState.waitingForNetwork: 2,
        TransferState.backingOff: 3,
        TransferState.verifyingRemote: 4,
        TransferState.restartRequired: 5,
        TransferState.completed: 6,
        TransferState.failed: 7,
        TransferState.canceled: 8,
      };
      for (final entry in expected.entries) {
        expect(entry.key.code, entry.value,
            reason: '${entry.key.name}.code 应为 ${entry.value}');
        expect(TransferState.fromCode(entry.value), entry.key);
      }
    });

    test('fromCode returns null for unknown code', () {
      expect(TransferState.fromCode(9), isNull);
      expect(TransferState.fromCode(-1), isNull);
    });

    // 对齐 Rust can_transition 的合法转移表（34 条边）
    group('canTransition', () {
      final expectedEdges = <TransferState, Set<TransferState>>{
        TransferState.pending: {
          TransferState.running,
          TransferState.waitingForNetwork,
          TransferState.restartRequired,
          TransferState.failed,
          TransferState.canceled,
        },
        TransferState.running: {
          TransferState.waitingForNetwork,
          TransferState.backingOff,
          TransferState.verifyingRemote,
          TransferState.restartRequired,
          TransferState.completed,
          TransferState.failed,
          TransferState.canceled,
        },
        TransferState.waitingForNetwork: {
          TransferState.running,
          TransferState.restartRequired,
          TransferState.failed,
          TransferState.canceled,
        },
        TransferState.backingOff: {
          TransferState.running,
          TransferState.restartRequired,
          TransferState.failed,
          TransferState.canceled,
        },
        TransferState.verifyingRemote: {
          TransferState.running,
          TransferState.waitingForNetwork,
          TransferState.backingOff,
          TransferState.restartRequired,
          TransferState.completed,
          TransferState.failed,
          TransferState.canceled,
        },
        TransferState.restartRequired: {
          TransferState.pending,
          TransferState.verifyingRemote,
          TransferState.failed,
          TransferState.canceled,
        },
        TransferState.failed: {
          TransferState.pending,
          TransferState.restartRequired,
          TransferState.canceled,
        },
        TransferState.completed: {},
        TransferState.canceled: {},
      };

      test('exhaustive 9x9 matrix matches Rust transition table', () {
        var edgeCount = 0;
        for (final from in TransferState.values) {
          for (final to in TransferState.values) {
            final expected = expectedEdges[from]!.contains(to);
            if (expected) edgeCount++;
            expect(
              from.canTransition(to),
              expected,
              reason: '${from.name} → ${to.name} 应为 $expected',
            );
          }
        }
        // Rust 表共 34 条边
        expect(edgeCount, 34);
      });

      test('no self transitions', () {
        for (final s in TransferState.values) {
          expect(s.canTransition(s), isFalse,
              reason: '${s.name} 不应允许自转移');
        }
      });

      test('terminal states have no outgoing edges', () {
        for (final to in TransferState.values) {
          expect(TransferState.completed.canTransition(to), isFalse);
          expect(TransferState.canceled.canTransition(to), isFalse);
        }
      });

      test('Failed can be retried via Pending or RestartRequired', () {
        expect(TransferState.failed.canTransition(TransferState.pending),
            isTrue);
        expect(
            TransferState.failed.canTransition(TransferState.restartRequired),
            isTrue);
        expect(TransferState.failed.canTransition(TransferState.running),
            isFalse);
      });
    });

    group('displayName', () {
      test('returns Chinese names for all 9 values', () {
        final expectedNames = {
          TransferState.pending: '等待中',
          TransferState.running: '传输中',
          TransferState.waitingForNetwork: '等待网络',
          TransferState.backingOff: '退避重试',
          TransferState.verifyingRemote: '远端校验',
          TransferState.restartRequired: '需重新开始',
          TransferState.completed: '已完成',
          TransferState.failed: '失败',
          TransferState.canceled: '已取消',
        };

        for (final entry in expectedNames.entries) {
          expect(
            entry.key.displayName,
            entry.value,
            reason: '${entry.key.name}.displayName should be "${entry.value}"',
          );
        }
      });
    });

    group('isTerminal', () {
      test('returns true for Completed, Failed, Canceled', () {
        expect(TransferState.completed.isTerminal, isTrue);
        expect(TransferState.failed.isTerminal, isTrue);
        expect(TransferState.canceled.isTerminal, isTrue);
      });

      test('returns false for non-terminal states', () {
        expect(TransferState.pending.isTerminal, isFalse);
        expect(TransferState.running.isTerminal, isFalse);
        expect(TransferState.waitingForNetwork.isTerminal, isFalse);
        expect(TransferState.backingOff.isTerminal, isFalse);
        expect(TransferState.verifyingRemote.isTerminal, isFalse);
        expect(TransferState.restartRequired.isTerminal, isFalse);
      });
    });

    group('isActive', () {
      test('returns true for Running and VerifyingRemote', () {
        expect(TransferState.running.isActive, isTrue);
        expect(TransferState.verifyingRemote.isActive, isTrue);
      });

      test('returns false for non-active states', () {
        expect(TransferState.pending.isActive, isFalse);
        expect(TransferState.waitingForNetwork.isActive, isFalse);
        expect(TransferState.backingOff.isActive, isFalse);
        expect(TransferState.restartRequired.isActive, isFalse);
        expect(TransferState.completed.isActive, isFalse);
        expect(TransferState.failed.isActive, isFalse);
        expect(TransferState.canceled.isActive, isFalse);
      });
    });
  });

  group('TransferDirection', () {
    test('persistent codes match Rust transfer_direction (0-3)', () {
      expect(TransferDirection.upload.code, 0);
      expect(TransferDirection.download.code, 1);
      expect(TransferDirection.delete.code, 2);
      expect(TransferDirection.downloadUpdate.code, 3);
    });

    test('fromCode roundtrip and unknown', () {
      for (final d in TransferDirection.values) {
        expect(TransferDirection.fromCode(d.code), d);
      }
      expect(TransferDirection.fromCode(4), isNull);
    });

    test('isDownload covers Download and DownloadUpdate', () {
      expect(TransferDirection.download.isDownload, isTrue);
      expect(TransferDirection.downloadUpdate.isDownload, isTrue);
      expect(TransferDirection.upload.isDownload, isFalse);
      expect(TransferDirection.delete.isDownload, isFalse);
    });
  });

  group('TransferOperation', () {
    test('persistent codes match Rust TransferOperation (0-7)', () {
      final expected = {
        TransferOperation.create: 0,
        TransferOperation.update: 1,
        TransferOperation.download: 2,
        TransferOperation.downloadUpdate: 3,
        TransferOperation.delete: 4,
        TransferOperation.move: 5,
        TransferOperation.rename: 6,
        TransferOperation.createFolder: 7,
      };
      for (final entry in expected.entries) {
        expect(entry.key.code, entry.value);
        expect(TransferOperation.fromCode(entry.value), entry.key);
      }
      expect(TransferOperation.fromCode(8), isNull);
    });
  });

  group('TransferErrorKind', () {
    test('persistent codes match Rust TransferErrorKind (0-11)', () {
      final expected = {
        TransferErrorKind.network: 0,
        TransferErrorKind.timeout: 1,
        TransferErrorKind.auth: 2,
        TransferErrorKind.rateLimit: 3,
        TransferErrorKind.server: 4,
        TransferErrorKind.quota: 5,
        TransferErrorKind.permission: 6,
        TransferErrorKind.validation: 7,
        TransferErrorKind.sessionExpired: 8,
        TransferErrorKind.remoteAmbiguous: 9,
        TransferErrorKind.localChanged: 10,
        TransferErrorKind.unknown: 11,
      };
      expect(TransferErrorKind.values.length, 12);
      for (final entry in expected.entries) {
        expect(entry.key.code, entry.value);
        expect(TransferErrorKind.fromCode(entry.value), entry.key);
      }
      expect(TransferErrorKind.fromCode(12), isNull);
    });
  });

  group('SyncItemStatus', () {
    test('persistent codes match Rust sync_status（6 空缺，Deleted=7）', () {
      final expected = {
        SyncItemStatus.synced: 0,
        SyncItemStatus.cloudOnly: 1,
        SyncItemStatus.localOnly: 2,
        SyncItemStatus.syncing: 3,
        SyncItemStatus.failed: 4,
        SyncItemStatus.conflict: 5,
        SyncItemStatus.deleted: 7,
      };
      for (final entry in expected.entries) {
        expect(entry.key.code, entry.value);
        expect(SyncItemStatus.fromCode(entry.value), entry.key);
      }
      // 6 为空缺值
      expect(SyncItemStatus.fromCode(6), isNull);
      expect(SyncItemStatus.fromCode(8), isNull);
    });
  });

  group('SyncPhase', () {
    test('wireName matches Rust sync_phase 字符串协议', () {
      final expected = {
        SyncPhase.indexingStartup: 'indexing-startup',
        SyncPhase.indexingManual: 'indexing-manual',
        SyncPhase.indexingAutoFull: 'indexing-auto-full',
        SyncPhase.queryingChanges: 'querying-changes',
        SyncPhase.syncingAutoIncremental: 'syncing-auto-incremental',
        SyncPhase.syncingLocal: 'syncing-local',
        SyncPhase.syncingManual: 'syncing-manual',
        SyncPhase.syncingRetry: 'syncing-retry',
        SyncPhase.syncingStartup: 'syncing-startup',
      };
      expect(SyncPhase.values.length, 9);
      for (final entry in expected.entries) {
        expect(entry.key.wireName, entry.value);
        expect(SyncPhase.fromWireName(entry.value), entry.key);
      }
    });

    test('fromWireName returns null for unknown or null', () {
      expect(SyncPhase.fromWireName('unknown-phase'), isNull);
      expect(SyncPhase.fromWireName(null), isNull);
    });

    test('isIndexing only for indexing phases', () {
      expect(SyncPhase.indexingStartup.isIndexing, isTrue);
      expect(SyncPhase.indexingManual.isIndexing, isTrue);
      expect(SyncPhase.indexingAutoFull.isIndexing, isTrue);
      expect(SyncPhase.queryingChanges.isIndexing, isFalse);
      expect(SyncPhase.syncingLocal.isIndexing, isFalse);
    });
  });

  group('SyncStatus', () {
    test('has exactly 5 values', () {
      expect(SyncStatus.values.length, 5);
      expect(SyncStatus.values, contains(SyncStatus.idle));
      expect(SyncStatus.values, contains(SyncStatus.scanning));
      expect(SyncStatus.values, contains(SyncStatus.syncing));
      expect(SyncStatus.values, contains(SyncStatus.offline));
      expect(SyncStatus.values, contains(SyncStatus.error));
    });
  });

  group('AuthStatus', () {
    test('has exactly 5 values', () {
      expect(AuthStatus.values.length, 5);
      expect(AuthStatus.values, contains(AuthStatus.init));
      expect(AuthStatus.values, contains(AuthStatus.authorizing));
      expect(AuthStatus.values, contains(AuthStatus.authorized));
      expect(AuthStatus.values, contains(AuthStatus.unauthorized));
      expect(AuthStatus.values, contains(AuthStatus.error));
    });
  });

  group('AppPage', () {
    test('has exactly 5 values', () {
      expect(AppPage.values.length, 5);
      expect(AppPage.values, contains(AppPage.login));
      expect(AppPage.values, contains(AppPage.files));
      expect(AppPage.values, contains(AppPage.settings));
      expect(AppPage.values, contains(AppPage.logs));
      expect(AppPage.values, contains(AppPage.update));
    });
  });
}

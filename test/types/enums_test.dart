import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/types/enums.dart';

void main() {
  group('TransferState', () {
    test('has exactly 9 values', () {
      expect(TransferState.values.length, 9);
    });

    test('persistent codes match Rust TransferState (0-8)', () {
      final expected = {
        TransferState.Pending: 0,
        TransferState.Running: 1,
        TransferState.WaitingForNetwork: 2,
        TransferState.BackingOff: 3,
        TransferState.VerifyingRemote: 4,
        TransferState.RestartRequired: 5,
        TransferState.Completed: 6,
        TransferState.Failed: 7,
        TransferState.Canceled: 8,
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
        TransferState.Pending: {
          TransferState.Running,
          TransferState.WaitingForNetwork,
          TransferState.RestartRequired,
          TransferState.Failed,
          TransferState.Canceled,
        },
        TransferState.Running: {
          TransferState.WaitingForNetwork,
          TransferState.BackingOff,
          TransferState.VerifyingRemote,
          TransferState.RestartRequired,
          TransferState.Completed,
          TransferState.Failed,
          TransferState.Canceled,
        },
        TransferState.WaitingForNetwork: {
          TransferState.Running,
          TransferState.RestartRequired,
          TransferState.Failed,
          TransferState.Canceled,
        },
        TransferState.BackingOff: {
          TransferState.Running,
          TransferState.RestartRequired,
          TransferState.Failed,
          TransferState.Canceled,
        },
        TransferState.VerifyingRemote: {
          TransferState.Running,
          TransferState.WaitingForNetwork,
          TransferState.BackingOff,
          TransferState.RestartRequired,
          TransferState.Completed,
          TransferState.Failed,
          TransferState.Canceled,
        },
        TransferState.RestartRequired: {
          TransferState.Pending,
          TransferState.VerifyingRemote,
          TransferState.Failed,
          TransferState.Canceled,
        },
        TransferState.Failed: {
          TransferState.Pending,
          TransferState.RestartRequired,
          TransferState.Canceled,
        },
        TransferState.Completed: {},
        TransferState.Canceled: {},
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
          expect(TransferState.Completed.canTransition(to), isFalse);
          expect(TransferState.Canceled.canTransition(to), isFalse);
        }
      });

      test('Failed can be retried via Pending or RestartRequired', () {
        expect(TransferState.Failed.canTransition(TransferState.Pending),
            isTrue);
        expect(
            TransferState.Failed.canTransition(TransferState.RestartRequired),
            isTrue);
        expect(TransferState.Failed.canTransition(TransferState.Running),
            isFalse);
      });
    });

    group('displayName', () {
      test('returns Chinese names for all 9 values', () {
        final expectedNames = {
          TransferState.Pending: '等待中',
          TransferState.Running: '传输中',
          TransferState.WaitingForNetwork: '等待网络',
          TransferState.BackingOff: '退避重试',
          TransferState.VerifyingRemote: '远端校验',
          TransferState.RestartRequired: '需重新开始',
          TransferState.Completed: '已完成',
          TransferState.Failed: '失败',
          TransferState.Canceled: '已取消',
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
        expect(TransferState.Completed.isTerminal, isTrue);
        expect(TransferState.Failed.isTerminal, isTrue);
        expect(TransferState.Canceled.isTerminal, isTrue);
      });

      test('returns false for non-terminal states', () {
        expect(TransferState.Pending.isTerminal, isFalse);
        expect(TransferState.Running.isTerminal, isFalse);
        expect(TransferState.WaitingForNetwork.isTerminal, isFalse);
        expect(TransferState.BackingOff.isTerminal, isFalse);
        expect(TransferState.VerifyingRemote.isTerminal, isFalse);
        expect(TransferState.RestartRequired.isTerminal, isFalse);
      });
    });

    group('isActive', () {
      test('returns true for Running and VerifyingRemote', () {
        expect(TransferState.Running.isActive, isTrue);
        expect(TransferState.VerifyingRemote.isActive, isTrue);
      });

      test('returns false for non-active states', () {
        expect(TransferState.Pending.isActive, isFalse);
        expect(TransferState.WaitingForNetwork.isActive, isFalse);
        expect(TransferState.BackingOff.isActive, isFalse);
        expect(TransferState.RestartRequired.isActive, isFalse);
        expect(TransferState.Completed.isActive, isFalse);
        expect(TransferState.Failed.isActive, isFalse);
        expect(TransferState.Canceled.isActive, isFalse);
      });
    });
  });

  group('TransferDirection', () {
    test('persistent codes match Rust transfer_direction (0-3)', () {
      expect(TransferDirection.Upload.code, 0);
      expect(TransferDirection.Download.code, 1);
      expect(TransferDirection.Delete.code, 2);
      expect(TransferDirection.DownloadUpdate.code, 3);
    });

    test('fromCode roundtrip and unknown', () {
      for (final d in TransferDirection.values) {
        expect(TransferDirection.fromCode(d.code), d);
      }
      expect(TransferDirection.fromCode(4), isNull);
    });

    test('isDownload covers Download and DownloadUpdate', () {
      expect(TransferDirection.Download.isDownload, isTrue);
      expect(TransferDirection.DownloadUpdate.isDownload, isTrue);
      expect(TransferDirection.Upload.isDownload, isFalse);
      expect(TransferDirection.Delete.isDownload, isFalse);
    });
  });

  group('TransferOperation', () {
    test('persistent codes match Rust TransferOperation (0-7)', () {
      final expected = {
        TransferOperation.Create: 0,
        TransferOperation.Update: 1,
        TransferOperation.Download: 2,
        TransferOperation.DownloadUpdate: 3,
        TransferOperation.Delete: 4,
        TransferOperation.Move: 5,
        TransferOperation.Rename: 6,
        TransferOperation.CreateFolder: 7,
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
        TransferErrorKind.Network: 0,
        TransferErrorKind.Timeout: 1,
        TransferErrorKind.Auth: 2,
        TransferErrorKind.RateLimit: 3,
        TransferErrorKind.Server: 4,
        TransferErrorKind.Quota: 5,
        TransferErrorKind.Permission: 6,
        TransferErrorKind.Validation: 7,
        TransferErrorKind.SessionExpired: 8,
        TransferErrorKind.RemoteAmbiguous: 9,
        TransferErrorKind.LocalChanged: 10,
        TransferErrorKind.Unknown: 11,
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
        SyncItemStatus.Synced: 0,
        SyncItemStatus.CloudOnly: 1,
        SyncItemStatus.LocalOnly: 2,
        SyncItemStatus.Syncing: 3,
        SyncItemStatus.Failed: 4,
        SyncItemStatus.Conflict: 5,
        SyncItemStatus.Deleted: 7,
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
        SyncPhase.IndexingStartup: 'indexing-startup',
        SyncPhase.IndexingManual: 'indexing-manual',
        SyncPhase.IndexingAutoFull: 'indexing-auto-full',
        SyncPhase.QueryingChanges: 'querying-changes',
        SyncPhase.SyncingAutoIncremental: 'syncing-auto-incremental',
        SyncPhase.SyncingLocal: 'syncing-local',
        SyncPhase.SyncingManual: 'syncing-manual',
        SyncPhase.SyncingRetry: 'syncing-retry',
        SyncPhase.SyncingStartup: 'syncing-startup',
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
      expect(SyncPhase.IndexingStartup.isIndexing, isTrue);
      expect(SyncPhase.IndexingManual.isIndexing, isTrue);
      expect(SyncPhase.IndexingAutoFull.isIndexing, isTrue);
      expect(SyncPhase.QueryingChanges.isIndexing, isFalse);
      expect(SyncPhase.SyncingLocal.isIndexing, isFalse);
    });
  });

  group('SyncStatus', () {
    test('has exactly 5 values', () {
      expect(SyncStatus.values.length, 5);
      expect(SyncStatus.values, contains(SyncStatus.Idle));
      expect(SyncStatus.values, contains(SyncStatus.Scanning));
      expect(SyncStatus.values, contains(SyncStatus.Syncing));
      expect(SyncStatus.values, contains(SyncStatus.Offline));
      expect(SyncStatus.values, contains(SyncStatus.Error));
    });
  });

  group('AuthStatus', () {
    test('has exactly 5 values', () {
      expect(AuthStatus.values.length, 5);
      expect(AuthStatus.values, contains(AuthStatus.Init));
      expect(AuthStatus.values, contains(AuthStatus.Authorizing));
      expect(AuthStatus.values, contains(AuthStatus.Authorized));
      expect(AuthStatus.values, contains(AuthStatus.Unauthorized));
      expect(AuthStatus.values, contains(AuthStatus.Error));
    });
  });

  group('AppPage', () {
    test('has exactly 5 values', () {
      expect(AppPage.values.length, 5);
      expect(AppPage.values, contains(AppPage.Login));
      expect(AppPage.values, contains(AppPage.Files));
      expect(AppPage.values, contains(AppPage.Settings));
      expect(AppPage.values, contains(AppPage.Logs));
      expect(AppPage.values, contains(AppPage.Update));
    });
  });
}

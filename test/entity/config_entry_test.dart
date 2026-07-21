import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/entity/config_entry.dart';

void main() {
  group('AppConfig', () {
    group('默认值（对齐 Rust Default）', () {
      test('构造安全默认配置', () {
        const config = AppConfig();

        expect(config.oauthRedirectUri,
            'http://127.0.0.1:9999/oauth/callback');
        expect(config.oauthCallbackPort, 9999);
        expect(config.mountDir, '');
        expect(config.mountConfigured, isFalse);
        expect(config.concurrency, 6);
        expect(config.pollIntervalSec, 60);
        expect(config.debounceSec, 3);
        expect(config.skipPatterns,
            ['.DS_Store', '.tmp', '~\$*', '.Trash']);
        expect(config.sortField, SortField.Name);
        expect(config.sortOrder, SortOrder.Ascending);
      });
    });

    group('fromJson / toJson', () {
      test('全字段往返保持一致（snake_case 键）', () {
        const config = AppConfig(
          oauthRedirectUri: 'http://127.0.0.1:8888/oauth/callback',
          oauthCallbackPort: 8888,
          mountDir: '~/my-drive',
          mountConfigured: true,
          concurrency: 10,
          pollIntervalSec: 900,
          debounceSec: 5,
          skipPatterns: ['.git', 'node_modules'],
          sortField: SortField.ModifiedTime,
          sortOrder: SortOrder.Descending,
        );

        final json = config.toJson();
        expect(json['oauth_redirect_uri'],
            'http://127.0.0.1:8888/oauth/callback');
        expect(json['oauth_callback_port'], 8888);
        expect(json['mount_dir'], '~/my-drive');
        expect(json['mount_configured'], true);
        expect(json['poll_interval_sec'], 900);
        expect(json['skip_patterns'], ['.git', 'node_modules']);
        expect(json['sort_field'], 'modifiedTime');
        expect(json['sort_order'], 'descending');

        final restored = AppConfig.fromJson(json);
        expect(restored.oauthRedirectUri, config.oauthRedirectUri);
        expect(restored.oauthCallbackPort, config.oauthCallbackPort);
        expect(restored.mountDir, config.mountDir);
        expect(restored.mountConfigured, config.mountConfigured);
        expect(restored.concurrency, config.concurrency);
        expect(restored.pollIntervalSec, config.pollIntervalSec);
        expect(restored.debounceSec, config.debounceSec);
        expect(restored.skipPatterns, config.skipPatterns);
        expect(restored.sortField, config.sortField);
        expect(restored.sortOrder, config.sortOrder);
      });

      test('缺失字段取默认值（对齐 serde(default)）', () {
        final config = AppConfig.fromJson(<String, dynamic>{});

        expect(config.oauthRedirectUri, AppConfig.defaultRedirectUri);
        expect(config.oauthCallbackPort, 9999);
        expect(config.concurrency, 6);
        expect(config.skipPatterns, AppConfig.defaultSkipPatterns);
      });

      test('int 字段容忍 String 数字', () {
        final config = AppConfig.fromJson({
          'oauth_callback_port': '7777',
          'concurrency': '12',
          'poll_interval_sec': '120',
          'debounce_sec': '8',
        });

        expect(config.oauthCallbackPort, 7777);
        expect(config.concurrency, 12);
        expect(config.pollIntervalSec, 120);
        expect(config.debounceSec, 8);
      });

      test('未知 sort_field / sort_order 回退默认', () {
        final config = AppConfig.fromJson({
          'sort_field': 'unknown',
          'sort_order': 'sideways',
        });

        expect(config.sortField, SortField.Name);
        expect(config.sortOrder, SortOrder.Ascending);
      });
    });

    group('expandedMountDir', () {
      test('~ 前缀展开为 HOME', () {
        final home = Platform.environment['HOME'];
        expect(home, isNotNull, reason: '测试环境应有 HOME');

        const config = AppConfig(mountDir: '~/my-drive');
        expect(config.expandedMountDir, '$home/my-drive');
      });

      test('绝对路径原样返回', () {
        const config = AppConfig(mountDir: '/data/drive');

        expect(config.expandedMountDir, '/data/drive');
      });
    });

    group('validate', () {
      test('默认配置合法', () {
        const config = AppConfig();

        expect(config.validate(), isNull);
      });

      test('回调端口越界 → ConfigError', () {
        expect(const AppConfig(oauthCallbackPort: 0).validate(), isNotNull);
        expect(
            const AppConfig(oauthCallbackPort: 65536).validate(), isNotNull);
        expect(const AppConfig(oauthCallbackPort: 1).validate(), isNull);
      });

      test('并发数必须在 1-20 之间', () {
        expect(const AppConfig(concurrency: 0).validate(), isNotNull);
        expect(const AppConfig(concurrency: 21).validate(), isNotNull);
        expect(const AppConfig(concurrency: 1).validate(), isNull);
        expect(const AppConfig(concurrency: 20).validate(), isNull);
      });

      test('pollIntervalSec：0=关闭合法，1-59 非法，≥60 合法', () {
        expect(const AppConfig(pollIntervalSec: 0).validate(), isNull);
        expect(const AppConfig(pollIntervalSec: 59).validate(), isNotNull);
        expect(const AppConfig(pollIntervalSec: 60).validate(), isNull);
      });

      test('debounceSec 必须 ≥ 1', () {
        expect(const AppConfig(debounceSec: 0).validate(), isNotNull);
        expect(const AppConfig(debounceSec: 1).validate(), isNull);
      });

      test('mountConfigured 时目录不能为空', () {
        const config = AppConfig(mountConfigured: true, mountDir: '  ');

        expect(config.validate(), isNotNull);
      });

      test('mountConfigured 时必须是绝对路径', () {
        const config =
            AppConfig(mountConfigured: true, mountDir: 'relative/dir');

        expect(config.validate(), isNotNull);
      });

      test('mountConfigured 时禁止系统根目录', () {
        const config = AppConfig(mountConfigured: true, mountDir: '/');

        expect(config.validate(), isNotNull);
      });

      test('mountConfigured 时禁止 Home 目录', () {
        final home = Platform.environment['HOME']!;
        final config = AppConfig(mountConfigured: true, mountDir: home);

        expect(config.validate(), isNotNull);
      });

      test('mountConfigured 时禁止 Application Support 目录', () {
        final home = Platform.environment['HOME']!;
        final config = AppConfig(
          mountConfigured: true,
          mountDir: '$home/Library/Application Support',
        );

        expect(config.validate(), isNotNull);
      });

      test('mountConfigured + 合法目录 → 通过', () {
        const config =
            AppConfig(mountConfigured: true, mountDir: '/tmp/petal-sync');

        expect(config.validate(), isNull);
      });

      test('~ 展开的 Home 子目录合法', () {
        const config =
            AppConfig(mountConfigured: true, mountDir: '~/my-drive');

        expect(config.validate(), isNull);
      });
    });

    group('copyWith', () {
      test('链式替换指定字段', () {
        const config = AppConfig();

        final copy = config.copyWith(
          concurrency: 10,
          sortOrder: SortOrder.Descending,
        );

        expect(copy.concurrency, 10);
        expect(copy.sortOrder, SortOrder.Descending);
        expect(copy.pollIntervalSec, 60);
        expect(copy.skipPatterns, config.skipPatterns);
      });
    });
  });

  group('SortField / SortOrder', () {
    test('wireName 对齐 Rust serde camelCase', () {
      expect(SortField.Name.wireName, 'name');
      expect(SortField.Size.wireName, 'size');
      expect(SortField.ModifiedTime.wireName, 'modifiedTime');
      expect(SortOrder.Ascending.wireName, 'ascending');
      expect(SortOrder.Descending.wireName, 'descending');
    });

    test('fromWireName 往返与未知值', () {
      for (final f in SortField.values) {
        expect(SortField.fromWireName(f.wireName), f);
      }
      for (final o in SortOrder.values) {
        expect(SortOrder.fromWireName(o.wireName), o);
      }
      expect(SortField.fromWireName('unknown'), isNull);
      expect(SortOrder.fromWireName(null), isNull);
    });
  });
}

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/service/platform/launch_at_login.dart';

void main() {
  group('LaunchAtLoginService.resolvePaths（对齐 Rust resolve_paths）', () {
    test('bundle 模式：可执行文件上溯三级为 .app', () {
      final (bundle, program) = LaunchAtLoginService.resolvePaths(
        '/Applications/PetalLink.app/Contents/MacOS/PetalLink',
      );
      expect(bundle, '/Applications/PetalLink.app');
      expect(program, 'Contents/MacOS/PetalLink');
    });

    test('dev 裸二进制模式：无 .app 上溯', () {
      final (bundle, program) = LaunchAtLoginService.resolvePaths(
        '/Users/x/dev/petal-link/build/petal_link',
      );
      expect(bundle, isNull);
      expect(program, '/Users/x/dev/petal-link/build/petal_link');
    });
  });

  group('LaunchAtLoginService.buildPlist（对齐 Rust plist 模板）', () {
    test('bundle 模式：Label/程序参数/--hidden/RunAtLoad/KeepAlive', () {
      final plist = LaunchAtLoginService.buildPlist(
        label: 'io.github.yuanbaobaoo.PetalLink',
        bundlePath: '/Applications/PetalLink.app',
        programPath: 'Contents/MacOS/PetalLink',
      );

      expect(plist, contains('<key>Label</key>'));
      expect(
        plist,
        contains('<string>io.github.yuanbaobaoo.PetalLink</string>'),
      );
      // 程序绝对路径 = bundle + 相对路径
      expect(
        plist,
        contains(
            '<string>/Applications/PetalLink.app/Contents/MacOS/PetalLink</string>'),
      );
      // --hidden 固定第二参数（对齐 Rust）
      expect(plist, contains('<string>--hidden</string>'));
      expect(plist, contains('<key>RunAtLoad</key>'));
      expect(plist, contains('<true/>'));
      expect(plist, contains('<key>KeepAlive</key>'));
      expect(plist, contains('<false/>'));
      // bundle 模式无 PATH 注入
      expect(plist, isNot(contains('EnvironmentVariables')));
    });

    test('dev 模式：注入 PATH 环境变量（对齐 Rust dev 分支）', () {
      final plist = LaunchAtLoginService.buildPlist(
        label: 'io.github.yuanbaobaoo.PetalLink-dev',
        bundlePath: null,
        programPath: '/Users/x/dev/petal_link',
      );

      expect(plist, contains('<string>/Users/x/dev/petal_link</string>'));
      expect(plist, contains('<key>EnvironmentVariables</key>'));
      expect(plist, contains('/usr/bin:/bin:/usr/sbin:/sbin:/usr/local/bin'));
    });
  });

  group('LaunchAtLoginService 启用/禁用（launchctl 交互）', () {
    late Directory tempHome;
    late List<(String, List<String>)> calls;
    late String plistPath;

    LaunchAtLoginService service({
      ProcResult Function(String exe, List<String> args)? onRun,
    }) {
      return LaunchAtLoginService(
        homeDir: tempHome.path,
        bundleId: 'io.github.yuanbaobaoo.PetalLink-dev',
        executablePath: '/Applications/PetalLink Dev.app/Contents/MacOS/PetalLink Dev',
        runner: (exe, args) async {
          calls.add((exe, args));
          if (onRun != null) return onRun(exe, args);
          if (exe == 'id') {
            return const ProcResult(exitCode: 0, stdout: '501\n');
          }
          return const ProcResult(exitCode: 0);
        },
      );
    }

    setUp(() {
      tempHome = Directory.systemTemp.createTempSync('launch_at_login_test');
      calls = [];
      plistPath =
          '${tempHome.path}/Library/LaunchAgents/io.github.yuanbaobaoo.PetalLink-dev.plist';
    });

    tearDown(() {
      if (tempHome.existsSync()) tempHome.deleteSync(recursive: true);
    });

    test('isEnabled 仅判断 plist 存在（对齐 Rust）', () async {
      final s = service();
      expect(s.isEnabled(), isFalse);
      await File(plistPath).create(recursive: true);
      expect(s.isEnabled(), isTrue);
    });

    test('启用：写 plist → bootstrap gui/uid → 移除 Login Items', () async {
      final s = service();
      final ok = await s.setEnabled(true);

      expect(ok, isTrue);
      expect(File(plistPath).existsSync(), isTrue);
      final content = await File(plistPath).readAsString();
      expect(content, contains('io.github.yuanbaobaoo.PetalLink-dev'));
      expect(content, contains('--hidden'));

      // launchctl bootstrap gui/501 <plist>
      expect(
        calls.any((c) =>
            c.$1 == 'launchctl' &&
            c.$2.join(' ') == 'bootstrap gui/501 $plistPath'),
        isTrue,
      );
      // osascript 移除 Login Items 同名项
      expect(calls.any((c) => c.$1 == 'osascript'), isTrue);
    });

    test('启用：已有旧 plist 先 bootout（对齐 Rust）', () async {
      await File(plistPath).create(recursive: true);
      final s = service();
      await s.setEnabled(true);

      final bootout = calls.firstWhere(
          (c) => c.$1 == 'launchctl' && c.$2.first == 'bootout');
      expect(
        bootout.$2.join(' '),
        'bootout gui/501/io.github.yuanbaobaoo.PetalLink-dev',
      );
    });

    test('启用：bootstrap 报 already bootstrapped 视为成功', () async {
      final s = service(onRun: (exe, args) {
        if (exe == 'launchctl' && args.first == 'bootstrap') {
          return const ProcResult(
              exitCode: 1, stderr: 'service already bootstrapped');
        }
        if (exe == 'id') return const ProcResult(exitCode: 0, stdout: '501');
        return const ProcResult(exitCode: 0);
      });
      expect(await s.setEnabled(true), isTrue);
    });

    test('禁用：bootout → 删 plist → 移除 Login Items', () async {
      await File(plistPath).create(recursive: true);
      final s = service();
      final ok = await s.setEnabled(false);

      expect(ok, isTrue);
      expect(File(plistPath).existsSync(), isFalse);
      expect(
        calls.any((c) =>
            c.$1 == 'launchctl' &&
            c.$2.join(' ') ==
                'bootout gui/501/io.github.yuanbaobaoo.PetalLink-dev'),
        isTrue,
      );
      expect(calls.any((c) => c.$1 == 'osascript'), isTrue);
    });

    test('id -u 失败兜底 501（对齐 Rust current_uid）', () async {
      final s = service(onRun: (exe, args) {
        if (exe == 'id') return const ProcResult(exitCode: 1);
        return const ProcResult(exitCode: 0);
      });
      await s.setEnabled(true);
      expect(
        calls.any((c) =>
            c.$1 == 'launchctl' && c.$2.join(' ').contains('gui/501')),
        isTrue,
      );
    });
  });
}

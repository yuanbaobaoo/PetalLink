import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/net/net_guard.dart';

void main() {
  setUp(() {
    NetGuard.instance.debugReset();
  });

  tearDown(() {
    NetGuard.instance.shutdown();
    NetGuard.instance.debugReset();
  });

  group('NetGuard 迟滞状态机', () {
    test('初始状态为在线', () {
      expect(NetGuard.instance.isOnline, isTrue);
    });

    test('一次探测失败立即离线', () {
      NetGuard.instance.observeProbeResult(false);
      expect(NetGuard.instance.isOnline, isFalse);
    });

    test('离线后连续两次探测成功才恢复在线', () {
      final guard = NetGuard.instance;
      guard.observeProbeResult(false);
      expect(guard.isOnline, isFalse);

      // 第一次成功：仍离线（迟滞）
      guard.observeProbeResult(true);
      expect(guard.isOnline, isFalse);

      // 第二次连续成功：恢复在线
      guard.observeProbeResult(true);
      expect(guard.isOnline, isTrue);
    });

    test('恢复途中失败重置连续成功计数', () {
      final guard = NetGuard.instance;
      guard.observeProbeResult(false);

      guard.observeProbeResult(true);
      // 中间夹一次失败 → 计数清零
      guard.observeProbeResult(false);
      guard.observeProbeResult(true);
      expect(guard.isOnline, isFalse);

      guard.observeProbeResult(true);
      expect(guard.isOnline, isTrue);
    });

    test('重复失败/成功不产生重复转换', () async {
      final transitions = <NetworkTransition>[];
      final sub = NetGuard.instance.transitions.listen(transitions.add);

      final guard = NetGuard.instance;
      guard.observeProbeResult(false);
      guard.observeProbeResult(false);
      guard.observeProbeResult(true);
      guard.observeProbeResult(true);
      guard.observeProbeResult(true);

      await Future<void>.delayed(Duration.zero);
      expect(transitions, [
        NetworkTransition.offline,
        NetworkTransition.online,
      ]);
      await sub.cancel();
    });
  });

  group('NetGuard.reportRequestNetworkFailure', () {
    test('在线时报告失败 → 离线并返回 true', () {
      final guard = NetGuard.instance;
      expect(guard.reportRequestNetworkFailure(), isTrue);
      expect(guard.isOnline, isFalse);
    });

    test('重复报告只发布一次边沿', () {
      final guard = NetGuard.instance;
      expect(guard.reportRequestNetworkFailure(), isTrue);
      expect(guard.reportRequestNetworkFailure(), isFalse);
    });

    test('请求层离线后仍需两次探测成功恢复', () {
      final guard = NetGuard.instance;
      guard.reportRequestNetworkFailure();

      guard.observeProbeResult(true);
      expect(guard.isOnline, isFalse);
      guard.observeProbeResult(true);
      expect(guard.isOnline, isTrue);
    });
  });

  group('NetGuard 探测任务生命周期', () {
    test('start 幂等且驱动探测循环', () async {
      var probeCount = 0;
      NetGuard.instance.debugConfigure(
        probe: () async {
          probeCount++;
          return true;
        },
        interval: const Duration(milliseconds: 20),
      );

      NetGuard.instance.start();
      NetGuard.instance.start(); // 幂等：不启动第二个任务
      expect(NetGuard.instance.isRunning, isTrue);

      await Future<void>.delayed(const Duration(milliseconds: 75));
      NetGuard.instance.shutdown();

      expect(NetGuard.instance.isRunning, isFalse);
      // 约 3-4 次探测（启动即探测 + 每 20ms）
      expect(probeCount, greaterThanOrEqualTo(2));
      expect(probeCount, lessThan(10));
    });

    test('探测失败驱动离线再恢复', () async {
      var online = true;
      NetGuard.instance.debugConfigure(
        probe: () async => online,
        interval: const Duration(milliseconds: 20),
      );

      NetGuard.instance.start();
      await Future<void>.delayed(const Duration(milliseconds: 35));

      // 断网：一次失败即离线
      online = false;
      await Future<void>.delayed(const Duration(milliseconds: 35));
      expect(NetGuard.instance.isOnline, isFalse);

      // 恢复：连续两次成功
      online = true;
      await Future<void>.delayed(const Duration(milliseconds: 60));
      expect(NetGuard.instance.isOnline, isTrue);

      NetGuard.instance.shutdown();
    });

    test('shutdown 后循环退出', () async {
      var probeCount = 0;
      NetGuard.instance.debugConfigure(
        probe: () async {
          probeCount++;
          return true;
        },
        interval: const Duration(milliseconds: 20),
      );

      NetGuard.instance.start();
      await Future<void>.delayed(const Duration(milliseconds: 35));
      NetGuard.instance.shutdown();
      final countAtShutdown = probeCount;

      await Future<void>.delayed(const Duration(milliseconds: 50));
      expect(probeCount, countAtShutdown);
    });
  });

  group('NetGuard.waitUntilOnline', () {
    test('在线时立即返回', () async {
      await NetGuard.instance.waitUntilOnline().timeout(
            const Duration(seconds: 1),
          );
    });

    test('离线时等待恢复', () async {
      final guard = NetGuard.instance;
      guard.observeProbeResult(false);

      var returned = false;
      final future = guard.waitUntilOnline().then((_) => returned = true);

      await Future<void>.delayed(const Duration(milliseconds: 50));
      expect(returned, isFalse);

      guard.observeProbeResult(true);
      guard.observeProbeResult(true);
      await future.timeout(const Duration(seconds: 5));
      expect(returned, isTrue);
    });

    test('isShutdown 为 true 时立即返回', () async {
      final guard = NetGuard.instance;
      guard.observeProbeResult(false);

      await guard
          .waitUntilOnline(isShutdown: () => true)
          .timeout(const Duration(seconds: 5));
    });
  });
}

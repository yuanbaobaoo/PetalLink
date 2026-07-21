import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:get/get.dart';
import 'package:petal_link/app/auth/auth_controller.dart';
import 'package:petal_link/app/bindings/global_binding.dart';
import 'package:petal_link/app/sync/sync_controller.dart';
import 'package:petal_link/app/transfer/transfer_controller.dart';
import 'package:petal_link/app/update/update_controller.dart';
import 'package:petal_link/service/sync/sync_service.dart';
import 'package:petal_link/service/transfer/task_runner.dart';
import 'package:petal_link/service/transfer/transfer_service.dart';
import 'package:sqflite_common_ffi/sqflite_ffi.dart';

/// GlobalBinding 依赖注册顺序测试。
///
/// 回归背景：2026-07-21 启动实测发现 SyncController.onInit 调用
/// `Get.find<SyncService>()` 时 SyncService 尚未注册，启动必抛
/// `"SyncService" not found` 未捕获异常。本测试锁定"控制器注册时
/// 其依赖的服务必须已注册"的顺序不变量。
void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  sqfliteFfiInit();
  databaseFactory = databaseFactoryFfi;

  setUp(() {
    // package_info_plus 平台通道 mock（UpdateService 装配依赖）
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
      const MethodChannel('dev.fluttercommunity.plus/package_info'),
      (call) async => {
        'appName': 'PetalLink Dev',
        'packageName': 'io.github.yuanbaobaoo.PetalLink-dev',
        'version': '0.0.0-test',
        'buildNumber': '1',
      },
    );
  });

  tearDown(() {
    Get.reset();
  });

  test('dependencies 注册完成：全部控制器与服务可解析，无顺序异常', () async {
    await GlobalBinding().dependencies();

    // 控制器（注册即解析依赖，任何顺序缺陷会在 dependencies() 内抛出）
    expect(Get.isRegistered<AuthController>(), isTrue);
    expect(Get.isRegistered<SyncController>(), isTrue);
    expect(Get.isRegistered<TransferController>(), isTrue);
    expect(Get.isRegistered<UpdateController>(), isTrue);

    // 关键服务
    expect(Get.isRegistered<SyncService>(), isTrue);
    expect(Get.isRegistered<TransferService>(), isTrue);
    expect(Get.isRegistered<TaskRunner>(), isTrue);
  });
}

import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/platform/tray_service.dart';
import 'package:petal_link/types/enums.dart';

/// 记录型 fake 托盘后端
class FakeTrayBackend implements TrayBackend {
  String? iconPath;
  String? tooltip;
  bool? isTemplate;
  final List<List<TrayMenuItem>> menus = [];

  @override
  Future<void> init({
    required String iconPath,
    required String tooltip,
    required bool isTemplate,
  }) async {
    this.iconPath = iconPath;
    this.tooltip = tooltip;
    this.isTemplate = isTemplate;
  }

  @override
  Future<void> setMenu(List<TrayMenuItem> items) async {
    menus.add(items);
  }

  @override
  Future<void> setToolTip(String tooltip) async {
    this.tooltip = tooltip;
  }

  @override
  Future<void> destroy() async {}
}

void main() {
  TransferTask task({
    int id = 1,
    String name = 'file.txt',
    TransferDirection direction = TransferDirection.Upload,
    int transferred = 0,
    int totalSize = 100,
    TransferState state = TransferState.Running,
    int createdAt = 0,
  }) {
    return TransferTask(
      id: id,
      name: name,
      direction: direction,
      transferred: transferred,
      totalSize: totalSize,
      state: state,
      createdAt: createdAt,
    );
  }

  group('TrayService.buildMenu（对齐 Rust build_menu 结构）', () {
    test('无活动传输：版本/分隔/显示主窗口/分隔/退出', () {
      final menu = TrayService.buildMenu(const []);

      expect(menu[0].id, 'version');
      expect(menu[0].enabled, isFalse);
      expect(menu[0].label, 'PetalLink - 华为云盘 Mac 客户端开源版');
      expect(menu[1].separator, isTrue);
      expect(menu[2].id, 'show_window');
      expect(menu[2].label, '显示主窗口');
      expect(menu[2].enabled, isTrue);
      expect(menu[3].separator, isTrue);
      expect(menu[4].id, 'quit');
      expect(menu[4].label, '退出 PetalLink');
      expect(menu.length, 5);
    });

    test('有活动传输：每任务两行禁用项 + 底部分隔线无条件保留', () {
      final menu = TrayService.buildMenu([
        task(id: 7, name: 'a.txt', transferred: 50, totalSize: 100),
        task(
            id: 8,
            name: 'b.txt',
            direction: TransferDirection.Download,
            transferred: 10,
            totalSize: 40),
      ]);

      // 5（无传输结构）+ 1（传输段分隔线）+ 2*2（每任务两行）
      expect(menu.length, 10);
      expect(menu[3].separator, isTrue);
      expect(menu[4].id, 'transfer_name_7');
      expect(menu[4].label, 'a.txt');
      expect(menu[5].id, 'transfer_status_7');
      expect(menu[5].label, '正在上传…50%');
      expect(menu[6].id, 'transfer_name_8');
      expect(menu[7].label, '正在下载…25%');
      // 底部分隔线 + 退出
      expect(menu[8].separator, isTrue);
      expect(menu[9].id, 'quit');
      // 传输项全部禁用
      expect(menu.sublist(4, 8).every((i) => !i.enabled), isTrue);
    });

    test('文件名超 20 字符截断加 …（按字符计，对齐 Rust truncate_name）', () {
      final longName = '这是一个非常非常长的文件名称-abcdefghij.txt';
      final menu = TrayService.buildMenu([task(name: longName)]);
      final label = menu[4].label;
      expect(label.runes.length, 21); // 20 + …
      expect(label.endsWith('…'), isTrue);

      // 边界：恰好 20 字符不截断
      final exact = 'a' * 20;
      final menu2 = TrayService.buildMenu([task(name: exact)]);
      expect(menu2[4].label, exact);
    });

    test('状态行：方向标签映射与百分比边界', () {
      expect(TrayService.transferStatusLine(
          task(direction: TransferDirection.Upload, transferred: 1, totalSize: 3)),
          '正在上传…33%');
      expect(TrayService.transferStatusLine(
          task(direction: TransferDirection.Delete, transferred: 0, totalSize: 0)),
          '正在删除…0%');
      expect(TrayService.transferStatusLine(
          task(direction: TransferDirection.DownloadUpdate, transferred: 200, totalSize: 100)),
          '正在更新…100%');
    });
  });

  group('TrayService.transferSignature（对齐 Rust 签名判等）', () {
    test('任务数/id/state/transferred/totalSize 任一变化 → 签名变化', () {
      final base = [task(id: 1, transferred: 10)];
      final same = [task(id: 1, transferred: 10)];
      final progressed = [task(id: 1, transferred: 11)];
      final more = [task(id: 1, transferred: 10), task(id: 2)];

      expect(TrayService.transferSignature(base),
          TrayService.transferSignature(same));
      expect(TrayService.transferSignature(base),
          isNot(TrayService.transferSignature(progressed)));
      expect(TrayService.transferSignature(base),
          isNot(TrayService.transferSignature(more)));
      expect(TrayService.transferSignature(const []),
          isNot(TrayService.transferSignature(base)));
    });
  });

  group('TrayService.refreshMenu（对齐 Rust refresh_menu 三段逻辑）', () {
    late FakeTrayBackend backend;
    late int nowMs;
    late List<TransferTask> active;

    TrayService newService() => TrayService(
          backend: backend,
          activeTransfersProvider: () async => active,
          nowMs: () => nowMs,
        );

    setUp(() {
      backend = FakeTrayBackend();
      nowMs = 100000;
      active = [];
    });

    test('签名相同 → 跳过重建', () async {
      final service = newService();
      active = [task(id: 1, transferred: 10)];
      await service.refreshMenu();
      expect(backend.menus.length, 1);

      // 内容相同（签名相同）→ 不重建
      active = [task(id: 1, transferred: 10)];
      await service.refreshMenu();
      expect(backend.menus.length, 1);
    });

    test('有活动传输时 5s 节流；超过 5s 允许重建', () async {
      final service = newService();
      active = [task(id: 1, transferred: 10)];
      await service.refreshMenu();
      expect(backend.menus.length, 1);

      // 进度变化（签名不同）但距上次 <5s → 节流跳过
      nowMs += 1000;
      active = [task(id: 1, transferred: 20)];
      await service.refreshMenu();
      expect(backend.menus.length, 1);

      // 超过 5s → 重建
      nowMs += 5000;
      await service.refreshMenu();
      expect(backend.menus.length, 2);
    });

    test('无活动传输 → 不节流立即重建（清场）', () async {
      final service = newService();
      active = [task(id: 1, transferred: 10)];
      await service.refreshMenu();
      expect(backend.menus.length, 1);

      // 立刻清空（无传输不节流）
      nowMs += 100;
      active = [];
      await service.refreshMenu();
      expect(backend.menus.length, 2);
      expect(backend.menus.last.length, 5);
    });

    test('init：模板图标 + tooltip（对齐 Rust 托盘初始化）', () async {
      final service = newService();
      await service.init();

      expect(backend.iconPath, 'assets/menubar-icon.png');
      expect(backend.isTemplate, isTrue);
      expect(backend.tooltip, 'PetalLink — 后台同步中');
      // init 后首建菜单
      expect(backend.menus.length, 1);
    });
  });
}

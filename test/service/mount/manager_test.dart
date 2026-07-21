import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/storage/database_service.dart';
import 'package:petal_link/service/mount/manager.dart';
import 'package:petal_link/types/enums.dart';
import 'package:sqflite_common_ffi/sqflite_ffi.dart';

import 'proc_inode.dart';
import 'proc_xattr.dart';

void main() {
  late Directory tempDir;
  late ProcXattrService xattr;
  late MountManager mount;
  late String dbPath;

  setUpAll(() {
    sqfliteFfiInit();
    databaseFactory = databaseFactoryFfi;
  });

  setUp(() {
    tempDir = Directory.systemTemp.createTempSync('petal_link_mount_test');
    xattr = ProcXattrService();
    dbPath = '${tempDir.path}/petal_link.db';
    DatabaseService.debugDatabasePath = dbPath;
    mount = MountManager(
      tempDir.path,
      xattr: xattr,
      db: DatabaseService.instance,
      inodeBatchProvider: procInodeBatch,
    );
  });

  tearDown(() async {
    await DatabaseService.instance.close();
    DatabaseService.debugDatabasePath = null;
    if (tempDir.existsSync()) {
      tempDir.deleteSync(recursive: true);
    }
  });

  String abs(String rel) => '${tempDir.path}/$rel';

  /// 插入 sync_items 基线行。
  Future<void> insertBaseline({
    required String fileId,
    required String localPath,
    String name = 'f',
    bool isFolder = false,
    int size = 0,
    int? localSize,
    int? localMtime,
    int? cloudEditedTime,
    SyncItemStatus status = SyncItemStatus.synced,
  }) async {
    final db = await DatabaseService.instance.database;
    await db.insert('sync_items', {
      'file_id': fileId,
      'local_path': localPath,
      'name': name,
      'is_folder': isFolder ? 1 : 0,
      'size': size,
      'local_size': localSize,
      'local_mtime': localMtime,
      'cloud_edited_time': cloudEditedTime,
      'status': status.code,
    });
  }

  group('MountManager.createPlaceholderIfNeeded', () {
    test('创建 0 字节占位 + state xattr + Finder 灰标（inode 方案只写 state）',
        () async {
      await mount.createPlaceholderIfNeeded('docs/a.txt', 'fid1', 1234);

      final file = File(abs('docs/a.txt'));
      expect(file.existsSync(), isTrue);
      expect(file.lengthSync(), 0);
      // inode 方案（docs/design/10 §2.1）：占位只写 state xattr，
      // fileId/size 不再写入（身份由 local_inode_map 承担）
      expect(await xattr.get(abs('docs/a.txt'), xattrFileId), isNull);
      expect(await xattr.get(abs('docs/a.txt'), xattrState), statePlaceholder);
      expect(await xattr.get(abs('docs/a.txt'), 'com.hwcloud.size'), isNull);
      // FinderInfo byte[9]=0x02（灰标）
      final finderInfo = await xattr.getBytes(abs('docs/a.txt'), finderInfoXattr);
      expect(finderInfo, isNotNull);
      expect(finderInfo!.length, 32);
      expect(finderInfo[9], grayLabelByte);
    });

    test('同 fileId 占位已存在 → 幂等跳过', () async {
      await mount.createPlaceholderIfNeeded('a.txt', 'fid1', 100);
      // 二次创建不报错
      await mount.createPlaceholderIfNeeded('a.txt', 'fid1', 100);
      expect(await mount.isPlaceholderFile(abs('a.txt')), isTrue);
    });

    test('同 fileId 已下载文件 → 跳过', () async {
      File(abs('b.txt')).writeAsStringSync('content');
      await xattr.set(abs('b.txt'), xattrFileId, 'fid1');
      await xattr.set(abs('b.txt'), xattrState, stateDownloaded);
      await mount.createPlaceholderIfNeeded('b.txt', 'fid1', 100);
      // 内容未被清空
      expect(File(abs('b.txt')).readAsStringSync(), 'content');
    });

    test('已有用户文件（无 xattr）→ 拒绝覆盖', () async {
      File(abs('user.txt')).writeAsStringSync('mine');
      expect(
        () => mount.createPlaceholderIfNeeded('user.txt', 'fid1', 100),
        throwsA(isA<AppError>()),
      );
      expect(File(abs('user.txt')).readAsStringSync(), 'mine');
    });

    test('已有其他 fileId 占位 → 拒绝覆盖', () async {
      await mount.createPlaceholderIfNeeded('c.txt', 'fid-other', 100);
      expect(
        () => mount.createPlaceholderIfNeeded('c.txt', 'fid1', 100),
        throwsA(isA<AppError>()),
      );
    });

    test('目标是目录 → 报错', () async {
      Directory(abs('dir1')).createSync();
      expect(
        () => mount.createPlaceholderIfNeeded('dir1', 'fid1', 100),
        throwsA(isA<AppError>()),
      );
    });

    test('路径穿越被拒绝', () async {
      expect(
        () => mount.createPlaceholderIfNeeded('../evil.txt', 'fid1', 100),
        throwsA(isA<AppError>()),
      );
    });
  });

  group('MountManager.createPlaceholderStrict', () {
    test('目标已存在 → 失败（绝不覆盖）', () async {
      File(abs('exist.txt')).writeAsStringSync('data');
      expect(
        () => mount.createPlaceholderStrict('exist.txt', 'fid1', 4),
        throwsA(isA<AppError>()),
      );
      expect(File(abs('exist.txt')).readAsStringSync(), 'data');
    });

    test('目标不存在 → 创建成功', () async {
      await mount.createPlaceholderStrict('new/x.bin', 'fid2', 88);
      expect(File(abs('new/x.bin')).lengthSync(), 0);
      expect(await xattr.get(abs('new/x.bin'), xattrState), statePlaceholder);
      expect(await xattr.get(abs('new/x.bin'), xattrFileId), isNull);
    });
  });

  group('MountManager.markDownloaded', () {
    test('markDownloaded 更新状态并清除灰标', () async {
      await mount.createPlaceholderIfNeeded('d.txt', 'fid1', 10);
      await mount.markDownloaded(abs('d.txt'));
      expect(await xattr.get(abs('d.txt'), xattrState), stateDownloaded);
      // 灰标清除后整块全 0 → xattr 删除
      expect(await xattr.getBytes(abs('d.txt'), finderInfoXattr), isNull);
    });

  });

  group('MountManager.backupModifiedPlaceholderIfNeeded', () {
    test('文件不存在 → null', () async {
      expect(await mount.backupModifiedPlaceholderIfNeeded(abs('none')), isNull);
    });

    test('非占位文件 → null', () async {
      File(abs('plain.txt')).writeAsStringSync('abc');
      expect(await mount.backupModifiedPlaceholderIfNeeded(abs('plain.txt')),
          isNull);
    });

    test('0 字节占位（未修改）→ null', () async {
      await mount.createPlaceholderIfNeeded('p0.txt', 'fid1', 55);
      expect(await mount.backupModifiedPlaceholderIfNeeded(abs('p0.txt')),
          isNull);
    });

    test('被用户写入的占位 → 改名备份并清占位 xattr', () async {
      await mount.createPlaceholderIfNeeded('report.docx', 'fid1', 999);
      File(abs('report.docx')).writeAsStringSync('user edited');

      final backup =
          await mount.backupModifiedPlaceholderIfNeeded(abs('report.docx'));
      expect(backup, isNotNull);
      expect(backup, contains('report.local-'));
      expect(backup, endsWith('.docx'));
      expect(File(abs('report.docx')).existsSync(), isFalse);
      expect(File(backup!).readAsStringSync(), 'user edited');
      // 备份的占位 xattr 已清（rename 后 xattr 随文件走，由 clearPlaceholderXattr 移除）
      expect(await xattr.get(backup, xattrFileId), isNull);
      expect(await xattr.get(backup, xattrState), isNull);
      expect(await xattr.get(backup, 'com.hwcloud.size'), isNull);
    });
  });

  group('MountManager.scanLocal', () {
    test('inodeBatchProvider 注入时批量填充 inode（docs/design/10 阶段1）',
        () async {
      File(abs('a.txt')).writeAsStringSync('12345');
      File(abs('sub/b.txt')).createSync(recursive: true);
      File(abs('sub/b.txt')).writeAsStringSync('x');
      final m = MountManager(
        tempDir.path,
        xattr: xattr,
        inodeBatchProvider: (paths) async =>
            {for (var i = 0; i < paths.length; i++) paths[i]: 9000 + i},
      );

      final entries = await m.scanLocal(const []);

      expect(entries, isNotEmpty);
      for (final e in entries) {
        expect(e.inode, isNotNull);
        expect(e.inode, greaterThanOrEqualTo(9000));
      }
    });

    test('inodeBatchProvider 缺失时 inode 为 null（行为不变）', () async {
      File(abs('a.txt')).writeAsStringSync('1');
      final bare = MountManager(tempDir.path, xattr: xattr);
      final entries = await bare.scanLocal(const []);
      expect(entries.single.inode, isNull);
    });

    test('递归收集并跳过内部项 / 符号链接', () async {
      // 普通文件
      File(abs('a.txt')).writeAsStringSync('12345');
      // 子目录 + 占位
      await mount.createPlaceholderIfNeeded('sub/b.bin', 'fid1', 77);
      // 内部文件与临时文件
      File(abs('.hwcloud_cache')).writeAsStringSync('x');
      File(abs('c.tmp')).writeAsStringSync('x');
      File(abs('.DS_Store')).writeAsStringSync('x');
      // 符号链接（文件与目录各一）
      Link(abs('link.txt')).createSync(abs('a.txt'));
      Link(abs('linkdir')).createSync(abs('sub'));

      final entries =
          await mount.scanLocal(const ['.DS_Store', '.tmp', '~\$*', '.Trash']);
      final byRel = {for (final e in entries) e.relativePath: e};

      expect(byRel.containsKey('a.txt'), isTrue);
      expect(byRel['a.txt']!.size, 5);
      expect(byRel['a.txt']!.isFolder, isFalse);
      expect(byRel['a.txt']!.isPlaceholder, isFalse);

      expect(byRel.containsKey('sub'), isTrue);
      expect(byRel['sub']!.isFolder, isTrue);

      expect(byRel.containsKey('sub/b.bin'), isTrue);
      expect(byRel['sub/b.bin']!.isPlaceholder, isTrue);
      expect(byRel['sub/b.bin']!.size, 0);

      // 跳过项不出现
      expect(byRel.containsKey('.hwcloud_cache'), isFalse);
      expect(byRel.containsKey('c.tmp'), isFalse);
      expect(byRel.containsKey('.DS_Store'), isFalse);
      expect(byRel.containsKey('link.txt'), isFalse);
      expect(byRel.containsKey('linkdir'), isFalse);
      // 符号链接目录不递归
      expect(byRel.containsKey('linkdir/b.bin'), isFalse);
    });

    test('0 字节用户文件不是占位符', () async {
      File(abs('.gitkeep')).writeAsBytesSync(const []);
      final entries = await mount.scanLocal(const []);
      final gitkeep = entries.singleWhere((e) => e.relativePath == '.gitkeep');
      expect(gitkeep.size, 0);
      expect(gitkeep.isPlaceholder, isFalse);
    });

    test('挂载目录为空串 → 返回空列表', () async {
      final empty = MountManager('', xattr: xattr);
      expect(await empty.scanLocal(const []), isEmpty);
    });
  });

  group('MountManager.deleteLocal', () {
    test('删除占位符（0 字节）', () async {
      await mount.createPlaceholderIfNeeded('del.txt', 'fid1', 10);
      await mount.deleteLocal(abs('del.txt'));
      expect(File(abs('del.txt')).existsSync(), isFalse);
    });

    test('拒绝删除非占位 0 字节文件', () async {
      File(abs('.gitkeep')).writeAsBytesSync(const []);
      await mount.deleteLocal(abs('.gitkeep'));
      expect(File(abs('.gitkeep')).existsSync(), isTrue);
    });

    test('删除普通文件并清理旧版占位符', () async {
      File(abs('old.txt')).writeAsStringSync('data');
      File(abs('old.txt.hwcloud_placeholder')).writeAsStringSync('x');
      await mount.deleteLocal(abs('old.txt'));
      expect(File(abs('old.txt')).existsSync(), isFalse);
      expect(File(abs('old.txt.hwcloud_placeholder')).existsSync(), isFalse);
    });

    test('递归删除目录', () async {
      Directory(abs('d1/d2')).createSync(recursive: true);
      File(abs('d1/d2/f.txt')).writeAsStringSync('x');
      await mount.deleteLocal(abs('d1'));
      expect(Directory(abs('d1')).existsSync(), isFalse);
    });

    test('含符号链接的目录拒绝递归删除（安全红线）', () async {
      Directory(abs('safe')).createSync();
      File(abs('safe/f.txt')).writeAsStringSync('x');
      Link(abs('safe/evil')).createSync(tempDir.path);
      expect(
        () => mount.deleteLocal(abs('safe')),
        throwsA(isA<AppError>()),
      );
      expect(Directory(abs('safe')).existsSync(), isTrue);
    });

    test('挂载目录外的路径被拒绝', () async {
      expect(
        () => mount.deleteLocal('/etc/passwd'),
        throwsA(isA<AppError>()),
      );
    });

    test('不存在的路径静默返回', () async {
      await mount.deleteLocal(abs('missing'));
    });
  });

  group('MountManager.checkFileLocalStatus / batchFileLocalStatus', () {
    test('无记录 → not_synced', () async {
      expect(await mount.checkFileLocalStatus('nope'), 'not_synced');
    });

    test('文件夹记录 → folder', () async {
      await insertBaseline(
          fileId: 'dir1', localPath: 'docs', isFolder: true, name: 'docs');
      expect(await mount.checkFileLocalStatus('dir1'), 'folder');
    });

    test('占位符 → placeholder', () async {
      await mount.createPlaceholderIfNeeded('a.txt', 'fid1', 100);
      await insertBaseline(
          fileId: 'fid1',
          localPath: 'a.txt',
          name: 'a.txt',
          size: 100,
          localSize: 0,
          status: SyncItemStatus.cloudOnly);
      expect(await mount.checkFileLocalStatus('fid1'), 'placeholder');
    });

    test('已下载文件 → synced', () async {
      File(abs('b.txt')).writeAsStringSync('hello');
      await xattr.set(abs('b.txt'), xattrFileId, 'fid2');
      await xattr.set(abs('b.txt'), xattrState, stateDownloaded);
      await insertBaseline(
          fileId: 'fid2', localPath: 'b.txt', name: 'b.txt', size: 5);
      expect(await mount.checkFileLocalStatus('fid2'), 'synced');
    });

    test('记录存在但本地文件缺失 → not_synced', () async {
      await insertBaseline(
          fileId: 'fid3', localPath: 'gone.txt', name: 'gone.txt');
      expect(await mount.checkFileLocalStatus('fid3'), 'not_synced');
    });

    test('同 fileId 多条歧义基线 → 抛 AppError', () async {
      await insertBaseline(fileId: 'dup', localPath: 'x1.txt', name: 'x1');
      await insertBaseline(fileId: 'dup', localPath: 'x2.txt', name: 'x2');
      expect(
        () => mount.checkFileLocalStatus('dup'),
        throwsA(isA<AppError>()),
      );
    });

    test('批量状态判定', () async {
      await mount.createPlaceholderIfNeeded('a.txt', 'fid1', 100);
      await insertBaseline(
          fileId: 'fid1',
          localPath: 'a.txt',
          name: 'a.txt',
          status: SyncItemStatus.cloudOnly);
      File(abs('b.txt')).writeAsStringSync('hi');
      await insertBaseline(fileId: 'fid2', localPath: 'b.txt', name: 'b.txt');
      await insertBaseline(
          fileId: 'fid3', localPath: 'docs', name: 'docs', isFolder: true);

      final result =
          await mount.batchFileLocalStatus(['fid1', 'fid2', 'fid3', 'fid4']);
      expect(result, {
        'fid1': 'placeholder',
        'fid2': 'synced',
        'fid3': 'folder',
        'fid4': 'not_synced',
      });
    });

    test('未配置挂载目录 → 仅 DB 状态判定', () async {
      final noMount = MountManager('', xattr: xattr, db: DatabaseService.instance);
      await insertBaseline(fileId: 'fid1', localPath: 'a.txt', name: 'a.txt');
      await insertBaseline(
          fileId: 'fid2',
          localPath: 'b.txt',
          name: 'b.txt',
          status: SyncItemStatus.cloudOnly);
      final result = await noMount.batchFileLocalStatus(['fid1', 'fid2']);
      expect(result, {'fid1': 'synced', 'fid2': 'not_synced'});
    });
  });

  group('MountManager.recoverInterruptedFreeUp', () {
    /// 构造暂存文件（含恢复标记与 fileId xattr）。
    Future<String> createStaging(String name, String relPath, String fileId,
        [String content = 'staged content']) async {
      final staging = abs(name);
      File(staging).writeAsStringSync(content);
      await xattr.set(staging, xattrFreeUpRelativePath, relPath);
      await xattr.set(staging, xattrFileId, fileId);
      return staging;
    }

    /// 构造 DB 恢复记录的暂存文件（free_up_staging 表，docs/design/10 §4.8）。
    Future<String> createDbStaging(String name, String relPath, String fileId,
        [String content = 'staged content']) async {
      final staging = abs(name);
      File(staging).writeAsStringSync(content);
      final db = await DatabaseService.instance.database;
      await db.insert('free_up_staging', {
        'staging_name': name,
        'relative_path': relPath,
        'file_id': fileId,
        'created_at': 1,
      });
      return staging;
    }

    test('DB 恢复记录：未提交 → 恢复原文件并清理记录（无 xattr 标记）',
        () async {
      final staging = await createDbStaging(
          '.hwcloud_freeup-1-db', 'db.txt', 'fid-db', 'db original');
      await insertBaseline(
          fileId: 'fid-db', localPath: 'db.txt', name: 'db.txt');

      final db = await DatabaseService.instance.database;
      final recovered = await mount.recoverInterruptedFreeUp(db);

      expect(recovered, 1);
      expect(File(staging).existsSync(), isFalse);
      expect(File(abs('db.txt')).readAsStringSync(), 'db original');
      // DB 恢复记录已清理
      expect(await db.query('free_up_staging'), isEmpty);
    });

    test('DB 恢复记录：暂存文件已缺失（清理窗口）→ 仅清理记录', () async {
      final db = await DatabaseService.instance.database;
      await db.insert('free_up_staging', {
        'staging_name': '.hwcloud_freeup-1-gone',
        'relative_path': 'gone.txt',
        'file_id': 'fid-gone',
        'created_at': 1,
      });

      final recovered = await mount.recoverInterruptedFreeUp(db);

      expect(recovered, 1);
      expect(await db.query('free_up_staging'), isEmpty);
    });

    test('已提交（占位+CloudOnly 基线）→ 清理暂存', () async {
      final staging = await createStaging(
          '.hwcloud_freeup-1-aa', 'a.txt', 'fid1', 'original');
      // 目标占位 + 同 fileId
      await mount.createPlaceholderStrict('a.txt', 'fid1', 8);
      await insertBaseline(
          fileId: 'fid1',
          localPath: 'a.txt',
          name: 'a.txt',
          localSize: 0,
          status: SyncItemStatus.cloudOnly);

      final db = await DatabaseService.instance.database;
      final recovered = await mount.recoverInterruptedFreeUp(db);
      expect(recovered, 1);
      expect(File(staging).existsSync(), isFalse);
      // 占位符保留（释放已完成）
      expect(await mount.isPlaceholderFile(abs('a.txt')), isTrue);
    });

    test('未提交（占位+Synced 基线）→ 移除占位恢复原文件', () async {
      final staging = await createStaging(
          '.hwcloud_freeup-1-bb', 'b.txt', 'fid2', 'original content');
      await mount.createPlaceholderStrict('b.txt', 'fid2', 16);
      await insertBaseline(
          fileId: 'fid2', localPath: 'b.txt', name: 'b.txt');

      final db = await DatabaseService.instance.database;
      final recovered = await mount.recoverInterruptedFreeUp(db);
      expect(recovered, 1);
      expect(File(staging).existsSync(), isFalse);
      expect(File(abs('b.txt')).readAsStringSync(), 'original content');
      // 基线恢复 Synced + 本地大小
      final record = await MountManager.findByFileId(db, 'fid2');
      expect(record!.status, SyncItemStatus.synced);
      expect(record.localSize, 16);
      // 恢复标记已清
      expect(await xattr.get(abs('b.txt'), xattrFreeUpRelativePath), isNull);
    });

    test('目标缺失 → 直接恢复暂存', () async {
      final staging = await createStaging(
          '.hwcloud_freeup-1-cc', 'c.txt', 'fid3', 'lost content');
      await insertBaseline(
          fileId: 'fid3', localPath: 'c.txt', name: 'c.txt');

      final db = await DatabaseService.instance.database;
      final recovered = await mount.recoverInterruptedFreeUp(db);
      expect(recovered, 1);
      expect(File(staging).existsSync(), isFalse);
      expect(File(abs('c.txt')).readAsStringSync(), 'lost content');
    });

    test('原路径已有用户内容 → 显式保留为恢复副本', () async {
      await createStaging(
          '.hwcloud_freeup-1-dd', 'd.txt', 'fid4', 'staged old');
      // 目标已是用户文件（无占位 xattr）
      File(abs('d.txt')).writeAsStringSync('user new');
      await insertBaseline(
          fileId: 'fid4', localPath: 'd.txt', name: 'd.txt');

      final db = await DatabaseService.instance.database;
      final recovered = await mount.recoverInterruptedFreeUp(db);
      expect(recovered, 1);
      // 用户内容不被覆盖
      expect(File(abs('d.txt')).readAsStringSync(), 'user new');
      // 暂存被改名为可见副本
      final copies = tempDir
          .listSync()
          .where((e) => e.path.contains('释放空间恢复-'))
          .toList();
      expect(copies, hasLength(1));
      expect(File(copies.single.path).readAsStringSync(), 'staged old');
      // 副本的恢复标记与身份 xattr 已清
      expect(await xattr.get(copies.single.path, xattrFreeUpRelativePath),
          isNull);
      expect(await xattr.get(copies.single.path, xattrFileId), isNull);
    });

    test('暂存缺恢复标记 → 显式恢复副本', () async {
      final staging = abs('.hwcloud_freeup-1-ee');
      File(staging).writeAsStringSync('orphan');

      final db = await DatabaseService.instance.database;
      final recovered = await mount.recoverInterruptedFreeUp(db);
      expect(recovered, 1);
      expect(File(staging).existsSync(), isFalse);
      final copies = tempDir
          .listSync()
          .where((e) => e.path.contains('释放空间恢复-'))
          .toList();
      expect(copies, hasLength(1));
    });

    test('无暂存项 → 0', () async {
      final db = await DatabaseService.instance.database;
      expect(await mount.recoverInterruptedFreeUp(db), 0);
    });
  });

  group('MountManager.ensureMountDir / ensureFolder', () {
    test('ensureMountDir 递归创建', () async {
      final dir = '${tempDir.path}/m1/m2';
      final m = MountManager(dir, xattr: xattr);
      await m.ensureMountDir();
      expect(Directory(dir).existsSync(), isTrue);
    });

    test('ensureFolder 返回完整路径并创建', () async {
      final full = await mount.ensureFolder('x/y');
      expect(full, abs('x/y'));
      expect(Directory(full).existsSync(), isTrue);
    });

    test('ensureFolder 拒绝路径穿越', () async {
      expect(
        () => mount.ensureFolder('../out'),
        throwsA(isA<AppError>()),
      );
    });
  });

  group('MountManager.setFinderLabel', () {
    test('灰标写入保留其他 FinderInfo 字段', () async {
      File(abs('f.txt')).writeAsStringSync('x');
      // 预置带其他字段的 FinderInfo（byte[0]=0x11）
      final preset = Uint8List(32)..[0] = 0x11;
      await xattr.setBytes(abs('f.txt'), finderInfoXattr, preset);

      await mount.setFinderLabel(abs('f.txt'), true);
      var info = (await xattr.getBytes(abs('f.txt'), finderInfoXattr))!;
      expect(info[0], 0x11);
      expect(info[9], grayLabelByte);

      // 清除灰标但保留其他字段 → 不删 xattr
      await mount.setFinderLabel(abs('f.txt'), false);
      info = (await xattr.getBytes(abs('f.txt'), finderInfoXattr))!;
      expect(info[0], 0x11);
      expect(info[9], 0);
    });

    test('失败不阻断（路径不存在）', () async {
      await mount.setFinderLabel(abs('missing'), true);
    });
  });
}

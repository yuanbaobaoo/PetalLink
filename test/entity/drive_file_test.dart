import 'package:flutter_test/flutter_test.dart';
import 'package:petal_link/entity/drive_file.dart';

void main() {
  group('FileCategory.fromMimeType', () {
    test('华为/Google 四种文件夹 mimeType → Folder', () {
      for (final mime in [
        'application/vnd.huawei-apps.folder',
        'application/vnd.huawei-app.folder',
        'application/vnd.google-apps.folder',
        'application/x-folder',
      ]) {
        expect(FileCategory.fromMimeType(mime), FileCategory.folder,
            reason: '$mime 应识别为 Folder');
      }
    });

    test('大小写不敏感', () {
      expect(FileCategory.fromMimeType('APPLICATION/VND.HUAWEI-APPS.FOLDER'),
          FileCategory.folder);
      expect(FileCategory.fromMimeType('IMAGE/PNG'), FileCategory.image);
    });

    test('image/ video/ audio/ 前缀', () {
      expect(FileCategory.fromMimeType('image/png'), FileCategory.image);
      expect(FileCategory.fromMimeType('video/mp4'), FileCategory.video);
      expect(FileCategory.fromMimeType('audio/mpeg'), FileCategory.audio);
    });

    test('文档类关键字', () {
      for (final mime in [
        'text/plain',
        'application/pdf',
        'application/msword',
        'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
        'application/vnd.ms-excel',
        'application/vnd.ms-powerpoint',
      ]) {
        expect(FileCategory.fromMimeType(mime), FileCategory.document,
            reason: '$mime 应识别为 Document');
      }
    });

    test('压缩包关键字', () {
      for (final mime in [
        'application/zip',
        'application/x-rar-compressed',
        'application/x-7z-compressed',
        'application/x-tar',
        'application/gzip',
      ]) {
        expect(FileCategory.fromMimeType(mime), FileCategory.archive,
            reason: '$mime 应识别为 Archive');
      }
    });

    test('安装包关键字', () {
      // 对齐 Rust contains 匹配：apk / dmg / pkg / debian / rpm
      expect(FileCategory.fromMimeType('application/x-dmg'),
          FileCategory.package);
      expect(FileCategory.fromMimeType('application/x-rpm'),
          FileCategory.package);
      expect(
          FileCategory.fromMimeType(
              'application/vnd.debian.binary-package'),
          FileCategory.package);
    });

    test('可执行关键字', () {
      expect(FileCategory.fromMimeType('application/x-msdownload'),
          FileCategory.executable);
      expect(FileCategory.fromMimeType('application/x-mach-binary'),
          FileCategory.executable);
    });

    test('null / 未知 → None', () {
      expect(FileCategory.fromMimeType(null), FileCategory.unknown);
      expect(FileCategory.fromMimeType('application/octet-stream'),
          FileCategory.unknown);
    });
  });

  group('DriveFile', () {
    test('fromJson parses huawei response（fileName 键）', () {
      final file = DriveFile.fromJson({
        'id': 'file-123',
        'fileName': '报告.pdf',
        'mimeType': 'application/pdf',
        'size': 1024,
        'parentFolder': ['parent-1', 'parent-2'],
        'description': '季度报告',
        'createdTime': '2025-01-15T08:30:00Z',
        'editedTime': '2025-06-20T12:00:00Z',
        'sha256': 'abc123',
        'thumbnailLink': 'https://example.com/thumb.png',
      });

      expect(file.id, 'file-123');
      expect(file.name, '报告.pdf');
      expect(file.category, FileCategory.document);
      expect(file.size, 1024);
      expect(file.parentFolder, ['parent-1', 'parent-2']);
      expect(file.parentId, 'parent-1');
      expect(file.description, '季度报告');
      expect(file.createdTime!.year, 2025);
      expect(file.createdTime!.isUtc, isTrue);
      expect(file.editedTime!.month, 6);
      expect(file.mimeType, 'application/pdf');
      expect(file.contentHash, 'abc123');
      expect(file.thumbnailLink, 'https://example.com/thumb.png');
      expect(file.isFolder, isFalse);
    });

    test('name 键兜底（标准命名）', () {
      final file = DriveFile.fromJson({'id': 'f1', 'name': 'via-name.txt'});

      expect(file.name, 'via-name.txt');
    });

    test('folder mimeType → isFolder', () {
      final file = DriveFile.fromJson({
        'id': 'folder-1',
        'fileName': 'Documents',
        'mimeType': 'application/vnd.huawei-apps.folder',
      });

      expect(file.isFolder, isTrue);
      expect(file.category, FileCategory.folder);
    });

    test('size 容忍 String 数字', () {
      final file = DriveFile.fromJson({
        'id': 'f1',
        'fileName': 'a.txt',
        'size': '2048',
      });

      expect(file.size, 2048);
    });

    test('size 容忍 double / 缺失默认 0', () {
      expect(
        DriveFile.fromJson({'id': 'f', 'size': 99.0}).size,
        99,
      );
      expect(DriveFile.fromJson({'id': 'f'}).size, 0);
    });

    test('parentId 无父目录时为 null', () {
      expect(DriveFile.fromJson({'id': 'f'}).parentId, isNull);
      expect(
        DriveFile.fromJson({'id': 'f', 'parentFolder': []}).parentId,
        isNull,
      );
    });

    group('contentHash 别名兼容', () {
      for (final key in [
        'sha256',
        'md5',
        'md5Checksum',
        'fileSha256',
        'hash',
        'contentHash',
      ]) {
        test('解析别名 $key', () {
          final file = DriveFile.fromJson({'id': 'f', key: 'hash-value'});

          expect(file.contentHash, 'hash-value');
        });
      }

      test('别名优先级：sha256 优先于 md5', () {
        final file = DriveFile.fromJson({
          'id': 'f',
          'md5': 'md5-value',
          'sha256': 'sha256-value',
        });

        expect(file.contentHash, 'sha256-value');
      });
    });

    test('tryFromJson 拒绝非字符串 id', () {
      expect(DriveFile.tryFromJson({'fileName': 'x'}), isNull);
      expect(DriveFile.tryFromJson({'id': 123}), isNull);
      expect(DriveFile.tryFromJson({'id': 'ok'}), isNotNull);
    });

    test('toJson 对齐 Rust：fileName 键 + contentHash→sha256 + size>0 才输出', () {
      const file = DriveFile(
        id: 'f1',
        name: 'a.txt',
        size: 100,
        mimeType: 'text/plain',
        contentHash: 'hash-1',
      );

      final json = file.toJson();
      expect(json['id'], 'f1');
      expect(json['fileName'], 'a.txt');
      expect(json['size'], 100);
      expect(json['mimeType'], 'text/plain');
      expect(json['sha256'], 'hash-1');
      expect(json.containsKey('contentHash'), isFalse);
    });

    test('toJson size 为 0 时不输出 size 键', () {
      const file = DriveFile(id: 'f1', name: 'a.txt');

      expect(file.toJson().containsKey('size'), isFalse);
    });

    test('toJson 可空字段为 null 时不输出对应键', () {
      const file = DriveFile(id: 'f1', name: 'a.txt');
      final json = file.toJson();

      for (final key in [
        'parentFolder',
        'description',
        'createdTime',
        'editedTime',
        'mimeType',
        'sha256',
      ]) {
        expect(json.containsKey(key), isFalse, reason: '$key 不应输出');
      }
    });

    test('fromJson/toJson 往返保留关键字段', () {
      final original = DriveFile.fromJson({
        'id': 'f1',
        'fileName': 'photo.jpg',
        'mimeType': 'image/jpeg',
        'size': 4096,
        'parentFolder': ['p1'],
        'createdTime': '2025-03-01T00:00:00Z',
        'editedTime': '2025-03-02T00:00:00Z',
        'sha256': 'hash-x',
      });

      final restored = DriveFile.fromJson(original.toJson());

      expect(restored.id, original.id);
      expect(restored.name, original.name);
      expect(restored.category, original.category);
      expect(restored.size, original.size);
      expect(restored.parentFolder, original.parentFolder);
      expect(restored.createdTime, original.createdTime);
      expect(restored.editedTime, original.editedTime);
      expect(restored.mimeType, original.mimeType);
      expect(restored.contentHash, original.contentHash);
    });

    test('copyWith 替换字段并可显式清空', () {
      const file = DriveFile(
        id: 'f1',
        name: 'a.txt',
        mimeType: 'text/plain',
        contentHash: 'h',
      );

      final renamed = file.copyWith(name: 'b.txt');
      expect(renamed.name, 'b.txt');
      expect(renamed.mimeType, 'text/plain');

      final cleared = file.copyWith(contentHash: null);
      expect(cleared.contentHash, isNull);
      expect(cleared.mimeType, 'text/plain');
    });
  });

  group('DriveAbout', () {
    test('fromJson 解析嵌套 storageQuota + user.displayName', () {
      final about = DriveAbout.fromJson({
        'storageQuota': {
          'userCapacity': 16106127360,
          'usedSpace': 5368709120,
        },
        'user': {'displayName': '张三'},
      });

      expect(about.userCapacity, 16106127360);
      expect(about.usedSpace, 5368709120);
      expect(about.userDisplayName, '张三');
    });

    test('配额字段容忍 String（华为返回 String）', () {
      final about = DriveAbout.fromJson({
        'storageQuota': {
          'userCapacity': '16106127360',
          'usedSpace': '5368709120',
        },
      });

      expect(about.userCapacity, 16106127360);
      expect(about.usedSpace, 5368709120);
    });

    test('无 storageQuota 时回退顶层字段', () {
      final about = DriveAbout.fromJson({
        'userCapacity': 100,
        'usedSpace': 40,
      });

      expect(about.userCapacity, 100);
      expect(about.usedSpace, 40);
    });

    test('remainingSpace / canFit', () {
      const about = DriveAbout(userCapacity: 1000, usedSpace: 300);

      expect(about.remainingSpace, 700);
      expect(about.canFit(700), isTrue);
      expect(about.canFit(701), isFalse);
    });

    test('toJson 使用 snake_case 键', () {
      const about = DriveAbout(
        userCapacity: 100,
        usedSpace: 40,
        userDisplayName: '张三',
      );

      final json = about.toJson();
      expect(json['user_capacity'], 100);
      expect(json['used_space'], 40);
      expect(json['user_display_name'], '张三');
    });
  });

  group('FileListResult', () {
    test('fromJson 解析 files + nextCursor', () {
      final result = FileListResult.fromJson({
        'files': [
          {'id': 'f1', 'fileName': 'a.txt'},
          {'id': 'f2', 'fileName': 'b.txt'},
        ],
        'nextCursor': 'cursor-page-2',
      });

      expect(result.files.length, 2);
      expect(result.files[0].name, 'a.txt');
      expect(result.nextCursor, 'cursor-page-2');
      expect(result.hasNext, isTrue);
    });

    test('游标兼容 cursor 键', () {
      final result = FileListResult.fromJson({
        'files': <dynamic>[],
        'cursor': 'legacy-cursor',
      });

      expect(result.nextCursor, 'legacy-cursor');
      expect(result.hasNext, isTrue);
    });

    test('空游标 / 缺失 → hasNext=false', () {
      expect(
        FileListResult.fromJson({'files': <dynamic>[]}).hasNext,
        isFalse,
      );
      expect(
        FileListResult.fromJson({
          'files': <dynamic>[],
          'nextCursor': '',
        }).hasNext,
        isFalse,
      );
    });

    test('跳过无 id 的无效条目', () {
      final result = FileListResult.fromJson({
        'files': [
          {'fileName': 'no-id.txt'},
          {'id': 'f1', 'fileName': 'valid.txt'},
          'not-a-map',
        ],
      });

      expect(result.files.length, 1);
      expect(result.files.single.id, 'f1');
    });
  });
}

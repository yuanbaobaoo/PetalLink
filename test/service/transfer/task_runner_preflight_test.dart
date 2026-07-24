import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:path/path.dart' as p;
import 'package:petal_link/entity/transfer_task.dart';
import 'package:petal_link/service/transfer/task_runner_preflight.dart';
import 'package:petal_link/types/enums.dart';

/// validateStaticTask 静态校验专项测试（对齐 Rust `task_runner/preflight.rs`）。
///
/// 聚焦 Update 上传分支的云端版本快照校验：规划时必须携带远端 editedTime，
/// 否则 preflight 无法拒绝覆盖并发修改，需回 planner 重新规划。

void main() {
  late Directory mountDir;

  setUp(() async {
    mountDir = await Directory.systemTemp.createTemp('preflight_');
  });

  tearDown(() async {
    if (mountDir.existsSync()) {
      await mountDir.delete(recursive: true);
    }
  });

  /// 构造一个已落盘的上传源文件，返回满足静态校验的 Update 任务基线。
  Future<TransferTask> makeUpdateTask(
    String rel, {
    int? expectedCloudEditedTime = 1690000000000,
    String fileId = 'real-file-id',
  }) async {
    final file = File(p.join(mountDir.path, rel));
    await file.create(recursive: true);
    await file.writeAsBytes(List<int>.filled(100, 7), flush: true);
    final stat = await file.stat();
    return TransferTask(
      direction: TransferDirection.upload,
      fileId: fileId,
      localPath: file.path,
      name: p.basename(rel),
      totalSize: stat.size,
      relativePath: rel,
      operation: TransferOperation.update,
      sourceMtime: stat.modified.millisecondsSinceEpoch,
      sourceSize: stat.size,
      expectedCloudEditedTime: expectedCloudEditedTime,
      createdAt: 1,
    );
  }

  group('Update 云端版本快照校验（对齐 Rust preflight.rs:70-76）', () {
    test('携带云端版本快照 → 校验通过', () async {
      final task = await makeUpdateTask('a.txt');
      final op = await validateStaticTask(task, mountRoot: mountDir.path);
      expect(op, TransferOperation.update);
    });

    test('缺少云端版本快照 → 判 localChanged 需重新规划', () async {
      // Update 任务若未携带规划时远端版本，preflight 必须拒绝，
      // 否则无法检测「规划后云端又变了」的并发覆盖。
      final task = await makeUpdateTask('a.txt', expectedCloudEditedTime: null);
      expect(
        () => validateStaticTask(task, mountRoot: mountDir.path),
        throwsA(isA<PreflightFailure>()
            .having((e) => e.target, 'target',
                TransferState.restartRequired)
            .having((e) => e.kind, 'kind', TransferErrorKind.localChanged)
            .having((e) => e.message, 'message', contains('云端版本快照'))),
      );
    });
  });
}

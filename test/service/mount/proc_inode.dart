/// 测试用批量 inode 查询：经 /usr/bin/stat 读取真实文件系统 inode。
///
/// 与真机语义一致：mv 改名 inode 不变，cp 复制产生新 inode——
/// 这是 inode 身份方案（docs/design/10）测试的关键前提。
library;

import 'dart:io';

/// 批量查询 paths 的 inode（stat -f %i）；失败路径跳过。
Future<Map<String, int>> procInodeBatch(List<String> paths) async {
  final out = <String, int>{};
  for (final path in paths) {
    final r = await Process.run('/usr/bin/stat', ['-f', '%i', path]);
    if (r.exitCode == 0) {
      out[path] = int.parse((r.stdout as String).trim());
    }
  }
  return out;
}

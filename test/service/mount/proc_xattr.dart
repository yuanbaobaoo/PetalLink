/// 测试用 xattr 实现：基于 /usr/bin/xattr CLI 读写真实文件系统 xattr。
///
/// 相比内存 fake，xattr 随 inode 走（rename 后仍挂在文件上），
/// 与真机语义一致，能覆盖 free-up 暂存改名等路径。
library;

import 'dart:io';
import 'dart:typed_data';

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/service/mount/xattr_service.dart';

/// 经 /usr/bin/xattr 读写的真实 xattr 实现（仅测试用）。
class ProcXattrService extends XattrService {
  @override
  Future<Uint8List?> getBytes(String path, String name) async {
    final r = await Process.run('/usr/bin/xattr', ['-px', name, path]);
    if (r.exitCode != 0) return null;
    final hex = (r.stdout as String).replaceAll(RegExp(r'\s+'), '');
    if (hex.isEmpty) return Uint8List(0);
    final out = Uint8List(hex.length ~/ 2);
    for (var i = 0; i < out.length; i++) {
      out[i] = int.parse(hex.substring(i * 2, i * 2 + 2), radix: 16);
    }
    return out;
  }

  @override
  Future<void> setBytes(String path, String name, Uint8List value) async {
    final hex = value.map((b) => b.toRadixString(16).padLeft(2, '0')).join();
    final r = await Process.run('/usr/bin/xattr', ['-wx', name, hex, path]);
    if (r.exitCode != 0) {
      throw AppError.generic('xattr 写入失败（$name）：${r.stderr}');
    }
  }

  @override
  Future<void> remove(String path, String name) async {
    // 幂等：不存在也视为成功
    await Process.run('/usr/bin/xattr', ['-d', name, path]);
  }

  @override
  Future<List<String>> list(String path) async {
    final r = await Process.run('/usr/bin/xattr', [path]);
    if (r.exitCode != 0) return const [];
    return (r.stdout as String)
        .split('\n')
        .map((e) => e.trim())
        .where((e) => e.isNotEmpty)
        .toList();
  }
}

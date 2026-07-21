/// 文件 SHA-256 哈希（带 mtime+size 缓存）。
///
/// 严格对齐 Rust 原版 `src/mount/file_hasher.rs`：
/// - 流式计算（不整文件加载到内存，64KB 缓冲）
/// - 缓存：key=绝对路径 → {mtimeMs, size, sha256}，若 mtime+size 未变则返回缓存
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'package:crypto/crypto.dart';

import 'package:petal_link/core/error/app_error.dart';

/// 哈希缓存条目
class _CacheEntry {
  /// 修改时间（毫秒 epoch）
  final int mtimeMs;

  /// 文件大小（字节）
  final int size;

  /// SHA-256（hex 小写）
  final String sha256;

  const _CacheEntry({
    required this.mtimeMs,
    required this.size,
    required this.sha256,
  });
}

/// 收集 chunked conversion 输出摘要的简易 Sink
/// （Dart SDK 已无 AccumulateSink，crypto 分块转换输出恰好一条 Digest）。
class _DigestCollector implements Sink<Digest> {
  /// 已产出的摘要
  Digest? value;

  @override
  void add(Digest data) => value = data;

  @override
  void close() {}
}

/// 文件哈希器（带 mtime+size 缓存）。
///
/// 对齐 Rust `FileHasher`。缓存为进程内存级，随实例销毁。
class FileHasher {
  /// 流式读缓冲大小（对齐 Rust 64KB）
  static const int _bufferSize = 64 * 1024;

  final Map<String, _CacheEntry> _cache = {};

  /// 计算文件 SHA-256（hex 小写）。
  ///
  /// 若 mtime+size 与缓存一致则返回缓存（不重算）。
  Future<String> hashFile(String path) async {
    final FileStat stat;
    try {
      stat = await FileStat.stat(path);
    } catch (e) {
      throw AppError.generic('读取文件元数据失败：$e');
    }
    final mtimeMs = stat.modified.millisecondsSinceEpoch;
    final size = stat.size;

    // 缓存命中检查
    final cached = _cache[path];
    if (cached != null && cached.mtimeMs == mtimeMs && cached.size == size) {
      return cached.sha256;
    }

    // 流式计算 SHA-256
    final output = _DigestCollector();
    final input = sha256.startChunkedConversion(output);
    try {
      await for (final chunk in File(path).openRead()) {
        input.add(chunk);
      }
    } catch (e) {
      throw AppError.generic('读取文件失败：$e');
    } finally {
      input.close();
    }
    final result = output.value.toString();

    // 更新缓存
    _cache[path] = _CacheEntry(mtimeMs: mtimeMs, size: size, sha256: result);
    return result;
  }

  /// 计算文件指定区间的 SHA-256（用于 resume 校验，可选偏移读取）。
  Future<String> hashRange(String path, int offset, int len) async {
    final output = _DigestCollector();
    final input = sha256.startChunkedConversion(output);
    RandomAccessFile? raf;
    try {
      raf = await File(path).open();
      await raf.setPosition(offset);
      var remaining = len;
      final buf = Uint8List(_bufferSize);
      while (remaining > 0) {
        final toRead = min(remaining, buf.length);
        final n = await raf.readInto(buf, 0, toRead);
        if (n == 0) break;
        input.add(Uint8List.sublistView(buf, 0, n));
        remaining -= n;
      }
    } catch (e) {
      throw AppError.generic('读取文件区间失败：$e');
    } finally {
      input.close();
      await raf?.close();
    }
    return output.value.toString();
  }

  /// 失效某文件的缓存（文件被修改/删除后调用）。
  void invalidate(String path) {
    _cache.remove(path);
  }

  /// 清空全部缓存。
  void clear() {
    _cache.clear();
  }

  /// 计算字符串的 SHA-256（非文件场景）。
  static String sha256OfString(String s) {
    return sha256.convert(utf8.encode(s)).toString();
  }
}

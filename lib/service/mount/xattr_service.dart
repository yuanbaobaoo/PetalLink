/// 扩展属性（xattr）服务 —— 占位符状态与 Finder 标签的数据源头。
///
/// 严格对齐 Rust 原版 `src/mount/manager.rs` 的 xattr 读写语义
/// （macOS getxattr(2)/setxattr(2)/removexattr(2)/listxattr(2)，
/// 经 MethodChannel `com.petallink/platform` 桥接，UTF-8 字符串值在 Dart 侧编解码）。
///
/// 测试可注入内存 fake，不依赖原生通道。
library;

import 'dart:convert';

import 'package:flutter/services.dart';

import 'package:petal_link/core/error/app_error.dart';

/// xattr 读写抽象（供 fake 注入）。
///
/// 字符串读写基于字节读写实现，子类只需提供 4 个原语。
abstract class XattrService {
  /// 读取 xattr 原始字节；属性不存在返回 null。
  Future<Uint8List?> getBytes(String path, String name);

  /// 写入 xattr 原始字节。
  Future<void> setBytes(String path, String name, Uint8List value);

  /// 移除 xattr（幂等：不存在也视为成功）。
  Future<void> remove(String path, String name);

  /// 列出全部 xattr 名。
  Future<List<String>> list(String path);

  /// 读取 UTF-8 字符串 xattr；缺失、读取失败或损坏均返回 null
  /// （对齐 Rust `read_xattr_string` 的尽力读取语义）。
  Future<String?> get(String path, String name) async {
    final Uint8List? bytes;
    try {
      bytes = await getBytes(path, name);
    } catch (_) {
      return null;
    }
    if (bytes == null) return null;
    final String value;
    try {
      value = utf8.decode(bytes);
    } catch (_) {
      return null;
    }
    return value.isEmpty ? null : value;
  }

  /// 写入 UTF-8 字符串 xattr。
  Future<void> set(String path, String name, String value) {
    return setBytes(path, name, Uint8List.fromList(utf8.encode(value)));
  }
}

/// 基于 MethodChannel `com.petallink/platform` 的原生 xattr 实现。
///
/// 原生侧（macos/Runner/MainFlutterWindow.swift）直接调用
/// getxattr(2)/setxattr(2)/removexattr(2)/listxattr(2)。
class ChannelXattrService extends XattrService {
  /// 平台通道名（与原生侧约定一致）
  static const MethodChannel channel = MethodChannel('com.petallink/platform');

  @override
  Future<Uint8List?> getBytes(String path, String name) async {
    try {
      return await channel.invokeMethod<Uint8List>('getXattr', {
        'path': path,
        'name': name,
      });
    } on PlatformException catch (e) {
      throw AppError.generic('读取 xattr 失败（$name）：${e.message}');
    }
  }

  @override
  Future<void> setBytes(String path, String name, Uint8List value) async {
    try {
      await channel.invokeMethod<void>('setXattr', {
        'path': path,
        'name': name,
        'value': value,
      });
    } on PlatformException catch (e) {
      throw AppError.generic('写入 xattr 失败（$name）：${e.message}');
    }
  }

  @override
  Future<void> remove(String path, String name) async {
    try {
      await channel.invokeMethod<void>('removeXattr', {
        'path': path,
        'name': name,
      });
    } on PlatformException catch (e) {
      throw AppError.generic('移除 xattr 失败（$name）：${e.message}');
    }
  }

  @override
  Future<List<String>> list(String path) async {
    try {
      final names = await channel.invokeMethod<List<Object?>>('listXattrs', {
        'path': path,
      });
      return (names ?? const []).whereType<String>().toList();
    } on PlatformException catch (e) {
      throw AppError.generic('列出 xattr 失败：${e.message}');
    }
  }
}

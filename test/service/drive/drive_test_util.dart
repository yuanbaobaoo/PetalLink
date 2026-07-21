import 'dart:convert';

import 'package:dio/dio.dart';

import 'package:petal_link/core/http/mate_http_client.dart';

import '../auth/fake_http.dart';

/// drive 服务测试共享工具。

/// 构造接入 [adapter] 的 MateHttpClient（固定 token，不触发刷新）。
MateHttpClient buildTestClient(
  FakeHttpAdapter adapter, {
  String token = 'test-token',
  Future<String?> Function()? refreshTokenProvider,
  void Function()? onAuthExpired,
}) {
  return MateHttpClient(
    baseUrl: '',
    tokenProvider: () async => token,
    refreshTokenProvider: refreshTokenProvider ?? () async => null,
    onAuthExpired: onAuthExpired ?? () {},
    dio: adapter.createDio(),
  );
}

/// 构造带自定义响应头的 JSON 响应体。
ResponseBody jsonResponseWithHeaders(
  Map<String, dynamic> json, {
  int status = 200,
  Map<String, List<String>> headers = const {},
}) {
  return ResponseBody.fromString(
    jsonEncode(json),
    status,
    headers: {
      Headers.contentTypeHeader: ['application/json'],
      ...headers,
    },
  );
}

/// 华为 File 资源的典型 JSON（严格解析所需的最小字段集）。
Map<String, dynamic> fileJson({
  required String id,
  required String name,
  String mimeType = 'text/plain',
  int? size,
  List<String>? parentFolder,
  String? editedTime,
  String? createdTime,
  String? sha256,
}) {
  return {
    'category': 'drive#file',
    'id': id,
    'fileName': name,
    'mimeType': mimeType,
    'size': ?size,
    'parentFolder': ?parentFolder,
    'createdTime': ?createdTime,
    'editedTime': ?editedTime,
    'sha256': ?sha256,
  };
}

/// 华为文件夹 File JSON。
Map<String, dynamic> folderJson({
  required String id,
  required String name,
  List<String>? parentFolder,
}) {
  return fileJson(
    id: id,
    name: name,
    mimeType: 'application/vnd.huawei-apps.folder',
    parentFolder: parentFolder,
  );
}

/// Files:list 单页 JSON。
Map<String, dynamic> fileListPageJson(
  List<Map<String, dynamic>> files, {
  String? nextCursor,
}) {
  return {
    'category': 'drive#fileList',
    'files': files,
    'nextCursor': ?nextCursor,
  };
}

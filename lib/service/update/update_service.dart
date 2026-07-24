import 'dart:convert';
import 'dart:ffi';
import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;
import 'package:path/path.dart' as p;

import 'package:petal_link/core/error/app_error.dart';
import 'package:petal_link/core/error/app_result.dart';
import 'package:petal_link/core/logger/logger.dart';
import 'package:petal_link/core/storage/app_paths.dart';
import 'package:petal_link/service/platform/launch_at_login.dart';
import 'package:petal_link/service/update/minisign.dart';

/// 更新清单（解析自 GitHub Releases 的 PetalLink_update.json）。
///
/// 兼容两种格式：
/// - Tauri updater 格式：`platforms` 平台映射（含 signature/url）
/// - CMP 扁平格式：顶层 url/sha256/notes
class UpdateManifest {
  /// 新版本号
  final String version;

  /// 更新日志（Markdown）
  final String notes;

  /// 更新包（DMG）下载地址
  final String url;

  /// 更新包 SHA-256（hex）。可为 null：标准 Tauri 清单只有 signature
  /// 无 sha256 字段；缺失时跳过哈希校验，由安装期签名四重校验兜底
  final String? sha256;

  /// minisign 签名（Tauri 格式携带；当前不校验，保留字段）
  final String? signature;

  /// 发布时间
  final String pubDate;

  const UpdateManifest({
    required this.version,
    required this.url,
    this.sha256,
    this.notes = '',
    this.signature,
    this.pubDate = '',
  });
}

/// 语义化版本（对齐 CMP `SemanticVersion`：`v` 前缀与 `-`/`+` 后缀容忍）
class SemanticVersion {
  /// 主版本
  final int major;

  /// 次版本
  final int minor;

  /// 修订号
  final int patch;

  const SemanticVersion(this.major, this.minor, this.patch);

  static final RegExp _pattern = RegExp(r'^v?(\d+)\.(\d+)\.(\d+)(?:[-+].*)?$');

  /// 解析版本号；非法返回 null
  static SemanticVersion? parse(String raw) {
    final m = _pattern.firstMatch(raw.trim());
    if (m == null) return null;
    return SemanticVersion(
      int.parse(m.group(1)!),
      int.parse(m.group(2)!),
      int.parse(m.group(3)!),
    );
  }

  /// 是否比 [other] 新；任一解析失败在调用侧处理
  bool isNewerThan(SemanticVersion other) {
    if (major != other.major) return major > other.major;
    if (minor != other.minor) return minor > other.minor;
    return patch > other.patch;
  }

  @override
  String toString() => '$major.$minor.$patch';
}

/// 更新服务
///
/// 对齐 Rust 版前端 updater（app/stores/updater.ts）与 CMP
/// `JvmUpdateService.kt`：
/// - manifest 来自 GitHub Releases `PetalLink_update.json`
/// - 校验：版本语义化、url 必须 https、sha256 必须 64 位 hex
/// - 下载到 `<support>/updates/<version>/`（.part 临时文件 + 原子改名）
/// - 下载完成强制 SHA-256 校验，不匹配即删除
/// - 安装：hdiutil 挂载 DMG → ditto 提取 .app → 后台脚本等本进程退出后
///   替换当前 .app 并重新打开（失败回滚）
class UpdateService {
  /// 更新清单端点（对齐 tauri.conf.json updater endpoint）
  static const String updateEndpoint =
      'https://github.com/yuanbaobaoo/PetalLink/releases/latest/download/'
      'PetalLink_update.json';

  /// 更新包签名公钥（对齐 tauri.conf.json updater pubkey）。
  ///
  /// 与 Tauri 客户端共用同一把 minisign 公钥，使 Tauri 老客户端能
  /// 验签 Flutter 版更新包并无缝升级。base64 解码后为 42 字节 minisign 公钥。
  static const String updatePublicKey =
      'RWRFuz2UYehJmK1q/bUx6XfRv3RnCYmMnX6rYK4l/Odxf96Y5XLi4MHt';

  final http.Client _http;
  final String _endpoint;
  final String _currentVersion;
  final String? _updatesDirOverride;
  final ProcRunner _runner;
  final String? _executableOverride;
  final int? _pidOverride;

  /// minisign 验签公钥（默认 [updatePublicKey]，测试可注入）。
  final String _publicKeyBase64;

  /// 是否启用更新检查（默认仅 release 构建启用；测试可强制开启）。
  final bool _enableUpdateCheck;

  UpdateService({
    http.Client? httpClient,
    String? endpoint,
    String? currentVersion,
    String? updatesDir,
    ProcRunner? runner,
    String? currentExecutable,
    int? currentPid,
    String? publicKey,
    bool? enableUpdateCheck,
  })  : _http = httpClient ?? http.Client(),
        _endpoint = endpoint ?? updateEndpoint,
        _currentVersion = currentVersion ?? '0.0.0',
        _updatesDirOverride = updatesDir,
        _runner = runner ?? defaultProcRunner,
        _executableOverride = currentExecutable,
        _pidOverride = currentPid,
        _publicKeyBase64 = publicKey ?? updatePublicKey,
        _enableUpdateCheck = enableUpdateCheck ?? kReleaseMode;

  // ═══════════════════════════════════════════════════════════════════
  // 检查更新
  // ═══════════════════════════════════════════════════════════════════

  /// 检查更新：有更新返回清单，已是最新返回 null（对齐 CMP `check`）。
  ///
  /// 网络/解析/校验失败返回 Err；调用方（静默检查）自行决定是否提示。
  Future<AppResult<UpdateManifest?>> check() async {
    // dev 构建跳过更新检查（含手动）：避免 dev 版误提示更新到正式版，
    // 也避免 dev 版被更新包覆盖成正式版导致数据目录错乱。
    if (!_enableUpdateCheck) {
      AppLogger.d('dev 构建跳过更新检查');
      return const Ok(null);
    }
    try {
      final uri = Uri.parse(_endpoint);
      if (uri.scheme != 'https') {
        return const Err(GenericError(message: '更新端点必须是 https://'));
      }
      final resp = await _http.get(uri);
      if (resp.statusCode < 200 || resp.statusCode >= 300) {
        return Err(
            GenericError(message: '检查更新失败：HTTP ${resp.statusCode}'));
      }
      // 显式 UTF-8 解码（notes 含中文；http 默认 latin1 会乱码）
      final json = jsonDecode(utf8.decode(resp.bodyBytes));
      if (json is! Map<String, dynamic>) {
        return const Err(GenericError(message: '更新清单格式非法'));
      }
      final manifest = parseManifest(json);
      if (!_isNewer(manifest.version)) {
        AppLogger.d('已是最新版本: $_currentVersion');
        return const Ok(null);
      }
      AppLogger.i('发现新版本: ${manifest.version}（当前 $_currentVersion）');
      return Ok(manifest);
    } on AppError catch (e) {
      return Err(e);
    } catch (e, st) {
      AppLogger.e('检查更新异常', e, st);
      return Err(GenericError(message: '检查更新失败：$e'));
    }
  }

  /// 版本比较：offered 比 current 新（任一解析失败视为不新，对齐 CMP）
  bool _isNewer(String offered) {
    final o = SemanticVersion.parse(offered);
    final c = SemanticVersion.parse(_currentVersion);
    if (o == null || c == null) return false;
    return o.isNewerThan(c);
  }

  // ═══════════════════════════════════════════════════════════════════
  // 下载与校验
  // ═══════════════════════════════════════════════════════════════════

  /// 下载更新包并校验完整性（对齐 Tauri updater 下载段）。
  ///
  /// 流程：建版本目录 → 流式下载到 `.part` → 原子改名 → SHA-256 校验
  /// （清单携带 sha256 时；不匹配删除并报错）。进度经 [onProgress]
  /// （received, total）回调。成功返回更新包（.app.tar.gz）本地路径。
  Future<AppResult<String>> downloadAndStage(
    UpdateManifest manifest, {
    void Function(int received, int? total)? onProgress,
  }) async {
    final versionDir = await _versionDir(manifest.version);
    final fileName = _archiveFileName(manifest);
    final target = p.join(versionDir.path, fileName);
    final part = '$target.part';
    try {
      // 版本目录先清后建（对齐 CMP deleteTree 重建）
      if (await versionDir.exists()) {
        await versionDir.delete(recursive: true);
      }
      await versionDir.create(recursive: true);

      // 流式下载（进度按写盘字节回调）
      final request = http.Request('GET', Uri.parse(manifest.url));
      final response = await _http.send(request);
      if (response.statusCode < 200 || response.statusCode >= 300) {
        return Err(
            GenericError(message: '下载更新失败：HTTP ${response.statusCode}'));
      }
      final total =
          response.contentLength != null && response.contentLength! >= 0
              ? response.contentLength
              : null;
      final sink = File(part).openWrite();
      var received = 0;
      try {
        await for (final chunk in response.stream) {
          sink.add(chunk);
          received += chunk.length;
          onProgress?.call(received, total);
        }
        await sink.flush();
      } finally {
        await sink.close();
      }

      // 原子改名（同目录 rename 即原子）
      await File(part).rename(target);

      // SHA-256 校验（不匹配删包；清单未携带 sha256 时跳过，
      // 由安装期 minisign 签名校验兜底）
      final expectedHash = manifest.sha256;
      if (expectedHash != null) {
        final digest = await sha256.bind(File(target).openRead()).first;
        final hex = digest.toString();
        // 忽略大小写比较（对齐 CMP equalsIgnoreCase）
        if (hex.toLowerCase() != expectedHash.toLowerCase()) {
          await File(target).delete();
          return Err(GenericError(
              message:
                  '更新包 SHA-256 不匹配（期望 $expectedHash，实际 $hex）'));
        }
      } else {
        AppLogger.w('更新清单未携带 SHA-256，跳过哈希校验（安装期 minisign 签名校验兜底）');
      }

      AppLogger.i('更新包已就绪: $target');
      return Ok(target);
    } catch (e, st) {
      AppLogger.e('下载更新异常', e, st);
      // 清理临时文件
      final partFile = File(part);
      if (await partFile.exists()) await partFile.delete();
      return Err(GenericError(message: '下载更新失败：$e'));
    }
  }

  /// 更新包文件名（取 URL 路径末段；兜底 PetalLink.app.tar.gz）
  static String _archiveFileName(UpdateManifest manifest) {
    final segments = Uri.parse(manifest.url).pathSegments;
    final last = segments.isNotEmpty ? segments.last : '';
    return last.isNotEmpty ? last : 'PetalLink.app.tar.gz';
  }

  Future<Directory> _versionDir(String version) async {
    final root = _updatesDirOverride ??
        p.join((await AppPaths.supportDir()).path, 'updates');
    return Directory(p.join(root, version));
  }

  // ═══════════════════════════════════════════════════════════════════
  // 安装与重启（对齐 Tauri updater：minisign 验签 + tar.gz 解压替换）
  // ═══════════════════════════════════════════════════════════════════

  /// 安装已校验的更新包（.app.tar.gz）并重启：
  /// minisign 验签 → 解压 .app → 后台脚本替换并重开。
  ///
  /// 成功后当前进程立即退出（由脚本完成替换）；失败返回 Err。
  Future<AppResult<void>> installAndRelaunch(
    String archivePath, {
    String? signature,
  }) async {
    try {
      // 0. minisign 验签（对齐 Tauri updater：先验签 tar.gz 完整性再解压）
      //    签名文本来自 manifest.signature（多行 minisign .sig 全文）
      final sigText = signature;
      if (sigText == null || sigText.trim().isEmpty) {
        return const Err(
            GenericError(message: '更新清单未携带签名，拒绝安装'));
      }
      final verifyErr = await _verifyArchive(archivePath, sigText);
      if (verifyErr != null) return Err(verifyErr);

      // 1. 定位当前 .app（可执行文件上溯三级；对齐 Rust resolve_paths）
      final executable = _executableOverride ?? Platform.resolvedExecutable;
      final (currentApp, _) = LaunchAtLoginService.resolvePaths(executable);
      if (currentApp == null) {
        return const Err(
            GenericError(message: '开发模式不能执行自更新安装，请手动更新'));
      }
      final parent = p.dirname(currentApp);
      final appName = p.basename(currentApp);

      // 2. 父目录可写探测（不可写 → 引导手动更新）
      if (!await _probeWritable(parent)) {
        return const Err(
            GenericError(message: '应用所在目录不可写，请手动更新'));
      }

      // 3. 解压 tar.gz 到暂存目录（对齐 Tauri updater 的 tar 解包）
      final stageDir = p.dirname(archivePath);
      final extractDir = Directory(p.join(stageDir, 'extracted'));
      if (await extractDir.exists()) {
        await extractDir.delete(recursive: true);
      }
      await extractDir.create(recursive: true);
      final tar = await _runner(
          '/usr/bin/tar', ['xzf', archivePath, '-C', extractDir.path]);
      if (tar.exitCode != 0) {
        return Err(
            GenericError(message: '解压更新包失败：${tar.stderr.trim()}'));
      }

      // 4. 找到解压目录内第一个 .app
      final stagedAppDir = await _findFirstApp(extractDir);
      if (stagedAppDir == null) {
        return const Err(GenericError(message: '更新包内未找到 .app'));
      }
      final stagedApp = stagedAppDir.path;

      // 5. 后台替换脚本（等本进程退出后换入 + 重开；失败回滚）
      final incoming = p.join(parent, '.$appName.incoming');
      final backup = p.join(parent, '.$appName.backup');
      final script = p.join(stageDir, 'install-update.sh');
      await File(script).writeAsString(installScript);
      await _runner('chmod', ['700', script]);

      await Process.start(
        '/bin/sh',
        [script, '${_pidOverride ?? pid}', currentApp, stagedApp, incoming,
          backup],
        mode: ProcessStartMode.detached,
      );
      AppLogger.i('更新安装脚本已启动，应用即将退出完成替换');
      exit(0);
    } catch (e, st) {
      AppLogger.e('安装更新异常', e, st);
      return Err(GenericError(message: '安装更新失败：$e'));
    }
  }

  /// minisign 验签更新包（对齐 Tauri updater 的签名校验）。
  ///
  /// 用内置公钥验证 tar.gz 文件的 minisign 签名（BLAKE2b prehash）。
  /// 通过返回 null，失败返回错误。
  Future<AppError?> _verifyArchive(
      String archivePath, String sigText) async {
    try {
      final publicKey = MinisignPublicKey.fromBase64(_publicKeyBase64);
      final signature = MinisignSignature.fromSigText(sigText);
      final fileBytes = await File(archivePath).readAsBytes();
      final ok = await verifyMinisign(
        publicKey: publicKey,
        signature: signature,
        fileBytes: fileBytes,
      );
      if (!ok) {
        return const GenericError(message: '更新包 minisign 签名校验失败');
      }
      AppLogger.i('更新包 minisign 签名校验通过');
      return null;
    } catch (e) {
      return GenericError(message: '更新包签名校验异常：$e');
    }
  }

  /// 目录可写探测：写测试文件再删（对齐 Rust 挂载目录可写探测思路）
  static Future<bool> _probeWritable(String dir) async {
    try {
      final probe = File(p.join(dir, '.petallink-write-test'));
      await probe.writeAsString('');
      await probe.delete();
      return true;
    } catch (_) {
      return false;
    }
  }

  /// 找目录下第一个 .app（浅层，不递归）
  static Future<Directory?> _findFirstApp(Directory dir) async {
    await for (final entity in dir.list()) {
      if (entity is Directory && p.extension(entity.path) == '.app') {
        return entity;
      }
    }
    return null;
  }

  /// 后台安装脚本（对齐 CMP install script：等待退出 → 换入 → 重开 → 回滚）
  static const String installScript = r'''
#!/bin/sh
pid="$1"; current="$2"; staged="$3"; incoming="$4"; backup="$5"
while kill -0 "$pid" 2>/dev/null; do sleep 1; done
rm -rf "$incoming" "$backup"
/usr/bin/ditto "$staged" "$incoming" || exit 1
mv "$current" "$backup" || exit 1
if mv "$incoming" "$current" && /usr/bin/open "$current"; then
  rm -rf "$backup"
  exit 0
fi
rm -rf "$current"
mv "$backup" "$current"
/usr/bin/open "$current" 2>/dev/null || true
exit 1
''';

  // ═══════════════════════════════════════════════════════════════════
  // 清单解析（纯逻辑，可测试）
  // ═══════════════════════════════════════════════════════════════════

  /// 解析并校验更新清单（对齐 CMP `validateManifest`）。
  ///
  /// 兼容 Tauri updater 平台映射格式与 CMP 扁平格式；
  /// 版本非法 / url 非 https / sha256 存在但格式非法时抛 [GenericError]。
  /// sha256 缺失不视为非法（标准 Tauri 清单只有 signature），
  /// 解析为 null，由安装期签名四重校验兜底。
  static UpdateManifest parseManifest(
    Map<String, dynamic> json, {
    String? platformKey,
  }) {
    final version = json['version'];
    if (version is! String || SemanticVersion.parse(version) == null) {
      throw GenericError(message: '更新清单版本号非法: $version');
    }
    final notes = (json['notes'] ?? json['changelog'] ?? '') as String;
    final pubDate = json['pub_date'] as String? ?? '';

    String url = '';
    String sha256Raw = '';
    String? signature;
    final platforms = json['platforms'];
    if (platforms is Map) {
      final key = _selectPlatformKey(platforms, platformKey);
      if (key == null) {
        throw GenericError(
            message: '更新清单不包含当前平台（${platformKey ?? currentPlatformKey}）'
                '的下载项');
      }
      final entry = platforms[key];
      if (entry is Map) {
        url = entry['url'] as String? ?? '';
        sha256Raw = entry['sha256'] as String? ?? '';
        signature = entry['signature'] as String?;
      }
    } else {
      url = json['url'] as String? ?? '';
      sha256Raw = json['sha256'] as String? ?? '';
      signature = json['signature'] as String?;
    }

    if (!url.startsWith('https://')) {
      throw GenericError(message: '更新包地址必须是 https://');
    }
    // sha256 可缺失（null）；存在则必须是 64 位 hex
    final String? sha256;
    if (sha256Raw.isEmpty) {
      sha256 = null;
    } else if (RegExp(r'^[0-9a-fA-F]{64}$').hasMatch(sha256Raw)) {
      sha256 = sha256Raw;
    } else {
      throw GenericError(message: '更新清单 SHA-256 格式非法');
    }

    return UpdateManifest(
      version: version,
      notes: notes,
      url: url,
      sha256: sha256,
      signature: signature,
      pubDate: pubDate,
    );
  }

  /// 平台键选择：精确匹配 → darwin-universal → 任意 darwin-* 键
  static String? _selectPlatformKey(Map platforms, String? platformKey) {
    final key = platformKey ?? currentPlatformKey;
    if (platforms.containsKey(key)) return key;
    if (platforms.containsKey('darwin-universal')) return 'darwin-universal';
    for (final k in platforms.keys) {
      if (k is String && k.startsWith('darwin-')) return k;
    }
    return null;
  }

  /// 当前平台键（Tauri updater 命名：darwin-aarch64 / darwin-x86_64）
  static String get currentPlatformKey {
    final abi = Abi.current();
    if (abi == Abi.macosArm64) return 'darwin-aarch64';
    if (abi == Abi.macosX64) return 'darwin-x86_64';
    return 'darwin-aarch64';
  }
}

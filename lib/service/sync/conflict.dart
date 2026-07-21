/// 冲突处理 —— 60s 容忍 + 副本去重。
///
/// 严格对齐 Rust 原版 `src/sync/conflict.rs`：
/// 仅当本地 mtime 比云端 editedTime 晚 > 60s 才以本地为准；
/// 否则以云端为准（云端时间是可信基准，避免本地时钟偏差误判）。
/// 副本命名：`原名 (本地/云端副本 YYYY-MM-DD HH-mm-ss).ext`，同名加序号。
library;

import 'dart:io';

import 'package:intl/intl.dart';
import 'package:path/path.dart' as p;

import 'package:petal_link/entity/drive_file.dart';

/// 本地赢的容忍阈值：本地 mtime 必须比云端 editedTime 晚超过 60s
const int conflictLocalWinThresholdSecs = 60;

/// 冲突方
enum ConflictSide {
  /// 本地
  local,

  /// 云端
  cloud,
}

/// 冲突解决结果（对齐 Rust `ConflictResolution`）。
class ConflictResolution {
  /// 获胜方
  final ConflictSide winner;

  /// 获胜方的本地路径
  final String localPath;

  /// 副本路径（失败方被拷贝到这里）
  final String copyPath;

  /// 日志描述
  final String logMessage;

  const ConflictResolution({
    required this.winner,
    required this.localPath,
    required this.copyPath,
    required this.logMessage,
  });
}

/// 时间戳格式化：`YYYY-MM-dd HH-mm-ss`（文件系统安全）。
final DateFormat _stampFormat = DateFormat('yyyy-MM-dd HH-mm-ss');

/// 冲突解决器（对齐 Rust `ConflictResolver`）。
class ConflictResolver {
  /// 冲突日志（newest-first）
  final List<String> _log = [];

  /// 冲突日志快照（newest-first）
  List<String> get log => List.unmodifiable(_log);

  /// 解决冲突：判断胜者 + 生成副本路径（对齐 Rust `resolve`）。
  ///
  /// [localMtimeMs] 本地 mtime（毫秒 epoch）；云端 editedTime 缺失时按现在处理。
  Future<ConflictResolution> resolve(
    String localPath,
    DriveFile cloudFile,
    int localMtimeMs,
  ) async {
    final cloudTime = cloudFile.editedTime ?? DateTime.now().toUtc();
    final localMtime =
        DateTime.fromMillisecondsSinceEpoch(localMtimeMs, isUtc: true);

    // 容忍度：仅当本地比云端晚 > 60s 才以本地为准
    final deltaSecs = localMtime.difference(cloudTime).inSeconds;
    final winner = deltaSecs > conflictLocalWinThresholdSecs
        ? ConflictSide.local
        : ConflictSide.cloud;

    // 时间戳来自败方（较早的一方）
    final stamp = winner == ConflictSide.local ? cloudTime : localMtime;
    final sideLabel = winner == ConflictSide.local ? '云端副本' : '本地副本';

    final copyPath = await dedupeCopyPath(localPath, sideLabel, stamp);

    final logEntry = '[${_stampFormat.format(DateTime.now())}] 冲突：'
        '${p.basename(localPath)} | '
        '正本=${winner == ConflictSide.local ? '本地' : '云端'} '
        '(本地=${_stampFormat.format(localMtime)} '
        '云端=${_stampFormat.format(cloudTime)}) → 副本 ${p.basename(copyPath)}';
    _log.insert(0, logEntry);

    return ConflictResolution(
      winner: winner,
      localPath: localPath,
      copyPath: copyPath,
      logMessage: logEntry,
    );
  }
}

/// 副本路径去重（对齐 Rust `dedupe_copy_path`）。
///
/// 首选 `stem (sideLabel stamp).ext`；撞名追加 ` (seq)` 序号（最多 1000 次）。
Future<String> dedupeCopyPath(
  String localPath,
  String sideLabel,
  DateTime stamp,
) async {
  final dir = p.dirname(localPath);
  final stem = p.basenameWithoutExtension(localPath);
  final ext = p.extension(localPath);
  final stampStr = _stampFormat.format(stamp);

  for (var seq = 0; seq < 1000; seq++) {
    final name = seq == 0
        ? '$stem ($sideLabel $stampStr)$ext'
        : '$stem ($sideLabel $stampStr) ($seq)$ext';
    final candidate = p.join(dir, name);
    if (await FileSystemEntity.type(candidate, followLinks: false) ==
        FileSystemEntityType.notFound) {
      return candidate;
    }
  }

  // 兜底（不应触发）
  return p.join(dir,
      '$stem ($sideLabel ${DateTime.now().millisecondsSinceEpoch})$ext');
}

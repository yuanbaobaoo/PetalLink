//! 冲突处理 —— 60s 容忍 + 副本去重。
//!
//! 对齐 `legacy/lib/conflict/conflict_resolver.dart`。
//!
//! 逻辑：仅当本地 mtime 比云端 editedTime 晚 > 60s 才以本地为准；
//! 否则以云端为准（云端时间是可信基准，避免本地时钟偏差误判）。
//! 副本命名：`原名 (本地/云端副本 YYYY-MM-DD HH-mm-ss).ext`，同名加序号。

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::drive::models::DriveFile;

/// 冲突方
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictSide {
    Local,
    Cloud,
}

/// 冲突解决结果
#[derive(Debug, Clone)]
pub struct ConflictResolution {
    /// 获胜方
    pub winner: ConflictSide,
    /// 获胜方的本地路径
    pub local_path: PathBuf,
    /// 副本路径（失败方被拷贝到这里）
    pub copy_path: PathBuf,
    /// 日志描述
    pub log_message: String,
}

/// 冲突解决器
pub struct ConflictResolver {
    /// 冲突日志（newest-first，对齐 dart _conflictLog）
    log: Vec<String>,
}

impl ConflictResolver {
    /// 创建一个无冲突历史的解决器。
    pub fn new() -> Self {
        Self { log: Vec::new() }
    }

    /// 冲突日志快照（newest-first）
    pub fn log(&self) -> &[String] {
        &self.log
    }

    /// 解决冲突：判断胜者 + 生成副本路径。
    /// 对齐 dart `ConflictResolver.resolve`。
    pub fn resolve(
        &mut self,
        local_path: &Path,
        cloud_file: &DriveFile,
        local_mtime: &DateTime<Utc>,
    ) -> ConflictResolution {
        let cloud_time = cloud_file.edited_time.unwrap_or_else(Utc::now);

        // 容忍度：仅当本地比云端晚 > 60s 才以本地为准（对齐 dart 60s 阈值）
        let delta = *local_mtime - cloud_time;
        let winner = if delta.num_seconds() > 60 {
            ConflictSide::Local
        } else {
            ConflictSide::Cloud
        };

        // 时间戳来自败方（较早的一方）
        let stamp = if winner == ConflictSide::Local {
            cloud_time
        } else {
            *local_mtime
        };

        // 副本标签
        let side_label = if winner == ConflictSide::Local {
            "云端副本"
        } else {
            "本地副本"
        };

        let copy_path = dedupe_copy_path(local_path, side_label, &stamp);

        let log_entry = format!(
            "[{now}] 冲突：{basename} | 正本={winner_side} (本地={local_ts} 云端={cloud_ts}) → 副本 {copy_basename}",
            now = Utc::now().format("%Y-%m-%d %H:%M:%S"),
            basename = local_path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
            winner_side = if winner == ConflictSide::Local { "本地" } else { "云端" },
            local_ts = format_timestamp(local_mtime),
            cloud_ts = format_timestamp(&cloud_time),
            copy_basename = copy_path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
        );
        self.log.insert(0, log_entry.clone());

        ConflictResolution {
            winner,
            local_path: local_path.to_path_buf(),
            copy_path,
            log_message: log_entry,
        }
    }
}

impl Default for ConflictResolver {
    /// 默认创建空冲突解决器。
    fn default() -> Self {
        Self::new()
    }
}

/// 副本路径去重。对齐 dart `_dedupeCopyPath`。
pub fn dedupe_copy_path(local_path: &Path, side_label: &str, stamp: &DateTime<Utc>) -> PathBuf {
    let dir = local_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = local_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = local_path
        .extension()
        .map(|e| {
            let e = e.to_string_lossy();
            format!(".{e}")
        })
        .unwrap_or_default();

    let stamp_str = format_timestamp(stamp); // YYYY-MM-DD HH-mm-ss（冒号替换为 -）

    for seq in 0..1000 {
        let name = if seq == 0 {
            format!("{stem} ({side_label} {stamp_str}){ext}")
        } else {
            format!("{stem} ({side_label} {stamp_str}) ({seq}){ext}")
        };
        let candidate = dir.join(&name);
        if !candidate.exists() {
            return candidate;
        }
    }

    // 兜底（不应触发）
    dir.join(format!(
        "{stem} ({side_label} {}){ext}",
        Utc::now().timestamp_millis()
    ))
}

/// 时间戳格式化：`YYYY-MM-DD HH-mm-ss`（文件系统安全，对齐 dart `_formatStamp`）。
fn format_timestamp(dt: &DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d %H-%M-%S").to_string()
}

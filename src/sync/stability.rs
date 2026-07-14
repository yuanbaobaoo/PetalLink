//! 稳定性检查器 —— mtime > 5s + size 稳定 3s + lsof 双重检查。
//!
//! 对齐 `legacy/lib/sync/stability_checker.dart`。
//!
//! 三阶段检查（F-MOUNT-10 / §2.8 第二阶段）：
//! 1. mtime 距今 > 5s（编辑已停止至少 5 秒）
//! 2. size 在 3s 窗口内不变（文件大小稳定）
//! 3. lsof 无进程以写模式打开（+ 双重检查 + 只读系统进程白名单）
//!
//! 特殊场景（F-MOUNT-11）：持续编辑 > 5min → 标记「用户编辑中」暂停自动同步。

use std::path::Path;
use std::time::Duration;

use tokio::time::sleep;

/// 稳定性检查最低 mtime 滞后（秒）
pub const MIN_MTIME_AGE_SECS: u64 = 5;
/// 大小稳定窗口（秒）
pub const SIZE_STABLE_WINDOW_SECS: u64 = 3;
/// 编辑阈值（秒）：超过此阈值 → 标记「用户编辑中」
pub const EDITING_THRESHOLD_SECS: u64 = 300; // 5 分钟

/// lsof 只读系统进程白名单（对齐 dart `_knownReadOnlyBundles`）
#[cfg(target_os = "macos")]
const READONLY_PROCESSES: &[&str] = &[
    "mds",
    "mdworker_shared",
    "mdimport",
    "mdflagworker",
    "QuickLookSatellite",
    "qlmanage",
    "corespotlightd",
    "secd",
    "bird",
    "CoreServicesUIAgent",
];

/// 稳定性检查结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StabilityResult {
    /// 文件稳定，可以传输
    Stable,
    /// 文件仍在变化中（不稳定，延迟到下一周期）
    Unstable,
    /// 有进程正在编辑（用户编辑中，标记暂停）
    Editing,
}

/// 稳定性检查器
pub struct StabilityChecker {
    /// 追踪持续编辑的文件（rel_path → 首次发现时间）
    tracking: std::collections::HashMap<String, i64>,
}

impl StabilityChecker {
    /// 创建一个无持续编辑记录的稳定性检查器。
    pub fn new() -> Self {
        Self {
            tracking: std::collections::HashMap::new(),
        }
    }

    /// 检查文件是否稳定（可传输）。
    /// 对齐 dart `_checkStability`。
    pub async fn check(&mut self, path: &Path) -> StabilityResult {
        // 1. mtime 年龄检查
        let mtime_age = match mtime_age_secs(path) {
            Ok(age) => age,
            Err(_) => return StabilityResult::Unstable,
        };
        if mtime_age < MIN_MTIME_AGE_SECS {
            return StabilityResult::Unstable;
        }

        // 2. 大小稳定性检查（3s 窗口）
        let size1 = file_size(path);
        sleep(Duration::from_secs(SIZE_STABLE_WINDOW_SECS)).await;
        let size2 = file_size(path);
        if size1 != size2 {
            // 对齐 dart：size 不稳定时也检查编辑阈值（>5min 升级为 Editing）
            let path_key = path.to_string_lossy().to_string();
            let now = chrono::Utc::now().timestamp_millis();
            let first_seen = self.tracking.entry(path_key).or_insert(now);
            let elapsed = (now - *first_seen) / 1000;
            if elapsed as u64 > EDITING_THRESHOLD_SECS {
                return StabilityResult::Editing;
            }
            return StabilityResult::Unstable;
        }

        // 3. lsof 检查
        if is_file_busy(path).await {
            // 双重检查（1s 后重查，消除 Spotlight/QuickLook 误报）
            sleep(Duration::from_secs(1)).await;
            if is_file_busy(path).await {
                // 检查是否已持续编辑超过 5min
                let path_key = path.to_string_lossy().to_string();
                let now = chrono::Utc::now().timestamp_millis();
                let first_seen = self.tracking.entry(path_key).or_insert(now);
                let elapsed = (now - *first_seen) / 1000;
                if elapsed as u64 > EDITING_THRESHOLD_SECS {
                    return StabilityResult::Editing;
                }
                return StabilityResult::Unstable;
            }
        }

        // 之前可能在 tracking 中（现在已稳定，移除追踪）
        let path_key = path.to_string_lossy().to_string();
        self.tracking.remove(&path_key);
        StabilityResult::Stable
    }

    /// 清除某路径的追踪状态（文件已被删除/不再同步时调用）。
    pub fn clear_tracking(&mut self, path: &str) {
        self.tracking.remove(path);
    }
}

impl Default for StabilityChecker {
    /// 默认创建空稳定性检查器。
    fn default() -> Self {
        Self::new()
    }
}

/// 文件 mtime 距今时间（秒）。
fn mtime_age_secs(path: &Path) -> std::io::Result<u64> {
    let meta = std::fs::metadata(path)?;
    let modified = meta
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let age = now.as_secs().saturating_sub(modified.as_secs());
    Ok(age)
}

/// 文件大小（字节）。
fn file_size(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

/// lsof 检查：是否有进程以写模式打开文件。
/// 对齐 dart `_isFileBusy`（双重检查 + 白名单在 check 方法中处理）。
#[cfg(target_os = "macos")]
async fn is_file_busy(path: &Path) -> bool {
    use tokio::process::Command;
    let path_str = path.to_string_lossy().to_string();
    let output = match Command::new("lsof")
        .args(["-nP", "-F", "pc", &path_str])
        .output()
        .await
    {
        Ok(o) => o,
        Err(_) => return false,
    };
    if !output.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);

    // 解析 lsof -F pc 输出：p 行 = pid, c 行 = command
    let commands: Vec<&str> = stdout
        .lines()
        .filter(|l| l.starts_with('c'))
        .map(|l| &l[1..])
        .collect();

    // 若只有白名单内的只读进程 → 不判定为 busy
    if !commands.is_empty() && commands.iter().all(|cmd| READONLY_PROCESSES.contains(cmd)) {
        return false;
    }
    !commands.is_empty()
}

#[cfg(not(target_os = "macos"))]
/// 非 macOS 平台不执行 `lsof` 占用检测。
async fn is_file_busy(_path: &Path) -> bool {
    false
}

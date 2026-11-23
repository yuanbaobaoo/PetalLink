//! 日志系统：tracing + 环形缓冲（供设置页日志查看用）。
//!
//! 对齐 `legacy/lib/core/logging/logger.dart` + `log_buffer.dart`。
//!
//! 默认 INFO 等级（v1.5：debug 模式也用 INFO，避免 17K 文件全量 BFS 产生数万条
//! FINE 级 HTTP 日志）。排查 HTTP 时在设置页临时调到 TRACE。

use std::path::PathBuf;

use parking_lot::Mutex;
use serde::Serialize;
use specta::Type;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use crate::core::config_store;
use crate::error::AppResult;

/// 环形缓冲最大条数（对齐 dart `_maxBufferSize = 1000`）
const MAX_BUFFER_SIZE: usize = 1000;

/// 日志级别（前端展示用，对齐 dart logging 的 Level）
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Type)]
#[serde(rename_all = "UPPERCASE")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl From<Level> for LogLevel {
    /// 将 tracing 级别转换为前端稳定枚举。
    fn from(level: Level) -> Self {
        match level {
            Level::ERROR => LogLevel::Error,
            Level::WARN => LogLevel::Warn,
            Level::INFO => LogLevel::Info,
            Level::DEBUG => LogLevel::Debug,
            Level::TRACE => LogLevel::Trace,
        }
    }
}

/// 单条日志记录（对齐 dart `LogRecord`）
#[derive(Debug, Clone, Serialize, Type)]
pub struct LogRecord {
    /// 级别
    pub level: LogLevel,
    /// logger 名称（模块名）
    pub logger_name: String,
    /// 消息内容
    pub message: String,
    /// 时间戳（毫秒，epoch）
    pub time_ms: i64,
}

/// 全局日志环形缓冲（newest-first，对齐 dart `logBuffer.insert(0)`）。
static LOG_BUFFER: Mutex<Vec<LogRecord>> = Mutex::new(Vec::new());

/// 返回缓冲中所有日志的快照（newest-first）。
/// 对齐 dart `logBuffer`（直接读取全局 List）。
pub fn snapshot() -> Vec<LogRecord> {
    snapshot_since(None)
}

/// 返回缓冲中不早于 `since_ms` 的日志快照（newest-first）。
///
/// 日志页按时间范围筛选时只取窗口内记录，避免每 2 秒把整段缓冲全量传给前端。
pub fn snapshot_since(since_ms: Option<i64>) -> Vec<LogRecord> {
    let buffer = LOG_BUFFER.lock();
    filter_since(&buffer, since_ms)
}

/// 在给定缓冲上按时间下界截断（纯函数，便于单测）。
fn filter_since(records: &[LogRecord], since_ms: Option<i64>) -> Vec<LogRecord> {
    match since_ms {
        Some(since) => records
            .iter()
            .filter(|record| record.time_ms >= since)
            .cloned()
            .collect(),
        None => records.to_vec(),
    }
}

/// 追加一条日志到缓冲（newest-first，溢出裁剪尾部）。
/// 由 tracing layer 调用，业务代码一般不直接用。
pub fn push(record: LogRecord) {
    let mut buf = LOG_BUFFER.lock();
    push_into(&mut buf, record);
}

/// 在给定缓冲上执行 newest-first 插入 + 溢出裁剪（纯函数，便于单测隔离）。
fn push_into(buf: &mut Vec<LogRecord>, record: LogRecord) {
    buf.insert(0, record);
    // 溢出裁剪：保留 newest 的 MAX 条
    if buf.len() > MAX_BUFFER_SIZE {
        buf.truncate(MAX_BUFFER_SIZE);
    }
}

/// 清空缓冲（日志查看页「清空」按钮）。
pub fn clear() {
    LOG_BUFFER.lock().clear();
}

// ===== tracing Layer：喂环形缓冲（供设置页日志查看）=====

/// tracing Layer：把每条事件转为 [`LogRecord`] 推入 [`LOG_BUFFER`]。
///
/// 之前 `init_logger` 只装了 fmt 层（写 stdout），缓冲恒空 → 日志查看页「暂无日志」。
/// 此 Layer 在 `init_logger` 与 fmt/file 层并列挂载，让缓冲与终端/文件同步填充。
pub struct LogBufferLayer;

impl<S: Subscriber> Layer<S> for LogBufferLayer {
    /// 收集事件字段并将其写入内存环形缓冲。
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut collector = FieldCollector::default();
        event.record(&mut collector);
        // message 字段 + 其余结构化字段拼成一行（对齐 fmt 层的可读性）
        let message = match (collector.message, collector.fields.is_empty()) {
            (Some(msg), true) => msg,
            (Some(msg), false) => format!("{msg} {}", collector.fields.join(" ")),
            (None, false) => collector.fields.join(" "),
            (None, true) => String::new(),
        };
        push(LogRecord {
            level: (*event.metadata().level()).into(),
            logger_name: event.metadata().target().to_string(),
            message,
            time_ms: chrono::Utc::now().timestamp_millis(),
        });
    }
}

/// 事件字段收集器：抽出 message 字段 + 其余字段为 `key=value` 串。
#[derive(Default)]
struct FieldCollector {
    message: Option<String>,
    fields: Vec<String>,
}

impl FieldCollector {
    /// 记录一个字段值：message 字段单独存，其余按 `key=value` 累积。
    fn capture(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.push(format!("{}={}", field.name(), value));
        }
    }
}

impl Visit for FieldCollector {
    /// 记录字符串字段。
    fn record_str(&mut self, field: &Field, value: &str) {
        self.capture(field, value);
    }
    /// 记录有符号整数字段。
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.capture(field, &value.to_string());
    }
    /// 记录无符号整数字段。
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.capture(field, &value.to_string());
    }
    /// 记录浮点数字段。
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.capture(field, &value.to_string());
    }
    /// 记录布尔字段。
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.capture(field, &value.to_string());
    }
    /// 记录错误字段的展示文本。
    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.capture(field, &value.to_string());
    }
    /// 必需方法（无默认实现）：其余类型最终经此 fallback。
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.capture(field, &format!("{value:?}"));
    }
}

// ===== 日志文件目录 =====

/// 日志文件目录：`<support_dir>/logs`（与 DB/config 同目录，不污染同步目录）。
/// 供 `tracing_appender` 滚动文件 sink 与 `logs_export` 命令共用。
pub fn log_dir() -> AppResult<PathBuf> {
    Ok(config_store::support_dir()?.join("logs"))
}

/// 清理超期日志文件（保留最近 MAX_LOG_DAYS 天），在启动 init_logger 时调用。
/// tracing-appender 0.2.x 无 with_max_log_files，需手动清理防止无限累积。
pub fn cleanup_old_logs() {
    /// 日志文件最大保留天数。
    const MAX_LOG_DAYS: i64 = 30;
    let dir = match log_dir() {
        Ok(d) => d,
        Err(_) => return,
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let now = chrono::Utc::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().map(|n| n.to_string_lossy().to_string()) {
            Some(n) => n,
            None => continue,
        };
        // 只处理 PetalLink.log.YYYY-MM-DD 格式
        if !name.starts_with("PetalLink.log.") {
            continue;
        }
        // 从文件名解析日期
        let date_str = name.trim_start_matches("PetalLink.log.");
        if let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
            let file_age = now.date_naive().signed_duration_since(date).num_days();
            if file_age > MAX_LOG_DAYS && std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
    }
    if removed > 0 {
        tracing::info!(
            removed = removed,
            max_days = MAX_LOG_DAYS,
            "清理超期日志文件"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造指定时间戳的测试记录。
    fn record(time_ms: i64) -> LogRecord {
        LogRecord {
            level: LogLevel::Info,
            logger_name: "test".to_string(),
            message: format!("t{time_ms}"),
            time_ms,
        }
    }

    /// 时间下界只保留窗口内记录，且保持 newest-first 顺序。
    #[test]
    fn filter_since_keeps_window_in_order() {
        let buffer = vec![record(300), record(200), record(100)];

        let kept = filter_since(&buffer, Some(150));
        assert_eq!(
            kept.iter().map(|record| record.time_ms).collect::<Vec<_>>(),
            vec![300, 200]
        );
        // 不传下界等价于全量。
        assert_eq!(filter_since(&buffer, None).len(), 3);
        // 下界晚于全部记录时返回空集，而不是回退成全量。
        assert!(filter_since(&buffer, Some(400)).is_empty());
    }

    /// 环形缓冲保持 newest-first，且溢出时裁剪最旧记录。
    #[test]
    fn buffer_is_newest_first_and_trimmed() {
        let mut buffer = Vec::new();
        push_into(&mut buffer, record(1));
        push_into(&mut buffer, record(2));
        assert_eq!(buffer[0].time_ms, 2);
        for time in 3..=(MAX_BUFFER_SIZE as i64 + 2) {
            push_into(&mut buffer, record(time));
        }
        assert_eq!(buffer.len(), MAX_BUFFER_SIZE);
        assert_eq!(buffer[0].time_ms, MAX_BUFFER_SIZE as i64 + 2);
        assert_eq!(buffer[MAX_BUFFER_SIZE - 1].time_ms, 3);
    }
}

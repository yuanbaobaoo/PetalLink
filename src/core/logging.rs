//! 日志系统：tracing + 环形缓冲（供设置页日志查看用）。
//!
//! 对齐 `legacy/lib/core/logging/logger.dart` + `log_buffer.dart`。
//!
//! 默认 INFO 等级（v1.5：debug 模式也用 INFO，避免 17K 文件全量 BFS 产生数万条
//! FINE 级 HTTP 日志）。排查 HTTP 时在设置页临时调到 TRACE。

use std::path::PathBuf;

use parking_lot::Mutex;
use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use crate::core::config_store;
use crate::error::AppResult;

/// 环形缓冲最大条数（对齐 dart `_maxBufferSize = 1000`）
const MAX_BUFFER_SIZE: usize = 1000;

/// 日志级别（前端展示用，对齐 dart logging 的 Level）
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl From<Level> for LogLevel {
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
#[derive(Debug, Clone, Serialize)]
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

/// 当前日志等级（可运行时调整，对齐 dart `currentLevel`）
#[allow(dead_code)]
static CURRENT_LEVEL: Mutex<Level> = Mutex::new(Level::INFO);

/// 返回缓冲中所有日志的快照（newest-first）。
/// 对齐 dart `logBuffer`（直接读取全局 List）。
pub fn snapshot() -> Vec<LogRecord> {
    LOG_BUFFER.lock().clone()
}

/// 按 level 过滤后的快照（前端日志查看页筛选用）。
#[allow(dead_code)]
pub fn snapshot_filtered(level: Option<LogLevel>) -> Vec<LogRecord> {
    let buf = LOG_BUFFER.lock();
    match level {
        None => buf.clone(),
        Some(filter) => buf.iter().filter(|r| r.level == filter).cloned().collect(),
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

/// 获取当前生效的日志等级。
#[allow(dead_code)]
pub fn current_level() -> Level {
    *CURRENT_LEVEL.lock()
}

/// 运行时调整日志等级（设置页临时调到 TRACE 排查 HTTP）。
#[allow(dead_code)]
pub fn update_level(level: Level) {
    *CURRENT_LEVEL.lock() = level;
    tracing::info!(level = %level, "日志等级已调整");
}

/// 记录一条业务日志（不依赖 tracing 的 macro，直接入缓冲 + 输出）。
/// 供无法接入 tracing span 的简单场景使用。
#[allow(dead_code)]
pub fn log(level: LogLevel, logger_name: &str, message: &str) {
    let record = LogRecord {
        level,
        logger_name: logger_name.to_string(),
        message: message.to_string(),
        time_ms: chrono::Utc::now().timestamp_millis(),
    };
    // 输出到 tracing（实际终端/文件输出）
    match level {
        LogLevel::Error => tracing::error!(target = logger_name, "{}", message),
        LogLevel::Warn => tracing::warn!(target = logger_name, "{}", message),
        LogLevel::Info => tracing::info!(target = logger_name, "{}", message),
        LogLevel::Debug => tracing::debug!(target = logger_name, "{}", message),
        LogLevel::Trace => tracing::trace!(target = logger_name, "{}", message),
    }
    push(record);
}

// ===== tracing Layer：喂环形缓冲（供设置页日志查看）=====

/// tracing Layer：把每条事件转为 [`LogRecord`] 推入 [`LOG_BUFFER`]。
///
/// 之前 `init_logger` 只装了 fmt 层（写 stdout），缓冲恒空 → 日志查看页「暂无日志」。
/// 此 Layer 在 `init_logger` 与 fmt/file 层并列挂载，让缓冲与终端/文件同步填充。
pub struct LogBufferLayer;

impl<S: Subscriber> Layer<S> for LogBufferLayer {
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
    fn record_str(&mut self, field: &Field, value: &str) {
        self.capture(field, value);
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.capture(field, &value.to_string());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.capture(field, &value.to_string());
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.capture(field, &value.to_string());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.capture(field, &value.to_string());
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一条测试日志
    fn rec(msg: &str, time_ms: i64) -> LogRecord {
        LogRecord {
            level: LogLevel::Info,
            logger_name: "test".into(),
            message: msg.into(),
            time_ms,
        }
    }

    #[test]
    fn test_push_into_newest_first() {
        // 隔离测试纯函数，不依赖全局缓冲（避免与其他用例竞态）
        let mut buf = Vec::new();
        push_into(&mut buf, rec("first", 1000));
        push_into(&mut buf, rec("second", 2000));
        assert_eq!(buf.len(), 2);
        assert_eq!(buf[0].message, "second"); // newest first
        assert_eq!(buf[1].message, "first");
    }

    #[test]
    fn test_push_into_caps_at_max() {
        let mut buf = Vec::new();
        for i in 0..(MAX_BUFFER_SIZE + 50) {
            push_into(&mut buf, rec(&format!("msg-{i}"), i as i64));
        }
        assert_eq!(buf.len(), MAX_BUFFER_SIZE);
        // 最新的（i 最大）应在最前
        assert_eq!(buf[0].message, format!("msg-{}", MAX_BUFFER_SIZE + 49));
    }

    #[test]
    fn test_filter_by_level() {
        // filter 是纯读取函数，无需全局状态
        let mut buf = vec![rec("err", 1), rec("info", 2)];
        buf[0].level = LogLevel::Error;
        let errs: Vec<LogRecord> = buf
            .iter()
            .filter(|r| r.level == LogLevel::Error)
            .cloned()
            .collect();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].message, "err");
    }
}

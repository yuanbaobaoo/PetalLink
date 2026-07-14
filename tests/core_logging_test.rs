//! 核心日志记录构造与级别过滤测试。

use petal_link_lib::core::logging::{clear, push, snapshot_filtered, LogLevel, LogRecord};

/// 构造带固定时间的测试日志记录。
fn rec(msg: &str, time_ms: i64) -> LogRecord {
    LogRecord {
        level: LogLevel::Info,
        logger_name: "test".into(),
        message: msg.into(),
        time_ms,
    }
}

/// 验证日志快照按级别过滤。
#[test]
fn test_filter_by_level() {
    clear();
    let mut error = rec("err", 1);
    error.level = LogLevel::Error;
    push(rec("info", 2));
    push(error);

    let errs = snapshot_filtered(Some(LogLevel::Error));
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].message, "err");
    clear();
}

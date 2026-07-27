//! 核心日志记录与快照顺序测试。

use petal_link_lib::core::logging::{clear, push, snapshot, LogLevel, LogRecord};

/// 构造带固定时间的测试日志记录。
fn rec(msg: &str, time_ms: i64) -> LogRecord {
    LogRecord {
        level: LogLevel::Info,
        logger_name: "test".into(),
        message: msg.into(),
        time_ms,
    }
}

/// 验证日志快照保持 newest-first 顺序。
#[test]
fn test_snapshot_order() {
    clear();
    push(rec("older", 1));
    push(rec("newer", 2));

    let records = snapshot();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].message, "newer");
    assert_eq!(records[1].message, "older");
    clear();
}

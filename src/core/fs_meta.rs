//! 文件系统元数据读取的小工具。

/// 将文件元数据的修改时间转为 epoch 毫秒；读取失败或超出表示范围时返回 None。
pub fn metadata_mtime_ms(metadata: &std::fs::Metadata) -> Option<i64> {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
}

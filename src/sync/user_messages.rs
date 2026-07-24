//! 将同步内部诊断转换为用户可执行的提示，同时允许日志保留技术细节。

use std::borrow::Cow;

/// 替换用户不需要理解的内部同步术语。
///
/// 未命中规则的消息保持原样，避免掩盖已有的清晰错误。
pub(crate) fn simplify_sync_error(message: &str) -> Cow<'_, str> {
    if message.contains("远端文件已在规划后变化") || message.contains("云端文件版本已变化")
    {
        return Cow::Borrowed("云端文件已更新。为避免覆盖，请同步索引后重试。");
    }
    if message.contains("用户正在编辑") || message.contains("文件正在编辑") {
        return Cow::Borrowed("文件正在编辑，保存并关闭后会自动继续。");
    }
    if message.contains("文件尚不稳定") || message.contains("文件仍在变化") {
        return Cow::Borrowed("文件仍在变化，稳定后会自动继续。");
    }
    if message.contains("本地上传源已变化")
        || message.contains("本地上传源在执行前发生变化")
        || message.contains("本地源已变化")
        || message.contains("下载目标已出现本地内容")
        || message.contains("更新下载目标已变化")
        || message.contains("更新下载目标已不存在")
    {
        return Cow::Borrowed("本地文件已发生变化，请重新检查并重试。");
    }
    if message.contains("缺少 fileId")
        || message.contains("缺少真实 fileId")
        || message.contains("缺少 parentId")
        || message.contains("缺少 operation")
        || message.contains("operation 与 direction 不一致")
        || message.contains("缺少云端版本")
        || message.contains("缺少云端版本快照")
    {
        return Cow::Borrowed("文件同步信息不完整，请同步索引后重试。");
    }
    if message.contains("session_url")
        || message.contains("上传断点")
        || message.contains("安全重放")
    {
        return Cow::Borrowed("续传信息已失效，请重新开始上传。");
    }
    if message.contains("找不到与路径匹配的成功同步基线") {
        return Cow::Borrowed("没有找到可用于核对的同步记录，暂时无法释放空间。");
    }
    if message.contains("本地内容与最后成功同步基线不一致") {
        return Cow::Borrowed("本地文件已更改，无法释放空间。");
    }
    if message.contains("可信云树中不存在同一 fileId") {
        return Cow::Borrowed("云端文件信息已变化，请同步索引后重试。");
    }
    if message.contains("远端副本不存在、已回收、大小或版本与成功基线不一致")
    {
        return Cow::Borrowed("云端文件已变化，无法释放空间。");
    }
    if message.contains("远端核验期间本地文件已变化") {
        return Cow::Borrowed("检查期间本地文件发生变化，无法释放空间。");
    }
    if message.contains("云端索引尚未追平") {
        return Cow::Borrowed("云端文件仍在更新，请稍后再试。");
    }
    if message.contains("释放租约已失效") {
        return Cow::Borrowed("文件状态已变化，请同步索引后重试。");
    }
    if message.contains("重新规划") {
        return Cow::Borrowed("文件状态已变化，请重新检查并重试。");
    }
    if message.contains("远端核验") {
        return Cow::Borrowed("正在确认同步结果，请稍后查看。");
    }
    Cow::Borrowed(message)
}

/// 验证内部术语转换与普通错误保留合同。
#[cfg(test)]
mod tests {
    use super::simplify_sync_error;

    /// 历史重新规划错误必须转换为用户可执行的建议。
    #[test]
    fn simplifies_replan_terms() {
        assert_eq!(
            simplify_sync_error("远端文件已在规划后变化，拒绝用旧任务覆盖"),
            "云端文件已更新。为避免覆盖，请同步索引后重试。"
        );
        assert_eq!(
            simplify_sync_error("本地上传源已变化，需要重新规划"),
            "本地文件已发生变化，请重新检查并重试。"
        );
    }

    /// 已经清晰的普通错误不得被泛化覆盖。
    #[test]
    fn preserves_plain_errors() {
        assert_eq!(simplify_sync_error("网络连接失败"), "网络连接失败");
    }
}

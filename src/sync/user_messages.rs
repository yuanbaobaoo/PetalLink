//! 将同步内部诊断转换为用户可执行的提示，同时向前端导出同源规则。

use std::borrow::Cow;

use serde::Serialize;
use specta::Type;

/// 一条内部诊断匹配规则。
struct UserMessageRule {
    patterns: &'static [&'static str],
    message: &'static str,
}

/// 同步诊断到用户提示的唯一规则表。
const USER_MESSAGE_RULES: &[UserMessageRule] = &[
    UserMessageRule {
        patterns: &["远端文件已在规划后变化", "云端文件版本已变化"],
        message: "云端文件已更新。为避免覆盖，请同步索引后重试。",
    },
    UserMessageRule {
        patterns: &["用户正在编辑", "文件正在编辑"],
        message: "文件正在编辑，保存并关闭后会自动继续。",
    },
    UserMessageRule {
        patterns: &["文件尚不稳定", "文件仍在变化"],
        message: "文件仍在变化，稳定后会自动继续。",
    },
    UserMessageRule {
        patterns: &[
            "本地上传源已变化",
            "本地上传源在执行前发生变化",
            "本地源已变化",
            "下载目标已出现本地内容",
            "更新下载目标已变化",
            "更新下载目标已不存在",
        ],
        message: "本地文件已发生变化，请重新检查并重试。",
    },
    UserMessageRule {
        patterns: &[
            "缺少 fileId",
            "缺少真实 fileId",
            "缺少 parentId",
            "缺少 operation",
            "operation 与 direction 不一致",
            "缺少云端版本",
            "缺少云端版本快照",
        ],
        message: "文件同步信息不完整，请同步索引后重试。",
    },
    UserMessageRule {
        patterns: &["session_url", "上传断点", "安全重放"],
        message: "续传信息已失效，请重新开始上传。",
    },
    UserMessageRule {
        patterns: &["找不到与路径匹配的成功同步基线"],
        message: "没有找到可用于核对的同步记录，暂时无法释放空间。",
    },
    UserMessageRule {
        patterns: &["本地内容与最后成功同步基线不一致"],
        message: "本地文件已更改，无法释放空间。",
    },
    UserMessageRule {
        patterns: &["可信云树中不存在同一 fileId"],
        message: "云端文件信息已变化，请同步索引后重试。",
    },
    UserMessageRule {
        patterns: &["远端副本不存在、已回收、大小或版本与成功基线不一致"],
        message: "云端文件已变化，无法释放空间。",
    },
    UserMessageRule {
        patterns: &["远端核验期间本地文件已变化"],
        message: "检查期间本地文件发生变化，无法释放空间。",
    },
    UserMessageRule {
        patterns: &["云端索引尚未追平"],
        message: "云端文件仍在更新，请稍后再试。",
    },
    UserMessageRule {
        patterns: &["释放租约已失效"],
        message: "文件状态已变化，请同步索引后重试。",
    },
    UserMessageRule {
        patterns: &["重新规划"],
        message: "文件状态已变化，请重新检查并重试。",
    },
    UserMessageRule {
        patterns: &["远端核验"],
        message: "正在确认同步结果，请稍后查看。",
    },
];

/// 可生成到前端 bindings 的用户提示规则。
#[derive(Serialize, Type)]
pub(crate) struct IpcUserMessageRule {
    /// 任一命中即可应用规则的内部诊断片段。
    patterns: Vec<&'static str>,
    /// 面向用户的稳定提示。
    message: &'static str,
}

/// 返回供前端兼容历史持久化错误使用的同源规则。
pub(crate) fn ipc_user_message_rules() -> Vec<IpcUserMessageRule> {
    USER_MESSAGE_RULES
        .iter()
        .map(|rule| IpcUserMessageRule {
            patterns: rule.patterns.to_vec(),
            message: rule.message,
        })
        .collect()
}

/// 替换用户不需要理解的内部同步术语。
///
/// 未命中规则的消息保持原样，避免掩盖已有的清晰错误。
pub(crate) fn simplify_sync_error(message: &str) -> Cow<'_, str> {
    USER_MESSAGE_RULES
        .iter()
        .find(|rule| {
            rule.patterns
                .iter()
                .any(|pattern| message.contains(pattern))
        })
        .map(|rule| Cow::Borrowed(rule.message))
        .unwrap_or_else(|| Cow::Borrowed(message))
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

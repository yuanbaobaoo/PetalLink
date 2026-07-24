/// 将同步内部诊断转换为用户可执行的提示，同时允许日志保留技术细节。
///
/// 对齐 Rust `src/sync/user_messages.rs` 的 `simplify_sync_error`。
///
/// 未命中规则的消息保持原样返回，避免掩盖已清晰的错误。
/// 调用方应同时记录技术原文（用于日志）与转换结果（用于落库/UI 展示）。
library;

/// 替换用户不需要理解的内部同步术语。
///
/// [message] 同步流程内部产生的技术错误原文。
/// 返回用户可执行的可读提示；未命中规则时原样返回 [message]。
String simplifySyncError(String message) {
  // 云端文件并发修改：规划后/执行前云端版本已变化
  if (message.contains('远端文件已在规划后变化') ||
      message.contains('云端文件版本已变化')) {
    return '云端文件已更新。为避免覆盖，请同步索引后重试。';
  }
  // 文件被占用（如编辑器持有）
  if (message.contains('用户正在编辑') || message.contains('文件正在编辑')) {
    return '文件正在编辑，保存并关闭后会自动继续。';
  }
  // 文件仍在变化（三段式稳定性未通过）
  if (message.contains('文件尚不稳定') || message.contains('文件仍在变化')) {
    return '文件仍在变化，稳定后会自动继续。';
  }
  // 本地源/下载目标在执行前后发生变化
  if (message.contains('本地上传源已变化') ||
      message.contains('本地上传源在执行前发生变化') ||
      message.contains('本地源已变化') ||
      message.contains('下载目标已出现本地内容') ||
      message.contains('更新下载目标已变化') ||
      message.contains('更新下载目标已不存在')) {
    return '本地文件已发生变化，请重新检查并重试。';
  }
  // 任务字段不完整（fileId/parentId/operation/云端版本等缺失）
  if (message.contains('缺少 fileId') ||
      message.contains('缺少真实 fileId') ||
      message.contains('缺少 parentId') ||
      message.contains('缺少 operation') ||
      message.contains('operation 与 direction 不一致') ||
      message.contains('缺少云端版本') ||
      message.contains('缺少云端版本快照')) {
    return '文件同步信息不完整，请同步索引后重试。';
  }
  // 续传信息失效（session_url 过期、断点不可用）
  if (message.contains('session_url') ||
      message.contains('上传断点') ||
      message.contains('安全重放')) {
    return '续传信息已失效，请重新开始上传。';
  }
  // 释放空间：无可核对基线
  if (message.contains('找不到与路径匹配的成功同步基线')) {
    return '没有找到可用于核对的同步记录，暂时无法释放空间。';
  }
  // 释放空间：本地已改
  if (message.contains('本地内容与最后成功同步基线不一致')) {
    return '本地文件已更改，无法释放空间。';
  }
  // 释放空间：可信云树无对应 fileId
  if (message.contains('可信云树中不存在同一 fileId')) {
    return '云端文件信息已变化，请同步索引后重试。';
  }
  // 释放空间：远端副本状态与基线不一致
  if (message.contains('远端副本不存在、已回收、大小或版本与成功基线不一致')) {
    return '云端文件已变化，无法释放空间。';
  }
  // 释放空间：核验期间本地又变了
  if (message.contains('远端核验期间本地文件已变化')) {
    return '检查期间本地文件发生变化，无法释放空间。';
  }
  // 云端索引尚未追平（增量同步落后）
  if (message.contains('云端索引尚未追平')) {
    return '云端文件仍在更新，请稍后再试。';
  }
  // 释放空间租约失效
  if (message.contains('释放租约已失效')) {
    return '文件状态已变化，请同步索引后重试。';
  }
  // 通用重新规划（如上述未覆盖的 replan 分支）
  if (message.contains('重新规划')) {
    return '文件状态已变化，请重新检查并重试。';
  }
  // 远端核验进行中（非失败，提示稍后查看）
  if (message.contains('远端核验')) {
    return '正在确认同步结果，请稍后查看。';
  }
  return message;
}

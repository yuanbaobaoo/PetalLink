/**
 * 错误处理工具 —— 统一从 unknown 类型提取可读错误信息。
 *
 * 替代散布在各视图/组件中的 `(e as { message?: string }).message ?? String(e)` 模式。
 */

/**
 * 将内部同步错误转换为用户能理解的提示。
 *
 * 已保存的历史任务仍可能包含旧术语，因此转换必须在展示层保留。
 *
 * @param message - 后端返回或数据库保存的原始错误
 * @returns 用户侧提示
 */
export function formatUserMessage(message: string): string {
  if (
    message.includes("远端文件已在规划后变化")
    || message.includes("云端文件版本已变化")
  ) {
    return "云端文件已更新。为避免覆盖，请同步索引后重试。";
  }
  if (
    message.includes("用户正在编辑")
    || message.includes("文件正在编辑")
  ) {
    return "文件正在编辑，保存并关闭后会自动继续。";
  }
  if (
    message.includes("文件尚不稳定")
    || message.includes("文件仍在变化")
  ) {
    return "文件仍在变化，稳定后会自动继续。";
  }
  if (
    message.includes("本地上传源已变化")
    || message.includes("本地上传源在执行前发生变化")
    || message.includes("本地源已变化")
    || message.includes("下载目标已出现本地内容")
    || message.includes("更新下载目标已变化")
    || message.includes("更新下载目标已不存在")
  ) {
    return "本地文件已发生变化，请重新检查并重试。";
  }
  if (
    message.includes("缺少 fileId")
    || message.includes("缺少真实 fileId")
    || message.includes("缺少 parentId")
    || message.includes("缺少 operation")
    || message.includes("operation 与 direction 不一致")
    || message.includes("缺少云端版本")
    || message.includes("缺少云端版本快照")
  ) {
    return "文件同步信息不完整，请同步索引后重试。";
  }
  if (
    message.includes("session_url")
    || message.includes("上传断点")
    || message.includes("安全重放")
  ) {
    return "续传信息已失效，请重新开始上传。";
  }
  if (message.includes("找不到与路径匹配的成功同步基线")) {
    return "没有找到可用于核对的同步记录，暂时无法释放空间。";
  }
  if (message.includes("本地内容与最后成功同步基线不一致")) {
    return "本地文件已更改，无法释放空间。";
  }
  if (message.includes("可信云树中不存在同一 fileId")) {
    return "云端文件信息已变化，请同步索引后重试。";
  }
  if (message.includes("远端副本不存在、已回收、大小或版本与成功基线不一致")) {
    return "云端文件已变化，无法释放空间。";
  }
  if (message.includes("远端核验期间本地文件已变化")) {
    return "检查期间本地文件发生变化，无法释放空间。";
  }
  if (message.includes("云端索引尚未追平")) {
    return "云端文件仍在更新，请稍后再试。";
  }
  if (message.includes("释放租约已失效")) {
    return "文件状态已变化，请同步索引后重试。";
  }
  if (message.includes("WaitingForNetwork")) {
    return "网络不可用，恢复后会自动继续。";
  }
  if (message.includes("BackingOff")) {
    return "服务暂时不可用，稍后会自动重试。";
  }
  if (message.includes("VerifyingRemote")) {
    return "正在确认上次同步是否成功。";
  }
  if (message.includes("RestartRequired")) {
    return "文件状态已变化，请重新检查并重试。";
  }
  if (message.includes("BlockedByActiveIntent")) {
    return "该文件正在执行其他同步任务，请稍后再试。";
  }
  if (message.includes("重新规划")) {
    return "文件状态已变化，请重新检查并重试。";
  }
  if (message.includes("远端核验")) {
    return "正在确认同步结果，请稍后查看。";
  }
  return message;
}

/**
 * 从 unknown 错误对象中提取人类可读的错误消息。
 *
 * 优先取 `.message`（后端 AppError / JS Error），否则回退到 String(e)，
 * 最后统一替换用户不需要理解的内部术语。
 *
 * @param e - 捕获到的错误（类型未知）
 * @returns 错误消息字符串
 */
export function extractErrorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) {
    // 后端错误消息
    const msg = (e as { message?: unknown }).message;
    if (typeof msg === "string" && msg) return formatUserMessage(msg);
  }
  if (typeof e === "string") return formatUserMessage(e);
  return formatUserMessage(String(e));
}

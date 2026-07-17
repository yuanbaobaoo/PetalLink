package io.github.yuanbaobaoo.petallink

/**
 * 应用统一错误模型（对标原项目 src/error.rs）
 *
 * 所有业务逻辑返回 [Result] 失败时统一携带 [AppError]，避免各层自行定义异常类型。
 * 序列化字段保持英文；展示给用户的文案在 UI 层按 [kind] 本地化。
 */
sealed class AppError(
    val kind: ErrorKind,
    message: String,
    cause: Throwable? = null,
) : Throwable(message, cause) {

    /** 错误大类，决定重试策略与 UI 提示文案 */
    enum class ErrorKind {
        NETWORK,        // 网络层（DNS/连接/超时/打断）→ 可重试
        AUTH,           // 401 / token 失效 → 触发刷新或要求重新登录
        REMOTE,         // 远端业务错误（非 2xx，含配额、文件不存在等）
        CONFLICT,       // 三方冲突 → 进入冲突解决流程
        DATA,           // 数据库 / 序列化 / 解析错误 → 不可重试
        LOCAL_IO,       // 本地文件系统错误（权限、磁盘满）
        CANCELED,       // 用户主动取消
        INTERNAL,       // 其他未分类（不应出现，出现即视为 bug）
    }

    /** 网络层错误（含超时） */
    class Network(message: String, cause: Throwable? = null) : AppError(ErrorKind.NETWORK, message, cause)

    /** 鉴权失败：token 过期 / 无效 / 刷新失败 */
    class Auth(message: String, cause: Throwable? = null) : AppError(ErrorKind.AUTH, message, cause)

    /** 远端业务错误（携带 HTTP 状态码） */
    class Remote(val status: Int, message: String, cause: Throwable? = null) :
        AppError(ErrorKind.REMOTE, message, cause)

    /** 写响应丢失或成功响应无法核验；上层只能进入 VerifyingRemote。 */
    class RemoteAmbiguous(message: String, cause: Throwable? = null) :
        AppError(ErrorKind.REMOTE, message, cause)

    /** Changes cursor 已失效；上层只能保留旧 checkpoint 并执行可信全量重建。 */
    class ChangesCursorInvalid(val status: Int, message: String) :
        AppError(ErrorKind.REMOTE, message)

    /** 同步冲突：本地与云端均发生修改 */
    class Conflict(message: String, cause: Throwable? = null) : AppError(ErrorKind.CONFLICT, message, cause)

    /** 数据层错误：DB / 序列化 / schema */
    class Data(message: String, cause: Throwable? = null) : AppError(ErrorKind.DATA, message, cause)

    /** 本地 IO 错误：权限 / 磁盘空间 / xattr 读写 */
    class LocalIo(message: String, cause: Throwable? = null) : AppError(ErrorKind.LOCAL_IO, message, cause)

    /** 用户主动取消 */
    class Canceled(message: String = "canceled") : AppError(ErrorKind.CANCELED, message)

    /** 内部错误（兜底，视为 bug） */
    class Internal(message: String, cause: Throwable? = null) : AppError(ErrorKind.INTERNAL, message, cause)
}

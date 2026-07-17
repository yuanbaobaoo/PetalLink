package io.github.yuanbaobaoo.petallink.sync

/**
 * 传输九态状态机（对标原项目 src/sync/transfer_state.rs）
 *
 * 每次状态变更必须通过 [canTransition] 校验，并通过 CAS（state_revision）
 * 乐观锁落库（docs/04 §6 / docs/06 §10）。详见 docs/06 §九态状态机。
 *
 * 状态语义：
 * - Pending            ：已入队待执行
 * - Running            ：正在传输（上传/下载）
 * - WaitingForNetwork  ：网络不可用，等待恢复
 * - BackingOff         ：传输失败，退避等待重试
 * - VerifyingRemote    ：传输完成后核验远端（数据安全规则，docs/06 §11）
 * - RestartRequired    ：可恢复中断（需用户/引擎确认后重启）
 * - Completed          ：完成（终态）
 * - Failed             ：失败（终态，超过 MAX_AUTOMATIC_ATTEMPTS）
 * - Canceled           ：用户取消（终态）
 */
enum class TransferState {
    Pending,
    Running,
    WaitingForNetwork,
    BackingOff,
    VerifyingRemote,
    RestartRequired,
    Completed,
    Failed,
    Canceled;

    companion object {
        // ------------------------------------------------------------------
        // 合法迁移矩阵（docs/06 §can_transition）
        // key = 源状态，value = 可达目标状态集合
        // ------------------------------------------------------------------
        private val TRANSITIONS: Map<TransferState, Set<TransferState>> = mapOf(
            Pending to setOf(Running, WaitingForNetwork, RestartRequired, Failed, Canceled),
            Running to setOf(
                VerifyingRemote,        // 正常完成 → 核验远端
                WaitingForNetwork,      // 网络中断
                BackingOff,             // 可重试错误
                RestartRequired,
                Completed,
                Failed,                 // 不可重试错误
                Canceled,               // 用户取消
            ),
            WaitingForNetwork to setOf(Running, RestartRequired, Failed, Canceled),
            BackingOff to setOf(Running, RestartRequired, Failed, Canceled),
            VerifyingRemote to setOf(
                Running, WaitingForNetwork, BackingOff, RestartRequired, Completed, Failed, Canceled,
            ),
            RestartRequired to setOf(Pending, VerifyingRemote, Failed, Canceled),
            Completed to emptySet(),
            Failed to setOf(Pending, RestartRequired, Canceled),
            Canceled to emptySet(),
        )

        /**
         * 校验状态迁移是否合法。非法迁移视为 bug，必须拒绝（防状态机错乱）。
         */
        fun canTransition(from: TransferState, to: TransferState): Boolean {
            if (from == to) return true // 幂等
            return TRANSITIONS[from]?.contains(to) == true
        }

        /**
         * 终态判定（不再变化）
         */
        fun isTerminal(state: TransferState): Boolean =
            state == Completed || state == Failed || state == Canceled

        /**
         * 活跃态判定（占用传输槽位）
         */
        fun isActive(state: TransferState): Boolean =
            state == Running || state == VerifyingRemote
    }
}

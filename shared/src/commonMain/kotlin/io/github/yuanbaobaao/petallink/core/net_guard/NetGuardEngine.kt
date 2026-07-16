package io.github.yuanbaobaao.petallink.core.net_guard

/**
 * 网络状态（对标 src/core/net_guard.rs）
 */
enum class NetState { ONLINE, OFFLINE }

/**
 * 网络探测纯逻辑（对标 src/core/net_guard.rs 防抖逻辑）。
 *
 * 纯状态机，无 IO，可单测。核心防抖规则：
 * - 失败立即转 OFFLINE
 * - **连续 2 次成功**才转 ONLINE（防抖，防短暂恢复误判）
 * - 代际（generation）机制：新探测周期开始时重置连续计数；
 *   旧代际的迟到回调被忽略（防止旧探测污染新状态）
 *
 * 详见 docs/06 §网络守卫。
 */
class NetGuardEngine {
    private var current: NetState = NetState.OFFLINE
    private var consecutiveSuccess: Int = 0
    private var currentGen: Int = -1

    /**
     * 处理一次探测结果。
     * @param success 探测是否成功（TCP 连通）
     * @param gen 探测代际（由 [newGeneration] 递增）；与当前代际不符则重置计数
     * @return 处理后的网络状态
     */
    fun onProbeResult(success: Boolean, gen: Int): NetState {
        // 代际切换：新代际重置连续成功计数
        if (gen != currentGen) {
            currentGen = gen
            consecutiveSuccess = 0
        }

        if (success) {
            consecutiveSuccess++
            // 连续 2 次成功才转 ONLINE
            if (consecutiveSuccess >= 2) {
                current = NetState.ONLINE
            }
        } else {
            // 失败立即清零并转 OFFLINE
            consecutiveSuccess = 0
            current = NetState.OFFLINE
        }
        return current
    }

    /** 当前网络状态 */
    fun state(): NetState = current
}

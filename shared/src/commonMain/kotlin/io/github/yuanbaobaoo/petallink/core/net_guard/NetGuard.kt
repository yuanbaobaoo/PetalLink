package io.github.yuanbaobaoo.petallink.core.net_guard

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.StateFlow

/**
 * 网络守卫接口（expect，macosMain 提供 actual）。
 *
 * 职责：周期性 TCP 探测华为域名 443，通过 [NetGuardEngine] 做防抖判定，
 * 暴露当前 [NetState]。
 *
 * 精确参数（docs/06 §网络守卫）：
 * - 探测目标：driveapis.cloud.huawei.com.cn:443
 * - 超时：3s
 * - 间隔：30s
 * - 连续 2 次成功才转 ONLINE
 */
expect class NetGuard(scope: CoroutineScope) {
    /** 当前网络状态 */
    val state: StateFlow<NetState>

    /** 启动周期性探测 */
    fun startProbe()

    /** 停止探测 */
    fun stopProbe()

    /**
     * 开启新的探测代际（用于网络恢复后强制重置防抖计数）。
     * @return 新的代际号
     */
    fun newGeneration(): Int

    /** 真实请求层报告网络失败；仅首次 Online→Offline 返回 true。 */
    fun reportRequestNetworkFailure(): Boolean
}

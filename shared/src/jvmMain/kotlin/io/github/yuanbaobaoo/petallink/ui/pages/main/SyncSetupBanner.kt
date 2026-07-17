@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateBannerVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButton
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButtonVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateHDivider
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateInfoBanner
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import io.github.yuanbaobaoo.petallink.ui.viewmodel.SetupPhase

/**
 * 首次同步引导条（对标原 Vue SyncSetupBanner.vue）。
 *
 * 三态互斥（error 优先 > setupPhase needsSetup > needsFirstSync）：
 * - error：MateInfoBanner error + 重试按钮
 * - needsSetup：MateInfoBanner info + 选择目录按钮
 * - needsFirstSync：MateInfoBanner warning + 同步索引按钮
 *
 * padding 4/16，底 0.5px border，白底。
 *
 * @param setupPhase 同步目录配置阶段
 * @param mountDir 当前挂载目录（needsFirstSync 文案展示）
 * @param errorMessage 错误消息（非空时优先 error 态）
 * @param onSelectDir 选择目录回调
 * @param onFirstSync 触发首次同步回调
 * @param onRetry 重试回调
 */
@Composable
fun SyncSetupBanner(
    setupPhase: SetupPhase,
    mountDir: String,
    errorMessage: String?,
    onSelectDir: () -> Unit,
    onFirstSync: () -> Unit,
    onRetry: () -> Unit,
) {
    val semantic = LocalSemanticColors.current
    when {
        errorMessage != null -> {
            BannerWrapper(semantic.bgContainer) {
                MateInfoBanner(
                    message = errorMessage,
                    variant = MateBannerVariant.ERROR,
                    action = { MateButton(label = "重试", variant = MateButtonVariant.TEXT, icon = "refresh", onClick = onRetry) },
                )
            }
        }
        setupPhase == SetupPhase.NEEDS_SETUP -> {
            BannerWrapper(semantic.bgContainer) {
                MateInfoBanner(
                    message = "尚未配置同步目录，选择一个空目录开始同步",
                    variant = MateBannerVariant.INFO,
                    action = { MateButton(label = "选择目录", variant = MateButtonVariant.TEXT, icon = "folder-open", onClick = onSelectDir) },
                )
            }
        }
        setupPhase == SetupPhase.NEEDS_FIRST_SYNC -> {
            BannerWrapper(semantic.bgContainer) {
                MateInfoBanner(
                    message = "同步目录已就绪：${mountDir.ifBlank { "未配置" }}，点击「同步索引」拉取云端索引",
                    variant = MateBannerVariant.WARNING,
                    action = { MateButton(label = "同步索引", variant = MateButtonVariant.TEXT, icon = "sync", onClick = onFirstSync) },
                )
            }
        }
        // ACTIVE / LOADING 不显示引导条
    }
}

/** 引导条容器：padding 4/16，白底，底 0.5px 分隔线（与原 .setup-banner 一致）。 */
@Composable
private fun BannerWrapper(bg: androidx.compose.ui.graphics.Color, content: @Composable () -> Unit) {
    androidx.compose.foundation.layout.Column(
        modifier = Modifier.fillMaxWidth(),
    ) {
        androidx.compose.foundation.layout.Column(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp).background(bg),
        ) { content() }
        // 底分隔线（对标 .setup-banner border-bottom: 0.5px）
        MateHDivider()
    }
}

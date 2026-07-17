@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.border
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.config.DEFAULT_CALLBACK_PORT
import io.github.yuanbaobaoo.petallink.config.UserConfig
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButton
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButtonVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateInfoBanner
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateBannerVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateNavItem
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateNumberField
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateSectionHeader
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateStepper
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateSwitch
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateTextField
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import io.github.yuanbaobaoo.petallink.update.UpdateManifest

private enum class SettingsTab(val label: String, val icon: String) {
    SYNC_DIR("同步目录", "folder"),
    TRANSFER("传输设置", "transfer"),
    ADVANCED("高级设置", "settings"),
    ACCOUNT("账号管理", "info"),
    LOGS("日志查看", "list"),
    ABOUT("关于", "cloud"),
}

/**
 * 设置页（对标原 Vue SettingsPage.vue）。
 *
 * 双栏：左导航 200px（6 个 MateNavItem）+ 右设置区(scroll)；
 * footer(56px)：保存/重置 + 自动保存状态（仅 syncDir/transfer/advanced 显示）。
 */
@Composable
fun SettingsScreen(
    initialConfig: UserConfig,
    launchAtLogin: Boolean,
    userName: String?,
    appVersion: String,
    availableUpdate: UpdateManifest?,
    updateStatus: String,
    onLaunchAtLoginChange: (Boolean) -> Boolean,
    onBack: () -> Unit,
    onLogout: () -> Unit,
    onOpenLogs: () -> Unit,
    onExportConfig: () -> Unit,
    onImportConfig: () -> Unit,
    onClearCache: () -> Unit,
    onCheckUpdate: () -> Unit,
    onInstallUpdate: () -> Unit,
    onSave: (UserConfig) -> List<String>,
) {
    val semantic = LocalSemanticColors.current
    var tab by remember { mutableStateOf(SettingsTab.SYNC_DIR) }
    var mountDir by remember(initialConfig) { mutableStateOf(initialConfig.mountDir) }
    var concurrency by remember(initialConfig) { mutableStateOf(initialConfig.concurrency) }
    var pollInterval by remember(initialConfig) { mutableStateOf(initialConfig.pollIntervalSec.toInt()) }
    var debounce by remember(initialConfig) { mutableStateOf(initialConfig.debounceSec.toInt()) }
    var oauthPort by remember(initialConfig) { mutableStateOf(initialConfig.oauthCallbackPort) }
    var launchEnabled by remember(launchAtLogin) { mutableStateOf(launchAtLogin) }
    var errors by remember { mutableStateOf<List<String>>(emptyList()) }
    val mountConfigured = mountDir.isNotBlank()
    val showFooter = tab in setOf(SettingsTab.SYNC_DIR, SettingsTab.TRANSFER, SettingsTab.ADVANCED)

    Column(modifier = Modifier.fillMaxSize().background(semantic.bgPage)) {
        // AppBar 56px
        Row(
            modifier = Modifier.fillMaxWidth().height(56.dp).background(semantic.bgContainer).padding(horizontal = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            MateButton(variant = MateButtonVariant.ICON, icon = "arrow", onClick = onBack, modifier = Modifier.rotate(180f))
            Text("设置", fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
        }
        Row(modifier = Modifier.weight(1f)) {
            // 左导航 200px
            Column(
                modifier = Modifier.width(200.dp).fillMaxHeight().background(semantic.bgPage).padding(horizontal = 8.dp, vertical = 16.dp),
            ) {
                Text("设置", fontSize = 12.sp, fontWeight = FontWeight.SemiBold, color = semantic.textSecondary, modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp))
                SettingsTab.values().forEach { t ->
                    MateNavItem(
                        label = t.label,
                        icon = t.icon,
                        active = tab == t,
                        onClick = { tab = t },
                    )
                }
            }
            // 右设置区
            Column(
                modifier = Modifier.weight(1f).fillMaxHeight().verticalScroll(rememberScrollState()).padding(24.dp),
            ) {
                when (tab) {
                    SettingsTab.SYNC_DIR -> {
                        MateSectionHeader("同步目录", icon = "folder")
                        if (!mountConfigured) {
                            SettingsCard {
                                MateIcon(name = "folder-open", size = 48.dp, tint = semantic.textSecondary)
                                Text("尚未配置同步目录", fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
                                Text("选择一个本地空目录作为云盘镜像，文件将自动双向同步。", fontSize = 13.sp, color = semantic.textSecondary)
                                MateButton(label = "选择目录", icon = "folder-open", onClick = { /* TODO 选目录 */ })
                            }
                        } else {
                            SettingsCard(success = true) {
                                MateIcon(name = "check", size = 20.dp, tint = io.github.yuanbaobaoo.petallink.ui.theme.SuccessColor)
                                Text("当前同步目录", fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
                                Text(mountDir, fontSize = 12.sp, color = semantic.textSecondary)
                                MateButton(label = "更换目录", variant = MateButtonVariant.TEXT, icon = "folder-open", onClick = {})
                            }
                        }
                        MateInfoBanner(message = "更换同步目录将清除所有本地缓存与登录状态并重启，云盘文件不受影响。", variant = MateBannerVariant.INFO)
                    }
                    SettingsTab.TRANSFER -> {
                        MateSectionHeader("传输设置", icon = "transfer")
                        SettingRow("并发上传数", "同时进行的文件传输任务数量。", "${concurrency}") {
                            MateStepper(value = concurrency, onValueChange = { concurrency = it }, min = 1, max = 20)
                        }
                        SettingRow("Debounce 时长", "文件变更后等待多少秒再触发同步上传。", "") {
                            MateNumberField(value = debounce, onValueChange = { debounce = it }, min = 1, max = 600, suffix = "秒")
                        }
                        SettingRow("自动同步间隔", "每隔多久自动从云端拉取最新变更。0 = 关闭。", "") {
                            MateNumberField(value = pollInterval, onValueChange = { pollInterval = it }, min = 0, max = 86400, suffix = "秒")
                        }
                    }
                    SettingsTab.ADVANCED -> {
                        MateSectionHeader("高级设置", icon = "settings")
                        SettingRow("开机自启动", "开机登录后自动在后台启动。", "") {
                            MateSwitch(checked = launchEnabled, onCheckedChange = { req ->
                                if (onLaunchAtLoginChange(req)) launchEnabled = req else errors = listOf("设置开机自启失败")
                            })
                        }
                        SettingRow("OAuth 回调端口", "本地 HTTP 回调服务器监听端口。", "") {
                            MateNumberField(value = oauthPort, onValueChange = { oauthPort = it }, min = 1, max = 65535)
                        }
                        Row {
                            MateButton(label = "导出配置", onClick = onExportConfig)
                            Spacer(Modifier.width(8.dp))
                            MateButton(label = "导入配置", onClick = onImportConfig)
                            Spacer(Modifier.width(8.dp))
                            MateButton(label = "清空缓存", danger = true, icon = "trash", onClick = onClearCache)
                        }
                    }
                    SettingsTab.ACCOUNT -> {
                        MateSectionHeader("账号管理", icon = "info")
                        Text(userName ?: "华为账号", fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
                        Spacer(Modifier.height(16.dp))
                        MateButton(label = "退出登录", danger = true, icon = "x", onClick = onLogout)
                    }
                    SettingsTab.LOGS -> {
                        MateSectionHeader("日志查看", icon = "list")
                        Text("运行日志使用共享 1000 条 ring buffer，并保留 30 天滚动文件。")
                        Spacer(Modifier.height(12.dp))
                        MateButton(label = "打开日志查看器", onClick = onOpenLogs)
                    }
                    SettingsTab.ABOUT -> {
                        MateSectionHeader("关于", icon = "cloud")
                        Text("PetalLink", fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
                        Text("版本 $appVersion", fontSize = 12.sp, color = semantic.textSecondary)
                        Spacer(Modifier.height(16.dp))
                        Text(updateStatus, fontSize = 12.sp, color = semantic.textSecondary)
                        Spacer(Modifier.height(8.dp))
                        if (availableUpdate == null) {
                            MateButton(label = "检查更新", variant = MateButtonVariant.TEXT, icon = "refresh", onClick = onCheckUpdate)
                        } else {
                            MateButton(label = "安装 ${availableUpdate.version}", onClick = onInstallUpdate)
                        }
                    }
                }
            }
        }
        // footer（仅 syncDir/transfer/advanced）
        if (showFooter) {
            Row(
                modifier = Modifier.fillMaxWidth().height(56.dp).background(semantic.bgContainer).padding(horizontal = 24.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                errors.firstOrNull()?.let { Text("⚠️ $it", fontSize = 12.sp, color = io.github.yuanbaobaoo.petallink.ui.theme.ErrorColor) }
                Spacer(Modifier.weight(1f))
                MateButton(
                    label = "保存设置",
                    icon = "check",
                    onClick = {
                        val config = UserConfig(
                            oauthRedirectUri = initialConfig.oauthRedirectUri,
                            mountDir = mountDir,
                            mountConfigured = mountDir.isNotBlank(),
                            concurrency = concurrency,
                            pollIntervalSec = pollInterval.toLong(),
                            debounceSec = debounce.toLong(),
                            oauthCallbackPort = oauthPort,
                            skipPatterns = initialConfig.skipPatterns,
                            sortField = initialConfig.sortField,
                            sortOrder = initialConfig.sortOrder,
                        )
                        errors = onSave(config)
                    },
                )
            }
        }
    }
}

/** 设置卡片容器（对标原 Vue .card：padding xl，border，radius-md，column center）。 */
@Composable
private fun SettingsCard(success: Boolean = false, content: @Composable () -> Unit) {
    val semantic = LocalSemanticColors.current
    val borderColor = if (success) io.github.yuanbaobaoo.petallink.ui.theme.SuccessColor else semantic.border
    Column(
        modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp).clip(RoundedCornerShape(6.dp))
            .background(semantic.bgContainer).border(1.dp, borderColor, RoundedCornerShape(6.dp)).padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) { content() }
}

/** 设置行（label + desc + control，底 0.5px border）。 */
@Composable
private fun SettingRow(label: String, desc: String, suffix: String, control: @Composable () -> Unit) {
    val semantic = LocalSemanticColors.current
    Column(modifier = Modifier.fillMaxWidth().padding(vertical = 16.dp)) {
        Text(label, fontSize = 14.sp, fontWeight = FontWeight.Medium, color = semantic.textPrimary)
        if (desc.isNotEmpty()) Text(desc, fontSize = 12.sp, color = semantic.textSecondary, modifier = Modifier.padding(top = 4.dp))
        Box(modifier = Modifier.padding(top = 8.dp)) { control() }
    }
}

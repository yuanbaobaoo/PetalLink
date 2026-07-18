@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import io.github.yuanbaobaoo.petallink.auth.UserInfo
import io.github.yuanbaobaoo.petallink.config.UserConfig
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateBannerVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButton
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButtonVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateInfoBanner
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateLogoWithText
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateNavGroupLabel
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateNavItem
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateNumberField
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateSectionHeader
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateStepper
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateSwitch
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateTextField
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateToastVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.showToast
import io.github.yuanbaobaoo.petallink.ui.theme.LOCAL_SEMANTIC_COLORS
import io.github.yuanbaobaoo.petallink.ui.theme.PetalTheme
import io.github.yuanbaobaoo.petallink.update.UpdateManifest
import java.awt.Desktop
import java.net.URI

/**
 * 设置页标签页：每个值带显示名称与图标名，涵盖同步目录、传输设置等分区。
 */
private enum class SettingsTab(val label: String, val icon: String) {
    SYNC_DIR("同步目录", "folder"),
    TRANSFER("传输设置", "transfer"),
    ADVANCED("高级设置", "settings"),
    ACCOUNT("账号管理", "info"),
    LOGS("日志查看", "list"),
    ABOUT("关于", "cloud"),
}

/**
 * 「通用」分组（同步目录/传输设置/高级设置），v2 settings-nav 分组结构。
 */
private val GeneralTabs = listOf(SettingsTab.SYNC_DIR, SettingsTab.TRANSFER, SettingsTab.ADVANCED)

/**
 * 「其他」分组（账号管理/日志查看/关于）。
 */
private val OtherTabs = listOf(SettingsTab.ACCOUNT, SettingsTab.LOGS, SettingsTab.ABOUT)

/**
 * 设置页（v2 重构，对标 design/v2/06-settings.html；原 Vue SettingsPage.vue）。
 *
 * 双栏：左导航 240px（标题「设置」+ 通用/其他两组 MateNavItem）+ 右设置区（bgPage，scroll，
 * 内容包白色 settings-panel）；footer(64px)：保存/重置 + saved/error 状态，仅随右侧设置区铺底。
 */
@Composable
fun SettingsScreen(
    initialConfig: UserConfig,
    launchAtLogin: Boolean,
    userInfo: UserInfo?,
    appVersion: String,
    quotaUsed: Long?,
    quotaTotal: Long?,
    availableUpdate: UpdateManifest?,
    updateStatus: String,
    updateChecking: Boolean,
    onLaunchAtLoginChange: (Boolean) -> Boolean,
    onBack: () -> Unit,
    onLogout: () -> Unit,
    onOpenLogs: () -> Unit,
    onExportConfig: () -> Unit,
    onImportConfig: () -> Unit,
    onClearCache: () -> Unit,
    onCheckUpdate: () -> Unit,
    onInstallUpdate: () -> Unit,
    onSelectDir: ((String) -> Unit) -> Unit,
    onSave: (UserConfig) -> List<String>,
) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    var tab by remember { mutableStateOf(SettingsTab.SYNC_DIR) }
    var mountDir by remember(initialConfig) { mutableStateOf(initialConfig.mountDir) }
    var concurrency by remember(initialConfig) { mutableStateOf(initialConfig.concurrency) }
    var pollInterval by remember(initialConfig) { mutableStateOf(initialConfig.pollIntervalSec.toInt()) }
    var debounce by remember(initialConfig) { mutableStateOf(initialConfig.debounceSec.toInt()) }
    var oauthPort by remember(initialConfig) { mutableStateOf(initialConfig.oauthCallbackPort) }
    var skipPatterns by remember(initialConfig) { mutableStateOf(initialConfig.skipPatterns.joinToString(", ") ) }
    var launchEnabled by remember(launchAtLogin) { mutableStateOf(launchAtLogin) }
    var errors by remember { mutableStateOf<List<String>>(emptyList()) }
    var saved by remember { mutableStateOf(false) }
    val mountConfigured = mountDir.isNotBlank()
    val showFooter = tab in setOf(SettingsTab.SYNC_DIR, SettingsTab.TRANSFER, SettingsTab.ADVANCED)
    val userLabel = userInfo?.displayName ?: userInfo?.nickname ?: "未获取到"

    Column(modifier = Modifier.fillMaxSize().background(semantic.bgPage)) {
        // AppBar 56px
        Row(
            modifier = Modifier.fillMaxWidth().height(PetalTheme.metrics.settings.headerHeight)
                .background(semantic.bgContainer).padding(horizontal = PetalTheme.metrics.settings.headerHorizontalPadding),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.settings.headerContentSpacing),
        ) {
            MateButton(variant = MateButtonVariant.ICON, icon = "arrow", onClick = onBack, modifier = Modifier.rotate(180f))
            Text("设置", style = PetalTheme.typography.settings.pageTitle)
        }
        Row(modifier = Modifier.weight(1f)) {
            // 左导航 240px（v2 .settings-nav：padding 20/12，项间距 6）
            Column(
                modifier = Modifier.width(PetalTheme.metrics.settings.navigationWidth).fillMaxHeight().background(semantic.bgPage)
                    .padding(
                        horizontal = PetalTheme.metrics.settings.navigationHorizontalPadding,
                        vertical = PetalTheme.metrics.settings.navigationVerticalPadding,
                    ),
                verticalArrangement = Arrangement.spacedBy(PetalTheme.metrics.settings.navigationItemSpacing),
            ) {
                // 导航分组（v2：无「设置」小字标题，直接分组）
                MateNavGroupLabel("通用")
                GeneralTabs.forEach { t ->
                    MateNavItem(label = t.label, icon = t.icon, active = tab == t, onClick = { tab = t })
                }
                MateNavGroupLabel("其他")
                OtherTabs.forEach { t ->
                    MateNavItem(label = t.label, icon = t.icon, active = tab == t, onClick = { tab = t })
                }
            }
            // 导航与设置区间的 0.5px 细边（v2 .settings-nav border-right）
            Box(Modifier.fillMaxHeight().width(PetalTheme.metrics.settings.navigationBorderWidth).background(semantic.border))
            // 右设置区（v2 .settings-body：bgApp，padding 28/32；footer 只铺右侧底部）
            Column(modifier = Modifier.weight(1f).fillMaxHeight().background(semantic.bgPage)) {
                Column(
                    modifier = Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState())
                        .padding(
                            horizontal = PetalTheme.metrics.settings.bodyHorizontalPadding,
                            vertical = PetalTheme.metrics.settings.bodyVerticalPadding,
                        ),
                ) {
                    when (tab) {
                        SettingsTab.SYNC_DIR -> SyncDirSection(
                            mountDir,
                            mountConfigured,
                            onSelectDir = { onSelectDir { selected -> mountDir = selected } },
                        )
                        SettingsTab.TRANSFER -> {
                            MateSectionHeader("传输设置", icon = "transfer")
                            SettingsPanel {
                                GroupHeader("传输参数", first = true)
                                SettingRow("并发上传数", "同时进行的文件传输任务数量。较高值可提升大文件传输效率，但会占用更多网络带宽。") {
                                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.settings.concurrencyContentSpacing)) {
                                        MateStepper(value = concurrency, onValueChange = { concurrency = it }, min = 1, max = 20)
                                        Text("范围 1-20", style = PetalTheme.typography.settings.numberRangeHint, color = semantic.textSecondary)
                                    }
                                }
                                SettingRow("Debounce 时长", "文件变更后等待多少秒再触发同步上传，避免频繁修改导致重复传输。") {
                                    MateNumberField(value = debounce, onValueChange = { debounce = it }, min = 1, max = 600, suffix = "秒")
                                }
                                SettingRow("自动同步间隔", "每隔多久自动从云端拉取最新变更（新增/修改/删除）。0 = 关闭自动同步，仅手动点「同步索引」。设为 60 以上时生效。") {
                                    MateNumberField(value = pollInterval, onValueChange = { pollInterval = it }, min = 0, max = 86400, suffix = "秒")
                                }
                                GroupHeader("同步过滤")
                                SettingRow("跳过文件（逗号分隔）", "匹配名称的文件不会被同步，如 .DS_Store、临时文件。", showDivider = false) {
                                    MateTextField(value = skipPatterns, onValueChange = { skipPatterns = it }, placeholder = ".DS_Store, .tmp, ~$*, .Trash", modifier = Modifier.width(PetalTheme.metrics.settings.skipPatternFieldWidth))
                                }
                            }
                        }
                        SettingsTab.ADVANCED -> {
                            MateSectionHeader("高级设置", icon = "settings")
                            SettingsPanel {
                                GroupHeader("通用", first = true)
                                SettingRow("开机自启动", "开机登录后自动在后台启动（仅菜单栏图标，不显示主窗口）。关闭后需手动打开 App。") {
                                    MateSwitch(checked = launchEnabled, onCheckedChange = { req ->
                                        if (onLaunchAtLoginChange(req)) launchEnabled = req else errors = listOf("设置开机自启失败")
                                    })
                                }
                                GroupHeader("OAuth")
                                SettingRow("OAuth 回调端口", "本地 HTTP 回调服务器监听端口。修改后需与 AGC 后台 redirect_uri 保持一致。") {
                                    MateNumberField(value = oauthPort, onValueChange = { oauthPort = it }, min = 1, max = 65535)
                                }
                                Box(Modifier.padding(
                                    top = PetalTheme.metrics.settings.oauthBannerTopPadding,
                                    bottom = PetalTheme.metrics.settings.oauthBannerBottomPadding,
                                )) {
                                    MateInfoBanner(message = "回调地址固定为 http://127.0.0.1:<端口>/oauth/callback，修改端口后请同步更新 AGC 后台配置。", variant = MateBannerVariant.INFO)
                                }
                                GroupHeader("维护")
                                SettingRow("清空缓存并重启", "清除登录状态、同步数据库、同步快照与配置文件，然后重启 App。适用于排查同步异常或切换账号时使用。", showDivider = false) {
                                    MateButton(label = "清空", icon = "trash", danger = true, onClick = onClearCache)
                                }
                            }
                        }
                        SettingsTab.ACCOUNT -> AccountSection(userInfo, userLabel, quotaUsed, quotaTotal, onLogout)
                        SettingsTab.LOGS -> {
                            MateSectionHeader("日志查看", icon = "list")
                            SettingsPanel(
                                contentPadding = PaddingValues(PetalTheme.metrics.settings.logPanelPadding),
                                contentSpacing = PetalTheme.metrics.settings.logPanelContentSpacing,
                            ) {
                                Text("运行日志使用共享 1000 条 ring buffer，并保留 30 天滚动文件。", style = PetalTheme.typography.settings.logRetentionDescription, color = semantic.textPrimary)
                                MateButton(label = "打开日志查看器", onClick = onOpenLogs)
                            }
                        }
                        SettingsTab.ABOUT -> AboutSection(appVersion, availableUpdate, updateStatus, updateChecking, onCheckUpdate, onInstallUpdate)
                    }
                }
                // footer（仅 syncDir/transfer/advanced；v2 .settings-footer：64px，padding 0/32，顶细边）
                if (showFooter) {
                    Box(Modifier.fillMaxWidth().height(PetalTheme.metrics.settings.footerBorderWidth).background(semantic.border))
                    Row(
                        modifier = Modifier.fillMaxWidth().height(PetalTheme.metrics.settings.footerHeight)
                            .background(semantic.bgContainer)
                            .padding(horizontal = PetalTheme.metrics.settings.footerHorizontalPadding),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.settings.footerActionSpacing),
                    ) {
                        MateButton(label = if (saved) "已保存" else "保存设置", icon = "check", onClick = {
                            val config = UserConfig(
                                oauthRedirectUri = initialConfig.oauthRedirectUri,
                                mountDir = mountDir, mountConfigured = mountDir.isNotBlank(),
                                concurrency = concurrency, pollIntervalSec = pollInterval.toLong(),
                                debounceSec = debounce.toLong(), oauthCallbackPort = oauthPort,
                                skipPatterns = skipPatterns.split(",").map { it.trim() }.filter { it.isNotEmpty() },
                                sortField = initialConfig.sortField, sortOrder = initialConfig.sortOrder,
                            )
                            val errs = onSave(config)
                            errors = errs
                            if (errs.isEmpty()) { saved = true; showToast("配置已保存", MateToastVariant.SUCCESS) }
                        }, disabled = saved)
                        MateButton(label = "重置默认", variant = MateButtonVariant.ICON_TEXT, onClick = {
                            // 重置为 initialConfig（从后端重新加载由调用方处理，这里恢复本地编辑态）
                            mountDir = initialConfig.mountDir; concurrency = initialConfig.concurrency
                            pollInterval = initialConfig.pollIntervalSec.toInt(); debounce = initialConfig.debounceSec.toInt()
                            oauthPort = initialConfig.oauthCallbackPort
                            skipPatterns = initialConfig.skipPatterns.joinToString(", ")
                            saved = false; errors = emptyList()
                        })
                        Spacer(Modifier.weight(1f))
                        errors.firstOrNull()?.let { Text("⚠️ $it", style = PetalTheme.typography.settings.validationError, color = PetalTheme.colors.error) }
                        if (saved && errors.isEmpty()) {
                            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.settings.savedIndicatorSpacing)) {
                                Box(Modifier.size(PetalTheme.metrics.settings.savedIndicatorSize).clip(CircleShape).background(PetalTheme.colors.success))
                                Text("配置已保存", style = PetalTheme.typography.settings.saveSuccess, color = PetalTheme.colors.success)
                            }
                        }
                    }
                }
            }
        }
    }
}

/**
 * 设置面板（v2 .settings-panel：白卡 bgContainer，radius 10，0.5px 细边，默认 padding 4/24）。
 *
 * @param contentPadding 面板内边距（头像卡片/日志/关于用 16 或 24 覆盖默认值）
 * @param contentSpacing 直接子项间距（日志/关于面板用 14）
 */
@Composable
private fun SettingsPanel(
    modifier: Modifier = Modifier,
    contentPadding: PaddingValues = PaddingValues(
        horizontal = PetalTheme.metrics.settings.panelHorizontalPadding,
        vertical = PetalTheme.metrics.settings.panelVerticalPadding,
    ),
    contentSpacing: Dp = PetalTheme.metrics.settings.panelDefaultContentSpacing,
    content: @Composable ColumnScope.() -> Unit,
) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    Column(
        modifier = modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(PetalTheme.metrics.settings.panelRadius))
            .background(semantic.bgContainer)
            .border(PetalTheme.metrics.settings.panelBorderWidth, semantic.border, RoundedCornerShape(PetalTheme.metrics.settings.panelRadius))
            .padding(contentPadding),
        verticalArrangement = Arrangement.spacedBy(contentSpacing),
        content = content,
    )
}

/**
 * 分组标题（v2 .group-header：12px semibold secondary uppercase，无分割线；面板内首个上 12，其余上 20）。
 */
@Composable
private fun GroupHeader(label: String, first: Boolean = false) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    Text(
        label,
        style = PetalTheme.typography.settings.groupHeader,
        color = semantic.textSecondary,
        modifier = Modifier.fillMaxWidth().padding(
            top = if (first) PetalTheme.metrics.settings.firstGroupTopPadding else PetalTheme.metrics.settings.groupTopPadding,
            bottom = PetalTheme.metrics.settings.groupBottomPadding,
        ),
    )
}

/**
 * 设置行（v2 .setting-row：左侧 label+desc 占满剩余宽度，右侧 control，行间距 24；非末行底 0.5px 细边）。
 */
@Composable
private fun SettingRow(label: String, desc: String, showDivider: Boolean = true, control: @Composable () -> Unit) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    Column(modifier = Modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(vertical = PetalTheme.metrics.settings.settingRowVerticalPadding),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.settings.settingRowContentSpacing),
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(label, style = PetalTheme.typography.settings.optionTitle, color = semantic.textPrimary)
                Text(desc, style = PetalTheme.typography.settings.optionDescription, color = semantic.textSecondary, modifier = Modifier.padding(top = PetalTheme.metrics.settings.settingDescriptionTopPadding))
            }
            control()
        }
        if (showDivider) Box(Modifier.fillMaxWidth().height(PetalTheme.metrics.settings.settingRowDividerWidth).background(semantic.border))
    }
}

/**
 * 同步目录 Section（v2：radius 10 卡片；已配置 1px PetalTheme.colors.success 描边 + 成功徽章，未配置 MateEmpty 风格徽章引导）。
 */
@Composable
private fun SyncDirSection(mountDir: String, mountConfigured: Boolean, onSelectDir: () -> Unit) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    MateSectionHeader("同步目录", icon = "folder")
    Column(
        modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(PetalTheme.metrics.settings.mountPanelRadius)).background(semantic.bgContainer)
            .border(
                width = if (mountConfigured) PetalTheme.metrics.settings.configuredMountBorderWidth else PetalTheme.metrics.settings.emptyMountBorderWidth,
                color = if (mountConfigured) PetalTheme.colors.success else semantic.border,
                shape = RoundedCornerShape(PetalTheme.metrics.settings.mountPanelRadius),
            )
            .padding(
                horizontal = PetalTheme.metrics.settings.mountPanelHorizontalPadding,
                vertical = if (mountConfigured) PetalTheme.metrics.settings.configuredMountVerticalPadding else PetalTheme.metrics.settings.emptyMountVerticalPadding,
            ),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(PetalTheme.metrics.settings.mountPanelContentSpacing),
    ) {
        if (!mountConfigured) {
            // MateEmpty 风格图标区：72×72 radius 14 品牌浅底渐变徽章
            Box(
                modifier = Modifier.size(PetalTheme.metrics.settings.emptyMountBadgeSize)
                    .clip(RoundedCornerShape(PetalTheme.metrics.settings.emptyMountBadgeRadius))
                    .background(PetalTheme.colors.brandGradientSoft),
                contentAlignment = Alignment.Center,
            ) {
                MateIcon(name = "folder-open", size = PetalTheme.metrics.settings.emptyMountIconSize, tint = PetalTheme.colors.brandHover)
            }
            Text("尚未配置同步目录", style = PetalTheme.typography.settings.emptyMountTitle)
            Text("选择一个本地空目录作为云盘镜像，文件将自动双向同步。", style = PetalTheme.typography.settings.emptyMountDescription, color = semantic.textSecondary)
            MateButton(label = "选择目录", icon = "folder-open", onClick = onSelectDir)
        } else {
            // 成功态图标徽章（v2 dialog__icon-badge--ok：40×40 radius 10，PetalTheme.colors.successBg + PetalTheme.colors.success）
            Box(
                modifier = Modifier.size(PetalTheme.metrics.settings.configuredMountBadgeSize)
                    .clip(RoundedCornerShape(PetalTheme.metrics.settings.configuredMountBadgeRadius))
                    .background(PetalTheme.colors.successBg),
                contentAlignment = Alignment.Center,
            ) {
                MateIcon(name = "check", size = PetalTheme.metrics.settings.configuredMountIconSize, tint = PetalTheme.colors.success)
            }
            Text("当前同步目录", style = PetalTheme.typography.settings.currentMountTitle)
            Text(mountDir, style = PetalTheme.typography.settings.currentMountPath, color = semantic.textSecondary, maxLines = 2, overflow = TextOverflow.Ellipsis,
                modifier = Modifier.clip(RoundedCornerShape(PetalTheme.metrics.settings.mountPathRadius))
                    .background(semantic.bgFill)
                    .padding(
                        horizontal = PetalTheme.metrics.settings.mountPathHorizontalPadding,
                        vertical = PetalTheme.metrics.settings.mountPathVerticalPadding,
                    ))
            MateButton(label = "更换目录", variant = MateButtonVariant.SOFT, icon = "folder-open", onClick = onSelectDir)
        }
    }
    Spacer(Modifier.height(PetalTheme.metrics.settings.mountBannerSpacing))
    MateInfoBanner(message = "更换同步目录将清除所有本地缓存与登录状态并重启，云盘文件不受影响。", variant = MateBannerVariant.INFO)
}

/**
 * 账号管理 Section（v2：头像卡片 radius 10 + 信息面板；账号信息表 + 配额 + 退出登录）。
 */
@Composable
private fun AccountSection(userInfo: UserInfo?, userLabel: String, quotaUsed: Long?, quotaTotal: Long?, onLogout: () -> Unit) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    MateSectionHeader("账号管理", icon = "info")
    // 头像卡片（56×56 品牌渐变头像 + 用户名；padding 16/24）
    SettingsPanel(contentPadding = PaddingValues(
        horizontal = PetalTheme.metrics.settings.accountPanelHorizontalPadding,
        vertical = PetalTheme.metrics.settings.accountPanelVerticalPadding,
    )) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.settings.accountContentSpacing),
        ) {
            Box(
                modifier = Modifier.size(PetalTheme.metrics.settings.accountAvatarSize).clip(CircleShape).background(PetalTheme.colors.brandGradient),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    userLabel.firstOrNull()?.toString() ?: "华",
                    color = PetalTheme.colors.settingsAccountAvatarText,
                    style = PetalTheme.typography.settings.accountAvatar,
                )
            }
            Text(userLabel, style = PetalTheme.typography.settings.accountName, color = semantic.textPrimary)
        }
    }
    Spacer(Modifier.height(PetalTheme.metrics.settings.accountSectionSpacing))
    SettingsPanel {
        // 账号信息
        GroupHeader("账号信息", first = true)
        InfoRow("显示名", userInfo?.displayName ?: "—")
        InfoRow("手机号", userInfo?.mobile ?: "未授权")
        InfoRow("邮箱", userInfo?.email ?: "未授权")
        InfoRow("OpenID", userInfo?.openId ?: "—", mono = true)
        // 存储配额
        GroupHeader("存储配额")
        InfoRow("已用空间", quotaUsed?.let { formatFileSize(it) } ?: "—")
        InfoRow("总容量", quotaTotal?.takeIf { it > 0 }?.let { formatFileSize(it) } ?: "—")
        InfoRow("剩余空间", if (quotaTotal != null && quotaTotal > 0 && quotaUsed != null) formatFileSize(quotaTotal - quotaUsed) else "—")
        // 退出登录
        GroupHeader("账号操作")
        SettingRow("退出登录", "清除本地 token 并返回登录页。后台进程仍会继续，可从菜单栏彻底退出。", showDivider = false) {
            MateButton(label = "退出登录", icon = "x", danger = true, onClick = onLogout)
        }
    }
}

/**
 * 信息行（label 96px + value flex，底 0.5px border）。
 */
@Composable
private fun InfoRow(label: String, value: String, mono: Boolean = false) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    Column(modifier = Modifier.fillMaxWidth().padding(vertical = PetalTheme.metrics.settings.detailRowVerticalPadding)) {
        Row {
            Text(label, style = PetalTheme.typography.settings.detailLabel, color = semantic.textSecondary, modifier = Modifier.width(PetalTheme.metrics.settings.detailLabelWidth))
            Text(value, style = PetalTheme.typography.settings.detailValue, color = semantic.textPrimary)
        }
        Spacer(Modifier.height(PetalTheme.metrics.settings.detailContentSpacing))
        Box(Modifier.fillMaxWidth().height(PetalTheme.metrics.settings.detailDividerWidth).background(semantic.border))
    }
}

/**
 * 关于 Section（v2 白卡：LogoWithText + 版本 + 检查更新 + GitHub/GitCode 外链）。
 */
@Composable
private fun AboutSection(
    appVersion: String,
    availableUpdate: UpdateManifest?,
    updateStatus: String,
    updateChecking: Boolean,
    onCheckUpdate: () -> Unit,
    onInstallUpdate: () -> Unit,
) {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    MateSectionHeader("关于", icon = "cloud")
    SettingsPanel(
        contentPadding = PaddingValues(PetalTheme.metrics.settings.aboutPanelPadding),
        contentSpacing = PetalTheme.metrics.settings.aboutPanelContentSpacing,
    ) {
        MateLogoWithText(height = PetalTheme.metrics.settings.aboutLogoHeight)
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.settings.versionContentSpacing)) {
            Text("版本 ${appVersion.ifBlank { "..." }}", style = PetalTheme.typography.settings.version, color = semantic.textSecondary)
            MateButton(label = if (updateChecking) "检查中…" else "检查更新", variant = MateButtonVariant.TEXT, icon = "refresh",
                onClick = onCheckUpdate, disabled = updateChecking)
            if (updateStatus.isNotEmpty()) Text(updateStatus, style = PetalTheme.typography.settings.updateStatus, color = semantic.textSecondary)
        }
        if (availableUpdate != null) {
            MateButton(label = "安装 ${availableUpdate.version}", icon = "download", onClick = onInstallUpdate)
        }
        Text("一款开源免费的华为云盘客户端", style = PetalTheme.typography.settings.aboutDescription, color = semantic.textSecondary)
        // GitHub / GitCode 外链
        Row(horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.settings.externalLinksSpacing)) {
            LinkItem("GitHub", "github", "https://github.com/yuanbaobaoo/PetalLink")
            LinkItem("GitCode", "gitcode", "https://gitcode.com/yuanbaobaoo/PetalLink")
        }
    }
}

/**
 * 外链项（brand 色，点击打开浏览器）。
 */
@Composable
private fun LinkItem(label: String, icon: String, url: String) {
    Row(
        modifier = Modifier.clickable {
            runCatching { Desktop.getDesktop().browse(URI(url)) }
        }.padding(vertical = PetalTheme.metrics.settings.externalLinkVerticalPadding),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.settings.externalLinkContentSpacing),
    ) {
        MateIcon(name = icon, size = PetalTheme.metrics.settings.externalLinkIconSize, tint = PetalTheme.colors.brand)
        Text(label, style = PetalTheme.typography.settings.externalLink, color = PetalTheme.colors.brand)
    }
}

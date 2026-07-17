@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
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
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.auth.UserInfo
import io.github.yuanbaobaoo.petallink.config.UserConfig
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateAppLogo
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateBannerVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButton
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButtonVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateInfoBanner
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateLogoWithText
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateNavItem
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateNumberField
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateSectionHeader
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateStepper
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateSwitch
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateTextField
import io.github.yuanbaobaoo.petallink.ui.components.mate.showToast
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateToastVariant
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.BrandHover
import io.github.yuanbaobaoo.petallink.ui.theme.ErrorColor
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import io.github.yuanbaobaoo.petallink.ui.theme.SuccessColor
import io.github.yuanbaobaoo.petallink.update.UpdateManifest
import java.awt.Desktop
import java.net.URI

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
 * 双栏：左导航 200px（6 Tab）+ 右设置区(scroll)；footer(56px)：保存/重置 + saved/error 状态。
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
    onSave: (UserConfig) -> List<String>,
) {
    val semantic = LocalSemanticColors.current
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
            modifier = Modifier.fillMaxWidth().height(56.dp).background(semantic.bgContainer).padding(horizontal = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            MateButton(variant = MateButtonVariant.ICON, icon = "arrow", onClick = onBack, modifier = Modifier.rotate(180f))
            Text("设置", fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
        }
        Row(modifier = Modifier.weight(1f)) {
            // 左导航 200px
            Column(modifier = Modifier.width(200.dp).fillMaxHeight().background(semantic.bgPage).padding(horizontal = 8.dp, vertical = 16.dp)) {
                Text("设置", fontSize = 12.sp, fontWeight = FontWeight.SemiBold, color = semantic.textSecondary, modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp))
                SettingsTab.values().forEach { t ->
                    MateNavItem(label = t.label, icon = t.icon, active = tab == t, onClick = { tab = t })
                }
            }
            // 右设置区
            Column(modifier = Modifier.weight(1f).fillMaxHeight().verticalScroll(rememberScrollState()).padding(24.dp)) {
                when (tab) {
                    SettingsTab.SYNC_DIR -> SyncDirSection(mountDir, mountConfigured, onSelectDir = {})
                    SettingsTab.TRANSFER -> {
                        MateSectionHeader("传输设置", icon = "transfer")
                        GroupHeader("传输参数")
                        SettingRow("并发上传数", "同时进行的文件传输任务数量。较高值可提升大文件传输效率，但会占用更多网络带宽。") {
                            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                MateStepper(value = concurrency, onValueChange = { concurrency = it }, min = 1, max = 20)
                                Text("范围 1-20", fontSize = 13.sp, color = semantic.textSecondary)
                            }
                        }
                        SettingRow("Debounce 时长", "文件变更后等待多少秒再触发同步上传，避免频繁修改导致重复传输。") {
                            MateNumberField(value = debounce, onValueChange = { debounce = it }, min = 1, max = 600, suffix = "秒")
                        }
                        SettingRow("自动同步间隔", "每隔多久自动从云端拉取最新变更（新增/修改/删除）。0 = 关闭自动同步，仅手动点「同步索引」。设为 60 以上时生效。") {
                            MateNumberField(value = pollInterval, onValueChange = { pollInterval = it }, min = 0, max = 86400, suffix = "秒")
                        }
                        GroupHeader("同步过滤")
                        SettingRow("跳过文件（逗号分隔）", "匹配名称的文件不会被同步，如 .DS_Store、临时文件。") {
                            MateTextField(value = skipPatterns, onValueChange = { skipPatterns = it }, placeholder = ".DS_Store, .tmp, ~$*, .Trash", modifier = Modifier.fillMaxWidth())
                        }
                    }
                    SettingsTab.ADVANCED -> {
                        MateSectionHeader("高级设置", icon = "settings")
                        GroupHeader("通用")
                        SettingRow("开机自启动", "开机登录后自动在后台启动（仅菜单栏图标，不显示主窗口）。关闭后需手动打开 App。") {
                            MateSwitch(checked = launchEnabled, onCheckedChange = { req ->
                                if (onLaunchAtLoginChange(req)) launchEnabled = req else errors = listOf("设置开机自启失败")
                            })
                        }
                        GroupHeader("OAuth")
                        SettingRow("OAuth 回调端口", "本地 HTTP 回调服务器监听端口。修改后需与 AGC 后台 redirect_uri 保持一致。") {
                            MateNumberField(value = oauthPort, onValueChange = { oauthPort = it }, min = 1, max = 65535)
                        }
                        MateInfoBanner(message = "回调地址固定为 http://127.0.0.1:<端口>/oauth/callback，修改端口后请同步更新 AGC 后台配置。", variant = MateBannerVariant.INFO)
                        GroupHeader("维护")
                        SettingRow("清空缓存并重启", "清除登录状态、同步数据库、同步快照与配置文件，然后重启 App。适用于排查同步异常或切换账号时使用。") {
                            MateButton(label = "清空", icon = "trash", danger = true, onClick = onClearCache)
                        }
                    }
                    SettingsTab.ACCOUNT -> AccountSection(userInfo, userLabel, quotaUsed, quotaTotal, onLogout)
                    SettingsTab.LOGS -> {
                        MateSectionHeader("日志查看", icon = "list")
                        Text("运行日志使用共享 1000 条 ring buffer，并保留 30 天滚动文件。")
                        Spacer(Modifier.height(12.dp))
                        MateButton(label = "打开日志查看器", onClick = onOpenLogs)
                    }
                    SettingsTab.ABOUT -> AboutSection(appVersion, availableUpdate, updateStatus, updateChecking, onCheckUpdate, onInstallUpdate)
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
                MateButton(label = "重置默认", variant = MateButtonVariant.TEXT, onClick = {
                    // 重置为 initialConfig（从后端重新加载由调用方处理，这里恢复本地编辑态）
                    mountDir = initialConfig.mountDir; concurrency = initialConfig.concurrency
                    pollInterval = initialConfig.pollIntervalSec.toInt(); debounce = initialConfig.debounceSec.toInt()
                    oauthPort = initialConfig.oauthCallbackPort
                    skipPatterns = initialConfig.skipPatterns.joinToString(", ")
                    saved = false; errors = emptyList()
                })
                Spacer(Modifier.weight(1f))
                errors.firstOrNull()?.let { Text("⚠️ $it", fontSize = 12.sp, color = ErrorColor) }
                if (saved && errors.isEmpty()) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                        Box(Modifier.size(6.dp).clip(CircleShape).background(SuccessColor))
                        Text("配置已保存", fontSize = 12.sp, color = SuccessColor)
                    }
                }
            }
        }
    }
}

/** 分组标题（对标 .group-header：13px semibold secondary，底 0.5px border，上下间距）。 */
@Composable
private fun GroupHeader(label: String) {
    val semantic = LocalSemanticColors.current
    Column(modifier = Modifier.fillMaxWidth().padding(top = 24.dp, bottom = 12.dp)) {
        Text(label, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, color = semantic.textSecondary)
        Spacer(Modifier.height(8.dp))
        Box(Modifier.fillMaxWidth().height(0.5.dp).background(semantic.border))
    }
}

/** 设置行（label + desc + control，底 0.5px border）。 */
@Composable
private fun SettingRow(label: String, desc: String, control: @Composable () -> Unit) {
    val semantic = LocalSemanticColors.current
    Column(modifier = Modifier.fillMaxWidth().padding(vertical = 16.dp)) {
        Text(label, fontSize = 14.sp, fontWeight = FontWeight.Medium, color = semantic.textPrimary)
        Text(desc, fontSize = 12.sp, color = semantic.textSecondary, modifier = Modifier.padding(top = 4.dp))
        Box(modifier = Modifier.padding(top = 8.dp)) { control() }
    }
}

/** 同步目录 Section。 */
@Composable
private fun SyncDirSection(mountDir: String, mountConfigured: Boolean, onSelectDir: () -> Unit) {
    val semantic = LocalSemanticColors.current
    MateSectionHeader("同步目录", icon = "folder")
    Column(
        modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(6.dp)).background(semantic.bgContainer)
            .border(1.dp, if (mountConfigured) SuccessColor else semantic.border, RoundedCornerShape(6.dp))
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (!mountConfigured) {
            MateIcon(name = "folder-open", size = 48.dp, tint = semantic.textSecondary)
            Text("尚未配置同步目录", fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
            Text("选择一个本地空目录作为云盘镜像，文件将自动双向同步。", fontSize = 13.sp, color = semantic.textSecondary)
            MateButton(label = "选择目录", icon = "folder-open", onClick = onSelectDir)
        } else {
            MateIcon(name = "check", size = 20.dp, tint = SuccessColor)
            Text("当前同步目录", fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
            Text(mountDir, fontSize = 12.sp, color = semantic.textSecondary, maxLines = 2, overflow = TextOverflow.Ellipsis,
                modifier = Modifier.clip(RoundedCornerShape(3.dp)).background(semantic.bgHover).padding(horizontal = 8.dp, vertical = 2.dp))
            MateButton(label = "更换目录", variant = MateButtonVariant.TEXT, icon = "folder-open", onClick = onSelectDir)
        }
    }
    Spacer(Modifier.height(16.dp))
    MateInfoBanner(message = "更换同步目录将清除所有本地缓存与登录状态并重启，云盘文件不受影响。", variant = MateBannerVariant.INFO)
}

/** 账号管理 Section（头像卡片 + 账号信息表 + 配额 + 退出登录）。 */
@Composable
private fun AccountSection(userInfo: UserInfo?, userLabel: String, quotaUsed: Long?, quotaTotal: Long?, onLogout: () -> Unit) {
    val semantic = LocalSemanticColors.current
    MateSectionHeader("账号管理", icon = "info")
    // 头像卡片（56×56 渐变头像 + 用户名）
    Row(
        modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(6.dp)).background(semantic.bgContainer)
            .border(1.dp, semantic.border, RoundedCornerShape(6.dp)).padding(24.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Box(
            modifier = Modifier.size(56.dp).clip(CircleShape)
                .background(Brush.linearGradient(listOf(BrandColor, BrandHover))),
            contentAlignment = Alignment.Center,
        ) { Text(userLabel.firstOrNull()?.toString() ?: "华", color = Color.White, fontSize = 22.sp, fontWeight = FontWeight.SemiBold) }
        Text(userLabel, fontSize = 16.sp, fontWeight = FontWeight.SemiBold, color = semantic.textPrimary)
    }
    Spacer(Modifier.height(16.dp))
    // 账号信息
    GroupHeader("账号信息")
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
    SettingRow("退出登录", "清除本地 token 并返回登录页。后台进程仍会继续，可从菜单栏彻底退出。") {
        MateButton(label = "退出登录", icon = "x", danger = true, onClick = onLogout)
    }
}

/** 信息行（label 96px + value flex，底 0.5px border）。 */
@Composable
private fun InfoRow(label: String, value: String, mono: Boolean = false) {
    val semantic = LocalSemanticColors.current
    Column(modifier = Modifier.fillMaxWidth().padding(vertical = 12.dp)) {
        Row {
            Text(label, fontSize = 13.sp, color = semantic.textSecondary, modifier = Modifier.width(96.dp))
            Text(value, fontSize = 13.sp, color = semantic.textPrimary)
        }
        Spacer(Modifier.height(12.dp))
        Box(Modifier.fillMaxWidth().height(0.5.dp).background(semantic.border))
    }
}

/** 关于 Section（LogoWithText + 版本 + 检查更新 + GitHub/GitCode 外链）。 */
@Composable
private fun AboutSection(
    appVersion: String,
    availableUpdate: UpdateManifest?,
    updateStatus: String,
    updateChecking: Boolean,
    onCheckUpdate: () -> Unit,
    onInstallUpdate: () -> Unit,
) {
    val semantic = LocalSemanticColors.current
    MateSectionHeader("关于", icon = "cloud")
    Column(
        modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(6.dp)).background(semantic.bgContainer)
            .border(1.dp, semantic.border, RoundedCornerShape(6.dp)).padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        MateLogoWithText(height = 30.dp)
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("版本 ${appVersion.ifBlank { "..." }}", fontSize = 12.sp, color = semantic.textSecondary)
            MateButton(label = if (updateChecking) "检查中…" else "检查更新", variant = MateButtonVariant.TEXT, icon = "refresh",
                onClick = onCheckUpdate, disabled = updateChecking)
            if (updateStatus.isNotEmpty()) Text(updateStatus, fontSize = 12.sp, color = semantic.textSecondary)
        }
        if (availableUpdate != null) {
            MateButton(label = "安装 ${availableUpdate.version}", icon = "download", onClick = onInstallUpdate)
        }
        Text("一款开源免费的华为云盘客户端", fontSize = 12.sp, color = semantic.textSecondary)
        // GitHub / GitCode 外链
        Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
            LinkItem("GitHub", "github", "https://github.com/yuanbaobaoo/PetalLink")
            LinkItem("GitCode", "gitcode", "https://gitcode.com/yuanbaobaoo/PetalLink")
        }
    }
}

/** 外链项（brand 色，点击打开浏览器）。 */
@Composable
private fun LinkItem(label: String, icon: String, url: String) {
    Row(
        modifier = Modifier.clickable {
            runCatching { Desktop.getDesktop().browse(URI(url)) }
        }.padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        MateIcon(name = icon, size = 16.dp, tint = BrandColor)
        Text(label, fontSize = 13.sp, color = BrandColor)
    }
}

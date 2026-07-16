package io.github.yuanbaobaao.petallink.ui.pages

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaao.petallink.config.UserConfig
import io.github.yuanbaobaao.petallink.config.ConfigValidator
import io.github.yuanbaobaao.petallink.core.logging.LogLevel
import io.github.yuanbaobaao.petallink.ui.components.*
import io.github.yuanbaobaao.petallink.ui.theme.*

// === LoginScreen ===
@Composable
fun LoginScreen(onLogin: () -> Unit) {
    var statusText by remember { mutableStateOf("点击下方按钮登录华为账号") }
    var isLoggingIn by remember { mutableStateOf(false) }

    Column(
        modifier = Modifier.fillMaxSize().padding(48.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Text("PetalLink", fontSize = 32.sp, fontWeight = FontWeight.Bold, color = BrandColor)
        Spacer(Modifier.height(8.dp))
        Text("华为云盘 macOS 客户端", fontSize = 14.sp, color = Color.Gray)
        Spacer(Modifier.height(24.dp))
        Text(statusText, fontSize = 13.sp, color = Color(0xFF666666))
        Spacer(Modifier.height(16.dp))
        MateButton(
            text = if (isLoggingIn) "登录中..." else "登录华为账号",
            onClick = {
                if (!isLoggingIn) {
                    isLoggingIn = true
                    statusText = "正在打开浏览器..."
                    onLogin()
                }
            },
            enabled = !isLoggingIn,
        )
    }
}

// === MainScreen ===
@Composable
fun MainScreen(
    syncStatus: String,
    isOnline: Boolean,
    fileItems: List<MateFileItemData>,
    transferItems: List<MateTransferItemData>,
    onRefresh: () -> Unit,
) {
    Row(modifier = Modifier.fillMaxSize()) {
        // 侧边栏
        Column(
            modifier = Modifier.width(200.dp).fillMaxHeight().padding(8.dp),
        ) {
            MateSectionHeader("PetalLink", subtitle = syncStatus)
            Spacer(Modifier.height(8.dp))
            Text(if (isOnline) "🟢 在线" else "🔴 离线", fontSize = 12.sp)
            Spacer(Modifier.height(16.dp))
            Text("📁 全部文件", fontSize = 13.sp, modifier = Modifier.padding(vertical = 4.dp))
            Text("📁 我的文件", fontSize = 13.sp, modifier = Modifier.padding(vertical = 4.dp))
        }

        // 主内容区
        Column(modifier = Modifier.weight(1f).fillMaxHeight().padding(8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("文件列表", fontSize = 16.sp, fontWeight = FontWeight.Bold, modifier = Modifier.weight(1f))
                TextButton(onClick = onRefresh) { Text("刷新") }
            }
            Spacer(Modifier.height(8.dp))

            if (fileItems.isEmpty()) {
                MateEmpty("📂", "暂无文件", "点击刷新加载文件列表")
            } else {
                // 列表头
                Row(modifier = Modifier.fillMaxWidth().padding(8.dp)) {
                    Text("名称", fontSize = 12.sp, color = Color.Gray, modifier = Modifier.weight(1f))
                    Text("大小", fontSize = 12.sp, color = Color.Gray, modifier = Modifier.width(80.dp))
                    Text("修改时间", fontSize = 12.sp, color = Color.Gray, modifier = Modifier.width(120.dp))
                }
                Divider()
                fileItems.forEach { item -> MateFileItem(item) }
            }

            // 传输状态浮层（底部）
            if (transferItems.isNotEmpty()) {
                Spacer(Modifier.height(8.dp))
                Divider()
                Text("传输中 (${transferItems.size})", fontSize = 14.sp, fontWeight = FontWeight.Medium, modifier = Modifier.padding(8.dp))
                transferItems.forEach { item -> MateTransferItem(item) }
            }
        }
    }
}

// === SettingsScreen ===
@Composable
fun SettingsScreen(onSave: (UserConfig) -> List<String>) {
    var mountDir by remember { mutableStateOf("/Users/me/PetalLink") }
    var concurrency by remember { mutableStateOf("6") }
    var pollInterval by remember { mutableStateOf("60") }
    var debounce by remember { mutableStateOf("3") }
    var oauthPort by remember { mutableStateOf("17890") }
    var launchAtLogin by remember { mutableStateOf(false) }
    var errors by remember { mutableStateOf<List<String>>(emptyList()) }

    Column(
        modifier = Modifier.fillMaxSize().padding(24.dp).verticalScroll(rememberScrollState()),
    ) {
        Text("设置", fontSize = 20.sp, fontWeight = FontWeight.Bold)
        Spacer(Modifier.height(24.dp))

        MateTextField(mountDir, { mountDir = it }, "挂载目录")
        Spacer(Modifier.height(12.dp))
        MateTextField(concurrency, { concurrency = it }, "并发传输数（1-20）")
        Spacer(Modifier.height(12.dp))
        MateTextField(pollInterval, { pollInterval = it }, "增量轮询间隔（秒，0=禁用）")
        Spacer(Modifier.height(12.dp))
        MateTextField(debounce, { debounce = it }, "文件监听去抖（秒）")
        Spacer(Modifier.height(12.dp))
        MateTextField(oauthPort, { oauthPort = it }, "OAuth 回调端口")
        Spacer(Modifier.height(12.dp))
        MateSwitch(checked = launchAtLogin, onCheckedChange = { launchAtLogin = it }, label = "开机自动启动")
        Spacer(Modifier.height(16.dp))

        errors.forEach { err ->
            Text("⚠️ $err", fontSize = 12.sp, color = ErrorColor)
        }

        Spacer(Modifier.height(16.dp))
        MateButton("保存设置", onClick = {
            val config = UserConfig(
                mountDir = mountDir,
                concurrency = concurrency.toIntOrNull() ?: 6,
                pollIntervalSec = pollInterval.toLongOrNull() ?: 60L,
                debounceSec = debounce.toLongOrNull() ?: 3L,
                oauthCallbackPort = oauthPort.toIntOrNull() ?: 17890,
            )
            errors = onSave(config)
        })
    }
}

// === LogViewerScreen ===
@Composable
fun LogViewerScreen(records: List<LogRecordDisplay>) {
    var filterLevel by remember { mutableStateOf<LogLevel?>(null) }
    val levels = listOf(null, LogLevel.ERROR, LogLevel.WARN, LogLevel.INFO, LogLevel.DEBUG)

    Column(modifier = Modifier.fillMaxSize().padding(16.dp)) {
        Text("日志查看", fontSize = 18.sp, fontWeight = FontWeight.Bold)
        Spacer(Modifier.height(8.dp))

        // 级别过滤栏
        Row {
            levels.forEach { level ->
                TextButton(onClick = { filterLevel = level }) {
                    Text(
                        level?.name ?: "全部",
                        color = if (filterLevel == level) BrandColor else Color.Gray,
                        fontSize = 12.sp,
                    )
                }
            }
        }
        Divider()

        val filtered = if (filterLevel != null) {
            records.filter { it.level.severity >= filterLevel!!.severity }
        } else records

        Column(modifier = Modifier.verticalScroll(rememberScrollState())) {
            filtered.forEach { record ->
                Row(modifier = Modifier.fillMaxWidth().padding(2.dp)) {
                    Text(record.level.name, fontSize = 10.sp, color = colorForLevel(record.level), modifier = Modifier.width(60.dp))
                    Text("[${record.target}] ${record.message}", fontSize = 10.sp, color = Color(0xFF444444))
                }
            }
        }
    }
}

data class LogRecordDisplay(
    val timestampMs: Long,
    val level: LogLevel,
    val target: String,
    val message: String,
)

private fun colorForLevel(level: LogLevel): Color = when (level) {
    LogLevel.ERROR -> ErrorColor
    LogLevel.WARN -> WarningColor
    LogLevel.INFO -> SuccessColor
    LogLevel.DEBUG -> Color.Gray
    LogLevel.TRACE -> Color.LightGray
}

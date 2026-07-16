package io.github.yuanbaobaao.petallink

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material.Surface
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.window.Window
import androidx.compose.ui.window.application
import io.github.yuanbaobaao.petallink.config.UserConfig
import io.github.yuanbaobaao.petallink.ui.components.MateFileItemData
import io.github.yuanbaobaao.petallink.ui.components.MateTransferItemData
import io.github.yuanbaobaao.petallink.ui.pages.LoginScreen
import io.github.yuanbaobaao.petallink.ui.pages.MainScreen
import io.github.yuanbaobaao.petallink.ui.pages.SettingsScreen
import io.github.yuanbaobaao.petallink.ui.theme.PetalLinkTheme

fun main() = application {
    // 手动路由状态
    var currentPage by remember { mutableStateOf("login") }  // login / main / settings
    var isLoggedIn by remember { mutableStateOf(false) }
    var syncStatus by remember { mutableStateOf("空闲") }
    var isOnline by remember { mutableStateOf(true) }

    // 示例数据
    val fileItems = remember {
        listOf(
            MateFileItemData("文档.pdf", "2.5 MB", "2026-07-15", false, "☁️"),
            MateFileItemData("照片.jpg", "4.8 MB", "2026-07-14", false, "✅"),
        )
    }
    val transferItems = remember { emptyList<MateTransferItemData>() }

    Window(
        title = "PetalLink",
        onCloseRequest = ::exitApplication,
    ) {
        PetalLinkTheme {
            Surface(modifier = Modifier.fillMaxSize()) {
                when {
                    !isLoggedIn || currentPage == "login" -> LoginScreen(
                        onLogin = {
                            isLoggedIn = true
                            currentPage = "main"
                            syncStatus = "同步中"
                        },
                    )
                    currentPage == "settings" -> SettingsScreen(
                        onSave = { config -> io.github.yuanbaobaao.petallink.config.ConfigValidator.validate(config) },
                    )
                    else -> MainScreen(
                        syncStatus = syncStatus,
                        isOnline = isOnline,
                        fileItems = fileItems,
                        transferItems = transferItems,
                        onRefresh = { syncStatus = "已刷新" },
                    )
                }
            }
        }
    }
}

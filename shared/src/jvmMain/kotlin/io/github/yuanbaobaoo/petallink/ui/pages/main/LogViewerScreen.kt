@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.core.logging.LogLevel
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButton
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButtonVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateCircularProgress
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateEmpty
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateTag
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateTagSize
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateTagTheme
import io.github.yuanbaobaoo.petallink.ui.theme.ErrorColor
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import io.github.yuanbaobaoo.petallink.ui.theme.WarningColor

/**
 * 日志记录 UI 模型（对标原项目 logs_list 返回的 LogRecord）。
 *
 * 从领域层 LogRecord 转换；newest-first，最多 1000 条。
 */
data class LogRecordDisplay(
    val timestampMs: Long,
    val level: LogLevel,
    val target: String,
    val message: String,
)

/** 级别 → tag 主题映射（对标原 Vue tagTheme）。 */
private fun levelTheme(level: LogLevel): MateTagTheme = when (level) {
    LogLevel.ERROR -> MateTagTheme.ERROR
    LogLevel.WARN -> MateTagTheme.WARNING
    LogLevel.INFO -> MateTagTheme.PRIMARY
    else -> MateTagTheme.DEFAULT
}

/** 级别过滤选项。 */
private enum class LevelFilter { ALL, INFO, WARN, ERROR }

/**
 * 日志查看页（对标原 Vue LogViewerPage.vue）。
 *
 * toolbar：4 个级别 Tag(ALL/INFO/WARN/ERROR，选中上色) + 导出/清空 icon 按钮；
 * body(scroll)：每条 Tag(small，按 level 上色) + content(msg primary + meta mono secondary)。
 *
 * @param records 日志记录（newest-first）
 * @param inline 是否内嵌模式（嵌入设置页，不渲染独立 AppBar）
 * @param loading 加载中
 * @param onExport 导出
 * @param onClear 清空
 * @param onBack 返回（独立模式）
 */
@Composable
fun LogViewerScreen(
    records: List<LogRecordDisplay>,
    inline: Boolean = false,
    loading: Boolean = false,
    onExport: () -> Unit = {},
    onClear: () -> Unit = {},
    onBack: () -> Unit = {},
) {
    val semantic = LocalSemanticColors.current
    var filter by remember { mutableStateOf(LevelFilter.ALL) }
    val filtered = if (filter == LevelFilter.ALL) records else records.filter { record ->
        when (filter) {
            LevelFilter.ERROR -> record.level == LogLevel.ERROR
            LevelFilter.WARN -> record.level == LogLevel.WARN
            LevelFilter.INFO -> record.level == LogLevel.INFO
            else -> true
        }
    }

    Column(
        modifier = Modifier.fillMaxSize().background(if (inline) semantic.bgContainer else semantic.bgPage),
    ) {
        if (!inline) {
            // 独立 AppBar
            Row(
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                MateButton(variant = MateButtonVariant.ICON, icon = "arrow", onClick = onBack)
                Text("同步日志", fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
            }
        }
        // 工具栏
        Row(
            modifier = Modifier.fillMaxWidth().padding(horizontal = if (inline) 0.dp else 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            LevelFilter.values().forEach { lv ->
                val active = filter == lv
                val theme = if (active) when (lv) {
                    LevelFilter.ERROR -> MateTagTheme.ERROR
                    LevelFilter.WARN -> MateTagTheme.WARNING
                    LevelFilter.INFO -> MateTagTheme.PRIMARY
                    LevelFilter.ALL -> MateTagTheme.DEFAULT
                } else MateTagTheme.DEFAULT
                MateTag(
                    label = lv.name,
                    theme = theme,
                    onClick = { filter = lv },
                )
            }
            androidx.compose.foundation.layout.Spacer(Modifier.weight(1f))
            MateButton(variant = MateButtonVariant.ICON, icon = "download", onClick = onExport, disabled = loading)
            MateButton(variant = MateButtonVariant.ICON, icon = "trash", onClick = onClear, disabled = loading)
        }
        // 列表
        if (loading) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { MateCircularProgress(size = 24.dp) }
        } else if (filtered.isEmpty()) {
            MateEmpty(title = "暂无日志", icon = "list")
        } else {
            LazyColumn(modifier = Modifier.fillMaxSize().padding(horizontal = if (inline) 0.dp else 12.dp)) {
                itemsIndexed(filtered) { _, record ->
                    Row(
                        modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp),
                        verticalAlignment = Alignment.Top,
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        MateTag(label = record.level.name, theme = levelTheme(record.level), size = MateTagSize.SMALL)
                        Column {
                            Text(record.message, fontSize = 14.sp, color = semantic.textPrimary)
                            Text(
                                "[${record.target}]",
                                fontSize = 12.sp,
                                color = semantic.textSecondary,
                                modifier = Modifier.padding(top = 2.dp),
                            )
                        }
                    }
                }
            }
        }
    }
}

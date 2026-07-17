@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
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
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.core.logging.LogLevel
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButton
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButtonVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateCircularProgress
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateHDivider
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateEmpty
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateTag
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateTagSize
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateTagTheme
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

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
 * 日志查看页（对标原 Vue LogViewerPage.vue，视觉对标 v2 原型 07-logs.html）。
 *
 * v2：toolbar 为 4 个级别过滤 MateTag(ALL/INFO/WARN/ERROR，选中上色) + 导出/清空 ICON 按钮；
 * 日志列表包白色 panel（bgContainer radius-lg(10) + 0.5dp 细边，独立模式外边距 0/20/20），
 * 每条 MateTag(SMALL，按 level 上色) + msg(14.5sp textPrimary) + meta(12.5sp mono textSecondary)。
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
            // 独立 AppBar（56px + 底分隔线 + 返回箭头 rotate 180）
            Column {
                Row(
                    modifier = Modifier.fillMaxWidth().height(56.dp).padding(horizontal = 16.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    MateButton(variant = MateButtonVariant.ICON, icon = "arrow", onClick = onBack,
                        modifier = Modifier.rotate(180f))
                    Text("同步日志", fontSize = 17.sp, fontWeight = FontWeight.SemiBold)
                }
                MateHDivider()
            }
        }
        // 工具栏（v2 log-toolbar：padding 14/20，级别过滤 chip + 右侧导出/清空）
        Row(
            modifier = Modifier.fillMaxWidth().padding(horizontal = if (inline) 0.dp else 20.dp, vertical = 14.dp),
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
            Spacer(Modifier.weight(1f))
            MateButton(variant = MateButtonVariant.ICON, icon = "download", onClick = onExport, disabled = loading)
            MateButton(variant = MateButtonVariant.ICON, icon = "trash", onClick = onClear, disabled = loading)
        }
        // 列表（v2 log-list：白色 panel，bgContainer radius-lg(10) + 0.5dp 细边，独立模式外边距 0/20/20）
        if (loading) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { MateCircularProgress(size = 24.dp) }
        } else if (filtered.isEmpty()) {
            MateEmpty(title = "暂无日志", icon = "list")
        } else {
            LazyColumn(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(
                        start = if (inline) 0.dp else 20.dp,
                        end = if (inline) 0.dp else 20.dp,
                        bottom = if (inline) 0.dp else 20.dp,
                    )
                    .clip(RoundedCornerShape(10.dp))
                    .background(semantic.bgContainer)
                    .border(0.5.dp, semantic.border, RoundedCornerShape(10.dp)),
            ) {
                itemsIndexed(filtered) { index, record ->
                    Column {
                    // v2 log-item：padding 12/16
                    Row(
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
                        verticalAlignment = Alignment.Top,
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        MateTag(label = record.level.name, theme = levelTheme(record.level), size = MateTagSize.SMALL)
                        Column {
                            Text(record.message, fontSize = 14.5.sp, color = semantic.textPrimary)
                            // meta：时间 · logger（对标原 Vue fmtTime(time_ms) · logger_name，v2 等宽字体）
                            val timeStr = remember(record.timestampMs) {
                                SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.getDefault()).format(Date(record.timestampMs))
                            }
                            Text(
                                "$timeStr · ${record.target}",
                                fontSize = 12.5.sp,
                                fontFamily = FontFamily.Monospace,
                                color = semantic.textSecondary,
                                modifier = Modifier.padding(top = 3.dp),
                            )
                        }
                    }
                    // log-item 底分隔线 0.5px（末条无，对标 v2 :last-child）
                    if (index < filtered.lastIndex) MateHDivider()
                    }
                }
            }
        }
    }
}

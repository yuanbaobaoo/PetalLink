@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.components.mate

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.wrapContentSize
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.yuanbaobaoo.petallink.ui.components.MateIcon
import io.github.yuanbaobaoo.petallink.ui.components.MateIcons
import io.github.yuanbaobaoo.petallink.ui.theme.LOCAL_SEMANTIC_COLORS
import io.github.yuanbaobaoo.petallink.ui.theme.PetalTheme

/**
 * 组件预览页（开发期工具，对标原 Vue 的组件 storybook）。
 *
 * 用 `./kotlin run` 可查看全部 Mate 组件的渲染效果。
 * 不参与正式路由，仅作开发期校验。
 */
@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
fun MateCatalog() {
    val semantic = LOCAL_SEMANTIC_COLORS.current
    var switchChecked by remember { mutableStateOf(false) }
    var checkboxState by remember { mutableStateOf<Boolean?>(false) }
    var stepperValue by remember { mutableStateOf(6) }
    var showDialog by remember { mutableStateOf(false) }
    val scroll = rememberScrollState()

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(semantic.bgPage)
            .verticalScroll(scroll)
            .padding(PetalTheme.metrics.catalog.pagePadding),
        verticalArrangement = Arrangement.spacedBy(PetalTheme.metrics.catalog.sectionSpacing),
    ) {
        // 图标全集
        MateSectionHeader(text = "图标系统（32 个）", icon = "list")
        FlowRow(horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.catalog.itemSpacing)) {
            MateIcons.NAMES.forEach { name ->
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    MateIcon(name = name, size = PetalTheme.metrics.catalog.iconPreviewSize, tint = semantic.textPrimary)
                    Text(name, style = PetalTheme.typography.catalog.iconName, color = semantic.textSecondary)
                }
            }
        }

        // 按钮
        MateSectionHeader(text = "按钮", icon = "edit")
        Row(horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.catalog.itemSpacing), verticalAlignment = Alignment.CenterVertically) {
            MateButton(label = "主按钮", onClick = {})
            MateButton(label = "危险", danger = true, onClick = {})
            MateButton(label = "加载中", loading = true, onClick = {})
            MateButton(label = "禁用", disabled = true, onClick = {})
            MateButton(label = "文字按钮", variant = MateButtonVariant.TEXT, onClick = {})
            MateButton(variant = MateButtonVariant.ICON, icon = "settings", onClick = {})
            MateButton(label = "图标文字", variant = MateButtonVariant.ICON_TEXT, icon = "refresh", onClick = {})
            MateButton(variant = MateButtonVariant.ICON, icon = "transfer", badge = 5, onClick = {})
        }

        // 表单
        MateSectionHeader(text = "表单", icon = "file-text")
        Row(horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.catalog.itemSpacing), verticalAlignment = Alignment.CenterVertically) {
            MateStepper(value = stepperValue, onValueChange = { stepperValue = it }, min = 1, max = 20)
            MateSwitch(checked = switchChecked, onCheckedChange = { switchChecked = it })
            MateCheckbox(checked = checkboxState, onCheckedChange = { checkboxState = it })
            var text by remember { mutableStateOf("") }
            MateTextField(
                value = text,
                onValueChange = { text = it },
                placeholder = "输入文本...",
                prefixIcon = "search",
                modifier = Modifier.width(PetalTheme.metrics.catalog.fieldPreviewWidth),
            )
        }

        // 进度
        MateSectionHeader(text = "进度", icon = "sync")
        Row(horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.catalog.itemSpacing), verticalAlignment = Alignment.CenterVertically) {
            MateLinearProgress(value = 0.6f, modifier = Modifier.width(PetalTheme.metrics.catalog.progressPreviewWidth))
            MateLinearProgress(modifier = Modifier.width(PetalTheme.metrics.catalog.progressPreviewWidth))
            MateCircularProgress(size = PetalTheme.metrics.catalog.circularProgressSize, value = 0.7f)
            MateCircularProgress(size = PetalTheme.metrics.catalog.circularProgressSize)
        }

        // 横幅
        MateSectionHeader(text = "横幅", icon = "info")
        Column(verticalArrangement = Arrangement.spacedBy(PetalTheme.metrics.catalog.verticalGroupSpacing)) {
            MateInfoBanner(message = "这是信息横幅（info）", variant = MateBannerVariant.INFO)
            MateInfoBanner(message = "操作成功", variant = MateBannerVariant.SUCCESS)
            MateInfoBanner(message = "请检查输入", variant = MateBannerVariant.WARNING)
            MateInfoBanner(message = "发生错误", variant = MateBannerVariant.ERROR)
        }

        // 标签
        MateSectionHeader(text = "标签", icon = "check")
        Row(horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.catalog.compactItemSpacing)) {
            MateTag("默认", theme = MateTagTheme.DEFAULT)
            MateTag("主要", theme = MateTagTheme.PRIMARY)
            MateTag("成功", theme = MateTagTheme.SUCCESS)
            MateTag("警告", theme = MateTagTheme.WARNING)
            MateTag("错误", theme = MateTagTheme.ERROR)
            MateTag("小标签", size = MateTagSize.SMALL, icon = "sync")
        }

        // 导航
        MateSectionHeader(text = "导航项", icon = "settings")
        Column(modifier = Modifier.width(PetalTheme.metrics.catalog.dialogPreviewWidth)) {
            MateNavItem(label = "同步目录", icon = "folder", active = true, onClick = {})
            MateNavItem(label = "传输设置", icon = "transfer", onClick = {})
            MateNavItem(label = "高级设置", icon = "settings", onClick = {}, indent = 1)
        }

        // 对话框/Toast 触发
        MateSectionHeader(text = "弹层", icon = "alert")
        Row(horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.catalog.itemSpacing)) {
            MateButton(
                label = "打开对话框",
                onClick = {
                    confirmDialog(
                        MateDialogOptions(
                            title = "确认操作",
                            titleIcon = "alert",
                            content = "这是一个确认对话框示例。",
                        ),
                    ) { /* resolver */ }
                },
            )
            MateButton(label = "显示 Toast", variant = MateButtonVariant.TEXT, onClick = {
                showToast("操作已完成", MateToastVariant.SUCCESS)
            })
        }
        // 宿主
        MateDialogHost()
        MateToastHost()
    }
}

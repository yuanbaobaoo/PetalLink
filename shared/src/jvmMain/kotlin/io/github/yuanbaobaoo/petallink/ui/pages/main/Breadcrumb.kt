@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.hoverable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateHDivider
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import io.github.yuanbaobaoo.petallink.ui.viewmodel.BrowserBreadcrumb

/**
 * 面包屑导航（视觉对标 design/v2/02-main.html 的 .breadcrumb）。
 *
 * v2：高 40px，横向 scroll（超宽不换行），padding 0/20，gap 6；底部 MateHDivider 分隔线保留。
 * 分隔符 `›`（13sp placeholder 灰）；普通段 14sp secondary 可点 hover→BrandColor；
 * 当前段 14sp primary + semibold + 不可点。
 *
 * @param crumbs 路径栈（最后一个为当前目录）
 * @param onNavigate 点击非末级段跳转
 */
@Composable
fun Breadcrumb(
    crumbs: List<BrowserBreadcrumb>,
    onNavigate: (BrowserBreadcrumb) -> Unit,
) {
    val semantic = LocalSemanticColors.current
    val scroll = rememberScrollState()
    Column {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .height(40.dp)
                .background(semantic.bgContainer)
                .horizontalScroll(scroll)
                .padding(horizontal = 20.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            crumbs.forEachIndexed { index, crumb ->
                if (index > 0) {
                    Text("›", fontSize = 13.sp, color = semantic.textPlaceholder)
                }
                val isCurrent = index == crumbs.lastIndex
                // hover 变 brand 色（对标 .breadcrumb__item:hover）
                val interactionSource = remember { MutableInteractionSource() }
                val hovered by interactionSource.collectIsHoveredAsState()
                val color = when {
                    isCurrent -> semantic.textPrimary
                    hovered -> BrandColor
                    else -> semantic.textSecondary
                }
                Text(
                    crumb.name,
                    fontSize = 14.sp,
                    color = color,
                    fontWeight = if (isCurrent) FontWeight.SemiBold else FontWeight.Normal,
                    modifier = Modifier
                        .hoverable(interactionSource = interactionSource, enabled = !isCurrent)
                        .then(
                            if (isCurrent) Modifier
                            else Modifier.clickable(
                                interactionSource = interactionSource,
                                indication = null,
                            ) { onNavigate(crumb) },
                        ),
                )
            }
        }
        // 底部分隔线（v2 保留 MateHDivider）
        MateHDivider()
    }
}

/** 面包屑底部分隔线（保留兼容，实际已内嵌在 Breadcrumb 内）。 */
@Composable
fun BreadcrumbDivider() {
    MateHDivider()
}

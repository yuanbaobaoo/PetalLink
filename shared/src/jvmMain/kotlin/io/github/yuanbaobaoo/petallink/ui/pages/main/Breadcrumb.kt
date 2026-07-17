@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors
import io.github.yuanbaobaoo.petallink.ui.viewmodel.BrowserBreadcrumb

/**
 * 面包屑导航（对标原 Vue Breadcrumb.vue）。
 *
 * 高 32px，横向 scroll（超宽不换行），padding 0/16，gap 4；底 0.5px border，白底。
 * 分隔符 `›`（11px placeholder 灰）；普通段 13px secondary 可点 hover→brand；当前段 primary + medium + 不可点。
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
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(32.dp)
            .background(semantic.bgContainer)
            .horizontalScroll(scroll)
            .padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        crumbs.forEachIndexed { index, crumb ->
            if (index > 0) {
                Text("›", fontSize = 11.sp, color = semantic.textPlaceholder)
            }
            val isCurrent = index == crumbs.lastIndex
            Text(
                crumb.name,
                fontSize = 13.sp,
                color = if (isCurrent) semantic.textPrimary else semantic.textSecondary,
                fontWeight = if (isCurrent) FontWeight.Medium else FontWeight.Normal,
                modifier = Modifier.then(
                    if (isCurrent) Modifier
                    else Modifier.clickable(
                        interactionSource = remember { MutableInteractionSource() },
                        indication = null,
                    ) { onNavigate(crumb) },
                ),
            )
        }
    }
}

/** 面包屑底部 0.5px 分隔线（对标 .breadcrumb border-bottom）。 */
@Composable
fun BreadcrumbDivider() {
    val semantic = LocalSemanticColors.current
    androidx.compose.foundation.layout.Box(
        Modifier.fillMaxWidth().height(0.5.dp).background(semantic.border),
    )
}

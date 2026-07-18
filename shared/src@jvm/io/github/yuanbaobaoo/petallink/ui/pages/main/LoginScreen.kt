@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateAppLogo
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateBannerVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButton
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButtonVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateCircularProgress
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateInfoBanner
import io.github.yuanbaobaoo.petallink.ui.theme.BrandColor
import io.github.yuanbaobaoo.petallink.ui.theme.BrandGradient
import io.github.yuanbaobaoo.petallink.ui.theme.BrandLighter
import io.github.yuanbaobaoo.petallink.ui.theme.LocalSemanticColors

private const val APP_TITLE = "PetalLink - 华为云盘客户端开源版"
private const val SECRET_WARNING =
    "尚未配置 OAuth 凭据。请在项目根目录的 .env 中同时配置：\n" +
        "HWCLOUD_CLIENT_ID=<你的 client_id>\nHWCLOUD_CLIENT_SECRET=<你的 client_secret>"
private const val HINT = "点击后将打开浏览器，支持账号密码或手机扫码登录"

/**
 * 登录页（v2 视觉，对标 design/v2/01-login.html）。
 *
 * 布局：蓝色系渐变背景（135deg BrandLighter→bgPage→bgContainer）+ 3 个 BrandColor 低透明度装饰圆
 * + 居中白卡片（width 480，radius 12，24dp 柔影，padding 32/24）。
 * 卡片内：MateAppLogo container(64×64 真实 logo) + 标题 + 品牌渐变分隔线(40×2)
 * + InfoBanner(secret/error) + 主按钮(全宽 46，渐变与柔影由 MateButton 自带) + hint。
 * 授权中：BrandLighter 面板(radius 8, h40) + spinner + 提示 + 取消按钮。
 *
 * @param loggingIn 是否正在授权
 * @param secretConfigured client_id 与 client_secret 是否均已配置
 * @param errorMessage 错误消息（null 时无 error banner）
 * @param onLogin 登录回调
 * @param onCancel 取消授权回调
 * @param onDismissError 关闭错误（重新授权）回调
 */
@Composable
fun LoginScreen(
    loggingIn: Boolean,
    secretConfigured: Boolean,
    errorMessage: String?,
    onLogin: () -> Unit,
    onCancel: () -> Unit,
    onDismissError: () -> Unit = {},
) {
    val showAuthorizing = loggingIn
    val semantic = LocalSemanticColors.current
    Box(
        modifier = Modifier
            .fillMaxSize()
            // v2：135deg 蓝色系渐变（#EFF4FE → #F6F6F8 → #FFFFFF，映射到 token）
            .background(
                Brush.linearGradient(
                    colors = listOf(BrandLighter, semantic.bgPage, semantic.bgContainer),
                ),
            ),
        contentAlignment = Alignment.Center,
    ) {
        // 装饰圆（v2：BrandColor 低透明度，用 align 模拟 CSS top/right/bottom/left 定位）
        // lg: 400×400, top -100, right -80, opacity 0.06（贴右上角溢出）
        Box(
            Modifier.align(Alignment.TopEnd).offset(x = 80.dp, y = (-100).dp)
                .width(400.dp).height(400.dp).alpha(0.06f)
                .clip(CircleShape).background(BrandColor),
        )
        // md: 300×300, bottom -60, left -80, opacity 0.06（贴左下角溢出）
        Box(
            Modifier.align(Alignment.BottomStart).offset(x = (-80).dp, y = 60.dp)
                .width(300.dp).height(300.dp).alpha(0.06f)
                .clip(CircleShape).background(BrandColor),
        )
        // sm: 200×200, top 45%, left 30%, opacity 0.05（居中偏左）
        Box(
            Modifier.align(Alignment.Center).offset(x = (-160).dp, y = (-40).dp)
                .width(200.dp).height(200.dp).alpha(0.05f)
                .clip(CircleShape).background(BrandColor),
        )

        // 居中卡片（v2：白底 width 480，radius 12 + 24dp 柔影，padding 32/24）
        Column(
            modifier = Modifier
                .width(480.dp)
                .shadow(24.dp, RoundedCornerShape(12.dp))
                .background(semantic.bgContainer, RoundedCornerShape(12.dp))
                .padding(horizontal = 24.dp, vertical = 32.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            // 品牌容器图标（64×64 真实 logo）
            MateAppLogo(container = true, text = "")
            Spacer(Modifier.height(12.dp))
            // 标题（21sp semibold，原型 20px，letter-spacing -0.2px）
            Text(
                APP_TITLE,
                fontSize = 21.sp,
                fontWeight = FontWeight.SemiBold,
                color = semantic.textPrimary,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(4.dp))
            // 品牌分隔线（40×2，v2 改用 BrandGradient 渐变，radius 1）
            Box(
                Modifier.width(40.dp).height(2.dp)
                    .clip(RoundedCornerShape(1.dp))
                    .background(BrandGradient),
            )
            Spacer(Modifier.height(12.dp))
            // secret 未配置警告
            if (!secretConfigured) {
                MateInfoBanner(
                    message = SECRET_WARNING,
                    variant = MateBannerVariant.WARNING,
                )
                Spacer(Modifier.height(12.dp))
            }
            // 错误 banner（带重新授权按钮）
            if (errorMessage != null) {
                MateInfoBanner(
                    message = errorMessage,
                    variant = MateBannerVariant.ERROR,
                    action = { MateButton(label = "重新授权", variant = MateButtonVariant.TEXT, onClick = onDismissError) },
                )
                Spacer(Modifier.height(12.dp))
            }
            Spacer(Modifier.height(24.dp))
            // 主按钮区
            if (showAuthorizing) {
                // 授权中面板（v2：BrandLighter 底，radius 8，h40）：spinner + 提示 + 取消按钮
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Box(
                        modifier = Modifier.fillMaxWidth().height(40.dp)
                            .clip(RoundedCornerShape(8.dp)).background(BrandLighter)
                            .padding(horizontal = 16.dp),
                        contentAlignment = Alignment.Center,
                    ) {
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            MateCircularProgress(size = 16.dp, strokeWidth = 2.dp)
                            Text(
                                "请在浏览器中完成授权...",
                                fontSize = 15.sp,
                                fontWeight = FontWeight.Medium,
                                color = BrandColor,
                            )
                        }
                    }
                    Spacer(Modifier.height(8.dp))
                    MateButton(
                        label = "取消授权",
                        variant = MateButtonVariant.TEXT,
                        icon = "x",
                        onClick = onCancel,
                    )
                }
            } else {
                // 登录按钮（v2：全宽 46px 高，cloud 图标，渐变与柔影由组件自带）
                MateButton(
                    label = "使用华为账号登录",
                    variant = MateButtonVariant.PRIMARY,
                    icon = "cloud",
                    onClick = onLogin,
                    fullWidth = true,
                    height = 46.dp,
                    disabled = !secretConfigured,
                )
            }
            Spacer(Modifier.height(12.dp))
            // 底部说明
            Text(
                HINT,
                fontSize = 13.sp,
                color = semantic.textSecondary,
                textAlign = TextAlign.Center,
            )
        }
    }
}

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
import androidx.compose.foundation.layout.size
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
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateAppLogo
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateBannerVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButton
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateButtonVariant
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateCircularProgress
import io.github.yuanbaobaoo.petallink.ui.components.mate.MateInfoBanner
import io.github.yuanbaobaoo.petallink.ui.theme.LOCAL_SEMANTIC_COLORS
import io.github.yuanbaobaoo.petallink.ui.theme.PetalTheme

private const val APP_TITLE = "PetalLink - 华为云盘客户端开源版"
private const val SECRET_WARNING =
    "尚未配置 OAuth 凭据。请在项目根目录的 .env 中同时配置：\n" +
        "HWCLOUD_CLIENT_ID=<你的 client_id>\nHWCLOUD_CLIENT_SECRET=<你的 client_secret>"
private const val HINT = "点击后将打开浏览器，支持账号密码或手机扫码登录"

/**
 * 登录页（v2 视觉，对标 design/v2/01-login.html）。
 *
 * 布局：蓝色系渐变背景（135deg PetalTheme.colors.brandLighter→bgPage→bgContainer）+ 3 个 PetalTheme.colors.brand 低透明度装饰圆
 * + 居中白卡片（width 480，radius 12，24dp 柔影，padding 32/24）。
 * 卡片内：MateAppLogo container(64×64 真实 logo) + 标题 + 品牌渐变分隔线(40×2)
 * + InfoBanner(secret/error) + 主按钮(全宽 46，渐变与柔影由 MateButton 自带) + hint。
 * 授权中：PetalTheme.colors.brandLighter 面板(radius 8, h40) + spinner + 提示 + 取消按钮。
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
    val semantic = LOCAL_SEMANTIC_COLORS.current
    Box(
        modifier = Modifier
            .fillMaxSize()
            // v2：135deg 蓝色系渐变（#EFF4FE → #F6F6F8 → #FFFFFF，映射到 token）
            .background(
                Brush.linearGradient(
                    colors = listOf(PetalTheme.colors.brandLighter, semantic.bgPage, semantic.bgContainer),
                ),
            ),
        contentAlignment = Alignment.Center,
    ) {
        // 装饰圆（v2：PetalTheme.colors.brand 低透明度，用 align 模拟 CSS top/right/bottom/left 定位）
        // lg: 400×400, top -100, right -80, opacity 0.06（贴右上角溢出）
        Box(
            Modifier.align(Alignment.TopEnd).offset(x = PetalTheme.metrics.login.topDecorationOffsetX, y = PetalTheme.metrics.login.topDecorationOffsetY)
                .size(PetalTheme.metrics.login.topDecorationSize).alpha(PetalTheme.metrics.login.topDecorationAlpha)
                .clip(CircleShape).background(PetalTheme.colors.brand),
        )
        // md: 300×300, bottom -60, left -80, opacity 0.06（贴左下角溢出）
        Box(
            Modifier.align(Alignment.BottomStart).offset(x = PetalTheme.metrics.login.bottomDecorationOffsetX, y = PetalTheme.metrics.login.bottomDecorationOffsetY)
                .size(PetalTheme.metrics.login.bottomDecorationSize).alpha(PetalTheme.metrics.login.bottomDecorationAlpha)
                .clip(CircleShape).background(PetalTheme.colors.brand),
        )
        // sm: 200×200, top 45%, left 30%, opacity 0.05（居中偏左）
        Box(
            Modifier.align(Alignment.Center).offset(x = (-160).dp, y = (-40).dp)
                .size(PetalTheme.metrics.login.centerDecorationSize).alpha(PetalTheme.metrics.login.centerDecorationAlpha)
                .clip(CircleShape).background(PetalTheme.colors.brand),
        )

        // 居中卡片（v2：白底 width 480，radius 12 + 24dp 柔影，padding 32/24）
        Column(
            modifier = Modifier
                .width(PetalTheme.metrics.login.cardWidth)
                .shadow(PetalTheme.metrics.login.cardShadowElevation, RoundedCornerShape(PetalTheme.metrics.login.cardRadius))
                .background(semantic.bgContainer, RoundedCornerShape(PetalTheme.metrics.login.cardRadius))
                .padding(horizontal = PetalTheme.metrics.login.cardHorizontalPadding, vertical = PetalTheme.metrics.login.cardVerticalPadding),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            // 品牌容器图标（64×64 真实 logo）
            MateAppLogo(container = true, text = "")
            Spacer(Modifier.height(PetalTheme.metrics.login.logoTitleSpacing))
            // 标题（21sp semibold，原型 20px，letter-spacing -0.2px）
            Text(
                APP_TITLE,
                style = PetalTheme.typography.login.title,
                color = semantic.textPrimary,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(PetalTheme.metrics.login.subtitleSpacing))
            // 品牌分隔线（40×2，v2 改用 PetalTheme.colors.brandGradient 渐变，radius 1）
            Box(
                Modifier.width(PetalTheme.metrics.login.accentWidth).height(PetalTheme.metrics.login.accentHeight)
                    .clip(RoundedCornerShape(PetalTheme.metrics.login.accentRadius))
                    .background(PetalTheme.colors.brandGradient),
            )
            Spacer(Modifier.height(PetalTheme.metrics.login.accentBottomSpacing))
            // secret 未配置警告
            if (!secretConfigured) {
                MateInfoBanner(
                    message = SECRET_WARNING,
                    variant = MateBannerVariant.WARNING,
                )
                Spacer(Modifier.height(PetalTheme.metrics.login.messageSpacing))
            }
            // 错误 banner（带重新授权按钮）
            if (errorMessage != null) {
                MateInfoBanner(
                    message = errorMessage,
                    variant = MateBannerVariant.ERROR,
                    action = { MateButton(label = "重新授权", variant = MateButtonVariant.TEXT, onClick = onDismissError) },
                )
                Spacer(Modifier.height(PetalTheme.metrics.login.messageSpacing))
            }
            Spacer(Modifier.height(PetalTheme.metrics.login.contentBottomSpacing))
            // 主按钮区
            if (showAuthorizing) {
                // 授权中面板（v2：PetalTheme.colors.brandLighter 底，radius 8，h40）：spinner + 提示 + 取消按钮
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Box(
                        modifier = Modifier.fillMaxWidth().height(PetalTheme.metrics.login.authorizingHeight)
                            .clip(RoundedCornerShape(PetalTheme.metrics.login.authorizingRadius)).background(PetalTheme.colors.brandLighter)
                            .padding(horizontal = PetalTheme.metrics.login.authorizingHorizontalPadding),
                        contentAlignment = Alignment.Center,
                    ) {
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(PetalTheme.metrics.login.authorizingContentSpacing),
                        ) {
                            MateCircularProgress(size = PetalTheme.metrics.login.authorizingSpinnerSize, strokeWidth = PetalTheme.metrics.login.authorizingSpinnerStroke)
                            Text(
                                "请在浏览器中完成授权...",
                                style = PetalTheme.typography.login.authorizingMessage,
                                color = PetalTheme.colors.brand,
                            )
                        }
                    }
                    Spacer(Modifier.height(PetalTheme.metrics.login.errorActionSpacing))
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
                    height = PetalTheme.metrics.login.loginButtonHeight,
                    disabled = !secretConfigured,
                )
            }
            Spacer(Modifier.height(PetalTheme.metrics.login.footerSpacing))
            // 底部说明
            Text(
                HINT,
                style = PetalTheme.typography.login.footerHint,
                color = semantic.textSecondary,
                textAlign = TextAlign.Center,
            )
        }
    }
}

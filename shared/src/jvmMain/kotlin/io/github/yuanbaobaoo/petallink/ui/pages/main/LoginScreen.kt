@file:Suppress("FunctionName")

package io.github.yuanbaobaoo.petallink.ui.pages.main

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
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
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
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

private val LOGIN_BG_START = Color(0xFFEBF1FF)
private val LOGIN_BG_MID = Color(0xFFF5F5F5)
private val LOGIN_BG_END = Color(0xFFFFFFFF)

private const val APP_TITLE = "PetalLink - 华为云盘客户端开源版"
private const val SECRET_WARNING =
    "尚未配置 client_secret。请在项目根目录创建 .env 文件，写入：\nHWCLOUD_CLIENT_SECRET=<你的 64 位 hex>\n（参考 .env.example）"
private const val HINT = "点击后将打开浏览器，支持账号密码或手机扫码登录"

/**
 * 登录页（对标原 Vue LoginPage.vue）。
 *
 * 布局：渐变背景（135deg #EBF1FF→#F5F5F5→#FFFFFF）+ 3 个低透明度装饰圆 + 居中卡片（max-width 480）。
 * 卡片内：MateAppLogo container(64) + 标题 + 品牌蓝分隔线 + InfoBanner(secret/error) + 主按钮(全宽40px) + hint。
 * 授权中：spinner + 提示 + 取消按钮。
 *
 * @param loggingIn 是否正在授权
 * @param secretConfigured client_secret 是否已配置
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
    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(
                Brush.linearGradient(
                    colors = listOf(LOGIN_BG_START, LOGIN_BG_MID, LOGIN_BG_END),
                    start = androidx.compose.ui.geometry.Offset(0f, 0f),
                    end = androidx.compose.ui.geometry.Offset(Float.MAX_VALUE, Float.MAX_VALUE),
                ),
            ),
        contentAlignment = Alignment.Center,
    ) {
        // 装饰圆（品牌色低透明度，用 align 模拟 CSS top/right/bottom/left 定位）
        // lg: 400×400, top -100, right -80, opacity 0.06（贴右上角溢出）
        Box(
            Modifier.align(Alignment.TopEnd).offset(x = 80.dp, y = (-100).dp)
                .width(400.dp).height(400.dp).alpha(0.06f)
                .clip(CircleShape).background(Color(0xFF0052D9)),
        )
        // md: 300×300, bottom -60, left -80, opacity 0.06（贴左下角溢出）
        Box(
            Modifier.align(Alignment.BottomStart).offset(x = (-80).dp, y = 60.dp)
                .width(300.dp).height(300.dp).alpha(0.06f)
                .clip(CircleShape).background(Color(0xFF0052D9)),
        )
        // sm: 200×200, top 45%, left 30%, opacity 0.04（居中偏左）
        Box(
            Modifier.align(Alignment.Center).offset(x = (-160).dp, y = (-40).dp)
                .width(200.dp).height(200.dp).alpha(0.04f)
                .clip(CircleShape).background(Color(0xFF0052D9)),
        )

        // 居中卡片（max-width 480dp，padding 32/24）
        Column(
            modifier = Modifier.width(480.dp).padding(horizontal = 24.dp, vertical = 32.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            // 品牌容器图标（64×64）
            MateAppLogo(container = true, text = "")
            Spacer(Modifier.height(12.dp))
            // 标题（20px semibold，letter-spacing -0.2px）
            Text(
                APP_TITLE,
                fontSize = 20.sp,
                fontWeight = FontWeight.SemiBold,
                color = Color(0xE6000000),
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(4.dp))
            // 品牌分隔线（40×2，brand，radius 1）
            Box(
                Modifier.width(40.dp).height(2.dp)
                    .clip(RoundedCornerShape(1.dp))
                    .background(Color(0xFF0052D9)),
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
                // 授权中面板：spinner + 提示 + 取消按钮
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Box(
                        modifier = Modifier.fillMaxWidth().height(40.dp)
                            .clip(RoundedCornerShape(3.dp)).background(Color(0xFFF2F3FF))
                            .padding(horizontal = 16.dp),
                        contentAlignment = Alignment.Center,
                    ) {
                        androidx.compose.foundation.layout.Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            MateCircularProgress(size = 16.dp, strokeWidth = 2.dp)
                            Text(
                                "请在浏览器中完成授权...",
                                fontSize = 14.sp,
                                fontWeight = FontWeight.Medium,
                                color = Color(0xFF0052D9),
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
                // 登录按钮（全宽，40px 高，cloud 图标）
                MateButton(
                    label = "使用华为账号登录",
                    variant = MateButtonVariant.PRIMARY,
                    icon = "cloud",
                    onClick = onLogin,
                    fullWidth = true,
                    height = 40.dp,
                    disabled = !secretConfigured,
                )
            }
            Spacer(Modifier.height(12.dp))
            // 底部说明
            Text(
                HINT,
                fontSize = 12.sp,
                color = Color(0x99000000),
                textAlign = TextAlign.Center,
            )
        }
    }
}

import 'package:flutter/material.dart';
import 'package:get/get.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/pages/login/controller/login_controller.dart';
import 'package:petal_link/widgets/index.dart';

/// 应用标题（对标 CMP LoginScreen.kt APP_TITLE）
const String _kAppTitle = 'PetalLink - 华为云盘客户端开源版';

/// OAuth 密钥未配置警告（对标 CMP SECRET_WARNING）
const String _kSecretWarning = '尚未配置 OAuth 凭据。请在项目根目录的 .env 中同时配置：\n'
    'HWCLOUD_CLIENT_ID=<你的 client_id>\n'
    'HWCLOUD_CLIENT_SECRET=<你的 client_secret>';

/// 底部说明（对标 CMP HINT）
const String _kHint = '点击后将打开浏览器，支持账号密码或手机扫码登录';

/// 登录页（对标 CMP LoginScreen.kt / design/v2/01-login.html）。
///
/// 布局：蓝色系渐变背景（brandLighter→bgPage→bgContainer）+ 3 个 brand
/// 低透明度装饰圆 + 居中白卡片（宽 480，radius 12，柔影）。
/// 卡片内：Logo(64×64) + 标题 + 品牌渐变分隔线 + secret/错误横幅
/// + 主按钮（全宽 46）/ 授权中面板（spinner + 提示 + 取消按钮）+ 底部说明。
class LoginPage extends StatefulWidget {
  const LoginPage({super.key});

  @override
  State<LoginPage> createState() => _LoginPageState();
}

class _LoginPageState extends State<LoginPage> {
  /// 页面控制器（xe-cloud 惯例：页面持有，dispose 时释放）
  final LoginController notifier = Get.put(LoginController());

  @override
  void dispose() {
    Get.delete<LoginController>();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).login;
    final typography = MateTheme.typographyOf(context).login;

    return Scaffold(
      body: Obx(() {
        final state = notifier.state.value;

        return Stack(
          children: [
            // 品牌渐变背景（135deg：brandLighter → bgPage → bgContainer）
            Positioned.fill(
              child: DecoratedBox(
                decoration: BoxDecoration(
                  gradient: LinearGradient(
                    colors: [
                      colors.brandLighter,
                      colors.bgPage,
                      colors.bgContainer,
                    ],
                    begin: Alignment.topLeft,
                    end: Alignment.bottomRight,
                  ),
                ),
              ),
            ),

            // 装饰圆 lg：贴右上角溢出（400×400, top -100, right -80, alpha 0.06）
            Positioned(
              top: metrics.topDecorationOffsetY,
              right: -metrics.topDecorationOffsetX,
              child: _DecorationCircle(
                size: metrics.topDecorationSize,
                color: colors.brand.withAlpha(
                  (metrics.topDecorationAlpha * 255).round(),
                ),
              ),
            ),

            // 装饰圆 md：贴左下角溢出（300×300, bottom -60, left -80, alpha 0.06）
            Positioned(
              bottom: -metrics.bottomDecorationOffsetY,
              left: metrics.bottomDecorationOffsetX,
              child: _DecorationCircle(
                size: metrics.bottomDecorationSize,
                color: colors.brand.withAlpha(
                  (metrics.bottomDecorationAlpha * 255).round(),
                ),
              ),
            ),

            // 装饰圆 sm：居中偏左（200×200, top 45%, left 30%, alpha 0.05；
            // 偏移量对标 CMP Center align + offset(-160, -40)，无 token 故保留常量）
            Align(
              alignment: Alignment.center,
              child: Transform.translate(
                offset: const Offset(-160, -40),
                child: _DecorationCircle(
                  size: metrics.centerDecorationSize,
                  color: colors.brand.withAlpha(
                    (metrics.centerDecorationAlpha * 255).round(),
                  ),
                ),
              ),
            ),

            // 居中白卡片
            Center(
              child: Container(
                width: metrics.cardWidth,
                padding: EdgeInsets.symmetric(
                  horizontal: metrics.cardHorizontalPadding,
                  vertical: metrics.cardVerticalPadding,
                ),
                decoration: BoxDecoration(
                  color: colors.bgContainer,
                  borderRadius: BorderRadius.circular(metrics.cardRadius),
                  boxShadow: [
                    BoxShadow(
                      // 柔影色派生自遮罩 token（与 MatePopupMenu 同一惯例）
                      color: colors.overlayDialogScrim.withAlpha(22),
                      blurRadius: metrics.cardShadowElevation,
                      offset: Offset(0, metrics.cardShadowElevation / 2),
                    ),
                  ],
                ),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    // 品牌容器图标（64×64 真实 logo）
                    const MateAppLogo(container: true, text: ''),
                    SizedBox(height: metrics.logoTitleSpacing),

                    // 标题
                    Text(
                      _kAppTitle,
                      style: typography.title.copyWith(
                        color: colors.textPrimary,
                      ),
                      textAlign: TextAlign.center,
                    ),
                    SizedBox(height: metrics.subtitleSpacing),

                    // 品牌渐变分隔线（40×2，radius 1）
                    Container(
                      width: metrics.accentWidth,
                      height: metrics.accentHeight,
                      decoration: BoxDecoration(
                        borderRadius:
                            BorderRadius.circular(metrics.accentRadius),
                        gradient:
                            LinearGradient(colors: colors.brandGradient),
                      ),
                    ),
                    SizedBox(height: metrics.accentBottomSpacing),

                    // secret 未配置警告
                    if (!state.secretConfigured) ...[
                      const MateInfoBanner(
                        message: _kSecretWarning,
                        variant: MateBannerVariant.warning,
                      ),
                      SizedBox(height: metrics.messageSpacing),
                    ],

                    // 错误横幅（带「重新授权」动作）
                    if (state.errorMessage != null) ...[
                      MateInfoBanner(
                        message: state.errorMessage!,
                        variant: MateBannerVariant.error,
                        action: MateButton(
                          label: '重新授权',
                          variant: MateButtonVariant.text,
                          onClick: notifier.dismissError,
                        ),
                      ),
                      SizedBox(height: metrics.messageSpacing),
                    ],

                    SizedBox(height: metrics.contentBottomSpacing),

                    // 主按钮区：授权中面板 / 登录按钮
                    if (state.isAuthorizing)
                      _AuthorizingPanel(onCancel: notifier.cancelLogin)
                    else
                      MateButton(
                        label: '使用华为账号登录',
                        variant: MateButtonVariant.primary,
                        icon: 'cloud',
                        onClick: notifier.login,
                        fullWidth: true,
                        height: metrics.loginButtonHeight,
                        disabled: !state.secretConfigured,
                      ),

                    SizedBox(height: metrics.footerSpacing),

                    // 底部说明
                    Text(
                      _kHint,
                      style: typography.footerHint.copyWith(
                        color: colors.textSecondary,
                      ),
                      textAlign: TextAlign.center,
                    ),
                  ],
                ),
              ),
            ),
          ],
        );
      }),
    );
  }
}

/// 装饰圆（brand 低透明度，配合渐变背景）
class _DecorationCircle extends StatelessWidget {
  final double size;
  final Color color;

  const _DecorationCircle({required this.size, required this.color});

  @override
  Widget build(BuildContext context) {
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(shape: BoxShape.circle, color: color),
    );
  }
}

/// 授权中面板：brandLighter 底（radius 8, h40）+ spinner + 提示 + 取消按钮。
class _AuthorizingPanel extends StatelessWidget {
  final VoidCallback onCancel;

  const _AuthorizingPanel({required this.onCancel});

  @override
  Widget build(BuildContext context) {
    final colors = MateTheme.colorsOf(context);
    final metrics = MateTheme.metricsOf(context).login;
    final typography = MateTheme.typographyOf(context).login;

    return Column(
      children: [
        Container(
          height: metrics.authorizingHeight,
          padding: EdgeInsets.symmetric(
            horizontal: metrics.authorizingHorizontalPadding,
          ),
          decoration: BoxDecoration(
            color: colors.brandLighter,
            borderRadius: BorderRadius.circular(metrics.authorizingRadius),
          ),
          child: Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              MateCircularProgress(
                size: metrics.authorizingSpinnerSize,
                strokeWidth: metrics.authorizingSpinnerStroke,
                color: colors.brand,
              ),
              SizedBox(width: metrics.authorizingContentSpacing),
              Text(
                '请在浏览器中完成授权...',
                style: typography.authorizingMessage.copyWith(
                  color: colors.brand,
                ),
              ),
            ],
          ),
        ),
        SizedBox(height: metrics.errorActionSpacing),
        MateButton(
          label: '取消授权',
          variant: MateButtonVariant.text,
          icon: 'x',
          onClick: onCancel,
        ),
      ],
    );
  }
}

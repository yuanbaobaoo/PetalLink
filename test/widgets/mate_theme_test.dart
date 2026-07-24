import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:petal_link/app/theme/mate_theme.dart';
import 'package:petal_link/app/theme/mate_tokens.dart';

// =============================================================================
// Mate 主题层测试：Token 完整性（关键色值 / 浅色语义解析）+ 访问入口。
// 注：已对齐 Tauri 移除深色主题（强制浅色）。
// 色值基准：CMP shared/src@jvm/.../ui/theme/DesignTokens.kt 与 Theme.kt。
// =============================================================================

void main() {
  group('MateColors 原始 token（对齐 CMP DesignTokens.kt）', () {
    test('品牌色', () {
      expect(MateColors.brand, const Color(0xFF0053DB));
      expect(MateColors.brandHover, const Color(0xFF4A8BF0));
      expect(MateColors.brandActive, const Color(0xFF0047B8));
      expect(MateColors.brandLight, const Color(0xFFB7D0F7));
      expect(MateColors.brand100, const Color(0xFFDCE8FC));
      expect(MateColors.brandLighter, const Color(0xFFEFF4FE));
    });

    test('功能色', () {
      expect(MateColors.success, const Color(0xFF0CA678));
      expect(MateColors.successBackground, const Color(0xFFE3F5EE));
      expect(MateColors.warning, const Color(0xFFF08C00));
      expect(MateColors.warningBackground, const Color(0xFFFFF3DE));
      expect(MateColors.error, const Color(0xFFE5484D));
      expect(MateColors.errorBackground, const Color(0xFFFDECEC));
      expect(MateColors.info, const Color(0xFF3B82F6));
      expect(MateColors.infoBackground, const Color(0xFFE8F0FE));
    });

    test('文件类型色', () {
      expect(MateColors.folder, const Color(0xFFF0A63C));
      expect(MateColors.document, const Color(0xFF6366F1));
      expect(MateColors.image, const Color(0xFFEC4899));
      expect(MateColors.video, const Color(0xFF8B5CF6));
      expect(MateColors.sheet, const Color(0xFF10B981));
    });

    test('浅色主题背景/边框/文字', () {
      expect(MateColors.lightBgPage, const Color(0xFFF5F5F7));
      expect(MateColors.lightBgContainer, const Color(0xFFFFFFFF));
      expect(MateColors.lightBgFill, const Color(0xFFF1F1F3));
      expect(MateColors.lightBgHover, const Color(0xFFF7F7F9));
      expect(MateColors.lightBgActive, const Color(0xFFECECEF));
      expect(MateColors.lightBorder, const Color(0x0F000000));
      expect(MateColors.lightTextPrimary, const Color(0xE6000000));
      expect(MateColors.lightTextSecondary, const Color(0x99000000));
      expect(MateColors.lightTextPlaceholder, const Color(0x59000000));
    });

  });

  group('MateSemanticColors 双主题解析（对齐 CMP Theme.kt）', () {
    test('浅色语义解析', () {
      const light = MateSemanticColors.light;
      expect(light.brand, const Color(0xFF0053DB));
      expect(light.brandHover, const Color(0xFF4A8BF0));
      expect(light.brandLight, const Color(0xFFB7D0F7));
      expect(light.bgPage, const Color(0xFFF5F5F7));
      expect(light.successBg, const Color(0xFFE3F5EE));
      expect(light.switchOffTrack, const Color(0xFFE3E3E6));
      // 品牌渐变 = brandHover → brand
      expect(light.brandGradient,
          [const Color(0xFF4A8BF0), const Color(0xFF0053DB)]);
      // 品牌浅色渐变 = brandLighter → brand100
      expect(light.brandGradientSoft,
          [const Color(0xFFEFF4FE), const Color(0xFFDCE8FC)]);
    });

    test('固定前景色（浅色，对齐 Tauri tokens.css）', () {
      const colors = MateSemanticColors.light;
      expect(colors.toastBackground, const Color(0xEB1C1C1E));
      expect(colors.toastSuccessIcon, const Color(0xFF4ADE80));
      expect(colors.toastWarningIcon, const Color(0xFFFBBF24));
      expect(colors.toastErrorIcon, const Color(0xFFFB7185));
      expect(colors.buttonPrimaryText, const Color(0xFFFFFFFF));
      // 遮罩对齐 Tauri rgba(28,28,30,0.36)
      expect(colors.overlayDialogScrim, const Color(0x5C1C1C1E));
      expect(colors.mainLoadingScrim, const Color(0x99FFFFFF));
      expect(colors.fileListBulkBackground, const Color(0xF01C1C1E));
      // 强悬停色（黑 0.04，对齐 Tauri rgba(0,0,0,0.04)）
      expect(colors.hoverStrong, const Color(0x0A000000));
    });
  });

  group('排版/尺寸 token 关键值', () {
    test('排版（对齐 CMP DesignTokens.TYPOGRAPHY）', () {
      final t = MateTypography.standard();
      expect(t.button.primaryLabel.fontSize, 12);
      expect(t.button.primaryLabel.fontWeight, FontWeight.w500);
      expect(t.button.softLabel.fontSize, 11);
      expect(t.dialog.title.fontSize, 17);
      expect(t.dialog.title.fontWeight, FontWeight.w600);
      expect(t.dialog.body.fontSize, 14);
      // CMP 绝对行高 24.75sp @14sp → height 倍率 24.75/14
      expect(t.dialog.body.height, closeTo(24.75 / 14, 1e-9));
      expect(t.sidebar.sectionLabel.letterSpacing, 0.4);
      expect(t.login.title.letterSpacing, -0.2);
      expect(t.transfer.taskName.fontSize, 13.5);
      expect(t.catalog.iconName.fontSize, 9);
    });

    test('尺寸（对齐 CMP DesignTokens.METRICS）', () {
      final m = MateMetrics.standard();
      expect(m.button.primaryHeight, 36);
      expect(m.button.primaryRadius, 8);
      expect(m.button.iconButtonSize, 32);
      expect(m.menu.defaultWidth, 168);
      expect(m.menu.containerRadius, 10);
      expect(m.form.textFieldHeight, 38);
      expect(m.form.controls.switchWidth, 46);
      expect(m.form.controls.switchHeight, 28);
      expect(m.navigation.sidebarItemHeight, 46);
      expect(m.navigation.breadcrumbHeight, 40);
      expect(m.dialog.containerRadius, 12);
      expect(m.sidebar.width, 248);
      expect(m.mainPage.appBarHeight, 64);
      expect(m.feedback.emptyBadgeSize, 72);
    });
  });

  group('MateTheme 访问入口', () {
    testWidgets('colorsOf/typographyOf/metricsOf 返回当前皮肤组件', (tester) async {
      late MateSemanticColors colors;
      late MateTypography typography;
      late MateMetrics metrics;
      await tester.pumpWidget(
        MaterialApp(
          home: MateLinkTheme(
            child: Builder(
              builder: (context) {
                colors = MateTheme.colorsOf(context);
                typography = MateTheme.typographyOf(context);
                metrics = MateTheme.metricsOf(context);
                return const SizedBox();
              },
            ),
          ),
        ),
      );
      expect(colors.brand, const Color(0xFF0053DB));
      expect(typography.button.primaryLabel.fontSize, 12);
      expect(metrics.button.primaryHeight, 36);
    });

    testWidgets('系统暗色模式下仍强制浅色皮肤（对齐 Tauri：不支持深色）', (tester) async {
      tester.platformDispatcher.platformBrightnessTestValue = Brightness.dark;
      addTearDown(() => tester.platformDispatcher.clearAllTestValues());

      late MateSemanticColors colors;
      await tester.pumpWidget(
        MaterialApp(
          home: MateLinkTheme(
            child: Builder(
              builder: (context) {
                colors = MateTheme.colorsOf(context);
                return const SizedBox();
              },
            ),
          ),
        ),
      );
      expect(colors.bgPage, const Color(0xFFF5F5F7));
      expect(colors.brand, const Color(0xFF0053DB));
    });

    testWidgets('reducedMotionOf 默认 false，跟随 disableAnimations', (tester) async {
      late bool reduced;
      await tester.pumpWidget(
        MaterialApp(
          home: MateLinkTheme(
            child: Builder(
              builder: (context) {
                reduced = MateTheme.reducedMotionOf(context);
                return const SizedBox();
              },
            ),
          ),
        ),
      );
      expect(reduced, isFalse);
    });
  });
}

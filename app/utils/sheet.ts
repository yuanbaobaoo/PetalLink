/**
 * 登录浮层几何计算。
 *
 * 形态参考主流桌面客户端：接近满窗的圆角卡片，四周留白，从底部滑入。
 * 原生 child webview 只能放在矩形区域里，所以卡片外壳的几何必须先由 JS 算好，
 * 再把**内容区**（扣掉标题区）的矩形交给后端，两边用同一组常量保证严丝合缝。
 */

/** 浮层内容区矩形（逻辑像素，原点为窗口内容区左上角，即 CSS 视口原点）。 */
export interface SheetRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** 卡片与窗口左右边缘的留白。 */
export const SHEET_MARGIN_X = 20;
/** 卡片与窗口顶部的留白（露出一部分背景，强调层级）。 */
export const SHEET_MARGIN_TOP = 20;
/** 卡片与窗口底部的留白。
 *
 * 底部必须贴边（0）：浮层是从窗口底部滑上来的 sheet，底部再留一条缝隙就变成
 * 「浮在空中的卡片」，和滑入动势对不上。
 */
export const SHEET_MARGIN_BOTTOM = 0;
/** 标题区高度（单行标题 + 右上角按钮），原生 webview 从这里下方开始。 */
export const SHEET_HEADER_HEIGHT = 56;
/** 卡片圆角（只用于顶部两角）。
 *
 * 底部贴边，两个下角保持直角——这是贴底 sheet 的标准形态；给底部加圆角反而会
 * 在窗口边缘露出一块缺口。
 */
export const SHEET_RADIUS = 18;

/**
 * 计算浮层外框矩形（含标题区）。
 *
 * @param viewport - 窗口内容区尺寸（CSS 像素）
 */
export function computeSheetRect(viewport: { width: number; height: number }): SheetRect {
  const width = Math.max(0, viewport.width - SHEET_MARGIN_X * 2);
  const height = Math.max(
    0,
    viewport.height - SHEET_MARGIN_TOP - SHEET_MARGIN_BOTTOM,
  );
  return {
    x: Math.round(SHEET_MARGIN_X),
    y: Math.round(SHEET_MARGIN_TOP),
    width: Math.round(width),
    height: Math.round(height),
  };
}

/**
 * 由浮层外框推出原生 webview 的内容区矩形。
 *
 * 内容区从标题区底边开始，一路铺到卡片底边（底部贴边，不留缝）。
 *
 * @param sheet - 浮层外框
 * @param headerHeight - 标题区实际高度（缺省用设计常量）
 */
export function sheetContentRect(
  sheet: SheetRect,
  headerHeight: number = SHEET_HEADER_HEIGHT,
): SheetRect {
  return {
    x: sheet.x,
    y: sheet.y + headerHeight,
    width: sheet.width,
    height: Math.max(0, sheet.height - headerHeight),
  };
}

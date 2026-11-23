import { describe, expect, it } from "vitest";

import {
  computeSheetRect,
  sheetContentRect,
  SHEET_HEADER_HEIGHT,
  SHEET_MARGIN_BOTTOM,
  SHEET_MARGIN_TOP,
  SHEET_MARGIN_X,
} from "@/utils/sheet";

describe("登录浮层几何", () => {
  it("宽屏下浮层铺满可用区域并保留四周留白", () => {
    const sheet = computeSheetRect({ width: 1600, height: 1000 });

    expect(sheet.x).toBe(SHEET_MARGIN_X);
    expect(sheet.y).toBe(SHEET_MARGIN_TOP);
    expect(sheet.width).toBe(1600 - SHEET_MARGIN_X * 2);
    expect(sheet.height).toBe(1000 - SHEET_MARGIN_TOP - SHEET_MARGIN_BOTTOM);
    expect(sheet.y + sheet.height + SHEET_MARGIN_BOTTOM).toBe(1000);
  });

  it("窄屏下同样保留留白，不会溢出窗口", () => {
    const sheet = computeSheetRect({ width: 520, height: 800 });

    expect(sheet.width).toBe(520 - SHEET_MARGIN_X * 2);
    expect(sheet.x).toBe(SHEET_MARGIN_X);
    expect(sheet.y + sheet.height).toBeLessThanOrEqual(800);
  });

  it("内容区从标题栏下方开始，高度扣掉标题栏", () => {
    const sheet = computeSheetRect({ width: 1200, height: 900 });
    const content = sheetContentRect(sheet);

    expect(content.x).toBe(sheet.x);
    expect(content.y).toBe(sheet.y + SHEET_HEADER_HEIGHT);
    expect(content.width).toBe(sheet.width);
    expect(content.height).toBe(sheet.height - SHEET_HEADER_HEIGHT);
  });

  it("内容区顶边紧贴标题区，不留白边", () => {
    const sheet = computeSheetRect({ width: 1200, height: 900 });
    const content = sheetContentRect(sheet);

    // 原生登录页顶边 = 标题区底边：不留额外缝隙，避免出现一条白带。
    expect(content.y).toBe(SHEET_MARGIN_TOP + SHEET_HEADER_HEIGHT);
  });

  it("内容区一路铺到卡片底边：浮层贴底滑动，底部不留缝", () => {
    const sheet = computeSheetRect({ width: 1200, height: 900 });
    const content = sheetContentRect(sheet);

    expect(content.y + content.height).toBe(sheet.y + sheet.height);
  });

  it("浮层贴住窗口底边（底部间距为 0）", () => {
    const viewport = { width: 1200, height: 900 };
    const sheet = computeSheetRect(viewport);

    expect(SHEET_MARGIN_BOTTOM).toBe(0);
    expect(sheet.y + sheet.height).toBe(viewport.height);
  });

  it("标题区高度可由调用方覆盖，避免写死常量错位", () => {
    const sheet = computeSheetRect({ width: 1200, height: 900 });
    const content = sheetContentRect(sheet, 92);

    expect(content.y).toBe(sheet.y + 92);
    expect(content.height).toBe(sheet.height - 92);
  });

  it("极小窗口下不会算出负尺寸", () => {
    const sheet = computeSheetRect({ width: 40, height: 60 });
    const content = sheetContentRect(sheet);

    expect(sheet.width).toBeGreaterThanOrEqual(0);
    expect(sheet.height).toBeGreaterThanOrEqual(0);
    expect(content.height).toBeGreaterThanOrEqual(0);
  });
});

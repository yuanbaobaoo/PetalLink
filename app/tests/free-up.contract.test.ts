import { describe, expect, it } from "vitest";
import rustFreeUpSource from "../../src/commands/free_up.rs?raw";

/**
 * 释放空间结构体跨语言合同：
 * FreeableItem / FreeUpBatchResult 的字段经 Tauri 返回值序列化时被前端按 camelCase 读取，
 * Rust 侧必须用 rename_all = "camelCase" 锁定，否则前端读不到计数（历史 BUG：
 * 释放成功但提示"没有文件被释放"）。
 */

// 从 Rust 源码提取指定 struct 声明前的 serde rename_all 属性
function rustSerdeRenameAll(structName: string): string | null {
  const pattern = new RegExp(
    `serde\\(rename_all\\s*=\\s*"([^"]+)"\\)[\\s\\S]{0,120}?pub struct ${structName}\\b`,
  );
  return rustFreeUpSource.match(pattern)?.[1] ?? null;
}

describe("释放空间结构体序列化合同", () => {
  it("FreeableItem 以 camelCase 序列化（前端 fileId/relPath 可读）", () => {
    expect(rustSerdeRenameAll("FreeableItem")).toBe("camelCase");
  });

  it("FreeUpBatchResult 以 camelCase 序列化（前端 freedCount/skippedCount/freedBytes 可读）", () => {
    expect(rustSerdeRenameAll("FreeUpBatchResult")).toBe("camelCase");
  });
});

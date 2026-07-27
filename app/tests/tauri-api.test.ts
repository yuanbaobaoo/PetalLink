import { describe, expect, it, vi } from "vitest";

// Tauri invoke 测试替身。
const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

import { invoke } from "@/api/tauri";

describe("Tauri API 错误合同", () => {
  it("将非结构化异常补齐为完整 Generic AppError", async () => {
    invokeMock.mockRejectedValueOnce("network");

    await expect(invoke("test_command")).rejects.toEqual({
      kind: "Generic",
      code: null,
      message: "network",
      status_code: null,
      error_code: null,
    });
  });

  it("保留后端返回的结构化 AppError", async () => {
    // 模拟 Rust AppError 的稳定五字段结构。
    const backendError = {
      kind: "DriveApi",
      code: "network",
      message: "网络连接失败",
      status_code: null,
      error_code: null,
    };

    invokeMock.mockRejectedValueOnce(backendError);

    await expect(invoke("test_command")).rejects.toBe(backendError);
  });
});

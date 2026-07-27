import { describe, expect, it } from "vitest";
import { call } from "@/api/tauri";

describe("Tauri API 错误合同", () => {
  it("将非结构化异常补齐为完整 Generic AppError", async () => {
    // 模拟 Tauri 或插件抛出的非结构化异常
    const operation = Promise.reject("network");

    await expect(call(operation)).rejects.toEqual({
      kind: "Generic",
      code: null,
      message: "network",
      status_code: null,
      error_code: null,
    });
  });

  it("保留后端返回的结构化 AppError", async () => {
    // 模拟 Rust AppError 的稳定五字段结构
    const backendError = {
      kind: "DriveApi",
      code: "network",
      message: "网络连接失败",
      status_code: null,
      error_code: null,
    };

    await expect(call(Promise.reject(backendError))).rejects.toBe(backendError);
  });
});

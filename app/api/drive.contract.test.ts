import { describe, expect, it } from "vitest";
import rustDriveSource from "../../src/commands/drive.rs?raw";
import { DELETE_TRACE_ERROR_PREFIX } from "./drive";

/// 从 Rust 源码提取 DELETE_TRACE_ERROR_PREFIX 常量值，锁定前后端机器合同。
function rustDeleteTracePrefix(): string {
  const match = rustDriveSource.match(
    /DELETE_TRACE_ERROR_PREFIX:\s*&str\s*=\s*"([^"]*)"/,
  );
  if (!match) throw new Error("无法从 Rust 源码读取 DELETE_TRACE_ERROR_PREFIX");
  return match[1];
}

describe("删除留痕前缀跨语言合同", () => {
  it("前端 DELETE_TRACE_ERROR_PREFIX 与 Rust 源码完全一致", () => {
    expect(DELETE_TRACE_ERROR_PREFIX).toBe(rustDeleteTracePrefix());
  });
});

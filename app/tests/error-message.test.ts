import { describe, expect, it } from "vitest";
import { extractErrorMessage, formatUserMessage } from "@/utils/error";

describe("用户侧错误提示", () => {
  it("隐藏旧任务中的重新规划和远端版本术语", () => {
    expect(formatUserMessage("远端文件已在规划后变化，拒绝用旧任务覆盖"))
      .toBe("云端文件已更新。为避免覆盖，请同步索引后重试。");
    expect(formatUserMessage("本地上传源已变化，需要重新规划"))
      .toBe("本地文件已发生变化，请重新检查并重试。");
  });

  it("将内部字段缺失转换为可执行建议", () => {
    expect(extractErrorMessage({ message: "Update 任务缺少真实 fileId" }))
      .toBe("文件同步信息不完整，请同步索引后重试。");
    expect(extractErrorMessage("非零上传断点缺少 session_url，拒绝作为全新请求重放"))
      .toBe("续传信息已失效，请重新开始上传。");
  });

  it("保留已经清晰的普通错误", () => {
    expect(formatUserMessage("网络连接失败")).toBe("网络连接失败");
  });
});

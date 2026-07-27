import { beforeEach, describe, expect, it, vi } from "vitest";
import { getThumbnail } from "@/api/drive";
import { commands } from "@/api/generated";

vi.mock("@/api/generated", () => ({
  DELETE_TRACE_ERROR_PREFIX: "TRACE_FAILED:",
  commands: {
    driveGetThumbnail: vi.fn(),
  },
}));

describe("缩略图 IPC 合同", () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  it("保留后端返回的真实图片 MIME data URL", async () => {
    // 后端已根据响应头或文件签名生成的 JPEG data URL
    const dataUrl = "data:image/jpeg;base64,/9j/4AAQ";
    vi.mocked(commands.driveGetThumbnail).mockResolvedValue(dataUrl);

    await expect(getThumbnail("file-1")).resolves.toBe(dataUrl);
    expect(commands.driveGetThumbnail).toHaveBeenCalledWith("file-1");
  });

  it("拒绝非图片 data URL", async () => {
    vi.mocked(commands.driveGetThumbnail).mockResolvedValue("data:text/html;base64,PGh0bWw+");

    await expect(getThumbnail("file-2")).resolves.toBeNull();
  });

  it("缩略图请求失败时回退为空", async () => {
    vi.mocked(commands.driveGetThumbnail).mockRejectedValue(new Error("network"));

    await expect(getThumbnail("file-3")).resolves.toBeNull();
  });
});

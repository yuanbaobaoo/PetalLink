import { beforeEach, describe, expect, it, vi } from "vitest";
import { getThumbnail } from "@/api/drive";
import { invoke } from "@/api/tauri";

vi.mock("@/api/tauri", () => ({
  invoke: vi.fn(),
}));

describe("缩略图 IPC 合同", () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  it("保留后端返回的真实图片 MIME data URL", async () => {
    // 后端已根据响应头或文件签名生成的 JPEG data URL
    const dataUrl = "data:image/jpeg;base64,/9j/4AAQ";
    vi.mocked(invoke).mockResolvedValue(dataUrl);

    await expect(getThumbnail("file-1")).resolves.toBe(dataUrl);
    expect(invoke).toHaveBeenCalledWith("drive_get_thumbnail", {
      fileId: "file-1",
    });
  });

  it("拒绝非图片 data URL", async () => {
    vi.mocked(invoke).mockResolvedValue("data:text/html;base64,PGh0bWw+");

    await expect(getThumbnail("file-2")).resolves.toBeNull();
  });

  it("缩略图请求失败时回退为空", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("network"));

    await expect(getThumbnail("file-3")).resolves.toBeNull();
  });
});

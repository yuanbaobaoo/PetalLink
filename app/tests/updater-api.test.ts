import { beforeEach, describe, expect, it, vi } from "vitest";

const checkMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/plugin-updater", () => ({
  check: checkMock,
}));

import {
  checkForUpdate,
  downloadAndInstall,
} from "@/api/updater";

beforeEach(() => {
  vi.clearAllMocks();
});

describe("更新 API", () => {
  it("检查失败时抛出可读错误，不把失败伪装成已是最新", async () => {
    checkMock.mockRejectedValue({ message: "无法下载更新清单" });

    await expect(checkForUpdate()).rejects.toThrow(
      "检查更新失败：无法下载更新清单",
    );
  });

  it("没有新版本时才返回 null", async () => {
    checkMock.mockResolvedValue(null);

    await expect(checkForUpdate()).resolves.toBeNull();
  });

  it("清单缺少当前平台时抛出可识别的非故障状态", async () => {
    checkMock.mockRejectedValue({
      message: 'None of the fallback platforms ["linux-x86_64-appimage", "linux-x86_64"] were found in the response `platforms` object',
    });

    await expect(checkForUpdate()).rejects.toMatchObject({
      name: "UpdatePlatformUnavailableError",
      code: "UPDATE_PLATFORM_UNAVAILABLE",
    });
  });

  it("下载安装失败时保留清晰的失败原因", async () => {
    checkMock.mockResolvedValue({
      version: "1.2.0",
      downloadAndInstall: vi.fn().mockRejectedValue({
        message: "更新包签名验证失败",
      }),
    });

    await expect(downloadAndInstall()).rejects.toThrow(
      "下载或安装更新失败：更新包签名验证失败",
    );
  });
});

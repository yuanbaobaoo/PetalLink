import { beforeEach, describe, expect, it, vi } from "vitest";

const readDirMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/plugin-fs", () => ({
  readDir: readDirMock,
}));

import { isCompletelyEmptyDir } from "@/utils/fs";

describe("Linux 云盘目录校验", () => {
  beforeEach(() => readDirMock.mockReset());

  it("FUSE 挂载点包含隐藏文件时也不算空目录", async () => {
    readDirMock.mockResolvedValue([{ name: ".keep", isDirectory: false }]);

    await expect(isCompletelyEmptyDir("/home/user/PetalLinkDrive")).resolves.toBe(false);
  });

  it("接受完全空的 FUSE 挂载点", async () => {
    readDirMock.mockResolvedValue([]);

    await expect(isCompletelyEmptyDir("/mnt/huawei_cloud")).resolves.toBe(true);
  });
});

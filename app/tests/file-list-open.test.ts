// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";

const commandMocks = vi.hoisted(() => ({
  syncBatchFileStatus: vi.fn(),
  syncCheckFileLocalStatus: vi.fn(),
  syncDownloadOnDemand: vi.fn(),
  syncCheckSafeFreeUp: vi.fn(),
  openLocalItem: vi.fn(),
  revealLocalItem: vi.fn(),
  virtualDriveStatus: vi.fn(),
}));
const platformMock = vi.hoisted(() => vi.fn(() => false));

vi.mock("@/utils/platform", () => ({
  isLinuxPlatform: platformMock,
}));

vi.mock("@/api/generated", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/generated")>();
  return {
    ...actual,
    commands: {
      ...actual.commands,
      ...commandMocks,
    },
  };
});

import type { DriveFile } from "@/api/generated";
import { ROOT, useFileBrowserStore } from "@/stores/fileBrowser";
import { useSyncStore } from "@/stores/sync";
import FileListView from "@/views/main/FileListView.vue";

let wrapper: VueWrapper | null = null;

/**
 * 构造文件列表测试对象。
 */
function driveFile(overrides: Partial<DriveFile> = {}): DriveFile {
  return {
    id: "file-1",
    name: "report.pdf",
    category: "document",
    size: 128,
    parent_folder: ["docs-id"],
    description: null,
    created_time: null,
    edited_time: "2026-07-29T08:00:00Z",
    mime_type: "application/pdf",
    content_hash: null,
    thumbnail_link: null,
    ...overrides,
  };
}

/**
 * 挂载一个位于“我的云盘/Docs”下的文件列表。
 */
function mountList(file: DriveFile): {
  browser: ReturnType<typeof useFileBrowserStore>;
  sync: ReturnType<typeof useSyncStore>;
} {
  const pinia = createPinia();
  setActivePinia(pinia);
  const browser = useFileBrowserStore();
  const sync = useSyncStore();
  browser.pathStack = [ROOT, { id: "docs-id", name: "Docs" }];
  browser.files = [file];
  sync.mountConfigured = true;
  sync.mountDir = "/sync-root";
  sync.isIndexing = false;
  wrapper = mount(FileListView, {
    attachTo: document.body,
    global: { plugins: [pinia] },
  });
  return { browser, sync };
}

beforeEach(() => {
  vi.clearAllMocks();
  platformMock.mockReturnValue(false);
  document.body.innerHTML = "";
  commandMocks.syncBatchFileStatus.mockResolvedValue({});
  commandMocks.syncCheckFileLocalStatus.mockResolvedValue("synced");
  commandMocks.syncDownloadOnDemand.mockResolvedValue(true);
  commandMocks.syncCheckSafeFreeUp.mockResolvedValue("not_synced");
  commandMocks.openLocalItem.mockResolvedValue(true);
  commandMocks.revealLocalItem.mockResolvedValue(true);
  commandMocks.virtualDriveStatus.mockResolvedValue({
    enabled: false,
    mounted: false,
    mount_dir: null,
    error: null,
  });
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
  document.body.innerHTML = "";
});

describe("FileListView 本地打开行为", () => {
  it("双击已同步文件时立即打开，不重复下载", async () => {
    mountList(driveFile());

    await wrapper!.find(".file-row").trigger("dblclick");
    await flushPromises();

    expect(commandMocks.syncCheckFileLocalStatus).toHaveBeenCalledWith("file-1");
    expect(commandMocks.syncDownloadOnDemand).not.toHaveBeenCalled();
    expect(commandMocks.openLocalItem).toHaveBeenCalledWith("Docs/report.pdf");
  });

  it.each(["placeholder", "not_synced"])(
    "双击 %s 文件时先等待下载完成，再打开本地文件",
    async (status) => {
      commandMocks.syncCheckFileLocalStatus.mockResolvedValue(status);
      mountList(driveFile());

      await wrapper!.find(".file-row").trigger("dblclick");
      await flushPromises();

      expect(commandMocks.syncDownloadOnDemand).toHaveBeenCalledWith(
        "file-1",
        "/sync-root/Docs/report.pdf",
      );
      expect(commandMocks.openLocalItem).toHaveBeenCalledWith("Docs/report.pdf");
      expect(
        commandMocks.syncDownloadOnDemand.mock.invocationCallOrder[0],
      ).toBeLessThan(commandMocks.openLocalItem.mock.invocationCallOrder[0]);
    },
  );

  it("按需云盘直接打开可见路径，由 FUSE 在首次读取时透明下载", async () => {
    const { sync } = mountList(driveFile());
    sync.virtualDriveEnabled = true;
    sync.virtualMountDir = "/home/user/PetalLinkDrive";

    await wrapper!.find(".file-row").trigger("dblclick");
    await flushPromises();

    expect(commandMocks.syncCheckFileLocalStatus).not.toHaveBeenCalled();
    expect(commandMocks.syncDownloadOnDemand).not.toHaveBeenCalled();
    expect(commandMocks.openLocalItem).toHaveBeenCalledWith("Docs/report.pdf");
  });

  it("FUSE 会话断开导致打开失败时立即刷新真实挂载状态", async () => {
    commandMocks.openLocalItem.mockRejectedValue(new Error("FUSE 会话已断开"));
    commandMocks.virtualDriveStatus.mockResolvedValue({
      enabled: true,
      mounted: false,
      mount_dir: null,
      error: "FUSE 会话已退出或挂载已断开，请重启 PetalLink",
    });
    const { sync } = mountList(driveFile());
    sync.applyVirtualDriveStatus({
      enabled: true,
      mounted: true,
      mount_dir: "/home/user/PetalLinkDrive",
      error: null,
    });

    await wrapper!.find(".file-row").trigger("dblclick");
    await flushPromises();

    expect(commandMocks.virtualDriveStatus).toHaveBeenCalledTimes(1);
    expect(sync.virtualDriveMounted).toBe(false);
    expect(sync.userVisibleRoot).toBe("");
    expect(sync.virtualDriveError).toContain("挂载已断开");
  });

  it("下载失败时不打开空占位文件", async () => {
    commandMocks.syncCheckFileLocalStatus.mockResolvedValue("placeholder");
    commandMocks.syncDownloadOnDemand.mockRejectedValue({
      kind: "Generic",
      code: null,
      message: "网络不可用",
      status_code: null,
      error_code: null,
    });
    mountList(driveFile());

    await wrapper!.find(".file-row").trigger("dblclick");
    await flushPromises();

    expect(commandMocks.openLocalItem).not.toHaveBeenCalled();
  });

  it("双击文件夹仍在应用内导航，右键操作才打开本地文件夹", async () => {
    const folder = driveFile({
      id: "folder-1",
      name: "Archive",
      category: "folder",
      mime_type: "application/vnd.huawei-apps.folder",
    });
    const { browser } = mountList(folder);
    const enterFolder = vi.spyOn(browser, "enterFolder").mockResolvedValue();

    await wrapper!.find(".file-row").trigger("dblclick");
    await flushPromises();

    expect(enterFolder).toHaveBeenCalledWith(folder);
    expect(commandMocks.openLocalItem).not.toHaveBeenCalled();

    await wrapper!.find(".file-row").trigger("contextmenu");
    await flushPromises();
    const openFolder = Array.from(
      document.body.querySelectorAll<HTMLButtonElement>(".ctx-item"),
    ).find((button) => button.textContent?.includes("在系统文件管理器中打开"));
    expect(openFolder).toBeDefined();

    openFolder!.click();
    await flushPromises();

    expect(commandMocks.openLocalItem).toHaveBeenCalledWith("Docs/Archive");
    expect(commandMocks.syncDownloadOnDemand).not.toHaveBeenCalled();
  });

  it("右键“打开所在文件夹”只调用文件管理器，不触发下载", async () => {
    mountList(driveFile());

    await wrapper!.find(".file-row").trigger("contextmenu");
    await flushPromises();
    const reveal = Array.from(
      document.body.querySelectorAll<HTMLButtonElement>(".ctx-item"),
    ).find((button) => button.textContent?.includes("打开所在文件夹"));
    expect(reveal).toBeDefined();

    reveal!.click();
    await flushPromises();

    expect(commandMocks.revealLocalItem).toHaveBeenCalledWith("Docs/report.pdf");
    expect(commandMocks.syncCheckFileLocalStatus).not.toHaveBeenCalled();
    expect(commandMocks.syncDownloadOnDemand).not.toHaveBeenCalled();
  });
});

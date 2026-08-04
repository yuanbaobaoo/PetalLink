// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";

const updaterApiMocks = vi.hoisted(() => ({
  checkForUpdate: vi.fn(),
  downloadAndInstall: vi.fn(),
}));

const commandMocks = vi.hoisted(() => ({
  updaterIsSupported: vi.fn(),
  transferHasActive: vi.fn(),
  appRelaunch: vi.fn(),
}));

vi.mock("@/api/updater", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/updater")>();
  return {
    ...actual,
    checkForUpdate: updaterApiMocks.checkForUpdate,
    downloadAndInstall: updaterApiMocks.downloadAndInstall,
  };
});

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

import UpdateDialog from "@/components/UpdateDialog.vue";
import { UpdatePlatformUnavailableError } from "@/api/updater";
import { useUpdaterStore } from "@/stores/updater";

let wrapper: VueWrapper | null = null;

beforeEach(() => {
  vi.clearAllMocks();
  document.body.innerHTML = "";
  setActivePinia(createPinia());
  commandMocks.updaterIsSupported.mockResolvedValue(true);
  commandMocks.transferHasActive.mockResolvedValue(false);
  commandMocks.appRelaunch.mockResolvedValue(null);
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
  document.body.innerHTML = "";
});

describe("更新 Store", () => {
  it("静默检查捕获网络错误且不展示错误状态", async () => {
    updaterApiMocks.checkForUpdate.mockRejectedValue({
      message: "更新服务器暂时不可用",
    });
    const store = useUpdaterStore();

    await store.silentCheck();

    expect(store.phase).toBe("idle");
    expect(store.errorMessage).toBe("");
    expect(store.lastCheckTime).not.toBeNull();
  });

  it("手动检查展示可读错误而不是 Error 前缀或对象字符串", async () => {
    updaterApiMocks.checkForUpdate.mockRejectedValue({
      message: "更新清单格式无效",
    });
    const store = useUpdaterStore();

    await store.manualCheck();

    expect(store.phase).toBe("error");
    expect(store.errorMessage).toBe("更新清单格式无效");
  });

  it("手动检查发现清单缺少当前平台时按已是最新处理", async () => {
    updaterApiMocks.checkForUpdate.mockRejectedValue(
      new UpdatePlatformUnavailableError(
        'None of the fallback platforms ["linux-x86_64-appimage", "linux-x86_64"] were found in the response `platforms` object',
      ),
    );
    const store = useUpdaterStore();

    await expect(store.manualCheck()).resolves.toBe(false);

    expect(store.phase).toBe("upToDate");
    expect(store.errorMessage).toBe("");
    expect(store.dialogOpen).toBe(false);
  });

  it("静默检查发现清单缺少当前平台时按已是最新处理", async () => {
    updaterApiMocks.checkForUpdate.mockRejectedValue(
      new UpdatePlatformUnavailableError(
        'None of the fallback platforms ["linux-x86_64-appimage", "linux-x86_64"] were found in the response `platforms` object',
      ),
    );
    const store = useUpdaterStore();

    await store.silentCheck();

    expect(store.phase).toBe("upToDate");
    expect(store.errorMessage).toBe("");
    expect(store.dialogOpen).toBe(false);
  });

  it("失败后重试会重置累计字节、总量和百分比", async () => {
    updaterApiMocks.downloadAndInstall
      .mockImplementationOnce(async (onProgress) => {
        onProgress({ stage: "started", total: 100 });
        onProgress({ stage: "progress", downloaded: 20, total: 100 });
        throw { message: "连接中断" };
      })
      .mockImplementationOnce(async (onProgress) => {
        onProgress({ stage: "started", total: 200 });
        onProgress({ stage: "progress", downloaded: 50, total: 200 });
      });
    const store = useUpdaterStore();

    await store.downloadAndInstall();
    expect(store.downloaded).toBe(20);
    expect(store.downloadTotal).toBe(100);
    expect(store.downloadProgress).toBe(20);
    expect(store.errorMessage).toBe("连接中断");

    await store.downloadAndInstall();
    expect(store.phase).toBe("downloaded");
    expect(store.downloaded).toBe(50);
    expect(store.downloadTotal).toBe(200);
    expect(store.downloadProgress).toBe(25);
  });
});

describe("更新对话框", () => {
  it("重试成功后继续等待传输并重启", async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    const store = useUpdaterStore();
    store.phase = "error";
    store.dialogOpen = true;
    store.errorMessage = "上次下载失败";
    const download = vi.spyOn(store, "downloadAndInstall")
      .mockImplementation(async () => {
        store.phase = "downloaded";
      });
    const wait = vi.spyOn(store, "waitForTransfers").mockResolvedValue(true);

    wrapper = mount(UpdateDialog, {
      attachTo: document.body,
      global: { plugins: [pinia] },
    });
    const retry = Array.from(document.body.querySelectorAll("button"))
      .find((button) => button.textContent?.includes("重试"));
    expect(retry).toBeDefined();

    retry!.click();
    await flushPromises();

    expect(download).toHaveBeenCalledOnce();
    expect(wait).toHaveBeenCalledOnce();
    expect(commandMocks.appRelaunch).toHaveBeenCalledOnce();
    expect(download.mock.invocationCallOrder[0]).toBeLessThan(
      wait.mock.invocationCallOrder[0],
    );
    expect(wait.mock.invocationCallOrder[0]).toBeLessThan(
      commandMocks.appRelaunch.mock.invocationCallOrder[0],
    );
  });
});

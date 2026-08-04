// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";

const platformMock = vi.hoisted(() => vi.fn(() => true));
const openMock = vi.hoisted(() => vi.fn());
const readDirMock = vi.hoisted(() => vi.fn());
const commandMocks = vi.hoisted(() => ({
  configLoad: vi.fn(),
  configSave: vi.fn(),
  launchAtLoginIsEnabled: vi.fn(),
  trayIsVisible: vi.fn(),
  driveGetAbout: vi.fn(),
  appGetVersion: vi.fn(),
  openExternalUrl: vi.fn(),
}));

vi.mock("@/utils/platform", () => ({
  isLinuxPlatform: platformMock,
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: openMock,
}));

vi.mock("@tauri-apps/plugin-fs", () => ({
  readDir: readDirMock,
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

import type { AppConfig } from "@/api/config";
import SettingsPage from "@/views/settings/SettingsPage.vue";
import SyncSetupBanner from "@/views/main/SyncSetupBanner.vue";
import { useFileBrowserStore } from "@/stores/fileBrowser";
import { useSyncStore } from "@/stores/sync";
import { toasts } from "@/components/mate/useToast";

let wrapper: VueWrapper | null = null;

/**
 * 构造组件测试使用的完整配置。
 */
function config(overrides: Partial<AppConfig> = {}): AppConfig {
  return {
    oauth_redirect_uri: "http://127.0.0.1:9999/oauth/callback",
    oauth_callback_port: 9999,
    mount_dir: "/home/user/.local/share/petallink/backing",
    mount_configured: true,
    virtual_drive_enabled: true,
    virtual_mount_dir: "/mnt/huawei_cloud",
    concurrency: 6,
    poll_interval_sec: 900,
    debounce_sec: 3,
    skip_patterns: [],
    sort_field: "name",
    sort_order: "ascending",
    show_tray_icon: true,
    ...overrides,
  };
}

function buttonByText(text: string) {
  const button = wrapper!.findAll("button").find((item) => item.text().includes(text));
  expect(button, `找不到按钮：${text}`).toBeDefined();
  return button!;
}

beforeEach(() => {
  vi.clearAllMocks();
  setActivePinia(createPinia());
  platformMock.mockReturnValue(true);
  commandMocks.configLoad.mockResolvedValue(config());
  commandMocks.configSave.mockResolvedValue(null);
  commandMocks.launchAtLoginIsEnabled.mockResolvedValue(false);
  commandMocks.trayIsVisible.mockResolvedValue(true);
  commandMocks.driveGetAbout.mockRejectedValue(new Error("not needed"));
  commandMocks.appGetVersion.mockResolvedValue("1.1.3");
  commandMocks.openExternalUrl.mockResolvedValue(true);
  openMock.mockResolvedValue("/mnt/new_huawei_cloud");
  readDirMock.mockResolvedValue([]);
  toasts.splice(0, toasts.length);
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
  toasts.splice(0, toasts.length);
});

describe("Linux 单一云盘目录 UX", () => {
  it("设置页只展示用户可见目录，不展示 backing、模式开关或实验标识", async () => {
    wrapper = mount(SettingsPage, {
      global: { plugins: [createPinia()] },
    });
    await flushPromises();

    const text = wrapper.text();
    expect(text).toContain("云盘目录");
    expect(text).toContain("/mnt/huawei_cloud");
    expect(text).toContain("首次打开时自动获取");
    expect(text).not.toContain("/home/user/.local/share/petallink/backing");
    expect(text).not.toContain("启用按需云盘");
    expect(text).not.toContain("实验");
    expect(text).not.toContain("传统同步模式");
  });

  it("设置页更换目录时只更新 FUSE 可见目录", async () => {
    wrapper = mount(SettingsPage, {
      global: { plugins: [createPinia()] },
    });
    await flushPromises();

    await buttonByText("更换目录").trigger("click");
    await flushPromises();
    await buttonByText("保存设置").trigger("click");
    await flushPromises();

    expect(openMock).toHaveBeenCalledWith(expect.objectContaining({
      title: "选择云盘目录",
    }));
    expect(commandMocks.configSave).toHaveBeenCalledWith(expect.objectContaining({
      mount_dir: "/home/user/.local/share/petallink/backing",
      mount_configured: true,
      virtual_drive_enabled: true,
      virtual_mount_dir: "/mnt/new_huawei_cloud",
    }));
  });

  it("旧传统配置显示为待选择云盘目录，不把 backing 当成用户目录", async () => {
    commandMocks.configLoad.mockResolvedValue(config({
      mount_dir: "/mnt/huawei_cloud",
      virtual_drive_enabled: false,
      virtual_mount_dir: "",
    }));
    wrapper = mount(SettingsPage, {
      global: { plugins: [createPinia()] },
    });
    await flushPromises();

    expect(wrapper.text()).toContain("尚未配置云盘目录");
    expect(wrapper.text()).not.toContain("/mnt/huawei_cloud");
  });

  it("首次引导把选择写入云盘目录字段并保留后端 backing", async () => {
    commandMocks.configLoad.mockResolvedValue(config({
      mount_configured: false,
      virtual_drive_enabled: false,
      virtual_mount_dir: "",
    }));
    const pinia = createPinia();
    setActivePinia(pinia);
    const sync = useSyncStore();
    const browser = useFileBrowserStore();
    sync.setupPhase = "needsSetup";
    sync.mountConfigured = false;
    vi.spyOn(sync, "init").mockResolvedValue();
    vi.spyOn(browser, "loadRoot").mockResolvedValue();
    wrapper = mount(SyncSetupBanner, {
      global: { plugins: [pinia] },
    });

    expect(wrapper.text()).toContain("尚未配置云盘目录");
    expect(wrapper.text()).toContain("选择云盘目录");
    await buttonByText("选择云盘目录").trigger("click");
    await flushPromises();

    expect(commandMocks.configSave).toHaveBeenCalledWith(expect.objectContaining({
      mount_dir: "/home/user/.local/share/petallink/backing",
      mount_configured: true,
      virtual_drive_enabled: true,
      virtual_mount_dir: "/mnt/new_huawei_cloud",
    }));
  });
});

describe("非 Linux 目录 UX", () => {
  it("继续展示并更新传统同步目录", async () => {
    platformMock.mockReturnValue(false);
    commandMocks.configLoad.mockResolvedValue(config({
      mount_dir: "/Users/user/HuaweiDrive",
      virtual_drive_enabled: false,
      virtual_mount_dir: "",
    }));
    wrapper = mount(SettingsPage, {
      global: { plugins: [createPinia()] },
    });
    await flushPromises();

    expect(wrapper.text()).toContain("同步目录");
    expect(wrapper.text()).toContain("/Users/user/HuaweiDrive");
    expect(wrapper.text()).not.toContain("启用按需云盘");
    await buttonByText("更换目录").trigger("click");
    await flushPromises();
    await buttonByText("保存设置").trigger("click");
    await flushPromises();

    expect(openMock).toHaveBeenCalledWith(expect.objectContaining({
      title: "选择同步目录",
    }));
    expect(commandMocks.configSave).toHaveBeenCalledWith(expect.objectContaining({
      mount_dir: "/mnt/new_huawei_cloud",
      virtual_drive_enabled: false,
      virtual_mount_dir: "",
    }));
  });
});

describe("关于页项目链接", () => {
  async function openAboutPage(): Promise<void> {
    wrapper = mount(SettingsPage, {
      global: { plugins: [createPinia()] },
    });
    await flushPromises();
    const aboutNav = wrapper.findAll(".mate-nav-item")
      .find((item) => item.text().includes("关于"));
    expect(aboutNav).toBeDefined();
    await aboutNav!.trigger("click");
  }

  it("通过后端安全命令交给系统默认浏览器，不使用 WebView 原生链接", async () => {
    await openAboutPage();

    expect(wrapper!.findAll("a.about-link")).toHaveLength(0);
    await buttonByText("GitHub").trigger("click");
    await flushPromises();

    expect(commandMocks.openExternalUrl).toHaveBeenCalledWith(
      "https://github.com/yuanbaobaoo/PetalLink",
    );
  });

  it("系统浏览器打开失败时展示可读错误提示", async () => {
    commandMocks.openExternalUrl.mockRejectedValueOnce({
      message: "系统没有可用的默认浏览器",
    });
    await openAboutPage();

    await buttonByText("GitCode").trigger("click");
    await flushPromises();

    expect(commandMocks.openExternalUrl).toHaveBeenCalledWith(
      "https://gitcode.com/yuanbaobaoo/PetalLink",
    );
    expect(toasts).toHaveLength(1);
    expect(toasts[0]).toMatchObject({
      message: "打开 GitCode 失败：系统没有可用的默认浏览器",
      variant: "error",
    });
  });
});

import { describe, expect, it, vi } from "vitest";
import type { AppConfig } from "@/api/config";
import {
  runSyncDirectorySetup,
  type SyncDirectorySetupPort,
} from "@/composables/useSyncDirectorySetup";

/**
 * 构造同步目录配置流程的最小完整配置。
 */
function config(overrides: Partial<AppConfig> = {}): AppConfig {
  return {
    oauth_redirect_uri: "http://127.0.0.1:9999/oauth/callback",
    oauth_callback_port: 9999,
    mount_dir: "",
    mount_configured: false,
    concurrency: 6,
    poll_interval_sec: 60,
    debounce_sec: 3,
    skip_patterns: [".DS_Store"],
    sort_field: "name",
    sort_order: "ascending",
    show_tray_icon: true,
    ...overrides,
  };
}

/**
 * 创建可观测调用顺序的配置端口。
 */
function port(selected: string | null, empty = true) {
  // 外部能力的调用顺序。
  const calls: string[] = [];
  // 实际提交的配置快照。
  let savedConfig: AppConfig | null = null;
  // 当前测试使用的配置端口。
  const setupPort: SyncDirectorySetupPort = {
    selectDirectory: vi.fn(async () => {
      calls.push("select");
      return selected;
    }),
    isEmptyDirectory: vi.fn(async () => {
      calls.push("validate");
      return empty;
    }),
    saveConfig: vi.fn(async (nextConfig) => {
      calls.push("save");
      savedConfig = nextConfig;
    }),
    applyMountConfiguration: vi.fn(() => {
      calls.push("apply");
    }),
    refreshSyncState: vi.fn(async () => {
      calls.push("sync");
    }),
    refreshBrowser: vi.fn(async () => {
      calls.push("browser");
    }),
  };

  return {
    calls,
    getSavedConfig: () => savedConfig,
    setupPort,
  };
}

describe("统一同步目录配置入口", () => {
  it("首次配置保存完整表单并刷新全局状态和文件列表", async () => {
    // 当前表单包含尚未单独保存的设置。
    const currentConfig = config({
      concurrency: 12,
      skip_patterns: ["*.cache"],
      sort_field: "modifiedTime",
      sort_order: "descending",
    });

    // 当前测试使用的外部能力。
    const fixture = port("/Users/test/PetalLink");

    await expect(runSyncDirectorySetup(currentConfig, fixture.setupPort)).resolves.toEqual({
      path: "/Users/test/PetalLink",
    });

    expect(fixture.getSavedConfig()).toMatchObject({
      mount_dir: "/Users/test/PetalLink",
      mount_configured: true,
      concurrency: 12,
      skip_patterns: ["*.cache"],
      sort_field: "modifiedTime",
      sort_order: "descending",
    });
    expect(fixture.calls).toEqual(["select", "validate", "save", "apply", "sync", "browser"]);
  });

  it("用户取消选择时不保存或刷新配置", async () => {
    // 当前测试使用的外部能力。
    const fixture = port(null);

    await expect(runSyncDirectorySetup(config(), fixture.setupPort)).resolves.toBeNull();
    expect(fixture.calls).toEqual(["select"]);
  });

  it("非空目录在保存前被拒绝", async () => {
    // 当前测试使用的外部能力。
    const fixture = port("/Users/test/not-empty", false);

    await expect(runSyncDirectorySetup(config(), fixture.setupPort)).rejects.toThrow(
      "所选目录不为空",
    );
    expect(fixture.calls).toEqual(["select", "validate"]);
  });

  it("已配置目录变更也由后端决定重启，保存返回后仍走统一刷新", async () => {
    // 当前已配置的完整表单。
    const currentConfig = config({
      mount_dir: "/Users/test/old",
      mount_configured: true,
      debounce_sec: 8,
    });
    // 当前测试使用的外部能力。
    const fixture = port("/Users/test/new");

    await expect(runSyncDirectorySetup(currentConfig, fixture.setupPort)).resolves.toEqual({
      path: "/Users/test/new",
    });
    expect(fixture.getSavedConfig()).toMatchObject({
      mount_dir: "/Users/test/new",
      debounce_sec: 8,
    });
    expect(fixture.calls).toEqual(["select", "validate", "save", "apply", "sync", "browser"]);
  });
});

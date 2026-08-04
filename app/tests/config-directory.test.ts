import { describe, expect, it } from "vitest";
import type { AppConfig } from "@/api/config";
import { withSelectedDriveDirectory } from "@/api/config";
import { isLinuxPlatform } from "@/utils/platform";

/**
 * 构造目录写入测试使用的完整配置。
 */
function config(): AppConfig {
  return {
    oauth_redirect_uri: "http://127.0.0.1:9999/oauth/callback",
    oauth_callback_port: 9999,
    mount_dir: "/home/user/.local/share/petallink/backing",
    mount_configured: false,
    virtual_drive_enabled: false,
    virtual_mount_dir: "",
    concurrency: 6,
    poll_interval_sec: 900,
    debounce_sec: 3,
    skip_patterns: [],
    sort_field: "name",
    sort_order: "ascending",
    show_tray_icon: true,
  };
}

describe("用户目录配置", () => {
  it("Linux 只写入用户可见云盘目录，不覆盖后端 backing", () => {
    const result = withSelectedDriveDirectory(config(), "/mnt/huawei_cloud", true);

    expect(result.mount_dir).toBe("/home/user/.local/share/petallink/backing");
    expect(result.mount_configured).toBe(true);
    expect(result.virtual_drive_enabled).toBe(true);
    expect(result.virtual_mount_dir).toBe("/mnt/huawei_cloud");
  });

  it("非 Linux 保留传统同步目录行为", () => {
    const result = withSelectedDriveDirectory(config(), "/Users/user/HuaweiDrive", false);

    expect(result.mount_dir).toBe("/Users/user/HuaweiDrive");
    expect(result.mount_configured).toBe(true);
    expect(result.virtual_drive_enabled).toBe(false);
    expect(result.virtual_mount_dir).toBe("");
  });
});

describe("平台判断", () => {
  it("优先采用 Tauri 构建平台", () => {
    expect(isLinuxPlatform("linux", "Macintosh")).toBe(true);
    expect(isLinuxPlatform("darwin", "X11; Linux x86_64")).toBe(false);
  });

  it("浏览器开发模式回退到 user agent", () => {
    expect(isLinuxPlatform("", "X11; Linux x86_64")).toBe(true);
    expect(isLinuxPlatform(undefined, "Macintosh; Intel Mac OS X")).toBe(false);
  });
});

import { commands } from "./generated";
import type { AppConfig as GeneratedAppConfig } from "./generated";
import { isLinuxPlatform } from "@/utils/platform";

/**
 * 后端序列化始终返回完整配置；Rust 的 `serde(default)` 仅让导入旧配置时兼容缺字段。
 */
export type AppConfig = Required<GeneratedAppConfig>;

/**
 * 加载后端保证字段完整的当前配置。
 */
export async function loadConfig(): Promise<AppConfig> {
  return await commands.configLoad() as AppConfig;
}

/**
 * 解析后端补齐默认值后的导入配置。
 *
 * @param json - 待校验的配置 JSON
 */
export async function importConfigJson(json: string): Promise<AppConfig> {
  return await commands.configImportJson(json) as AppConfig;
}

/**
 * 将用户选择的目录写入对应平台的配置字段。
 *
 * Linux 只暴露 FUSE 云盘目录；`mount_dir` 是后端管理的私有 backing，前端不能
 * 用用户选择覆盖它。其他平台继续把选择写入传统同步目录。
 */
export function withSelectedDriveDirectory(
  config: AppConfig,
  selected: string,
  linux = isLinuxPlatform(),
): AppConfig {
  if (linux) {
    return {
      ...config,
      mount_configured: true,
      virtual_drive_enabled: true,
      virtual_mount_dir: selected,
    };
  }
  return {
    ...config,
    mount_configured: true,
    mount_dir: selected,
  };
}

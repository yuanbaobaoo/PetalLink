import { open } from "@tauri-apps/plugin-dialog";
import { commands } from "@/api/generated";
import * as configApi from "@/api/config";
import type { AppConfig } from "@/api/config";
import { useSyncStore } from "@/stores/sync";
import { useFileBrowserStore } from "@/stores/fileBrowser";
import { isCompletelyEmptyDir, isEmptyDir } from "@/utils/fs";

// 原生目录选择器标题。
const DIRECTORY_PICKER_TITLE = "选择同步目录";
// 非空目录统一错误文案。
const NON_EMPTY_DIRECTORY_MESSAGE =
  "所选目录不为空。请选择一个空目录作为同步目录，避免与已有文件冲突。";

/**
 * 同步目录配置流程所需的外部能力。
 */
export interface SyncDirectorySetupPort {
  selectDirectory: () => Promise<string | null>;
  isEmptyDirectory: (path: string) => Promise<boolean>;
  saveConfig: (config: AppConfig) => Promise<void>;
  applyMountConfiguration: (path: string) => void;
  refreshSyncState: () => Promise<void>;
  refreshBrowser: () => Promise<void>;
}

/**
 * 同步目录配置完成后的明确结果。
 */
export interface SyncDirectorySetupResult {
  // 已保存的同步目录。
  path: string;
}

/**
 * 选择空目录、提交调用方的完整配置，并刷新依赖目录配置的全局状态。
 *
 * @param config - 调用方当前完整配置快照
 * @param port - 目录配置流程依赖
 * @returns 成功结果；用户取消选择时返回 null
 */
export async function runSyncDirectorySetup(
  config: AppConfig,
  port: SyncDirectorySetupPort,
): Promise<SyncDirectorySetupResult | null> {
  // 用户取消时不得读取或修改任何配置。
  const selected = await port.selectDirectory();
  if (!selected) return null;

  // 目录准入失败必须发生在配置提交之前。
  if (!(await port.isEmptyDirectory(selected))) {
    throw new Error(NON_EMPTY_DIRECTORY_MESSAGE);
  }

  // 保留设置页当前表单的全部字段，避免目录选择覆盖尚未提交的编辑。
  const nextConfig: AppConfig = {
    ...config,
    mount_dir: selected,
    mount_configured: true,
    skip_patterns: [...config.skip_patterns],
  };

  await port.saveConfig(nextConfig);
  port.applyMountConfiguration(selected);

  // 保存命令正常返回时统一刷新；实际目录切换由后端按规范化路径决定是否重启。
  await port.refreshSyncState();
  await port.refreshBrowser();
  return { path: selected };
}

/**
 * 主页与设置页“选择目录”按钮共用的唯一生产入口。
 *
 * @param config - 调用方当前完整配置快照
 */
export async function selectAndConfigureSyncDirectory(
  config: AppConfig,
): Promise<SyncDirectorySetupResult | null> {
  // 当前同步状态。
  const sync = useSyncStore();
  // 当前文件浏览器状态。
  const browser = useFileBrowserStore();
  return runSyncDirectorySetup(config, {
    selectDirectory: async () => {
      // 原生目录选择结果。
      const selected = await open({
        directory: true,
        multiple: false,
        title: DIRECTORY_PICKER_TITLE,
      });
      return typeof selected === "string" ? selected : null;
    },
    isEmptyDirectory: isEmptyDir,
    saveConfig: async (nextConfig) => {
      await commands.configSave(nextConfig);
    },
    applyMountConfiguration: sync.applyMountConfiguration,
    refreshSyncState: sync.init,
    refreshBrowser: browser.loadRoot,
  });
}

/**
 * Linux 云盘目录选择结果：取消 / 非空（调用方自定提示文案）/ 已选定并提交。
 */
export type LinuxDriveSelection =
  | { status: "cancelled" }
  | { status: "not-empty" }
  | { status: "selected"; path: string; config: AppConfig };

/**
 * Linux 云盘目录配置的唯一生产入口（设置页与首次引导共用）：
 * 选择完全空的 FUSE 可见目录、提交配置并收敛全局状态。
 * 用户可见目录不覆盖后端管理的 backing。
 *
 * @param config - 调用方当前完整配置快照
 */
export async function selectLinuxDriveDirectory(
  config: AppConfig,
): Promise<LinuxDriveSelection> {
  const sync = useSyncStore();
  const browser = useFileBrowserStore();

  // 用户在系统目录选择器中选中的路径。
  const selected = await open({
    directory: true,
    multiple: false,
    title: "选择云盘目录",
  });
  if (!selected || typeof selected !== "string") return { status: "cancelled" };

  // 目录是否为完全空的挂载候选；无法读取时按非空处理，避免挂载覆盖未知内容。
  const isEmpty = await isCompletelyEmptyDir(selected).catch(() => false);
  if (!isEmpty) return { status: "not-empty" };

  // 合并用户可见目录后的完整待提交配置。
  const nextConfig = configApi.withSelectedDriveDirectory(config, selected, true);
  await commands.configSave(nextConfig);
  sync.applyMountConfiguration(selected);
  await sync.init();
  await browser.loadRoot();
  return { status: "selected", path: selected, config: nextConfig };
}

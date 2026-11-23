/**
 * 应用身份信息：展示名与完整标题。
 *
 * dev 包（`tauri.dev.conf.json` 把 productName 覆盖成 `PetalLink-Dev`）会在所有
 * 标签上带 `-Dev`，方便开发者同时安装正式版和开发版时一眼区分。
 */
import { computed, onMounted, ref } from "vue";

import { commands } from "@/api/generated";

// 进程内缓存的展示名，多组件共用只请求一次。
let cachedDisplayName: string | null = null;

/**
 * 读取当前构建的展示名（首次调用会请求后端，后续走缓存）。
 */
async function loadDisplayName(): Promise<string> {
  if (cachedDisplayName !== null) return cachedDisplayName;
  try {
    cachedDisplayName = await commands.appDisplayName();
  } catch {
    // 后端不可用时退回正式名，避免界面出现空标题。
    cachedDisplayName = "PetalLink";
  }
  return cachedDisplayName;
}

/**
 * 组合式：返回展示名与完整标题。
 */
export function useAppIdentity() {
  // 当前构建的展示名；缓存已就绪时直接用它，避免首帧闪一下正式名。
  const displayName = ref(cachedDisplayName ?? "PetalLink");
  // 窗口/登录页共用的完整标题。
  const fullTitle = computed(() => `${displayName.value} - 华为云盘客户端开源版`);

  // 挂载后补齐真实展示名（dev 包会带 -Dev）。
  onMounted(async () => {
    displayName.value = await loadDisplayName();
  });

  return { displayName, fullTitle };
}

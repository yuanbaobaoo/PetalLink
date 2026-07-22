<!-- 设置页（v2：左侧分组导航 240px + 右侧白卡面板 + 毛玻璃保存底栏） -->
<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { MateNavItem, MateSectionHeader, MateStepper, MateNumberField, MateTextField, MateSwitch, MateButton, MateInfoBanner, MateLogoWithText, MateIcon } from "@/components/mate";
import { confirmDialog, showToast } from "@/components/mate";
import * as configApi from "@/api/config";
import * as platformApi from "@/api/platform";
import * as driveApi from "@/api/drive";
import * as authApi from "@/api/auth";
import LogViewerPage from "@/views/settings/LogViewerPage.vue";
import { useAuthStore } from "@/stores/auth";
import { useUpdaterStore } from "@/stores/updater";
import { useAsyncAction } from "@/composables/useAsyncAction";
import { open } from "@tauri-apps/plugin-dialog";
import { formatFileSize } from "@/utils/format";
import { isEmptyDir } from "@/utils/fs";

const auth = useAuthStore();
const updater = useUpdaterStore();

type TabKey = "syncDir" | "transfer" | "advanced" | "account" | "logs" | "about";
// 当前激活的 Tab
const activeTab = ref<TabKey>("syncDir");

// 左侧导航分组（通用 / 其他）
const tabGroups: { group: string; items: { key: TabKey; icon: string; label: string }[] }[] = [
  {
    group: "通用",
    items: [
      { key: "syncDir", icon: "folder", label: "同步目录" },
      { key: "transfer", icon: "transfer", label: "传输设置" },
      { key: "advanced", icon: "settings", label: "高级设置" },
    ],
  },
  {
    group: "其他",
    items: [
      { key: "account", icon: "info", label: "账号管理" },
      { key: "logs", icon: "list", label: "日志查看" },
      { key: "about", icon: "cloud", label: "关于" },
    ],
  },
];

// 传输设置
const concurrency = ref(6);
const debounceSec = ref(3);
// 云端定时刷新间隔（秒）：0=关闭，默认 900（15 分钟）
const pollIntervalSec = ref(60);
const skipPatterns = ref(".DS_Store, .tmp, ~$*, .Trash");
// 同步目录设置
const mountDir = ref("");
const mountConfigured = ref(false);
// OAuth 设置
const oauthPort = ref(9999);
// 开机自启
const autoLaunch = ref(false);
// 托盘图标显示
const showTrayIcon = ref(true);
// 保存状态
const saving = ref(false);
const saved = ref(false);
const errorMessage = ref<string | null>(null);
// 异步按钮 loading + 防重复点击
const { loading: clearLoading, run: runClearCache } = useAsyncAction();
const { loading: logoutLoading, run: runLogout } = useAsyncAction();
const { loading: selectDirLoading, run: runSelectDir } = useAsyncAction();

// 用户信息
const userInfo = computed(() => auth.userInfo);
const userLabel = computed(() => authApi.primaryLabel(userInfo.value) ?? "未获取到");
const userInitial = computed(() => authApi.initial(userInfo.value) ?? "华");
// 存储配额
const about = ref<driveApi.DriveAbout | null>(null);
// 应用版本号
const appVersion = ref("");

const showFooter = computed(() => ["syncDir", "transfer", "advanced"].includes(activeTab.value));

const emit = defineEmits<{ (e: "back"): void; (e: "open-logs"): void }>();

/**
 * 挂载后加载配置、开机自启状态、存储配额
 */
onMounted(async () => {
  try {
    const config = await configApi.loadConfig();
    concurrency.value = config.concurrency;
    debounceSec.value = config.debounce_sec;
    skipPatterns.value = config.skip_patterns.join(", ");
    mountDir.value = config.mount_dir;
    mountConfigured.value = config.mount_configured;
    oauthPort.value = config.oauth_callback_port;
    pollIntervalSec.value = config.poll_interval_sec;
  } catch {}
  try { autoLaunch.value = await platformApi.launchAtLoginIsEnabled(); } catch {}
  // 托盘图标以运行时实际状态为准（配置只是持久化目标值）
  try { showTrayIcon.value = await platformApi.trayIsVisible(); } catch {}
  try { about.value = await driveApi.getAbout(); } catch {}
  try { appVersion.value = await platformApi.getAppVersion(); } catch {}
  saved.value = true;
});

async function handleSave(): Promise<void> {
  if (saving.value) return; // 防重复点击
  saving.value = true; errorMessage.value = null;
  try {
    await configApi.saveConfig({
      oauth_redirect_uri: `http://127.0.0.1:${oauthPort.value}/oauth/callback`,
      oauth_callback_port: oauthPort.value,
      mount_dir: mountDir.value,
      mount_configured: mountConfigured.value,
      concurrency: concurrency.value,
      poll_interval_sec: pollIntervalSec.value,
      debounce_sec: debounceSec.value,
      skip_patterns: skipPatterns.value.split(",").map(s => s.trim()).filter(s => s),
      sort_field: "name",
      sort_order: "ascending",
      show_tray_icon: showTrayIcon.value,
    });
    saved.value = true;
    showToast("配置已保存");
  } catch (e) { errorMessage.value = String(e); }
  finally { saving.value = false; }
}

async function handleReset(): Promise<void> {
  try {
    const config = await configApi.loadConfig();
    concurrency.value = config.concurrency;
    debounceSec.value = config.debounce_sec;
    skipPatterns.value = config.skip_patterns.join(", ");
    mountDir.value = config.mount_dir;
    mountConfigured.value = config.mount_configured;
    oauthPort.value = config.oauth_callback_port;
    pollIntervalSec.value = config.poll_interval_sec;
    saved.value = true;
  } catch {}
}

/**
 * 切换开机自启
 *
 * @param v - 是否开启
 */
async function onToggleAutoLaunch(v: boolean): Promise<void> {
  autoLaunch.value = v;
  const ok = await platformApi.launchAtLoginSetEnabled(v);
  if (!ok) { autoLaunch.value = !v; showToast("设置开机自启失败", { variant: "error" }); }
}

/**
 * 切换托盘图标显示（立即生效 + 持久化）
 *
 * @param v - 是否显示
 */
async function onToggleTrayIcon(v: boolean): Promise<void> {
  showTrayIcon.value = v;
  try {
    await platformApi.traySetVisible(v);
  } catch {
    showTrayIcon.value = !v;
    showToast("设置托盘图标失败", { variant: "error" });
  }
}

async function handleClearCache(): Promise<void> {
  await runClearCache(async () => {
    const ok = await confirmDialog({
      title: "清空缓存并重启", titleIcon: "alert", danger: true, confirmText: "确认清空",
      content: "此操作将清除：\n• 登录状态（需重新登录）\n• 同步数据库（本地镜像与传输队列）\n• 缓存文件（同步状态快照 + 云端树缓存，工作目录）\n• 配置文件（端口、并发、过滤等设置）\n\n云盘文件不会被删除。",
    });
    if (!ok) return;
    // 后端 app_clear_cache 会停引擎+删 config+relaunch（进程将替换），先提示再调，不依赖返回
    showToast("正在清空并重启…");
    try { await configApi.clearCache(); } catch { /* 进程将重启，忽略响应 */ }
  });
}

async function handleLogout(): Promise<void> {
  await runLogout(async () => {
    const ok = await confirmDialog({
      title: "退出登录", titleIcon: "x", danger: true, confirmText: "退出",
      content: "确定退出当前账号吗？本地 token 将被清除。",
    });
    if (!ok) return;
    await auth.logout();
    emit("back");
  });
}

async function handleCheckUpdate(): Promise<void> {
  const hasUpdate = await updater.manualCheck();
  if (!hasUpdate && updater.phase === "upToDate") {
    showToast("已是最新版本", { variant: "success" });
  } else if (updater.phase === "error") {
    showToast(updater.errorMessage || "检查更新失败", { variant: "error" });
  }
  // hasUpdate=true → updater.phase="available" → UpdateDialog 会自动弹出
}

async function handleSelectDir(): Promise<void> {
  await runSelectDir(async () => {
    try {
      const selected = await open({ directory: true, multiple: false, title: "选择同步目录" });
      if (selected && typeof selected === "string") {
        // 校验：必须空目录（过滤隐藏文件 + skipPatterns）
        const isEmpty = await isEmptyDir(selected).catch(() => false);
        if (!isEmpty) {
          showToast("所选目录不为空，请选择一个空目录", { variant: "warning" });
          return;
        }
        mountDir.value = selected; mountConfigured.value = true; saved.value = false;
      }
    } catch {}
  });
}

/**
 * 格式化文件大小显示
 *
 * @param bytes - 字节数
 */
function fmtSize(bytes: number): string {
  return formatFileSize(bytes);
}
</script>

<template>
  <div class="settings-page">
    <!-- 标题栏（返回 + 标题） -->
    <div class="settings-appbar">
      <MateButton variant="icon" icon="arrow" tooltip="返回" class="back-btn" @click="emit('back')" />
      <span class="settings-appbar__title">设置</span>
    </div>

    <div class="settings-body">
      <!-- 左导航 240px（分组） -->
      <nav class="settings-nav">
        <template v-for="g in tabGroups" :key="g.group">
          <div class="settings-nav__group">{{ g.group }}</div>
          <MateNavItem v-for="tab in g.items" :key="tab.key" :label="tab.label" :icon="tab.icon" :active="activeTab === tab.key" @click="activeTab = tab.key" />
        </template>
      </nav>

      <!-- 右设置区 -->
      <div class="settings-main">
        <!-- 同步目录 -->
        <section v-if="activeTab === 'syncDir'" class="settings-section">
          <MateSectionHeader icon="folder" text="同步目录" />
          <div v-if="!mountConfigured" class="card">
            <div class="card__badge"><MateIcon name="folder-open" :size="32" /></div>
            <div class="card-title">尚未配置同步目录</div>
            <div class="card-desc">选择一个本地空目录作为云盘镜像，文件将自动双向同步。</div>
            <MateButton variant="primary" icon="folder-open" :loading="selectDirLoading" :disabled="selectDirLoading" @click="handleSelectDir">选择目录</MateButton>
          </div>
          <div v-else class="card">
            <MateIcon name="check" :size="20" class="card-icon card-icon--success" />
            <div class="card-title">当前同步目录</div>
            <code class="card-path">{{ mountDir }}</code>
            <MateButton variant="text" icon="folder-open" :loading="selectDirLoading" :disabled="selectDirLoading" @click="handleSelectDir">更换目录</MateButton>
          </div>
          <MateInfoBanner variant="info" class="info-banner">更换同步目录将清除所有本地缓存与登录状态并重启，云盘文件不受影响。</MateInfoBanner>
        </section>

        <!-- 传输设置 -->
        <section v-if="activeTab === 'transfer'" class="settings-section">
          <MateSectionHeader icon="transfer" text="传输设置" />
          <div class="settings-panel">
            <div class="group-header">传输参数</div>
            <div class="setting-row">
              <div class="setting-row__text">
                <div class="setting-label">并发上传数</div>
                <div class="setting-desc">同时进行的文件传输任务数量。较高值可提升大文件传输效率，但会占用更多网络带宽。</div>
              </div>
              <div class="setting-control"><MateStepper v-model="concurrency" :min="1" :max="20" /><span class="suffix">范围 1-20</span></div>
            </div>
            <div class="setting-row">
              <div class="setting-row__text">
                <div class="setting-label">Debounce 时长</div>
                <div class="setting-desc">文件变更后等待多少秒再触发同步上传，避免频繁修改导致重复传输。</div>
              </div>
              <div class="setting-control"><MateNumberField v-model="debounceSec" :min="1" :max="600" suffix="秒" /></div>
            </div>
            <div class="setting-row">
              <div class="setting-row__text">
                <div class="setting-label">自动同步间隔</div>
                <div class="setting-desc">每隔多久自动从云端拉取最新变更（新增/修改/删除）。0 = 关闭自动同步，仅手动点「同步索引」。设为 60 以上时生效。</div>
              </div>
              <div class="setting-control"><MateNumberField v-model="pollIntervalSec" :min="0" :max="86400" suffix="秒" /></div>
            </div>
            <div class="group-header">同步过滤</div>
            <div class="setting-row setting-row--column">
              <div class="setting-row__text">
                <div class="setting-label">跳过文件（逗号分隔）</div>
                <div class="setting-desc">匹配名称的文件不会被同步，如 .DS_Store、临时文件。</div>
              </div>
              <MateTextField v-model="skipPatterns" width="100%" />
            </div>
          </div>
        </section>

        <!-- 高级设置 -->
        <section v-if="activeTab === 'advanced'" class="settings-section">
          <MateSectionHeader icon="settings" text="高级设置" />
          <div class="settings-panel">
            <div class="group-header">通用</div>
            <div class="setting-row">
              <div class="setting-row__text">
                <div class="setting-label">开机自启动</div>
                <div class="setting-desc">开机登录后自动在后台启动（仅菜单栏图标，不显示主窗口）。关闭后需手动打开 App。</div>
              </div>
              <div class="setting-control"><MateSwitch :model-value="autoLaunch" @update:model-value="onToggleAutoLaunch" /></div>
            </div>
            <div class="setting-row">
              <div class="setting-row__text">
                <div class="setting-label">显示托盘图标</div>
                <div class="setting-desc">在菜单栏显示 PetalLink 图标（后台同步入口）。关闭后 App 仍在后台运行，此时可通过 Cmd+Q 完全退出。</div>
              </div>
              <div class="setting-control"><MateSwitch :model-value="showTrayIcon" @update:model-value="onToggleTrayIcon" /></div>
            </div>
            <div class="group-header">OAuth</div>
            <div class="setting-row">
              <div class="setting-row__text">
                <div class="setting-label">OAuth 回调端口</div>
                <div class="setting-desc">本地 HTTP 回调服务器监听端口。修改后需与 AGC 后台 redirect_uri 保持一致。</div>
              </div>
              <div class="setting-control"><MateNumberField v-model="oauthPort" :min="1" :max="65535" /></div>
            </div>
          </div>
          <MateInfoBanner variant="info" class="info-banner">回调地址固定为 http://127.0.0.1:&lt;端口&gt;/oauth/callback，修改端口后请同步更新 AGC 后台配置。</MateInfoBanner>
          <div class="settings-panel">
            <div class="group-header">维护</div>
            <div class="setting-row">
              <div class="setting-row__text">
                <div class="setting-label">清空缓存并重启</div>
                <div class="setting-desc">清除登录状态、同步数据库、同步快照与配置文件，然后重启 App。适用于排查同步异常或切换账号时使用。</div>
              </div>
              <div class="setting-control"><MateButton variant="primary" icon="trash" danger :loading="clearLoading" :disabled="clearLoading" @click="handleClearCache">清空</MateButton></div>
            </div>
          </div>
        </section>

        <!-- 账号管理 -->
        <section v-if="activeTab === 'account'" class="settings-section">
          <MateSectionHeader icon="info" text="账号管理" />
          <div class="card card--account">
            <div class="account-avatar">{{ userInitial }}</div>
            <div class="account-name">{{ userLabel }}</div>
          </div>
          <div class="settings-panel">
            <div class="group-header">账号信息</div>
            <div class="info-row"><span class="info-label">显示名</span><span class="info-value">{{ userInfo?.display_name ?? "—" }}</span></div>
            <div class="info-row"><span class="info-label">手机号</span><span class="info-value">{{ authApi.secondaryLabel(userInfo) ?? "未授权" }}</span></div>
            <div class="info-row"><span class="info-label">邮箱</span><span class="info-value">{{ userInfo?.email ?? "未授权" }}</span></div>
            <div class="info-row"><span class="info-label">OpenID</span><span class="info-value info-mono">{{ userInfo?.open_id ?? "—" }}</span></div>
            <div class="group-header">存储配额</div>
            <div class="info-row"><span class="info-label">已用空间</span><span class="info-value">{{ about ? fmtSize(about.used_space) : "—" }}</span></div>
            <div class="info-row"><span class="info-label">总容量</span><span class="info-value">{{ about && about.user_capacity > 0 ? fmtSize(about.user_capacity) : "—" }}</span></div>
            <div class="info-row"><span class="info-label">剩余空间</span><span class="info-value">{{ about && about.user_capacity > 0 ? fmtSize(about.user_capacity - about.used_space) : "—" }}</span></div>
          </div>
          <div class="settings-panel">
            <div class="group-header">账号操作</div>
            <div class="setting-row">
              <div class="setting-row__text">
                <div class="setting-label">退出登录</div>
                <div class="setting-desc">清除本地 token 并返回登录页。后台进程仍会继续，可从菜单栏彻底退出。</div>
              </div>
              <div class="setting-control"><MateButton variant="primary" icon="x" danger :loading="logoutLoading" :disabled="logoutLoading" @click="handleLogout">退出登录</MateButton></div>
            </div>
          </div>
        </section>

        <!-- 日志查看 — 内嵌在设置页中，保留左侧导航 -->
        <LogViewerPage v-if="activeTab === 'logs'" inline />

        <!-- 关于 -->
        <section v-if="activeTab === 'about'" class="settings-section">
          <MateSectionHeader icon="cloud" text="关于" />
          <div class="card card--about">
            <MateLogoWithText :height="30" />
            <div class="about-version-row">
              <span class="about-version">版本 {{ appVersion || "..." }}</span>
              <MateButton
                variant="text"
                icon="refresh"
                :loading="updater.isChecking"
                :disabled="updater.isChecking"
                @click="handleCheckUpdate"
              >
                {{ updater.isChecking ? '检查中…' : '检查更新' }}
              </MateButton>
              <span v-if="updater.phase === 'upToDate'" class="about-update-hint">已是最新版本</span>
              <span v-else-if="updater.phase === 'error'" class="about-update-hint about-update-hint--error">检查失败</span>
              <span v-else-if="updater.phase === 'available'" class="chip chip--brand">
                新版本 v{{ updater.newVersion }}
              </span>
            </div>
            <!-- 有可用更新时显示"查看更新日志"按钮 -->
            <div v-if="updater.phase === 'available'" class="about-update-action">
              <MateButton variant="soft" icon="info" @click="updater.showDialog()">查看更新日志</MateButton>
            </div>
            <div class="about-tagline">一款开源免费的华为云盘客户端</div>
            <!-- 更新包下载进度：正在下载时展示进度条，点击重新打开弹窗 -->
            <div
              v-if="updater.isUpdateDownloading"
              class="about-update-progress"
              @click="updater.showDownloadDialog()"
            >
              <div class="about-update-progress__head">
                <span class="about-update-progress__label">正在下载更新</span>
                <span class="about-update-progress__pct">{{ updater.downloadProgress }}%</span>
              </div>
              <div class="about-update-progress__bar">
                <div class="about-update-progress__fill" :style="{ width: `${updater.downloadProgress}%` }" />
              </div>
            </div>
            <div class="about-links">
              <a href="https://github.com/yuanbaobaoo/PetalLink" target="_blank" class="about-link" rel="noopener noreferrer">
                <MateIcon name="github" :size="16" />
                GitHub
              </a>
              <a href="https://gitcode.com/yuanbaobaoo/PetalLink" target="_blank" class="about-link" rel="noopener noreferrer">
                <MateIcon name="gitcode" :size="16" />
                GitCode
              </a>
            </div>
          </div>
        </section>
      </div>
    </div>

    <!-- 底部保存栏（毛玻璃） -->
    <div v-if="showFooter" class="settings-footer">
      <MateButton variant="primary" icon="check" :disabled="saved || saving" :loading="saving" @click="handleSave">{{ saving ? "保存中…" : "保存设置" }}</MateButton>
      <MateButton variant="text" @click="handleReset">重置默认</MateButton>
      <span v-if="errorMessage" class="footer-error">{{ errorMessage }}</span>
      <span v-else-if="saved" class="chip chip--ok footer-saved"><span class="footer-dot" /> 配置已保存</span>
    </div>
  </div>
</template>

<style scoped>
.settings-page { display: flex; flex-direction: column; width: 100%; height: 100%; background: var(--bg-app); }
.settings-appbar { height: 56px; display: flex; align-items: center; padding: 0 20px; gap: var(--space-sm); border-bottom: 1px solid var(--line); background: var(--bg-card); flex-shrink: 0; }
.back-btn { transform: rotate(180deg); }
.settings-appbar__title { font-size: var(--font-title-sm); font-weight: var(--fw-semibold); color: var(--ink-900); }
.settings-body { flex: 1; display: flex; min-height: 0; }

/* 左导航（v2：分组 + 大行高） */
.settings-nav {
  width: var(--settings-nav-width);
  padding: 20px 12px;
  border-right: 1px solid var(--line);
  background: rgba(247, 247, 249, 0.85);
  flex-shrink: 0;
  display: flex; flex-direction: column; gap: 6px;
  overflow-y: auto;
}
.settings-nav__group {
  font-size: 11px; font-weight: var(--fw-semibold); letter-spacing: 0.4px;
  color: var(--ink-300); padding: 20px 14px 6px;
}
.settings-nav__group:first-of-type { padding-top: 4px; }

/* 右设置区（v2：灰底 + 白卡面板） */
.settings-main { flex: 1; padding: 28px 32px; overflow-y: auto; background: var(--bg-app); }
.settings-panel {
  background: var(--bg-card);
  border-radius: var(--radius-lg);
  box-shadow: var(--sh-sm), 0 0 0 0.5px var(--line);
  padding: 4px 24px 8px;
  margin-bottom: 20px;
}
.group-header {
  font-size: var(--font-caption); font-weight: var(--fw-semibold); letter-spacing: 0.4px;
  color: var(--ink-400); padding: 20px 0 8px; text-transform: uppercase;
}
.group-header:first-of-type { padding-top: 12px; }
.setting-row { display: flex; align-items: center; gap: var(--space-xl); padding: var(--space-lg) 0; border-bottom: 1px solid var(--line); }
.setting-row:last-child { border-bottom: none; }
.setting-row--column { flex-direction: column; align-items: stretch; gap: var(--space-md); }
.setting-row__text { flex: 1; min-width: 0; }
.setting-label { font-size: var(--font-body); font-weight: var(--fw-medium); color: var(--ink-900); }
.setting-desc { font-size: 12.5px; color: var(--ink-400); margin-top: 3px; line-height: 1.55; }
.setting-control { display: flex; align-items: center; gap: var(--space-sm); flex-shrink: 0; }
.suffix { font-size: var(--font-body-sm); color: var(--ink-400); }
.info-banner { margin-top: 0; margin-bottom: 20px; }
.info-row { display: flex; padding: 13px 0; border-bottom: 1px solid var(--line); font-size: 13.5px; }
.info-row:last-child { border-bottom: none; }
.info-label { width: 96px; flex-shrink: 0; color: var(--ink-400); }
.info-value { flex: 1; color: var(--ink-900); word-break: break-all; }
.info-mono { font-family: var(--font-mono); font-size: var(--font-caption); }

/* 卡片 */
.card {
  padding: var(--space-xl); background: var(--bg-card);
  border-radius: var(--radius-lg);
  box-shadow: var(--sh-sm), 0 0 0 0.5px var(--line);
  display: flex; flex-direction: column; align-items: center; gap: var(--space-md);
  text-align: center; margin-bottom: 20px;
}
.card--account { flex-direction: row; align-items: center; gap: var(--space-lg); text-align: left; }
.card--about { align-items: flex-start; text-align: left; }
.card__badge {
  width: 72px; height: 72px; border-radius: 14px;
  background: var(--grad-brand-soft); color: var(--brand-400);
  display: flex; align-items: center; justify-content: center;
}
.card-icon { color: var(--ink-400); }
.card-icon--success { color: var(--ok); }
.card-title { font-size: var(--font-body); font-weight: var(--fw-semibold); color: var(--ink-900); }
.card-desc { font-size: var(--font-body-sm); color: var(--ink-400); }
.card-path { font-size: var(--font-caption); font-family: var(--font-mono); color: var(--ink-600); background: var(--bg-fill); padding: 2px var(--space-sm); border-radius: var(--radius-sm); }
.account-avatar { width: 56px; height: 56px; border-radius: 50%; background: var(--grad-brand); color: #fff; font-size: 22px; font-weight: var(--fw-semibold); display: flex; align-items: center; justify-content: center; flex-shrink: 0; }
.account-name { font-size: var(--font-title-sm); font-weight: var(--fw-semibold); color: var(--ink-900); }

/* 关于页 */
.about-version-row { display: flex; align-items: center; gap: var(--space-sm); margin-top: var(--space-sm); flex-wrap: wrap; }
.about-version { font-size: var(--font-caption); color: var(--ink-400); }
.about-tagline { font-size: var(--font-caption); color: var(--ink-400); }
.about-update-hint { font-size: var(--font-caption); color: var(--ok); }
.about-update-hint--error { color: var(--err); }
.about-update-action { margin-top: var(--space-xs); }
.about-update-progress { width: 100%; margin-top: var(--space-md); cursor: pointer; padding: var(--space-sm) 0; }
.about-update-progress__head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px; }
.about-update-progress__label { font-size: var(--font-caption); color: var(--ink-400); }
.about-update-progress__pct { font-size: var(--font-caption); font-weight: var(--fw-semibold); color: var(--brand-500); }
.about-update-progress__bar { height: 4px; background-color: var(--bg-fill); border-radius: var(--radius-full); overflow: hidden; }
.about-update-progress__fill { height: 100%; background: var(--grad-brand); border-radius: var(--radius-full); transition: width 0.3s ease; }
.about-links { display: flex; gap: var(--space-lg); margin-top: var(--space-md); }
.about-link { display: inline-flex; align-items: center; gap: var(--space-xs); font-size: var(--font-body-sm); color: var(--brand-500); text-decoration: none; transition: color 0.15s; }
.about-link:hover { color: var(--brand-400); text-decoration: underline; }

/* chip（关于页新版本提示 / 底栏已保存） */
.chip {
  display: inline-flex; align-items: center; gap: 5px;
  height: 24px; padding: 0 10px; border-radius: var(--radius-sm);
  font-size: var(--font-caption); font-weight: var(--fw-medium); white-space: nowrap;
}
.chip--brand { background: var(--brand-50); color: var(--brand-500); }
.chip--ok { background: var(--ok-bg); color: var(--ok); }

/* 底部保存栏（v2：毛玻璃） */
.settings-footer {
  height: 64px; display: flex; align-items: center; gap: 10px; padding: 0 32px;
  border-top: 1px solid var(--line);
  background: rgba(255, 255, 255, 0.85); backdrop-filter: blur(12px);
  flex-shrink: 0;
}
.footer-saved { margin-left: auto; }
.footer-dot { width: 6px; height: 6px; border-radius: 50%; background-color: var(--ok); display: inline-block; }
.footer-error { font-size: var(--font-caption); color: var(--err); margin-left: auto; }
</style>

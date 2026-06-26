/**
 * Auth Store —— 登录态管理。
 *
 * 状态机：initial → authorizing → loggedIn / loggedOut / error
 */
import { defineStore } from "pinia";
import { ref } from "vue";
import type { AppError } from "@/api/tauri";
import * as authApi from "@/api/auth";
import { useSyncStore } from "@/stores/sync";

/** 登录态枚举 */
export type AuthStatus =
  | "initial"
  | "authorizing"
  | "loggedIn"
  | "loggedOut"
  | "error";

// 默认回调端口（对齐后端 DEFAULT_CALLBACK_PORT）
const DEFAULT_PORT = 9999;

export const useAuthStore = defineStore("auth", () => {
  // 当前登录态（initial 表示启动时未确定）
  const status = ref<AuthStatus>("initial");
  // 是否正在执行异步操作（恢复/登录中）
  const loading = ref(false);
  // 错误信息（非 null 时 UI 显示错误 banner）
  const errorMessage = ref<string | null>(null);
  // client_secret 是否已配置（控制登录按钮可点）
  const secretConfigured = ref(false);
  // OAuth 回调端口
  const callbackPort = ref(DEFAULT_PORT);
  // 用户信息（登录后拉取）
  const userInfo = ref<authApi.UserInfo | null>(null);

  /**
   * 启动时恢复登录态。
   * 
   */
  async function restore(): Promise<void> {
    loading.value = true;
    try {
      secretConfigured.value = await authApi.checkSecret();
      const state = await authApi.restore();
      callbackPort.value = state.callback_port;
      status.value = state.logged_in ? "loggedIn" : "loggedOut";
      // restore 成功后立即拉 userInfo
      if (state.logged_in) {
        try {
          userInfo.value = await authApi.getUserInfo();
        } catch (e) {
          console.warn("[auth] getUserInfo failed:", e);
        }
      }
    } catch (e) {
      status.value = "error";
      errorMessage.value = (e as AppError).message;
    } finally {
      loading.value = false;
    }
  }

  /**
   * 发起 OAuth 登录。
   * 用户取消时静默回到 loggedOut。
   */
  async function login(): Promise<boolean> {
    if (loading.value) return false; // 防重复点击
    status.value = "authorizing";
    loading.value = true;
    errorMessage.value = null;
    try {
      await authApi.login(callbackPort.value);
      status.value = "loggedIn";
      loading.value = false;
      // 登录成功后立即拉用户信息
      try {
        userInfo.value = await authApi.getUserInfo();
      } catch (e) {
        console.warn("[auth] login getUserInfo failed:", e);
      }
      // 登录后（重新）初始化同步状态：onMounted 在未登录时已跑过、未调 sync.init，
      // 此处补调以加载挂载配置（引擎已由后端 auth_login 原地重启，sync_state 事件会随之流入）
      try {
        await useSyncStore().init();
      } catch (e) {
        console.warn("[auth] post-login sync init failed:", e);
      }
      return true;
    } catch (e) {
      loading.value = false;
      const msg = (e as AppError).message;
      // 用户主动取消（非错误，静默回到未登录态）
      if (msg.includes("用户取消授权")) {
        status.value = "loggedOut";
        return false;
      }
      status.value = "error";
      errorMessage.value = msg;
      return false;
    }
  }

  /** 取消正在进行的授权 */
  async function cancelLogin(): Promise<void> {
    await authApi.cancelLogin();
    status.value = "loggedOut";
    loading.value = false;
  }

  /** 清除错误信息（用户点「重新授权」时调用） */
  function dismissError(): void {
    errorMessage.value = null;
    status.value = "loggedOut";
  }

  /** 退出登录 */
  async function logout(): Promise<void> {
    await authApi.logout();
    status.value = "loggedOut";
  }

  return {
    status,
    loading,
    errorMessage,
    secretConfigured,
    callbackPort,
    userInfo,
    restore,
    login,
    cancelLogin,
    dismissError,
    logout,
  };
});

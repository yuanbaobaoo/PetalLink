/// PetalLink 前端 Vite 配置
import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { fileURLToPath, URL } from "node:url";

// Tauri 用固定端口 1420，HMR 与 Rust 端 devUrl 对齐
const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [vue()],

  // @ 路径别名指向当前目录（与 tsconfig.json paths 一致）
  // @assets 指向项目根 assets/，图标唯一图源，不再到处复制
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./", import.meta.url)),
      "@assets": fileURLToPath(new URL("../assets", import.meta.url)),
    },
  },

  // Tauri 期望固定端口，清空默认自动端口行为
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 忽略 Rust 端变更（由 cargo watch 处理）
      ignored: ["**/src-tauri/**"],
    },
    fs: {
      // 允许 Vite 读取项目根 assets/（@assets 别名指向此处）
      allow: ["..", "../assets"],
    },
  },

  // 环境变量前缀：只暴露 TAURI_ 开头的变量
  envPrefix: ["VITE_", "TAURI_"],

  build: {
    // Tauri 使用 Chromium on macOS，支持现代 ES
    target:
      process.env.TAURI_ENV_PLATFORM === "windows"
        ? "chrome105"
        : "safari13",
    minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
}));

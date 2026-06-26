<p align="center"><img alt="PetalLink" src="assets/logo.png" width="80"></p>

# PetalLink

华为云盘 Mac 客户端 —— 基于 Tauri 2.x，将华为云空间挂载到本地，双向实时同步。

> 华为云空间目前并不支持 macOS。PetalLink 通过华为 Drive REST API 直连，不依赖 HMS Core SDK，为 macOS 用户提供接近原生的云盘体验。
>
> 灵感来自 [ccdarkness/huaweicloud](https://github.com/ccdarkness/huaweicloud)，一个 Node.js 实现的华为云盘同步脚本，验证了 REST API 方案的可行性。

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Tauri](https://img.shields.io/badge/Tauri-2.x-FFC131?logo=tauri)](https://v2.tauri.app/)
[![Rust](https://img.shields.io/badge/Rust-1.77+-DEA584?logo=rust)](https://www.rust-lang.org/)
[![macOS](https://img.shields.io/badge/macOS-12+-silver?logo=apple)](https://www.apple.com/macos/)

---

## 特性

| 模块 | 能力 |
|---|---|
| **授权登录** | OAuth 2.0 + PKCE（S256）；`openid profile drive` 全盘访问；Token 自动刷新 + Keychain 安全存储 |
| **网盘主界面** | 双栏布局（侧边栏递归目录树 + 文件列表）；面包屑导航；搜索；新建文件夹 |
| **文件操作** | 上传（≤20MB 单次 + >20MB 5MB 分片断点续传）、下载（流式原子写）、删除、重命名、移动、缩略图 |
| **双向同步** | 本地 FSEvents 实时监听 + 3s debounce；云端 BFS 全量索引 + 磁盘缓存（17K 文件启动 ~200ms）；三段式稳定性检查（mtime/size/lsof） |
| **后台运行** | 关闭窗口 / ⌘Q 不退出，仅隐藏 UI 层；系统菜单栏图标始终保留；同步引擎在后台持续运行 |
| **开机自启动** | 设置页开关，LaunchAgent plist（带 `--hidden` 参数，开机只显示菜单栏图标） |
| **冲突处理** | 自动重命名副本（60s 容忍窗口 + 副本去重 + 云端删除时保护本地修改） |
| **配置管理** | 集中设置页：OAuth 回调地址、挂载目录、并发数（默认 6）、debounce 时长（默认 3s）、跳过文件列表 |
| **传输队列** | 上传/下载进度实时展示；清除已完成/清除失败项 |
| **释放空间** | 双重安全校验（云端存在 + 本地已上传），防止误删未同步的修改 |
| **日志系统** | 三层输出（终端 + 滚动文件 + 环形缓冲）；日志查看导出页面 |

---

## 技术栈

| 层 | 技术 |
|---|---|
| **后端** | Rust + [Tauri 2.x](https://v2.tauri.app/) |
| **前端** | Vue 3 + TypeScript + Vite + Pinia |
| **数据库** | SQLite（rusqlite, bundled） |
| **HTTP** | reqwest（rustls-tls） |
| **文件监听** | notify（macOS FSEvents） |
| **安全存储** | keyring（macOS Keychain） |
| **UI 设计** | Mate 组件库 + 自建设计令牌（主色 `#0052D9`） |
| **日志** | tracing + tracing-appender |

---

## 前置要求

- macOS 12 Monterey 及以上
- [Rust](https://rustup.rs/) 1.77+
- [Node.js](https://nodejs.org/) 20+（推荐 24+）
- Xcode Command Line Tools（`xcode-select --install`）

---

## 快速开始

### 1. 克隆仓库

```bash
git clone git@github.com:yuanbaobaoo/PetalLink.git
cd PetalLink
```

### 2. 安装依赖

```bash
# Rust 依赖
cargo fetch

# 前端依赖
cd app && npm install && cd ..
```

### 3. 配置 OAuth 凭据

```bash
cp .env.example .env
```

编辑 `.env`，填入真实的 `HWCLOUD_CLIENT_ID` 和 `HWCLOUD_CLIENT_SECRET`（从 [AGC 控制台](https://developer.huawei.com/consumer/cn/service/josp/agc/index.html) 获取）。

### 4. 启动开发环境

```bash
cargo tauri dev
```

前端运行在 `http://localhost:1420`（HMR 热更新），Rust 后端自动编译启动。

---

## 测试

```bash
# Rust 单元测试（164 个）
cargo test --lib

# Rust 集成测试（wiremock 模拟华为 API，12 个）
cargo test --test drive_api_test
cargo test --test oauth_flow_test

# 全部 Rust 测试
cargo test

# Rust 代码质量检查（零警告）
cargo clippy -- -D warnings

# 前端类型检查
cd app && npm run type-check

# 前端测试（Vitest）
cd app && npm run test
```

测试覆盖范围：

| 类型 | 数量 | 覆盖模块 |
|---|---|---|
| Rust 单元测试 | 164 | auth / config / pkce / conflict / sync / stability / constants / drive |
| Rust 集成测试 | 12 | Drive API（7）+ OAuth 流程（5） |
| 前端 TS 类型检查 | — | 严格模式，零错误 |

---

## 构建 & 打包

### 开发构建（仅前端）

```bash
cd app && npm run build    # 类型检查 + Vite 打包
```

### 生产发布（.app + DMG）

```bash
# 编译期注入 client_id 和 client_secret（两者均无默认值，必须提供）
TAURI_CLIENT_ID=<你的AGC_CLIENT_ID> TAURI_CLIENT_SECRET=<你的64位hex> cargo tauri build
```

**产物位置**：

```
target/release/bundle/macos/PetalLink.app    # macOS 应用包
target/release/bundle/dmg/PetalLink_1.0.0_aarch64.dmg    # DMG 安装镜像 (~3.2MB)
```

**打包配置**：
- Release profile：`panic=abort`、`lto=true`、`opt-level=s`、`strip=true`
- 应用图标由 `build.rs` 自动从 `assets/` 生成多分辨率 PNG + `icon.icns`
- 最低 macOS 版本：12.0
- 架构：Apple Silicon (arm64) + Intel (x86_64) Universal Binary

---

## 应用启动流程

1. 启动 App → 登录页 → 点「使用华为账号登录」→ 浏览器打开华为授权页
2. 在浏览器中完成授权（账号密码或手机扫码）→ 自动回到 App
3. **主界面顶部出现「尚未配置同步目录」提示条** → 点「选择目录」→ 选一个空目录
4. 提示条变为「同步索引」按钮 → 点击开始首次云端索引拉取（拉取全量文件树到本地）
5. 完成后自动进入「双端对齐」模式：本地文件变更实时上传，云端的变更通过手动点「同步索引」更新

**后台运行**：
- 启动后系统菜单栏右上角出现云朵图标，鼠标悬停显示「PetalLink — 后台同步中」
- 关闭窗口 / ⌘W / ⌘Q → **仅隐藏 UI 层**（窗口 + Dock 图标消失），同步引擎继续运行
- 点击菜单栏图标 →「显示主窗口」→ UI 恢复
- **菜单栏「退出 PetalLink」是真正终止进程的唯一路径**

---

## 项目结构

```
PetalLink/
├── src/                         # Rust 后端（Tauri）
│   ├── main.rs                  # 入口
│   ├── lib.rs                   # 应用装配 + 命令注册 + setup
│   ├── commands.rs              # 42 个 Tauri 命令
│   ├── auth/                    # OAuth + PKCE + Keychain
│   ├── drive/                   # 华为 Drive REST API 客户端
│   ├── sync/                    # 同步引擎（planner/executor/conflict/stability）
│   ├── mount/                   # 本地镜像 + FSEvents 监听
│   ├── data/                    # SQLite 数据层
│   ├── core/                    # 配置/日志/缓存
│   └── platform/                # macOS 原生（托盘/activation/开机自启）
├── app/                         # Vue3 前端
│   ├── views/                   # 页面（Login/Main/Settings/LogViewer）
│   ├── stores/                  # Pinia 状态管理
│   ├── api/                     # Tauri invoke 封装
│   ├── components/mate/         # Mate 组件库（20+ 组件）
│   └── styles/                  # 设计令牌
├── assets/                      # 品牌图标资源（唯一图源）
├── design/prototype/            # UI 设计原型
├── tests/                       # 集成测试（wiremock）
├── docs/                        # 需求文档 + 概要设计 + API 整理
├── Cargo.toml                   # Rust 依赖
├── tauri.conf.json              # Tauri 配置
└── build.rs                     # 构建脚本（图标自动同步）
```

---

## 文档

| 文档 | 说明 |
|---|---|
| [docs/概要设计文档.md](./docs/概要设计文档.md) | 项目架构、功能需求、数据流、API 接口、设计决策 |
| [docs/api调用整理.md](./docs/api调用整理.md) | 华为 Drive REST API 完整清单（20 个接口） |
| [ai-context/design-rules.md](./ai-context/design-rules.md) | UI 设计规范（Mate 组件库 + 设计令牌体系） |
| [design/prototype/](./design/prototype/) | UI 设计原型（login / main / settings） |

---

## License

[Apache License 2.0](LICENSE) © 2026 PetalLink

本项目仅供个人学习与自用。华为云空间服务及相关 API 归华为所有，请遵守华为开发者联盟相关协议。使用本项目产生的任何数据丢失、账号封禁等问题，项目维护者不承担责任。

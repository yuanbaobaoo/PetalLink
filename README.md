<p align="center"><img alt="PetalLink" src="assets/logo.png" width="80"></p>

# PetalLink

华为云盘 Mac 客户端 —— 基于 Tauri 2.x，将华为云空间挂载到本地，双向实时同步。

> 华为云空间目前并不支持 macOS。PetalLink 通过华为 Drive REST API 直连，不依赖 HMS Core SDK，为 macOS 用户提供接近原生的云盘体验。

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Tauri](https://img.shields.io/badge/Tauri-2.x-FFC131?logo=tauri)](https://v2.tauri.app/)
[![Rust](https://img.shields.io/badge/Rust-1.77+-DEA584?logo=rust)](https://www.rust-lang.org/)
[![macOS](https://img.shields.io/badge/macOS-12+-silver?logo=apple)](https://www.apple.com/macos/)

---

## 特性

| 模块 | 能力 |
|---|---|
| **授权登录** | OAuth 2.0 + PKCE（S256）；`openid profile drive` 全盘访问；Token 自动刷新 + 机器码绑定的 `token.bin` 加密存储 |
| **网盘主界面** | 双栏布局（侧边栏递归目录树 + 文件列表）；面包屑导航；搜索；新建文件夹 |
| **文件操作** | 上传（≤20MB multipart + >20MB 分片续传）、Range 断点下载、删除、重命名、移动、缩略图；写操作按 fileId 核验后才结算 |
| **双向同步** | 本地 FSEvents + 3s debounce；华为 Changes 增量同步 + BFS 兜底；tree/path/cursor 原子可信 checkpoint；三段式稳定性检查 |
| **后台运行** | 关闭窗口 / ⌘W 不退出，仅隐藏 UI 层（Dock 图标消失）；托盘图标可见时 ⌘Q 同样隐藏至后台；系统菜单栏图标可开关；同步引擎在后台持续运行 |
| **开机自启动** | 设置页开关，LaunchAgent plist（带 `--hidden` 参数，开机只显示菜单栏图标） |
| **冲突处理** | 自动重命名副本（60s 容忍窗口 + 副本去重 + 云端删除时保护本地修改） |
| **配置管理** | 集中设置页：OAuth 回调地址、挂载目录、并发数（默认 6）、debounce 时长（默认 3s）、跳过文件列表、是否显示托盘图标 |
| **传输队列** | 持久化状态机；上传/下载排队执行，主页删除完成后留痕可见；等待网络、退避、远端核验、需重新规划、永久失败分开展示；网络恢复自动续跑 |
| **释放空间** | 支持文件与目录（递归子树）；执行前弹窗列出可释放文件名与大小供二次确认；逐项复核可信云树、远端 fileId、成功基线、本地 mtime/size 与活动任务，防止 TOCTOU 误删 |
| **日志系统** | 三层输出（终端 + 滚动文件 + 环形缓冲）；日志查看导出页面 |

断网或频繁网络抖动时，未完成任务不会被当成永久失败：上传分片恢复前先查询服务端确认偏移，下载保留带版本身份的 `.tmp` 并用 `Range` 继续；响应丢失的创建、更新和删除先核验远端结果，禁止盲目重放。离线启动只加载 checkpoint 作为增量基线，在 Changes 追平前不会恢复上传、执行删除或进入同步规划。

---

## 技术栈

| 层 | 技术 |
|---|---|
| **后端** | Rust + [Tauri 2.x](https://v2.tauri.app/) |
| **前端** | Vue 3 + TypeScript + Vite + Pinia |
| **数据库** | SQLite（rusqlite, bundled） |
| **HTTP** | reqwest（rustls-tls） |
| **文件监听** | notify（macOS FSEvents） |
| **安全存储** | `token.bin` + ChaCha20-Poly1305 AEAD（密钥由本机 IOPlatformUUID 派生） |
| **UI 设计** | Mate 组件库 + 自建 design token v2（主色 `#0053DB`，详见 ai-context/design-rules.md） |
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
cargo tauri dev --config tauri.dev.conf.json
```

前端运行在 `http://localhost:1420`（HMR 热更新），Rust 后端自动编译启动。

---

## 测试

```bash
# 查看当前测试清单（不在文档中维护固定数量）
cargo test -- --list

# 编译全部 target 与测试，但不执行
cargo test --all-targets --no-run

# 全部 Rust 测试
cargo test

# 真实云端手工上传测试（默认忽略）
HWCLOUD_TEST_FILE="<file_path>" cargo test --test upload_tester -- --ignored --nocapture
```

测试集中在根目录 `tests/`，覆盖以下核心合同：

| 类型 | 覆盖模块 |
|---|---|
| 核心合同测试 | auth / config / paths / logging / platform / error contract |
| Drive 协议测试 | Files / Changes / download / upload / ASCII JSON / model parsing / client error |
| 同步测试 | cloud tree / conflict / engine / path recovery / retry policy / stability / state / task recovery / transfer state / checkpoint store |
| 手工集成测试 | `upload_tester.rs`，需要真实 OAuth 环境和 `HWCLOUD_TEST_FILE`，默认 `#[ignore]` |

---

## 构建 & 打包

### 开发构建（仅前端）

```bash
cd app && npm run build    # 类型检查 + Vite 打包
```

### 开发打包（.app + DMG，带 Dev 后缀）

```bash
cargo tauri build --debug --config tauri.dev.conf.json
```

产物带 `-Dev` 后缀，与正式版共存互不影响（不生成更新包）：

```
target/debug/bundle/macos/PetalLink-Dev.app
target/debug/bundle/dmg/PetalLink-Dev_<version>_aarch64.dmg
```

### 生产发布（.app + DMG）

```bash
# 编译期自动读取 .env 文件注入凭据（无需手动设置环境变量）
cargo tauri build
```

**产物位置**：

```
target/release/bundle/macos/PetalLink.app    # macOS 应用包
target/release/bundle/dmg/PetalLink_<version>_aarch64.dmg    # DMG 安装镜像 (~3.2MB)
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
- 启动后系统菜单栏右上角出现云朵图标（可关闭），鼠标悬停显示「PetalLink — 后台同步中」
- 关闭窗口 / ⌘W → **仅隐藏 UI 层**（窗口 + Dock 图标消失），同步引擎继续运行
- 托盘可见时 ⌘Q → 同样隐藏至后台；托盘隐藏时 ⌘Q → 真正退出
- 点击菜单栏图标 →「显示主窗口」→ UI 恢复
- **菜单栏「退出 PetalLink」→ 真正退出进程**

---

## 项目结构

```
PetalLink/
├── src/                         # Rust 后端（Tauri）
│   ├── main.rs                  # 入口
│   ├── lib.rs                   # 应用装配 + 命令注册 + setup
│   ├── commands.rs              # 命令运行时与统一导出
│   ├── commands/                # Tauri 命令按领域拆分
│   ├── auth/                    # OAuth + PKCE + token.bin 加密存储
│   ├── drive/                   # 华为 Drive REST API 客户端（Files/Upload 已按职责拆分）
│   ├── sync/                    # 同步引擎（engine/executor/TaskRunner 均按职责拆分）
│   ├── mount/                   # 本地镜像 + FSEvents 监听
│   ├── data/                    # SQLite 数据层
│   ├── core/                    # 配置/日志/缓存
│   └── platform/                # macOS 原生（托盘/activation/开机自启）
├── app/                         # Vue3 前端
│   ├── views/                   # 页面（Login/Main/Settings/LogViewer）
│   ├── stores/                  # Pinia 状态管理
│   ├── api/                     # Tauri invoke 封装
│   ├── components/mate/         # Mate 组件库
│   └── styles/                  # design token
├── assets/                      # 品牌图标资源（唯一图源）
├── design/prototype/            # UI 设计原型
├── tests/                       # 集成测试
│   └── upload_tester.rs         # 默认忽略的真实云端手工集成测试
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
| [docs/api调用整理.md](./docs/api调用整理.md) | 华为 Drive REST API 完整清单（23 个调用场景，含分片上传 308/Location 详解） |
| [design/prototype/](./design/prototype/) | UI 设计原型（login / main / settings） |

---

## License

[Apache License 2.0](LICENSE) © 2026 PetalLink

本项目仅供个人学习与自用。华为云空间服务及相关 API 归华为所有，请遵守华为开发者联盟相关协议。使用本项目产生的任何数据丢失、账号封禁等问题，项目维护者不承担责任。

---

## 相关项目

[ccdarkness/huaweicloud](https://github.com/ccdarkness/huaweicloud)：一个 Node.js 实现的华为云盘同步脚本，曾验证了 REST API 方案的可行性。

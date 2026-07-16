# PetalLink — Kotlin / Kuikly 重构工作目录

> 本目录是 **PetalLink**（华为云盘客户端）从 **Tauri 2.x (Rust + Vue3)** 迁移到 **Kotlin + Kuikly** 的重构工作目录。
> 原始项目位于 `/Users/Shared/codes/personal/petal-link`（Rust 后端 + Vue3 前端）。
> 本目录已整理出迁移所需的全部上下文信息，后续开发以此目录为工作根。

---

## 这是什么

PetalLink 是一个把华为云空间挂载到本地目录、双向实时同步的桌面客户端。华为云空间官方未提供 macOS 版本，PetalLink 通过华为 Drive REST API 直连（不依赖 HMS Core SDK），实现接近原生的云盘体验。

当前实现（被重构对象）：

| 层 | 技术 | 规模 |
|---|---|---|
| 后端 | Rust + Tauri 2.x | 99 个源文件，约 2.7 万行 |
| 前端 | Vue 3 + TypeScript + Vite + Pinia | 约 8000 行 |
| IPC | Tauri invoke | 49 个命令 + 4 个事件广播 |
| 存储 | SQLite（rusqlite, bundled）schemaVersion=5 | sync_items + transfer_queue |
| 安全 | ChaCha20-Poly1305 AEAD（机器码绑定的 token.bin） | |

---

## 目录导航

```
petal-link-kuikly/
├── README.md                          # ← 本文件：工作目录导航
├── settings.gradle.kts                # Gradle 根配置（KMP 模块声明）
├── build.gradle.kts                   # Gradle 根构建（插件 apply false）
├── gradle.properties                  # Kotlin/Gradle 配置
├── gradle/libs.versions.toml          # 版本目录（Kotlin 2.1.21 + 协程/序列化/Ktor）
├── gradlew, gradlew.bat               # Gradle wrapper（9.0.0，支持 JDK 25）
├── shared/                            # KMP 共享模块（核心业务逻辑）
│   ├── build.gradle.kts               # macosArm64/macosX64 + cocoapods → PetalLink.framework
│   ├── native.def/xattr.def           # cinterop：sys/xattr.h
│   └── src/
│       ├── commonMain/                # 跨平台（华为 API / 九态状态机 / inode 身份 / DDL）
│       └── macosMain/                 # macOS 平台实现（xattr / inode / FSEvents / 加密）
├── macosApp/                          # macOS 宿主壳（XcodeGen + CocoaPods + Kuikly 渲染）
│   ├── project.yml                    # XcodeGen 工程描述
│   ├── Podfile                        # pod 'OpenKuiklyIOSRender' + pod 'PetalLink'
│   └── macosApp/AppDelegate.swift     # 启动入口（TODO: 接入 Kuikly 渲染）
├── docs/                              # 迁移所需的核心信息文档（由原始项目提炼）
│   ├── 01-项目总览.md                  # 产品定位、技术栈、架构、启动流程
│   ├── 02-功能需求清单.md              # F-AUTH/F-UI/F-FILE/F-MOUNT/F-CONFLICT 全量需求
│   ├── 03-华为Drive-API接口规范.md         # 全部 23 个 API 场景 + 18 条踩坑清单
│   ├── 04-数据模型与持久化.md          # SQLite schema v6 + Rust struct + 前端类型映射
│   ├── 05-IPC命令与事件.md             # 49 个 Tauri 命令签名 + 4 个事件广播
│   ├── 06-同步引擎与传输状态机.md      # 3-way diff / planner / executor / TaskRunner / 九态状态机
│   ├── 07-安全与OAuth与占位符.md       # OAuth+PKCE / token.bin / xattr 占位模型
│   ├── 08-前端API与Store接口规范.md       # 前端 API 层 + Pinia stores + 类型定义
│   ├── 09-设计系统.md                  # 色彩/排版/间距/组件/布局（TDesign × macOS）
│   ├── 10-Kuikly重构迁移指南.md        # Rust→Kotlin、Vue→Kuikly 的逐模块映射与建议
│   └── 11-基于inode的文件身份识别方案.md  # ★ 目标架构：inode 替代 fileId xattr（净减 ~520 行）
├── ai-context/                        # 编码与设计规则（从原项目复制，重构需继续遵守）
│   ├── coding-rules.md
│   └── design-rules.md
└── reference/                         # 原始文档归档（只读参考，不直接修改）
    ├── 概要设计文档.md                 # 原项目 docs/概要设计文档.md
    └── api调用整理.md                  # 原项目 docs/api调用整理.md
```

---

## 推荐阅读顺序（开始重构前）

1. **`docs/01-项目总览.md`** —— 先建立全局认知
2. **`docs/02-功能需求清单.md`** —— 明确要重建哪些功能
3. **`docs/03-华为Drive-API接口规范.md`** —— 这是整个项目最难、最易踩坑的部分，务必先读
4. **`docs/06-同步引擎与传输状态机.md`** —— 核心业务逻辑
5. **`docs/10-Kuikly重构迁移指南.md`** —— 技术栈映射与迁移策略
6. 其余文档按需查阅

---

## 关键约束（重构必须保留）

> 以下约束均源自原项目源码逐行核对，数值为精确实现值。

1. **华为 API 18 条踩坑**：scope 的 `/` 不编码、authorization code 的 `+` 必须 `%2B` 编码（手工拼 form body）、中文文件名 `\uXXXX` 转义（含 UTF-16 代理对）、`multipart/related`（非 form-data）、308 rangeList 连续性校验、`nextCursor` vs `newStartCursor` 语义不同、配额 String 容忍等——详见 `docs/03`。
2. **数据安全**：写操作核验远端 `200 + File` 后才结算；响应丢失按 fileId GET 核验；上传恢复迁移 VerifyingRemote **绝不推算 offset**（只按服务端 rangeList 确认前进）；下载恢复只认 `.tmp` 实际长度；启动恢复固定 8 步顺序——详见 `docs/06`。
3. **九态状态机**：`can_transition` 严格校验 + CAS revision（`WHERE state_revision=?` + 1）；退避算法 `2^attempt` 秒上限 300s + jitter，`MAX_AUTOMATIC_ATTEMPTS=5`——详见 `docs/06`。
4. **占位符与文件身份（inode 方案）**：xattr 仅 **2 个键**（`com.hwcloud.state` + `com.apple.FinderInfo`）；文件身份由 **`local_inode_map` 表**（inode→fileId 映射，schemaVersion=6）承担，替代原 fileId xattr（复制 bug 从结构上消除，净减 ~520 行）；`com.hwcloud.state` 是占位状态唯一权威判据；Finder 灰标直接写 xattr（`buf[9]=0x02`）；无 state xattr 文件视为用户文件绝不转换——详见 `docs/11`（方案）/`docs/04`/`docs/07`。
5. **文件监听**：3s debounce + **2s warmup**（非 8s，防历史回放误吞用户删除）+ 纯事件驱动无轮询——详见 `docs/02`/`docs/06`。
6. **稳定性检查**：mtime>5s + size 稳定 3s + lsof（白名单 **10 个**只读系统进程，双重检查 1s）；持续编辑 >5min 标记 Editing——详见 `docs/06`。
7. **网络守卫**：TCP 探测华为域名 443，30s 间隔 3s 超时，**连续 2 次成功才转 Online**（防抖）；恢复固定顺序 cloud catch-up → VerifyingRemote → planner——详见 `docs/06`。
8. **内部文件隔离**：`.hwcloud_` 前缀全局硬编码过滤，绝不参与同步。
9. **单实例 + macOS 生命周期**：swizzle `NSApplication terminate:` 拦截 Dock/Cmd+Q；检测 Apple Event 区分系统关机放行；accessory 模式窗口恢复须切 regular——详见 `docs/10`。
10. **中文注释、英文标识符**：遵循 `ai-context/coding-rules.md`。

---

## 构建骨架（KMP + Kuikly macOS）

骨架采用 KuiklyUI 官方 macOS 路线：**KMP framework + CocoaPods + Xcode 宿主壳**。

### 编译共享模块

```bash
# 验证 KMP 编译（macosArm64 + macosX64）
./gradlew :shared:compileKotlinMacosArm64 :shared:compileKotlinMacosX64

# 生成 PetalLink.framework（供 CocoaPods 集成）
./gradlew :shared:linkPodDebugFrameworkMacosArm64
```

> Gradle 9.0.0 + Kotlin 2.1.21。环境仅需 JDK（本机 Java 25 已验证通过）。

### 构建 macOS 应用

```bash
# 前置：克隆 KuiklyUI 渲染层源码
git clone https://github.com/Tencent-TDS/KuiklyUI.git ../KuiklyUI

# 生成 Xcode 工程 + 安装依赖
cd macosApp && xcodegen generate && pod install

# 用 Xcode 打开运行
open macosApp.xcworkspace
```

详见 `macosApp/README.md`。

---

## 原始项目快速命令（参考）

```bash
# 原项目开发
cd /Users/Shared/codes/personal/petal-link
cargo tauri dev --config tauri.dev.conf.json

# 原项目测试
cargo test                    # Rust 集成测试（根目录 tests/）
cd app && npm run test        # 前端 Vitest 测试（app/tests/）
```

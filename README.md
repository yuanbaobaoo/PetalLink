# PetalLink

华为云空间 macOS 桌面客户端。PetalLink 通过华为 Drive REST API 直连云端，将云端文件映射到本地目录，并提供可恢复的双向同步。

> **⚠️ `dev/cmp-kotlin-toolchain` 已冻结，暂停维护。Kotlin/Compose Multiplatform 迁移技术验证已完成——应用可编译、运行并通过测试，但 Kotlin Toolchain 生态尚不成熟（Compose Desktop 打包、热更新、IDE 集成等方面），继续投入性价比较低。代码保留，待 Kotlin Toolchain 迭代后再评估是否继续。

本分支是 Kotlin/Compose Multiplatform Desktop JVM 移植版本：以 Kotlin Toolchain 为唯一构建入口，不要求预装 Gradle 或 JDK。

> 华为云空间目前不原生支持 macOS。PetalLink 不依赖 HMS Core SDK，而是使用 OAuth 2.0 和华为 Drive REST API 提供桌面端云盘体验。

## 特性

| 模块 | 能力 |
|---|---|
| 授权登录 | OAuth 2.0 + PKCE（S256）、loopback 回调、state 校验、Token 自动刷新与本地安全存储 |
| 网盘界面 | 侧边栏递归目录树、面包屑、文件列表、搜索、新建文件夹、缩略图与文件属性 |
| 文件操作 | 上传、Range 断点下载、删除、重命名、移动；写操作必须经过远端核验后才结算 |
| 双向同步 | FSEvents 本地监听、3 秒 debounce、Changes 增量同步、BFS 全量兜底、可信 checkpoint |
| 传输队列 | 持久化九态状态机；等待网络、退避、远端核验、重新规划和永久失败分别展示；网络恢复后自动续跑 |
| 冲突与安全 | inode 身份映射、冲突副本、上传稳定性检查、路径与占位符保护、释放空间二次核验 |
| 后台运行 | 关闭窗口后隐藏 UI 并持续同步；菜单栏图标保留；只有菜单栏“退出 PetalLink”真正结束进程 |
| macOS 集成 | LaunchAgent 开机自启动、Finder xattr 占位标记、JNA/JNI 原生能力、DMG 打包与签名公证发布链 |
| 日志与更新 | 终端、滚动文件和内存缓冲日志；内置版本检查、下载、校验、等待传输完成和安全替换流程 |

离线或网络抖动不会把未完成传输误判为永久失败：上传恢复以服务端确认的偏移为准，下载保留 `.tmp` 和版本 sidecar 继续传输；创建、更新和删除在响应不确定时先核验远端状态，禁止盲目重放。

## 技术栈

| 层 | 技术 |
|---|---|
| 构建入口 | [Kotlin Toolchain](https://kotlin-toolchain.org/) 0.11.1 |
| 应用与 UI | Kotlin 2.3.20、Compose Multiplatform Desktop 1.11.1、JVM 25 |
| 共享架构 | Kotlin Multiplatform：`shared` 业务/数据层 + `jvm-app` 桌面入口 |
| 网络与并发 | Ktor CIO、Kotlin Coroutines、Kotlin Serialization |
| 本地数据 | Room KMP + bundled SQLite |
| macOS 原生能力 | JNA、JVM FSEvents/xattr/lsof 集成 |
| 打包 | Compose Desktop、jpackage；DMG、Developer ID 签名和 Apple notarization 由内部 `build-plugin` 兼容桥调用 |

## 前置要求

- macOS 12 Monterey 及以上（应用目标平台）
- Git
- 首次执行时的网络连接：wrapper 会下载锁定版本的 Kotlin Toolchain、JDK 25 和项目依赖

不需要安装 Gradle、Gradle Wrapper 或系统 JDK，也不需要设置 `JAVA_HOME`。

生成签名发布包时还需要 Apple Developer ID、notary 凭据和 Xcode Command Line Tools；普通开发、编译、测试和无签名 DMG 不需要手动配置 JDK。

## 快速开始

### 1. 克隆并配置 OAuth

```bash
git clone git@github.com:yuanbaobaoo/PetalLink.git
cd PetalLink
cp .env.example .env
```

编辑 `.env`，填写从 AppGallery Connect 获取的凭据：

```dotenv
HWCLOUD_CLIENT_ID=<你的 client_id>
HWCLOUD_CLIENT_SECRET=<你的 client_secret>
```

开发环境使用回调地址 `http://127.0.0.1:9999/oauth/callback`；创建 OAuth 客户端时还需开通云空间 API。`.env` 已被忽略，不应提交。

### 2. 编译、测试和运行

```bash
./kotlin build               # 编译全部模块
./kotlin test                # 运行全部 JVM 单元、合同和集成测试
./kotlin run                 # 启动桌面应用
```

Windows 使用 `kotlin.bat`。首次运行会较慢；之后 Toolchain、JDK 和依赖均复用用户缓存。

`./kotlin run` 默认使用 dev 档案：bundle id 为 `io.github.yuanbaobaoo.PetalLink-dev`，与正式版的数据目录和 LaunchAgent 隔离。开发版首次启动需要登录，并应选择与正式版不同的同步目录。

### Compose Hot Reload

终端普通运行不会自动热更新：

```bash
./kotlin run
```

可以用下列命令启用 Compose Hot Reload 运行模式：

```bash
./kotlin run --compose-hot-reload-mode
```

但 Kotlin Toolchain CLI 目前不监听文件系统，因此“保存即刷新”需要在 IntelliJ IDEA 中完成：安装 Kotlin Toolchain 插件，选择 **Run with Compose Hot Reload**。详情见 [Compose Multiplatform 的官方说明](https://kotlin-toolchain.org/dev/user-guide/builtin-tech/compose-multiplatform/)。

## 测试

```bash
./kotlin test
```

测试覆盖 OAuth/Drive HTTP 合同、传输状态机、同步规划、冲突处理、Room 持久化、inode、FSEvents、xattr、占位符、更新和 JVM 集成路径。测试使用临时数据目录，不写入用户真实的 Application Support 目录。

测试通过代表已覆盖的逻辑合同自洽；真实华为账号、Drive 写入限流、DMG 覆盖安装、托盘生命周期和 Apple 签名公证仍需按发布验收矩阵人工验证。

## 构建与发布

```bash
./kotlin do packageDmg       # 构建本地 dev 无签名 DMG
./kotlin do releaseDmg       # 签名、公证并构建正式 release DMG
```

日常开发、CI 和发布均只从 `./kotlin` 进入。Kotlin Toolchain 的构建产物位于 `.kotlin/build/`；Compose Desktop 分发产物位于 `build/compose/binaries/`。

`packageDmg` 和 `releaseDmg` 是 `build-plugin` 提供的兼容命令：当前 Kotlin Toolchain 尚未原生提供 Compose Desktop 的 DMG、Developer ID 签名和 Apple notarization 任务，因此该插件在内部调用打包桥接层。它不是业务模块，也不应在 IDEA 中作为 Gradle 项目导入。

CI 在 macOS 上执行 `./kotlin test` 和 `./kotlin do packageDmg`；带 `v<version>` 标签的 Release 工作流执行签名、公证、staple、更新清单生成和 GitHub Release 发布。应用版本唯一来源是 [version.properties](./version.properties)。

## IntelliJ IDEA

1. 安装并启用 Kotlin Toolchain 插件。
2. 直接打开仓库根目录，让 `project.yaml` 与各模块 `module.yaml` 建立项目模型。
3. 不要把根目录或 `build-plugin/.desktop-packaging` 链接为 Gradle 项目。
4. 可将 `build-plugin/.desktop-packaging` 在 Project 视图中标记为 `Excluded`；这是内部打包兼容目录，不是业务源码。

## 项目结构

```text
petal-link-cmp/
├── jvm-app/                     # Kotlin Toolchain 的桌面应用入口
├── shared/
│   ├── src/                      # 业务、Drive API、Room、同步与 ViewModel
│   ├── test/                     # 跨平台纯逻辑测试
│   ├── src@jvm/                  # Compose Desktop 与 macOS JVM 实现
│   ├── test@jvm/                 # JVM/macOS 集成测试
│   └── resources@jvm/            # 桌面资源和签名配置
├── build-plugin/                 # BuildInfo 与内部 DMG 打包兼容桥
├── docs/
│   ├── plan/                     # 当前有效设计、审计和实施计划
│   └── reference/                # 原 Tauri 项目只读参考资料
├── project.yaml                  # Kotlin Toolchain 根模块配置
├── version.properties            # 应用版本唯一真相源
├── kotlin / kotlin.bat           # 唯一用户构建入口
└── .env.example                  # OAuth 本地开发配置模板
```

## 文档

| 文档 | 说明 |
|---|---|
| [项目与文档导航](./docs/plan/00-项目与文档导航.md) | 当前项目目标、工程结构和构建入口 |
| [功能需求清单](./docs/plan/02-功能需求清单.md) | 用户可见功能与验收基线 |
| [同步引擎与传输状态机](./docs/plan/06-同步引擎与传输状态机.md) | 同步、恢复、状态机和安全约束 |
| [安全与 OAuth 与占位符](./docs/plan/07-安全与OAuth与占位符.md) | OAuth、Token、路径、xattr 和日志安全 |
| [设计系统](./docs/plan/09-设计系统.md) | UI 组件与设计令牌规范 |
| [主题与皮肤](./docs/主题与皮肤.md) | 字体、颜色、尺寸与新增皮肤方式 |
| [当前实现审计](./docs/plan/11-当前实现审计.md) | 已完成能力、风险和人工验收项 |
| [发布与兼容验收](./docs/plan/13-发布与兼容验收.md) | CI、Release、DMG 和兼容性验收矩阵 |

## 相关项目

- [原 Tauri 实现](https://github.com/yuanbaobaoo/PetalLink)：本分支的迁移基线和协议行为参考。
- [ccdarkness/huaweicloud](https://github.com/ccdarkness/huaweicloud)：曾验证华为 Drive REST API 可行性的 Node.js 同步脚本。

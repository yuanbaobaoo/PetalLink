# CMP JVM 迁移设计：Kuikly Kotlin/Native → Compose Multiplatform JVM

> 日期：2026-07-16
> 状态：待评审
> 关联：替换之前的 Kuikly KMP 方案

---

## 一、目标

将 PetalLink 从 **Kotlin/Native macOS + Kuikly UI** 全量迁移到 **JVM + Compose Multiplatform Desktop + JNA**。

### 动机

1. **Kuikly core 无 macOS native variant**（只有 iOS/JVM），导致 UI 层只能用自建 facade 桩而非真实框架，无法真正渲染运行。
2. **Kotlin/Native cinterop 复杂**：FSEvents/xattr/Foundation 的 cinterop 绑定反复出现编译问题，开发效率低。
3. **JVM 生态成熟**：Compose Multiplatform Desktop 是官方支持的桌面 UI 框架，JVM 原生 API（java.nio/java.security/ProcessBuilder）稳定可靠。
4. **JNA 保留原生体验**：macOS 特有功能（NSStatusItem 托盘、FSEvents 文件监听、xattr 扩展属性、LaunchAgent 自启动）通过 JNA 调用原生 API，不牺牲体验。

### 不变的部分

- **commonMain 业务逻辑**（60+ 文件）：auth/drive/sync/mount/data/config/error/logging/net_guard 等纯逻辑层**完全保留**，仅 `expect` 声明的平台 API 实现从 macosMain 改为 jvmMain。
- **docs/ 文档**：不变。
- **gradle wrapper**：不变（Gradle 9.0.0 + Kotlin 2.1.21）。

---

## 二、架构变更

| 维度 | 现在（Kuikly） | 改为（CMP JVM） |
|---|---|---|
| **构建** | KMP macosArm64/macosX64 + cocoapods | KMP jvm() + Compose Multiplatform |
| **UI** | Kuikly facade (Pager/@Page/DSL) | Compose `@Composable` |
| **平台 API** | cinterop (xattr/fsevents/POSIX/Foundation) | JNA (AppKit/CoreServices) + JVM (java.nio/java.security) |
| **宿主壳** | macosApp/ (XcodeGen + Podfile + Xcode) | jvmMain main() + Compose Desktop Window |
| **产物** | PetalLink.framework（CocoaPods） | 可执行 JAR / macOS .app（jpackage） |

---

## 三、详细变更

### 3.1 构建系统

**`shared/build.gradle.kts`**：
- 删除 `macosArm64{}`/`macosX64{}` + cinterop 配置
- 删除 `cocoapods{}` 块
- 添加 `jvm()` target
- 添加 Compose Multiplatform 插件 + `compose.desktop` 依赖
- 添加 JNA 依赖（`net.java.dev.jna:jna` + `net.java.dev.jna:jna-platform`）
- 删除 SQLDelight native driver → 改用 SQLDelight JDBC driver（sqlite-jdbc）

**`gradle/libs.versions.toml`**：
- 删除 kuikly-annotations
- 删除 ktor-darwin
- 删除 sqldelight-native
- 添加 compose-multiplatform
- 添加 sqldelight-jvm（sqlite-jdbc driver）
- 添加 jna、jna-platform

### 3.2 删除的文件/目录

| 路径 | 说明 |
|---|---|
| `shared/src/commonMain/kotlin/com/tencent/kuikly/` | 9 个 Kuikly facade 文件 |
| `shared/native.def/` | xattr.def + fsevents.def |
| `shared/src/macosMain/` | 全部 20 个 cinterop 平台文件 |
| `macosApp/` | Xcode 宿主壳（project.yml/Podfile/AppDelegate.swift/Info.plist） |

### 3.3 新增的文件

**`jvmMain/`**（平台实现，替代 macosMain）：

| 文件 | 实现方式 | 对标原 macosMain |
|---|---|---|
| `config/ConfigStore.kt` | java.nio.file Files.readAllBytes/writeString | POSIX open/read/write |
| `core/net_guard/NetGuard.kt` | java.net.Socket connect(host,443) + 3s timeout | gethostbyname |
| `data/PetalLinkDb.kt` | SQLDelight JdbcSqliteDriver | NativeSqliteDriver |
| `data/repository/*Impl.kt` | 同接口（不变） | 同 |
| `drive/PlatformTime.kt` | System.currentTimeMillis() / System.nanoTime() | posix time() |
| `platform/Crypto.kt` | 保留纯 Kotlin ChaCha20Poly1305（已有）+ java.security.KeyDerivation | cinterop |
| `platform/FsEvents.kt` | JNA FSEvents 或 java.nio.file.WatchService | enumerator 轮询 |
| `platform/Inode.kt` | java.nio.file.Files.getAttribute("unix:ino") | posix stat |
| `platform/Xattr.kt` | JNA setxattr/getxattr/removexattr | cinterop sys/xattr.h |
| `platform/TrayManagerImpl.kt` | JNA AppKit NSStatusItem + NSMenu | 同 |
| `platform/ActivationManagerImpl.kt` | JNA AppKit NSApplication | 同 |
| `platform/LaunchAtLoginImpl.kt` | ProcessBuilder launchctl | popen |
| `platform/ShutdownManagerImpl.kt` | 同逻辑（withTimeout） | 同 |
| `platform/SingleInstanceGuardImpl.kt` | java.nio.channels.FileLock | posix flock |
| `sync/engine/PlatformFileOps.kt` | java.nio.file Files.move/delete | posix rename/remove |

**`jvmMain/kotlin/.../Main.kt`**（应用入口）：
```kotlin
fun main() = application {
    Window(title = "PetalLink", ...) {
        App()  // Compose 根组件
    }
}
```

### 3.4 UI 层重写

**删除**：4 个 Kuikly 页面（LoginPage/MainPage/SettingsPage/LogViewerPage）+ MateComponents + HomePage + PetalLinkPages

**新增**（Compose `@Composable`）：

| 文件 | 说明 |
|---|---|
| `ui/App.kt` | 根组件，根据登录状态切换 Login/Main |
| `ui/pages/LoginScreen.kt` | 登录页（Button + Text） |
| `ui/pages/MainScreen.kt` | 主页（Sidebar + FileList + TransferPopover） |
| `ui/pages/SettingsScreen.kt` | 设置页（表单） |
| `ui/pages/LogViewerScreen.kt` | 日志查看页 |
| `ui/components/MateComponents.kt` | Compose 版 Mate 组件（Button/TextField/Tag/Dialog 等） |
| `ui/theme/Theme.kt` | Compose 主题（MaterialTheme + 自定义配色） |

**保留**：ViewModel（AuthViewModel/SyncViewModel 等）+ DesignTokens（配色常量）

### 3.5 expect/actual 适配

commonMain 中的 `expect` 声明不变，actual 从 `macosMain` 改为 `jvmMain`：

| expect（commonMain） | actual（jvmMain） |
|---|---|
| `expect class ConfigStore()` | java.nio.file |
| `expect class NetGuard()` | java.net.Socket |
| `expect class PetalLinkDb(path)` | SQLDelight JdbcSqliteDriver |
| `expect fun platformName()` | "JVM (macOS)" |
| `expect object PlatformInode` | Files.getAttribute |
| `expect fun platformRenameExpect()` | Files.move |
| `expect fun platformDeleteExpect()` | Files.delete |

---

## 四、验证标准

1. `./gradlew :shared:jvmJar` 编译通过
2. `./gradlew :shared:jvmTest` 全部 171 个单测通过
3. Compose Desktop 应用可启动（`./gradlew :shared:run`）
4. 无 Kuikly 残留依赖（`grep -r "kuikly" shared/src/` 零结果）

---

## 五、风险与对策

| 风险 | 对策 |
|---|---|
| JNA 调 AppKit 需在主线程 | 用 AWT EventQueue 或 JNA Callback 线程管理 |
| SQLDelight JDBC driver 行为差异 | 用 in-memory :memory: 测试验证 SQL 兼容 |
| Compose Desktop 托盘不如原生 | JNA 封装 NSStatusItem 保留原生菜单 |
| java.nio.file.WatchService 灵敏度不如 FSEvents | JNA 直接调 FSEventStreamCreate |

---

## 六、执行顺序

1. 构建系统改造（build.gradle.kts + libs.versions.toml）
2. 删除 Kuikly（facade + 页面 + cinterop def + macosApp/）
3. 创建 jvmMain 平台层（expect actual 适配）
4. 重写 UI（Compose 页面 + 组件）
5. 编译 + 测试验证

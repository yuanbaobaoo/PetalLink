# PetalLink

PetalLink 使用 [Kotlin Toolchain](https://kotlin-toolchain.org/) 作为唯一的开发者构建入口。无需预装 Gradle 或 JDK；首次运行时，仓库内的 wrapper 会自动下载锁定版本的 Toolchain，并由 Toolchain 配置、下载和复用 JDK 25。

```bash
./kotlin build               # 编译
./kotlin test                # 全量测试
./kotlin run                 # 运行桌面应用
./kotlin do packageDmg       # 生成本地 dev DMG
./kotlin do releaseDmg       # 签名、公证并生成正式 release DMG
```

Windows 环境使用对应的 `kotlin.bat`。首次执行需要联网下载 Toolchain、JDK 和项目依赖，后续会复用用户缓存。

项目采用官方教程推荐的 `jvm-app + shared(KMP)` 结构。当前 Kotlin Toolchain 尚未原生提供 Compose Desktop 的 DMG、Developer ID 签名和 Apple notarization 任务，因此两个 DMG 命令由项目插件在内部调用保留的 Gradle 发布链；这是实现细节，日常开发、CI 和发布流程均不再直接调用 `gradlew` 或依赖系统 `JAVA_HOME`。

Toolchain 自身产物位于 `.kotlin/build/`，桌面分发产物仍位于 `build/compose/binaries/`。

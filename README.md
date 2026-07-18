# PetalLink

PetalLink 使用 [Kotlin Toolchain](https://kotlin-toolchain.org/) 作为唯一的开发者构建入口。无需预装额外构建工具或 JDK；首次运行时，仓库内的 wrapper 会自动下载锁定版本的 Toolchain，并由 Toolchain 配置、下载和复用 JDK 25。

```bash
./kotlin build               # 编译
./kotlin test                # 全量测试
./kotlin run                 # 运行桌面应用
./kotlin do packageDmg       # 生成本地 dev DMG
./kotlin do releaseDmg       # 签名、公证并生成正式 release DMG
```

Windows 环境使用对应的 `kotlin.bat`。首次执行需要联网下载 Toolchain、JDK 和项目依赖，后续会复用用户缓存。

IntelliJ IDEA 请安装 Kotlin Toolchain 插件后直接打开仓库根目录，并由 `project.yaml` / `module.yaml` 配置项目模型；不要将根目录或 `build-plugin/.desktop-packaging` 链接为 Gradle 项目。`.desktop-packaging` 不是业务模块，可在 IDEA 的 Project 视图中标记为 `Excluded`。

项目采用官方教程推荐的 `jvm-app + shared(KMP)` 结构。当前 Kotlin Toolchain 尚未原生提供 Compose Desktop 的 DMG、Developer ID 签名和 Apple notarization 任务，因此两个 DMG 命令由 `build-plugin` 中的隐藏兼容桥完成；日常开发、CI 和发布流程均只使用 `./kotlin`，也不依赖系统 `JAVA_HOME`。

Toolchain 自身产物位于 `.kotlin/build/`，桌面分发产物仍位于 `build/compose/binaries/`。

# PetalLink Compose Desktop 发布规则

1. 版本只修改根目录 `gradle.properties` 的 `petalLinkVersion`。
2. 构建档案由 `-Prelease` 布尔开关决定。它是 bundle id、运行时数据目录、LaunchAgent label 的唯一真相源。**默认 dev**（不带 / false / 任意非 true 值）：`./gradlew :shared:run`、`jvmTest`、`packageDmg` 均落到 dev 数据目录，不污染正式数据。**正式发布必须 `-Prelease=true`**：`./gradlew :shared:packageDmg -Prelease=true`。不得用 dev 包发布。
3. 本地先执行 `./gradlew :shared:jvmTest`，打包时 `./gradlew :shared:packageDmg -Prelease=true`，再依次运行制品校验脚本（带 profile 参数，如 `scripts/verify-macos-artifacts.sh <app> "" unsigned release`）和 `scripts/smoke-test-macos-app.sh shared/build/compose/binaries/main/app/PetalLink.app`。
4. tag 必须严格为 `v<petalLinkVersion>`；`.github/workflows/release.yml` 会拒绝不一致的 tag。
5. Release 必须配置 Developer ID、Apple notarization 和 Team ID secrets；禁止发布未签名或未 notarize 的 DMG/update zip。
6. 更新 manifest 为 `PetalLink-update.json`，SHA-256 指向同 Release 的 `.app.zip`；客户端还会验证 codesign、Gatekeeper 和固定 Team ID。
7. 发布后执行 `docs/plan/13-发布与兼容验收.md` 的人工矩阵，不得用 CI 成功代替真实账号和生命周期验收。

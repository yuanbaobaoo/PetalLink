# PetalLink Compose Desktop 发布规则

1. 版本只修改根目录 `gradle.properties` 的 `petalLinkVersion`。
2. 本地先执行 `./gradlew :shared:jvmTest :shared:packageDmg`，再依次运行制品校验脚本和 `scripts/smoke-test-macos-app.sh shared/build/compose/binaries/main/app/PetalLink.app`。
3. tag 必须严格为 `v<petalLinkVersion>`；`.github/workflows/release.yml` 会拒绝不一致的 tag。
4. Release 必须配置 Developer ID、Apple notarization 和 Team ID secrets；禁止发布未签名或未 notarize 的 DMG/update zip。
5. 更新 manifest 为 `PetalLink-update.json`，SHA-256 指向同 Release 的 `.app.zip`；客户端还会验证 codesign、Gatekeeper 和固定 Team ID。
6. 发布后执行 `docs/plan/13-发布与兼容验收.md` 的人工矩阵，不得用 CI 成功代替真实账号和生命周期验收。

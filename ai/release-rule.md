# PetalLink Compose Desktop 发布规则

1. 版本只修改根目录 `version.properties` 的 `petalLinkVersion`。
2. 构建档案是 bundle id、运行时数据目录、LaunchAgent label 的唯一真相源。`./kotlin run`、`./kotlin test` 与 `./kotlin do packageDmg` 默认使用 dev 档案，不污染正式数据；正式发布只允许使用 `./kotlin do releaseDmg`，该命令在内部固定启用 release 档案。不得用 dev 包发布。
3. 本地先执行 `./kotlin test`，本地无签名预览执行 `./kotlin do packageDmg`，正式签名与公证执行 `./kotlin do releaseDmg`；CI 以 Toolchain 命令成功作为产物生成完成的标准，不再执行打包产物校验或启动冒烟。
4. tag 必须严格为 `v<petalLinkVersion>`；`.github/workflows/release.yml` 会拒绝不一致的 tag。
5. Release 必须配置 Developer ID、Apple notarization 和 Team ID secrets；禁止发布未签名或未 notarize 的 DMG/update zip。
6. 更新 manifest 为 `PetalLink-update.json`，SHA-256 指向同 Release 的 `.app.zip`；客户端还会验证 codesign、Gatekeeper 和固定 Team ID。
7. 发布后执行 `docs/plan/13-发布与兼容验收.md` 的人工矩阵，不得用 CI 成功代替真实账号和生命周期验收。

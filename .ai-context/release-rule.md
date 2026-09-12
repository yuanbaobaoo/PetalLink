# PetalLink 自动更新 / Release 发布流程

## 发布原则

- macOS arm64 与 Linux x86_64 AppImage 必须来自同一个 tag、同一次 Release workflow。
- 两个平台沿用 `tauri.conf.json` 中的同一更新公钥以及与之匹配的现有私钥；不得为
  Linux 另建或轮换密钥。
- `PetalLink_update.json` 是已安装客户端写死的 endpoint 文件名，不能只改成
  `latest.json`。
- Linux 应用内更新只属于官方、用户可写的 AppImage。AUR、Deb/RPM、源码构建和
  `/opt` 等只读安装继续由包管理器或用户维护。
- 已公开 Release 视为不可变；workflow 只允许补传尚未公开的 draft，不覆盖正式资产。

## 1. 修改版本号

将版本号更新为线上最新版本以上的新 SemVer，并保持两处完全一致：

- `Cargo.toml`: `version = "<version>"`
- `tauri.conf.json`: `"version": "<version>"`

## 2. 本地验证

至少运行 Rust 测试、前端测试、类型检查和普通 Linux AppImage 冒烟构建。普通本地
构建使用 `tauri.linux.conf.json`，不生成签名，也不设置官方更新渠道标记。

## 3. 提交并打 tag

```bash
git add Cargo.toml tauri.conf.json
git commit -m "release v<version>"
git tag v<version>
git push origin main v<version>
```

也可在 GitHub Actions 手动触发 Release workflow，但必须填写一个已经存在的
`v<version>` tag；workflow 不再为无 tag 构造伪版本。

## 4. GitHub Actions 发布

workflow 会：

1. 强制校验 tag、`Cargo.toml`、`tauri.conf.json` 三处版本一致。
2. 在 macOS 构建 `.dmg`、`.app.tar.gz` 与 `.app.tar.gz.sig`。
3. 在 Ubuntu 22.04 构建 `.AppImage` 与 `.AppImage.sig`；该构建设置
   `PETALLINK_UPDATE_CHANNEL=appimage`。
4. 使用内嵌公钥通过 minisign 复验两平台产物。
5. 生成唯一的 `PetalLink_update.json`，同时包含 `darwin-aarch64` 与
   `linux-x86_64`。
6. 创建 draft Release、逐项核对资产，然后再公开为 latest。

第一份带 Linux 官方更新渠道标记的 AppImage 仍需用户手动安装；它之后的版本才可以
在应用内自动替换。

## GitHub Secrets

以下 Secret 都是发布构建必需项：

- `HWCLOUD_CLIENT_ID`
- `HWCLOUD_CLIENT_SECRET`
- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`（仅当现有私钥有密码时需要，可为空）

`GITHUB_TOKEN` 由 GitHub Actions 自动提供。私钥必须与 `tauri.conf.json` 中现有
公钥匹配，且应另做离线备份；私钥丢失后，已经安装的客户端无法信任后续新密钥签名
的更新。

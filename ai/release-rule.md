# PetalLink 自动更新 / Release 发布流程

## 如何发布新版本

## 1. 修改版本号
两个文件需要同步更新（将 `<version>` 替换为实际版本号，如 `1.0.2`）：
- `Cargo.toml`: `version = "<version>"`
- `tauri.conf.json`: `"version": "<version>"`

### 2. 提交并打 tag
```bash
# 将 <version> 替换为实际版本号，如 1.0.2
git add Cargo.toml tauri.conf.json
git commit -m "release v<version>"
git tag v<version>
git push origin main v<version>
```

### 3. GitHub Actions 自动执行（约 10-15 分钟）
- 构建 macOS `aarch64` 单架构（Apple Silicon）
- 产物：`PetalLink_*_aarch64.dmg`、`PetalLink_*_aarch64.app.tar.gz`、`PetalLink_*_aarch64.app.tar.gz.sig`（更新签名）
- 生成 `PetalLink_update.json`（含真实 signature）并上传到 Release Assets
- 创建 GitHub Release

### 4. 手动触发（不用 tag）
在 GitHub Actions 页面 → Release workflow → `Run workflow` 手动触发。

## GitHub 配置要求

### 必须配置
- `TAURI_SIGNING_PRIVATE_KEY` — Tauri 更新签名私钥（构建期对 `.app.tar.gz` 签名，生成 `.sig`）。本地免费生成，**未配置则 CI 会因找不到 `.sig` 而失败**。
- `GITHUB_TOKEN` 由 GitHub Actions 自动注入
- 仓库地址：`https://github.com/yuanbaobaoo/PetalLink`

> 注：`TAURI_SIGNING_PRIVATE_KEY` 生成方式： cargo tauri signer generate -w ~/.tauri/petal-link.key -p ""。

### 可选配置（构建用）
在仓库 `Settings → Secrets and variables → Actions → Secrets` 添加：
- `HWCLOUD_CLIENT_ID` — 华为云盘 OAuth Client ID（构建期注入）
- `HWCLOUD_CLIENT_SECRET` — 华为云盘 OAuth Client Secret（构建期注入）
- 不配置则使用 `.env` 文件或占位符，不影响更新功能

### 可选配置（Apple 代码签名，仅影响首次 DMG 安装）
- Apple Developer 账号 + 证书（$99/年）
- 配置后首次双击 DMG 不再被 Gatekeeper 拦截，无需手动 `xattr -d`
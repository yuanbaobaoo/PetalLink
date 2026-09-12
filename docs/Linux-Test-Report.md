# Linux 跨发行版与桌面环境测试报告

测试日期：2026-08-04

测试分支：`feature/linux-on-demand-drive`

上游基线：PetalLink v1.1.6（`32e1209`）

测试范围：Linux AppImage、OAuth、FUSE3 按需云盘、文件管理器后台预览拦截与交互式打开

## 结论

本轮测试通过。Linux 版本可以在 CachyOS、Ubuntu 26.04 和 Fedora 44 上启动、登录、
恢复登录态并挂载按需云盘。GNOME、KDE Plasma 和 Xfce 文件管理器产生的后台缩略图
读取不会触发占位文件批量下载；用户真正打开文件时，只会下载目标文件。

测试没有发现对 macOS 目录模式的运行时改动。Linux 继续使用独立的 FUSE3 后端，
macOS 仍使用原有传统同步目录。

## 测试环境

| 系统 | 内核 | 桌面与文件管理器 | backing 文件系统 | 用途 |
|---|---|---|---|---|
| CachyOS x86_64 | 7.1.5-1-cachyos | Plasma 6.7.3、Dolphin 26.04.3、Wayland | Btrfs | 本机构建、默认浏览器与基础按需读取 |
| Ubuntu 26.04 x86_64 | 7.0.0-28-generic | GNOME 50.1 / Nautilus 50.0；Plasma 6.6.5 / Dolphin 25.12.3；Xfce 4.20 / Thunar 4.20.7 | ext4 | 三桌面会话与缩略图回归 |
| Fedora Workstation 44 x86_64 | 6.19.10-300.fc44.x86_64 | GNOME 50.0、Nautilus 50.0、Papers 49.3 | Btrfs | AppImage 跨发行版、OAuth 与 GNOME 按需读取 |

Ubuntu 与 Fedora 运行在 VirtualBox 虚拟机中，每台虚拟机配置 4 vCPU、8 GiB 内存和
64 GiB 磁盘。虚拟机验证用于兼容性与行为检查，不替代真实硬件上的性能、休眠唤醒和
长期稳定性测试。

## 自动化检查

最终提交历史整理完成后执行以下检查：

```bash
CARGO_BUILD_JOBS=2 cargo fmt --all -- --check
CARGO_BUILD_JOBS=2 cargo test -j2
CARGO_BUILD_JOBS=2 cargo clippy -j2 --all-targets --all-features -- -D warnings
npm --prefix app test
npm --prefix app run build
```

| 检查 | 结果 |
|---|---|
| Rust 格式检查 | 通过 |
| Rust 单元与集成测试 | 290 项通过；6 项按设计忽略（4 项真实 FUSE、1 项真实华为云上传、1 项文档示例） |
| Clippy（全部 target 与 feature，警告视为错误） | 通过 |
| Vue/Vitest 测试 | 17 个测试文件、84 项测试全部通过 |
| 前端生产构建 | 通过 |

真实华为云账号只用于手工验证；凭据、Token 和用户文件内容均不写入仓库或测试报告。

## 手工测试结果

### OAuth 与桌面集成

| 场景 | 预期 | 结果 |
|---|---|---|
| CachyOS AppImage 发起 OAuth | 使用系统默认 Microsoft Edge | 通过 |
| Fedora AppImage 发起 OAuth | 使用系统默认 Firefox，而不是 AppImage 内部组件 | 通过 |
| OAuth 回调 | 本机 `127.0.0.1:9999` 收到授权码并关闭临时监听 | 通过 |
| Token 持久化 | 退出应用、切换桌面会话后仍能恢复登录态 | 通过 |
| AppImage 外部打开器 | 清除 AppImage 注入的 GLib/GIO 环境后调用宿主 `xdg-open` | 通过 |

Fedora 首次复现了 AppImage 环境污染：系统 `gio` 会错误继承 AppImage 内的 Ubuntu
GLib 路径，导致默认浏览器不能启动。修复后，OAuth 授权页由 Fedora 的 Firefox 正常
打开；CachyOS 仍遵循系统设置，使用 Microsoft Edge。

### 索引与占位文件

| 场景 | 观察值 | 结果 |
|---|---|---|
| Fedora 首次建立云端索引 | 658 个条目；backing 中非空正文文件为 0 | 通过 |
| Ubuntu 应用重启与桌面切换 | 658 个条目；已有正文文件数量保持不变 | 通过 |
| FUSE 挂载类型 | `fuse.petallink`，仅当前用户访问 | 通过 |
| backing 扩展属性 | Ubuntu ext4 与 Fedora/CachyOS Btrfs 均可保存 `user.com.hwcloud.*` | 通过 |

“同步索引”只同步目录、元数据与占位状态，没有下载全部正文。

### GNOME / Nautilus

在 Fedora GNOME 中打开包含多份未下载 PDF 的目录，Nautilus 同时请求 5 个 PDF
缩略图。PetalLink 识别并拒绝 `papers-thumbnailer` 及其 `bwrap` 沙箱读取：

- 测试前：backing 中非空正文文件为 0；
- 预览后：backing 中非空正文文件仍为 0；
- 真正打开 `PRML Pattern Recognition And Machine Learning.pdf` 后：只新增该文件；
- 目标文件落地大小为 9,678,672 字节，Papers 正常显示 746 页；
- 其他占位文件没有开始下载。

结果：通过。

### KDE Plasma / Dolphin

在 Ubuntu 的完整 Plasma Wayland 会话中启用 Dolphin“显示预览”，打开包含 9 个未下载
PDF 的目录：

- 测试前：backing 中有 2 个此前主动打开的正文文件；
- 浏览并等待预览后：非空正文文件仍为 2；
- 没有产生新增批量下载任务；
- Dolphin 窗口保持正常运行。

结果：通过。

### Xfce / Thunar

在 Ubuntu 的完整 Xfce 会话中打开相同目录。Thunar 的 `tumblerd` 和
`papers-thumbnailer` 对 9 个 PDF 逐一发起缩略图读取：

- 所有后台读取都在 FUSE `open` 阶段被识别并拒绝；
- 测试前后 backing 中非空正文文件均为 2；
- Thunar 正常显示通用 PDF 图标，没有产生正文下载任务。

结果：通过。

## 回归边界

本轮重点验证 Linux 新路径，没有改变以下平台边界：

- macOS 不编译 Linux FUSE 实现，继续使用现有传统同步目录；
- Linux 专用代码使用 `cfg(target_os = "linux")` 隔离；
- AppImage 环境清理只在检测到 `APPIMAGE` 或 `APPDIR` 时启用；
- 普通 Linux 开发运行、macOS `open` 和非 AppImage 发行包不会进入该清理分支；
- 后台读取识别只匹配明确的索引器、缩略图器或携带缩略图命令行的沙箱；身份无法确认时
  fail-open，避免误伤用户主动打开文件。

## 尚未覆盖

以下项目不属于本轮“按需读取与跨桌面”测试结论，合并或发布前仍建议继续验证：

- Ubuntu 22.04/24.04 等更老 glibc 基线上的最终发布 AppImage；
- AArch64 Linux 与非 VirtualBox 的真实硬件；
- NFS、SMB、嵌套 FUSE、禁用 `user_xattr` 的文件系统；
- 长时间休眠/唤醒、断网重连和大文件并发传输压力；
- AUR 打包安装、升级与卸载；
- 官方 AppImage 签名更新的端到端发布流程。

## PR 建议

当前结果足以提交 Linux 适配 PR。PR 描述中应明确这是 x86_64 Linux 初始支持，并把
AppImage 的最低 glibc 基线、AUR 打包和签名自动更新列为后续工作，不应直接宣称支持
所有发行版、桌面环境与文件系统。

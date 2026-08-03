# Linux 支持说明

PetalLink 在 Linux 上只提供 **FUSE3 按需云盘**：用户只选择一个在文件管理器中
使用的可见云盘目录，应用自行创建和维护私有 backing（本地缓存与同步数据）。
Linux 不提供传统同步目录，也没有模式切换开关。

macOS 不受这项决定影响，仍使用项目现有的传统同步目录：用户选择的目录就是实际的
本地镜像，不经过 Linux FUSE 层。两个平台继续复用 OAuth、Drive API、SQLite、
同步引擎与 Vue 界面，平台差异集中在 Tauri 生命周期、扩展属性、机器标识、系统
打开器、登录自启动以及 Linux FUSE 文件系统。

## 当前范围

| 项目 | Linux 状态 |
|---|---|
| Rust / Vue / Tauri 编译 | 已适配 x86_64 Linux |
| OAuth 浏览器授权 | 通过 `xdg-open` 打开默认浏览器 |
| Token 持久化 | 使用 `/etc/machine-id`，回退 `/var/lib/dbus/machine-id` |
| 占位文件元数据 | 逻辑键 `com.hwcloud.*` 映射到 Linux `user.com.hwcloud.*` |
| 目录模式 | 仅 FUSE3 按需云盘；用户只配置一个可见云盘目录 |
| 私有 backing | 由应用在私有数据区创建、选择和维护，不进入普通设置界面 |
| 文件管理器透明访问 | FUSE3 显示完整目录；交互式应用首次读取未下载文件时整文件下载，后台缩略图与索引读取不会触发落地 |
| 文件管理器写入 | 在 FUSE 目录中的创建、编辑、改名、移动和删除落到 backing，再由现有同步引擎上传 |
| 文件变更监听 | `notify::RecommendedWatcher` 使用 inotify；丢事件或队列饱和时强制全量重扫 |
| 文件占用检查 | 安装 `lsof` 时使用进程级占用检查；缺失时使用 mtime 与文件大小检查 |
| 打开云盘目录 | 通过 `xdg-open` 交给桌面文件管理器 |
| 登录自启动 | 写入 `$XDG_CONFIG_HOME/autostart/*.desktop` |
| 打包 | `tauri.linux.conf.json` 生成 AppImage |
| 自动更新 | 官方、用户可写的 x86_64 AppImage 支持签名更新；AUR/系统包与源码构建交给包管理器或用户更新 |

当前已在 CachyOS x86_64/KDE Wayland、Ubuntu 26.04 的 GNOME/Plasma/Xfce 以及
Fedora Workstation 44/GNOME 上完成 AppImage、OAuth、FUSE 挂载和文件管理器后台
预览验证，详见 [Linux 测试报告](./Linux-Test-Report.md)。这些结果不代表最低 glibc
基线；正式发布前仍应在 Ubuntu 22.04/24.04 等计划支持的较老系统上验证最终发布
产物，也不应宣称支持所有 Linux 发行版、桌面环境或文件系统。

## 系统依赖

Rust、Node.js 的版本要求与 README 一致。Debian/Ubuntu 可安装 Tauri 官方依赖：

```bash
sudo apt update
sudo apt install \
  libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev \
  libssl-dev libayatana-appindicator3-dev librsvg2-dev \
  xdg-utils lsof fuse3
```

- `xdg-utils` 提供 OAuth 与文件管理器使用的 `xdg-open`。
- `lsof` 用于上传前的文件占用检查；缺失时会退化为 mtime 与文件大小检查。
- `fuse3` 提供 `/dev/fuse` 与普通用户挂载所需的 `fusermount3`。无需安装
  `libfuse3-dev`：Rust `fuser` 使用内核 ABI，运行时仍需要系统的挂载助手。
- GNOME 可能需要启用 AppIndicator/KStatusNotifierItem 扩展才能显示托盘图标。

运行前应确认：

```bash
test -c /dev/fuse
command -v fusermount3
```

`/dev/fuse` 必须对当前桌面用户可用，`fusermount3` 必须位于 `PATH`。容器、受限
沙箱、禁用用户挂载的企业环境或未加载 FUSE 内核模块时，即使应用本身能够启动，也
无法创建云盘目录。

## 开发与打包

```bash
cp .env.example .env
# 编辑 .env，填入真实的 HWCLOUD_CLIENT_ID 与 HWCLOUD_CLIENT_SECRET

cd app
npm ci
cd ..

./app/node_modules/.bin/tauri dev --config tauri.dev.conf.json
./app/node_modules/.bin/tauri build --bundles appimage
```

Linux 构建会自动合并 `tauri.linux.conf.json`。AppImage 产物位于：

```text
target/release/bundle/appimage/
```

AppImage 会继承构建系统的 glibc 等基础兼容边界。用于发布的 AppImage 应在计划支持
的最老基础系统上构建，而不是直接把滚动发行版上的本机构建作为通用发行包。

滚动发行版的系统库可能包含旧版 linuxdeploy 内置 `strip` 无法识别的
`.relr.dyn`。这种情况下，本地冒烟包可临时关闭 linuxdeploy 的 strip：

```bash
NO_STRIP=1 ./app/node_modules/.bin/tauri build --bundles appimage
```

这会增大产物，只解决本地打包器兼容问题，不会降低 glibc 基线，也不应代替 Ubuntu
发布流水线。环境变量只要存在就生效；恢复默认行为需执行 `unset NO_STRIP`。

## 按需云盘目录模型

界面只要求用户选择一个目录，内部仍保持显示层和数据层分离：

```text
用户选择的云盘目录（FUSE 挂载点） ── 文件管理器 / 普通应用读写
                     │
                     ▼
PetalLink 管理的私有 backing ── 占位、已下载内容、inotify 与同步引擎
```

- 云盘目录必须 **完全为空**，包括点号开头的隐藏文件、回收站目录和其他挂载残留；
  “看起来没有文件”不等于通过检查。
- 云盘目录不能是 `/`、Home、应用私有数据目录或已经使用中的挂载点，其父目录必须
  允许当前用户创建 FUSE 挂载。
- backing 是唯一实际保存占位元数据与已下载内容的目录。它由 PetalLink 自动创建、
  选择和维护，不显示在普通设置中。用户不得把它设为云盘目录，也不应手动
  浏览、移动、改名、删除或向其中写入文件。
- 显示目录与 backing 必须分离，不能相同或互相嵌套；应用负责保证这项内部约束。
- 挂载仅对当前用户开放，不启用 `allow_other`，也不暴露 backing 中的
  `.hwcloud_*` 临时文件。
- 首次读取采用“整文件落地后再交给应用”的策略；离线时保留占位并返回明确错误，
  网络恢复后可再次打开。
- Linux 文件管理器会为了缩略图、内容搜索或媒体属性后台读取目录中的许多文件。
  PetalLink 在占位文件首次读取前识别 Dolphin、Baloo、Tracker、Tumbler 等明确的
  后台预览/索引调用者，并向它们报告预览不可用；默认应用打开、命令行读取和普通
  KIO 文件复制仍会触发按需下载。进程身份无法可靠读取时采用 fail-open，保证正常
  应用不会因 `/proc` 权限或进程退出竞态而被误拦截。
- 正常退出、注销、清缓存或修改目录时会先停止新文件活动并卸载 FUSE。启动时只会
  自动清理由当前用户持有、身份为 PetalLink 且已断开的崩溃遗留挂载。

Linux 不会在 FUSE 挂载失败时悄悄改用传统同步。挂载条件不满足时应明确报错并保持
未挂载状态，避免同一路径在两种语义之间切换。

## 旧开发版迁移

旧 Linux 开发版可能把用户选择的目录直接当作传统同步 backing，或者同时保存
`mountDir` 与 `virtualMountDir` 两个路径。升级时必须遵守以下边界：

1. 升级前正常退出旧版本，确认 PetalLink 进程已经结束、旧 FUSE 挂载已经卸载，并
   确认所有需要保留的本地修改都已上传。存在本地独有、失败或冲突任务时不要清目录。
2. 先备份旧配置、SQLite 数据库和云树缓存。旧传统同步目录中的内容也应保留，直到
   云端内容与迁移结果都核对完成；不要直接执行递归删除。
3. 对未启用按需云盘的旧配置，新版本会禁止启动传统同步并回到“选择云盘目录”状态。
   旧目录原样保留，不承诺自动搬入新的私有 backing，也不会自动删除。
4. 已启用按需云盘的旧双目录配置可以继续沿用原 backing；升级后的界面只展示和允许
   修改可见云盘目录，不再把 backing 暴露为用户配置项。
5. 旧传统同步目录不能直接作为新的 FUSE 云盘目录。若希望复用同一路径，必须先在
   应用完全退出且挂载已卸载的情况下，将旧内容完整备份或改名移走，并确认该目录
   包括隐藏文件在内完全为空；也可以直接选择一个新的空目录。

切勿把 FUSE 直接挂载到仍保存旧缓存或本地文件的目录上。挂载会遮住底层内容，容易
造成“文件消失”的误判；贸然清理还可能永久丢失尚未上传的本地数据。

## 文件系统要求

PetalLink 把占位状态、云端 fileId 和原始大小保存在私有 backing 的扩展属性中。
Linux 普通用户属性必须位于 `user.*` 命名空间，因此平台层会做以下映射：

```text
com.hwcloud.state  -> user.com.hwcloud.state
com.hwcloud.fileId -> user.com.hwcloud.fileId
com.hwcloud.size   -> user.com.hwcloud.size
```

创建或恢复私有 backing 时，应用会对临时文件执行一次扩展属性的写入、读取和删除
探测。探测失败会拒绝挂载云盘，避免同步到一半才发现占位状态无法持久化。用户选择的
可见目录只是 FUSE 挂载点；`user.*` 要求针对应用私有 backing 所在的文件系统。

本地 ext4、Btrfs、XFS 等文件系统通常可用；FUSE 层不依赖 backing 的具体本地
文件系统类型。但不能据此承诺所有 NFS、SMB、嵌套 FUSE、加密盘、容器挂载或禁用
`user_xattr` 的文件系统。macOS Finder 灰色标签只是一种视觉提示，
Linux 上不会写文件管理器专用标签，业务判定仍只依赖扩展属性。

Linux 外置盘常见的 `.Trash` 与 `.Trash-$uid` 回收站目录由后端硬跳过，避免删除后
的文件被 watcher 当成新内容重新上传。

## 桌面集成

- 登录自启动文件默认位于 `~/.config/autostart/io.github.yuanbaobaoo.PetalLink.desktop`。
- AppImage 运行时优先记录 `$APPIMAGE` 原始路径，不能记录临时挂载目录中的内部二进制。
- 托盘创建失败或用户关闭托盘图标后，关闭主窗口会完全退出，避免留下无法恢复的后台进程。
- 用户从界面触发重启时会以前台模式恢复；Tauri 负责识别 AppImage 原始路径。

## 合入上游前的验证

自动检查：

```bash
HWCLOUD_CLIENT_ID=test HWCLOUD_CLIENT_SECRET=test cargo fmt --all -- --check
HWCLOUD_CLIENT_ID=test HWCLOUD_CLIENT_SECRET=test cargo check --all-targets --locked
HWCLOUD_CLIENT_ID=test HWCLOUD_CLIENT_SECRET=test cargo test --all-targets --locked
HWCLOUD_CLIENT_ID=test HWCLOUD_CLIENT_SECRET=test \
  cargo test virtual_fs::linux::tests::real_fuse_read_smoke --lib -- --ignored --nocapture
HWCLOUD_CLIENT_ID=test HWCLOUD_CLIENT_SECRET=test cargo clippy --all-targets --locked -- -D warnings
cd app && npm ci && npm test
cd .. && HWCLOUD_CLIENT_ID=test HWCLOUD_CLIENT_SECRET=test npm --prefix app run build
```

真机检查至少覆盖：

1. KDE Wayland 与 Ubuntu/GNOME 各一套。
2. OAuth 回调、Token 跨重启恢复和注销。
3. FUSE 目录浏览、逻辑大小、首次打开自动下载、并发打开、离线重试与释放空间；开启
   Dolphin/Nautilus 缩略图及 Baloo/Tracker 索引时不得产生批量下载任务。
4. FUSE 中 create/modify/rename/move/delete 后的 inotify 事件及云端收敛。
5. 托盘显示/恢复、隐藏托盘后的退出和托盘不可用时的降级行为。
6. AppImage 的启动、重启、登录自启动与路径移动。
7. 对不支持 `user.*` 扩展属性的目录给出明确错误。

## 建议的 PR 拆分

为降低 macOS 回归风险，不建议把运行时、同步安全、打包和发布一次性塞进一个 PR：

1. `refactor(platform): 抽取桌面平台能力边界`
2. `feat(linux): 支持 xattr 占位与原子路径恢复`
3. `feat(linux): 支持登录自启动与桌面生命周期`
4. `fix(sync): 在 watcher 丢事件时强制收敛`
5. `build(linux): 增加 AppImage 与文档`
6. `ci(linux): 增加 Ubuntu 编译与测试`
7. `feat(linux): 将 FUSE 按需云盘设为唯一目录模式`
8. `release(linux): 增加签名更新产物`

自动更新发布链使用现有 Tauri 公钥和仓库中已经配置的私钥 Secret，不生成或轮换
密钥。CI 在 Ubuntu 22.04 上生成 `.AppImage` 与 `.AppImage.sig`，并由单一发布任务
把 `linux-x86_64` 和 `darwin-aarch64` 聚合到现有 `PetalLink_update.json`。普通
本地构建及 AUR 源码构建不设置 `PETALLINK_UPDATE_CHANNEL=appimage`，因此不会绕过
包管理器；即使是官方 AppImage，放在不可写目录中也不会启用应用内更新。

第一份带 Linux 更新渠道标记的 AppImage 需要手动安装。此后，应用才可以验证签名、
下载并原位替换后续 AppImage。发布前必须保证 tag、`Cargo.toml` 和
`tauri.conf.json` 三处版本一致，并使用比线上最新版本更高的新版本号。

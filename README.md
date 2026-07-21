# PetalLink（Flutter 版）

华为云盘 macOS 客户端——Drive REST API 直连、本地目录双向同步、占位符按需下载、inode 文件身份识别。

## 构建

dev/release 由 Flutter 构建模式直接区分（xcconfig 已按模式配置 bundle id 与 app 名）：

```bash
# 开发版（debug）
#   产物: build/macos/Build/Products/Debug/PetalLink Dev.app
#   Bundle ID: io.github.yuanbaobaoo.PetalLink-dev（数据目录同为 -dev，与正式版隔离）
flutter run -d macos              # 直接运行
flutter build macos --debug       # 只编译

# 正式版（release）
#   产物: build/macos/Build/Products/Release/PetalLink.app
#   Bundle ID: io.github.yuanbaobaoo.PetalLink（正式数据目录）
flutter build macos --release
```

### 构建期配置注入（--dart-define，与 .env 并存）

| 键 | 用途 |
|---|---|
| `HWCLOUD_CLIENT_ID` | OAuth client_id（必填，否则登录页提示未配置） |
| `HWCLOUD_CLIENT_SECRET` | OAuth client_secret（必填） |
| `PETALLINK_UPDATE_TEAM_ID` | 更新器 Apple Team ID 签名校验（可选；未配置时更新安装被拒绝） |

```bash
flutter build macos --release \
  --dart-define=HWCLOUD_CLIENT_ID=xxx \
  --dart-define=HWCLOUD_CLIENT_SECRET=yyy \
  --dart-define=PETALLINK_UPDATE_TEAM_ID=XXXXXXXXXX
```

凭据解析优先级：`--dart-define` > 打包 asset `.env` > 工作目录 `.env` > 进程环境变量。
日常开发只需在项目根目录维护 `.env`（已 gitignore，`.env.example` 为模板）并打包为 asset，无需任何额外参数。

## 验证

```bash
flutter pub get
flutter analyze     # 必须 0 issue
flutter test        # 全部通过
flutter run -d macos
```

## 文档

- `docs/design/`：有效设计文档（00 导航为入口；业务以 Tauri 原版为基线，文件身份为 inode 方案）
- `docs/reference/`：Tauri 原版只读参考
- `docs/review/`：代码审查与修复/实施记录
- `ai/coding-rules.md`：编码规范

## 技术栈

Flutter + Dart / GetX / Dio / sqflite / MethodChannel（macOS 原生桥接：xattr、FSEvents、lstat 批量 stat）

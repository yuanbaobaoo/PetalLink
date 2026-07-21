# PetalLink（Flutter 版）

华为云盘 macOS 客户端——Drive REST API 直连、本地目录双向同步、占位符按需下载、inode 文件身份识别。

## 构建

统一使用 `tool/build.sh`（编译期自动从 `.env` 注入凭据；追加的 `--dart-define` 可覆盖）：

```bash
# 开发版（debug；bundle id / 数据目录为 PetalLink-dev，与正式版隔离）
tool/build.sh dev                    # 等价 flutter build macos --debug
tool/build.sh run                    # 等价 flutter run -d macos

# 正式版（release）
tool/build.sh release                # 等价 flutter build macos --release

# 追加注入（覆盖 .env 同键）
tool/build.sh release --dart-define=PETALLINK_UPDATE_TEAM_ID=XXXXXXXXXX
```

### `.env` 支持的键（项目根目录，已 gitignore；`.env.example` 为模板）

| 键 | 用途 |
|---|---|
| `HWCLOUD_CLIENT_ID` | OAuth client_id（必填，否则登录页提示未配置） |
| `HWCLOUD_CLIENT_SECRET` | OAuth client_secret（必填） |
| `PETALLINK_UPDATE_TEAM_ID` | 更新器 Apple Team ID 签名校验（可选；未配置时更新安装被拒绝） |

凭据解析优先级：`--dart-define` > 打包 asset `.env` > 工作目录 `.env` > 进程环境变量。

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

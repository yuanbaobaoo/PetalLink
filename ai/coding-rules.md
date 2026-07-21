# PetalLink Flutter 编码规范

> 主参考：xe-cloud-app-x（GetX + Dio + 分层架构）
> 补充参考：petal-link-cmp ai/coding-rules.md（设计Token、错误模型、注释规范）

---

## 一、语言与注释

1. **注释语言**：简短中文
2. **标识符**：英文（类名 PascalCase，变量/方法 camelCase，文件 snake_case）
3. **技术术语**：保留英文原词（BFS, token, cursor, checkpoint, PKCE, OAuth, inode）
4. **公开 API**：必须写 `///` 文档注释（三斜线）
5. **私有成员**：`_` 前缀（`_client`, `_onSubmit()`）
6. **常量**：`static const` 用 camelCase，不强制 UPPER_SNAKE_CASE

## 二、文件与命名

| 类别 | 约定 | 示例 |
|------|------|------|
| 文件名 | `snake_case.dart` | `auth_controller.dart` |
| 页面文件 | `*_page.dart` | `login_page.dart` |
| 控制器 | `*_controller.dart` | `auth_controller.dart` |
| 服务 | `*_service.dart` | `auth_service.dart` |
| 实体/模型 | 直接命名 | `auth.dart`, `transfer_task.dart` |
| 组件 | `mate_*.dart` | `mate_button.dart` |
| 工具 | `*_util.dart` | `date_util.dart` |
| 类型定义 | 描述性命名 | `enums.dart`, `app_result.dart` |

## 三、类与标识符命名

1. **Mate 前缀**：所有自定义 UI 组件使用 `Mate` 前缀（`MateButton`, `MateDialog`, `MateToast`）
2. **Controller 后缀**：`AuthController`, `FileBrowserController`
3. **Service 后缀**：`AuthService`, `FilesService`
4. **State 后缀**：状态类用 `State` 后缀（`LoginState`, `SyncState`）
5. **枚举值**：PascalCase，每个值加短中文注释

## 四、设计 Token（严禁硬编码）

1. **所有颜色**必须引用 `MateTokens` 或 `MateTheme`，严禁直接写 `Color(0xFF...)` 在组件中
2. **所有尺寸**（宽高、间距、圆角、字号）必须引用 `MateMetrics` 或 `MateTypography`
3. **组件通过 `MateTheme.colorsOf(context)` / `.typographyOf(context)` / `.metricsOf(context)` 访问**
4. 主题切换自动跟随 macOS 系统设置（亮色/暗色），组件不判断 `isDark`

## 五、错误处理

1. 错误模型：`AppError` sealed class（`AuthError`, `TokenError`, `DriveApiError`, `ConfigError`, `QuotaExceededError`, `GenericError`，严格对齐 Rust `src/error.rs`）
2. 返回类型：`AppResult<T>`（`Ok<T>` / `Err<T>`）
3. Service 层抛出 `AppError`，Controller 层 catch 并展示 `MateToast`
4. HTTP 客户端统一处理 401 自动刷新（singleflight）

## 六、状态管理（GetX）

1. 响应式状态：`final Rx<MyState> state = MyState.empty().obs`
2. 状态更新：`state.value = state.value.copyWith(...)`（不可变）
3. Widget 观察：`Obx(() => ...)`
4. 副作用：`ever(controller.state, (state) { ... })`
5. Controller 生命周期：`Get.put()` on create, `Get.delete()` on dispose
6. 页面进入时的数据加载：`Future.microtask(() => controller.loadData())`

## 七、网络层（Dio）

1. 使用 `MateHttpClient` 封装，不直接使用 Dio
2. API 按域分包：`service/auth/`, `service/drive/`, `service/sync/`
3. Token 注入：通过 `tokenProvider` 回调自动注入 Authorization header
4. 401 处理：自动 refresh token，失败则退出登录

## 八、持久化

1. SQLite（sqflite）存储：传输队列、同步状态、云文件树缓存、配置
2. OAuth token：`<Application Support>/token.bin`，ChaCha20-Poly1305 AEAD 加密
   （key = SHA-256(IOPlatformUUID)，机器码绑定，文件权限 0600，对齐 Rust token_store.rs）
3. 数据库迁移：通过 `DatabaseService` 统一管理，版本递增

## 九、导入顺序

```dart
// 1. dart: 库
import 'dart:async';
import 'dart:convert';

// 2. package: 库
import 'package:flutter/material.dart';
import 'package:get/get.dart';

// 3. 项目相对导入
import '../core/error/app_result.dart';
import '../entity/auth.dart';
```

## 十、文件系统安全

> 教训来源：2026-07-17 事故 — build 任务符号链接 + `deleteRecursively` 清空了 `/Applications`

1. **递归删除前**必须验证目标路径在允许的根目录白名单内
2. **符号链接**：递归操作前先扫描目标路径是否包含符号链接
3. 代理只能调用已有构建命令，不得执行自编代码

## 十一、Git 规范

1. 不自动提交，用户明确要求时才提交
2. Commit message 可使用中文
3. `.env` 已 gitignore，`.env.example` 作为模板提交
4. `docs/superpowers/` 不纳入版本控制

//! 全局常量 —— 华为云盘 macOS 客户端
//!
//! # 安全提醒
//! `CLIENT_SECRET` 和 `CLIENT_ID` 切勿提交到仓库。解析优先级（高 → 低）：
//! 1. 构建期环境变量（`HWCLOUD_CLIENT_SECRET` / `HWCLOUD_CLIENT_ID`，由 build.rs 从 .env 注入，或手动设置）
//! 2. `.env` 文件（开发期通过 dotenvy 加载）
//! 3. 硬编码默认值（CLIENT_SECRET 默认占位符会导致登录被拒）
//!
//! 构建期若缺失任一凭据，build.rs 会 panic 阻断编译（cargo tauri dev / build 均适用）。

use once_cell::sync::OnceCell;

/// AGC Web 应用 CLIENT_ID —— 无硬编码默认值，必须由用户通过 .env 提供。
/// 实际运行时优先读取构建期注入的值，其次读取运行时环境变量（dotenvy）。

/// 构建期通过 `HWCLOUD_CLIENT_ID` 环境变量注入的 client_id（由 build.rs 从 .env 注入）。
/// 未配置时为空字符串，运行时再回退到 .env / 空值。
pub const BUILD_CLIENT_ID: &str = match option_env!("HWCLOUD_CLIENT_ID") {
    Some(v) => v,
    None => "",
};

/// 构建期通过 `HWCLOUD_CLIENT_SECRET` 环境变量注入的 secret（由 build.rs 从 .env 注入）。
/// 未配置时为空字符串，运行时再回退到 .env / 占位符。
pub const BUILD_SECRET: &str = match option_env!("HWCLOUD_CLIENT_SECRET") {
    Some(v) => v,
    None => "",
};

/// 占位符 secret（仅作类型占位；登录会被拒绝）
pub const PLACEHOLDER_SECRET: &str = "REPLACE_WITH_REAL_SECRET";

/// 应用展示名（菜单栏标题、关于页等）
pub const APP_NAME: &str = "PetalLink";

/// 应用完整标题（窗口标题栏、关于页、任务切换器）
pub const APP_FULL_TITLE: &str = "PetalLink - 华为云盘客户端开源版";

/// 应用版本号
pub const APP_VERSION: &str = "1.0.1";

/// **Bundle Identifier（io.github.yuanbaobaoo.PetalLink）**
/// 原 Flutter 工程 legacy/ 保留 io.gitcode.cloudmate 不变。
pub const BUNDLE_IDENTIFIER: &str = "io.github.yuanbaobaoo.PetalLink";

/// 可执行文件名（须与 Cargo.toml [[bin]] name 一致；决定进程名 / .app 内 MacOS/<exec>）
pub const EXECUTABLE_NAME: &str = "PetalLink";

/// Keychain service 前缀（与原 Flutter 工程一致，避免换 bundle id 丢凭据）
pub const KEYCHAIN_SERVICE_PREFIX: &str = "hwcloud.";

// ===== OAuth scope =====
/// 授权域。当前用 `drive`（全盘访问），原因见需求文档 §6.1：
/// `drive.file` 只能访问本应用创建/打开过的文件，网页/其他客户端上传的看不到。
/// 必须在 AGC 后台开通 `drive` scope（否则登录报 1101 invalid scope）。
pub const SCOPES: &[&str] = &[
    "openid",
    "profile",
    "https://www.huawei.com/auth/drive",
];

// ===== OAuth 端点 =====
/// Token Host
pub const TOKEN_HOST: &str = "oauth-login.cloud.huawei.com";

/// 授权页地址（华为 OAuth2.0 授权端点，非 account.php）
pub const AUTHORIZE_URL: &str = "https://oauth-login.cloud.huawei.com/oauth2/v3/authorize";

/// Token 端点（授权码换 token / 刷新 token）
pub const TOKEN_URL: &str = "https://oauth-login.cloud.huawei.com/oauth2/v3/token";

/// UserInfo 端点（OIDC 标准）
pub const USER_INFO_URL: &str = "https://oauth-login.cloud.huawei.com/oauth2/v3/userinfo";

// ===== Drive API =====
/// Drive REST API base URL
pub const DRIVE_API_BASE: &str = "https://driveapis.cloud.huawei.com.cn/drive/v1";

/// 上传 API base URL（与 drive 端点是兄弟路径，非父子）
pub const UPLOAD_API_BASE: &str = "https://driveapis.cloud.huawei.com.cn/upload/drive/v1";

// ===== 回调监听 =====
/// 仅绑定 127.0.0.1（满足安全要求，不监听 0.0.0.0）
pub const LOOPBACK_HOST: &str = "127.0.0.1";

/// 默认 OAuth 回调端口
pub const DEFAULT_CALLBACK_PORT: u16 = 9999;

/// OAuth 回调路径
pub const CALLBACK_PATH: &str = "/oauth/callback";

/// OAuth 回调等待超时（用户长时间不操作则关闭 server）
pub const OAUTH_TIMEOUT_SECS: u64 = 5 * 60;

// ===== Token 过期缓冲 =====
/// access_token 过期前缓冲时间（秒）：到期前此时间内主动刷新
pub const TOKEN_EXPIRY_BUFFER_SECS: i64 = 60;

// ===== 内部文件前缀（v1.8 全局硬编码过滤） =====
/// 所有以 `.hwcloud_` 开头的内部文件（cloudtree 缓存 / syncstate 快照）一律不参与同步。
/// 硬编码而非依赖用户可配置的 skipPatterns——内部文件绝不能同步，无论用户如何配置。
pub const INTERNAL_FILE_PREFIX: &str = ".hwcloud_";

/// 原子写临时文件后缀（下载流式写 .tmp 再 rename）
pub const TMP_SUFFIX: &str = ".tmp";

/// .env 文件运行时加载结果（由 main 启动期写入）。
static ENV_SECRET: OnceCell<String> = OnceCell::new();

/// .env 文件加载的 client_id（启动期写入）。
static ENV_CLIENT_ID: OnceCell<String> = OnceCell::new();

/// 设置 .env 解析得到的 client_secret（启动期调用）。
pub fn set_env_secret(value: String) {
    let _ = ENV_SECRET.set(value);
}

/// 设置 .env 解析得到的 client_id（启动期调用）。
pub fn set_env_client_id(value: String) {
    let _ = ENV_CLIENT_ID.set(value);
}

/// 运行时解析得到的最终 client_id（合并优先级：构建期 > .env）。
/// 无默认值 —— 必须由用户显式提供。
pub fn resolved_client_id() -> &'static str {
    if !BUILD_CLIENT_ID.is_empty() {
        return BUILD_CLIENT_ID;
    }
    if let Some(from_env) = ENV_CLIENT_ID.get() {
        if !from_env.is_empty() {
            return from_env;
        }
    }
    ""
}

/// 运行时解析得到的最终 client_secret（合并优先级：构建期 > .env > 占位符）。
/// 对齐 dart `AppConstants.resolvedClientSecret`。
pub fn resolved_client_secret() -> String {
    if !BUILD_SECRET.is_empty() && BUILD_SECRET != PLACEHOLDER_SECRET {
        return BUILD_SECRET.to_string();
    }
    if let Some(from_env) = ENV_SECRET.get() {
        if !from_env.is_empty() && from_env != PLACEHOLDER_SECRET {
            return from_env.clone();
        }
    }
    PLACEHOLDER_SECRET.to_string()
}

/// 是否已配置有效的 client_id（任一来源命中非空值即 true）。
pub fn client_id_configured() -> bool {
    if !BUILD_CLIENT_ID.is_empty() {
        return true;
    }
    if let Some(from_env) = ENV_CLIENT_ID.get() {
        return !from_env.is_empty();
    }
    false
}

/// 是否已配置非占位符的 client_secret（任一来源命中即 true）。
/// UI 在登录按钮可点之前用此判断。
/// 对齐 dart `AppConstants.clientSecretConfigured`。
pub fn client_secret_configured() -> bool {
    if !BUILD_SECRET.is_empty() && BUILD_SECRET != PLACEHOLDER_SECRET {
        return true;
    }
    if let Some(from_env) = ENV_SECRET.get() {
        return !from_env.is_empty() && from_env != PLACEHOLDER_SECRET;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolved_secret_uses_placeholder_when_unset() {
        // 未调用 set_env_secret 时，应回退到占位符（除非构建期注入）
        if BUILD_SECRET.is_empty() {
            assert_eq!(resolved_client_secret(), PLACEHOLDER_SECRET);
            assert!(!client_secret_configured());
        }
    }

    #[test]
    fn test_resolved_client_id_empty_when_unset() {
        // 未设置任何来源时，CLIENT_ID 为空，不再有硬编码默认值
        if BUILD_CLIENT_ID.is_empty() {
            assert_eq!(resolved_client_id(), "");
            assert!(!client_id_configured());
        }
    }

    #[test]
    fn test_bundle_identifier_is_github() {
        // 关键断言：bundle id 为 io.github.yuanbaobaoo.PetalLink（原 io.gitcode.cloudmate 仅保留在 legacy/）
        assert_eq!(BUNDLE_IDENTIFIER, "io.github.yuanbaobaoo.PetalLink");
    }

    #[test]
    fn test_scopes_use_full_drive() {
        // 必须用 drive（全盘访问），不能用 drive.file（只能看本应用创建的文件）
        assert!(SCOPES.contains(&"https://www.huawei.com/auth/drive"));
        assert!(!SCOPES.iter().any(|s| s.contains("drive.file")));
    }

    #[test]
    fn test_loopback_only() {
        // 安全要求：仅绑定 127.0.0.1，不监听 0.0.0.0
        assert_eq!(LOOPBACK_HOST, "127.0.0.1");
    }
}

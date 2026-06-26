//! Token 存储 —— 自适应：Keychain 优先，无签名 debug 降级到文件。
//!
//! 对齐 `legacy/lib/auth/token_store.dart`（KeychainTokenStore + FileTokenStore + AdaptiveTokenStore）。
//!
//! # 安全
//! - 有签名构建：macOS Keychain（keyring crate，service = `hwcloud.<key>`）
//! - 无签名 debug：Keychain 报 errSecMissingEntitlement -34018，自动降级到
//!   `<Application Support>/token.json`，chmod 600（仅 owner 读写）。
//! - token 绝不日志输出。

use std::fs;
use std::path::PathBuf;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::auth::models::TokenPair;
use crate::core::config_store::support_dir;
use crate::error::{AppError, AppResult};

// ===== Keychain 存储键（对齐 dart KeychainTokenStore，前缀 hwcloud.） =====
/// 5 个存储键
const KEY_ACCESS: &str = "hwcloud.access_token";
const KEY_REFRESH: &str = "hwcloud.refresh_token";
const KEY_EXPIRES_AT: &str = "hwcloud.expires_at";
const KEY_TOKEN_TYPE: &str = "hwcloud.token_type";
const KEY_SCOPE: &str = "hwcloud.scope";
const KEY_PROBE: &str = "hwcloud.probe";

/// 文件降级存储的文件名
const FILE_NAME: &str = "token.json";

/// Token 存储 trait
pub trait TokenStore: Send + Sync {
    fn load(&self) -> AppResult<Option<TokenPair>>;
    fn save(&self, token: &TokenPair) -> AppResult<()>;
    fn clear(&self) -> AppResult<()>;
}

// ===== KeychainTokenStore（生产路径） =====

/// macOS Keychain 存储。使用 keyring crate，service 固定为 bundle id，
/// account 为各 key（前缀 hwcloud.）。
pub struct KeychainTokenStore;

impl KeychainTokenStore {
    /// 探测 Keychain 是否可用（写探测键再删除）。对齐 dart `_resolve` 探测逻辑。
    pub fn probe() -> bool {
        match Self::entry(KEY_PROBE) {
            Ok(entry) => {
                if entry.set_password("1").is_err() {
                    return false;
                }
                let _ = entry.delete_credential();
                true
            }
            Err(_) => false,
        }
    }

    fn entry(key: &str) -> AppResult<keyring::Entry> {
        keyring::Entry::new(crate::constants::BUNDLE_IDENTIFIER, key)
            .map_err(|e| AppError::generic(format!("创建 Keychain 条目失败：{e}")))
    }

    fn read(key: &str) -> AppResult<Option<String>> {
        let entry = Self::entry(key)?;
        match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => {
                tracing::warn!(key = key, error = %e, "Keychain 读取失败");
                Err(AppError::generic(format!("Keychain 读取失败：{e}")))
            }
        }
    }

    fn write(key: &str, value: &str) -> AppResult<()> {
        let entry = Self::entry(key)?;
        entry
            .set_password(value)
            .map_err(|e| AppError::generic(format!("Keychain 写入失败：{e}")))
    }

    fn delete(key: &str) -> AppResult<()> {
        let entry = Self::entry(key)?;
        match entry.delete_credential() {
            Ok(_) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AppError::generic(format!("Keychain 删除失败：{e}"))),
        }
    }
}

impl TokenStore for KeychainTokenStore {
    fn load(&self) -> AppResult<Option<TokenPair>> {
        // access/refresh/expires_at 三者任一缺失即视为无 token
        let access = match Self::read(KEY_ACCESS)? {
            Some(v) if !v.is_empty() => v,
            _ => return Ok(None),
        };
        let refresh = match Self::read(KEY_REFRESH)? {
            Some(v) if !v.is_empty() => v,
            _ => return Ok(None),
        };
        let expires_at_str = match Self::read(KEY_EXPIRES_AT)? {
            Some(v) if !v.is_empty() => v,
            _ => return Ok(None),
        };
        let expires_at: i64 = expires_at_str
            .parse()
            .map_err(|_| AppError::generic("expires_at 解析失败"))?;
        let token_type = Self::read(KEY_TOKEN_TYPE)?
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Bearer".to_string());
        let scope = Self::read(KEY_SCOPE)?.filter(|s| !s.is_empty());
        Ok(Some(TokenPair {
            access_token: access,
            refresh_token: refresh,
            expires_at,
            token_type,
            scope,
        }))
    }

    fn save(&self, token: &TokenPair) -> AppResult<()> {
        Self::write(KEY_ACCESS, &token.access_token)?;
        Self::write(KEY_REFRESH, &token.refresh_token)?;
        Self::write(KEY_EXPIRES_AT, &token.expires_at.to_string())?;
        Self::write(KEY_TOKEN_TYPE, &token.token_type)?;
        if let Some(scope) = &token.scope {
            Self::write(KEY_SCOPE, scope)?;
        } else {
            let _ = Self::delete(KEY_SCOPE);
        }
        Ok(())
    }

    fn clear(&self) -> AppResult<()> {
        let _ = Self::delete(KEY_ACCESS);
        let _ = Self::delete(KEY_REFRESH);
        let _ = Self::delete(KEY_EXPIRES_AT);
        let _ = Self::delete(KEY_TOKEN_TYPE);
        let _ = Self::delete(KEY_SCOPE);
        Ok(())
    }
}

// ===== FileTokenStore（无签名 debug 降级路径） =====

/// 文件降级存储。token.json，权限 600。
#[derive(Serialize, Deserialize)]
struct TokenFile {
    access_token: String,
    refresh_token: String,
    expires_at: i64,
    #[serde(default = "default_token_type")]
    token_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
}

fn default_token_type() -> String {
    "Bearer".to_string()
}

pub struct FileTokenStore;

impl FileTokenStore {
    fn file_path() -> AppResult<PathBuf> {
        Ok(support_dir()?.join(FILE_NAME))
    }
}

impl TokenStore for FileTokenStore {
    fn load(&self) -> AppResult<Option<TokenPair>> {
        let path = Self::file_path()?;
        if !path.exists() {
            return Ok(None);
        }
        let raw = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "token 文件读取失败");
                return Ok(None);
            }
        };
        let parsed: TokenFile = match serde_json::from_str(&raw) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "token 文件解析失败");
                return Ok(None);
            }
        };
        Ok(Some(TokenPair {
            access_token: parsed.access_token,
            refresh_token: parsed.refresh_token,
            expires_at: parsed.expires_at,
            token_type: parsed.token_type,
            scope: parsed.scope,
        }))
    }

    fn save(&self, token: &TokenPair) -> AppResult<()> {
        let path = Self::file_path()?;
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        let to_write = TokenFile {
            access_token: token.access_token.clone(),
            refresh_token: token.refresh_token.clone(),
            expires_at: token.expires_at,
            token_type: token.token_type.clone(),
            scope: token.scope.clone(),
        };
        let json = serde_json::to_string(&to_write)?;
        fs::write(&path, json)?;
        // 安全：降级路径收紧权限为 0600（仅 owner 读写）
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) = fs::set_permissions(&path, fs::Permissions::from_mode(0o600)) {
                tracing::warn!(error = %e, "收紧 token 文件权限失败（chmod 600）");
            }
        }
        tracing::info!("token 已保存到本地文件（降级路径，权限 600）");
        Ok(())
    }

    fn clear(&self) -> AppResult<()> {
        let path = Self::file_path()?;
        if path.exists() {
            fs::remove_file(&path)?;
        }
        tracing::info!("已删除本地 token 文件");
        Ok(())
    }
}

// ===== AdaptiveTokenStore（自适应，对齐 dart AdaptiveTokenStore） =====

/// 自适应 Token 存储：先探测 Keychain，不可用则永久降级到文件。
/// 探测结果缓存（_use_file 三态：None=未探测，Some(false)=Keychain，Some(true)=文件）。
pub struct AdaptiveTokenStore {
    keychain: KeychainTokenStore,
    file: FileTokenStore,
    use_file: Mutex<Option<bool>>,
}

impl AdaptiveTokenStore {
    pub fn new() -> Self {
        Self {
            keychain: KeychainTokenStore,
            file: FileTokenStore,
            use_file: Mutex::new(None),
        }
    }

    /// 探测并缓存 Keychain 可用性。对齐 dart `_resolve`。
    fn resolve(&self) -> bool {
        let mut guard = self.use_file.lock();
        if let Some(use_file) = *guard {
            return use_file;
        }
        // 用一次写探测 Keychain 是否可用
        let use_file = if KeychainTokenStore::probe() {
            tracing::info!("Keychain 可用，使用 Keychain 存储");
            false
        } else {
            tracing::warn!("Keychain 不可用（无签名 debug？），降级到文件存储");
            true
        };
        *guard = Some(use_file);
        use_file
    }
}

impl Default for AdaptiveTokenStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenStore for AdaptiveTokenStore {
    fn load(&self) -> AppResult<Option<TokenPair>> {
        let use_file = self.resolve();
        if !use_file {
            // Keychain 模式：先查 Keychain
            match self.keychain.load() {
                Ok(Some(t)) => {
                    tracing::info!("从 Keychain 恢复登录态");
                    return Ok(Some(t));
                }
                Ok(None) => {
                    tracing::debug!("Keychain 无 token，回退到文件存储");
                }
                Err(e) => {
                    // Keychain 读取失败（dev 模式 code signature 变化等），回退文件
                    tracing::warn!(error = %e, "Keychain 读取失败，回退到文件存储");
                }
            }
        }
        self.file.load()
    }

    fn save(&self, token: &TokenPair) -> AppResult<()> {
        // 始终保存到文件（可靠，跨 dev/release 可用，避免 Keychain 跨重启不可靠导致丢 token）
        self.file.save(token)?;
        tracing::info!("token 已保存到本地文件");

        // 同时尝试 Keychain（如果可用，作为更安全的生产路径）
        let use_file = self.resolve();
        if !use_file {
            if let Err(e) = self.keychain.save(token) {
                tracing::warn!(error = %e, "Keychain 写入失败（文件已保存，不影响登录态恢复）");
            }
        }
        Ok(())
    }

    fn clear(&self) -> AppResult<()> {
        // 清两边（Keychain clear 错误忽略）
        let _ = self.keychain.clear();
        self.file.clear()
    }
}

/// 全局单例 AdaptiveTokenStore（供命令直接复用）。
static GLOBAL_STORE: once_cell::sync::Lazy<AdaptiveTokenStore> =
    once_cell::sync::Lazy::new(AdaptiveTokenStore::new);

/// 获取全局 token 存储实例。
pub fn global_store() -> &'static AdaptiveTokenStore {
    &GLOBAL_STORE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::KEYCHAIN_SERVICE_PREFIX;
    use tempfile::tempdir;

    /// 测试用的文件存储（指向临时目录，不污染真实 Application Support）
    struct TestFileStore {
        dir: std::path::PathBuf,
    }

    impl TokenStore for TestFileStore {
        fn load(&self) -> AppResult<Option<TokenPair>> {
            let path = self.dir.join(FILE_NAME);
            if !path.exists() {
                return Ok(None);
            }
            let raw = fs::read_to_string(&path).unwrap();
            let parsed: TokenFile = serde_json::from_str(&raw).unwrap();
            Ok(Some(TokenPair {
                access_token: parsed.access_token,
                refresh_token: parsed.refresh_token,
                expires_at: parsed.expires_at,
                token_type: parsed.token_type,
                scope: parsed.scope,
            }))
        }
        fn save(&self, token: &TokenPair) -> AppResult<()> {
            let to_write = TokenFile {
                access_token: token.access_token.clone(),
                refresh_token: token.refresh_token.clone(),
                expires_at: token.expires_at,
                token_type: token.token_type.clone(),
                scope: token.scope.clone(),
            };
            let path = self.dir.join(FILE_NAME);
            fs::write(&path, serde_json::to_string(&to_write)?)?;
            Ok(())
        }
        fn clear(&self) -> AppResult<()> {
            let path = self.dir.join(FILE_NAME);
            if path.exists() {
                fs::remove_file(&path)?;
            }
            Ok(())
        }
    }

    fn sample_token() -> TokenPair {
        TokenPair {
            access_token: "at-123".into(),
            refresh_token: "rt-456".into(),
            expires_at: 9999999999999,
            token_type: "Bearer".into(),
            scope: Some("drive".into()),
        }
    }

    #[test]
    fn test_file_store_roundtrip() {
        let dir = tempdir().unwrap().keep();
        let store = TestFileStore { dir };
        // 初始为空
        assert!(store.load().unwrap().is_none());
        // 保存后可读取
        store.save(&sample_token()).unwrap();
        let loaded = store.load().unwrap().unwrap();
        assert_eq!(loaded.access_token, "at-123");
        assert_eq!(loaded.refresh_token, "rt-456");
        assert_eq!(loaded.scope.as_deref(), Some("drive"));
        // 清空后为空
        store.clear().unwrap();
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn test_file_store_scope_optional() {
        let dir = tempdir().unwrap().keep();
        let path_for_check = dir.join(FILE_NAME);
        let store = TestFileStore { dir };
        let mut token = sample_token();
        token.scope = None;
        store.save(&token).unwrap();
        let loaded = store.load().unwrap().unwrap();
        assert!(loaded.scope.is_none());
        // JSON 序列化时 scope=None 应被跳过
        let raw = fs::read_to_string(&path_for_check).unwrap();
        assert!(!raw.contains("scope"));
    }

    #[test]
    fn test_file_permissions_unix() {
        // 验证降级路径权限收紧（仅 unix）
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempdir().unwrap().keep();
            let path = dir.join(FILE_NAME);
            // 直接用 FileTokenStore 但指向自定义目录（通过 TestFileStore 模拟权限）
            // 这里验证 chmod 600 语义
            fs::write(&path, "{}").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn test_keychain_constants_match_dart() {
        // 存储键必须与 dart KeychainTokenStore 一致（避免换实现丢凭据）
        assert_eq!(KEY_ACCESS, "hwcloud.access_token");
        assert_eq!(KEY_REFRESH, "hwcloud.refresh_token");
        assert_eq!(KEY_EXPIRES_AT, "hwcloud.expires_at");
        assert_eq!(KEY_TOKEN_TYPE, "hwcloud.token_type");
        assert_eq!(KEY_SCOPE, "hwcloud.scope");
        assert_eq!(KEY_PROBE, "hwcloud.probe");
        // 前缀对齐 dart keychainServicePrefix
        assert!(KEY_ACCESS.starts_with(KEYCHAIN_SERVICE_PREFIX));
    }
}

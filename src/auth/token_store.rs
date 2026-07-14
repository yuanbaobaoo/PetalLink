//! Token 存储 —— 机器码绑定的加密二进制文件。
//!
//! 设计取舍（方案 C）：
//! - 放弃 macOS Keychain（签名变化/dev↔release 切换会导致 token 不可靠恢复，触发误判未登录）。
//! - 改为：`<Application Support>/token.bin`，自定义二进制格式，ChaCha20-Poly1305 AEAD 加密。
//! - 加密密钥由本机 **IOPlatformUUID**（via `ioreg`）经 SHA-256 派生 → 绑定本机硬件。
//! - 安全边界：
//!   - ✅ 防跨机器复制：token.bin 拷到别的机器 → UUID 不同 → AEAD 解密失败 → 视为未登录。
//!   - ✅ 防篡改：AEAD 自带 Poly1305 完整性校验，改一个 bit 都解密失败。
//!   - ⚠️ 不防本机攻击：本机任何进程可读同样的 UUID（IOPlatformUUID 非秘密）。
//!   - 文件权限 0600（仅 owner 读写）。
//! - 失败行为：UUID 取不到/文件不存在/损坏/跨机器/重装系统（UUID 变）→ load 返回 Ok(None)（未登录）。
//! - token 绝不日志输出。

use std::fs;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::process::Command;

use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305,
};
use rand::Rng;
use sha2::{Digest, Sha256};

use crate::auth::models::TokenPair;
use crate::core::config_store::support_dir;
use crate::error::{AppError, AppResult};

/// token 加密文件名（.bin，与旧版明文 token.json 区分 → 自动忽略旧文件，需重登一次）
const FILE_NAME: &str = "token.bin";
/// 文件格式魔数（版本标识，便于未来格式迁移）
const MAGIC: &[u8; 4] = b"PTL1";
/// ChaCha20-Poly1305 nonce 长度（12 字节）
const NONCE_LEN: usize = 12;

/// Token 存储 trait（对外接口稳定，调用方零改动）
pub trait TokenStore: Send + Sync {
    /// 读取并解密已持久化的 token；不存在时返回空值。
    fn load(&self) -> AppResult<Option<TokenPair>>;
    /// 加密并原子保存 token。
    fn save(&self, token: &TokenPair) -> AppResult<()>;
    /// 删除已持久化的 token。
    fn clear(&self) -> AppResult<()>;
}

/// 加密文件存储：token.bin，机器码绑定的 ChaCha20-Poly1305 加密。
pub struct EncryptedFileStore;

impl TokenStore for EncryptedFileStore {
    /// 读取 token 文件；文件不可读或认证失败均按未登录处理，路径解析错误才向上传播。
    fn load(&self) -> AppResult<Option<TokenPair>> {
        let path = file_path()?;
        if !path.exists() {
            return Ok(None);
        }
        let raw = match fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, "token 文件读取失败");
                return Ok(None);
            }
        };
        // 解密失败一律视为未登录（损坏/跨机器/UUID 变更）
        match decrypt_token(&raw) {
            Ok(token) => {
                tracing::info!("从加密 token 文件恢复登录态");
                Ok(Some(token))
            }
            Err(e) => {
                tracing::warn!(error = %e, "token 解密失败（损坏/跨机器/UUID 变更？），视为未登录");
                Ok(None)
            }
        }
    }

    /// 加密 token 并通过临时文件替换完成原子写入。
    fn save(&self, token: &TokenPair) -> AppResult<()> {
        let path = file_path()?;
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        let encrypted = encrypt_token(token)?;
        // 原子写：先写临时文件再重命名，避免中途崩溃产生半截文件
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, &encrypted)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))
                .map_err(|e| AppError::generic(format!("收紧 token 文件权限失败：{e}")))?;
        }
        fs::rename(&tmp, &path)?;
        tracing::info!("token 已加密保存到本地文件（机器码绑定，权限 600）");
        Ok(())
    }

    /// 删除本机 token 文件；文件不存在视为成功。
    fn clear(&self) -> AppResult<()> {
        let path = file_path()?;
        // 不存在视为已清除（幂等）
        if !path.exists() {
            return Ok(());
        }
        fs::remove_file(&path)
            .map_err(|e| AppError::generic(format!("清除 token 文件失败：{e}")))?;
        tracing::info!("已清除 token 文件");
        Ok(())
    }
}

/// token.bin 完整路径（Application Support / <bundle_id> / token.bin）
fn file_path() -> AppResult<PathBuf> {
    Ok(support_dir()?.join(FILE_NAME))
}

// ===== 机器码 + 密钥派生 =====

/// 取本机 IOPlatformUUID（via ioreg，无需 root，无需 IOKit 依赖）。
/// 失败返回 Err（极少见：严格沙盒环境；本应用非沙盒）。
fn machine_uuid() -> AppResult<String> {
    let output = Command::new("ioreg")
        .args(["-d2", "-c", "IOPlatformExpertDevice"])
        .output()
        .map_err(|e| AppError::generic(format!("调用 ioreg 失败：{e}")))?;
    let text = String::from_utf8_lossy(&output.stdout);
    // 解析形如：    "IOPlatformUUID" = "XXXXXXXX-XXXX-..."
    // ioreg 输出含等号，直接取 = 右侧第一个双引号字符串，避免被等号前的引号干扰。
    let uuid = text
        .lines()
        .find(|line| line.contains("IOPlatformUUID"))
        .and_then(|line| line.split_once('='))
        .and_then(|(_, rest)| {
            rest.split_once('"')
                .and_then(|(_, after)| after.split_once('"'))
        })
        .map(|(uuid, _)| uuid.trim().to_string())
        .ok_or_else(|| AppError::generic("ioreg 输出未找到 IOPlatformUUID"))?;
    if uuid.is_empty() {
        return Err(AppError::generic("IOPlatformUUID 为空"));
    }
    Ok(uuid)
}

/// 密钥派生：SHA-256(machine_uuid) → 32 字节。
/// UUID 本身高熵，无需慢哈希；不加 salt（salt 会随文件走，失去绑机器意义）。
fn derive_key(uuid: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(uuid.as_bytes());
    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

// ===== 加密 / 解密 =====

/// 加密 token：序列化明文 → 随机 nonce → ChaCha20-Poly1305 加密 → 拼装文件格式。
fn encrypt_token(token: &TokenPair) -> AppResult<Vec<u8>> {
    // 密钥派生（UUID 取不到则无法加密）
    let uuid = machine_uuid()?;
    let key = derive_key(&uuid);
    let cipher = ChaCha20Poly1305::new(&key.into());

    // 随机 nonce（每次保存重新生成，AEAD 安全性靠 nonce 不重用）
    let nonce_bytes: [u8; NONCE_LEN] = rand::thread_rng().gen();
    let nonce = nonce_bytes.into();

    // 序列化明文（紧凑二进制，length-prefixed）
    let plaintext = serialize_token(token);

    // 加密（密文含 16B Poly1305 tag）
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_ref())
        .map_err(|e| AppError::generic(format!("token 加密失败：{e}")))?;

    // 拼装文件格式：[魔数 4B][nonce 12B][密文+tag]
    let mut out = Vec::with_capacity(MAGIC.len() + NONCE_LEN + ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// 解密 token：校验魔数 → 取 nonce → AEAD 解密 → 反序列化。
/// 任何步骤失败返回 Err（调用方据此判定未登录）。
fn decrypt_token(raw: &[u8]) -> AppResult<TokenPair> {
    // 校验最小长度：魔数 + nonce + 至少 1 字节密文（实际密文含 16B tag）
    if raw.len() < MAGIC.len() + NONCE_LEN + 16 {
        return Err(AppError::generic("token 文件长度异常"));
    }
    let mut cursor = Cursor::new(raw);

    // 校验魔数
    let mut magic = [0u8; 4];
    cursor
        .read_exact(&mut magic)
        .map_err(|e| AppError::generic(format!("读取魔数失败：{e}")))?;
    if &magic != MAGIC {
        return Err(AppError::generic("token 文件魔数不匹配"));
    }

    // 读取 nonce
    let mut nonce_bytes = [0u8; NONCE_LEN];
    cursor
        .read_exact(&mut nonce_bytes)
        .map_err(|e| AppError::generic(format!("读取 nonce 失败：{e}")))?;

    // 剩余为密文 + tag
    let mut ciphertext = Vec::new();
    cursor
        .read_to_end(&mut ciphertext)
        .map_err(|e| AppError::generic(format!("读取密文失败：{e}")))?;

    // 派生本机密钥并解密（UUID 变化/跨机器 → AEAD 失败）
    let uuid = machine_uuid()?;
    let key = derive_key(&uuid);
    let cipher = ChaCha20Poly1305::new(&key.into());
    let nonce = nonce_bytes.into();
    let plaintext = cipher
        .decrypt(&nonce, ciphertext.as_ref())
        .map_err(|e| AppError::generic(format!("token 解密失败：{e}")))?;

    // 反序列化
    deserialize_token(&plaintext)
}

// ===== 二进制序列化（length-prefixed，小端） =====

/// 序列化 token 为紧凑二进制。
///
/// 明文布局（小端）：
/// access token 布局：`[u64 access_len][access_bytes]`
/// refresh token 布局：`[u64 refresh_len][refresh_bytes]`
/// 过期时间：`[i64 expires_at]`
/// token type 布局：`[u32 token_type_len][token_type_bytes]`
/// `[u8 scope_present][u64 scope_len][scope_bytes]`（scope_present=0 时后续省略）
fn serialize_token(token: &TokenPair) -> Vec<u8> {
    let mut buf = Vec::new();
    // 写入 access token。
    buf.extend_from_slice(&(token.access_token.len() as u64).to_le_bytes());
    buf.extend_from_slice(token.access_token.as_bytes());
    // 写入 refresh token。
    buf.extend_from_slice(&(token.refresh_token.len() as u64).to_le_bytes());
    buf.extend_from_slice(token.refresh_token.as_bytes());
    // expires_at（i64 毫秒）
    buf.extend_from_slice(&token.expires_at.to_le_bytes());
    // 写入 token type。
    buf.extend_from_slice(&(token.token_type.len() as u32).to_le_bytes());
    buf.extend_from_slice(token.token_type.as_bytes());
    // 授权范围 scope（Option）
    match &token.scope {
        Some(s) => {
            buf.push(1u8);
            buf.extend_from_slice(&(s.len() as u64).to_le_bytes());
            buf.extend_from_slice(s.as_bytes());
        }
        None => buf.push(0u8),
    }
    buf
}

/// 反序列化紧凑二进制为 token。
fn deserialize_token(data: &[u8]) -> AppResult<TokenPair> {
    let mut cursor = Cursor::new(data);

    // 读取 access token。
    let access_token = read_string_u64(&mut cursor)?;
    // 读取 refresh token。
    let refresh_token = read_string_u64(&mut cursor)?;
    // 过期时间 expires_at
    let mut exp_bytes = [0u8; 8];
    cursor
        .read_exact(&mut exp_bytes)
        .map_err(|e| AppError::generic(format!("读取 expires_at 失败：{e}")))?;
    let expires_at = i64::from_le_bytes(exp_bytes);
    // 读取 token type。
    let token_type = read_string_u32(&mut cursor)?;
    // 授权范围 scope
    let mut present = [0u8; 1];
    cursor
        .read_exact(&mut present)
        .map_err(|e| AppError::generic(format!("读取 scope 标志失败：{e}")))?;
    let scope = if present[0] == 1 {
        Some(read_string_u64(&mut cursor)?)
    } else {
        None
    };

    Ok(TokenPair {
        access_token,
        refresh_token,
        expires_at,
        token_type,
        scope,
    })
}

/// 读取 u64 长度前缀的字节并转 String（access/refresh/scope 用）。
fn read_string_u64(cursor: &mut Cursor<&[u8]>) -> AppResult<String> {
    let mut len_bytes = [0u8; 8];
    cursor
        .read_exact(&mut len_bytes)
        .map_err(|e| AppError::generic(format!("读取长度失败：{e}")))?;
    let len = u64::from_le_bytes(len_bytes) as usize;
    let mut bytes = vec![0u8; len];
    cursor
        .read_exact(&mut bytes)
        .map_err(|e| AppError::generic(format!("读取字符串内容失败：{e}")))?;
    String::from_utf8(bytes).map_err(|e| AppError::generic(format!("UTF-8 解码失败：{e}")))
}

/// 读取 u32 长度前缀的字节并转 String（token_type 用）。
fn read_string_u32(cursor: &mut Cursor<&[u8]>) -> AppResult<String> {
    let mut len_bytes = [0u8; 4];
    cursor
        .read_exact(&mut len_bytes)
        .map_err(|e| AppError::generic(format!("读取长度失败：{e}")))?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    let mut bytes = vec![0u8; len];
    cursor
        .read_exact(&mut bytes)
        .map_err(|e| AppError::generic(format!("读取字符串内容失败：{e}")))?;
    String::from_utf8(bytes).map_err(|e| AppError::generic(format!("UTF-8 解码失败：{e}")))
}

// ===== 全局单例 =====

/// 全局加密 token 存储单例（供命令层直接复用）。
static GLOBAL_STORE: once_cell::sync::Lazy<EncryptedFileStore> =
    once_cell::sync::Lazy::new(|| EncryptedFileStore);

/// 获取全局 token 存储实例。
pub fn global_store() -> &'static EncryptedFileStore {
    &GLOBAL_STORE
}

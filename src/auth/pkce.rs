//! PKCE (Proof Key for Code Exchange) 生成器（OAuth 2.0 增强安全）。
//!
//! 对齐 `legacy/lib/util/security.dart`。
//!
//! 华为 OAuth 支持授权码 + PKCE。code_verifier 随机生成，code_challenge
//! 为其 SHA256 的 base64url。授权请求带 challenge，换 token 时带 verifier。

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

/// PKCE 对
#[derive(Debug, Clone)]
pub struct PkcePair {
    /// 原始随机串，换 token 时回传给华为（仅本次会话使用）
    pub code_verifier: String,
    /// code_verifier 的 S256 摘要 base64url（去 = 填充），授权请求携带
    pub code_challenge: String,
}

impl std::fmt::Display for PkcePair {
    /// 以不含 verifier 的摘要形式输出，避免泄露授权凭据。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 故意隐藏 verifier（对齐 dart toString）
        write!(
            f,
            "PkcePair(challenge={}, verifier=<hidden>)",
            self.code_challenge
        )
    }
}

/// 生成随机 state（防 CSRF），返回 32 字节 hex（64 字符）。
/// 对齐 dart `generateState`。
pub fn generate_state() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// 生成 PKCE 对。code_verifier 为 64 字节随机数的 base64url（去 = 填充），
/// 长度约 86 字符（在 RFC 43-128 范围内）。method = S256。
/// 对齐 dart `generatePkce`。
pub fn generate_pkce() -> PkcePair {
    let mut bytes = [0u8; 64];
    rand::thread_rng().fill_bytes(&mut bytes);
    // base64url 去 = 填充（对齐 dart replaceAll('=', '')）
    let verifier = URL_SAFE_NO_PAD.encode(bytes);

    // SHA256(verifier) → base64url 去 =
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let digest = hasher.finalize();
    let challenge = URL_SAFE_NO_PAD.encode(digest);

    PkcePair {
        code_verifier: verifier,
        code_challenge: challenge,
    }
}

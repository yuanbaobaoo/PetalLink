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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_is_64_hex_chars() {
        let state = generate_state();
        assert_eq!(state.len(), 64);
        assert!(state.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_state_is_random() {
        let s1 = generate_state();
        let s2 = generate_state();
        assert_ne!(s1, s2);
    }

    #[test]
    fn test_pkce_verifier_length_in_rfc_range() {
        let pkce = generate_pkce();
        // RFC 7636: 43-128 字符
        assert!(
            (43..=128).contains(&pkce.code_verifier.len()),
            "verifier 长度 {} 不在 RFC 范围",
            pkce.code_verifier.len()
        );
    }

    #[test]
    fn test_pkce_verifier_no_padding() {
        let pkce = generate_pkce();
        // 不应含 = 填充
        assert!(!pkce.code_verifier.contains('='));
        assert!(!pkce.code_challenge.contains('='));
    }

    #[test]
    fn test_pkce_challenge_is_sha256_of_verifier() {
        // 手动验证 challenge = base64url(SHA256(verifier))
        let pkce = generate_pkce();
        let mut hasher = Sha256::new();
        hasher.update(pkce.code_verifier.as_bytes());
        let digest = hasher.finalize();
        let expected = URL_SAFE_NO_PAD.encode(digest);
        assert_eq!(pkce.code_challenge, expected);
    }

    #[test]
    fn test_pkce_is_random() {
        let p1 = generate_pkce();
        let p2 = generate_pkce();
        assert_ne!(p1.code_verifier, p2.code_verifier);
        assert_ne!(p1.code_challenge, p2.code_challenge);
    }

    #[test]
    fn test_display_hides_verifier() {
        let pkce = generate_pkce();
        let s = format!("{pkce}");
        assert!(s.contains("<hidden>"));
        assert!(!s.contains(&pkce.code_verifier));
    }
}

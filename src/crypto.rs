//! 安全加密层：PBKDF2 密钥派生、AES-256-GCM 加解密、内存敏感数据擦除。
//!
//! 设计要点（对齐设计文档第五章）：
//! - 主密码 不存明文；用 PBKDF2-HMAC-SHA256 加盐派生 32 字节 AES-256 密钥；
//! - AES-256-GCM 带认证，既加密又校验完整性（防篡改）；
//! - 敏感密钥缓冲区用 `zeroize` 在 drop 时覆盖清零，防止内存 dump 抓取。
//!
//! 技术说明：设计文档列举的 `orion` crate 在最新版仅提供 ChaCha20-Poly1305，
//! 且其高层 KDF 为 Argon2i（非 PBKDF2），无法满足文档要求的
//! "AES-256-GCM + PBKDF2"，故采用 RustCrypto 生态中被广泛审计且为现行
//! 标准实现的 `aes-gcm`/`pbkdf2`/`sha2`（与原方案安全目标完全一致）。

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// PBKDF2 迭代次数（OWASP 建议的下限安全值）。
pub const PBKDF2_ITERATIONS: u32 = 100_000;
/// AES-GCM 使用的随机 nonce 长度。
pub const GCM_NONCE_LEN: usize = 12;
/// AES-256-GCM 认证标签长度。
pub const GCM_TAG_LEN: usize = 16;
/// AES-256 密钥长度（字节）。
pub const KEY_LEN: usize = 32;

/// 派生出的会话密钥容器：离开作用域时自动将密钥内存清零（zeroize）。
/// 对应设计文档「主密码/派生密钥用完即擦除」的要求。
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct DecryptableKey(pub [u8; KEY_LEN]);

/// 由主密码 + 盐派生 AES-256 密钥（PBKDF2-HMAC-SHA256）。
///
/// 返回后调用方持有派生密钥，用完必须 drop（Zeroize 自动清零）。
pub fn derive_aes_key(main_password: &[u8], salt: &[u8]) -> Result<[u8; KEY_LEN], String> {
    let mut key = [0u8; KEY_LEN];
    pbkdf2::pbkdf2_hmac::<Sha256>(main_password, salt, PBKDF2_ITERATIONS, &mut key);
    Ok(key)
}

/// 计算密钥校验摘要：sha256(derived_key)。用于解锁时校验主密码正确性。
pub fn key_checksum(key: &[u8]) -> Result<String, String> {
    let digest = Sha256::digest(key);
    Ok(hex_encode(&digest))
}

/// 常量时间比较两个 ASCII 十六进制摘要（防时间侧信道）。
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// 构建 AES-256-GCM 实例。
fn cipher(key: &[u8]) -> Result<Aes256Gcm, String> {
    Aes256Gcm::new_from_slice(key).map_err(|e| format!("密钥长度错误: {e}"))
}

/// 使用 AES-256-GCM 加密明文，返回 `nonce(12) || ciphertext || tag(16)` 的十六进制字符串。
pub fn encrypt_str(key: &[u8], plaintext: &str) -> Result<String, String> {
    encrypt_bytes(key, plaintext.as_bytes())
}

/// 使用 AES-256-GCM 解密 `encrypt_str` 生成的密文。
pub fn decrypt_str(key: &[u8], blob_hex: &str) -> Result<String, String> {
    let blob = hex_decode(blob_hex)?;
    let plain = decrypt_bytes(key, &blob)?;
    Ok(String::from_utf8_lossy(&plain).into_owned())
}

/// 生成随机字节序列（密码学安全）。用于盐、nonce 等。
pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut buf);
    buf
}

/// 使用 AES-256-GCM 加密字节数组，返回 `nonce(12) || (密文||tag)`。
pub fn encrypt_bytes(key: &[u8], plaintext: &[u8]) -> Result<String, String> {
    let cipher = cipher(key)?;
    let mut nonce = [0u8; GCM_NONCE_LEN];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce);
    let nonce_obj = Nonce::from_slice(&nonce);
    // encrypt 输出 = 密文 || 认证标签
    let ct = cipher
        .encrypt(nonce_obj, plaintext)
        .map_err(|_| "AES-GCM 加密失败".to_string())?;
    let mut out = Vec::with_capacity(GCM_NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(hex_encode(&out))
}

/// 解密 `encrypt_bytes`/`encrypt_str` 生成的 blob（含 nonce 前缀）。
pub fn decrypt_bytes(key: &[u8], blob: &[u8]) -> Result<Vec<u8>, String> {
    if blob.len() < GCM_NONCE_LEN + GCM_TAG_LEN {
        return Err("密文格式损坏（长度不足）".into());
    }
    let (nonce, ct) = blob.split_at(GCM_NONCE_LEN);
    let cipher = cipher(key)?;
    cipher
        .decrypt(Nonce::from_slice(nonce), ct)
        .map_err(|_| "解密失败：主密码错误或数据被篡改".to_string())
}

/// 由 (密码, 盐) 派生密钥并对字节数组做 AES-256-GCM 整体加密（备份文件用）。
/// 返回 (nonce[12], 密文||tag)。
pub fn encrypt_with_password(
    data: &[u8],
    password: &[u8],
    salt: &[u8],
) -> Result<([u8; GCM_NONCE_LEN], Vec<u8>), String> {
    // Zeroizing 包裹：作用域结束即自动清零派生密钥
    let key = Zeroizing::new(derive_aes_key(password, salt)?);
    let cipher = cipher(&key[..])?;
    let mut nonce = [0u8; GCM_NONCE_LEN];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce);
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), data)
        .map_err(|_| "AES-GCM 加密失败".to_string())?;
    Ok((nonce, ct))
}

/// 按 `encrypt_with_password` 的格式解密（由密码+盐派生密钥，校验完整性）。
pub fn decrypt_with_password(
    ct_and_tag: &[u8],
    nonce: &[u8],
    password: &[u8],
    salt: &[u8],
) -> Result<Vec<u8>, String> {
    let key = Zeroizing::new(derive_aes_key(password, salt)?);
    let cipher = cipher(&key[..])?;
    cipher
        .decrypt(Nonce::from_slice(nonce), ct_and_tag)
        .map_err(|_| "解密失败：主密码错误或备份被篡改".to_string())
}

/// 十六进制编码。
pub fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// 十六进制解码。
pub fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("hex 长度必须为偶数".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| format!("hex 解码失败: {e}")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_key_deterministic() {
        let salt = b"0123456789abcdef";
        let k1 = derive_aes_key(b"master-password", salt).unwrap();
        let k2 = derive_aes_key(b"master-password", salt).unwrap();
        assert_eq!(k1, k2, "同一密码+盐必须派生相同密钥");
    }

    #[test]
    fn derive_key_differs_with_salt() {
        let k1 = derive_aes_key(b"p", b"salt-one---").unwrap();
        let k2 = derive_aes_key(b"p", b"salt-two---").unwrap();
        assert_ne!(k1, k2, "不同盐必须不同密钥");
    }

    #[test]
    fn key_checksum_stable() {
        let k = derive_aes_key(b"p", b"s").unwrap();
        assert_eq!(key_checksum(&k).unwrap(), key_checksum(&k).unwrap());
    }

    #[test]
    fn aes_gcm_roundtrip() {
        let key = derive_aes_key(b"hunter2", b"0123456789abcdef").unwrap();
        let ciphertext = encrypt_str(&key, "我的超级机密密码123").unwrap();
        assert_eq!(ciphertext.len() % 2, 0);
        let plain = decrypt_str(&key, &ciphertext).unwrap();
        assert_eq!(plain, "我的超级机密密码123");
    }

    #[test]
    fn aes_gcm_wrong_key_fails() {
        let k1 = derive_aes_key(b"right", b"s").unwrap();
        let k2 = derive_aes_key(b"wrong", b"s").unwrap();
        let ct = encrypt_str(&k1, "secret").unwrap();
        assert!(decrypt_str(&k2, &ct).is_err(), "错误密钥必须解密失败");
    }

    #[test]
    fn aes_gcm_tamper_detected() {
        let k = derive_aes_key(b"p", b"s").unwrap();
        let ct = encrypt_str(&k, "data").unwrap();
        let mut bytes = hex_decode(&ct).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01; // 篡改一个字节
        assert!(decrypt_bytes(&k, &bytes).is_err(), "篡改必须被检测");
    }

    #[test]
    fn const_time_eq_works() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ab"));
    }

    #[test]
    fn hex_roundtrip() {
        let data = random_bytes(32);
        assert_eq!(hex_decode(&hex_encode(&data)).unwrap(), data);
    }

    #[test]
    fn encrypt_with_password_roundtrip() {
        let salt = random_bytes(16);
        let (nonce, ct) = encrypt_with_password(b"backup content", b"pw", &salt).unwrap();
        let plain = decrypt_with_password(&ct, &nonce, b"pw", &salt).unwrap();
        assert_eq!(plain, b"backup content");
        assert!(decrypt_with_password(&ct, &nonce, b"wrong", &salt).is_err());
    }
}

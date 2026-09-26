//! 备份与恢复：自定义 `.pwbackup` 格式。
//!
//! 文件结构（整体 AES-256-GCM 加密，无主密码无法解密 —— 设计文档 5.4）：
//! ```text
//! [8B magic "PWB1.2.0"] [16B salt] [12B nonce] [密文||tag ...]
//! ```
//! 密文 = AES-GCM(bincode(BackupPayload))，密钥由 (主密码, salt) PBKDF2 派生。
//! salt 单独明文存于文件头部，restore 时据此派生密钥（与数据库同思路）。

use crate::crypto;
use crate::model::PlainRecord;
use serde::{Deserialize, Serialize};

const MAGIC: &[u8; 8] = b"PWB1.2.0";
const SALT_LEN: usize = 16;

#[derive(Serialize, Deserialize)]
struct Payload {
    version: u32,
    records: Vec<PlainRecord>,
}

/// 导出备份：master_password 用于派生备份密钥；导出前先校验主密码正确。
pub fn export_backup(
    records: &[PlainRecord],
    master_password: &str,
    salt: &[u8],
) -> Result<Vec<u8>, String> {
    if salt.len() != SALT_LEN {
        return Err("盐长度必须为 16 字节".into());
    }
    let payload = Payload {
        version: 1,
        records: records.to_vec(),
    };
    let serialized = bincode::serialize(&payload).map_err(|e| format!("序列化备份失败: {e}"))?;
    // 主密码错误会在此处失败（密钥派生后再校验由解密完成），提前暴露错误
    let _ = crypto::derive_aes_key(master_password.as_bytes(), salt)?;

    let (nonce, ct) = crypto::encrypt_with_password(&serialized, master_password.as_bytes(), salt)?;
    let mut file = Vec::with_capacity(MAGIC.len() + SALT_LEN + nonce.len() + ct.len());
    file.extend_from_slice(MAGIC);
    file.extend_from_slice(salt);
    file.extend_from_slice(&nonce);
    file.extend_from_slice(&ct);
    Ok(file)
}

/// 恢复备份：校验 magic、用主密码解密并反序列化。
pub fn import_backup(file: &[u8], master_password: &str) -> Result<Vec<PlainRecord>, String> {
    if file.len() < MAGIC.len() + SALT_LEN + crypto::GCM_NONCE_LEN + crypto::GCM_TAG_LEN {
        return Err("备份文件损坏（长度不足）".into());
    }
    if &file[..MAGIC.len()] != MAGIC {
        return Err("不是有效的 .pwbackup 备份文件".into());
    }
    let salt = &file[MAGIC.len()..MAGIC.len() + SALT_LEN];
    let nonce = &file[MAGIC.len() + SALT_LEN..MAGIC.len() + SALT_LEN + crypto::GCM_NONCE_LEN];
    let ct = &file[MAGIC.len() + SALT_LEN + crypto::GCM_NONCE_LEN..];

    let plain = crypto::decrypt_with_password(ct, nonce, master_password.as_bytes(), salt)?;
    let payload: Payload = bincode::deserialize(&plain)
        .map_err(|_| "解析备份内容失败（密码错误或文件损坏）".to_string())?;
    Ok(payload.records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::random_bytes;

    fn sample_records() -> Vec<PlainRecord> {
        vec![
            PlainRecord {
                id: Some(1),
                category: "办公".into(),
                name: "GitHub".into(),
                account: "alice".into(),
                password: "Ac3#k9!q".into(),
                url: "https://github.com".into(),
                phone: "139".into(),
                remark: "代码托管".into(),
                create_time: "2026-01-01 09:00:00".into(),
                update_time: "2026-01-01 09:00:00".into(),
            },
            PlainRecord {
                id: Some(2),
                category: "娱乐".into(),
                name: "Steam".into(),
                account: "bob".into(),
                password: "G$h2@Lm8".into(),
                url: "".into(),
                phone: "".into(),
                remark: "".into(),
                create_time: "2026-02-03 10:00:00".into(),
                update_time: "2026-02-03 10:00:00".into(),
            },
        ]
    }

    #[test]
    fn backup_roundtrip() {
        let salt = random_bytes(SALT_LEN);
        let records = sample_records();
        let file = export_backup(&records, "master-pw", &salt).unwrap();
        let restored = import_backup(&file, "master-pw").unwrap();
        assert_eq!(restored.len(), 2);
        assert_eq!(restored[0].name, "GitHub");
        assert_eq!(restored[0].password, "Ac3#k9!q");
        assert_eq!(restored[1].account, "bob");
    }

    #[test]
    fn wrong_password_fails() {
        let salt = random_bytes(SALT_LEN);
        let file = export_backup(&sample_records(), "master-pw", &salt).unwrap();
        assert!(import_backup(&file, "wrong-pw").is_err());
    }

    #[test]
    fn tamper_detected() {
        let salt = random_bytes(SALT_LEN);
        let file = export_backup(&sample_records(), "master-pw", &salt).unwrap();
        let mut file = file.clone();
        let last = file.len() - 1;
        file[last] ^= 0x01;
        assert!(import_backup(&file, "master-pw").is_err());
    }

    #[test]
    fn bad_magic_rejected() {
        let bad = b"NOTABACKUPFILE...abc...".to_vec();
        assert!(import_backup(&bad, "pw").is_err());
    }
}

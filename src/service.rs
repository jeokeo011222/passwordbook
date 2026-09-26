//! 业务逻辑层：把加密层与持久化层组合起来，对上层提供面向明文的操作。
//! 包括首次设置、解锁、记录 CRUD、修改主密码、分类集合、清空数据等。

use crate::crypto::{key_checksum, random_bytes, DecryptableKey};
use crate::db;
use crate::model::{EncRow, PlainRecord};
use chrono::Local;
use rusqlite::Connection;
use zeroize::Zeroizing;

/// 解锁失败原因区分（用于防暴力锁定展示）。
#[derive(Debug)]
pub enum UnlockError {
    WrongPassword,
    Db(String),
}

impl std::fmt::Display for UnlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnlockError::WrongPassword => write!(f, "主密码错误"),
            UnlockError::Db(e) => write!(f, "数据库错误: {e}"),
        }
    }
}

/// 首次启动：创建主密码并初始化系统。
/// 生成随机盐 -> PBKDF2 派生密钥 -> 存储盐与密钥校验摘要。
/// 密钥不单独保存（设计文档 3.1.1），仅存在于调用方到达作用域即被 zeroize。
pub fn setup(
    conn: &Connection,
    master_password: &str,
    lock_seconds: i64,
) -> Result<DecryptableKey, String> {
    if db::has_config(conn)? {
        return Err("系统已初始化，请直接解锁".into());
    }
    if master_password.is_empty() {
        return Err("主密码不能为空".into());
    }
    if !(30..=3600).contains(&lock_seconds) {
        return Err("自动锁屏时间需在 30~3600 秒之间".into());
    }
    let salt = random_bytes(16);
    let key = crate::crypto::derive_aes_key(master_password.as_bytes(), &salt)?;
    let checksum = key_checksum(&key)?;
    db::init_config(
        conn,
        &crate::crypto::hex_encode(&salt),
        &checksum,
        lock_seconds,
        0, // theme 默认浅色
        1, // require_login 默认开启
    )?;
    Ok(DecryptableKey(key))
}

/// 解锁：读取盐，派生密钥，常量时间比对校验摘要。
pub fn unlock(conn: &Connection, master_password: &str) -> Result<DecryptableKey, UnlockError> {
    let cfg = db::get_config(conn).map_err(UnlockError::Db)?;
    let salt = crate::crypto::hex_decode(&cfg.salt).map_err(|e| UnlockError::Db(e))?;
    let key = crate::crypto::derive_aes_key(master_password.as_bytes(), &salt)
        .map_err(|e| UnlockError::Db(e))?;
    let checksum = key_checksum(&key).map_err(|e| UnlockError::Db(e))?;
    if !crate::crypto::constant_time_eq(&checksum, &cfg.main_pwd_hash) {
        let _ = Zeroizing::new(key); // 主动擦除后返回错误
        return Err(UnlockError::WrongPassword);
    }
    Ok(DecryptableKey(key))
}

/// 读取系统设置（已解锁后调用）。
pub fn get_settings(conn: &Connection) -> Result<(i64, i64, i64), String> {
    let cfg = db::get_config(conn)?;
    Ok((cfg.lock_seconds, cfg.theme, cfg.require_login))
}

/// 保存设置。
pub fn save_settings(
    conn: &Connection,
    lock_seconds: i64,
    theme: i64,
    require_login: i64,
) -> Result<(), String> {
    if !(30..=3600).contains(&lock_seconds) {
        return Err("自动锁屏时间需在 30~3600 秒之间".into());
    }
    if !matches!(theme, 0 | 1) {
        return Err("主题设置无效".into());
    }
    if !matches!(require_login, 0 | 1) {
        return Err("启动登录选项无效".into());
    }
    db::update_settings(conn, lock_seconds, theme, require_login)
}

/// 新增记录。
pub fn add_record(
    conn: &Connection,
    key: &DecryptableKey,
    mut rec: PlainRecord,
) -> Result<i64, String> {
    validate_record(&rec)?;
    let now = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    rec.create_time = now.clone();
    rec.update_time = now;
    let row = encrypt_record(key, &rec)?;
    db::insert_record(conn, &row)
}

/// 更新记录。
pub fn update_record(
    conn: &Connection,
    key: &DecryptableKey,
    mut rec: PlainRecord,
) -> Result<(), String> {
    validate_record(&rec)?;
    rec.update_time = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let row = encrypt_record(key, &rec)?;
    db::update_record(conn, &row)
}

/// 删除记录。
pub fn delete_record(conn: &Connection, id: i64) -> Result<(), String> {
    db::delete_record(conn, id)
}

/// 列出全部明文记录（解密内存短暂存在）。
pub fn list_records(conn: &Connection, key: &DecryptableKey) -> Result<Vec<PlainRecord>, String> {
    let rows = db::list_records(conn)?;
    rows.iter().map(|r| decrypt_record(key, r)).collect()
}

/// 清空全部数据（含回收站）。
#[allow(dead_code)]
pub fn clear_all(conn: &Connection) -> Result<(), String> {
    db::clear_records(conn)
}

// ===========================================================================
// 回收站（软删除 + 恢复 + 彻底删除）
// ===========================================================================

/// 列出回收站（已软删除）的明文记录。
pub fn list_deleted(conn: &Connection, key: &DecryptableKey) -> Result<Vec<PlainRecord>, String> {
    let rows = db::list_deleted_records(conn)?;
    rows.iter().map(|r| decrypt_record(key, r)).collect()
}

/// 恢复回收站里的记录。
pub fn restore(conn: &Connection, id: i64) -> Result<(), String> {
    db::restore_record(conn, id)
}

/// 彻底删除（回收站里）。
pub fn hard_delete(conn: &Connection, id: i64) -> Result<(), String> {
    db::hard_delete_record(conn, id)
}

// ===========================================================================
// CSV 导入（Chrome / Edge / Firefox 密码导出格式）
// ===========================================================================

/// 从 CSV 文本导入记录。
/// CSV 列序兼容 Chrome / Edge / Firefox：name,url,username,password,note
pub fn import_csv(
    conn: &mut Connection,
    key: &DecryptableKey,
    csv_text: &str,
    target_category: &str,
) -> Result<usize, String> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(csv_text.as_bytes());

    let tx = conn.transaction().map_err(|e| format!("开启事务失败: {e}"))?;
    let mut count = 0usize;
    for result in rdr.records() {
        let record = result.map_err(|e| format!("CSV 解析失败: {e}"))?;
        let name = record.get(0).unwrap_or("").to_string();
        let url = record.get(1).unwrap_or("").to_string();
        let account = record.get(2).unwrap_or("").to_string();
        let password = record.get(3).unwrap_or("").to_string();
        let remark = record.get(4).unwrap_or("").to_string();

        if name.is_empty() && account.is_empty() && password.is_empty() {
            continue;
        }

        let now = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let rec = PlainRecord {
            id: None,
            category: target_category.to_string(),
            name,
            account,
            password,
            url,
            phone: String::new(),
            remark,
            create_time: now.clone(),
            update_time: now,
        };
        let row = encrypt_record(key, &rec)?;
        db::insert_record(&tx, &row)?;
        count += 1;
    }
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(count)
}

// ===========================================================================
// 弱口令 / 重复口令扫描
// ===========================================================================

/// 单条记录的安全审计结果。
#[derive(Debug, Clone)]
pub struct AuditIssue {
    pub record_id: i64,
    pub record_name: String,
    pub kind: AuditKind,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditKind {
    Weak,       // 弱口令
    Reused,     // 与其他记录重复
    Empty,      // 密码为空
    TooShort,   // 太短
}

pub fn audit_passwords(records: &[PlainRecord]) -> Vec<AuditIssue> {
    let mut issues = Vec::new();
    let mut seen: std::collections::HashMap<String, Vec<i64>> = std::collections::HashMap::new();

    const WEAK_PASSWORDS: &[&str] = &[
        "123456", "12345678", "123456789", "password", "12345",
        "qwerty", "abc123", "111111", "000000", "666666",
        "888888", "987654321", "123123", "admin", "root",
        "iloveyou", "monkey", "dragon", "master", "hello",
        "freedom", "passw0rd", "letmein", "welcome",
    ];

    for rec in records {
        let pwd = &rec.password;
        let id = rec.id.unwrap_or(0);
        if pwd.is_empty() {
            issues.push(AuditIssue {
                record_id: id,
                record_name: rec.name.clone(),
                kind: AuditKind::Empty,
                detail: "密码为空".into(),
            });
            continue;
        }
        if pwd.len() < 8 {
            issues.push(AuditIssue {
                record_id: id,
                record_name: rec.name.clone(),
                kind: AuditKind::TooShort,
                detail: format!("仅 {} 位，建议至少 8 位", pwd.len()),
            });
        }
        let lower = pwd.to_lowercase();
        if WEAK_PASSWORDS.contains(&lower.as_str()) {
            issues.push(AuditIssue {
                record_id: id,
                record_name: rec.name.clone(),
                kind: AuditKind::Weak,
                detail: format!("常见弱口令「{pwd}」，极易被猜到"),
            });
        }
        // 记录密码（去首尾空白）用于重复检测
        let key = pwd.trim().to_string();
        seen.entry(key).or_default().push(id);
    }

    // 重复检测：同一密码出现 >= 2 次
    for (_pwd, ids) in seen {
        if ids.len() >= 2 {
            for &id in &ids {
                let name = records
                    .iter()
                    .find(|r| r.id == Some(id))
                    .map(|r| r.name.clone())
                    .unwrap_or_else(|| id.to_string());
                issues.push(AuditIssue {
                    record_id: id,
                    record_name: name,
                    kind: AuditKind::Reused,
                    detail: format!("密码与 {} 条记录重复", ids.len() - 1),
                });
            }
        }
    }

    issues
}

/// 修改主密码：用旧密钥解密全部记录 -> 生成新盐新密钥 -> 重加密写回。
/// 全部写操作包在事务中：中途失败自动回滚，不会造成数据丢失。
pub fn change_master_password(
    conn: &mut Connection,
    old_master: &str,
    new_master: &str,
) -> Result<(), String> {
    let old_key = match unlock(conn, old_master) {
        Ok(k) => k,
        Err(UnlockError::WrongPassword) => return Err("旧主密码错误".into()),
        Err(UnlockError::Db(e)) => return Err(e),
    };
    if new_master.len() < 4 {
        return Err("新主密码至少 4 位".into());
    }
    // 解密全部记录（在改盐之前用旧密钥完成）
    let plain_records = list_records(conn, &old_key)?;
    // 生成新盐与新密钥
    let new_salt = random_bytes(16);
    let new_key = crate::crypto::derive_aes_key(new_master.as_bytes(), &new_salt)?;
    let new_checksum = key_checksum(&new_key)?;
    let new_key = DecryptableKey(new_key);
    // 事务：重写配置（盐 + 校验摘要）与全部记录，任一失败整体回滚
    let tx = conn
        .transaction()
        .map_err(|e| format!("开启事务失败: {e}"))?;
    db::update_main_pwd_hash(&tx, &new_checksum)?;
    db::update_salt(&tx, &crate::crypto::hex_encode(&new_salt))?;
    db::clear_records(&tx)?;
    for rec in &plain_records {
        let row = encrypt_record(&new_key, rec)?;
        db::insert_record(&tx, &row)?;
    }
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    // 派生密钥随作用域结束释放；各次加密已用 DecryptableKey 包裹并逐次清零
    Ok(())
}

/// 恢复备份：用当前密钥重加密记录并写回。
/// 覆盖模式（merge=false）与全部插入在同一事务中完成，失败自动回滚，保留原数据。
pub fn restore_records(
    conn: &mut Connection,
    key: &DecryptableKey,
    records: &[PlainRecord],
    merge: bool,
) -> Result<(), String> {
    for rec in records {
        validate_record(rec)?;
    }
    let tx = conn
        .transaction()
        .map_err(|e| format!("开启事务失败: {e}"))?;
    if !merge {
        db::clear_records(&tx)?;
    }
    for rec in records {
        let row = encrypt_record(key, rec)?;
        db::insert_record(&tx, &row)?;
    }
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(())
}

/// 收集分类：默认分类 + 已有记录的明文分类去重。
pub fn collect_categories(conn: &Connection, key: &DecryptableKey) -> Result<Vec<String>, String> {
    let mut cats: Vec<String> = db::DEFAULT_CATEGORIES
        .iter()
        .map(|s| s.to_string())
        .collect();
    for rec in list_records(conn, key)? {
        let c = rec.category.trim().to_string();
        if !c.is_empty() && !cats.contains(&c) {
            cats.push(c);
        }
    }
    Ok(cats)
}

/// 将明文记录加密为 EncRow（逐字段 AES-GCM + random nonce）。
fn encrypt_record(key: &DecryptableKey, rec: &PlainRecord) -> Result<EncRow, String> {
    Ok(EncRow {
        id: rec.id.unwrap_or(0),
        category: crate::crypto::encrypt_str(key.0.as_ref(), &rec.category)?,
        name: crate::crypto::encrypt_str(key.0.as_ref(), &rec.name)?,
        account: crate::crypto::encrypt_str(key.0.as_ref(), &rec.account)?,
        password: crate::crypto::encrypt_str(key.0.as_ref(), &rec.password)?,
        url: crate::crypto::encrypt_str(key.0.as_ref(), &rec.url)?,
        phone: crate::crypto::encrypt_str(key.0.as_ref(), &rec.phone)?,
        remark: crate::crypto::encrypt_str(key.0.as_ref(), &rec.remark)?,
        create_time: crate::crypto::encrypt_str(key.0.as_ref(), &rec.create_time)?,
        update_time: crate::crypto::encrypt_str(key.0.as_ref(), &rec.update_time)?,
    })
}

fn validate_record(rec: &PlainRecord) -> Result<(), String> {
    if rec.name.trim().is_empty() || rec.password.trim().is_empty() {
        return Err("名称和密码不能为空".into());
    }
    Ok(())
}

/// 将 EncRow 解密为明文记录。
fn decrypt_record(key: &DecryptableKey, row: &EncRow) -> Result<PlainRecord, String> {
    Ok(PlainRecord {
        id: Some(row.id),
        category: crate::crypto::decrypt_str(key.0.as_ref(), &row.category)?,
        name: crate::crypto::decrypt_str(key.0.as_ref(), &row.name)?,
        account: crate::crypto::decrypt_str(key.0.as_ref(), &row.account)?,
        password: crate::crypto::decrypt_str(key.0.as_ref(), &row.password)?,
        url: crate::crypto::decrypt_str(key.0.as_ref(), &row.url)?,
        phone: crate::crypto::decrypt_str(key.0.as_ref(), &row.phone)?,
        remark: crate::crypto::decrypt_str(key.0.as_ref(), &row.remark)?,
        create_time: crate::crypto::decrypt_str(key.0.as_ref(), &row.create_time)?,
        update_time: crate::crypto::decrypt_str(key.0.as_ref(), &row.update_time)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_family = "windows")]
    fn temp_db(name: &str) -> (Connection, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("passwordbook_test_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db");
        let conn = db::open(&path).unwrap();
        (conn, path)
    }

    #[test]
    fn full_service_flow() {
        let (mut conn, path) = temp_db("flow");
        // 首次设置 + 解锁
        let key = setup(&conn, "master-pass", 300).unwrap();
        assert!(db::has_config(&conn).unwrap());

        // 新增
        let rec = PlainRecord {
            id: None,
            category: "社交".into(),
            name: "Gmail".into(),
            account: "user@mail.com".into(),
            password: "secret123".into(),
            phone: "13800000000".into(),
            remark: "主邮箱".into(),
            ..Default::default()
        };
        let id = add_record(&conn, &key, rec).unwrap();
        let list = list_records(&conn, &key).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Gmail");
        assert_eq!(list[0].password, "secret123");
        assert!(list[0].create_time.len() >= 10);

        // 更新
        let mut upd = list[0].clone();
        upd.name = "Gmail2".into();
        upd.id = Some(id);
        update_record(&conn, &key, upd).unwrap();
        assert_eq!(list_records(&conn, &key).unwrap()[0].name, "Gmail2");

        // 数据库里的字段是密文，不是明文
        let rows = db::list_records(&conn).unwrap();
        assert_ne!(rows[0].password, "secret123");

        // 修改主密码
        change_master_password(&mut conn, "master-pass", "new-master-789").unwrap();
        assert!(match unlock(&conn, "master-pass") {
            Err(UnlockError::WrongPassword) => true,
            _ => false,
        });
        let new_key = unlock(&conn, "new-master-789").unwrap();
        let list = list_records(&conn, &new_key).unwrap();
        assert_eq!(list[0].password, "secret123", "改密后数据不丢失且可解密");

        // 分类收集
        let cats = collect_categories(&conn, &new_key).unwrap();
        assert!(cats.contains(&"社交".to_string()));

        // 清空
        clear_all(&conn).unwrap();
        assert!(list_records(&conn, &new_key).unwrap().is_empty());

        drop(conn);
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn wrong_password_rejected_at_unlock() {
        let (conn, path) = temp_db("wrongpwd");
        setup(&conn, "correct", 300).unwrap();
        assert!(match unlock(&conn, "incorrect") {
            Err(UnlockError::WrongPassword) => true,
            _ => false,
        });
        assert!(unlock(&conn, "correct").is_ok());
        drop(conn);
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn setup_requires_config() {
        let (conn, path) = temp_db("setupdup");
        setup(&conn, "p", 300).unwrap();
        assert!(setup(&conn, "q", 300).is_err(), "重复初始化应失败");
        drop(conn);
        let _ = std::fs::remove_dir_all(&path);
    }
}

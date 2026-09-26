//! 持久化层：SQLite 数据库，表结构与设计文档第四章完全一致。
//! rusqlite 使用参数化绑定，杜绝 SQL 注入。

use crate::model::{EncRow, SysConfig};
use rusqlite::{params, Connection};
use std::path::Path;

/// 内置默认分类（设计文档 3.3）。
pub const DEFAULT_CATEGORIES: [&str; 7] =
    ["社交", "办公", "游戏", "购物", "银行卡", "系统", "其他"];

/// 打开（必要时创建）数据库并初始化表结构。
pub fn open(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| format!("打开数据库失败: {e}"))?;
    // 先用普通 DELETE journal（稳定优先，避免 WAL 损坏），后续验证没问题再切 WAL
    conn.execute_batch("PRAGMA journal_mode=DELETE;")
        .map_err(|e| format!("设置 journal 失败: {e}"))?;
    create_schema(&conn)?;
    Ok(conn)
}

/// 建表：sys_config 与 pwd_data。
pub fn create_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS sys_config (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            salt              TEXT NOT NULL,
            main_pwd_hash     TEXT NOT NULL,
            lock_seconds      INTEGER NOT NULL DEFAULT 300,
            theme             INTEGER NOT NULL DEFAULT 0,
            require_login     INTEGER NOT NULL DEFAULT 1
        );
        CREATE TABLE IF NOT EXISTS pwd_data (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            category    TEXT NOT NULL,
            name        TEXT NOT NULL,
            account     TEXT NOT NULL,
            password    TEXT NOT NULL,
            url         TEXT NOT NULL DEFAULT '',
            phone       TEXT NOT NULL,
            remark      TEXT NOT NULL,
            create_time TEXT NOT NULL,
            update_time TEXT NOT NULL,
            deleted_at  TEXT NOT NULL DEFAULT ''
        );
        "#,
    )
    .map_err(|e| format!("建表失败: {e}"))?;
    // 旧库兼容迁移
    let _ = conn.execute_batch("ALTER TABLE pwd_data ADD COLUMN url TEXT NOT NULL DEFAULT '';");
    let _ = conn.execute_batch("ALTER TABLE pwd_data ADD COLUMN deleted_at TEXT NOT NULL DEFAULT '';");
    let _ = conn.execute_batch("ALTER TABLE sys_config ADD COLUMN require_login INTEGER NOT NULL DEFAULT 1;");
    Ok(())
}

/// 是否已经完成首次初始化（存在 sys_config 行）。
pub fn has_config(conn: &Connection) -> Result<bool, String> {
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM sys_config", [], |r| r.get(0))
        .map_err(|e| format!("查询配置失败: {e}"))?;
    Ok(n > 0)
}

/// 检查 sys_config 是否包含指定列（用于旧数据库兼容迁移）。
fn has_column(conn: &Connection, table: &str, col: &str) -> bool {
    let names: Vec<String> = {
        let mut stmt = match conn.prepare(&format!("PRAGMA table_info({table})")) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let rows = match stmt.query_map([], |r| r.get::<_, String>(1)) {
            Ok(rows) => rows,
            Err(_) => return false,
        };
        rows.filter_map(|r| r.ok()).collect()
    };
    names.iter().any(|name| name == col)
}

/// 读取系统配置（假定已初始化）。
pub fn get_config(conn: &Connection) -> Result<SysConfig, String> {
    let has_require = has_column(conn, "sys_config", "require_login");
    let sql = if has_require {
        "SELECT salt, main_pwd_hash, lock_seconds, theme, require_login FROM sys_config WHERE id = 1"
    } else {
        "SELECT salt, main_pwd_hash, lock_seconds, theme FROM sys_config WHERE id = 1"
    };
    conn.query_row(sql, [], |r| {
        if has_require {
            Ok(SysConfig {
                salt: r.get(0)?,
                main_pwd_hash: r.get(1)?,
                lock_seconds: r.get(2)?,
                theme: r.get(3)?,
                require_login: r.get(4)?,
            })
        } else {
            Ok(SysConfig {
                salt: r.get(0)?,
                main_pwd_hash: r.get(1)?,
                lock_seconds: r.get(2)?,
                theme: r.get(3)?,
                require_login: 1, // 旧库默认需要登录
            })
        }
    })
    .map_err(|e| format!("读取配置失败: {e}"))
}

/// 首次初始化时写入配置（id=1）。
pub fn init_config(
    conn: &Connection,
    salt: &str,
    main_pwd_hash: &str,
    lock_seconds: i64,
    theme: i64,
    require_login: i64,
) -> Result<(), String> {
    let has_clip = has_column(conn, "sys_config", "clip_clear_seconds");
    if has_clip {
        conn.execute(
            "INSERT INTO sys_config (id, salt, main_pwd_hash, lock_seconds, clip_clear_seconds, theme, require_login) VALUES (1, ?1, ?2, ?3, 0, ?4, ?5)",
            params![salt, main_pwd_hash, lock_seconds, theme, require_login],
        )
    } else {
        conn.execute(
            "INSERT INTO sys_config (id, salt, main_pwd_hash, lock_seconds, theme, require_login) VALUES (1, ?1, ?2, ?3, ?4, ?5)",
            params![salt, main_pwd_hash, lock_seconds, theme, require_login],
        )
    }
    .map_err(|e| format!("初始化配置失败: {e}"))?;
    Ok(())
}

/// 更新设置（锁屏秒数、主题、启动是否需要登录）。
pub fn update_settings(
    conn: &Connection,
    lock_seconds: i64,
    theme: i64,
    require_login: i64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE sys_config SET lock_seconds = ?1, theme = ?2, require_login = ?3 WHERE id = 1",
        params![lock_seconds, theme, require_login],
    )
    .map_err(|e| format!("更新设置失败: {e}"))?;
    Ok(())
}

/// 修改主密码：更新校验摘要。
pub fn update_main_pwd_hash(conn: &Connection, main_pwd_hash: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE sys_config SET main_pwd_hash = ?1 WHERE id = 1",
        params![main_pwd_hash],
    )
    .map_err(|e| format!("更新主密码失败: {e}"))?;
    Ok(())
}

/// 修改主密码：更新盐（新盐 → 新密钥）。
pub fn update_salt(conn: &Connection, salt: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE sys_config SET salt = ?1 WHERE id = 1",
        params![salt],
    )
    .map_err(|e| format!("更新盐失败: {e}"))?;
    Ok(())
}

/// 新增一条密码记录。
pub fn insert_record(conn: &Connection, row: &EncRow) -> Result<i64, String> {
    let has_url = has_column(conn, "pwd_data", "url");
    if has_url {
        conn.execute(
            "INSERT INTO pwd_data (category, name, account, password, url, phone, remark, create_time, update_time) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![row.category, row.name, row.account, row.password, row.url, row.phone, row.remark, row.create_time, row.update_time],
        )
    } else {
        conn.execute(
            "INSERT INTO pwd_data (category, name, account, password, phone, remark, create_time, update_time) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![row.category, row.name, row.account, row.password, row.phone, row.remark, row.create_time, row.update_time],
        )
    }
    .map_err(|e| format!("新增记录失败: {e}"))?;
    Ok(conn.last_insert_rowid())
}

/// 更新一条密码记录。
pub fn update_record(conn: &Connection, row: &EncRow) -> Result<(), String> {
    let has_url = has_column(conn, "pwd_data", "url");
    let changed = if has_url {
        conn.execute(
            "UPDATE pwd_data SET category=?1, name=?2, account=?3, password=?4, url=?5, phone=?6, remark=?7, create_time=?8, update_time=?9 WHERE id=?10",
            params![row.category, row.name, row.account, row.password, row.url, row.phone, row.remark, row.create_time, row.update_time, row.id],
        )
    } else {
        conn.execute(
            "UPDATE pwd_data SET category=?1, name=?2, account=?3, password=?4, phone=?5, remark=?6, create_time=?7, update_time=?8 WHERE id=?9",
            params![row.category, row.name, row.account, row.password, row.phone, row.remark, row.create_time, row.update_time, row.id],
        )
    }
    .map_err(|e| format!("更新记录失败: {e}"))?;
    if changed == 0 {
        return Err("更新记录失败：记录不存在或已被删除".into());
    }
    Ok(())
}

/// 软删除：把 deleted_at 设为当前时间，记录不被物理删除。
pub fn soft_delete_record(conn: &Connection, id: i64, now: &str) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE pwd_data SET deleted_at = ?1 WHERE id = ?2 AND deleted_at = ''",
            params![now, id],
        )
        .map_err(|e| format!("软删除失败: {e}"))?;
    if changed == 0 {
        return Err("记录不存在或已被删除".into());
    }
    Ok(())
}

/// 恢复软删除的记录。
pub fn restore_record(conn: &Connection, id: i64) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE pwd_data SET deleted_at = '' WHERE id = ?1 AND deleted_at != ''",
            params![id],
        )
        .map_err(|e| format!("恢复失败: {e}"))?;
    if changed == 0 {
        return Err("恢复失败：记录不存在或未被删除".into());
    }
    Ok(())
}

/// 彻底删除（回收站里）。
pub fn hard_delete_record(conn: &Connection, id: i64) -> Result<(), String> {
    let changed = conn
        .execute("DELETE FROM pwd_data WHERE id = ?1 AND deleted_at != ''", params![id])
        .map_err(|e| format!("彻底删除失败: {e}"))?;
    if changed == 0 {
        return Err("彻底删除失败：记录不存在".into());
    }
    Ok(())
}

/// 普通删除（兼容调用方，转为软删除）。
pub fn delete_record(conn: &Connection, id: i64) -> Result<(), String> {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    soft_delete_record(conn, id, &now)
}

/// 列出**未删除**的全部记录（加密 blob）。
pub fn list_records(conn: &Connection) -> Result<Vec<EncRow>, String> {
    let has_url = has_column(conn, "pwd_data", "url");
    let sql = if has_url {
        "SELECT id, category, name, account, password, url, phone, remark, create_time, update_time FROM pwd_data WHERE deleted_at = '' ORDER BY id DESC"
    } else {
        "SELECT id, category, name, account, password, phone, remark, create_time, update_time FROM pwd_data WHERE deleted_at = '' ORDER BY id DESC"
    };
    build_list(conn, sql, has_url)
}

/// 列出**已软删除**的记录（回收站）。
pub fn list_deleted_records(conn: &Connection) -> Result<Vec<EncRow>, String> {
    let has_url = has_column(conn, "pwd_data", "url");
    let sql = if has_url {
        "SELECT id, category, name, account, password, url, phone, remark, create_time, update_time FROM pwd_data WHERE deleted_at != '' ORDER BY deleted_at DESC"
    } else {
        "SELECT id, category, name, account, password, phone, remark, create_time, update_time FROM pwd_data WHERE deleted_at != '' ORDER BY id DESC"
    };
    build_list(conn, sql, has_url)
}

fn build_list(conn: &Connection, sql: &str, has_url: bool) -> Result<Vec<EncRow>, String> {
    let mut stmt = conn.prepare(sql).map_err(|e| format!("查询记录失败: {e}"))?;
    let rows = stmt.query_map([], |r| {
        if has_url {
            Ok(EncRow {
                id: r.get(0)?, category: r.get(1)?, name: r.get(2)?,
                account: r.get(3)?, password: r.get(4)?, url: r.get(5)?,
                phone: r.get(6)?, remark: r.get(7)?, create_time: r.get(8)?, update_time: r.get(9)?,
            })
        } else {
            Ok(EncRow {
                id: r.get(0)?, category: r.get(1)?, name: r.get(2)?,
                account: r.get(3)?, password: r.get(4)?, url: String::new(),
                phone: r.get(5)?, remark: r.get(6)?, create_time: r.get(7)?, update_time: r.get(8)?,
            })
        }
    })
    .map_err(|e| format!("查询记录失败: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("解析记录失败: {e}"))?);
    }
    Ok(out)
}

/// 按 ID 读取单条记录（加密 blob）。
pub fn get_record(conn: &Connection, id: i64) -> Result<EncRow, String> {
    let has_url = has_column(conn, "pwd_data", "url");
    let sql = if has_url {
        "SELECT id, category, name, account, password, url, phone, remark, create_time, update_time FROM pwd_data WHERE id = ?1"
    } else {
        "SELECT id, category, name, account, password, phone, remark, create_time, update_time FROM pwd_data WHERE id = ?1"
    };
    conn.query_row(sql, params![id], |r| {
        if has_url {
            Ok(EncRow {
                id: r.get(0)?, category: r.get(1)?, name: r.get(2)?,
                account: r.get(3)?, password: r.get(4)?, url: r.get(5)?,
                phone: r.get(6)?, remark: r.get(7)?, create_time: r.get(8)?, update_time: r.get(9)?,
            })
        } else {
            Ok(EncRow {
                id: r.get(0)?, category: r.get(1)?, name: r.get(2)?,
                account: r.get(3)?, password: r.get(4)?, url: String::new(),
                phone: r.get(5)?, remark: r.get(6)?, create_time: r.get(7)?, update_time: r.get(8)?,
            })
        }
    })
    .map_err(|e| format!("读取记录失败: {e}"))
}

/// 一键清空**所有**记录（含回收站）。
pub fn clear_records(conn: &Connection) -> Result<(), String> {
    conn.execute("DELETE FROM pwd_data", [])
        .map_err(|e| format!("清空数据失败: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn schema_and_config_init() {
        let conn = test_conn();
        assert!(!has_config(&conn).unwrap());
        init_config(&conn, "salt_hex", "hash_hex", 300, 30, 1).unwrap();
        assert!(has_config(&conn).unwrap());
        let cfg = get_config(&conn).unwrap();
        assert_eq!(cfg.salt, "salt_hex");
        assert_eq!(cfg.lock_seconds, 300);
        update_settings(&conn, 600, 0, 0).unwrap();
        let cfg = get_config(&conn).unwrap();
        assert_eq!(cfg.lock_seconds, 600);
        assert_eq!(cfg.theme, 0);
    }

    #[test]
    fn record_crud() {
        let conn = test_conn();
        let mut row = EncRow {
            id: 0,
            category: "c-enc".into(),
            name: "n-enc".into(),
            account: "a-enc".into(),
            password: "p-enc".into(),
            url: "u-enc".into(),
            phone: "ph".into(),
            remark: "rm".into(),
            create_time: "t1".into(),
            update_time: "t1".into(),
        };
        let id = insert_record(&conn, &row).unwrap();
        assert_eq!(id, 1);
        let rows = list_records(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "n-enc");

        row.id = id;
        row.name = "n-enc2".into();
        row.update_time = "t2".into();
        update_record(&conn, &row).unwrap();
        let got = get_record(&conn, id).unwrap();
        assert_eq!(got.name, "n-enc2");
        assert_eq!(got.update_time, "t2");

        delete_record(&conn, id).unwrap();
        assert!(list_records(&conn).unwrap().is_empty());
        assert!(delete_record(&conn, id).is_err());
    }

    #[test]
    fn update_missing_record_is_rejected() {
        let conn = test_conn();
        let row = EncRow {
            id: 999,
            name: "missing".into(),
            ..Default::default()
        };
        assert!(update_record(&conn, &row).is_err());
    }

    #[test]
    fn clear_records_works() {
        let conn = test_conn();
        let row = EncRow {
            id: 0,
            name: "x".into(),
            ..Default::default()
        };
        insert_record(&conn, &row).unwrap();
        clear_records(&conn).unwrap();
        assert!(list_records(&conn).unwrap().is_empty());
    }
}

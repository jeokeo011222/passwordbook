//! 数据模型：明文记录与数据库中加密行（blob）的映射。

use serde::{Deserialize, Serialize};

/// 明文密码记录（GUI/业务层使用，仅在内存中短暂存在，用完即弃）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlainRecord {
    pub id: Option<i64>,
    pub category: String,
    pub name: String,
    pub account: String,
    pub password: String,
    pub url: String,
    pub phone: String,
    pub remark: String,
    pub create_time: String,
    pub update_time: String,
}

/// 数据库中存储的加密行。
#[derive(Debug, Clone, Default)]
pub struct EncRow {
    pub id: i64,
    pub category: String,
    pub name: String,
    pub account: String,
    pub password: String,
    pub url: String,
    pub phone: String,
    pub remark: String,
    pub create_time: String,
    pub update_time: String,
}

/// 系统配置（sys_config 单行）。
#[derive(Debug, Clone)]
pub struct SysConfig {
    pub salt: String,
    pub main_pwd_hash: String,
    pub lock_seconds: i64,
    pub theme: i64,
    pub require_login: i64,
}

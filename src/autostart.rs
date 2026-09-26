//! 开机自启：通过 Windows 注册表 Run 键实现（设计文档 3.6）。
//! 使用 winreg，仅 Windows 编译。

#[cfg(target_family = "windows")]
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
#[cfg(target_family = "windows")]
use winreg::RegKey;

/// exe 工作目录 + 参数：`"exe_path" --autostart`。
#[cfg(target_family = "windows")]
fn run_value() -> String {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "passwordbook.exe".into());
    format!("\"{}\" --autostart", exe)
}

/// 设置或取消开机自启。
#[cfg(target_family = "windows")]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .map_err(|e| format!("打开注册表失败: {e}"))?;
    let name = "PasswordBook";
    if enabled {
        key.set_value(name, &run_value())
            .map_err(|e| format!("写入注册表失败: {e}"))
    } else {
        match key.delete_value(name) {
            Ok(_) => Ok(()),
            Err(_) => Ok(()), // 已不存在视为成功
        }
    }
}

/// 查询当前自启状态。
#[cfg(target_family = "windows")]
pub fn get_autostart() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = match hkcu.open_subkey_with_flags(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Run",
        KEY_READ,
    ) {
        Ok(k) => k,
        Err(_) => return false,
    };
    !key.get_value::<String, _>("PasswordBook").is_err()
}

#[cfg(not(target_family = "windows"))]
pub fn set_autostart(_enabled: bool) -> Result<(), String> {
    Err("当前平台不支持开机自启".into())
}

#[cfg(not(target_family = "windows"))]
pub fn get_autostart() -> bool {
    false
}

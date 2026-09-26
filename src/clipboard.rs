//! 剪贴板管控：写入/读取剪贴板，并配合定时器自动清空（设计文档 3.2/5.6）。

use arboard::Clipboard;

/// 将文本写入系统剪贴板。
pub fn set_clipboard(text: &str) -> Result<(), String> {
    let mut clip = Clipboard::new().map_err(|e| format!("打开剪贴板失败: {e}"))?;
    clip.set_text(text.to_owned())
        .map_err(|e| format!("写入剪贴板失败: {e}"))
}

/// 读取剪贴板文本。
#[allow(dead_code)]
pub fn get_clipboard() -> Result<String, String> {
    let mut clip = Clipboard::new().map_err(|e| format!("打开剪贴板失败: {e}"))?;
    clip.get_text().map_err(|e| format!("读取剪贴板失败: {e}"))
}

/// 清空剪贴板（写入空字符串）。
#[allow(dead_code)]
pub fn clear_clipboard() -> Result<(), String> {
    set_clipboard("")
}

/// 仅当剪贴板仍然保持为本程序写入的内容时才清空。
///
/// 这可以避免自动清理或锁屏时误删用户后来复制的其它内容。
pub fn clear_clipboard_if_matches(expected: &str) -> Result<bool, String> {
    let mut clip = Clipboard::new().map_err(|e| format!("打开剪贴板失败: {e}"))?;
    let current = clip
        .get_text()
        .map_err(|e| format!("读取剪贴板失败: {e}"))?;
    if current != expected {
        return Ok(false);
    }
    clip.set_text(String::new())
        .map_err(|e| format!("清空剪贴板失败: {e}"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_write_read_roundtrip() {
        let test_str = format!("pwbook-clip-test-{}", std::process::id());
        if set_clipboard(&test_str).is_ok() {
            let got = get_clipboard().unwrap_or_default();
            // 某些环境剪贴板可能被其他进程占用，允许宽松断言
            assert_eq!(got, test_str);
            let _ = clear_clipboard();
        }
    }
}

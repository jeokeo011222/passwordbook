//! Windows 离线本地密码本 —— 程序入口。
//! 开发环境：Rust stable + egui(GUI) + rusqlite(SQLite) + orion(AES-256-GCM/PBKDF2)。

// 以 GUI 子系统运行：release 构建不再弹出 cmd 控制台黑窗
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod backup;
mod clipboard;
mod crypto;
mod db;
mod generator;
mod model;
mod service;
mod tray;

use eframe::egui;
use std::sync::Arc;

/// 为 egui 加载系统中文字体，解决中文显示为乱码/方块的问题。
/// 将 CJK 字体追加为默认字体（Ubuntu-Light 等）的回退字体：
/// 拉丁字符仍用默认字体，中文字形自动回退到系统字体。
fn setup_fonts(ctx: &egui::Context) {
    // Windows 系统字体候选（按优先级尝试，找到可读的即加载）
    const FONT_CANDIDATES: &[(&str, &str)] = &[
        ("msyh", r"C:\Windows\Fonts\msyh.ttc"),         // 微软雅黑
        ("simhei", r"C:\Windows\Fonts\simhei.ttf"),     // 黑体
        ("simsun", r"C:\Windows\Fonts\simsun.ttc"),     // 宋体
        ("deng", r"C:\Windows\Fonts\deng.ttf"),         // 等线
        ("seguisym", r"C:\Windows\Fonts\seguisym.ttf"), // Segoe UI Symbol（覆盖⚙🔒ⓘ等）
        ("seguiemj", r"C:\Windows\Fonts\seguiemj.ttf"), // Segoe UI Emoji
    ];

    let mut fonts = egui::FontDefinitions::default();
    let mut loaded: Vec<String> = Vec::new();
    for (name, path) in FONT_CANDIDATES {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                name.to_string(),
                Arc::new(egui::FontData::from_owned(bytes)),
            );
            loaded.push(name.to_string());
        }
    }
    if loaded.is_empty() {
        eprintln!("警告: 未找到系统中文字体，界面中文可能显示为乱码");
        return;
    }
    // 追加到 Proportional 与 Monospace 两个字族末尾作为回退
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        if let Some(list) = fonts.families.get_mut(&family) {
            list.extend(loaded.iter().cloned());
        }
    }
    ctx.set_fonts(fonts);
}

/// 自启验证模式：命令行直接调用 autostart 模块，不启动 GUI。
/// 用于验证注册表 Run 项写入/读取/删除（不干扰正常使用）。
fn run_autostart_self_test() {
    use crate::autostart;
    println!("[autostart-self-test]");
    println!("before -> enabled={}", autostart::get_autostart());
    match autostart::set_autostart(true) {
        Ok(()) => println!("set(true) -> ok"),
        Err(e) => println!("set(true) -> error: {e}"),
    }
    println!("after-set -> enabled={}", autostart::get_autostart());
    match autostart::set_autostart(false) {
        Ok(()) => println!("set(false) -> ok"),
        Err(e) => println!("set(false) -> error: {e}"),
    }
    println!("after-unset -> enabled={}", autostart::get_autostart());
}

fn main() -> eframe::Result<()> {
    // `--test-autostart`：自启注册表验证（不弹 GUI）
    if std::env::args().any(|a| a == "--test-autostart") {
        run_autostart_self_test();
        std::process::exit(0);
    }
    // `--autostart` 开关：由开机自启注册表项调用，正常拉起窗口即可。
    // 托盘图标在独立线程创建（tray.rs::spawn_tray），让 tray-icon 隐藏窗口挂在有消息泵的线程上
    let tray_state = tray::spawn_tray();

    // 计算屏幕中央位置（逻辑点）。GetSystemMetrics 返回物理像素，按系统 DPI 换算。
    let win_size = (340.0, 210.0);
    let initial_pos = {
        use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN,
        };
        let px = unsafe { GetSystemMetrics(SM_CXSCREEN) } as f32;
        let py = unsafe { GetSystemMetrics(SM_CYSCREEN) } as f32;
        let scale = unsafe { GetDpiForSystem() as f32 } / 96.0;
        egui::pos2(
            (px / scale - win_size.0) * 0.5,
            (py / scale - win_size.1) * 0.5,
        )
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("密码本")
            .with_inner_size(win_size)
            .with_min_inner_size([340.0, 200.0])
            .with_position(initial_pos)
            .with_decorations(false)
            .with_icon(Arc::new(load_window_icon())),
        ..Default::default()
    };
    eframe::run_native(
        "密码本",
        options,
        Box::new(|cc| {
            setup_fonts(&cc.egui_ctx);
            Ok(Box::new(app::PasswordBookApp::new(tray_state)))
        }),
    )
}

/// 从 logo2.png 构造 egui 窗口图标。
fn load_window_icon() -> egui::IconData {
    let (w, h, rgba) = tray::icon_rgba();
    egui::IconData {
        rgba,
        width: w,
        height: h,
    }
}

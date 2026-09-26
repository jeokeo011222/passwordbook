//! 系统托盘：常驻任务栏托盘图标 + 右键菜单。
//!
//! 架构要点：
//! 1. **托盘图标创建在独立线程** —— tray-icon 创建的隐藏窗口（类名 `tray_icon_app`）
//!    必须在创建线程上跑 PeekMessage/DispatchMessage 消息泵，否则它的 WndProc 不会被调用。
//!    eframe 主线程的 winit 消息循环只关心自己的主窗口 HWND，不处理托盘隐藏窗口的消息队列。
//! 2. **托盘事件处理不依赖 eframe update 循环** —— eframe 0.30 在主窗口 Visible(false) 后
//!    会停止调用 update()，所以我们在托盘线程里直接处理事件：
//!    - quit 事件 → 托盘线程直接 std::process::exit(0)
//!    - show 事件 → 托盘线程用 FindWindowW + ShowWindow + SetForegroundWindow 直接操作主窗口
//!      同时设 external_show_pending 标志，主线程 update 里同步 eframe 的 viewport 状态

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// 托盘与 GUI 线程共享的状态标志。
#[derive(Clone)]
pub struct TrayState {
    /// 主线程需要把 viewport visible 设置为 true（托盘线程已直接 ShowWindow）。
    pub external_show_pending: Arc<AtomicBool>,
}

impl TrayState {
    /// 空状态（托盘未启动时使用）。
    pub fn new_empty() -> Self {
        Self {
            external_show_pending: Arc::new(AtomicBool::new(false)),
        }
    }
}

fn log(msg: &str) {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    let dir = std::path::PathBuf::from(&appdata).join("passwordbook");
    let _ = std::fs::create_dir_all(&dir);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let line = format!("[{ts}] {msg}\n");
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("tray_debug.log"))
        .and_then(|mut f| f.write_all(line.as_bytes()));
}

/// 启动托盘（独立线程创建托盘图标 + 跑消息泵）。
pub fn spawn_tray() -> Option<TrayState> {
    let external_show = Arc::new(AtomicBool::new(false));
    let s_external_show = external_show.clone();

    std::thread::Builder::new()
        .name("pwbook-tray".into())
        .spawn(move || {
            log("tray thread started");

            let show_id = tray_icon::menu::MenuId::new("show");
            let quit_id = tray_icon::menu::MenuId::new("quit");

            let icon = match build_tray_icon() {
                Ok(i) => i,
                Err(e) => {
                    log(&format!("build_icon failed: {e}"));
                    return;
                }
            };

            let menu = tray_icon::menu::Menu::new();
            let _ = menu.append(&tray_icon::menu::MenuItem::with_id(
                show_id.clone(),
                "显示主程序",
                true,
                None,
            ));
            let _ = menu.append(&tray_icon::menu::PredefinedMenuItem::separator());
            let _ = menu.append(&tray_icon::menu::MenuItem::with_id(
                quit_id.clone(),
                "退出",
                true,
                None,
            ));

            let tray = match tray_icon::TrayIconBuilder::new()
                .with_tooltip("密码本 — 右键菜单")
                .with_menu(Box::new(menu))
                .with_icon(icon)
                .with_menu_on_left_click(true)
                .build()
            {
                Ok(t) => {
                    log(&format!(
                        "tray icon created OK on thread {:?}",
                        std::thread::current().id()
                    ));
                    t
                }
                Err(e) => {
                    log(&format!("TrayIconBuilder failed: {e}"));
                    return;
                }
            };

            // ---- 处理托盘事件的辅助函数 ----
            let handle_show = || {
                log("-> show_requested: FindWindow + ShowWindow + SetForegroundWindow");
                // 1. 先让主线程知道要同步 eframe 的 viewport visible 状态
                s_external_show.store(true, Ordering::Relaxed);
                // 2. 枚举找到标题为 "密码本" 的顶层窗口并显示
                unsafe {
                    use windows_sys::Win32::UI::WindowsAndMessaging::{
                        FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
                    };
                    // 构造宽字符串 "密码本"
                    let title_wide: Vec<u16> =
                        "密码本".encode_utf16().chain(std::iter::once(0)).collect();
                    let hwnd = FindWindowW(std::ptr::null(), title_wide.as_ptr());
                    if !hwnd.is_null() {
                        log(&format!("-> FindWindow found hwnd={:p}", hwnd));
                        // 先 ShowWindow SW_SHOW 如果已经隐藏，再 SW_RESTORE 确保从最小化恢复
                        ShowWindow(hwnd, SW_SHOW);
                        ShowWindow(hwnd, SW_RESTORE);
                        SetForegroundWindow(hwnd);
                    } else {
                        log("-> FindWindow returned null (窗口可能还没创建)");
                    }
                }
            };

            let handle_quit = || {
                log("-> quit_requested: process::exit(0) from tray thread");
                std::process::exit(0);
            };

            // 3. 消息泵 + 事件处理循环
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
                };
                let _ = tray;

                log("tray thread entering message loop");
                loop {
                    let mut msg: MSG = std::mem::zeroed();
                    if PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }

                    // 处理托盘点击事件（tray-icon 在 WndProc 里 PostEvent 到 channel）
                    if let Ok(ev) = tray_icon::TrayIconEvent::receiver().try_recv() {
                        log(&format!("tray-thread TrayIconEvent: {:?}", ev));
                        if let tray_icon::TrayIconEvent::Click { .. } = ev {
                            handle_show();
                        }
                    }
                    if let Ok(ev) = tray_icon::menu::MenuEvent::receiver().try_recv() {
                        log(&format!("tray-thread MenuEvent: {:?}", ev.id));
                        if ev.id == show_id {
                            log("-> MenuEvent show");
                            handle_show();
                        } else if ev.id == quit_id {
                            log("-> MenuEvent quit");
                            handle_quit();
                        }
                    }
                }
            }
        })
        .ok()?;

    Some(TrayState {
        external_show_pending: external_show,
    })
}

/// 给主线程在 update() 里调用：消费托盘线程设的标志，
/// 确保 eframe 的 viewport 状态与实际窗口状态一致。
pub fn poll_tray_events(tray: &TrayState) {
    // 托盘线程已经直接操作了 HWND，主线程只需要同步 viewport visible 状态
    // 这个函数保留是为了调用方代码不变——实际同步逻辑在 app.rs 的 update 里做
    let _ = tray;
}

// ---------------------------------------------------------------------------
// 图标：从 build.rs 预解码的 logo2.png RGBA 数据生成
// ---------------------------------------------------------------------------

/// 读取 logo2.png 的 RGBA 原始数据，返回 (width, height, rgba_bytes)。
/// 供托盘图标、关于窗口图标、窗口图标共用。
pub fn icon_rgba() -> (u32, u32, Vec<u8>) {
    static RAW: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/logo_rgba.raw"));
    let w = u32::from_be_bytes([RAW[0], RAW[1], RAW[2], RAW[3]]);
    let h = u32::from_be_bytes([RAW[4], RAW[5], RAW[6], RAW[7]]);
    (w, h, RAW[8..].to_vec())
}

fn build_tray_icon() -> Result<tray_icon::Icon, tray_icon::BadIcon> {
    let (w, h, rgba) = icon_rgba();
    tray_icon::Icon::from_rgba(rgba, w, h)
}

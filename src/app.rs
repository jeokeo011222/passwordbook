//! GUI 界面层（egui/eframe）。
//! 包含：解锁/首次设置窗口、主窗口（分类树、搜索、记录表格、右键菜单）、
//! 新增/编辑弹窗、密码生成器、备份恢复、设置、主题、闲置自动锁屏、剪贴板倒计时自动清空。

use crate::autostart;
use crate::backup;
use crate::clipboard;
use crate::crypto::DecryptableKey;
use crate::generator;
use crate::model::PlainRecord;
use crate::tray::{self, TrayState};
use crate::{db, service};
use eframe::egui;
use eframe::egui::{Color32, FontId, Frame, Margin, RichText, Rounding, Stroke};
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const APP_VERSION: &str = "1.0.0";

// ---- Outlook 风格设计常量 ----
const ACCENT: Color32 = Color32::from_rgb(0x00, 0x78, 0xd4); // 默认 Outlook 蓝
const ROUND_CARD: u8 = 8;
const ROUND_BTN: u8 = 6;

// 间距体系（Apple HIG 风格）
const SPACE_PANEL: f32 = 16.0;
const SPACE_CARD: f32 = 16.0;
const SPACE_ROW: f32 = 8.0;
const BTN_H: f32 = 32.0;
const SIDE_W: f32 = 160.0;
const POPUP_PAD_X: f32 = 20.0;
const POPUP_PAD_Y: f32 = 22.0;

fn paint_gradient_background(ctx: &egui::Context, dark: bool, _t: f32) {
    let painter = ctx.layer_painter(egui::LayerId::background());
    let rect = ctx.screen_rect();
    if rect.is_negative() {
        return;
    }
    let fill = if dark {
        Color32::from_rgb(0x1b, 0x1b, 0x1c)
    } else {
        Color32::from_rgb(0xf5, 0xf6, 0xf7)
    };
    painter.rect_filled(rect, 0.0, fill);
}

/// 安装全局主题（Outlook 风格：扁平、纯净、克制）。
fn install_apple_theme(ctx: &egui::Context, dark: bool, accent: Color32) {
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    let (window_bg, panel_bg, _card_bg, faint, text, sel_hover, border) = if dark {
        (
            Color32::from_rgb(0x1b, 0x1b, 0x1c),
            Color32::from_rgb(0x24, 0x24, 0x25),
            Color32::from_rgb(0x2b, 0x2b, 0x2c),
            Color32::from_rgb(0x30, 0x30, 0x32),
            Color32::from_rgb(0xf2, 0xf2, 0xf3),
            Color32::from_rgb(0x37, 0x37, 0x3a),
            Color32::from_rgb(0x38, 0x38, 0x3a),
        )
    } else {
        (
            Color32::from_rgb(0xf5, 0xf6, 0xf7),
            Color32::from_rgb(0xff, 0xff, 0xff),
            Color32::from_rgb(0xff, 0xff, 0xff),
            Color32::from_rgb(0xf0, 0xf1, 0xf2),
            Color32::from_rgb(0x1f, 0x1f, 0x20),
            Color32::from_rgb(0xed, 0xef, 0xf2),
            Color32::from_rgb(0xe1, 0xe3, 0xe6),
        )
    };

    visuals.window_fill = window_bg;
    visuals.panel_fill = panel_bg;
    visuals.extreme_bg_color = faint;
    visuals.faint_bg_color = faint;
    visuals.code_bg_color = faint;
    visuals.override_text_color = Some(text);
    visuals.selection.bg_fill = accent;
    visuals.selection.stroke = Stroke::NONE;
    visuals.hyperlink_color = accent;
    visuals.window_stroke = Stroke::new(1.0_f32, border);

    for w in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        w.rounding = Rounding::same(ROUND_BTN as f32);
        w.weak_bg_fill = Color32::TRANSPARENT;
        w.bg_fill = faint;
        w.fg_stroke = Stroke::new(1.0_f32, text);
        w.bg_stroke = Stroke::new(1.0_f32, border);
    }
    visuals.widgets.hovered.bg_fill = sel_hover;
    visuals.widgets.active.bg_fill = sel_hover;

    let mut style = ctx.style().as_ref().clone();
    style.visuals = visuals;
    style
        .text_styles
        .insert(egui::TextStyle::Heading, FontId::proportional(22.0));
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::proportional(14.0));
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 6.0);
    style.spacing.menu_margin = Margin::same(6.0);
    ctx.set_style(style);
}

/// 霓虹色胶囊式主按钮。
/// 读取 Windows 系统主题强调色（注册表 DWM.AccentColor，ABGR），读取失败时返回 None。
fn system_accent_color() -> Option<Color32> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let dwm = hkcu.open_subkey("Software\\Microsoft\\Windows\\DWM").ok()?;
    let raw: u32 = dwm.get_value("AccentColor").ok()?;
    // AccentColor 为 0xAABBGGRR：依次取 R/G/B
    let r = (raw & 0xFF) as u8;
    let g = ((raw >> 8) & 0xFF) as u8;
    let b = ((raw >> 16) & 0xFF) as u8;
    Some(Color32::from_rgb(r, g, b))
}

/// 计算使窗口居中于主屏的左上角逻辑坐标（按系统 DPI 把物理像素换算为逻辑点）。
fn center_position(w: f32, h: f32) -> egui::Pos2 {
    use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    let px = unsafe { GetSystemMetrics(SM_CXSCREEN) } as f32;
    let py = unsafe { GetSystemMetrics(SM_CYSCREEN) } as f32;
    let scale = unsafe { GetDpiForSystem() as f32 } / 96.0;
    egui::pos2((px / scale - w) * 0.5, (py / scale - h) * 0.5)
}

/// 弹窗统一使用的 Frame：加大内边距，让内容与边框留出呼吸空间。
fn popup_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::window(&ctx.style())
        .inner_margin(egui::Margin::symmetric(POPUP_PAD_X, POPUP_PAD_Y))
}

/// 弹窗字体优化：与系统 UI 匹配的适中字号，避免过大。
fn optimize_popup_fonts(ui: &mut egui::Ui) {
    let ts = ui.style_mut();
    ts.text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(14.5));
    ts.text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
    ts.text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(16.0));
    ts.text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
}

/// 弹窗分组标题：采用系统一致的中等字号加粗，用主题色区分。
fn section_title(ui: &mut egui::Ui, text: &str, accent: Color32) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).size(15.0).strong().color(accent));
    ui.add_space(4.0);
}

/// 依据背景色亮度挑选高对比度的前景文字色（黑/白）。
fn text_on(bg: Color32) -> Color32 {
    let lum = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if lum > 150.0 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

/// 状态提示色：浅色模式用深红/深绿保证对比度，深色模式用亮色变体。
fn status_color(dark: bool, is_error: bool) -> Color32 {
    match (dark, is_error) {
        (false, true) => Color32::from_rgb(0xC4, 0x2B, 0x1B),
        (false, false) => Color32::from_rgb(0x0F, 0x7B, 0x34),
        (true, true) => Color32::from_rgb(0xF2, 0x8C, 0x8C),
        (true, false) => Color32::from_rgb(0x6F, 0xD8, 0x8F),
    }
}

/// 用系统默认浏览器打开 URL（Windows 走 rundll32 url.dll,FileProtocolHandler）。
fn open_url_in_browser(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err("网址为空".into());
    }
    #[cfg(windows)]
    {
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", trimmed])
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("open")
            .arg(trimmed)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

fn accent_button(
    ui: &mut egui::Ui,
    text: impl Into<egui::WidgetText>,
    accent: Color32,
) -> egui::Response {
    ui.add(
        egui::Button::new(
            RichText::new(text.into().text())
                .color(text_on(accent))
                .size(14.0)
                .strong(),
        )
        .fill(accent)
        .stroke(Stroke::NONE)
        .rounding(Rounding::same(ROUND_BTN as f32))
        .min_size(egui::vec2(108.0, 34.0)),
    )
}

/// 扁平卡片（Outlook 风格：白底/近黑 + 细边框 + 轻投影）。
fn card_frame(dark: bool, _accent: Color32) -> egui::Frame {
    let (fill, border) = if dark {
        (
            Color32::from_rgb(0x2b, 0x2b, 0x2c),
            Color32::from_rgb(0x3a, 0x3a, 0x3c),
        )
    } else {
        (
            Color32::from_rgb(0xff, 0xff, 0xff),
            Color32::from_rgb(0xe5, 0xe7, 0xea),
        )
    };
    Frame::none()
        .fill(fill)
        .rounding(Rounding::same(ROUND_CARD as f32))
        .inner_margin(Margin::same(SPACE_CARD))
        .stroke(Stroke::new(1.0_f32, border))
        .shadow(egui::epaint::Shadow {
            offset: egui::vec2(0.0, 3.0),
            blur: 14.0,
            spread: 0.0,
            color: Color32::from_black_alpha(28),
        })
}

pub struct PasswordBookApp {
    conn: Connection,
    db_path: PathBuf,

    // 会话与锁定
    key: Option<DecryptableKey>,
    unlocked: bool,
    need_setup: bool,
    locked_until: Option<Instant>,
    failed_attempts: u32,
    last_interaction: Instant,

    // 解锁/设置输入
    master_input: String,
    master_confirm: String,
    setup_lock_seconds: i64,

    // 设置（已加载）
    lock_seconds: i64,
    theme: i64,
    require_login: i64,

    // 数据视图
    records: Vec<PlainRecord>,
    categories: Vec<String>,
    selected_category: String,
    search: String,
    revealed: HashSet<i64>,

    // 编辑器
    editing: bool,
    editor: PlainRecord,
    editor_is_new: bool,
    editor_show_pwd: bool,

    // 生成器
    show_generator: bool,
    gen_len: usize,
    gen_upper: bool,
    gen_lower: bool,
    gen_digit: bool,
    gen_special: bool,
    gen_result: String,
    gen_strength: String,

    // 弹窗
    show_settings: bool,
    show_about: bool,
    show_backup: bool,
    confirm_delete: Option<i64>, // 待删除记录 ID，None 表示未开启
    change_pwd: (String, String, String), // old, new, confirm

    // 回收站 / CSV 导入 / 安全扫描
    show_deleted: bool,
    deleted_records: Vec<PlainRecord>,
    show_csv_import: bool,
    show_audit: bool,
    audit_issues: Vec<service::AuditIssue>,
    csv_import_cat: String,

    // 备份/恢复（导出与恢复各自维护路径和密码，避免两个输入框互相同步）
    backup_export_path: String,
    backup_import_path: String,
    backup_export_pwd: String,
    backup_import_pwd: String,
    merge_mode: bool,

    // 数据库启动错误。不能在持久化数据库不可用时悄悄退回内存库，否则用户会误以为数据已保存。
    startup_error: Option<String>,

    // 提示
    status: String,
    status_is_error: bool,
    toast_at: Option<Instant>,

    // 系统托盘
    tray: TrayState,
    closing: bool,

    // 动态霓虹主色（由 Windows 系统主题色驱动，每帧刷新）
    sys_accent: Color32,
    accent: Color32,

    // 视口处于“放大”状态（解锁后为完整主界面，锁定态为小解锁窗）
    // 初始为 true：保证首帧必触发一次尺寸设置（按 need_setup 选 210/330），
    // 否则首窗固定为 main.rs 的 340×210，首次设置窗内容会被裁掉。
    viewport_large: bool,
}

impl PasswordBookApp {
    pub fn new(tray_state: Option<tray::TrayState>) -> Self {
        // db 统一放 %APPDATA%\passwordbook\password.db（debug 和 release 共享）
        let appdata = std::env::var("APPDATA")
            .unwrap_or_else(|_| ".".to_string());
        let app_dir = std::path::PathBuf::from(&appdata).join("passwordbook");
        let _ = std::fs::create_dir_all(&app_dir);
        let db_path = app_dir.join("password.db");
        let (conn, startup_error) = match db::open(&db_path) {
            Ok(conn) => (conn, None),
            Err(e) => {
                eprintln!("打开数据库失败: {e}");
                let fallback = Connection::open_in_memory().unwrap_or_else(|fallback_error| {
                    panic!("无法创建错误页数据库连接: {fallback_error}")
                });
                (fallback, Some(e))
            }
        };
        let need_setup =
            startup_error.is_none() && db::has_config(&conn).map(|h| !h).unwrap_or(true);
        let require_login = if !need_setup {
            db::get_config(&conn).map(|c| c.require_login).unwrap_or(1)
        } else {
            1
        };
        let viewport_large = !need_setup && require_login == 0;
        let tray = tray_state.unwrap_or_else(tray::TrayState::new_empty);
        Self {
            conn,
            db_path,
            key: None,
            unlocked: false,
            need_setup,
            locked_until: None,
            failed_attempts: 0,
            last_interaction: Instant::now(),
            master_input: String::new(),
            master_confirm: String::new(),
            setup_lock_seconds: 300,
            lock_seconds: 300,
            theme: 0,
            require_login,
            records: Vec::new(),
            categories: db::DEFAULT_CATEGORIES
                .iter()
                .map(|s| s.to_string())
                .collect(),
            selected_category: String::new(), // 空 = 全部
            search: String::new(),
            revealed: HashSet::new(),
            editing: false,
            editor: PlainRecord::default(),
            editor_is_new: false,
            editor_show_pwd: false,
            show_generator: false,
            gen_len: 16,
            gen_upper: true,
            gen_lower: true,
            gen_digit: true,
            gen_special: true,
            gen_result: String::new(),
            gen_strength: String::new(),
            show_settings: false,
            show_about: false,
            show_backup: false,
            confirm_delete: None,
            change_pwd: (String::new(), String::new(), String::new()),
            show_deleted: false,
            deleted_records: Vec::new(),
            show_csv_import: false,
            show_audit: false,
            audit_issues: Vec::new(),
            csv_import_cat: String::new(),
            backup_export_path: String::new(),
            backup_import_path: String::new(),
            backup_export_pwd: String::new(),
            backup_import_pwd: String::new(),
            merge_mode: true,
            startup_error,
            status: String::new(),
            status_is_error: false,
            toast_at: None,
            tray,
            closing: false,
            sys_accent: system_accent_color().unwrap_or(ACCENT),
            accent: ACCENT,
            viewport_large: true,
        }
    }

    fn show_status(&mut self, msg: &str, is_error: bool) {
        self.status = msg.to_string();
        self.status_is_error = is_error;
        self.toast_at = Some(Instant::now() + Duration::from_secs(5));
    }

    fn load_settings_after_unlock(&mut self) -> Result<(), String> {
        let (lock, theme, req_login) = service::get_settings(&self.conn)?;
        self.lock_seconds = lock;
        self.theme = theme;
        self.require_login = req_login;
        Ok(())
    }

    fn refresh_records(&mut self) -> Result<(), String> {
        let key = self
            .key
            .as_ref()
            .ok_or_else(|| "会话已锁定".to_string())?
            .clone();
        let records = service::list_records(&self.conn, &key)?;
        let cats = service::collect_categories(&self.conn, &key)?;
        self.records = records;
        self.categories = cats;
        Ok(())
    }

    fn refresh_deleted(&mut self) {
        if let Some(key) = self.key.as_ref() {
            match service::list_deleted(&self.conn, key) {
                Ok(v) => self.deleted_records = v,
                Err(_) => self.deleted_records = Vec::new(),
            }
        }
    }

    fn lock(&mut self) {
        self.key = None; // 离开作用域即 zeroize
        self.unlocked = false;
        self.master_input.clear();
        self.master_confirm.clear();
        self.records.clear();
        self.revealed.clear();
        self.editor = PlainRecord::default();
        self.editor_show_pwd = false;
        self.gen_result.clear();
        self.gen_strength.clear();
        self.change_pwd = (String::new(), String::new(), String::new());
        self.backup_export_pwd.clear();
        self.backup_import_pwd.clear();
        self.editing = false;
        self.show_generator = false;
        self.show_settings = false;
        self.show_about = false;
        self.show_backup = false;
        self.confirm_delete = None;
        self.last_interaction = Instant::now();
    }

    fn unlock_attempt(&mut self) {
        if let Some(until) = self.locked_until {
            if Instant::now() < until {
                let s = until.duration_since(Instant::now()).as_secs();
                self.show_status(&format!("尝试过于频繁，请 {s} 秒后再试"), true);
                return;
            }
        }
        match service::unlock(&self.conn, &self.master_input) {
            Ok(key) => {
                self.key = Some(key);
                self.unlocked = true;
                self.master_input.clear();
                self.failed_attempts = 0;
                self.locked_until = None;
                if let Err(e) = self
                    .load_settings_after_unlock()
                    .and_then(|_| self.refresh_records())
                {
                    self.lock();
                    self.show_status(&format!("读取数据失败: {e}"), true);
                    return;
                }
                self.show_status("解锁成功", false);
            }
            Err(service::UnlockError::WrongPassword) => {
                self.failed_attempts += 1;
                self.show_status("主密码错误", true);
                if self.failed_attempts >= 5 {
                    self.locked_until = Some(Instant::now() + Duration::from_secs(30));
                    self.failed_attempts = 0;
                    self.show_status("连续错误次数过多，已锁定 30 秒", true);
                }
            }
            Err(service::UnlockError::Db(e)) => self.show_status(&format!("数据库错误: {e}"), true),
        }
    }

    fn setup_attempt(&mut self) {
        if self.master_input.is_empty() {
            self.show_status("主密码不能为空", true);
            return;
        }
        if self.master_input != self.master_confirm {
            self.show_status("两次输入的主密码不一致", true);
            return;
        }
        if self.master_input.len() < 4 {
            self.show_status("主密码至少 4 位", true);
            return;
        }
        match service::setup(&self.conn, &self.master_input, self.setup_lock_seconds) {
            Ok(key) => {
                self.need_setup = false;
                self.key = Some(key);
                self.unlocked = true;
                self.master_input.clear();
                self.master_confirm.clear();
                if let Err(e) = self
                    .load_settings_after_unlock()
                    .and_then(|_| self.refresh_records())
                {
                    self.lock();
                    self.show_status(&format!("读取数据失败: {e}"), true);
                    return;
                }
                self.show_status("初始化完成，欢迎使用", false);
            }
            Err(e) => self.show_status(&e, true),
        }
    }

    // ---- 业务动作 ----

    /// 确保同一时刻只显示一个弹窗（单窗口模态）。
    fn close_popups(&mut self) {
        self.editing = false;
        self.editor = PlainRecord::default();
        self.editor_show_pwd = false;
        self.show_generator = false;
        self.gen_result.clear();
        self.gen_strength.clear();
        self.show_settings = false;
        self.show_about = false;
        self.change_pwd = (String::new(), String::new(), String::new());
        self.show_backup = false;
        self.backup_export_pwd.clear();
        self.backup_import_pwd.clear();
        self.confirm_delete = None;
        self.show_csv_import = false;
        self.show_audit = false;
        self.csv_import_cat.clear();
    }

    fn open_editor(&mut self, record: Option<PlainRecord>) {
        self.close_popups();
        self.editor_is_new = record.is_none();
        self.editor = record.unwrap_or_default();
        self.editor_show_pwd = false;
        self.editing = true;
    }

    fn save_editor(&mut self) {
        if self.editor.name.trim().is_empty() || self.editor.password.trim().is_empty() {
            self.show_status("名称和密码不能为空", true);
            return;
        }
        let key = match self.key.as_ref() {
            Some(k) => k.clone(),
            None => {
                self.show_status("会话已锁定", true);
                return;
            }
        };
        if self.editor_is_new {
            match service::add_record(&self.conn, &key, self.editor.clone()) {
                Ok(_) => {
                    self.show_status("已新增记录", false);
                    self.editing = false;
                }
                Err(e) => self.show_status(&e, true),
            }
        } else if let Some(id) = self.editor.id {
            let mut rec = self.editor.clone();
            rec.id = Some(id);
            match service::update_record(&self.conn, &key, rec) {
                Ok(_) => {
                    self.show_status("已保存修改", false);
                    self.editing = false;
                }
                Err(e) => self.show_status(&e, true),
            }
        }
        if let Err(e) = self.refresh_records() {
            self.show_status(&format!("刷新记录失败: {e}"), true);
        }
    }

    fn delete_record(&mut self, id: i64) {
        self.close_popups();
        self.confirm_delete = Some(id);
    }

    fn confirm_delete_do(&mut self) {
        if let Some(id) = self.confirm_delete.take() {
            match service::delete_record(&self.conn, id) {
                Ok(_) => self.show_status("已删除记录", false),
                Err(e) => self.show_status(&e, true),
            }
            if let Err(e) = self.refresh_records() {
                self.show_status(&format!("刷新记录失败: {e}"), true);
            }
        }
    }

    fn copy_to_clipboard(&mut self, text: &str, what: &str) {
        match clipboard::set_clipboard(text) {
            Ok(_) => self.show_status(&format!("已复制{what}到剪贴板"), false),
            Err(e) => self.show_status(&e, true),
        }
    }

    // ---- 视图 ----

    fn draw_unlock(&mut self, ctx: &egui::Context) {
        let bg = Color32::from_rgb(0xf5, 0xf7, 0xfc);
        egui::CentralPanel::default()
            .frame(Frame::none().fill(bg).inner_margin(Margin::same(20.0)))
            .show(ctx, |ui| {
                let mut submit = false;
                let mut quit = false;
                if let Some(error) = self.startup_error.clone() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(28.0);
                        ui.label(RichText::new("无法打开密码数据库").size(20.0).strong());
                        ui.add_space(10.0);
                        ui.colored_label(status_color(false, true), error);
                        ui.add_space(8.0);
                        ui.label(RichText::new(self.db_path.display().to_string()).weak());
                        ui.label("请检查文件权限或磁盘状态后重新启动程序。");
                        ui.add_space(18.0);
                        if accent_button(ui, "退出", Color32::from_rgb(0x9a, 0x9d, 0xa6)).clicked()
                        {
                            quit = true;
                        }
                    });
                    if quit {
                        self.closing = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    return;
                }
                // 首次设置窗内容较多，用固定小留白，避免按比例留白把内容挤出窗口
                let top_gap = if self.need_setup {
                    24.0
                } else {
                    (ui.available_height() * 0.28).max(20.0)
                };
                ui.vertical_centered(|ui| {
                    ui.add_space(top_gap);
                    let password_response = ui.add(
                        egui::TextEdit::singleline(&mut self.master_input)
                            .password(true)
                            .hint_text("主密码")
                            .desired_width(300.0)
                            .margin(Margin::symmetric(14.0, 10.0)),
                    );
                    if password_response.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        submit = true;
                    }
                    if self.need_setup {
                        ui.add_space(12.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.master_confirm)
                                .password(true)
                                .hint_text("再次输入主密码")
                                .desired_width(300.0)
                                .margin(Margin::symmetric(14.0, 10.0)),
                        );
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            ui.label("自动锁屏");
                            ui.add(
                                egui::Slider::new(&mut self.setup_lock_seconds, 60..=3600)
                                    .suffix(" 秒"),
                            );
                        });
                    }
                    ui.add_space(18.0);
                    ui.horizontal_centered(|ui| {
                        let r = accent_button(
                            ui,
                            if self.need_setup { "创建" } else { "解锁" },
                            self.accent,
                        );
                        if r.clicked() {
                            submit = true;
                        }
                        ui.add_space(10.0);
                        let g = Color32::from_rgb(0x9a, 0x9d, 0xa6);
                        let r = accent_button(ui, "退出", g);
                        if r.clicked() {
                            quit = true;
                        }
                    });
                    if !self.status.is_empty() {
                        ui.add_space(14.0);
                        // 解锁窗始终为浅色底，使用高对比状态色
                        ui.colored_label(status_color(false, self.status_is_error), &self.status);
                    }
                });

                if submit {
                    if self.need_setup {
                        self.setup_attempt();
                    } else {
                        self.unlock_attempt();
                    }
                }
                if quit {
                    self.closing = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
    }

    fn draw_main(&mut self, ctx: &egui::Context) {
        let dark = self.theme == 1;
        let side_bg = if dark {
            Color32::from_rgb(0x24, 0x24, 0x25)
        } else {
            Color32::from_rgb(0xff, 0xff, 0xff)
        };
        let side_text = if dark {
            Color32::from_rgb(0xd8, 0xd8, 0xda)
        } else {
            Color32::from_rgb(0x1f, 0x1f, 0x20)
        };
        let side_text_muted = if dark {
            Color32::from_rgb(0x8a, 0x8d, 0x93)
        } else {
            Color32::from_rgb(0x66, 0x66, 0x66)
        };
        let side_text_sel = text_on(self.accent);

        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(SIDE_W)
            .frame(
                Frame::none()
                    .fill(side_bg)
                    .inner_margin(Margin::symmetric(10.0, 12.0)),
            )
            .show(ctx, |ui| {
                // ---- 新增（主按钮，窄一点） ----
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("＋ 新增")
                                .color(text_on(self.accent))
                                .size(14.5)
                                .strong(),
                        )
                        .fill(self.accent)
                        .stroke(Stroke::NONE)
                        .rounding(Rounding::same(ROUND_BTN as f32))
                        .min_size(egui::vec2(0.0, BTN_H + 2.0)),
                    )
                    .clicked()
                {
                    self.open_editor(None);
                }

                // ---- 分类 ----
                ui.add_space(14.0);
                ui.label(
                    RichText::new("分类")
                        .strong()
                        .size(12.0)
                        .color(side_text_muted),
                );
                ui.add_space(4.0);
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(format!("全部   {}", self.records.len()))
                                .color(if self.selected_category.is_empty() {
                                    side_text_sel
                                } else {
                                    side_text
                                })
                                .strong(),
                        )
                        .fill(if self.selected_category.is_empty() {
                            self.accent
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(Stroke::NONE)
                        .rounding(Rounding::same(ROUND_BTN as f32))
                        .min_size(egui::vec2(0.0, BTN_H)),
                    )
                    .clicked()
                {
                    self.selected_category.clear();
                }
                let cats = self.categories.clone();
                for c in cats {
                    let sel = self.selected_category == c;
                    let count = self.records.iter().filter(|r| r.category == c).count();
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(format!("{c}   {count}")).color(if sel {
                                    side_text_sel
                                } else {
                                    side_text
                                }),
                            )
                            .fill(if sel {
                                self.accent
                            } else {
                                Color32::TRANSPARENT
                            })
                            .stroke(Stroke::NONE)
                            .rounding(Rounding::same(ROUND_BTN as f32))
                            .min_size(egui::vec2(0.0, BTN_H)),
                        )
                        .clicked()
                    {
                        self.selected_category = c;
                    }
                }

                // ---- 工具 ----
                ui.add_space(16.0);
                ui.label(
                    RichText::new("工具")
                        .strong()
                        .size(12.0)
                        .color(side_text_muted),
                );
                ui.add_space(4.0);
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("备份 / 恢复").color(side_text),
                        )
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE)
                        .rounding(Rounding::same(ROUND_BTN as f32))
                        .min_size(egui::vec2(0.0, BTN_H)),
                    )
                    .on_hover_text("导出或恢复加密备份")
                    .clicked()
                {
                    self.close_popups();
                    self.show_backup = true;
                }
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("密码生成器").color(side_text),
                        )
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE)
                        .rounding(Rounding::same(ROUND_BTN as f32))
                        .min_size(egui::vec2(0.0, BTN_H)),
                    )
                    .on_hover_text("生成高强度随机密码")
                    .clicked()
                {
                    self.close_popups();
                    self.show_generator = true;
                }
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("CSV 导入").color(side_text),
                        )
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE)
                        .rounding(Rounding::same(ROUND_BTN as f32))
                        .min_size(egui::vec2(0.0, BTN_H)),
                    )
                    .on_hover_text("从 Chrome/Edge/Firefox CSV 导入")
                    .clicked()
                {
                    self.close_popups();
                    self.show_csv_import = true;
                }
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("安全检查").color(side_text),
                        )
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE)
                        .rounding(Rounding::same(ROUND_BTN as f32))
                        .min_size(egui::vec2(0.0, BTN_H)),
                    )
                    .on_hover_text("扫描弱口令、重复口令、空密码")
                    .clicked()
                {
                    self.close_popups();
                    self.show_audit = true;
                    self.run_audit();
                }
                // 回收站入口：单独一行，用不同样式（浅红底）
                let deleted_count = self.deleted_count();
                let trash_label = if deleted_count > 0 {
                    format!("回收站 ({})", deleted_count)
                } else {
                    "回收站".into()
                };
                let trash_fill = if self.show_deleted {
                    self.accent // 选中主题色填充
                } else {
                    Color32::TRANSPARENT
                };
                let trash_color = if self.show_deleted {
                    text_on(self.accent)
                } else {
                    if deleted_count > 0 {
                        Color32::from_rgb(0xc4, 0x2b, 0x1b)
                    } else {
                        side_text
                    }
                };
                if ui
                    .add(
                        egui::Button::new(RichText::new(trash_label).color(trash_color))
                            .fill(trash_fill)
                            .stroke(Stroke::NONE)
                            .rounding(Rounding::same(ROUND_BTN as f32))
                            .min_size(egui::vec2(0.0, BTN_H)),
                    )
                    .on_hover_text(if deleted_count > 0 {
                        format!("已删除 {deleted_count} 条记录（点击查看）")
                    } else {
                        "回收站（点击查看）".to_string()
                    })
                    .clicked()
                {
                    self.close_popups();
                    self.show_deleted = !self.show_deleted;
                    if self.show_deleted {
                        self.refresh_deleted();
                    }
                }

            });

        egui::CentralPanel::default().show(ctx, |ui| {
            // ---- 顶部搜索栏 + 右侧工具图标 ----
            ui.horizontal(|ui| {
                // 左：搜索框 + 放大镜 + 清除
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("名称 / 账号 / 备注")
                        .desired_width(220.0)
                        .margin(Margin::symmetric(10.0, 6.0)),
                );
                let search_btn = ui.add(
                    egui::Button::new(RichText::new("🔍").size(16.0))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE)
                        .rounding(Rounding::same(ROUND_BTN as f32))
                        .min_size(egui::vec2(28.0, 28.0)),
                );
                search_btn.on_hover_text("搜索");
                if !self.search.is_empty() && ui.small_button("清除").clicked() {
                    self.search.clear();
                }

                // 右：锁定 / 设置 / 关于（靠右对齐，right_to_left 布局让最后写的在最左）
                let bar_text = if dark {
                    Color32::from_rgb(0xd8, 0xd8, 0xda)
                } else {
                    Color32::from_rgb(0x1f, 0x1f, 0x20)
                };
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        // ① 关于（最右）
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("ⓘ").size(14.0).color(bar_text),
                                )
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE)
                                .rounding(Rounding::same(ROUND_BTN as f32))
                                .min_size(egui::vec2(40.0, 28.0)),
                            )
                            .on_hover_text("关于密码本")
                            .clicked()
                        {
                            self.close_popups();
                            self.show_about = true;
                        }
                        // ② 设置
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("⚙").size(14.0).color(bar_text),
                                )
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE)
                                .rounding(Rounding::same(ROUND_BTN as f32))
                                .min_size(egui::vec2(40.0, 28.0)),
                            )
                            .on_hover_text("设置：自动锁屏、主题、主密码")
                            .clicked()
                        {
                            self.close_popups();
                            self.show_settings = true;
                        }
                        // ③ 锁定（最左，靠近搜索栏）
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("🔒").size(14.0).color(bar_text),
                                )
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE)
                                .rounding(Rounding::same(ROUND_BTN as f32))
                                .min_size(egui::vec2(40.0, 28.0)),
                            )
                            .on_hover_text("立即锁定并清除敏感数据")
                            .clicked()
                        {
                            self.lock();
                        }
                    },
                );
            });
            ui.add_space(4.0);
            card_frame(dark, self.accent).show(ui, |ui| {
                ui.set_min_size(egui::vec2(ui.available_width(), ui.available_height()));
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.draw_records_table(ui);
                    });
            });
            self.draw_status_bar(ui);
        });

        // 弹窗（同一时刻仅显示一个即可）
        if self.editing
            || self.show_generator
            || self.show_settings
            || self.show_backup
            || self.show_about
            || self.show_csv_import
            || self.show_audit
            || self.confirm_delete.is_some()
        {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("modal-backdrop"),
            ));
            painter.rect_filled(ctx.screen_rect(), 0.0, Color32::from_black_alpha(38));
        }
        if self.editing {
            self.draw_editor_window(ctx);
        }
        if self.show_generator {
            self.draw_generator_window(ctx);
        }
        if self.show_settings {
            self.draw_settings_window(ctx);
        }
        if self.show_backup {
            self.draw_backup_window(ctx);
        }
        if self.show_about {
            self.draw_about_window(ctx);
        }
        if self.show_csv_import {
            self.draw_csv_import_window(ctx);
        }
        if self.show_audit {
            self.draw_audit_window(ctx);
        }
        if self.confirm_delete.is_some() {
            self.draw_confirm_delete(ctx);
        }
    }

    fn draw_about_window(&mut self, ctx: &egui::Context) {
        let mut open_ok = true;
        egui::Window::new("关于密码本")
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open_ok)
            .collapsible(false)
            .resizable(false)
            .frame(
                egui::Frame::window(&ctx.style())
                    .inner_margin(Margin::symmetric(20.0, 12.0)),
            )
            .default_width(360.0)
            .default_height(330.0)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                optimize_popup_fonts(ui);

                // 应用图标（与托盘图标一致）——水平居中，避免 justified 撑满垂直空间
                let (iw, ih, icon_bytes) = crate::tray::icon_rgba();
                let color_image =
                    egui::ColorImage::from_rgba_unmultiplied([iw as usize, ih as usize], &icon_bytes);
                let texture = ctx.load_texture(
                    "about_app_icon",
                    color_image,
                    egui::TextureOptions::LINEAR,
                );
                ui.vertical_centered(|ui| {
                    ui.add_sized(
                        [52.0, 52.0],
                        egui::Image::new(&texture).rounding(10.0),
                    );
                });

                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("密码本").size(18.0).strong().color(self.accent));
                    ui.label(RichText::new(format!("版本 v{APP_VERSION}")).weak());
                });
                ui.add_space(2.0);
                ui.separator();

                ui.label(
                    RichText::new("一款本地离线的加密密码本，数据仅保存在本机。")
                        .weak(),
                );
                ui.label(
                    RichText::new("采用 AES-256-GCM 加密与 PBKDF2 密钥派生。")
                        .weak(),
                );

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("技术栈").strong());
                    ui.label(RichText::new("Rust · egui · SQLite").weak());
                });

                ui.add_space(6.0);
                ui.separator();
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("© 版权所有 侵权必究").size(12.0).weak());
                    ui.label(RichText::new("联系：QQ 3571038944").size(12.0).weak());
                });

                ui.add_space(6.0);
                ui.vertical_centered(|ui| {
                    if accent_button(ui, "关闭", self.accent).clicked() {
                        self.show_about = false;
                    }
                });
            });
        self.show_about = open_ok && self.show_about;
    }

    fn run_audit(&mut self) {
        let issues = service::audit_passwords(&self.records);
        self.audit_issues = issues;
    }

    fn deleted_count(&self) -> usize {
        match service::list_deleted(&self.conn, self.key.as_ref().unwrap()) {
            Ok(v) => v.len(),
            Err(_) => 0,
        }
    }

    fn draw_csv_import_window(&mut self, ctx: &egui::Context) {
        let mut open_ok = true;
        egui::Window::new("从 CSV 导入")
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open_ok)
            .collapsible(false)
            .resizable(false)
            .frame(popup_frame(ctx))
            .default_width(440.0)
            .default_height(220.0)
            .show(ctx, |ui| {
                optimize_popup_fonts(ui);
                ui.spacing_mut().item_spacing.y = 8.0;

                ui.label(
                    RichText::new("支持 Chrome / Edge / Firefox 导出的 CSV 格式")
                        .weak()
                        .size(12.0),
                );

                ui.label(RichText::new("目标分类").strong().size(12.0));
                egui::ComboBox::from_id_source("csv_import_cat")
                    .selected_text(if self.csv_import_cat.is_empty() {
                        "（自动：CSV 导入）"
                    } else {
                        &self.csv_import_cat
                    })
                    .show_ui(ui, |ui| {
                        let cats: Vec<String> =
                            vec!["CSV 导入".to_string(), "社交".into(), "邮箱".into(),
                                 "购物".into(), "工作".into(), "其他".into()];
                        for c in cats {
                            if ui.selectable_label(self.csv_import_cat == c, &c).clicked() {
                                self.csv_import_cat = c;
                            }
                        }
                    });

                ui.add_space(4.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(12.0);
                    if accent_button(ui, "选择 CSV 文件", self.accent).clicked() {
                        if let Some(file_path) = rfd::FileDialog::new()
                            .add_filter("CSV 文件", &["csv"])
                            .pick_file()
                        {
                            if let Ok(text) = std::fs::read_to_string(&file_path) {
                                let cat = if self.csv_import_cat.is_empty() {
                                    "CSV 导入"
                                } else {
                                    self.csv_import_cat.as_str()
                                };
                                match service::import_csv(
                                    &mut self.conn,
                                    self.key.as_ref().unwrap(),
                                    &text,
                                    cat,
                                ) {
                                    Ok(n) => {
                                        self.refresh_records();
                                        self.show_status(
                                            &format!("成功导入 {n} 条记录"),
                                            false,
                                        );
                                        self.show_csv_import = false;
                                    }
                                    Err(e) => {
                                        self.show_status(
                                            &format!("导入失败: {e}"),
                                            true,
                                        );
                                    }
                                }
                            } else {
                                self.show_status("无法读取文件".into(), true);
                            }
                        }
                    }
                    ui.add_space(4.0);
                    if ui.button("取消").clicked() {
                        self.show_csv_import = false;
                    }
                });
            });
        self.show_csv_import = open_ok && self.show_csv_import;
    }

    fn draw_audit_window(&mut self, ctx: &egui::Context) {
        let mut open_ok = true;
        let total = self.records.len();
        let issue_count = self.audit_issues.len();
        let weak_count = self
            .audit_issues
            .iter()
            .filter(|i| matches!(i.kind, service::AuditKind::Weak))
            .count();
        let reuse_count = self
            .audit_issues
            .iter()
            .filter(|i| matches!(i.kind, service::AuditKind::Reused))
            .count();
        let empty_count = self
            .audit_issues
            .iter()
            .filter(|i| matches!(i.kind, service::AuditKind::Empty))
            .count();
        let short_count = self
            .audit_issues
            .iter()
            .filter(|i| matches!(i.kind, service::AuditKind::TooShort))
            .count();

        egui::Window::new("安全检查结果")
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open_ok)
            .collapsible(false)
            .resizable(true)
            .frame(popup_frame(ctx))
            .default_width(480.0)
            .default_height(380.0)
            .show(ctx, |ui| {
                optimize_popup_fonts(ui);
                ui.spacing_mut().item_spacing.y = 6.0;

                if issue_count == 0 {
                    ui.vertical_centered(|ui| {
                        ui.add_space(16.0);
                        ui.label(RichText::new("✓ 全部检查通过").size(16.0).color(
                            Color32::from_rgb(0x0f, 0x7b, 0x34),
                        ));
                        ui.label(
                            RichText::new(format!("你共有 {total} 条记录，未发现弱口令或重复"))
                                .weak(),
                        );
                    });
                    ui.add_space(20.0);
                } else {
                    ui.label(
                        RichText::new(format!("{total} 条记录中发现 {issue_count} 条问题："))
                            .strong(),
                    );
                    ui.add_space(2.0);

                    let summary = format!(
                        "弱口令 {weak_count} · 重复 {reuse_count} · 空密码 {empty_count} · 过短 {short_count}"
                    );
                    ui.label(RichText::new(summary).weak().size(12.0));
                    ui.add_space(8.0);

                    egui::ScrollArea::vertical()
                        .max_height(220.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            // 先克隆 audit_issues，避免借用冲突
                            let issues = self.audit_issues.clone();
                            let mut to_edit: Option<i64> = None;
                            // 按 kind 分组展示
                            let mut current_kind: Option<service::AuditKind> = None;
                            for issue in &issues {
                                let kind = issue.kind.clone();
                                if Some(kind.clone()) != current_kind {
                                    let label = match kind {
                                        service::AuditKind::Weak => "⚠ 弱口令",
                                        service::AuditKind::Reused => "⟲ 重复口令",
                                        service::AuditKind::Empty => "✕ 空密码",
                                        service::AuditKind::TooShort => "— 过短",
                                    };
                                    ui.add_space(4.0);
                                    ui.label(RichText::new(label).strong().size(13.0));
                                    current_kind = Some(kind.clone());
                                }
                                let id = issue.record_id;
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(&issue.record_name).strong().color(self.accent));
                                    ui.label(RichText::new(" — ").weak());
                                    ui.label(RichText::new(&issue.detail).weak().size(12.0));
                                    if ui.small_button("编辑").clicked() {
                                        to_edit = Some(id);
                                    }
                                });
                            }
                            if let Some(id) = to_edit {
                                if let Some(rec) =
                                    self.records.iter().find(|r| r.id == Some(id))
                                {
                                    self.open_editor(Some(rec.clone()));
                                    self.show_audit = false;
                                }
                            }
                        });
                }

                ui.add_space(6.0);
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.small_button("重新扫描").clicked() {
                        self.run_audit();
                    }
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if accent_button(ui, "关闭", self.accent).clicked() {
                                self.show_audit = false;
                            }
                        },
                    );
                });
            });
        self.show_audit = open_ok && self.show_audit;
    }

    fn draw_records_table(&mut self, ui: &mut egui::Ui) {
        let dark = self.theme == 1;
        let query = self.search.trim().to_lowercase();

        // 根据回收站/正常模式选择数据源
        let pool: Vec<PlainRecord> = if self.show_deleted {
            self.deleted_records.clone()
        } else {
            self.records.clone()
        };

        let filtered: Vec<usize> = pool
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                let cat_ok = if self.show_deleted {
                    true // 回收站不按分类筛选
                } else {
                    self.selected_category.is_empty() || r.category == self.selected_category
                };
                let search_ok = query.is_empty()
                    || r.name.to_lowercase().contains(&query)
                    || r.account.to_lowercase().contains(&query)
                    || r.url.to_lowercase().contains(&query)
                    || r.remark.to_lowercase().contains(&query);
                cat_ok && search_ok
            })
            .map(|(i, _)| i)
            .collect();

        if filtered.is_empty() {
            ui.centered_and_justified(|ui| {
                let message = if self.show_deleted {
                    "回收站是空的"
                } else if self.search.trim().is_empty() && self.selected_category.is_empty() {
                    "暂无记录，点击左侧「＋ 新增」添加"
                } else {
                    "没有匹配的记录"
                };
                ui.label(RichText::new(message).weak());
            });
            return;
        }

        egui::Grid::new("records_grid")
            .striped(true)
            .min_col_width(60.0)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                // 表头（回收站模式下加"删除时间"列，操作列变为"恢复 / 彻底删除"）
                if self.show_deleted {
                    ui.label(RichText::new("分类").strong());
                    ui.label(RichText::new("名称").strong());
                    ui.label(RichText::new("账号").strong());
                    ui.label(RichText::new("密码").strong());
                    ui.label(RichText::new("删除时间").strong());
                    ui.label(RichText::new("操作").strong());
                } else {
                    ui.label(RichText::new("分类").strong());
                    ui.label(RichText::new("名称").strong());
                    ui.label(RichText::new("账号").strong());
                    ui.label(RichText::new("密码").strong());
                    ui.label(RichText::new("更新时间").strong());
                    ui.label(RichText::new("操作").strong());
                }
                ui.end_row();

                let mut to_delete: Option<i64> = None;
                let mut to_edit: Option<i64> = None;
                let mut to_copy_acct: Option<i64> = None;
                let mut to_copy_pwd: Option<i64> = None;
                let mut to_open_url: Option<i64> = None;
                let mut to_restore: Option<i64> = None;
                let mut to_hard_delete: Option<i64> = None;

                for idx in filtered {
                    let rec = pool[idx].clone();
                    let id = rec.id.unwrap_or(-1);
                    let masked = !self.revealed.contains(&id);

                    if ui.selectable_label(false, &rec.category).clicked() {
                        self.selected_category = rec.category.clone();
                    }
                    let name_resp = ui.selectable_label(
                        false,
                        RichText::new(&rec.name).strong().color(self.accent),
                    );
                    // 使用显式按钮复制，避免用户只是想选中内容却意外把密码写入剪贴板。
                    ui.horizontal(|ui| {
                        ui.label(if rec.account.is_empty() {
                            "—"
                        } else {
                            &rec.account
                        });
                        if !rec.account.is_empty()
                            && ui.small_button("复制").on_hover_text("复制账号").clicked()
                        {
                            to_copy_acct = Some(id);
                        }
                        if !rec.url.trim().is_empty()
                            && ui
                                .small_button("打开")
                                .on_hover_text(format!("打开网址 {}", rec.url))
                                .clicked()
                        {
                            to_open_url = Some(id);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(if masked {
                            "••••••••"
                        } else {
                            &rec.password
                        });
                        if ui
                            .small_button(if masked { "显示" } else { "隐藏" })
                            .on_hover_text("切换密码显示状态")
                            .clicked()
                        {
                            if masked {
                                self.revealed.insert(id);
                            } else {
                                self.revealed.remove(&id);
                            }
                        }
                        if ui.small_button("复制").on_hover_text("复制密码").clicked() {
                            to_copy_pwd = Some(id);
                        }
                    });
                    ui.label(&rec.update_time);
                    if self.show_deleted {
                        // 回收站模式：恢复 + 彻底删除
                        if ui.small_button("恢复").clicked() {
                            to_restore = Some(id);
                        }
                        if ui
                            .small_button(RichText::new("彻底删除").color(status_color(dark, true)))
                            .clicked()
                        {
                            to_hard_delete = Some(id);
                        }
                    } else {
                        // 正常模式：编辑 + 删除
                        if ui.small_button("编辑").clicked() {
                            to_edit = Some(id);
                        }
                        if ui
                            .small_button(RichText::new("删除").color(status_color(dark, true)))
                            .clicked()
                        {
                            to_delete = Some(id);
                        }
                    }

                    // 右键菜单挂在名称上
                    name_resp.context_menu(|ui| {
                        if !rec.url.trim().is_empty() && ui.button("打开网址").clicked() {
                            to_open_url = Some(id);
                            ui.close_menu();
                        }
                        if ui.button("复制账号").clicked() {
                            to_copy_acct = Some(id);
                            ui.close_menu();
                        }
                        if ui.button("复制密码").clicked() {
                            to_copy_pwd = Some(id);
                            ui.close_menu();
                        }
                        if ui
                            .button(if masked {
                                "显示密码"
                            } else {
                                "隐藏密码"
                            })
                            .clicked()
                        {
                            if masked {
                                self.revealed.insert(id);
                            } else {
                                self.revealed.remove(&id);
                            }
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("编辑").clicked() {
                            to_edit = Some(id);
                            ui.close_menu();
                        }
                        if ui
                            .button(RichText::new("删除").color(status_color(dark, true)))
                            .clicked()
                        {
                            to_delete = Some(id);
                            ui.close_menu();
                        }
                    });
                    ui.end_row();
                }

                if let Some(id) = to_copy_acct {
                    let text = self
                        .records
                        .iter()
                        .find(|x| x.id == Some(id))
                        .map(|r| r.account.clone());
                    if let Some(text) = text {
                        self.copy_to_clipboard(&text, "账号");
                    }
                }
                if let Some(id) = to_copy_pwd {
                    let text = self
                        .records
                        .iter()
                        .find(|x| x.id == Some(id))
                        .map(|r| r.password.clone());
                    if let Some(text) = text {
                        self.copy_to_clipboard(&text, "密码");
                    }
                }
                if let Some(id) = to_open_url {
                    let url = self
                        .records
                        .iter()
                        .find(|x| x.id == Some(id))
                        .map(|r| r.url.clone());
                    if let Some(url) = url {
                        match open_url_in_browser(&url) {
                            Ok(_) => self.show_status("已在浏览器中打开", false),
                            Err(e) => self.show_status(&format!("打开失败：{e}"), true),
                        }
                    }
                }
                if let Some(id) = to_edit {
                    if let Some(rec) = self.records.iter().find(|x| x.id == Some(id)) {
                        self.open_editor(Some(rec.clone()));
                    }
                }
                if let Some(id) = to_delete {
                    self.delete_record(id);
                }
                if let Some(id) = to_restore {
                    match service::restore(&self.conn, id) {
                        Ok(_) => {
                            self.refresh_records();
                            self.refresh_deleted();
                            self.show_status("记录已恢复", false);
                        }
                        Err(e) => self.show_status(&format!("恢复失败: {e}"), true),
                    }
                }
                if let Some(id) = to_hard_delete {
                    match service::hard_delete(&self.conn, id) {
                        Ok(_) => {
                            self.refresh_deleted();
                            self.show_status("记录已彻底删除（不可恢复）", false);
                        }
                        Err(e) => self.show_status(&format!("彻底删除失败: {e}"), true),
                    }
                }
            });
    }

    fn draw_editor_window(&mut self, ctx: &egui::Context) {
        let title = if self.editor_is_new {
            "新增记录"
        } else {
            "编辑记录"
        };
        let dark = self.theme == 1;
        let mut open_ok = true;
        egui::Window::new(title)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open_ok)
            .collapsible(false)
            .resizable(false)
            .frame(popup_frame(ctx))
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                optimize_popup_fonts(ui);
                egui::Grid::new("editor_grid")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label("分类");
                        let cats = self.categories.clone();
                        let mut cur = self.editor.category.clone();
                        egui::ComboBox::from_id_salt("cat_combo")
                            .selected_text(if cur.is_empty() { "请选择" } else { &cur })
                            .show_ui(ui, |ui| {
                                for c in cats {
                                    ui.selectable_value(&mut cur, c.clone(), &c);
                                }
                            });
                        if cur != self.editor.category {
                            self.editor.category = cur;
                        }
                        ui.end_row();

                        ui.label("名称 *");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.editor.name)
                                .hint_text("如：GitHub"),
                        );
                        ui.end_row();

                        ui.label("账号");
                        ui.add(egui::TextEdit::singleline(&mut self.editor.account));
                        ui.end_row();

                        ui.label("密码 *");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.editor.password)
                                    .password(!self.editor_show_pwd)
                                    .desired_width(180.0),
                            );
                            let toggle_label = if self.editor_show_pwd {
                                "隐藏"
                            } else {
                                "显示"
                            };
                            if ui.small_button(toggle_label).clicked() {
                                self.editor_show_pwd = !self.editor_show_pwd;
                            }
                            // 生成一串随机密码填入
                            if ui.button("🎲").on_hover_text("生成随机密码").clicked() {
                                if let Ok(p) = generator::generate(16, true, true, true, true) {
                                    self.editor.password = p;
                                }
                            }
                        });
                        ui.end_row();

                        ui.label("网址");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.editor.url)
                                    .hint_text("如：https://github.com")
                                    .desired_width(160.0),
                            );
                            if !self.editor.url.trim().is_empty() {
                                if ui.small_button("打开").on_hover_text("用默认浏览器打开").clicked() {
                                    let _ = open_url_in_browser(&self.editor.url);
                                }
                                if ui.small_button("复制").on_hover_text("复制网址到剪贴板").clicked() {
                                    let _ = clipboard::set_clipboard(&self.editor.url);
                                    self.show_status("网址已复制", false);
                                }
                            }
                        });
                        ui.end_row();

                        ui.label("手机号");
                        ui.add(egui::TextEdit::singleline(&mut self.editor.phone));
                        ui.end_row();

                        ui.label("备注");
                        ui.add(
                            egui::TextEdit::multiline(&mut self.editor.remark)
                                .desired_rows(2)
                                .desired_width(240.0),
                        );
                        ui.end_row();
                    });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("保存").clicked() {
                        self.save_editor();
                    }
                    if ui.button("取消").clicked() {
                        self.editing = false;
                    }
                });
                if !self.editor.password.is_empty() {
                    let (score, label, detail) = generator::strength(&self.editor.password);
                    ui.label(
                        RichText::new(format!("密码强度：{label}")).color(match score {
                            0..=1 => status_color(dark, true),
                            2 => Color32::YELLOW,
                            _ => status_color(dark, false),
                        }),
                    );
                    ui.label(RichText::new(detail).weak());
                }
            });
        self.editing = open_ok && self.editing;
        if !self.editing {
            self.editor = PlainRecord::default();
            self.editor_show_pwd = false;
        }
    }

    fn draw_generator_window(&mut self, ctx: &egui::Context) {
        let dark = self.theme == 1;
        let mut open_ok = true;
        egui::Window::new("密码生成器")
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open_ok)
            .collapsible(false)
            .resizable(false)
            .frame(popup_frame(ctx))
            .default_width(360.0)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                optimize_popup_fonts(ui);
                ui.horizontal(|ui| {
                    ui.label("长度");
                    ui.add(egui::Slider::new(&mut self.gen_len, 6..=32).suffix(" 位"));
                });
                ui.checkbox(&mut self.gen_upper, "大写字母 A-Z");
                ui.checkbox(&mut self.gen_lower, "小写字母 a-z");
                ui.checkbox(&mut self.gen_digit, "数字 0-9");
                ui.checkbox(&mut self.gen_special, "特殊字符 !@#...");
                ui.add_space(8.0);
                if ui.button("生成密码").clicked() {
                    match generator::generate(
                        self.gen_len,
                        self.gen_upper,
                        self.gen_lower,
                        self.gen_digit,
                        self.gen_special,
                    ) {
                        Ok(p) => {
                            self.gen_result = p;
                            let (_, _, detail) = generator::strength(&self.gen_result);
                            self.gen_strength = detail;
                        }
                        Err(e) => self.show_status(&e, true),
                    }
                }
                if !self.gen_result.is_empty() {
                    ui.add_space(8.0);
                    let (score, label, _) = generator::strength(&self.gen_result);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.gen_result)
                            .desired_width(320.0)
                            .font(egui::TextStyle::Monospace),
                    );
                    ui.label(RichText::new(format!("强度：{label}")).color(match score {
                        0..=1 => status_color(dark, true),
                        2 => Color32::YELLOW,
                        _ => status_color(dark, false),
                    }));
                    ui.label(RichText::new(&self.gen_strength).weak());
                    if ui.button("复制密码").clicked() {
                        self.copy_to_clipboard(&self.gen_result.clone(), "密码");
                    }
                }
                ui.add_space(6.0);
                if ui.button("关闭").clicked() {
                    self.show_generator = false;
                }
            });
        self.show_generator = open_ok && self.show_generator;
        if !self.show_generator {
            self.gen_result.clear();
            self.gen_strength.clear();
        }
    }

    fn draw_settings_window(&mut self, ctx: &egui::Context) {
        let mut open_ok = true;
        egui::Window::new("设置")
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open_ok)
            .collapsible(false)
            .frame(popup_frame(ctx))
            .default_width(400.0)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                optimize_popup_fonts(ui);
                section_title(ui, "常规", self.accent);
                ui.horizontal(|ui| {
                    ui.label("自动锁屏超时");
                    ui.add(egui::Slider::new(&mut self.lock_seconds, 30..=3600).suffix(" 秒"));
                });
                let mut req_login = self.require_login == 1;
                if ui.checkbox(&mut req_login, "启动时需要登录（关闭后直接进入主界面）").changed() {
                    self.require_login = if req_login { 1 } else { 0 };
                }
                ui.horizontal(|ui| {
                    ui.label("主题");
                    let mut t = self.theme;
                    egui::ComboBox::from_id_salt("theme_combo")
                        .selected_text(if t == 1 { "深色" } else { "浅色" })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut t, 0, "浅色");
                            ui.selectable_value(&mut t, 1, "深色");
                        });
                    if t != self.theme {
                        self.theme = t;
                    }
                });
                let mut autostart = autostart::get_autostart();
                if ui.checkbox(&mut autostart, "开机自启").changed() {
                    if let Err(e) = autostart::set_autostart(autostart) {
                        self.show_status(&e, true);
                    }
                }

                if ui.button("保存设置").clicked() {
                    match service::save_settings(
                        &self.conn,
                        self.lock_seconds,
                        self.theme,
                        self.require_login,
                    ) {
                        Ok(_) => self.show_status("设置已保存", false),
                        Err(e) => self.show_status(&e, true),
                    }
                }

                ui.separator();
                section_title(ui, "修改主密码", self.accent);
                ui.add(
                    egui::TextEdit::singleline(&mut self.change_pwd.0)
                        .password(true)
                        .hint_text("旧主密码"),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.change_pwd.1)
                        .password(true)
                        .hint_text("新主密码"),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.change_pwd.2)
                        .password(true)
                        .hint_text("确认新主密码"),
                );
                ui.add_space(6.0);
                ui.horizontal_centered(|ui| {
                    if ui.button("确认修改主密码").clicked() {
                        let (o, n, c) = self.change_pwd.clone();
                        if n != c {
                            self.show_status("两次新主密码不一致", true);
                        } else if n.len() < 4 {
                            self.show_status("新主密码至少 4 位", true);
                        } else {
                            match service::change_master_password(&mut self.conn, &o, &n) {
                                Ok(_) => {
                                    self.show_settings = false;
                                    self.change_pwd = (String::new(), String::new(), String::new());
                                    self.lock();
                                    self.show_status("主密码已修改，请重新解锁", false);
                                }
                                Err(e) => self.show_status(&e, true),
                            }
                        }
                    }
                });
            });
        self.show_settings = open_ok && self.show_settings;
        if !self.show_settings {
            self.change_pwd = (String::new(), String::new(), String::new());
        }
    }

    fn draw_confirm_delete(&mut self, ctx: &egui::Context) {
        let mut open_ok = true;
        egui::Window::new("确认删除")
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open_ok)
            .collapsible(false)
            .resizable(false)
            .default_width(320.0)
            .frame(popup_frame(ctx))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = SPACE_ROW;
                optimize_popup_fonts(ui);
                ui.label("确定要删除这条记录吗？此操作不可恢复。");
                ui.horizontal(|ui| {
                    let confirm_btn =
                        egui::Button::new(RichText::new("删除").color(Color32::WHITE).strong())
                            .fill(Color32::from_rgb(0xd1, 0x34, 0x38))
                            .stroke(Stroke::NONE)
                            .rounding(Rounding::same(ROUND_BTN as f32))
                            .min_size(egui::vec2(80.0, BTN_H));
                    if ui.add(confirm_btn).clicked() {
                        self.confirm_delete_do();
                    }
                    if ui
                        .add(egui::Button::new("取消").min_size(egui::vec2(80.0, BTN_H)))
                        .clicked()
                    {
                        self.confirm_delete = None;
                    }
                });
            });
        self.confirm_delete = if open_ok { self.confirm_delete } else { None };
    }

    fn draw_backup_window(&mut self, ctx: &egui::Context) {
        let mut open_ok = true;
        egui::Window::new("备份 / 恢复")
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open_ok)
            .collapsible(false)
            .frame(popup_frame(ctx))
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                optimize_popup_fonts(ui);
                section_title(ui, "导出备份 (.pwbackup)", self.accent);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.backup_export_path)
                            .hint_text("备份文件路径")
                            .desired_width(280.0),
                    );
                    if ui.button("浏览…").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("备份", &["pwbackup"])
                            .set_file_name("password_backup.pwbackup")
                            .save_file()
                        {
                            self.backup_export_path = p.to_string_lossy().into_owned();
                        }
                    }
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.backup_export_pwd)
                        .password(true)
                        .hint_text("输入主密码以加密备份"),
                );
                if ui.button("导出备份").clicked() {
                    self.do_export();
                }

                ui.separator();
                section_title(ui, "恢复备份", self.accent);
                ui.horizontal(|ui| {
                    if ui.button("选择备份文件…").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("备份", &["pwbackup"])
                            .pick_file()
                        {
                            self.backup_import_path = p.to_string_lossy().into_owned();
                        }
                    }
                    if !self.backup_import_path.is_empty() {
                        ui.label(RichText::new(&self.backup_import_path).weak());
                    }
                });
                ui.checkbox(&mut self.merge_mode, "合并到现有数据（取消则覆盖）");
                ui.add(
                    egui::TextEdit::singleline(&mut self.backup_import_pwd)
                        .password(true)
                        .hint_text("输入主密码以解密备份"),
                );
                if ui.button("恢复备份").clicked() {
                    self.do_import();
                }

                ui.add_space(6.0);
                if ui.button("关闭").clicked() {
                    self.show_backup = false;
                }
            });
        self.show_backup = open_ok && self.show_backup;
        if !self.show_backup {
            self.backup_export_pwd.clear();
            self.backup_import_pwd.clear();
        }
    }

    fn do_export(&mut self) {
        let key = match self.key.as_ref() {
            Some(k) => k.clone(),
            None => {
                self.show_status("会话已锁定", true);
                return;
            }
        };
        if self.backup_export_path.trim().is_empty() {
            self.show_status("请选择备份路径", true);
            return;
        }
        if self.backup_export_pwd.is_empty() {
            self.show_status("请输入主密码", true);
            return;
        }
        // 导出前先校验主密码正确，避免用错误密码生成无法恢复的备份
        if service::unlock(&self.conn, &self.backup_export_pwd).is_err() {
            self.show_status("主密码错误，备份未导出", true);
            return;
        }
        let records = match service::list_records(&self.conn, &key) {
            Ok(r) => r,
            Err(e) => {
                self.show_status(&e, true);
                return;
            }
        };
        let cfg = match db::get_config(&self.conn) {
            Ok(c) => c,
            Err(e) => {
                self.show_status(&e, true);
                return;
            }
        };
        let salt = match crate::crypto::hex_decode(&cfg.salt) {
            Ok(s) => s,
            Err(e) => {
                self.show_status(&e, true);
                return;
            }
        };
        match backup::export_backup(&records, &self.backup_export_pwd, &salt) {
            Ok(bytes) => match std::fs::write(&self.backup_export_path, bytes) {
                Ok(_) => {
                    self.show_status("备份已导出", false);
                    self.backup_export_pwd.clear();
                }
                Err(e) => self.show_status(&format!("写入文件失败: {e}"), true),
            },
            Err(e) => self.show_status(&e, true),
        }
    }

    fn do_import(&mut self) {
        if self.backup_import_path.trim().is_empty() {
            self.show_status("请选择备份文件", true);
            return;
        }
        if self.backup_import_pwd.is_empty() {
            self.show_status("请输入主密码", true);
            return;
        }
        let data = match std::fs::read(&self.backup_import_path) {
            Ok(d) => d,
            Err(e) => {
                self.show_status(&format!("读取备份失败: {e}"), true);
                return;
            }
        };
        let records = match backup::import_backup(&data, &self.backup_import_pwd) {
            Ok(r) => r,
            Err(e) => {
                self.show_status(&e, true);
                return;
            }
        };
        let key = match self.key.as_ref() {
            Some(k) => k.clone(),
            None => {
                self.show_status("会话已锁定", true);
                return;
            }
        };
        match service::restore_records(&mut self.conn, &key, &records, self.merge_mode) {
            Ok(_) => {
                self.show_status("恢复完成", false);
                self.backup_import_pwd.clear();
            }
            Err(e) => self.show_status(&e, true),
        }
        if let Err(e) = self.refresh_records() {
            self.show_status(&format!("刷新记录失败: {e}"), true);
        }
    }

    fn draw_status_bar(&mut self, ui: &mut egui::Ui) {
        if let Some(at) = self.toast_at {
            if Instant::now() > at {
                self.status.clear();
                self.toast_at = None;
            }
        }
        ui.separator();
        ui.horizontal(|ui| {
            let shown = self.records.len();
            ui.label(format!("共 {shown} 条记录"));
            ui.separator();
            if !self.status.is_empty() {
                let dark = self.theme == 1;
                let color = status_color(dark, self.status_is_error);
                ui.colored_label(color, &self.status);
            }
        });
    }

    fn update_auto_lock_and_clipboard(&mut self) {
        // 空闲自动锁屏：last_interaction 每帧由 update() 里的交互检测刷新
        if self.unlocked && self.lock_seconds > 0 {
            if self.last_interaction.elapsed() >= Duration::from_secs(self.lock_seconds as u64) {
                self.lock();
                self.show_status("因闲置自动锁定", false);
            }
        }
    }
}

impl eframe::App for PasswordBookApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let t = ctx.input(|i| i.time) as f32;
        let dark = self.unlocked && self.theme == 1;
        // 动态霓虹主色 + 渐变背景（仅主界面，锁定登录窗保持干净底色）
        // 主色：使用 Windows 系统主题强调色，并据此刷新各处的控件配色
        self.accent = self.sys_accent;
        if self.unlocked {
            paint_gradient_background(ctx, dark, t);
        }
        install_apple_theme(ctx, dark, self.accent);

        // ---- 系统托盘与关闭拦截 ----
        // 托盘线程在独立线程里直接处理事件（FindWindowW + ShowWindow / process::exit），
        // 不依赖 eframe 的 update 循环。这里只负责同步 eframe 的 viewport 状态。
        //
        // 注意：不再让窗口 Visible(false) 退托盘（那会让 eframe 停止 update 循环）。
        // 关闭时改为最小化窗口，托盘线程随时可以把它 ShowWindow 回来。

        // 消费托盘线程设的 external_show_pending：它已经 ShowWindow 了主窗口，
        // 我们需要告诉 eframe 同步 visible 状态并聚焦。
        tray::poll_tray_events(&self.tray);
        if self
            .tray
            .external_show_pending
            .swap(false, Ordering::Relaxed)
        {
            self.closing = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        // 拦截窗口 X 关闭：退到托盘（用最小化而非隐藏，保持 update 循环运行）
        let close_pending = ctx.input(|i| i.viewport().close_requested());
        if close_pending && !self.closing {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }

        // 交互检测刷新最后活跃时间
        let interacted = ctx.input(|i| {
            i.pointer.any_down()
                || i.pointer.button_pressed(egui::PointerButton::Primary)
                || !i.keys_down.is_empty()
                || i.pointer.button_pressed(egui::PointerButton::Secondary)
        });
        if interacted {
            self.last_interaction = Instant::now();
        }
        self.update_auto_lock_and_clipboard();

        // 锁定态缩成刚好容纳登录表单的小窗，解锁后放成完整主界面
        let want_large = self.unlocked;
        if want_large != self.viewport_large {
            self.viewport_large = want_large;
            let size = if want_large {
                egui::vec2(920.0, 640.0)
            } else if self.need_setup {
                egui::vec2(340.0, 330.0)
            } else {
                egui::vec2(340.0, 210.0)
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            // 无论放大还是缩小，都重新居中于屏幕中央
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(center_position(
                size.x, size.y,
            )));
            // 锁定登录窗无标题栏，解锁后恢复标题栏
            ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(want_large));
        }

        // 仅在存在 toast 倒计时时持续重绘，避免每帧空转；
        // 用户交互本身会触发重绘，闲置自动锁定会在最后一次交互帧上判定。
        if self.toast_at.is_some() {
            ctx.request_repaint();
        }

        if !self.unlocked {
            self.draw_unlock(ctx);
        } else {
            self.draw_main(ctx);
        }
    }
}

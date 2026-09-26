<div align="center">

# 🔒 密码本 PasswordBook

**纯本地 · 离线加密 · Windows 密码管理工具**

[![Rust](https://img.shields.io/badge/Rust-1.75%2B-CE422B?style=for-the-badge&logo=rust)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg?style=for-the-badge)](LICENSE)
[![OS](https://img.shields.io/badge/Windows-10%2F11-0078D4?style=for-the-badge&logo=windows)](https://www.microsoft.com/windows)
[![Release](https://img.shields.io/github/v/release/jeokeo011222/passwordbook?style=for-the-badge&color=brightgreen)](https://github.com/jeokeo011222/passwordbook/releases)
[![Downloads](https://img.shields.io/github/downloads/jeokeo011222/passwordbook/total?style=for-the-badge)](https://github.com/jeokeo011222/passwordbook/releases)

**零网络请求 · 零账号注册 · 零数据上传** · 所有密码用 **AES-256-GCM** 加密存本机

[✨ 功能特性](#-功能特性) · [🚀 快速开始](#-快速开始) · [🛡 安全架构](#-安全架构) · [📥 下载安装](#-下载安装) · [🛠 构建](#-构建) · [❓ FAQ](#-常见问题)

---

![screenshot](logo2.png)

</div>

---

## ✨ 功能特性

### 🔐 安全核心
- **AES-256-GCM** 对称加密（业界标准）
- **PBKDF2-SHA256** 密钥派生（60 万次迭代，抗暴力破解）
- **每条记录独立 Nonce**，相同明文密文不同
- **解锁态内存安全**：敏感缓冲区自动擦除
- **主密码强度校验** + 错误锁定机制

### 📋 记录管理
- 自定义分类（工作 / 生活 / 购物 / 金融…）
- 软删除回收站，可恢复 / 彻底删除
- 网址字段一键打开浏览器 / 复制链接
- 备注字段支持多行富文本

### 🛠 工具箱
| 工具 | 说明 |
|------|------|
| **密码生成器** | 可配置长度 8-128、字符集、排除易混字符 |
| **备份 / 恢复** | 导出加密 `.pbk` 文件（主密码保护） |
| **CSV 导入** | 一键导入 Chrome / Edge / Firefox 导出 |
| **安全检查** | 弱口令扫描、重复口令检测、空密码提醒 |

### 🪟 系统集成
- 关闭退托盘常驻，**零打扰**
- 托盘菜单：显示主程序 / 退出
- 可选 **开机自启**（注册表 Run 项）
- **空闲自动锁定**（可配置超时秒数）
- Windows 系统主题色自动跟随
- 深色 / 浅色主题切换

### � 数据保护
- SQLite 数据库完整性校验
- 所有写入操作包裹事务，失败自动回滚
- 防断电损坏（事务 journal 模式）
- 改密 / 恢复备份全程原子操作

---

## 🚀 快速开始

### 首次启动

```
双击 passwordbook.exe
    │
    ▼
首次启动 → 设置主密码（⚠️ 丢失无法找回！）
    │
    ▼
主界面 → 点左侧「＋ 新增」→ 添加第一条密码
```

### 主界面一览

```
┌──────────────────────────────────────────────────────┐
│ [🔍 搜索框] [清除]           [🔒] [⚙] [ⓘ]          │
├─────────┬────────────────────────────────────────────┤
│ ＋ 新增 │                                            │
│         │                                            │
│ ▼ 分类  │                                            │
│ 全部 12 │         📋 密码记录表格                    │
│ 常用 5  │         ┌─────┬──────┬──────┬────┐        │
│ 购物 3  │         │ 名称│ 账号 │ 密码 │网址│        │
│         │         ├─────┼──────┼──────┼────┤        │
│ ▼ 工具  │         │ Gmail│a*** │•••• │🔗 │        │
│ 备份/恢复│         │ 银行 │1*** │•••• │🔗 │        │
│ 密码生成器│        │ ... │ ...  │ ...  │... │        │
│ CSV 导入 │         └─────┴──────┴──────┴────┘        │
│ 安全检查 │                                            │
│ 回收站  │                                            │
└─────────┴────────────────────────────────────────────┘
```

### 常用操作速查

| 想做什么 | 怎么操作 |
|----------|----------|
| 新增密码 | 左侧 **「＋ 新增」** |
| 编辑记录 | 表格行 **「编辑」** 按钮 |
| 删除记录 | 表格行 **「删除」** （移回收站） |
| 彻底删除 | 左侧 **「回收站」** → 选中 → 彻底删除 |
| 搜索密码 | 顶部搜索框实时过滤 |
| 打开网址 | 网址列的 **🔗** 按钮 |
| 一键锁定 | 右上角 **🔒** 图标 |
| 修改主密码 | ⚙ 设置 → 修改主密码 |
| 导出备份 | 左侧 **备份/恢复** → 导出（需验证主密码） |
| 浏览器导入 | 左侧 **CSV 导入** → 选择 Chrome/Edge 导出的 CSV |
| 最小化托盘 | 点窗口 **×** 关闭按钮 |

---

## � 安全架构

### 加密流程

```
  主密码（用户输入）
       │
       ▼
  ┌─────────────────┐
  │ PBKDF2-SHA256    │  600,000 次迭代
  │ salt = 16 bytes  │  每条记录独立 salt ❗
  └─────────────────┘
       │ 32-byte 密钥
       ▼
  ┌─────────────────┐
  │ AES-256-GCM     │  对称加密 + 认证
  │ nonce = 12 bytes │  每条记录独立 nonce ❗
  └─────────────────┘
       │
       ▼
  存储格式：[12B nonce][密文+16B auth tag][base64]
```

> **为什么每条记录独立 salt 和 nonce？**
> 相同的主密码 + 相同的明文 → 加密后密文完全不同。防止彩虹表攻击，防止"看起来相同"泄露信息。

### 主密码丢了怎么办？

**无法找回。** 这是有意设计——如果程序能帮你找回主密码，意味着主密码被存在某个地方，加密就白做了。

✅ **建议**：定期用 **备份** 功能导出 `.pbk` 文件到 U 盘 / 云盘，备份文件也用同一个主密码保护。

### 数据存哪了？

```
%APPDATA%\passwordbook\
├── password.db        # SQLite 加密数据库（可直接复制备份）
└── tray_debug.log     # 托盘运行日志（不含敏感数据）
```

### 安全设计清单

- [x] AES-256-GCM 认证加密（NIST 推荐）
- [x] PBKDF2 60 万次迭代（OWASP 2024 标准）
- [x] 零明文驻留内存（锁定态自动擦除）
- [x] 数据库事务包裹所有写操作
- [x] 错误密码锁定（30 秒冷却）
- [x] 备份/恢复前强制验证主密码
- [x] 日志不包含任何敏感数据
- [x] 无网络请求（纯离线）

---

## � 下载安装

### 最新 Release

👉 前往 [Releases 页面](https://github.com/jeokeo011222/passwordbook/releases) 下载最新版本

| 文件 | 说明 |
|------|------|
| `passwordbook-v1.0.0-release.zip` | Windows x64，解压即用，无需安装 |

### 系统要求

| 项目 | 要求 |
|------|------|
| 操作系统 | Windows 10 / 11 (x64) |
| 依赖 | **无** — 单文件 exe，双击运行 |
| 磁盘空间 | ~7 MB |

---

## 🛠 构建

### 从源码构建

```bash
# 1. 克隆仓库
git clone https://github.com/jeokeo011222/passwordbook.git
cd passwordbook

# 2. 安装 Rust（如果还没装）
# https://rustup.rs/

# 3. Release 构建
cargo build --release

# 4. 产物
target/release/passwordbook.exe
```

> 构建会自动：
> - 程序化生成 PNG → ICO → 嵌入 exe 图标
> - 通过 winresource 注入版本信息（FileDescription / ProductName / FileVersion / Copyright / 语言）
> - 从 `logo2.png` 预解码 RGBA 供 tray-icon 和 egui 窗口图标复用

### 技术栈

| 组件 | 技术 | 作用 |
|------|------|------|
| GUI | egui / eframe 0.30 | 即时模式 Rust GUI |
| 数据库 | SQLite (rusqlite 0.31) | 嵌入式数据库 |
| 加密 | aes-gcm 0.10 + pbkdf2 0.12 | AES-256-GCM + PBKDF2 |
| 托盘 | tray-icon 0.19 | 系统托盘常驻 |
| 剪贴板 | arboard 3.3 | 跨平台剪贴板 |
| CSV | csv 1.3 | Chrome/Edge 导入 |
| 资源注入 | winresource | exe 图标 + 版本信息 |
| 构建 | Cargo + build.rs | 自动图标打包 |

---

## 🤝 贡献

欢迎 Issue / PR！

```bash
git clone https://github.com/jeokeo011222/passwordbook.git
cd passwordbook
cargo run          # 开发模式运行（带 cmd 黑窗，方便看日志）
cargo test         # 运行 28 个单元测试
cargo build --release  # Release 构建
```

---

## ❓ 常见问题

<details>
<summary><b>Q: 忘记主密码怎么办？</b></summary>
<br>
无法找回。请用之前导出的 `.pbk` 备份文件恢复（备份也需要主密码解密）。
</details>

<details>
<summary><b>Q: 数据会上传到云端吗？</b></summary>
<br>
<b>绝对不会。</b> 程序是纯离线的，启动后零网络请求。所有密码在你自己电脑上。
</details>

<details>
<summary><b>Q: 支持 macOS / Linux 吗？</b></summary>
<br>
当前仅 Windows。UI 框架 eframe 跨平台，但托盘、开机自启等绑定 Windows API。
</details>

<details>
<summary><b>Q: 数据库可以手动迁移到另一台电脑吗？</b></summary>
<br>
可以。关闭程序后，把 `%APPDATA%\passwordbook\password.db` 复制到新电脑同一位置即可（主密码不变）。也可以用「备份/恢复」功能。
</details>

<details>
<summary><b>Q: 为什么没有浏览器扩展 / 自动填充？</b></summary>
<br>
浏览器扩展 Native Messaging 协议各浏览器实现不同，设计复杂且安全风险高。目前保持简洁可靠。
</details>

<details>
<summary><b>Q: exe 被杀软拦截？</b></summary>
<br>
如果是从本仓库 Release 下载的 exe，可以放心。Rust 编写、无网络请求、无可疑行为。某些杀软对未签名 exe 有启发式拦截是正常现象。
</details>

---

## 📄 License

[MIT License](LICENSE) — 自由使用、修改、分发

---

<div align="center">
<sub>Built with ❤️ by <a href="https://github.com/jeokeo011222">Jeokeo011222</a> in Rust</sub>
</div>

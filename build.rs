//! Windows PE 资源注入：把图标 + 版本信息嵌入 exe。
//! 构建时由 cargo 自动调用（根目录 build.rs）。

fn main() {
    // ---- 1. 从项目根目录读取 logo2.png ----
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let logo_path = std::path::Path::new(&manifest_dir).join("logo2.png");
    let png_bytes = std::fs::read(&logo_path)
        .unwrap_or_else(|_| panic!("找不到 logo2.png：{}", logo_path.display()));

    // ---- 2. PNG -> ICO（Windows Vista+ 原生支持 PNG payload） ----
    let ico = pack_png_as_ico(&png_bytes);
    let out_dir = std::env::var("OUT_DIR").unwrap_or_else(|_| ".".into());
    let ico_path = std::path::Path::new(&out_dir).join("app.ico");
    std::fs::write(&ico_path, &ico).expect("ico write");

    // ---- 3. 解码 PNG 为 RGBA raw bytes，供 tray.rs include_bytes! ----
    // 格式：前 8 字节 = [width:u32 BE][height:u32 BE]，后面是 RGBA 像素流
    let (w, h, rgba) = decode_png_rgba(&png_bytes);
    let mut raw: Vec<u8> = Vec::with_capacity(8 + rgba.len());
    raw.extend_from_slice(&w.to_be_bytes());
    raw.extend_from_slice(&h.to_be_bytes());
    raw.extend_from_slice(&rgba);
    std::fs::write(std::path::Path::new(&out_dir).join("logo_rgba.raw"), &raw).expect("raw write");

    // ---- 4. winresource 注入 exe 图标 + 版本信息 ----
    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico_path.to_str().expect("ico path"));
    res.set("FileVersion", "1.0.0.0");
    res.set("ProductVersion", "1.0.0.0");
    res.set("FileDescription", "本地离线加密密码本");
    res.set("ProductName", "密码本");
    res.set("CompanyName", "QQ 3571038944");
    res.set("LegalCopyright", "版权所有 侵权必究");
    res.set("OriginalFilename", "passwordbook.exe");
    res.set("InternalName", "PasswordBook");
    res.set_language(0x0804); // 简体中文
    if let Err(e) = res.compile() {
        panic!("winresource compile error: {e}");
    }

    println!("cargo:rerun-if-changed=logo2.png");
}

// ---------------------------------------------------------------------------
// PNG -> ICO 包装
// ---------------------------------------------------------------------------

fn pack_png_as_ico(png: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());

    let (w, h) = read_png_size(png);
    let w_byte: u8 = if w >= 256 { 0 } else { w as u8 };
    let h_byte: u8 = if h >= 256 { 0 } else { h as u8 };

    let data_offset: u32 = 6 + 16;
    out.push(w_byte);
    out.push(h_byte);
    out.push(0);
    out.push(0);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&(png.len() as u32).to_le_bytes());
    out.extend_from_slice(&data_offset.to_le_bytes());
    out.extend_from_slice(png);
    out
}

fn read_png_size(png: &[u8]) -> (u32, u32) {
    if png.len() < 24 {
        return (64, 64);
    }
    let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    (w, h)
}

// ---------------------------------------------------------------------------
// PNG 解码（用 png crate）
// ---------------------------------------------------------------------------

fn decode_png_rgba(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    use png::{BitDepth, ColorType, Decoder};

    let decoder = Decoder::new(png_bytes);
    let mut reader = decoder.read_info().expect("png: read_info");
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("png: next_frame");

    let w = info.width;
    let h = info.height;

    // 统一转成 RGBA
    let rgba = match (info.color_type, info.bit_depth) {
        (ColorType::Rgba, BitDepth::Eight) => buf,
        (ColorType::Rgb, BitDepth::Eight) => {
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for px in buf.chunks_exact(3) {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        }
        (ColorType::Grayscale, BitDepth::Eight) => {
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for g in &buf {
                out.extend_from_slice(&[*g, *g, *g, 255]);
            }
            out
        }
        _ => {
            // fallback: 直接返回，让调用方处理
            buf
        }
    };

    (w, h, rgba)
}

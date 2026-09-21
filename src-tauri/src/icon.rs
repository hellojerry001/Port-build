use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::projects;

/* ============================== 读当前图标 ============================== */

/// 读取 Mac 项目当前启动图标（`src-tauri/icons/icon.png`）的 base64，供前端做缩略图。
/// 没有图标文件时返回 `Ok(None)`，前端显示占位。
#[tauri::command]
pub fn project_icon(id: String) -> Result<Option<String>, String> {
    let p = projects::find(&id).ok_or("项目不存在")?;
    let path = Path::new(&p.path).join("src-tauri/icons/icon.png");
    match fs::read(&path) {
        Ok(b) => Ok(Some(b64(&b))),
        Err(_) => Ok(None),
    }
}

/* ============================== 一键换图标 ============================== */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IconSwapResult {
    /// 被覆盖的源码图标目录
    pub icon_dir: String,
    /// 实际生成的图标文件清单
    pub sizes: Vec<String>,
    /// 顺手热更新的已构建 .app 路径；没找到内置构建产物时为 None
    pub hot_patched: Option<String>,
    pub note: String,
}

/// 一键换 Mac 项目启动图标：
/// 1. 任意图片经 sips 居中裁方 + 缩放成 1024² 主图
/// 2. 派生各尺寸 PNG、按 .icns 容器格式手拼 icns、手写 ico
/// 3. 覆盖 `src-tauri/icons/`，并尽力热更新 `target/.../bundle/macos/` 里的 .app
#[tauri::command]
pub fn swap_icon(id: String, image_path: String) -> Result<IconSwapResult, String> {
    let p = projects::find(&id).ok_or("项目不存在")?;
    let (sizes, hot) = generate_icons(&p.path, &image_path)?;
    let icon_dir = Path::new(&p.path).join("src-tauri/icons");
    let note = match &hot {
        Some(app) => format!("已热更新已构建的 .app：{}", app),
        None => "已更新源码图标，重新打包即生效".into(),
    };
    Ok(IconSwapResult {
        icon_dir: icon_dir.to_string_lossy().to_string(),
        sizes,
        hot_patched: hot,
        note,
    })
}

/// 纯逻辑：给定项目根目录与源图，生成整套图标并热更新已构建的 .app。
/// 与全局项目表解耦，便于单测。
fn generate_icons(project_path: &str, image_path: &str) -> Result<(Vec<String>, Option<String>), String> {
    let icon_dir = Path::new(project_path).join("src-tauri/icons");
    fs::create_dir_all(&icon_dir).map_err(|e| e.to_string())?;

    let tmp = std::env::temp_dir().join(format!("pb-icon-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;

    // 1) 任意格式 → png；2) 居中裁方 + 缩放成 1024² 主图
    let conv = tmp.join("conv.png");
    let master = icon_dir.join("icon.png");
    sips(&[
        "-s".into(),
        "format".into(),
        "png".into(),
        image_path.to_string(),
        "--out".into(),
        conv.to_string_lossy().into_owned(),
    ])?;
    sips(&[
        "-c".into(),
        "1024".into(),
        "1024".into(),
        conv.to_string_lossy().into_owned(),
        "--out".into(),
        master.to_string_lossy().into_owned(),
    ])?;

    // 派生 Tauri bundle.icon 读取的那几个 PNG
    resize_copy(&master, &icon_dir.join("32x32.png"), 32)?;
    resize_copy(&master, &icon_dir.join("128x128.png"), 128)?;
    resize_copy(&master, &icon_dir.join("128x128@2x.png"), 256)?;

    // 生成 icns：把各尺寸 PNG 直接按 .icns 容器格式拼装（现代 .icns 用 PNG 数据，
    // 不依赖 iconutil——本机 iconutil 对 sips 产出的 PNG 会无差别报 “Failed to generate ICNS”）
    let icns = build_icns(&icon_dir, &master)?;

    // 手写 ico（用 256 PNG 当容器，Tauri 在 Windows 构建时读）
    let png256 = read_resize(&master, 256)?;
    write_ico(&png256, &icon_dir.join("icon.ico"))?;

    let _ = fs::remove_dir_all(&tmp);

    // 尽力热更新已构建的 .app（免去重新打包）
    let hot = hot_patch_built_app(project_path, &icns);

    let sizes = vec![
        "icon.png (1024)".into(),
        "32x32.png".into(),
        "128x128.png".into(),
        "128x128@2x.png".into(),
        "icon.icns".into(),
        "icon.ico".into(),
    ];
    Ok((sizes, hot))
}

fn sips(args: &[String]) -> Result<(), String> {
    let mut cmd = Command::new("/usr/bin/sips");
    for a in args {
        cmd.arg(a);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("sips 执行失败：{e}"))?;
    if !out.status.success() {
        return Err(format!("sips 失败：{}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(())
}

fn resize_copy(master: &Path, out: &Path, size: u32) -> Result<(), String> {
    let s = size.to_string();
    sips(&[
        "-z".into(),
        s.clone(),
        s,
        master.to_string_lossy().into_owned(),
        "--out".into(),
        out.to_string_lossy().into_owned(),
    ])
}

fn read_resize(master: &Path, size: u32) -> Result<Vec<u8>, String> {
    let tmp = std::env::temp_dir().join(format!("pb-ico-{}-{}.png", std::process::id(), size));
    resize_copy(master, &tmp, size)?;
    fs::read(&tmp).map_err(|e| e.to_string())
}

/// 直接拼装 .icns 容器：magic `icns` + 各尺寸 PNG 条目（OSType + 长度 + PNG 数据）。
/// 现代 .icns 的 icp4/icp5/icp6/ic07..ic10 都是直接存 PNG，无需 iconutil。
fn build_icns(dir: &Path, master: &Path) -> Result<PathBuf, String> {
    let sizes: &[(u32, &[u8; 4])] = &[
        (16, b"icp4"),
        (32, b"icp5"),
        (64, b"icp6"),
        (128, b"ic07"),
        (256, b"ic08"),
        (512, b"ic09"),
        (1024, b"ic10"),
    ];
    let mut body = Vec::new();
    for (size, code) in sizes {
        let png = read_resize(master, *size)?;
        let mut entry = Vec::with_capacity(png.len() + 8);
        entry.extend_from_slice(*code);
        entry.extend_from_slice(&((png.len() as u32) + 8).to_be_bytes());
        entry.extend_from_slice(&png);
        body.extend_from_slice(&entry);
    }
    let total = (body.len() as u32) + 8;
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&total.to_be_bytes());
    out.extend_from_slice(&body);
    let path = dir.join("icon.icns");
    fs::write(&path, &out).map_err(|e| e.to_string())?;
    Ok(path)
}

/// ICO 容器：ICONDIR + ICONDIRENTRY + PNG 数据（256 尺寸的宽高字节用 0 表示）
fn write_ico(png: &[u8], out: &Path) -> Result<(), String> {
    use std::io::Write;
    let mut f = fs::File::create(out).map_err(|e| e.to_string())?;
    f.write_all(&[0, 0, 1, 0, 1, 0]).map_err(|e| e.to_string())?; // reserved, type=1, count=1
    f.write_all(&[0, 0, 0, 0]).map_err(|e| e.to_string())?; // width, height, colorCount, reserved
    f.write_all(&[1, 0, 32, 0]).map_err(|e| e.to_string())?; // planes=1, bitCount=32
    f.write_all(&(png.len() as u32).to_le_bytes())
        .map_err(|e| e.to_string())?; // bytesInRes
    f.write_all(&22u32.to_le_bytes()).map_err(|e| e.to_string())?; // imageOffset
    f.write_all(png).map_err(|e| e.to_string())?;
    Ok(())
}

fn hot_patch_built_app(project_path: &str, icns: &Path) -> Option<String> {
    let macos_dir = Path::new(project_path).join("src-tauri/target/release/bundle/macos");
    let entries = fs::read_dir(&macos_dir).ok()?;
    let app: Option<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().map(|x| x == "app").unwrap_or(false));
    let app = app?;

    let res = app.join("Contents/Resources");
    let _ = fs::create_dir_all(&res);
    let _ = fs::copy(icns, res.join("icon.icns"));
    // 改了资源会让 ad-hoc 签名失效，重新签一次；否则 macOS 可能拒绝启动
    let _ = Command::new("codesign")
        .arg("--force")
        .arg("--deep")
        .arg("--sign")
        .arg("-")
        .arg(&app)
        .output();
    let _ = Command::new("touch").arg(&app).output();
    Some(app.to_string_lossy().to_string())
}

/* ============================== base64（不引第三方 crate） ============================== */

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn b64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[((n >> 18) & 63) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            B64[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_encodes_known_vector() {
        assert_eq!(b64(b"Man"), "TWFu");
        assert_eq!(b64(b"Ma"), "TWE=");
        assert_eq!(b64(b"M"), "TQ==");
        assert_eq!(b64(b"PortButler"), "UG9ydEJ1dGxlcg==");
    }

    /// 端到端：给一个带已构建 .app 的 Mac 工程换图标，应生成整套图标并热更新 .app。
    /// 源图用仓库自带的有效 PNG（CARGO_MANIFEST_DIR/icons/icon.png），链路真实可跑。
    #[test]
    fn generate_icons_builds_set_and_hot_patches_app() {
        let manifest = env!("CARGO_MANIFEST_DIR");
        let real_icon = Path::new(manifest).join("icons/icon.png");
        if !real_icon.is_file() {
            eprintln!("跳过：缺少真实源图 {}", real_icon.display());
            return;
        }

        let base = std::env::temp_dir().join(format!("pb-icontest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let proj = base.join("MyMac");
        fs::create_dir_all(proj.join("src-tauri/icons")).unwrap();
        let app_res = proj.join("src-tauri/target/release/bundle/macos/MyMac.app/Contents/Resources");
        fs::create_dir_all(&app_res).unwrap();

        let (sizes, hot) = generate_icons(proj.to_str().unwrap(), real_icon.to_str().unwrap())
            .expect("生成图标应成功");

        assert!(proj.join("src-tauri/icons/icon.icns").is_file(), "应生成 icon.icns");
        assert!(proj.join("src-tauri/icons/icon.ico").is_file(), "应生成 icon.ico");
        assert!(proj.join("src-tauri/icons/32x32.png").is_file(), "应生成 32x32.png");
        assert!(sizes.iter().any(|s| s.contains("icns")), "返回清单含 icns");
        // 带已构建 .app → 应热更新
        assert!(hot.is_some(), "应定位到已构建的 .app 并热更新");
        assert!(
            app_res.join("icon.icns").is_file(),
            "已构建 .app 的 Resources/icon.icns 应被更新"
        );
        // 覆盖一份应该和源码 icon.icns 内容一致
        let src = fs::read(proj.join("src-tauri/icons/icon.icns")).unwrap();
        let dst = fs::read(app_res.join("icon.icns")).unwrap();
        assert_eq!(src, dst, "热更新拷贝的 icns 应与源码一致");

        // 手拼的 .icns 必须真被 macOS 认（sips 能读出 1024 宽）。
        // 这条是防「格式写歪了但字节都在」——纯字节断言看不出来。
        #[cfg(target_os = "macos")]
        {
            let out = Command::new("/usr/bin/sips")
                .arg("-g")
                .arg("pixelWidth")
                .arg(proj.join("src-tauri/icons/icon.icns"))
                .output()
                .expect("sips 应可执行");
            assert!(
                out.status.success(),
                "macOS 应能解析手拼的 icon.icns：{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let s = String::from_utf8_lossy(&out.stdout);
            assert!(s.contains("1024"), "icns 应含 1024 尺寸，实际：{s}");
        }

        let _ = fs::remove_dir_all(&base);
    }
}

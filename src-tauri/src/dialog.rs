use std::process::Command;

use crate::scaffolds::expand_tilde;

/// 转义 AppleScript 字符串字面量里的 `\` 与 `"`
fn esc_as(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// 拼装 `choose folder` 脚本。
/// 默认定位目录必须真实存在，否则 osascript 会直接报错，所以这里先做一次校验。
fn build_script(prompt: &str, default_path: Option<&str>) -> String {
    let prompt = match prompt.trim() {
        "" => "选择文件夹",
        p => p,
    };
    let mut script = format!(
        "POSIX path of (choose folder with prompt \"{}\"",
        esc_as(prompt)
    );
    if let Some(raw) = default_path.map(str::trim).filter(|d| !d.is_empty()) {
        let dir = expand_tilde(raw);
        if dir.is_dir() {
            script.push_str(&format!(
                " default location POSIX file \"{}\"",
                esc_as(&dir.to_string_lossy())
            ));
        }
    }
    script.push(')');
    script
}

/// 调起 macOS 原生目录选择器（Standard Additions 的 `choose folder`，
/// 底层即 NSOpenPanel 的目录选择模式），返回去掉尾斜杠的 POSIX 绝对路径。
/// 用户点取消返回 `Ok(None)`。
fn pick_folder_blocking(
    prompt: Option<String>,
    default_path: Option<String>,
) -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    {
        let script = build_script(
            prompt.as_deref().unwrap_or_default(),
            default_path.as_deref(),
        );

        let out = Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| format!("无法调起系统目录选择器：{e}"))?;

        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            // -128 = 用户取消（User canceled）
            if err.contains("-128") || err.to_lowercase().contains("cancel") {
                return Ok(None);
            }
            return Err(format!("目录选择器出错：{}", err.trim()));
        }

        let picked = String::from_utf8_lossy(&out.stdout).trim().to_string();
        // `choose folder` 结果带尾斜杠，去掉；根目录 "/" 保持原样
        let picked = if picked.len() > 1 {
            picked.trim_end_matches('/').to_string()
        } else {
            picked
        };
        if picked.is_empty() {
            return Ok(None);
        }
        Ok(Some(picked))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (prompt, default_path);
        Err("原生目录选择器目前仅支持 macOS".to_string())
    }
}

/// 弹出 macOS 原生目录选择器，把用户选中的文件夹路径回传给前端。
#[tauri::command]
pub async fn pick_folder(
    prompt: Option<String>,
    default_path: Option<String>,
) -> Result<Option<String>, String> {
    // osascript 会阻塞等用户操作，丢到阻塞线程池，避免卡住主线程
    tauri::async_runtime::spawn_blocking(move || pick_folder_blocking(prompt, default_path))
        .await
        .map_err(|e| format!("目录选择器执行失败：{e}"))?
}

/* ============================== 文件选择器（换图标用） ============================== */

fn ext_to_uti(ext: &str) -> String {
    match ext.to_ascii_lowercase().as_str() {
        "png" => "public.png".into(),
        "jpg" | "jpeg" => "public.jpeg".into(),
        "heic" => "public.heic".into(),
        "tiff" | "tif" => "public.tiff".into(),
        "gif" => "public.gif".into(),
        // zip 的 UTI 不叫 public.zip：AppleScript 拿到无效 UTI 会直接语法错（-2741），
        // 选择器弹不出来且只在用户点「新增模板 → 选压缩包」时才暴露
        "zip" => "public.zip-archive".into(),
        other => format!("public.{}", other),
    }
}

/// 拼装 `choose file of type {...}` 脚本。
/// UTI 列表与默认目录都要做校验，否则 osascript 会在用户面前报错。
fn build_file_script(
    prompt: Option<&str>,
    default_path: Option<&str>,
    exts: Option<Vec<String>>,
) -> String {
    let prompt = match prompt.unwrap_or("").trim() {
        "" => "选择图片",
        p => p,
    };
    let utis: Vec<String> = exts
        .map(|v| v.iter().map(|s| ext_to_uti(s)).collect())
        .filter(|v: &Vec<String>| !v.is_empty())
        .unwrap_or_else(|| {
            vec![
                "public.png".into(),
                "public.jpeg".into(),
                "public.heic".into(),
                "public.tiff".into(),
            ]
        });
    // ⚠️ UTI 必须写成带引号的字符串字面量。写成 `{public.png, public.jpeg}` 会让
    // AppleScript 直接语法错（-2741），而这种错只在用户点「换图标」弹面板时才暴露。
    let uti_list = utis
        .iter()
        .map(|u| format!("\"{}\"", esc_as(u)))
        .collect::<Vec<_>>()
        .join(", ");
    let mut script = format!(
        "POSIX path of (choose file of type {{{}}} with prompt \"{}\"",
        uti_list,
        esc_as(prompt)
    );
    if let Some(raw) = default_path.map(str::trim).filter(|d| !d.is_empty()) {
        let dir = expand_tilde(raw);
        if dir.is_dir() {
            script.push_str(&format!(
                " default location POSIX file \"{}\"",
                esc_as(&dir.to_string_lossy())
            ));
        }
    }
    script.push(')');
    script
}

/// 调起 macOS 原生文件选择器（Standard Additions 的 `choose file of type ...`），
/// 返回去掉尾斜杠的 POSIX 绝对路径。用户点取消返回 `Ok(None)`。
fn pick_file_blocking(
    prompt: Option<String>,
    default_path: Option<String>,
    exts: Option<Vec<String>>,
) -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    {
        let script = build_file_script(prompt.as_deref(), default_path.as_deref(), exts);

        let out = Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| format!("无法调起系统文件选择器：{e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            if err.contains("-128") || err.to_lowercase().contains("cancel") {
                return Ok(None);
            }
            return Err(format!("文件选择器出错：{}", err.trim()));
        }
        let picked = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let picked = if picked.len() > 1 {
            picked.trim_end_matches('/').to_string()
        } else {
            picked
        };
        if picked.is_empty() {
            return Ok(None);
        }
        Ok(Some(picked))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (prompt, default_path, exts);
        Err("原生文件选择器目前仅支持 macOS".to_string())
    }
}

/// 弹出 macOS 原生文件选择器，把用户选中的图片路径回传给前端（换图标用）。
#[tauri::command]
pub async fn pick_file(
    prompt: Option<String>,
    default_path: Option<String>,
    exts: Option<Vec<String>>,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || pick_file_blocking(prompt, default_path, exts))
        .await
        .map_err(|e| format!("文件选择器执行失败：{e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_quotes_and_backslashes() {
        assert_eq!(esc_as(r#"a"b"#), r#"a\"b"#);
        assert_eq!(esc_as(r"a\b"), r"a\\b");
        assert_eq!(esc_as("普通中文"), "普通中文");
    }

    #[test]
    fn script_without_default_location() {
        let s = build_script("选择文件夹", None);
        assert_eq!(s, "POSIX path of (choose folder with prompt \"选择文件夹\")");
        // 空提示回落默认文案
        assert_eq!(s, build_script("   ", None));
    }

    #[test]
    fn script_escapes_prompt() {
        assert_eq!(
            build_script(r#"含"引号"的提示"#, None),
            "POSIX path of (choose folder with prompt \"含\\\"引号\\\"的提示\")"
        );
    }

    #[test]
    fn script_omits_nonexistent_default_location() {
        let s = build_script("选目录", Some("/___pb_not_exist___/x"));
        assert!(!s.contains("default location"), "不存在的目录不应写进脚本: {s}");
        assert!(s.ends_with(')'));
    }

    #[test]
    fn script_includes_existing_default_location() {
        let home = std::env::var("HOME").unwrap();
        let s = build_script("选目录", Some("~"));
        assert!(
            s.contains(&format!("default location POSIX file \"{home}\"")),
            "~ 应展开为 HOME 并写入脚本: {s}"
        );
    }

    /// 真机冒烟：会真的弹出系统目录选择器。
    /// 手动运行：`cargo test -- --ignored pick_folder_real`
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn pick_folder_real() {
        let got = pick_folder_blocking(Some("请选一个文件夹（测试）".into()), None).unwrap();
        eprintln!("用户选择了: {got:?}");
        if let Some(p) = got {
            assert!(std::path::Path::new(&p).is_dir(), "返回的路径应当是已存在的目录");
            assert!(!p.ends_with('/') || p == "/", "不应带尾斜杠");
        }
    }

    /// 语法守护：把这些脚本交给 `osacompile` **只编译不执行**。
    /// `choose folder/file` 跑起来会弹原生面板阻塞测试，但语法错（比如 UTI 列表
    /// 拼成一个字符串、漏了逗号）只会在用户点「换图标」时才炸——那种 bug 必须
    /// 在单测里拦掉。osacompile 不弹窗，纯语法检查。
    #[cfg(target_os = "macos")]
    #[test]
    fn scripts_compile_as_applescript() {
        let cases = [
            ("choose folder 无默认目录", build_script("选目录", None)),
            ("choose folder 带默认目录", build_script("选目录", Some("~"))),
            (
                "choose file 默认类型",
                build_file_script(Some("选图片"), None, None),
            ),
            (
                "choose file 指定 png/jpg",
                build_file_script(Some("换图标"), Some("~"), Some(vec!["png".into(), "jpg".into()])),
            ),
            (
                "choose file 空 exts 走默认",
                build_file_script(None, None, Some(vec![])),
            ),
            (
                "choose file 含引号提示",
                build_file_script(Some(r#"含"引号""#), None, None),
            ),
        ];
        let dir = std::env::temp_dir().join(format!("pb-osac-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        for (i, (name, src)) in cases.into_iter().enumerate() {
            // 用例名可能带 `/`、引号等，不能直接当文件名
            let out_path = dir.join(format!("case{i}.scpt"));
            let out = Command::new("/usr/bin/osacompile")
                .arg("-e")
                .arg(&src)
                .arg("-o")
                .arg(&out_path)
                .output()
                .expect("osacompile 应可执行");
            assert!(
                out.status.success(),
                "[{name}] AppleScript 语法错：{} | 脚本：{src}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

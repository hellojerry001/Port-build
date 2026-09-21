/* =============================================================================
   appmeta.rs · Mac 项目的应用元信息（打包弹窗配置态用）
   -----------------------------------------------------------------------------
   只碰 tauri.conf.json 里 productName / identifier 两个字段：
   productName 决定 .app 文件名与 Dock / Finder 显示名，identifier 是只读展示。
   ============================================================================= */

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::icon;
use crate::projects;

/// 应用名允许的最大字符数（不是字节数）
const APP_NAME_MAX: usize = 40;

/// 会破坏 .app 文件名或 JSON 字面量的字符
const APP_NAME_BAD: [char; 5] = ['/', '\\', ':', '"', '\n'];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppMeta {
    /// tauri.conf.json 的 productName
    pub product_name: String,
    /// bundle identifier，只读展示
    pub identifier: String,
    /// 当前图标 `src-tauri/icons/icon.png` 的裸 base64；无图标为 None
    pub icon: Option<String>,
    /// tauri.conf.json 绝对路径（前端提示用）
    pub config_path: String,
}

fn config_path_of(project_path: &str) -> PathBuf {
    Path::new(project_path).join("src-tauri/tauri.conf.json")
}

/// 读 Mac 项目的应用元信息。配置文件缺失或没有 productName 时返回空串，
/// 前端会把空串当「未配置」处理，而不是报错挡住打包。
#[tauri::command]
pub fn app_meta(id: String) -> Result<AppMeta, String> {
    let p = projects::find(&id).ok_or("项目不存在")?;
    let cfg = config_path_of(&p.path);

    let text = fs::read_to_string(&cfg).unwrap_or_default();
    let json: serde_json::Value = if text.trim().is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(&text).map_err(|e| format!("tauri.conf.json 不是合法 JSON：{e}"))?
    };

    let icon_path = Path::new(&p.path).join("src-tauri/icons/icon.png");
    let icon_b64 = fs::read(&icon_path).ok().map(|b| icon::b64(&b));

    Ok(AppMeta {
        product_name: json
            .get("productName")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        identifier: json
            .get("identifier")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        icon: icon_b64,
        config_path: cfg.to_string_lossy().to_string(),
    })
}

/// 改 Mac 应用的显示名：写 tauri.conf.json 的 productName。
/// 下次 `tauri build` 产出的 .app 就会带新名字（.app 文件名与 Dock 显示名都来自它）。
#[tauri::command]
pub fn set_app_name(id: String, name: String) -> Result<String, String> {
    let p = projects::find(&id).ok_or("项目不存在")?;
    let clean = clean_app_name(&name)?;
    let cfg = config_path_of(&p.path);

    let text = fs::read_to_string(&cfg)
        .map_err(|e| format!("读不到 {}：{e}", cfg.to_string_lossy()))?;
    let next = rewrite_product_name(&text, &clean)?;
    fs::write(&cfg, next).map_err(|e| format!("写入失败：{e}"))?;

    Ok(format!("应用名称已改为「{clean}」"))
}

/* ============================== 纯逻辑（可单测） ============================== */

/// 校验并规整应用名。macOS 的 HFS+/APFS 会把 `:` 当路径分隔符，
/// `.app` 名字里再带 `:` / `/` 会让目录结构直接坏掉，所以一律拒掉。
fn clean_app_name(raw: &str) -> Result<String, String> {
    let n = raw.trim();
    if n.is_empty() {
        return Err("应用名称不能为空".into());
    }
    if n.chars().count() > APP_NAME_MAX {
        return Err(format!("应用名称最多 {APP_NAME_MAX} 个字符"));
    }
    for bad in APP_NAME_BAD {
        if n.contains(bad) {
            return Err(format!("应用名称不能包含 {bad}"));
        }
    }
    if n.starts_with('.') {
        return Err("应用名称不能以 . 开头".into());
    }
    // 控制字符（含制表符）会污染 JSON 字面量
    if n.chars().any(|c| c.is_control()) {
        return Err("应用名称不能包含控制字符".into());
    }
    Ok(n.to_string())
}

/// 文本级替换 productName 那一行。
///
/// 不用 serde_json 反序列化再序列化，是因为那样会把用户文件里
/// `"build": { "frontendDist": "../ui" }` 这类紧凑写法展开成多行，
/// 凭空制造一大片无关 diff。这里只动目标行，其余字节原样保留。
///
/// 只认**独占一行**的 `"productName": "..."`（Tauri CLI 生成的配置就是这格式）。
/// 被压缩成一行或内联在别的对象里时一律保守报错，不猜、不乱改。
fn rewrite_product_name(json: &str, name: &str) -> Result<String, String> {
    let mut hits = 0usize;
    let mut out = String::with_capacity(json.len() + name.len());

    for line in json.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("\"productName\"") {
            out.push_str(line);
            continue;
        }
        hits += 1;

        let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
        let nl = if line.ends_with('\n') { "\n" } else { "" };
        // 沿用原行的尾逗号状态：productName 可能是最后一个字段
        let comma = if line.trim_end().ends_with(',') { "," } else { "" };
        out.push_str(&format!("{indent}\"productName\": \"{name}\"{comma}{nl}"));
    }

    match hits {
        0 if json.contains("\"productName\"") => Err(
            "tauri.conf.json 里的 productName 不是独立一行（文件可能被压缩过），\
             为免改坏配置，请先把它格式化回标准缩进"
                .into(),
        ),
        0 => Err("tauri.conf.json 里没有 productName 字段，无法改名".into()),
        1 => Ok(out),
        n => Err(format!("tauri.conf.json 里有 {n} 处 productName，不敢猜改哪个")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONF: &str = r#"{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "PortButler",
  "version": "0.3.0",
  "identifier": "com.jerry.portbutler",
  "build": { "frontendDist": "../ui" },
  "bundle": {
    "icon": ["icons/icon.icns"]
  }
}
"#;

    #[test]
    fn rewrites_only_the_product_name_line() {
        let got = rewrite_product_name(CONF, "端口管家").unwrap();
        assert!(got.contains("\"productName\": \"端口管家\","));
        // 其余字节一字不动：紧凑写法不会被展开
        assert!(got.contains("\"build\": { \"frontendDist\": \"../ui\" },"));
        assert!(got.contains("\"$schema\": \"https://schema.tauri.app/config/2\","));
        assert_eq!(got.lines().count(), CONF.lines().count());
        // 只换了一行的内容
        let diff = got
            .lines()
            .zip(CONF.lines())
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(diff, 1, "只应改动 productName 那一行");
    }

    #[test]
    fn keeps_indent_and_handles_last_field_without_comma() {
        let json = "{\n\t\"a\": 1,\n\t\"productName\": \"Old\"\n}";
        let got = rewrite_product_name(json, "New").unwrap();
        assert_eq!(got, "{\n\t\"a\": 1,\n\t\"productName\": \"New\"\n}");
    }

    #[test]
    fn missing_product_name_is_an_error() {
        let err = rewrite_product_name("{\n  \"version\": \"1.0.0\"\n}\n", "X").unwrap_err();
        assert!(err.contains("没有 productName"), "{err}");
    }

    #[test]
    fn duplicate_product_name_is_refused() {
        let json = "{\n  \"productName\": \"A\",\n  \"x\": 1,\n  \"productName\": \"B\"\n}";
        let err = rewrite_product_name(json, "C").unwrap_err();
        assert!(err.contains("2 处"), "{err}");
    }

    /// 压缩/内联写法：宁可不改也不能改坏用户的配置
    #[test]
    fn inline_product_name_is_refused_instead_of_guessed() {
        let json = "{\"productName\":\"A\",\"version\":\"1.0.0\"}";
        let err = rewrite_product_name(json, "C").unwrap_err();
        assert!(err.contains("不是独立一行"), "{err}");
        // 嵌套在别的对象里同理
        let nested = "{\n  \"x\": {\"productName\": \"B\"}\n}\n";
        assert!(rewrite_product_name(nested, "C").is_err());
    }

    #[test]
    fn app_name_rejects_path_hostile_input() {
        assert!(clean_app_name("  ").is_err());
        assert!(clean_app_name("有/斜杠").is_err());
        assert!(clean_app_name("有\\反斜杠").is_err());
        assert!(clean_app_name("有:冒号").is_err());
        assert!(clean_app_name("有\"引号").is_err());
        assert!(clean_app_name(".hidden").is_err());
        assert!(clean_app_name(&"字".repeat(APP_NAME_MAX + 1)).is_err());
        // 首尾空白会被规整掉
        assert_eq!(clean_app_name("  端口管家  ").unwrap(), "端口管家");
        // 中文与空格合法（.app 名可以带空格）
        assert_eq!(clean_app_name("Port Butler").unwrap(), "Port Butler");
        assert_eq!(clean_app_name(&"字".repeat(APP_NAME_MAX)).unwrap().chars().count(), APP_NAME_MAX);
    }
}

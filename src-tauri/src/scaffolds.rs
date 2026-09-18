use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::ports;
use crate::projects::{self, Project};

/// 模板中的占位符
const SLUG_PLACEHOLDER: &str = "__PB_SLUG__";
const PORT_PLACEHOLDER: &str = "__PB_PORT__";

/// 需要做占位符替换的文件（相对项目根）；不存在则跳过
const INJECT_FILES: [&str; 3] = ["package.json", "zmi.config.js", "publish.json"];

/// 克隆后要清掉的构建产物
const DROP_ENTRIES: [&str; 1] = ["dist"];

fn default_port_start() -> u16 {
    8100
}

fn default_command() -> String {
    "npm run serve".to_string()
}

/// 一份模板的描述，来自 `<模板目录>/.pb-scaffold.json`
#[derive(Serialize, Deserialize, Clone)]
pub struct Scaffold {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub desc: String,
    #[serde(rename = "portStart", default = "default_port_start")]
    pub port_start: u16,
    #[serde(rename = "nodeVersion", default)]
    pub node_version: String,
    #[serde(default = "default_command")]
    pub command: String,

    // ↓ 运行时填充，不写进描述文件
    #[serde(default)]
    pub path: String,
    /// 是否找到了对应版本的 Node（找不到就退回登录 shell 默认版本）
    #[serde(default)]
    pub node_ready: bool,
}

/// 模板根目录：`~/.portbutler/scaffolds`
pub fn scaffold_root() -> PathBuf {
    projects::data_dir().join("scaffolds")
}

fn read_scaffold(dir: &Path) -> Option<Scaffold> {
    let text = fs::read_to_string(dir.join(".pb-scaffold.json")).ok()?;
    let mut s: Scaffold = serde_json::from_str(&text).ok()?;
    if s.key.is_empty() {
        s.key = dir.file_name()?.to_string_lossy().to_string();
    }
    s.node_ready = projects::node_bin_dir(&s.node_version).is_some();
    s.path = dir.to_string_lossy().to_string();
    Some(s)
}

#[tauri::command]
pub fn list_scaffolds() -> Vec<Scaffold> {
    let mut out: Vec<Scaffold> = Vec::new();
    if let Ok(entries) = fs::read_dir(scaffold_root()) {
        for e in entries.flatten() {
            let dir = e.path();
            if dir.is_dir() {
                if let Some(s) = read_scaffold(&dir) {
                    out.push(s);
                }
            }
        }
    }
    out.sort_by_key(|s| s.port_start);
    out
}

/// 已被占用的端口 = 已登记项目 + 系统正在监听
fn reserved_ports() -> Vec<u16> {
    let mut v: Vec<u16> = projects::load().iter().map(|p| p.port).collect();
    if let Ok(list) = ports::scan_ports() {
        v.extend(list.iter().map(|p| p.port));
    }
    v
}

fn pick_port(start: u16) -> u16 {
    let used = reserved_ports();
    let mut p = start;
    while used.contains(&p) && p < 65535 {
        p += 1;
    }
    p
}

/// 给某个模板建议一个空闲端口（前端打开弹窗时调用）
#[tauri::command]
pub fn suggest_port(scaffold: String) -> u16 {
    let start = list_scaffolds()
        .into_iter()
        .find(|s| s.key == scaffold)
        .map(|s| s.port_start)
        .unwrap_or(8100);
    pick_port(start)
}

fn validate_slug(slug: &str) -> Result<(), String> {
    if slug.is_empty() {
        return Err("目录名不能为空".into());
    }
    if slug.len() > 64 {
        return Err("目录名过长（最多 64 字符）".into());
    }
    if slug == "." || slug == ".." {
        return Err("目录名不合法".into());
    }
    if !slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err("目录名只能用小写字母、数字、- 和 _".into());
    }
    let first = slug.chars().next().unwrap();
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err("目录名要以字母或数字开头".into());
    }
    Ok(())
}

/// 展开开头的 `~` —— Rust 的 PathBuf 不会自动展开，而用户习惯写 `~/WorkBuddy`
pub(crate) fn expand_tilde(p: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    if p == "~" {
        return PathBuf::from(home);
    }
    if let Some(rest) = p.strip_prefix("~/") {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(p)
}

/// 复制模板：优先用 APFS 写时复制（`cp -Rc`），跨卷失败时回退普通复制
fn clone_template(src: &Path, dst: &Path) -> Result<String, String> {
    let fast = Command::new("cp").arg("-Rc").arg(src).arg(dst).output();
    if let Ok(o) = &fast {
        if o.status.success() {
            return Ok("APFS 克隆".to_string());
        }
    }
    // 回退前清掉可能留下的半成品，否则普通复制会把 dst 套在里面
    let _ = fs::remove_dir_all(dst);
    let slow = Command::new("cp")
        .arg("-R")
        .arg(src)
        .arg(dst)
        .output()
        .map_err(|e| format!("调用 cp 失败: {e}"))?;
    if slow.status.success() {
        Ok("普通复制".to_string())
    } else {
        Err(format!(
            "复制模板失败: {}",
            String::from_utf8_lossy(&slow.stderr).trim()
        ))
    }
}

/// 克隆后清理：删掉构建产物与系统垃圾文件（模板是只读基线，不受影响）
fn clean_target(target: &Path) {
    for entry in DROP_ENTRIES {
        let _ = fs::remove_dir_all(target.join(entry));
    }
    let _ = fs::remove_file(target.join(".DS_Store"));
}

/// 把模板中的占位符替换成真实值；返回被改写的文件名
fn inject(target: &Path, slug: &str, port: u16) -> Result<Vec<String>, String> {
    let mut touched: Vec<String> = Vec::new();
    for rel in INJECT_FILES {
        let path = target.join(rel);
        if !path.is_file() {
            continue;
        }
        let src = fs::read_to_string(&path).map_err(|e| format!("读取 {rel} 失败: {e}"))?;
        if !src.contains(SLUG_PLACEHOLDER) && !src.contains(PORT_PLACEHOLDER) {
            continue;
        }
        let out = src
            .replace(SLUG_PLACEHOLDER, slug)
            .replace(PORT_PLACEHOLDER, &port.to_string());
        fs::write(&path, out).map_err(|e| format!("写入 {rel} 失败: {e}"))?;
        touched.push(rel.to_string());
    }
    if touched.is_empty() {
        return Err("模板里没找到占位符，可能不是由端口管家生成的模板".into());
    }
    Ok(touched)
}

/// 从模板新建一个项目：克隆 → 清理 → 注入 → 登记。
/// 返回新建的 Project（前端拿到 id 后即可调用 start_project 启动）。
#[tauri::command]
pub fn create_project(
    name: String,
    slug: String,
    parent: String,
    scaffold: String,
    port: u16,
) -> Result<Project, String> {
    let name = name.trim().to_string();
    let slug = slug.trim().to_lowercase();
    if name.is_empty() {
        return Err("项目名不能为空".into());
    }
    validate_slug(&slug)?;

    let tpl = list_scaffolds()
        .into_iter()
        .find(|s| s.key == scaffold)
        .ok_or_else(|| format!("找不到模板「{scaffold}」"))?;
    let tpl_path = PathBuf::from(&tpl.path);
    if !tpl_path.is_dir() {
        return Err(format!("模板目录不存在：{}", tpl.path));
    }

    let parent_path = expand_tilde(parent.trim());
    if parent_path.as_os_str().is_empty() {
        return Err("请填写项目存放目录".into());
    }
    if !parent_path.is_dir() {
        fs::create_dir_all(&parent_path)
            .map_err(|e| format!("目录不存在且创建失败：{}（{e}）", parent_path.display()))?;
    }

    let target = parent_path.join(&slug);
    if target.exists() {
        return Err(format!("目录已存在：{}", target.display()));
    }
    if target.starts_with(&tpl_path) {
        return Err("项目目录不能建在模板目录里面".into());
    }

    // 端口：0 或已被占用就自动挑一个空闲的
    let port = if port == 0 || reserved_ports().contains(&port) {
        pick_port(tpl.port_start)
    } else {
        port
    };

    clone_template(&tpl_path, &target)?;
    clean_target(&target);
    if let Err(e) = inject(&target, &slug, port) {
        let _ = fs::remove_dir_all(&target); // 注入失败则回滚，不留半个项目
        return Err(e);
    }

    let project = Project {
        id: String::new(),
        name,
        path: target.to_string_lossy().to_string(),
        command: tpl.command.clone(),
        port,
        scaffold: tpl.key.clone(),
        node_version: tpl.node_version.clone(),
        // 模板目前都是 Web 脚手架；以后加了桌面端模板就从这个描述文件里读
        kind: projects::KIND_WEB.to_string(),
        build_command: String::new(),
    };
    let (project, _) = projects::upsert(project)?;
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pb-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn expand_tilde_works() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(expand_tilde("~"), PathBuf::from(&home));
        assert_eq!(
            expand_tilde("~/WorkBuddy"),
            PathBuf::from(&home).join("WorkBuddy")
        );
        assert_eq!(expand_tilde("/tmp/x"), PathBuf::from("/tmp/x"));
    }

    #[test]
    fn slug_validation() {
        assert!(validate_slug("abc-1").is_ok());
        assert!(validate_slug("a_b2").is_ok());
        assert!(validate_slug("").is_err());
        assert!(validate_slug("Abc").is_err());
        assert!(validate_slug("中文").is_err());
        assert!(validate_slug("-a").is_err());
        assert!(validate_slug("a/b").is_err());
    }

    #[test]
    fn clone_clean_inject_pipeline() {
        let root = tmp("pipe");
        let tpl = root.join("tpl");
        fs::create_dir_all(tpl.join("dist")).unwrap();
        fs::write(tpl.join("package.json"), r#"{"name":"__PB_SLUG__"}"#).unwrap();
        fs::write(tpl.join("zmi.config.js"), "port: __PB_PORT__,").unwrap();
        fs::write(tpl.join("publish.json"), r#"{"appKey":"__PB_SLUG__"}"#).unwrap();
        fs::write(tpl.join("dist/junk.js"), "x").unwrap();
        fs::write(tpl.join(".DS_Store"), "x").unwrap();

        let dst = root.join("out");
        clone_template(&tpl, &dst).unwrap();
        clean_target(&dst);
        let touched = inject(&dst, "demo-app", 8101).unwrap();

        assert!(!dst.join("dist").exists(), "dist 没清掉");
        assert!(!dst.join(".DS_Store").exists(), ".DS_Store 没清掉");
        assert_eq!(touched.len(), 3);

        let pkg = fs::read_to_string(dst.join("package.json")).unwrap();
        assert!(pkg.contains("demo-app") && !pkg.contains(SLUG_PLACEHOLDER));
        let cfg = fs::read_to_string(dst.join("zmi.config.js")).unwrap();
        assert!(cfg.contains("8101") && !cfg.contains(PORT_PLACEHOLDER));

        // 写时复制：模板本身必须毫发无损
        assert!(tpl.join("dist/junk.js").exists(), "模板被改到了");
        assert_eq!(
            fs::read_to_string(tpl.join("package.json")).unwrap(),
            r#"{"name":"__PB_SLUG__"}"#
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pick_port_avoids_reserved() {
        let p = pick_port(8100);
        assert!(p >= 8100);
        assert!(!reserved_ports().contains(&p), "挑到了已占用的端口 {p}");
    }

    /// 真机冒烟：用真实模板走完整的 create_project（含登记）。
    /// 临时改写 projects.json，跑完还原 —— 只在 `cargo test -- --ignored` 时执行。
    #[test]
    #[ignore]
    fn create_project_real_template() {
        let json = projects::data_dir().join("projects.json");
        let backup = fs::read(&json).ok();

        let parent = tmp("real");
        let res = create_project(
            "冒烟测试项目".into(),
            "pb-smoke".into(),
            parent.to_string_lossy().to_string(),
            "pc".into(),
            0,
        );

        // 无论成败都还原用户数据
        match &backup {
            Some(b) => fs::write(&json, b).unwrap(),
            None => {
                let _ = fs::remove_file(&json);
            }
        }

        let p = res.expect("create_project 失败");
        println!(
            "→ name={} port={} node={} scaffold={} path={}",
            p.name, p.port, p.node_version, p.scaffold, p.path
        );
        assert_eq!(p.scaffold, "pc");
        assert_eq!(p.node_version, "18.16.0");
        assert!(p.port >= 8100, "端口没落到 8100 段");

        let dir = PathBuf::from(&p.path);
        assert!(
            dir.join("node_modules/.bin/zmi").exists(),
            "依赖没被克隆过来"
        );
        let cfg = fs::read_to_string(dir.join("zmi.config.js")).unwrap();
        assert!(cfg.contains(&p.port.to_string()), "端口没注入");
        assert!(cfg.contains("pb-smoke"), "slug 没注入");
        assert!(!cfg.contains(PORT_PLACEHOLDER));

        let _ = fs::remove_dir_all(&parent);
    }
}

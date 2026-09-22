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

/// 克隆后要清掉的构建产物。
/// `dist` 是 zmi 系（pc / mobile）的输出目录；`.next` / `out` / `.content-collections`
/// 是 Next 系文档站（spell-docs）的产物与缓存 —— 不存在的会被静默跳过，
/// 但留着会把旧构建结果和过期内容索引一起带进新项目。
/// `.git` 也必须清：模板若是从某个真实仓库导进来的，带着它的历史与 remote，
/// 新项目第一次提交就可能推到模板自己的仓库去。
const DROP_ENTRIES: [&str; 5] = ["dist", ".next", "out", ".content-collections", ".git"];

fn default_port_start() -> u16 {
    8100
}

fn default_command() -> String {
    "npm run serve".to_string()
}

/// 一份模板的描述，来自 `<模板目录>/.pb-scaffold.json`
#[derive(Serialize, Deserialize, Clone, Debug)]
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
    /// ⚠️ 前端读的是 `nodeReady`：这个结构体没有 `rename_all`，只逐字段 rename，
    /// 漏标一个字段就会序列化成 snake_case，前端拿到 `undefined` 后会把
    /// 「缺 Node」的警告挂在**每一个**模板上。
    #[serde(default, rename = "nodeReady")]
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

/// 模板根目录的绝对路径（不存在就建出来），供模板库页的「打开模板目录」定位
#[tauri::command]
pub fn scaffold_root_path() -> Result<String, String> {
    let root = scaffold_root();
    if !root.exists() {
        fs::create_dir_all(&root).map_err(|e| format!("创建模板目录失败：{e}"))?;
    }
    Ok(root.to_string_lossy().to_string())
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

/* ============================== 新增模板（导入本机项目 / zip 包） ==============================
   用户在模板库点「＋ 新增模板」，选一个本机项目目录或一个 .zip：
   1. `probe_scaffold_source` 先看一眼（有没有 .pb-scaffold.json、package.json 里
      dev/serve 脚本、.nvmrc、体积多大、有哪些坑），把能推断出的字段回给前端预填；
   2. 用户在弹窗里改完点「导入」→ `import_scaffold` 把它落到
      ~/.portbutler/scaffolds/<key>，顺手清掉 .git/构建产物/.DS_Store，
      并保证至少有一个文件含占位符（否则以后用它新建项目会失败）。
*/

/// 写进 `.pb-scaffold.json` 的字段集合。
/// 与 `Scaffold` 用同一套字段名（`portStart` / `nodeVersion`），这样两边读写同一份文件。
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScaffoldDescriptor {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub desc: String,
    #[serde(default = "default_port_start")]
    pub port_start: u16,
    #[serde(default)]
    pub node_version: String,
    #[serde(default = "default_command")]
    pub command: String,
}

/// 导入前的体检结果
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScaffoldProbe {
    /// "dir" | "zip"
    pub kind: String,
    /// 用户选中的原始路径
    pub source: String,
    /// 真正的模板根：目录给绝对路径，zip 给包内前缀（"" = 包根）
    pub root: String,
    /// 自动下钻了一层子目录（zip 常见「外层套一层同名目录」）
    pub drilled: bool,
    pub has_descriptor: bool,
    pub descriptor: Option<ScaffoldDescriptor>,
    /// 预填进表单的建议值（已含描述文件里的字段，没有描述文件就靠推断）
    pub suggested: ScaffoldDescriptor,
    pub file_count: u64,
    pub size_bytes: u64,
    /// 顶层条目预览
    pub entries: Vec<String>,
    pub warnings: Vec<String>,
    /// 有占位符才能被 create_project 注入（缺 __PB_SLUG__ 时导入会自动补一个）
    pub has_port_placeholder: bool,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub scaffold: Scaffold,
    /// APFS 克隆 / 普通复制 / 解压
    pub method: String,
    /// 导入过程中做的事（清掉了什么、补了什么），前端原样展示
    pub notes: Vec<String>,
}

fn is_hidden_entry(p: &Path) -> bool {
    p.file_name()
        .map(|n| n.to_string_lossy().starts_with('.'))
        .unwrap_or(true)
}

/// 目录名 → 合法的模板 key（小写字母 / 数字 / - _）
fn slugify(raw: &str) -> String {
    let s: String = raw
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    // 把连续 '-' 压成一个，顺手去掉首尾
    let mut out = String::new();
    for c in s.chars() {
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "my-template".to_string()
    } else {
        out
    }
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// 目录/包内是否含 `name`（大小写不敏感）
fn find_entry(entries: &[String], name: &str) -> Option<String> {
    entries
        .iter()
        .find(|e| e.rsplit('/').next().map(|f| f.eq_ignore_ascii_case(name)).unwrap_or(false))
        .cloned()
}

/// package.json 里挑一条启动命令：dev > serve > start
fn command_from_pkg(pkg: Option<&serde_json::Value>) -> Option<String> {
    let scripts = pkg?.get("scripts")?.as_object()?;
    for (key, verb) in [("dev", "dev"), ("serve", "serve"), ("start", "start")] {
        if scripts.contains_key(key) {
            return Some(format!("npm run {verb}"));
        }
    }
    None
}

/// Node 版本：优先 .nvmrc（写死的版本），其次 package.json 的 engines.node（只认 x.y.z）
fn node_version_hint(nvmrc: &str, pkg: Option<&serde_json::Value>) -> String {
    let v = nvmrc.trim().trim_start_matches('v').to_string();
    if v.split('.').count() == 3 && v.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return v;
    }
    if let Some(e) = pkg
        .and_then(|p| p.get("engines"))
        .and_then(|e| e.get("node"))
        .and_then(|n| n.as_str())
    {
        let v = e.trim().trim_start_matches(['^', '~', '>', '=', 'v', ' ']);
        if v.split('.').count() == 3 && v.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return v.to_string();
        }
    }
    String::new()
}

/// 下一个可用的模板端口段：现有模板里最大的 portStart 往上加 100
fn next_port_start() -> u16 {
    list_scaffolds()
        .iter()
        .map(|s| s.port_start)
        .max()
        .unwrap_or(8000)
        + 100
}

/// 走一遍目录树统计体积与文件数（node_modules 也数，用户要知道模板多大）
fn dir_stats(root: &Path) -> (u64, u64) {
    let mut files = 0u64;
    let mut bytes = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                stack.push(p);
            } else if md.is_file() {
                files += 1;
                bytes += md.len();
            }
            if files > 300_000 {
                return (files, bytes); // 兜底：别为了一个数字卡住界面
            }
        }
    }
    (files, bytes)
}

fn top_entries(dir: &Path, limit: usize) -> Vec<String> {
    let Ok(rd) = fs::read_dir(dir) else {
        return vec![];
    };
    let mut v: Vec<String> = rd
        .flatten()
        .map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if e.path().is_dir() {
                format!("{name}/")
            } else {
                name
            }
        })
        .collect();
    v.sort();
    v.truncate(limit);
    v
}

/// 模板根下如果只有一个目录（zip 里常见「外层套一层」），下钻一层
fn resolve_dir_root(src: &Path) -> (PathBuf, bool) {
    if src.join(".pb-scaffold.json").is_file() {
        return (src.to_path_buf(), false);
    }
    if let Ok(rd) = fs::read_dir(src) {
        let dirs: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && !is_hidden_entry(p))
            .collect();
        if dirs.len() == 1 {
            let c = &dirs[0];
            if c.join(".pb-scaffold.json").is_file() || c.join("package.json").is_file() {
                return (c.clone(), true);
            }
        }
    }
    (src.to_path_buf(), false)
}

/// zip 包内所有条目名（`unzip -Z1`）。
/// 过滤掉 macOS 归档工具塞进来的 `__MACOSX/` 资源叉目录，否则
/// 「包内唯一顶层目录」的判断会被它污染（`app/` 与 `__MACOSX/` 两个顶层 → 认不出模板根）。
fn zip_entries(zip: &Path) -> Result<Vec<String>, String> {
    let out = Command::new("unzip")
        .arg("-Z1")
        .arg(zip)
        .output()
        .map_err(|e| format!("无法读取压缩包：{e}"))?;
    if !out.status.success() {
        return Err(format!(
            "读取压缩包失败：{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .filter(|s| !s.starts_with("__MACOSX"))
        .collect())
}

fn zip_read(zip: &Path, entry: &str) -> Option<String> {
    let out = Command::new("unzip").arg("-p").arg(zip).arg(entry).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

/// zip 包 → 模板根前缀（描述文件所在目录，"" 表示包根）
fn zip_root_prefix(entries: &[String]) -> String {
    let deepest_desc = entries
        .iter()
        .filter(|e| {
            let low = e.to_lowercase();
            low == ".pb-scaffold.json" || low.ends_with("/.pb-scaffold.json")
        })
        .min_by_key(|e| e.matches('/').count());
    if let Some(b) = deepest_desc {
        return b
            .rsplit_once('/')
            .map(|(dir, _)| dir.to_string())
            .unwrap_or_default();
    }
    // 没有描述文件：若有唯一顶层目录，就以它为根
    let mut tops: Vec<&str> = entries
        .iter()
        .filter_map(|e| e.split_once('/').map(|(top, _)| top))
        .collect();
    tops.sort();
    tops.dedup();
    match tops.len() {
        1 => tops[0].to_string(),
        _ => String::new(),
    }
}

/// 在已解压/已克隆出来的目录里删掉 .DS_Store / .git，并记录做了什么
fn scrub_template(dst: &Path, notes: &mut Vec<String>) {
    let mut ds_store = 0u64;
    let mut stack = vec![dst.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                stack.push(p);
            } else if name == ".DS_Store" {
                if fs::remove_file(&p).is_ok() {
                    ds_store += 1;
                }
            }
        }
    }
    if ds_store > 0 {
        notes.push(format!("清理了 {ds_store} 个 .DS_Store"));
    }
}

/// 保证模板里至少有一个可注入的占位符。
/// `create_project` 注入后要求至少改到一个文件，否则新项目建不出来 ——
/// 这里兜底：package.json 缺 `__PB_SLUG__` 就把 name 字段写成占位符。
fn ensure_placeholders(dst: &Path, notes: &mut Vec<String>) -> Result<(), String> {
    let mut has_slug = false;
    let mut has_port = false;
    for f in INJECT_FILES {
        if let Ok(s) = fs::read_to_string(dst.join(f)) {
            has_slug |= s.contains(SLUG_PLACEHOLDER);
            has_port |= s.contains(PORT_PLACEHOLDER);
        }
    }

    if !has_slug {
        let pkg_path = dst.join("package.json");
        if !pkg_path.is_file() {
            return Err(
                "模板里没有 package.json / zmi.config.js / publish.json，没地方注入项目名与端口，\
                 无法作为模板使用"
                    .into(),
            );
        }
        let mut v = read_json(&pkg_path).ok_or("package.json 不是合法 JSON，无法注入占位符")?;
        v["name"] = serde_json::Value::String(SLUG_PLACEHOLDER.to_string());
        let text = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
        fs::write(&pkg_path, format!("{text}\n")).map_err(|e| format!("写入 package.json 失败：{e}"))?;
        notes.push(format!(
            "package.json 的 name 已写成 {SLUG_PLACEHOLDER}（新建项目时会替换成目录名）"
        ));
    }

    if !has_port {
        notes.push(format!(
            "没发现 {PORT_PLACEHOLDER} 占位符：用它新建项目时端口不会写进配置，\
             想自动绑定端口的话，可以在 package.json / zmi.config.js 里手写占位符"
        ));
    }
    Ok(())
}

/// 体检出来的原始素材（目录与 zip 两条路各自填，最后统一算建议值）
struct RawProbe {
    kind: &'static str,
    root_label: String,
    drilled: bool,
    descriptor: Option<ScaffoldDescriptor>,
    pkg: Option<serde_json::Value>,
    /// .nvmrc 里的版本（目录读文件，zip 读包内条目）
    nvmrc: String,
    entries: Vec<String>,
    files: u64,
    bytes: u64,
    has_port: bool,
    folder_name: String,
}

/// 三个可注入文件里有没有 `__PB_PORT__`
fn pkg_has_port<S: AsRef<str>>(read: impl Fn(&str) -> Option<S>) -> bool {
    INJECT_FILES
        .iter()
        .any(|f| read(f).map(|s| s.as_ref().contains(PORT_PLACEHOLDER)).unwrap_or(false))
}

fn probe_zip(src: &Path, warnings: &mut Vec<String>) -> Result<RawProbe, String> {
    let entries = zip_entries(src)?;
    if entries.is_empty() {
        return Err("压缩包是空的".into());
    }
    let prefix = zip_root_prefix(&entries);
    let descriptor: Option<ScaffoldDescriptor> = entries
        .iter()
        .filter(|e| e.to_lowercase().ends_with(".pb-scaffold.json"))
        .min_by_key(|e| e.matches('/').count())
        .and_then(|e| zip_read(src, e))
        .and_then(|t| serde_json::from_str(&t).ok());
    let pkg = find_entry(&entries, "package.json")
        .and_then(|e| zip_read(src, &e))
        .and_then(|t| serde_json::from_str(&t).ok());
    let nvmrc = find_entry(&entries, ".nvmrc")
        .and_then(|e| zip_read(src, &e))
        .unwrap_or_default();
    if pkg.is_none() {
        warnings.push("压缩包里没看到 package.json，确认这是能启动的项目".into());
    }
    // 包内路径带前缀，读注入文件时要拼回去
    let with_prefix = |name: &str| -> Option<String> {
        let key = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        entries
            .iter()
            .find(|e| e.eq_ignore_ascii_case(&key))
            .and_then(|e| zip_read(src, e))
    };
    let has_port = pkg_has_port(|f| with_prefix(f));
    let mut tops: Vec<String> = entries
        .iter()
        .map(|e| {
            let head = e.split('/').next().unwrap_or("").to_string();
            if e.contains('/') {
                format!("{head}/")
            } else {
                head
            }
        })
        .collect();
    tops.sort();
    tops.dedup();
    tops.truncate(8);

    Ok(RawProbe {
        kind: "zip",
        root_label: prefix,
        drilled: false,
        descriptor,
        pkg,
        nvmrc,
        entries: tops,
        files: entries.len() as u64,
        bytes: fs::metadata(src).map(|m| m.len()).unwrap_or(0),
        has_port,
        folder_name: src
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
    })
}

fn probe_dir(src: &Path, warnings: &mut Vec<String>) -> Result<RawProbe, String> {
    let (root, drilled) = resolve_dir_root(src);
    if drilled {
        warnings.push(format!(
            "已自动下钻到子目录 {}（外层那层不是模板根）",
            root.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        ));
    }
    let descriptor: Option<ScaffoldDescriptor> = fs::read_to_string(root.join(".pb-scaffold.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    let pkg = read_json(&root.join("package.json"));
    if pkg.is_none() {
        warnings.push("没看到 package.json，确认这是能启动的项目".into());
    }
    if root.join(".git").exists() {
        warnings.push("含 .git：导入时会删掉它（否则新项目会带着原仓库的历史与 remote）".into());
    }
    if root.join("node_modules").is_dir() {
        let mb = dir_stats(&root.join("node_modules")).1 / 1024 / 1024;
        warnings.push(format!(
            "含 node_modules（约 {mb}MB）：会一起成为模板基线，新建项目时写时复制它"
        ));
    }
    let (files, bytes) = dir_stats(&root);
    Ok(RawProbe {
        kind: "dir",
        root_label: root.to_string_lossy().to_string(),
        drilled,
        descriptor,
        pkg,
        nvmrc: fs::read_to_string(root.join(".nvmrc")).unwrap_or_default(),
        entries: top_entries(&root, 8),
        files,
        bytes,
        has_port: pkg_has_port(|f| fs::read_to_string(root.join(f)).ok()),
        folder_name: root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
    })
}

/// 体检：选中的目录 / zip 能不能当模板
fn probe_blocking(source: String) -> Result<ScaffoldProbe, String> {
    let src = expand_tilde(source.trim());
    if !src.exists() {
        return Err(format!("路径不存在：{}", src.display()));
    }

    let mut warnings: Vec<String> = Vec::new();
    let is_zip = src.is_file()
        && src
            .extension()
            .map(|e| e.to_string_lossy().eq_ignore_ascii_case("zip"))
            .unwrap_or(false);

    let raw = if is_zip {
        probe_zip(&src, &mut warnings)?
    } else if src.is_dir() {
        probe_dir(&src, &mut warnings)?
    } else {
        return Err("只支持选择文件夹或 .zip 压缩包".into());
    };

    if raw.descriptor.is_none() {
        warnings.push("没有 .pb-scaffold.json：下面这些值是推断出来的，导入前请核对".into());
    }
    if !raw.has_port {
        warnings.push(format!(
            "没发现 {PORT_PLACEHOLDER}：用它新建项目时端口不会被写进配置（项目名仍会注入）"
        ));
    }

    // 预填建议值：描述文件优先，缺什么补什么
    let mut suggested = raw.descriptor.clone().unwrap_or_default();
    if suggested.key.is_empty() {
        suggested.key = slugify(&raw.folder_name);
    }
    if suggested.name.is_empty() {
        suggested.name = raw.folder_name.clone();
    }
    if suggested.desc.is_empty() {
        suggested.desc = raw
            .pkg
            .as_ref()
            .and_then(|p| p.get("description"))
            .and_then(|d| d.as_str())
            .unwrap_or("")
            .to_string();
    }
    if suggested.port_start == 0 {
        suggested.port_start = next_port_start();
    }
    if suggested.command.is_empty() {
        suggested.command =
            command_from_pkg(raw.pkg.as_ref()).unwrap_or_else(|| "npm run dev".into());
    }
    if suggested.node_version.is_empty() {
        suggested.node_version = node_version_hint(&raw.nvmrc, raw.pkg.as_ref());
    }

    Ok(ScaffoldProbe {
        kind: raw.kind.to_string(),
        source: src.to_string_lossy().to_string(),
        root: raw.root_label,
        drilled: raw.drilled,
        has_descriptor: raw.descriptor.is_some(),
        descriptor: raw.descriptor,
        suggested,
        file_count: raw.files,
        size_bytes: raw.bytes,
        entries: raw.entries,
        warnings,
        has_port_placeholder: raw.has_port,
    })
}

/// zip 解压后如果只有一层目录，把里面的内容搬到顶层（省掉手动进两层）
fn flatten_single_root(dst: &Path) -> Result<Option<String>, String> {
    let Ok(rd) = fs::read_dir(dst) else {
        return Ok(None);
    };
    let items: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| n != ".DS_Store" && n != "__MACOSX")
                .unwrap_or(true)
        })
        .collect();
    if items.len() != 1 || !items[0].is_dir() {
        return Ok(None);
    }
    let inner = items[0].clone();
    let name = inner
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let Ok(rd) = fs::read_dir(&inner) else {
        return Ok(None);
    };
    for e in rd.flatten() {
        let from = e.path();
        let Some(fname) = from.file_name() else { continue };
        let to = dst.join(fname);
        fs::rename(&from, &to).map_err(|err| format!("解压后整理目录失败：{err}"))?;
    }
    let _ = fs::remove_dir_all(&inner);
    Ok(Some(name))
}

fn import_blocking(
    source: String,
    key: String,
    name: String,
    desc: String,
    port_start: u16,
    node_version: String,
    command: String,
) -> Result<ImportResult, String> {
    import_into(
        &scaffold_root(),
        source,
        key,
        name,
        desc,
        port_start,
        node_version,
        command,
    )
}

/// 真正干活的版本：目标根目录显式传入，方便单测用临时目录（不碰用户的真实模板库）
fn import_into(
    root_dir: &Path,
    source: String,
    key: String,
    name: String,
    desc: String,
    port_start: u16,
    node_version: String,
    command: String,
) -> Result<ImportResult, String> {
    let key = key.trim().to_lowercase();
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("模板名称不能为空".into());
    }
    validate_slug(&key)?;

    let src = expand_tilde(source.trim());
    if !src.exists() {
        return Err(format!("路径不存在：{}", src.display()));
    }

    fs::create_dir_all(root_dir).map_err(|e| format!("创建模板目录失败：{e}"))?;
    let dst = root_dir.join(&key);
    if dst.exists() {
        return Err(format!(
            "已存在同名模板「{key}」，换个标识，或先删掉 {}",
            dst.display()
        ));
    }

    let mut notes: Vec<String> = Vec::new();
    let method: String;
    let is_zip = src.is_file()
        && src
            .extension()
            .map(|e| e.to_string_lossy().eq_ignore_ascii_case("zip"))
            .unwrap_or(false);

    if is_zip {
        fs::create_dir_all(&dst).map_err(|e| format!("创建模板目录失败：{e}"))?;
        let out = Command::new("ditto")
            .arg("-x")
            .arg("-k")
            .arg(&src)
            .arg(&dst)
            .output()
            .map_err(|e| format!("解压失败：{e}"))?;
        if !out.status.success() {
            let _ = fs::remove_dir_all(&dst);
            return Err(format!(
                "解压失败：{}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        method = "解压".to_string();
        // 先清 __MACOSX 再拍平：否则「唯一顶层目录」的判断会被它挡掉
        if dst.join("__MACOSX").exists() {
            let _ = fs::remove_dir_all(dst.join("__MACOSX"));
            notes.push("清掉了 macOS 压缩包自带的 __MACOSX".into());
        }
        if let Some(inner) = flatten_single_root(&dst)? {
            notes.push(format!("压缩包外层是目录 {inner}，已把内容提到模板根"));
        }
    } else if src.is_dir() {
        let (root, drilled) = resolve_dir_root(&src);
        if root == src
            && !src.join(".pb-scaffold.json").is_file()
            && !src.join("package.json").is_file()
        {
            notes.push("目录里没看到 package.json，导入后请确认启动命令".into());
        }
        if drilled {
            notes.push(format!(
                "已下钻到子目录 {}",
                root.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            ));
        }
        method = clone_template(&root, &dst)?;
    } else {
        return Err("只支持选择文件夹或 .zip 压缩包".into());
    }

    // 清理：构建产物 + .git（DROP_ENTRIES），再递归扫 .DS_Store
    let before: Vec<&str> = DROP_ENTRIES.iter().copied().filter(|e| dst.join(e).exists()).collect();
    clean_target(&dst);
    if !before.is_empty() {
        notes.push(format!("清掉了 {}", before.join(" / ")));
    }
    scrub_template(&dst, &mut notes);
    if let Err(e) = ensure_placeholders(&dst, &mut notes) {
        let _ = fs::remove_dir_all(&dst); // 注入不了就回滚，别留半个坏模板
        return Err(e);
    }

    let descriptor = ScaffoldDescriptor {
        key: key.clone(),
        name,
        desc: desc.trim().to_string(),
        port_start,
        node_version: node_version.trim().trim_start_matches('v').to_string(),
        command: command.trim().to_string(),
    };
    let text = serde_json::to_string_pretty(&descriptor).map_err(|e| e.to_string())?;
    fs::write(dst.join(".pb-scaffold.json"), format!("{text}\n"))
        .map_err(|e| format!("写入 .pb-scaffold.json 失败：{e}"))?;

    let scaffold = read_scaffold(&dst).ok_or("导入完成但读取模板失败")?;
    Ok(ImportResult {
        scaffold,
        method,
        notes,
    })
}

/// 导入前体检（异步：目录可能有几十万个文件，别卡住界面）
#[tauri::command]
pub async fn probe_scaffold_source(source: String) -> Result<ScaffoldProbe, String> {
    tauri::async_runtime::spawn_blocking(move || probe_blocking(source))
        .await
        .map_err(|e| format!("体检失败：{e}"))?
}

/// 把选中的目录 / zip 导入成模板（异步：解压与克隆都可能耗时）
#[tauri::command]
pub async fn import_scaffold(
    source: String,
    key: String,
    name: String,
    desc: String,
    port_start: u16,
    node_version: String,
    command: String,
) -> Result<ImportResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        import_blocking(source, key, name, desc, port_start, node_version, command)
    })
    .await
    .map_err(|e| format!("导入失败：{e}"))?
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
        // Next 系文档站的产物 / 缓存，同样不该带进新项目
        for gen in [".next", "out", ".content-collections"] {
            fs::create_dir_all(tpl.join(gen)).unwrap();
            fs::write(tpl.join(gen).join("junk.js"), "x").unwrap();
        }
        fs::write(tpl.join(".DS_Store"), "x").unwrap();

        let dst = root.join("target");
        clone_template(&tpl, &dst).unwrap();
        clean_target(&dst);
        let touched = inject(&dst, "demo-app", 8101).unwrap();

        for gone in ["dist", ".next", "out", ".content-collections"] {
            assert!(!dst.join(gone).exists(), "{gone} 没清掉");
        }
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

    /* ---------------- 新增模板（导入目录 / zip） ---------------- */

    #[test]
    fn slugify_rules() {
        assert_eq!(slugify("My Project!"), "my-project");
        assert_eq!(slugify("a---b"), "a-b");
        assert_eq!(slugify("  --  "), "my-template");
        assert_eq!(slugify("Spell Docs"), "spell-docs");
    }

    #[test]
    fn node_version_hint_prefers_nvmrc() {
        // .nvmrc 里的版本优先
        let pkg: serde_json::Value = serde_json::json!({ "engines": { "node": ">=18.16.0" } });
        assert_eq!(node_version_hint("v22.22.2\n", Some(&pkg)), "22.22.2");
        // 没有 .nvmrc 就用 engines.node，把 >= ^ ~ 之类的修饰去掉
        assert_eq!(node_version_hint("", Some(&pkg)), "18.16.0");
        let pkg2: serde_json::Value = serde_json::json!({ "engines": { "node": "^20.1.3" } });
        assert_eq!(node_version_hint("", Some(&pkg2)), "20.1.3");
        // 不是 x.y.z 的写法不敢猜，交给系统默认版本
        let pkg3: serde_json::Value = serde_json::json!({ "engines": { "node": ">=18" } });
        assert_eq!(node_version_hint("", Some(&pkg3)), "");
        assert_eq!(node_version_hint("lts/*", Some(&pkg3)), "");
    }

    #[test]
    fn command_from_pkg_prefers_dev() {
        let p: serde_json::Value =
            serde_json::json!({ "scripts": { "serve": "zmi serve", "dev": "vite" } });
        assert_eq!(command_from_pkg(Some(&p)).as_deref(), Some("npm run dev"));
        let p2: serde_json::Value = serde_json::json!({ "scripts": { "serve": "zmi serve" } });
        assert_eq!(command_from_pkg(Some(&p2)).as_deref(), Some("npm run serve"));
        assert!(command_from_pkg(None).is_none());
    }

    #[test]
    fn zip_root_prefix_prefers_shallowest_descriptor() {
        let entries: Vec<String> = ["outer/", "outer/inner/", "outer/inner/.pb-scaffold.json", "outer/inner/package.json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(zip_root_prefix(&entries), "outer/inner");
        // 没有描述文件时退回唯一顶层目录
        let flat: Vec<String> = ["app/", "app/package.json"].iter().map(|s| s.to_string()).collect();
        assert_eq!(zip_root_prefix(&flat), "app");
        // 多个顶层目录 → 包根
        let many: Vec<String> = ["a/x", "b/y"].iter().map(|s| s.to_string()).collect();
        assert_eq!(zip_root_prefix(&many), "");
    }

    #[test]
    fn resolve_dir_root_drills_into_single_child() {
        let root = tmp("drill");
        let project = root.join("project");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("package.json"), "{}").unwrap();
        let (got, drilled) = resolve_dir_root(&root);
        assert!(drilled);
        assert_eq!(got, project);

        // 自己就是模板根时不下钻
        let (got2, drilled2) = resolve_dir_root(&project);
        assert!(!drilled2);
        assert_eq!(got2, project);

        // 两个子目录时不下钻（拿不准，交给用户）
        fs::create_dir_all(root.join("another")).unwrap();
        let (_, drilled3) = resolve_dir_root(&root);
        assert!(!drilled3);

        let _ = fs::remove_dir_all(&root);
    }

    /// 导入一个本地目录：清理产物 / 清 .git / 补占位符 / 写描述文件，全流程走一遍
    #[test]
    fn import_dir_full_pipeline() {
        let root = tmp("import");
        let lib = root.join("lib");
        let src = root.join("My Cool App");
        fs::create_dir_all(src.join("dist")).unwrap();
        fs::create_dir_all(src.join(".git")).unwrap();
        fs::create_dir_all(src.join("node_modules")).unwrap();
        fs::create_dir_all(src.join("src")).unwrap();
        fs::write(src.join(".DS_Store"), "x").unwrap();
        fs::write(src.join("src/.DS_Store"), "x").unwrap();
        fs::write(src.join(".git/config"), "[remote]").unwrap();
        fs::write(src.join("dist/junk.js"), "x").unwrap();
        fs::write(
            src.join("package.json"),
            r#"{"name":"cool-app","description":"一个示例","scripts":{"dev":"vite"}}"#,
        )
        .unwrap();
        fs::write(src.join(".nvmrc"), "20.11.0\n").unwrap();

        // 先体检
        let probe = probe_blocking(src.to_string_lossy().to_string()).unwrap();
        assert_eq!(probe.kind, "dir");
        assert!(!probe.has_descriptor);
        assert_eq!(probe.suggested.key, "my-cool-app");
        assert_eq!(probe.suggested.name, "My Cool App");
        assert_eq!(probe.suggested.desc, "一个示例");
        assert_eq!(probe.suggested.command, "npm run dev");
        assert_eq!(probe.suggested.node_version, "20.11.0");
        assert!(probe.file_count >= 5, "文件数没数出来：{}", probe.file_count);
        assert!(
            probe.warnings.iter().any(|w| w.contains(".git")),
            "没提示 .git：{:?}",
            probe.warnings
        );
        assert!(
            probe.warnings.iter().any(|w| w.contains(".pb-scaffold.json")),
            "没提示缺描述文件：{:?}",
            probe.warnings
        );

        let res = import_into(
            &lib,
            src.to_string_lossy().to_string(),
            "my-cool-app".into(),
            "我的酷应用".into(),
            "示例模板".into(),
            8400,
            "v20.11.0".into(),
            "npm run dev".into(),
        )
        .unwrap();

        let dst = lib.join("my-cool-app");
        assert!(dst.is_dir());
        assert!(!dst.join(".git").exists(), ".git 没清掉");
        assert!(!dst.join("dist").exists(), "dist 没清掉");
        assert!(!dst.join(".DS_Store").exists(), ".DS_Store 没清掉");
        assert!(!dst.join("src/.DS_Store").exists(), "子目录 .DS_Store 没清掉");
        assert!(dst.join("node_modules").is_dir(), "node_modules 被误删");
        assert!(dst.join("src").is_dir(), "src 被误删");

        // 缺 __PB_SLUG__ 时自动写进 package.json
        let pkg = fs::read_to_string(dst.join("package.json")).unwrap();
        assert!(pkg.contains(SLUG_PLACEHOLDER), "name 没补成占位符：{pkg}");

        // 描述文件字段与 camelCase 写法
        let desc: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dst.join(".pb-scaffold.json")).unwrap()).unwrap();
        assert_eq!(desc["key"], "my-cool-app");
        assert_eq!(desc["name"], "我的酷应用");
        assert_eq!(desc["portStart"], 8400);
        assert_eq!(desc["nodeVersion"], "20.11.0", "v 前缀没去掉");
        assert_eq!(desc["command"], "npm run dev");

        // 回读出来的模板：运行时字段齐全，且 key 是 camelCase 的 nodeReady
        assert_eq!(res.scaffold.key, "my-cool-app");
        let wire = serde_json::to_string(&res.scaffold).unwrap();
        assert!(wire.contains("\"nodeReady\""), "序列化没给成 camelCase：{wire}");
        assert!(!wire.contains("node_ready"), "漏了 snake_case 字段：{wire}");

        // 重复导入同一个 key 要报错，且不覆盖已有模板
        let again = import_into(
            &lib,
            src.to_string_lossy().to_string(),
            "my-cool-app".into(),
            "x".into(),
            String::new(),
            8401,
            String::new(),
            "npm run dev".into(),
        );
        assert!(again.is_err(), "同名模板居然允许覆盖");
        assert!(dst.join("package.json").exists());

        let _ = fs::remove_dir_all(&root);
    }

    /// 没有 package.json 的目录不能当模板（没地方注入，新建项目必失败）
    #[test]
    fn import_dir_requires_inject_target() {
        let root = tmp("noinject");
        let lib = root.join("lib");
        let src = root.join("plain");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("README.md"), "# hi").unwrap();

        let err = import_into(
            &lib,
            src.to_string_lossy().to_string(),
            "plain".into(),
            "空项目".into(),
            String::new(),
            8500,
            String::new(),
            "npm run dev".into(),
        )
        .unwrap_err();
        assert!(err.contains("无法作为模板使用"), "错误提示不明确：{err}");
        assert!(!lib.join("plain").exists(), "失败后留了半个模板目录");

        let _ = fs::remove_dir_all(&root);
    }

    /// zip 导入：解压、拍平外层目录、清产物、写描述文件
    #[test]
    fn import_zip_full_pipeline() {
        let root = tmp("zipimp");
        let lib = root.join("lib");
        let stage = root.join("stage");
        let inner = stage.join("zip-app");
        fs::create_dir_all(inner.join("dist")).unwrap();
        fs::write(inner.join("package.json"), r#"{"name":"__PB_SLUG__"}"#).unwrap();
        fs::write(inner.join("dist/junk.js"), "x").unwrap();
        let zip = root.join("zip-app.zip");

        // 用系统的 zip 打包（带外层目录），验证单层拍平与解压
        let ok = Command::new("zip")
            .arg("-r")
            .arg("-q")
            .arg(&zip)
            .arg("zip-app")
            .current_dir(&stage)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            eprintln!("系统没有 zip 命令，跳过 zip 用例");
            let _ = fs::remove_dir_all(&root);
            return;
        }

        let probe = probe_blocking(zip.to_string_lossy().to_string()).unwrap();
        assert_eq!(probe.kind, "zip");
        assert!(probe.file_count >= 2, "zip 条目数不对：{}", probe.file_count);

        let res = import_into(
            &lib,
            zip.to_string_lossy().to_string(),
            "zip-app".into(),
            "压缩包模板".into(),
            String::new(),
            8500,
            String::new(),
            "npm run dev".into(),
        )
        .unwrap();
        assert_eq!(res.method, "解压");

        let dst = lib.join("zip-app");
        assert!(dst.join("package.json").is_file(), "外层目录没拍平");
        assert!(!dst.join("zip-app").exists(), "还留着外层同名目录");
        assert!(!dst.join("dist").exists(), "dist 没清掉");
        assert!(res.notes.iter().any(|n| n.contains("拍平") || n.contains("提到模板根")), "没记录拍平：{:?}", res.notes);

        let _ = fs::remove_dir_all(&root);
    }

    /// 真机冒烟：拿一个**真实项目目录**（含 node_modules）走完体检 + 导入，
    /// 用临时模板库，不碰 ~/.portbutler/scaffolds。`cargo test -- --ignored`
    #[test]
    #[ignore]
    fn import_real_project_dir() {
        let src = PathBuf::from(expand_tilde(
            "~/WorkBuddy/2026-09-15-10-03-12/spell-style-docs-template",
        ));
        if !src.is_dir() {
            eprintln!("跳过：真实目录不存在 {}", src.display());
            return;
        }
        let root = tmp("realdir");
        let lib = root.join("lib");

        let t0 = std::time::Instant::now();
        let probe = probe_blocking(src.to_string_lossy().to_string()).unwrap();
        println!(
            "体检 {:.2}s → kind={} 文件={} 体积={:.1}MB key={} cmd={} node={}",
            t0.elapsed().as_secs_f32(),
            probe.kind,
            probe.file_count,
            probe.size_bytes as f64 / 1024.0 / 1024.0,
            probe.suggested.key,
            probe.suggested.command,
            probe.suggested.node_version
        );
        for w in &probe.warnings {
            println!("  警告：{w}");
        }

        let t1 = std::time::Instant::now();
        let res = import_into(
            &lib,
            src.to_string_lossy().to_string(),
            "real-docs".into(),
            "真实文档站模板".into(),
            probe.suggested.desc.clone(),
            8400,
            probe.suggested.node_version.clone(),
            probe.suggested.command.clone(),
        )
        .unwrap();
        println!(
            "导入 {:.2}s → method={} notes={:?}",
            t1.elapsed().as_secs_f32(),
            res.method,
            res.notes
        );

        let dst = lib.join("real-docs");
        assert!(dst.join("package.json").is_file());
        assert!(dst.join("node_modules").is_dir(), "依赖没跟过来");
        assert!(!dst.join(".git").exists(), ".git 没清掉");
        assert!(!dst.join(".next").exists(), ".next 没清掉");
        assert!(!dst.join(".content-collections").exists());
        let pkg = fs::read_to_string(dst.join("package.json")).unwrap();
        assert!(pkg.contains(SLUG_PLACEHOLDER), "占位符没补上");

        let _ = fs::remove_dir_all(&root);
    }

    /// 真机冒烟：真实项目打包成 zip 再导入（用 Finder 归档工具那种外层目录结构）
    #[test]
    #[ignore]
    fn import_real_project_zip() {
        let src = PathBuf::from(expand_tilde(
            "~/WorkBuddy/2026-09-15-10-03-12/spell-style-docs-template",
        ));
        if !src.is_dir() {
            eprintln!("跳过：真实目录不存在");
            return;
        }
        let root = tmp("realzip");
        let zip = root.join("real-docs.zip");
        // 只压源码，别把 node_modules 压进去（zip 用例只验结构与解压链路）
        let ok = Command::new("zip")
            .arg("-r")
            .arg("-q")
            .arg(&zip)
            .arg("spell-style-docs-template")
            .arg("-x")
            .arg("spell-style-docs-template/node_modules/*")
            .arg("-x")
            .arg("spell-style-docs-template/.next/*")
            .arg("-x")
            .arg("spell-style-docs-template/.content-collections/*")
            .current_dir(src.parent().unwrap())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            eprintln!("跳过：系统没有 zip 命令");
            return;
        }

        let probe = probe_blocking(zip.to_string_lossy().to_string()).unwrap();
        println!(
            "zip 体检 → 条目={} key={} node={} 描述文件={}",
            probe.file_count, probe.suggested.key, probe.suggested.node_version, probe.has_descriptor
        );
        assert!(probe.file_count > 5, "zip 条目太少");

        let lib = root.join("lib");
        let res = import_into(
            &lib,
            zip.to_string_lossy().to_string(),
            "zip-docs".into(),
            "压缩包文档站".into(),
            String::new(),
            8500,
            "22.22.2".into(),
            "npm run dev".into(),
        )
        .unwrap();
        println!("导入 → method={} notes={:?}", res.method, res.notes);

        let dst = lib.join("zip-docs");
        assert!(dst.join("package.json").is_file(), "外层目录没拍平");
        assert!(dst.join("app").is_dir());
        assert!(!dst.join("spell-style-docs-template").exists());
        assert!(!dst.join("__MACOSX").exists());

        let _ = fs::remove_dir_all(&root);
    }
}

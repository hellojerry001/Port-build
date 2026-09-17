use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::scaffolds::expand_tilde;

pub type ProcTable = Mutex<HashMap<String, u32>>;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub command: String,
    pub port: u16,
    /// 来源脚手架 key（手工添加的项目为空串）
    #[serde(default)]
    pub scaffold: String,
    /// 该项目需要的 Node 版本，如 "18.16.0"；为空则沿用登录 shell 默认
    #[serde(default)]
    pub node_version: String,
}

/// 删除项目的结果：最新列表 + 给用户看的结果文案 + 废纸篓里的新路径
#[derive(Serialize, Debug)]
pub struct DeleteResult {
    pub list: Vec<Project>,
    pub message: String,
    #[serde(rename = "trashPath")]
    pub trash_path: String,
}

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

pub fn data_dir() -> PathBuf {
    let dir = home_dir().join(".portbutler");
    let _ = fs::create_dir_all(dir.join("logs"));
    dir
}

pub fn load() -> Vec<Project> {
    fs::read_to_string(data_dir().join("projects.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(list: &[Project]) -> Result<(), String> {
    let json = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    fs::write(data_dir().join("projects.json"), json).map_err(|e| e.to_string())
}

/// 按 id 找项目
pub fn find(id: &str) -> Option<Project> {
    load().into_iter().find(|p| p.id == id)
}

/// 新增或更新一个项目，返回（该项目本身, 最新列表）。
/// 同时被 `save_project` 命令与「从脚手架新建项目」复用。
pub fn upsert(mut project: Project) -> Result<(Project, Vec<Project>), String> {
    let mut list = load();
    if project.id.is_empty() {
        project.id = format!(
            "p{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis()
        );
        list.push(project.clone());
    } else if let Some(x) = list.iter_mut().find(|x| x.id == project.id) {
        *x = project.clone();
    } else {
        list.push(project.clone());
    }
    save(&list)?;
    Ok((project, list))
}

/// 找到某个 Node 版本对应的 `bin` 目录。
/// 覆盖 fnm（默认目录 + macOS Application Support）与 nvm 两种常见布局。
pub fn node_bin_dir(version: &str) -> Option<PathBuf> {
    if version.is_empty() {
        return None;
    }
    let home = std::env::var("HOME").ok()?;
    let rel = format!("node-versions/v{version}/installation/bin");
    let candidates = [
        PathBuf::from(&home).join(".local/share/fnm").join(&rel),
        PathBuf::from(&home)
            .join("Library/Application Support/fnm")
            .join(&rel),
        PathBuf::from(&home)
            .join(".nvm/versions/node")
            .join(format!("v{version}/bin")),
        PathBuf::from(&home).join(".fnm").join(&rel),
    ];
    candidates.into_iter().find(|p| p.is_dir())
}

/// 把启动命令包一层：指定了 Node 版本就把它前置到 PATH。
/// 比 `fnm exec` 更快更确定 —— 不依赖 fnm 本体在 PATH 上，也省掉 fnm 的启动开销。
pub fn wrap_command(p: &Project) -> String {
    match node_bin_dir(&p.node_version) {
        Some(dir) => format!("export PATH=\"{}:$PATH\"; {}", dir.display(), p.command),
        None => p.command.clone(),
    }
}

/* ------------------------- 删除项目（含磁盘文件） ------------------------- */

fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// 无论配置怎么写，都不允许被删的目录。
fn protected_dirs() -> Vec<PathBuf> {
    let home = home_dir();
    let mut v: Vec<PathBuf> = [
        "/",
        "/Users",
        "/Applications",
        "/Library",
        "/System",
        "/Volumes",
        "/usr",
        "/bin",
        "/sbin",
        "/etc",
        "/var",
        "/opt",
        "/private",
        "/tmp",
        "/cores",
        "/Network",
        "/dev",
    ]
    .iter()
    .map(|s| canon(Path::new(s)))
    .collect();
    for name in [
        "",
        "Desktop",
        "Documents",
        "Downloads",
        "Library",
        "Pictures",
        "Movies",
        "Music",
        "Public",
        "WorkBuddy",
        ".portbutler",
        ".Trash",
        ".ssh",
    ] {
        v.push(canon(&home.join(name)));
    }
    v
}

/// 删除前的最后一道闸：解析真实路径，挡住家目录 / 系统目录 / 符号链接 / 层级过浅的路径。
/// 通过校验返回规范化后的绝对路径。
pub fn resolve_deletable(raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("项目路径为空，拒绝删除".into());
    }
    let p = expand_tilde(raw);
    if !p.is_absolute() {
        return Err("项目路径不是绝对路径，拒绝删除".into());
    }
    let meta = fs::symlink_metadata(&p).map_err(|e| format!("无法访问项目目录：{e}"))?;
    // 不跟随符号链接 —— 跟随会删掉链接指向的另一个目录
    if meta.file_type().is_symlink() {
        return Err("项目路径是符号链接，出于安全不自动删除，请手动处理".into());
    }
    if !meta.is_dir() {
        return Err("项目路径不是一个文件夹，拒绝删除".into());
    }
    let c = p
        .canonicalize()
        .map_err(|e| format!("无法解析项目路径：{e}"))?;
    if protected_dirs().iter().any(|d| d == &c) {
        return Err(format!("{} 是受保护的目录，拒绝删除", c.display()));
    }
    let home = canon(&home_dir());
    match c.strip_prefix(&home) {
        // 家目录下必须至少两级：~/WorkBuddy/proj 可以，~/proj 不行
        Ok(rel) if rel.components().count() < 2 => {
            Err("为了安全，不删除家目录下的一级目录".into())
        }
        Err(_) if c.components().count() < 3 => Err("路径层级过浅，拒绝删除".into()),
        _ => Ok(c),
    }
}

/// 在废纸篓里挑一个不重名的落点（`foo`、`foo 1`、`foo 2`…）
fn unique_dest(trash: &Path, base: &str) -> Result<PathBuf, String> {
    let first = trash.join(base);
    if !first.exists() {
        return Ok(first);
    }
    for n in 1..=500u32 {
        let p = trash.join(format!("{base} {n}"));
        if !p.exists() {
            return Ok(p);
        }
    }
    Err("废纸篓里同名目录太多，请先清理废纸篓".into())
}

/// 递归复制（仅用于跨卷回退路径）
fn copy_dir(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let to = dst.join(entry.file_name());
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else {
            fs::copy(entry.path(), &to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// 把目录移入废纸篓（`~/.Trash`）。重名自动加序号；跨卷时退回「复制 + 删原目录」。
/// 返回废纸篓里的新路径。
pub fn move_to_trash(src: &Path) -> Result<PathBuf, String> {
    let trash = home_dir().join(".Trash");
    fs::create_dir_all(&trash).map_err(|e| format!("无法访问废纸篓：{e}"))?;
    let base = src
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty() && s != "/")
        .ok_or("项目目录名异常，拒绝删除")?;
    let dest = unique_dest(&trash, &base)?;
    if fs::rename(src, &dest).is_ok() {
        return Ok(dest);
    }
    // 跨卷等场景：复制过去再删原目录
    copy_dir(src, &dest)?;
    fs::remove_dir_all(src).map_err(|e| format!("文件已复制到废纸篓，但原目录删除失败：{e}"))?;
    Ok(dest)
}

/// 终止整个进程组（先 TERM 再 KILL）
fn kill_group(pid: u32) {
    let _ = Command::new("kill")
        .args(["-TERM", &format!("-{pid}")])
        .status();
    std::thread::sleep(std::time::Duration::from_millis(600));
    let _ = Command::new("kill")
        .args(["-KILL", &format!("-{pid}")])
        .status();
}

/// 删除的真实实现：`delete_project` 命令只是它的一层薄封装，
/// 抽出来是为了能在单测里不依赖 tauri State 跑完整路径。
///
/// 顺序讲究：先确认文件能安全移走，成功后才动配置 —— 失败时配置原样保留，不会两头空。
pub fn delete_impl(id: &str, want_files: bool, procs: &ProcTable) -> Result<DeleteResult, String> {
    let p = find(id).ok_or("项目不存在")?;
    let mut trash_path = String::new();

    if want_files {
        // 先校验（不通过就直接报错，配置不动）
        let dir = resolve_deletable(&p.path)?;
        // 还在跑就先停掉，否则服务持续写文件、删不干净
        let running = procs.lock().unwrap().remove(id);
        if let Some(pid) = running {
            kill_group(pid);
        }
        let dest = move_to_trash(&dir)?;
        trash_path = dest.to_string_lossy().to_string();
    }

    let mut list = load();
    list.retain(|x| x.id != id);
    save(&list)?;
    let _ = fs::remove_file(data_dir().join("logs").join(format!("{id}.log")));

    let message = if want_files {
        format!("已删除「{}」，项目文件夹已移入废纸篓", p.name)
    } else {
        format!("已移除「{}」的项目配置（磁盘文件保留）", p.name)
    };
    Ok(DeleteResult {
        list,
        message,
        trash_path,
    })
}

#[tauri::command]
pub fn list_projects() -> Vec<Project> {
    load()
}

#[tauri::command]
pub fn save_project(project: Project) -> Result<Vec<Project>, String> {
    upsert(project).map(|(_, list)| list)
}

/// 删除项目。`delete_files` 为真时把项目目录一并移入废纸篓。
#[tauri::command]
pub fn delete_project(
    id: String,
    delete_files: Option<bool>,
    state: tauri::State<'_, ProcTable>,
) -> Result<DeleteResult, String> {
    delete_impl(&id, delete_files.unwrap_or(false), state.inner())
}

#[tauri::command]
pub fn start_project(id: String, state: tauri::State<'_, ProcTable>) -> Result<u32, String> {
    let p = find(&id).ok_or_else(|| "项目不存在".to_string())?;
    let log = fs::File::create(data_dir().join("logs").join(format!("{id}.log")))
        .map_err(|e| e.to_string())?;
    let child = Command::new("zsh")
        .arg("-lc")
        .arg(wrap_command(&p))
        .current_dir(&p.path)
        .stdout(Stdio::from(
            log.try_clone().map_err(|e| e.to_string())?,
        ))
        .stderr(Stdio::from(log))
        .process_group(0)
        .spawn()
        .map_err(|e| format!("启动失败: {e}"))?;
    let pid = child.id();
    state.lock().unwrap().insert(id.clone(), pid);
    std::mem::forget(child); // 交由 OS 托管，应用退出不连带杀进程
    Ok(pid)
}

#[tauri::command]
pub fn stop_project(id: String, state: tauri::State<'_, ProcTable>) -> Result<String, String> {
    let pid = state
        .lock()
        .unwrap()
        .get(&id)
        .copied()
        .ok_or("该进程不是端口管家启动的，请用端口雷达处理")?;
    kill_group(pid);
    state.lock().unwrap().remove(&id);
    Ok(format!("已停止进程组 {pid}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = home_dir().join(".portbutler").join(format!(".t-{tag}-{n}"));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn rejects_empty_and_relative() {
        assert!(resolve_deletable("").is_err());
        assert!(resolve_deletable("   ").is_err());
        assert!(resolve_deletable("WorkBuddy/foo").is_err());
        assert!(
            resolve_deletable("~/__pb_no_such_dir__/x").is_err(),
            "展开后目录不存在"
        );
    }

    #[test]
    fn rejects_protected_and_shallow_paths() {
        let home = home_dir();
        // 家目录本身、一级目录、系统目录全部拒绝
        for p in [
            home.to_string_lossy().to_string(),
            home.join("Desktop").to_string_lossy().to_string(),
            home.join("WorkBuddy").to_string_lossy().to_string(),
            "/".into(),
            "/Users".into(),
            "/Applications".into(),
            "/System".into(),
            "/tmp".into(),
        ] {
            assert!(resolve_deletable(&p).is_err(), "{p} 不应被允许删除");
        }
    }

    #[test]
    fn accepts_deep_dir_under_home() {
        let d = tmp_dir("ok"); // ~/.portbutler/.t-ok-xxx → 已在家目录下两级
        let got = resolve_deletable(&d.to_string_lossy()).expect("应当允许删除");
        assert_eq!(got, d.canonicalize().unwrap());
        let _ = fs::remove_dir_all(&d); // 沙箱里删不掉，尽力而为
    }

    #[test]
    fn rejects_symlink() {
        let target = tmp_dir("tgt");
        let link = home_dir().join(".portbutler").join(format!(
            ".link-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let err = resolve_deletable(&link.to_string_lossy()).unwrap_err();
        assert!(err.contains("符号链接"), "错误信息应说明原因: {err}");
        let _ = fs::remove_file(&link);
        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn unique_dest_avoids_collision() {
        let trash = tmp_dir("trash");
        assert_eq!(unique_dest(&trash, "proj").unwrap(), trash.join("proj"));
        fs::create_dir_all(trash.join("proj")).unwrap();
        assert_eq!(unique_dest(&trash, "proj").unwrap(), trash.join("proj 1"));
        fs::create_dir_all(trash.join("proj 1")).unwrap();
        assert_eq!(unique_dest(&trash, "proj").unwrap(), trash.join("proj 2"));
        let _ = fs::remove_dir_all(&trash);
    }

    /// 真机冒烟：真的把目录移进 `~/.Trash`，跑完自动清理。
    /// 手动运行：`cargo test -- --ignored trash_roundtrip`
    #[test]
    #[ignore]
    fn trash_roundtrip() {
        let src = tmp_dir("trash-src");
        fs::write(src.join("a.txt"), "hello").unwrap();
        fs::create_dir_all(src.join("node_modules")).unwrap();
        fs::write(src.join("node_modules/b.js"), "x").unwrap();

        let dest = move_to_trash(&src).unwrap();
        eprintln!("移入废纸篓: {}", dest.display());
        assert!(!src.exists(), "原目录应已不存在");
        assert!(dest.join("a.txt").exists(), "文件应完整搬过去");
        assert!(dest.join("node_modules/b.js").exists(), "子目录应完整搬过去");

        fs::remove_dir_all(&dest).unwrap(); // 还原环境
    }

    /// 临时造一个项目写进 projects.json，返回其 id（调用方负责还原 projects.json）
    fn seed_project(path: &Path, tag: &str) -> String {
        let id = format!(
            "pb-{tag}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        upsert(Project {
            id: id.clone(),
            name: format!("冒烟-{tag}"),
            path: path.to_string_lossy().to_string(),
            command: "true".into(),
            port: 9,
            scaffold: String::new(),
            node_version: String::new(),
        })
        .unwrap();
        id
    }

    /// 真机冒烟：完整删除路径 + 受保护路径护栏。
    /// 两个场景串在同一个测试里 —— 它们都要改写真实的 projects.json，
    /// 拆成两个 `#[test]` 会并行执行互相踩（一个的还原会把另一个的项目删掉）。
    /// 手动运行：`cargo test -- --ignored delete_impl_real --test-threads=1`
    #[test]
    #[ignore]
    fn delete_impl_real() {
        let json = data_dir().join("projects.json");
        let backup = fs::read(&json).ok();
        let procs: ProcTable = Mutex::new(HashMap::new());

        // ---- 场景 1：正常项目，连文件一起删 ----
        let dir = tmp_dir("del-proj");
        fs::write(dir.join("a.txt"), "hello").unwrap();
        fs::create_dir_all(dir.join("node_modules")).unwrap();
        fs::write(dir.join("node_modules/b.js"), "x").unwrap();
        let id = seed_project(&dir, "del");
        assert!(find(&id).is_some(), "项目没登记上");

        let r = delete_impl(&id, true, &procs).unwrap();
        println!("→ {}", r.message);
        println!("→ 废纸篓: {}", r.trash_path);
        let trash = PathBuf::from(&r.trash_path);

        assert!(r.list.iter().all(|p| p.id != id), "配置应已移除");
        assert!(!dir.exists(), "原目录应已被移走");
        assert!(trash.join("a.txt").exists(), "文件应完整落到废纸篓");
        assert!(
            trash.join("node_modules/b.js").exists(),
            "子目录应完整落到废纸篓"
        );
        let _ = fs::remove_dir_all(&trash);

        // ---- 场景 2：受保护路径，必须报错且配置原样保留 ----
        let guarded = home_dir().join("WorkBuddy");
        let gid = seed_project(&guarded, "guard");
        let err = delete_impl(&gid, true, &procs).unwrap_err();
        println!("→ 预期被拒: {err}");
        let config_kept = find(&gid).is_some();
        let dir_kept = guarded.exists();

        // ---- 断言都取完，再还原用户数据 ----
        match &backup {
            Some(b) => fs::write(&json, b).unwrap(),
            None => {
                let _ = fs::remove_file(&json);
            }
        }

        assert!(err.contains("受保护"), "应为受保护目录的错误: {err}");
        assert!(dir_kept, "受保护目录绝不能被删");
        assert!(config_kept, "删除失败时配置应原样保留");
    }
}

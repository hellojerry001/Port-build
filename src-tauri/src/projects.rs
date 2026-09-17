use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub type ProcTable = Mutex<HashMap<String, u32>>;

#[derive(Serialize, Deserialize, Clone)]
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

pub fn data_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let dir = PathBuf::from(home).join(".portbutler");
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

#[tauri::command]
pub fn list_projects() -> Vec<Project> {
    load()
}

#[tauri::command]
pub fn save_project(project: Project) -> Result<Vec<Project>, String> {
    upsert(project).map(|(_, list)| list)
}

#[tauri::command]
pub fn delete_project(id: String) -> Result<Vec<Project>, String> {
    let mut list = load();
    list.retain(|p| p.id != id);
    save(&list)?;
    Ok(list)
}

#[tauri::command]
pub fn start_project(id: String, state: tauri::State<'_, ProcTable>) -> Result<u32, String> {
    let p = load()
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "项目不存在".to_string())?;
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
    let _ = Command::new("kill")
        .args(["-TERM", &format!("-{pid}")])
        .status();
    std::thread::sleep(std::time::Duration::from_millis(600));
    let _ = Command::new("kill")
        .args(["-KILL", &format!("-{pid}")])
        .status();
    state.lock().unwrap().remove(&id);
    Ok(format!("已停止进程组 {pid}"))
}

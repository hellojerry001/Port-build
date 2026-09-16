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
}

fn data_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let dir = PathBuf::from(home).join(".portbutler");
    let _ = fs::create_dir_all(dir.join("logs"));
    dir
}

fn load() -> Vec<Project> {
    fs::read_to_string(data_dir().join("projects.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(list: &[Project]) -> Result<(), String> {
    let json = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    fs::write(data_dir().join("projects.json"), json).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_projects() -> Vec<Project> {
    load()
}

#[tauri::command]
pub fn save_project(mut project: Project) -> Result<Vec<Project>, String> {
    let mut list = load();
    if project.id.is_empty() {
        project.id = format!(
            "p{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis()
        );
        list.push(project);
    } else if let Some(x) = list.iter_mut().find(|x| x.id == project.id) {
        *x = project;
    } else {
        list.push(project);
    }
    save(&list)?;
    Ok(list)
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
        .arg(&p.command)
        .current_dir(&p.path)
        .stdout(Stdio::from(
            log.try_clone().map_err(|e| e.to_string())?,
        ))
        .stderr(Stdio::from(log))
        .process_group(0)
        .spawn()
        .map_err(|e| format!("启动失败: {e}"))?;
    let pid = child.id();
    state.lock().unwrap().insert(id, pid);
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

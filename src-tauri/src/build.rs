//! build.rs · Mac 项目打包（.dmg）
//! -----------------------------------------------------------------------------
//! 打包动辄几分钟，绝不能让前端干等一个同步命令。这里的做法是：
//!   `build_dmg`  只负责把进程拉起来并立刻返回 pid；
//!   `build_status` 由前端轮询，拿「还跑不跑 + 日志尾部 + 产物在哪」三件事。
//!
//! 日志落盘而不是留在内存里：进程是本应用 `forget` 掉的，应用重启后前端
//! 仍能通过 `build_status` 读到上次的输出（配合产物路径判断结果）。

use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::projects::{self, ProcTable, Project};

/// 项目 id → 打包进程 pid。只用一张表：同一项目同时只允许一个打包进程
pub type BuildTable = Mutex<HashMap<String, u32>>;

/// 回给前端的日志行数。多了没意义（cargo 输出极长），少了看不出进度
const TAIL_LINES: usize = 30;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildStatus {
    pub running: bool,
    /// 日志尾部（已剥 ANSI）
    pub tail: String,
    /// 找到的最新的 .dmg 绝对路径
    pub dmg: Option<String>,
    /// dmg 所在目录（「在访达中显示」用它）
    pub bundle_dir: Option<String>,
}

fn log_path(id: &str) -> PathBuf {
    projects::data_dir().join("logs").join(format!("{id}.build.log"))
}

/// 取日志最后 n 行。进程正在写，所以只保证「大致最新」，不保证行完整。
fn tail_text(path: &Path, n: usize) -> String {
    let s = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return String::new(),
    };
    let lines: Vec<&str> = s.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..]
        .iter()
        .map(|l| publish_strip_ansi(l).trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 复用 publish 里那个 ANSI 剥离实现，避免两处行为不一致
fn publish_strip_ansi(s: &str) -> String {
    crate::publish::strip_ansi(s)
}

/// 递归找目录下最新的 .dmg（按修改时间）
fn newest_dmg(dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = match fs::read_dir(&d) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().and_then(|s| s.to_str()) != Some("dmg") {
                continue;
            }
            let mtime = fs::metadata(&p)
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH);
            if best.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
                best = Some((mtime, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Tauri 的产物固定落在 `src-tauri/target/<profile>/bundle/dmg/`。
/// 先按标准路径找，找不到就在整个 bundle 目录里兜一圈（不同版本布局略有差异）。
fn find_dmg(root: &Path) -> Option<PathBuf> {
    let base = root.join("src-tauri").join("target");
    for profile in ["release", "debug"] {
        let dmg_dir = base.join(profile).join("bundle").join("dmg");
        if let Some(p) = newest_dmg(&dmg_dir) {
            return Some(p);
        }
        let bundle = base.join(profile).join("bundle");
        if let Some(p) = newest_dmg(&bundle) {
            return Some(p);
        }
    }
    None
}

/// 起打包进程。已在跑就报错，不让同个项目叠两个 cargo。
#[tauri::command]
pub fn build_dmg(
    id: String,
    builds: tauri::State<'_, BuildTable>,
    _procs: tauri::State<'_, ProcTable>,
) -> Result<u32, String> {
    let p: Project = projects::find(&id).ok_or("项目不存在")?;
    {
        let table = builds.lock().unwrap();
        if let Some(pid) = table.get(&id) {
            if projects::alive(*pid) {
                return Err("这个项目正在打包中".into());
            }
        }
    }

    let script = projects::wrap_cmd(&p, &projects::build_command_of(&p));
    let path = log_path(&id);
    let _ = fs::remove_file(&path); // 每次打包都是一份全新日志
    let log = projects::open_log(&format!("{id}.build.log"))?;

    let pid = projects::spawn_grouped(&p.path, &script, &log)?;
    builds.lock().unwrap().insert(id, pid);
    Ok(pid)
}

#[tauri::command]
pub fn build_status(id: String, builds: tauri::State<'_, BuildTable>) -> Result<BuildStatus, String> {
    let p = projects::find(&id).ok_or("项目不存在")?;
    let running = builds
        .lock()
        .unwrap()
        .get(&id)
        .copied()
        .map(projects::alive)
        .unwrap_or(false);
    let dmg = find_dmg(Path::new(&p.path));
    let bundle_dir = dmg
        .as_ref()
        .and_then(|d| d.parent())
        .map(|x| x.to_string_lossy().to_string());
    Ok(BuildStatus {
        running,
        tail: tail_text(&log_path(&id), TAIL_LINES),
        dmg: dmg.map(|d| d.to_string_lossy().to_string()),
        bundle_dir,
    })
}

/// 终止打包。进程组一起收掉，否则 cargo 的子进程会留在本机继续吃 CPU
#[tauri::command]
pub fn cancel_build(id: String, builds: tauri::State<'_, BuildTable>) -> Result<String, String> {
    let pid = builds
        .lock()
        .unwrap()
        .get(&id)
        .copied()
        .ok_or("这个项目没有在打包")?;
    projects::kill_group(pid);
    builds.lock().unwrap().remove(&id);
    Ok(format!("已终止打包进程组 {pid}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_last_lines_only() {
        let d = std::env::temp_dir().join(format!("pb-tail-{}", std::process::id()));
        let _ = fs::create_dir_all(&d);
        let f = d.join("x.log");
        fs::write(&f, "a\nb\nc\nd\ne").unwrap();
        assert_eq!(tail_text(&f, 3), "c\nd\ne");
        assert_eq!(tail_text(&f, 99), "a\nb\nc\nd\ne");
        assert_eq!(tail_text(&d.join("nope.log"), 3), "");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn finds_newest_dmg_recursively() {
        let root = std::env::temp_dir().join(format!("pb-dmg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let nested = root.join("src-tauri/target/release/bundle/dmg");
        fs::create_dir_all(&nested).unwrap();
        let old = nested.join("app-old.dmg");
        let new = nested.join("app-new.dmg");
        fs::write(&old, "x").unwrap();
        fs::write(&new, "x").unwrap();
        // 让 new 的 mtime 明确更新
        let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        fs::metadata(&new).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&new, "xy").unwrap();
        let _ = t;

        let got = find_dmg(&root).expect("应找到 dmg");
        assert_eq!(got.file_name().unwrap(), "app-new.dmg");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn no_dmg_returns_none() {
        let root = std::env::temp_dir().join(format!("pb-nodmg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        assert!(find_dmg(&root).is_none());
        let _ = fs::remove_dir_all(&root);
    }
}

use serde::Serialize;
use std::process::Command;

#[derive(Serialize, Clone)]
pub struct PortInfo {
    pub port: u16,
    pub pid: i32,
    pub process: String,
}

#[tauri::command]
pub fn list_ports() -> Result<Vec<PortInfo>, String> {
    let out = Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN"])
        .output()
        .map_err(|e| format!("lsof 执行失败: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut list: Vec<PortInfo> = Vec::new();
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 9 {
            continue;
        }
        let Some((_, port)) = cols[8].rsplit_once(':') else {
            continue;
        };
        let Ok(port) = port.parse::<u16>() else {
            continue;
        };
        let info = PortInfo {
            port,
            pid: cols[1].parse().unwrap_or(0),
            process: cols[0].to_string(),
        };
        if !list
            .iter()
            .any(|p| p.port == info.port && p.pid == info.pid)
        {
            list.push(info);
        }
    }
    list.sort_by_key(|p| p.port);
    Ok(list)
}

#[tauri::command]
pub fn kill_port(port: u16) -> Result<String, String> {
    let out = Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN"])
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut killed: Vec<String> = Vec::new();
    for line in text.lines().skip(1) {
        if let Some(pid) = line.split_whitespace().nth(1) {
            if Command::new("kill").args(["-9", pid]).status().is_ok() {
                killed.push(pid.to_string());
            }
        }
    }
    if killed.is_empty() {
        Err(format!("端口 {port} 当前没有监听进程"))
    } else {
        Ok(format!("已终止 PID: {}", killed.join(", ")))
    }
}

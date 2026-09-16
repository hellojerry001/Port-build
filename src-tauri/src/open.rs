use std::process::Command;

/// 用系统默认浏览器打开链接
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    // 白名单校验，只允许 http/https，且以单参数传入，杜绝命令注入
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("只允许打开 http/https 链接".into());
    }
    Command::new("open")
        .arg(&url)
        .spawn()
        .map_err(|e| format!("打开浏览器失败: {e}"))?;
    Ok(())
}

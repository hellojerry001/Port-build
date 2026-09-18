use std::path::Path;
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

/// 在访达中选中某个文件 / 文件夹（打包完的 .dmg 用它直达产物）
#[tauri::command]
pub fn show_in_finder(path: String) -> Result<(), String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("路径为空".into());
    }
    if !Path::new(path).exists() {
        return Err(format!("路径不存在：{path}"));
    }
    // -R 是「reveal」：选中并高亮，而不是把 .dmg 直接挂载起来
    Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()
        .map_err(|e| format!("打开访达失败: {e}"))?;
    Ok(())
}

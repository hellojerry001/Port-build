use serde::Serialize;
use std::path::Path;
use std::process::Command;

#[derive(Serialize)]
pub struct PublishResult {
    pub ok: bool,
    pub url: Option<String>,
    pub claim_url: Option<String>,
    pub error: Option<String>,
}

/// 用单引号包裹路径，并把路径内的单引号转义，避免 shell 注入 / 空格问题（仅兜底分支用到）
fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

/// 从命令输出里粗提取 live URL（*.workers.dev）与 claim URL（dash.cloudflare.com/...claim...）
/// 使用 str::find 而非逐字节切片，避免多字节字符（如 emoji）导致的切片 panic
fn extract_urls(s: &str) -> (Option<String>, Option<String>) {
    let bytes = s.as_bytes();
    let mut live = None;
    let mut claim = None;
    let mut start = 0;
    while let Some(pos) = s[start..].find("https://") {
        let abs = start + pos;
        let mut end = abs + 8;
        while end < bytes.len() {
            let c = bytes[end];
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r'
                || c == b'"' || c == b'\'' || c == b'<' || c == b')'
            {
                break;
            }
            end += 1;
        }
        if let Some(url) = s.get(abs..end) {
            if url.contains("workers.dev") && live.is_none() {
                live = Some(url.to_string());
            } else if url.contains("cloudflare.com") && url.contains("claim") && claim.is_none() {
                claim = Some(url.to_string());
            }
        }
        // 跳到 URL 之后继续；abs/end 均落在 ASCII 字符边界，切片安全
        start = end.max(abs + 8);
    }
    (live, claim)
}

/// 找一个 Node >=22 的可执行目录：优先用户的 petclaw node，其次 WorkBuddy 托管的 node
fn find_node22_bin() -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut candidates: Vec<String> = Vec::new();
    if !home.is_empty() {
        candidates.push(format!("{}/.petclaw/node/bin", home));
    }
    if let Ok(entries) = std::fs::read_dir("/Users/jerry/.workbuddy/binaries/node/versions") {
        for e in entries.flatten() {
            let p = e.path().join("bin");
            if p.is_dir() {
                candidates.push(p.to_string_lossy().to_string());
            }
        }
    }
    for c in &candidates {
        if Path::new(&format!("{}/node", c)).exists() {
            return Some(c.clone());
        }
    }
    None
}

/// 取今天日期（YYYY-MM-DD），用于 wrangler 的 --compatibility-date
fn today() -> String {
    match Command::new("/bin/date").arg("+%F").output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => "2026-09-01".to_string(), // 兜底：一个保证合法且 <= 今天的日期
    }
}

/// 执行部署。
/// 关键 1：用隔离的 HOME 让 wrangler 看不到本机已有的 Cloudflare 登录态，使 --temporary 匿名临时部署可用。
/// 关键 2：直接用 Node22 绝对路径执行 npx，绕开 zsh/login shell 的 PATH 重置（否则会命中系统 Node v20，
///         而 wrangler 4.x 硬性要求 Node>=22）。npm 缓存仍指向真实 ~/.npm，保证后续部署快速。
fn run_wrangler(dist: &str, name: &str, home: &Path, npm_cache: &Path) -> String {
    let date = today();
    let args = vec![
        "--yes".to_string(),
        "wrangler@latest".to_string(),
        "deploy".to_string(),
        "--temporary".to_string(),
        "--assets".to_string(),
        dist.to_string(),
        "--name".to_string(),
        name.to_string(),
        "--compatibility-date".to_string(),
        date,
    ];

    let child = if let Some(bin) = find_node22_bin() {
        Command::new(format!("{}/node", bin))
            .arg(format!("{}/npx", bin))
            .args(args)
            .env("HOME", home)
            .env("npm_config_cache", npm_cache)
            .output()
    } else {
        // 兜底：用 zsh -lc（可能命中系统 Node v20，但至少尝试一次）
        let full = format!(
            "npx --yes wrangler@latest deploy --temporary --assets {} --name {} --compatibility-date $(date +%F)",
            shell_quote(dist),
            name
        );
        Command::new("zsh")
            .arg("-lc")
            .arg(&full)
            .env("HOME", home)
            .env("npm_config_cache", npm_cache)
            .output()
    };

    match child {
        Ok(o) => format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
        Err(e) => format!("执行失败：{}", e),
    }
}

#[tauri::command]
pub fn publish_project(dist_path: String) -> PublishResult {
    let p = Path::new(&dist_path);
    if !p.exists() || !p.is_dir() {
        return PublishResult {
            ok: false,
            url: None,
            claim_url: None,
            error: Some(format!("目录不存在或不是文件夹：{}", dist_path)),
        };
    }

    // 时间戳生成唯一 worker 名（小写 + 连字符，符合 Cloudflare 命名规则）
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let name = format!("pb-{}", ts);

    // 隔离 HOME：让 wrangler 看不到本机已有的 Cloudflare 登录态
    let real_home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let tmp_home = std::env::temp_dir().join("portbutler-wrangler");
    let _ = std::fs::create_dir_all(&tmp_home);
    let npm_cache = Path::new(&real_home).join(".npm");
    let _ = std::fs::create_dir_all(&npm_cache);

    let out = run_wrangler(&dist_path, &name, &tmp_home, &npm_cache);
    let (live, claim) = extract_urls(&out);
    if live.is_some() {
        PublishResult {
            ok: true,
            url: live,
            claim_url: claim,
            error: None,
        }
    } else {
        PublishResult {
            ok: false,
            url: None,
            claim_url: claim,
            error: Some(out.trim().to_string()),
        }
    }
}

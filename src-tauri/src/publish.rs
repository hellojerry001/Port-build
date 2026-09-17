use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::publishes::{self, PublishRecord};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishResult {
    pub ok: bool,
    pub url: Option<String>,
    pub claim_url: Option<String>,
    /// 临时链接失效时刻（毫秒时间戳）；None = 长期有效
    pub expires_at: Option<i64>,
    pub error: Option<String>,
}

/// 常见的静态产物目录（相对项目根），按出现频率排序
const DIST_CANDIDATES: [&str; 14] = [
    "dist",
    "build",
    "out",
    "public",
    "_site",
    "docs/.vitepress/dist",
    "storybook-static",
    "book",
    ".output/public",
    ".next/standalone",
    "bundle",
    "web-build",
    "www",
    "site",
];

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DistCandidate {
    pub path: String,
    pub rel: String,
    pub files: usize,
    pub size_mb: f64,
    /// 该目录下有 index.html —— 更像站点入口
    pub has_index: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishProbe {
    /// 探测到的可用产物目录
    pub candidates: Vec<DistCandidate>,
    /// package.json 里的构建命令，如 "npm run build"
    pub build_cmd: String,
    /// 是否 Next.js 项目
    pub is_next: bool,
    /// Next 项目是否开了 output: 'export'（没开就不会产出 out/）
    pub static_export: bool,
    /// 项目根自带 index.html（纯静态项目，可直接发布根目录）
    pub root_static: bool,
    /// 给用户的一句话建议
    pub hint: String,
}

/// 用单引号包裹路径，并把路径内的单引号转义，避免 shell 注入 / 空格问题（仅兜底分支用到）
fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

/// 剥掉 ANSI 转义序列（wrangler 输出带颜色，原样展示会变成 [41;31m 这种乱码）
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // ESC [ ... 终止于一个字母；ESC ] ... 终止于 BEL 或 ESC \
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    for c2 in chars.by_ref() {
                        if c2.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(c2) = chars.next() {
                        if c2 == '\u{7}' {
                            break;
                        }
                        if c2 == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else {
            out.push(c);
        }
    }
    out
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

/// 跑 `node -v` 拿主版本号
fn node_major(exe: &Path) -> Option<u32> {
    let o = Command::new(exe).arg("-v").output().ok()?;
    let v = String::from_utf8_lossy(&o.stdout);
    let v = v.trim().trim_start_matches('v');
    v.split('.').next()?.parse().ok()
}

/// 找一个 Node >=22 的可执行目录。
/// wrangler 4.x 硬性要求 Node>=22，所以这里必须验版本，不能只看文件在不在。
fn find_node22_bin() -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if !home.is_empty() {
        candidates.push(PathBuf::from(format!("{}/.petclaw/node/bin", home)));
    }
    // WorkBuddy 托管的 node（多版本，逐个验）
    if let Ok(entries) = std::fs::read_dir(format!("{home}/.workbuddy/binaries/node/versions")) {
        let mut list: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path().join("bin"))
            .filter(|p| p.is_dir())
            .collect();
        list.sort();
        list.reverse(); // 版本号字符串倒序，大版本优先
        candidates.extend(list);
    }
    // 常见包管理器路径
    for p in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        candidates.push(PathBuf::from(p));
    }

    for c in candidates {
        let node = c.join("node");
        if !node.exists() {
            continue;
        }
        if let Some(major) = node_major(&node) {
            if major >= 22 {
                return Some(c.to_string_lossy().to_string());
            }
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

/// 遍历统计文件数与体积，限制深度与数量避免在大目录上卡住
fn scan_size(dir: &Path, depth: usize) -> (usize, u64) {
    if depth > 8 {
        return (0, 0);
    }
    let (mut files, mut bytes) = (0usize, 0u64);
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return (0, 0),
    };
    for e in entries.flatten() {
        if files > 20000 {
            break;
        }
        let p = e.path();
        let md = match e.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if md.is_dir() {
            let (f, b) = scan_size(&p, depth + 1);
            files += f;
            bytes += b;
        } else if md.is_file() {
            files += 1;
            bytes += md.len();
        }
    }
    (files, bytes)
}

/// 探测某个项目下可发布的产物目录，并给出针对性建议
#[tauri::command]
pub fn publish_probe(project_path: String) -> PublishProbe {
    let root = PathBuf::from(&project_path);

    // package.json 的 build 脚本
    let mut build_cmd = String::new();
    let mut has_build = false;
    if let Ok(txt) = std::fs::read_to_string(root.join("package.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
            if let Some(s) = v
                .get("scripts")
                .and_then(|s| s.get("build"))
                .and_then(|s| s.as_str())
            {
                has_build = true;
                build_cmd = format!("npm run build  →  {s}");
            }
        }
    }

    // Next.js 静态导出判定：只在产物为 out/ 时需要
    let mut is_next = false;
    let mut static_export = false;
    for name in [
        "next.config.js",
        "next.config.mjs",
        "next.config.ts",
        "next.config.cjs",
    ] {
        if let Ok(txt) = std::fs::read_to_string(root.join(name)) {
            is_next = true;
            let compact: String = txt.chars().filter(|c| !c.is_whitespace()).collect();
            static_export = compact.contains("output:'export'") || compact.contains("output:\"export\"");
            break;
        }
    }
    if !is_next && root.join("package.json").exists() {
        if let Ok(txt) = std::fs::read_to_string(root.join("package.json")) {
            if txt.contains("\"next\"") {
                is_next = true;
            }
        }
    }

    // 项目根自带 index.html 且不是 npm 项目 → 多半就是个纯静态站点，可直接发布根目录
    let root_static = root.join("index.html").is_file() && !root.join("package.json").exists();

    // 扫候选
    let mut candidates: Vec<DistCandidate> = Vec::new();
    for rel in DIST_CANDIDATES {
        let p = root.join(rel);
        if !p.is_dir() {
            continue;
        }
        let (files, bytes) = scan_size(&p, 0);
        if files == 0 {
            continue; // 空目录（常见于 build 失败或占位）不算可发布
        }
        candidates.push(DistCandidate {
            path: p.to_string_lossy().to_string(),
            rel: rel.to_string(),
            files,
            size_mb: (bytes as f64 / 1024.0 / 1024.0 * 10.0).round() / 10.0,
            has_index: p.join("index.html").is_file(),
        });
    }

    // public/ 是**源目录**不是构建产物（Vite/Next 会把它的内容拷进 dist），
    // 只在它孤零零一个候选时才列出来，免得用户把源文件发出去
    if candidates.iter().any(|c| c.rel != "public") {
        candidates.retain(|c| c.rel != "public");
    }
    // 带 index.html 的排前面 —— 那才像站点入口（sort_by_key 是稳定排序）
    candidates.sort_by_key(|c| !c.has_index);

    let hint = if !candidates.is_empty() {
        let best = candidates[0].clone();
        let tail = if best.has_index { "（含 index.html）" } else { "" };
        format!(
            "找到 {} 个可发布的目录，已自动选中「{}」{}",
            candidates.len(),
            best.rel,
            tail
        )
    } else if is_next && !static_export {
        "这是 Next.js 项目，但没开静态导出 —— `next build` 只会产出服务端版本，没有可发布的静态目录。\
         在 next.config 里加 `output: 'export'` 后重新 build，产物会落到 out/"
            .to_string()
    } else if has_build {
        "还没构建过：先在项目里执行 npm run build，产物出来后再发布".to_string()
    } else if root_static {
        "没找到构建产物，但项目根有 index.html —— 可以直接发布项目根目录".to_string()
    } else {
        "没找到任何构建产物目录（常见目录：dist / build / out）。先构建项目，或手动选择目录"
            .to_string()
    };

    PublishProbe {
        candidates,
        build_cmd,
        is_next,
        static_export,
        root_static,
        hint,
    }
}

/// 校验一个路径是不是存在的目录（供前端实时校验输入框）
#[tauri::command]
pub fn check_dir(path: String) -> bool {
    let p = Path::new(&path);
    p.exists() && p.is_dir()
}

/// 执行部署。
/// 关键 1：用隔离的 HOME 让 wrangler 看不到本机已有的 Cloudflare 登录态，使 --temporary 匿名临时部署可用。
/// 关键 2：直接用 Node22 绝对路径执行 npx，绕开 zsh/login shell 的 PATH 重置（否则会命中系统 Node v20，
///         而 wrangler 4.x 硬性要求 Node>=22）。npm 缓存仍指向真实 ~/.npm，保证后续部署快速。
/// 关键 3：**必须设置 cwd**。GUI 应用由 launchd 启动时工作目录是 `/`，而 wrangler 会在 cwd 下建
///         `.wrangler/tmp` 放临时资源 → 落到根目录 `/.wrangler/tmp`（不存在也不可写），
///         报 "Missing file or directory: /.wrangler/tmp"。指定到可写目录即可。
fn run_wrangler(dist: &str, name: &str, home: &Path, npm_cache: &Path, run_dir: &Path) -> String {
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

    // 复用同一个 cwd，但清掉上次残留的 .wrangler，避免临时文件堆积 / 误读旧状态
    let _ = std::fs::remove_dir_all(run_dir.join(".wrangler"));
    let _ = std::fs::create_dir_all(run_dir);

    let child = if let Some(bin) = find_node22_bin() {
        Command::new(format!("{}/node", bin))
            .arg(format!("{}/npx", bin))
            .args(args)
            .current_dir(run_dir)
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
            .current_dir(run_dir)
            .env("HOME", home)
            .env("npm_config_cache", npm_cache)
            .output()
    };

    match child {
        Ok(o) => strip_ansi(&format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )),
        Err(e) => format!("执行失败：{}", e),
    }
}

#[tauri::command]
pub fn publish_project(
    dist_path: String,
    project_id: Option<String>,
    project_name: Option<String>,
) -> PublishResult {
    let p = Path::new(&dist_path);
    if !p.exists() || !p.is_dir() {
        return PublishResult {
            ok: false,
            url: None,
            claim_url: None,
            expires_at: None,
            error: Some(format!(
                "目录不存在或不是文件夹：{}\n\n提示：先在项目里构建（npm run build），\
                 或点「重新探测」让应用自动找出产物目录。",
                dist_path
            )),
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
    let run_dir = tmp_home.join("run");
    let _ = std::fs::create_dir_all(&run_dir);
    let npm_cache = Path::new(&real_home).join(".npm");
    let _ = std::fs::create_dir_all(&npm_cache);

    let out = run_wrangler(&dist_path, &name, &tmp_home, &npm_cache, &run_dir);
    let (live, claim) = extract_urls(&out);
    if let Some(url) = live {
        // 只有临时部署才给认领链接；窗口时长读 wrangler 的实际输出
        // （复用旧临时账号时会小于 60 分钟，写死会导致倒计时不准）
        let expires_at = if claim.is_some() {
            let mins = publishes::parse_claim_minutes(&out)
                .unwrap_or(publishes::DEFAULT_CLAIM_MINUTES);
            Some(publishes::now_ms() + mins * 60_000)
        } else {
            None
        };
        // 落盘：让发布记录在弹窗关掉之后仍然找得回来
        publishes::push(PublishRecord {
            id: name.clone(),
            project_id: project_id.unwrap_or_default(),
            project_name: project_name.unwrap_or_default(),
            dist_path: dist_path.clone(),
            url: url.clone(),
            claim_url: claim.clone(),
            published_at: publishes::now_ms(),
            expires_at,
        });
        PublishResult {
            ok: true,
            url: Some(url),
            claim_url: claim,
            expires_at,
            error: None,
        }
    } else {
        // 把几个已知的失败模式翻译成人话
        let friendly = if out.contains("wrangler/tmp") || out.contains("could not be found") {
            format!(
                "{}\n\n（工作目录问题，请把这条报错反馈给开发者）",
                out.trim()
            )
        } else {
            out.trim().to_string()
        };
        PublishResult {
            ok: false,
            url: None,
            claim_url: claim,
            expires_at: None,
            error: Some(friendly),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_is_stripped() {
        let raw = "\u{1b}[31m✘ \u{1b}[41;31m[\u{1b}[41;97mERROR\u{1b}[41;31m]\u{1b}[0m \u{1b}[1mboom\u{1b}[0m";
        assert_eq!(strip_ansi(raw), "✘ [ERROR] boom");
    }

    #[test]
    fn urls_extracted_from_real_output() {
        let out = "Temporary account ready:\nClaim URL: https://dash.cloudflare.com/claim-preview?claimToken=abc123\n\n  https://pb-123.cord-jackal-b00.workers.dev\nCurrent Version ID: 27055184";
        let (live, claim) = extract_urls(out);
        assert_eq!(live.as_deref(), Some("https://pb-123.cord-jackal-b00.workers.dev"));
        assert_eq!(
            claim.as_deref(),
            Some("https://dash.cloudflare.com/claim-preview?claimToken=abc123")
        );
    }

    #[test]
    fn scan_size_counts_recursively() {
        let d = std::env::temp_dir().join(format!("pb-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("a.html"), "x".repeat(100)).unwrap();
        std::fs::write(d.join("sub/b.js"), "y".repeat(50)).unwrap();
        let (files, bytes) = scan_size(&d, 0);
        assert_eq!(files, 2, "应数到 2 个文件");
        assert_eq!(bytes, 150, "字节数应为 150");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn check_dir_rejects_missing_and_files() {
        assert!(!check_dir("/definitely/not/here-12345".into()));
        let f = std::env::temp_dir().join("pb-check-file.txt");
        std::fs::write(&f, "x").unwrap();
        assert!(!check_dir(f.to_string_lossy().to_string()), "文件不算目录");
        assert!(check_dir("/tmp".into()));
        let _ = std::fs::remove_file(&f);
    }

    /// 真机冒烟：cwd 必须是可写的，且 wrangler 会在其下建 .wrangler/tmp
    #[test]
    #[ignore]
    fn wrangler_runs_in_writable_cwd() {
        let run_dir = std::env::temp_dir().join("pb-wr-smoke/run");
        let _ = std::fs::create_dir_all(&run_dir);
        // 造一个最小静态站
        let site = std::env::temp_dir().join("pb-wr-smoke/site");
        std::fs::create_dir_all(&site).unwrap();
        std::fs::write(site.join("index.html"), "<h1>smoke</h1>").unwrap();

        let home = std::env::temp_dir().join("pb-wr-smoke/home");
        std::fs::create_dir_all(&home).unwrap();
        let cache = dirs_npm();
        let out = run_wrangler(
            &site.to_string_lossy(),
            &format!("pb-smoke-{}", std::process::id()),
            &home,
            &cache,
            &run_dir,
        );
        println!("--- wrangler 输出 ---\n{out}\n---------------------");
        let (live, _) = extract_urls(&out);
        assert!(live.is_some(), "应部署成功并拿到 workers.dev 链接");
        assert!(
            !out.contains("wrangler/tmp"),
            "不应再出现 /.wrangler/tmp 报错"
        );
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("pb-wr-smoke"));
    }

    #[test]
    fn probe_drops_public_when_other_candidates_exist() {
        let root = std::env::temp_dir().join(format!("pb-probe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("dist")).unwrap();
        std::fs::write(root.join("dist/index.html"), "<h1>x</h1>").unwrap();
        std::fs::create_dir_all(root.join("public")).unwrap();
        std::fs::write(root.join("public/logo.svg"), "<svg/>").unwrap();

        let r = publish_probe(root.to_string_lossy().to_string());
        let rels: Vec<&str> = r.candidates.iter().map(|c| c.rel.as_str()).collect();
        assert!(
            !rels.contains(&"public"),
            "有 dist 时不该把 public 列为候选，实际拿到 {rels:?}"
        );
        assert_eq!(rels.first(), Some(&"dist"), "应自动选中 dist");
        assert!(r.candidates[0].has_index, "dist 含 index.html");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn probe_lists_public_when_it_is_the_only_one() {
        let root = std::env::temp_dir().join(format!("pb-probe2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("public")).unwrap();
        std::fs::write(root.join("public/index.html"), "x").unwrap();

        let r = publish_probe(root.to_string_lossy().to_string());
        assert_eq!(r.candidates.len(), 1, "孤身一个时 public 应予保留");
        assert_eq!(r.candidates[0].rel, "public");
        assert!(r.candidates[0].has_index);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn dirs_npm() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(".npm")
    }
}

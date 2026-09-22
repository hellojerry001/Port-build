/* =============================================================================
   update.rs · 关于页与版本更新
   -----------------------------------------------------------------------------
   版本清单托管在仓库根的 release.json，检查更新 = GET 它的 raw 直链。
   raw.githubusercontent.com 返回 `access-control-allow-origin: *`，而应用的
   CSP 是 null，所以「取清单」这一步交给前端 fetch，后端不必背一个 HTTP 客户端。

   后端只做两件前端做不到的事：
     1. app_info / check_manifest —— 应用自身元信息；清单解析与版本比较
        （放后端是为了可单测：版本号比较规则是这功能里唯一容易写错的地方）
     2. download_dmg —— 用系统自带 /usr/bin/curl 起进程组下载到 ~/Downloads
   ============================================================================= */

use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use crate::icon;
use crate::projects;

/// 版本清单所在仓库。改这里前先确认 release.json 已在仓库根目录。
const REPO: &str = "hellojerry001/Port-build";
const BRANCH: &str = "main";
const MANIFEST: &str = "release.json";

/// 下载源白名单：清单被改坏或被伪造时，也不至于把包下到任意地址
const ALLOWED_HOSTS: [&str; 3] = [
    "github.com",
    "objects.githubusercontent.com",
    "raw.githubusercontent.com",
];

/// 文件名长度上限（含扩展名）
const NAME_MAX: usize = 120;

const CURL: &str = "/usr/bin/curl";

/* ============================== 应用元信息 ============================== */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub identifier: String,
    /// 内嵌图标的裸 base64（前端自己拼 data: 前缀）
    pub icon: String,
    pub repo: String,
    pub manifest_url: String,
    pub issues_url: String,
    pub releases_url: String,
}

#[tauri::command]
pub fn app_info(app: tauri::AppHandle) -> AppInfo {
    let pi = app.package_info();
    let cfg = app.config();

    AppInfo {
        // productName 是 .app 文件名与 Dock 显示名的来源；没配才退回 crate 名
        name: cfg
            .product_name
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| pi.name.clone()),
        version: pi.version.to_string(),
        identifier: cfg.identifier.clone(),
        // 编译期就嵌进二进制的图标 —— 关于页显示的必然是「这个版本」的图标
        icon: icon::b64(include_bytes!("../icons/icon.png")),
        repo: REPO.to_string(),
        manifest_url: format!("https://raw.githubusercontent.com/{REPO}/{BRANCH}/{MANIFEST}"),
        issues_url: format!("https://github.com/{REPO}/issues"),
        releases_url: format!("https://github.com/{REPO}/releases"),
    }
}

/* ============================== 版本清单 ============================== */

/// release.json 的结构。字段全部给默认值 —— 清单是人手写的，
/// 少写一项不该让整个「检查更新」失败，能显示多少显示多少。
#[derive(Deserialize, Default)]
#[serde(default)]
struct Manifest {
    version: String,
    date: String,
    notes: Vec<String>,
    dmg: String,
    dmg_name: String,
    dmg_size: u64,
    min_macos: String,
    mandatory: bool,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub current: String,
    pub latest: String,
    pub has_update: bool,
    pub date: String,
    pub notes: Vec<String>,
    pub dmg_url: String,
    pub dmg_name: String,
    pub dmg_size: u64,
    pub min_macos: String,
    pub mandatory: bool,
}

/// 解析清单并和当前版本比对。前端 fetch 到原文后原样传进来，
/// 解析与版本比较都留在这一侧，方便单测。
#[tauri::command]
pub fn check_manifest(text: String, current: String) -> Result<UpdateCheck, String> {
    let m: Manifest = serde_json::from_str(&text)
        .map_err(|e| format!("版本清单不是合法 JSON：{e}"))?;

    if m.version.trim().is_empty() {
        return Err("版本清单里没有 version 字段".into());
    }

    let latest = m.version.trim().to_string();
    // dmg 给了完整地址就按它下；只给了文件名则拼 Release 资产直链
    let dmg_url = if m.dmg.trim().is_empty() {
        if m.dmg_name.trim().is_empty() {
            String::new()
        } else {
            format!(
                "https://github.com/{REPO}/releases/download/v{latest}/{}",
                m.dmg_name.trim()
            )
        }
    } else {
        m.dmg.trim().to_string()
    };
    let dmg_name = if m.dmg_name.trim().is_empty() {
        file_name_of(&dmg_url).unwrap_or_default()
    } else {
        m.dmg_name.trim().to_string()
    };

    Ok(UpdateCheck {
        has_update: version_gt(&latest, current.trim()),
        current: current.trim().to_string(),
        latest,
        date: m.date.trim().to_string(),
        notes: m.notes,
        dmg_url,
        dmg_name,
        dmg_size: m.dmg_size,
        min_macos: m.min_macos.trim().to_string(),
        mandatory: m.mandatory,
    })
}

/* ============================== 下载 DMG ============================== */

/// 项目 id → 打包进程。⚠️ 必须是 newtype：
/// Tauri 的 `.manage()` 按 `TypeId` 去重，类型别名不产生新类型，
/// 写成 `type X = Mutex<..>` 会和别的表撞成同一个 TypeId，启动即 panic。
#[derive(Default)]
pub struct DownloadTable(Mutex<Option<Job>>);

struct Job {
    /// 持有 Child 而不是裸 pid：退出时靠 `try_wait()` 回收，
    /// 不会留下骗过 `kill -0` 的僵尸进程（见 build.rs 的同类修复）。
    child: Child,
    path: PathBuf,
    total: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStatus {
    /// 表里有任务（含已结束、尚未消费的）
    pub active: bool,
    pub running: bool,
    pub done: bool,
    pub got: u64,
    pub total: u64,
    pub name: String,
    pub path: String,
    pub error: Option<String>,
}

#[tauri::command]
pub fn download_dmg(
    url: String,
    name: String,
    table: tauri::State<'_, DownloadTable>,
) -> Result<String, String> {
    let url = url.trim().to_string();
    check_download_url(&url)?;
    let name = clean_file_name(&name)?;

    {
        let mut slot = table.inner().0.lock().unwrap();
        // 上一单已经结束（try_wait 有值）就允许开新的
        if let Some(job) = slot.as_mut() {
            if matches!(job.child.try_wait(), Ok(None)) {
                return Err("已有一个下载在进行中".into());
            }
        }
        *slot = None;
    }

    let dir = downloads_dir()?;
    fs::create_dir_all(&dir).map_err(|e| format!("建下载目录失败：{e}"))?;
    let path = dir.join(&name);

    let log = fs::File::create(projects::data_dir().join("logs/download.log"))
        .map_err(|e| format!("建下载日志失败：{e}"))?;

    let child = Command::new(CURL)
        .args(["-L", "--fail", "--silent", "--show-error", "--output"])
        .arg(&path)
        .arg(&url)
        .stdout(Stdio::from(log.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::from(log))
        .stdin(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("启动下载失败：{e}"))?;

    // 总大小提前问一次，进度才有分母；拿不到就按 0 处理（前端只显示已下载量）
    let total = content_length(&url);
    *table.inner().0.lock().unwrap() = Some(Job {
        child,
        path,
        total,
    });

    Ok(format!("开始下载 {name}"))
}

#[tauri::command]
pub fn download_status(table: tauri::State<'_, DownloadTable>) -> Result<DownloadStatus, String> {
    let mut slot = table.inner().0.lock().unwrap();
    let Some(job) = slot.as_mut() else {
        return Ok(DownloadStatus {
            active: false,
            running: false,
            done: false,
            got: 0,
            total: 0,
            name: String::new(),
            path: String::new(),
            error: None,
        });
    };

    let got = fs::metadata(&job.path).map(|m| m.len()).unwrap_or(0);
    let name = file_name_of(&job.path.to_string_lossy()).unwrap_or_default();
    let (running, done, error) = match job.child.try_wait() {
        // 还在跑
        Ok(None) => (true, false, None),
        Ok(Some(st)) if st.success() && got > 0 => (false, true, None),
        Ok(Some(st)) => (
            false,
            false,
            Some(format!(
                "下载未完成（curl 退出码 {}），详情见 ~/.portbutler/logs/download.log",
                st.code().unwrap_or(-1)
            )),
        ),
        Err(e) => (false, false, Some(format!("读进程状态失败：{e}"))),
    };

    Ok(DownloadStatus {
        active: true,
        running,
        done,
        got,
        total: job.total,
        name,
        path: job.path.to_string_lossy().to_string(),
        error,
    })
}

/// 取消下载：进程组一起收掉，并删掉下了一半的文件
#[tauri::command]
pub fn cancel_download(table: tauri::State<'_, DownloadTable>) -> Result<String, String> {
    let mut slot = table.inner().0.lock().unwrap();
    let Some(job) = slot.as_mut() else {
        return Err("没有正在进行的下载".into());
    };
    let pid = job.child.id();
    projects::kill_group(pid);
    // kill_group 之后进程已死，再 try_wait 一次把僵尸收掉
    let _ = job.child.try_wait();
    let path = job.path.clone();
    *slot = None;
    let _ = fs::remove_file(&path);
    Ok(format!("已取消下载 {} ({pid})", path.to_string_lossy()))
}

/* ============================== 纯逻辑（可单测） ============================== */

/// 版本号比较：`a` 是否比 `b` 新。
/// 只认主流的 `主.次.修订` 三段数字，缺位补 0；带预发布后缀（如 `0.4.0-beta.1`）
/// 的一律判为**比同号正式版旧** —— 语义化版本的规矩，也符合直觉。
/// `install.rs` 也用它拦「装回旧版本」，所以是 pub(crate)。
pub(crate) fn version_gt(a: &str, b: &str) -> bool {
    let (an, apre) = parse_version(a);
    let (bn, bpre) = parse_version(b);
    let n = an.len().max(bn.len());
    for i in 0..n {
        let x = *an.get(i).unwrap_or(&0);
        let y = *bn.get(i).unwrap_or(&0);
        if x != y {
            return x > y;
        }
    }
    // 数字段相同：没有后缀的更新
    match (apre, bpre) {
        (false, true) => true,
        (true, false) => false,
        _ => false,
    }
}

fn parse_version(v: &str) -> (Vec<u64>, bool) {
    let v = v.trim().trim_start_matches('v');
    let (nums, pre) = match v.split_once('-') {
        Some((head, _)) => (head, true),
        None => (v, false),
    };
    let parts = nums
        .split('.')
        .map(|p| p.trim().parse::<u64>().unwrap_or(0))
        .collect();
    (parts, pre)
}

/// 下载地址校验：必须 https，且主机在白名单里
fn check_download_url(url: &str) -> Result<(), String> {
    let host = host_of(url).ok_or("下载地址必须是 https 链接")?;
    if ALLOWED_HOSTS.contains(&host.as_str()) {
        Ok(())
    } else {
        Err(format!(
            "下载地址的主机 {host} 不在允许列表里（只允许从 GitHub 取包）"
        ))
    }
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.trim().strip_prefix("https://")?;
    let host_port = rest.split(['/', '?', '#']).next()?;
    let host_port = host_port.rsplit('@').next()?; // 去掉 userinfo
    let host = host_port.split(':').next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

fn file_name_of(path: &str) -> Option<String> {
    let p = path.trim().trim_end_matches('/');
    let name = p.rsplit('/').next()?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// 只接受「单纯的文件名」：不含路径分隔符、不以 `.` 开头、不含控制字符。
/// 清单是从网上拉下来的，文件名最终会拼进 `~/Downloads/`，
/// 出现 `../` 就能写到任意位置，所以这几条必须卡死；
/// 其余可见字符（含空格、括号、中文）一律放行 —— 卡太死只会让正常的
/// 资产名（如 `VibeButler 0.4.0 (2).dmg`）被莫名其妙拒掉。
fn clean_file_name(raw: &str) -> Result<String, String> {
    let n = raw.trim();
    if n.is_empty() {
        return Err("文件名不能为空".into());
    }
    if n.chars().count() > NAME_MAX {
        return Err(format!("文件名最多 {NAME_MAX} 个字符"));
    }
    if n.starts_with('.') {
        return Err("文件名不能以 . 开头".into());
    }
    // `:` 在 macOS 的文件系统层会被当成路径分隔符，一并拒掉
    if n.contains('/') || n.contains('\\') || n.contains(':') {
        return Err("文件名不能包含路径分隔符".into());
    }
    if n.chars().any(|c| c.is_control()) {
        return Err("文件名不能包含控制字符".into());
    }
    Ok(n.to_string())
}

fn downloads_dir() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "读不到 HOME 环境变量".to_string())?;
    if home.trim().is_empty() {
        return Err("HOME 为空".into());
    }
    Ok(PathBuf::from(home).join("Downloads"))
}

/// 问一次总大小（跟随 302，取最后一跳的 content-length）。拿不到返回 0。
fn content_length(url: &str) -> u64 {
    let out = match Command::new(CURL)
        .args(["-sIL", "--max-time", "12", url])
        .output()
    {
        Ok(o) => o,
        Err(_) => return 0,
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            if k.trim().eq_ignore_ascii_case("content-length") {
                v.trim().parse::<u64>().ok()
            } else {
                None
            }
        })
        .last()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_by_segment_not_lexically() {
        assert!(version_gt("0.4.0", "0.3.0"));
        // 字符串比较会在这里判错：'1' < '9'
        assert!(version_gt("0.10.0", "0.9.0"));
        assert!(version_gt("1.0.0", "0.99.99"));
        assert!(!version_gt("0.3.0", "0.3.0"));
        assert!(!version_gt("0.2.9", "0.3.0"));
    }

    #[test]
    fn handles_missing_segments_and_v_prefix() {
        assert!(version_gt("0.4", "0.3.9"));
        assert!(version_gt("v0.4.0", "0.3.0"));
        assert!(version_gt("1.0.1", "1"));
        // 缺位补 0 之后两者相等，不算更新
        assert!(!version_gt("1.0.0", "1"));
        assert!(!version_gt("0.3", "0.3.0"));
    }

    #[test]
    fn prerelease_is_older_than_the_release() {
        assert!(version_gt("0.4.0", "0.4.0-beta.1"));
        assert!(!version_gt("0.4.0-beta.1", "0.4.0"));
        assert!(!version_gt("0.4.0-beta.2", "0.4.0-beta.1"));
    }

    #[test]
    fn manifest_missing_fields_do_not_break_the_check() {
        // 只写版本号也不该报错，其余字段留空
        let got = check_manifest(r#"{"version":"0.4.0"}"#.into(), "0.3.0".into()).unwrap();
        assert!(got.has_update);
        assert_eq!(got.latest, "0.4.0");
        assert!(got.notes.is_empty());
        assert!(got.dmg_url.is_empty());
        assert!(!got.mandatory);
    }

    #[test]
    fn manifest_derives_release_download_url_from_name() {
        let got = check_manifest(
            r#"{"version":"0.4.0","dmg_name":"VibeButler_0.4.0_aarch64.dmg","notes":["a","b"]}"#
                .into(),
            "0.3.0".into(),
        )
        .unwrap();
        assert_eq!(
            got.dmg_url,
            "https://github.com/hellojerry001/Port-build/releases/download/v0.4.0/VibeButler_0.4.0_aarch64.dmg"
        );
        assert_eq!(got.dmg_name, "VibeButler_0.4.0_aarch64.dmg");
        assert_eq!(got.notes, vec!["a", "b"]);
    }

    #[test]
    fn manifest_explicit_url_wins_and_gives_back_the_name() {
        let got = check_manifest(
            r#"{"version":"0.4.0","dmg":"https://github.com/o/r/releases/download/v0.4.0/Pick.dmg"}"#
                .into(),
            "0.3.0".into(),
        )
        .unwrap();
        assert_eq!(got.dmg_name, "Pick.dmg");
    }

    #[test]
    fn manifest_without_version_is_an_error() {
        assert!(check_manifest(r#"{"date":"2026-01-01"}"#.into(), "0.3.0".into())
            .unwrap_err()
            .contains("没有 version"));
        assert!(check_manifest("not json".into(), "0.3.0".into()).is_err());
    }

    #[test]
    fn same_version_is_not_an_update() {
        let got = check_manifest(r#"{"version":"0.3.0"}"#.into(), "0.3.0".into()).unwrap();
        assert!(!got.has_update);
    }

    #[test]
    fn only_github_https_hosts_are_allowed_to_download() {
        assert!(check_download_url(
            "https://github.com/hellojerry001/Port-build/releases/download/v0.4.0/x.dmg"
        )
        .is_ok());
        assert!(check_download_url("https://objects.githubusercontent.com/x.dmg").is_ok());
        // 明文 http 拒绝
        assert!(check_download_url("http://github.com/x.dmg").is_err());
        // 非白名单主机拒绝
        assert!(check_download_url("https://evil.example.com/x.dmg").is_err());
        // 伪装成 github 子域名的也不行
        assert!(check_download_url("https://github.com.evil.com/x.dmg").is_err());
        assert!(check_download_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn host_parsing_ignores_userinfo_and_port() {
        assert_eq!(
            host_of("https://user:pw@github.com:443/a/b").unwrap(),
            "github.com"
        );
        assert_eq!(host_of("https://GitHub.com/x").unwrap(), "github.com");
        assert!(host_of("https://").is_none());
    }

    #[test]
    fn file_name_rejects_path_traversal() {
        assert!(clean_file_name("../../etc/passwd").is_err());
        assert!(clean_file_name("a/b.dmg").is_err());
        assert!(clean_file_name("a\\b.dmg").is_err());
        assert!(clean_file_name("a:b.dmg").is_err());
        assert!(clean_file_name(".hidden").is_err());
        assert!(clean_file_name("").is_err());
        assert!(clean_file_name("带\t制表符.dmg").is_err());
        assert!(clean_file_name(&"a".repeat(NAME_MAX + 1)).is_err());
        // 正常文件名放行：空格、括号、加号、中文都是合法的资产名用字
        assert_eq!(
            clean_file_name("VibeButler_0.4.0_aarch64.dmg").unwrap(),
            "VibeButler_0.4.0_aarch64.dmg"
        );
        assert!(clean_file_name("Vibe Butler 0.4.0 (build 2).dmg").is_ok());
        assert!(clean_file_name("版本管家 0.4.0.dmg").is_ok());
        // 首尾空白会被规整掉
        assert_eq!(clean_file_name("  a.dmg  ").unwrap(), "a.dmg");
    }
}

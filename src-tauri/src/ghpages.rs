//! 发布静态产物到 GitHub Pages（托管设置里的「GitHub 托管」）。
//!
//! 与 Cloudflare 那条路（wrangler 匿名临时部署）最大的不同：**这是长期托管**。
//! 产物被推到一个分支，Pages 从该分支构建，地址形如
//! `https://<owner>.github.io/<repo>/`，不失效、不需要认领。
//!
//! 两条设计原则：
//!
//! 1. **不碰 token**（推送那一步）。临时仓库里 `git push` 复用用户的 credential
//!    helper（本机是 osxkeychain），与应用绑定账号时写入钥匙串的那份凭证同源。
//!    只有调 REST API（建仓库 / 开 Pages）才取一次 token，且经 stdin 交给 curl。
//! 2. **给用户留退路**。Pages 开不了（权限、private 仓库）不该让整个发布失败 ——
//!    分支已经推上去了，此时报错要带上「去仓库设置里手动开」的链接。

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::git;
use crate::publishes;

/* ============================== 纯函数（可单测，不打网络） ============================== */

/// 项目名 → 合法的 GitHub 仓库名。
///
/// GitHub 只接受 `[A-Za-z0-9._-]`，且仓库名大小写不敏感，统一转小写更省心。
/// 中文项目名会被全部转成连字符，此时返回空串，调用方需要自己兜底一个名字。
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    // 仓库名上限 100 字符，留出前缀的余地
    trimmed.chars().take(80).collect::<String>().trim_end_matches('-').to_string()
}

/// 仓库名前缀归一：`pb` / `pb-` / `PB_` → `pb-`；空 → 空（表示不加前缀）。
///
/// 特意**不**用 `slugify` —— 它会把 `pb-` 的尾部连字符裁掉，拼出来就成了 `pbmyapp`。
pub fn normalize_repo_prefix(s: &str) -> String {
    let cleaned: String = s
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}-")
    }
}

/// 从 git remote 地址解析 `(owner, repo)`，只认 github.com。
///
/// 覆盖四种写法（`git remote get-url` 会原样返回用户当初 add 的那个）：
/// `https://github.com/o/r.git`、`http://…`、`git@github.com:o/r.git`、`ssh://git@github.com/o/r.git`。
/// 非 GitHub 的远端（GitLab / 自建）返回 None —— 那种情况我们要另外建仓库，而不是往上推。
pub fn parse_github_remote(url: &str) -> Option<(String, String)> {
    let s = url.trim();
    if s.is_empty() {
        return None;
    }

    // 先切出 github.com 之后的路径部分
    let rest = if let Some(idx) = s.find("github.com") {
        let after = &s[idx + "github.com".len()..];
        // `git@github.com:owner/repo` 用冒号分隔；其余（含 ssh://git@github.com/）用斜杠
        after.trim_start_matches([':', '/']).to_string()
    } else {
        return None;
    };

    let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        return None;
    }
    let owner = parts[0].trim().to_string();
    let repo = parts[1].trim().trim_end_matches(".git").trim().to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}

/// Pages 站点地址。
///
/// 仓库名等于 `<owner>.github.io` 时是用户主页，地址里没有仓库段 —— 这是
/// GitHub Pages 的既定规则，多一段就会 404。
pub fn pages_url(owner: &str, repo: &str) -> String {
    if repo.eq_ignore_ascii_case(&format!("{owner}.github.io")) {
        format!("https://{owner}.github.io/")
    } else {
        format!("https://{owner}.github.io/{repo}/")
    }
}

/// 待发布的产物目录名 -> 提交信息。带上时间，方便在仓库历史里对账。
fn commit_message(ts_ms: i64) -> String {
    let secs = ts_ms / 1000;
    format!("publish: VibeButler {secs}")
}

/* ============================== 目标仓库 ============================== */

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub owner: String,
    pub repo: String,
    /// true = 本次新建的仓库（前端要明确告诉用户建了个 public 仓库）
    pub created: bool,
}

/// 取当前 token 对应的登录名（API 权威来源，不依赖本地设置里的缓存）。
fn current_login() -> Result<String, String> {
    let (code, body) = git::gh_api("https://api.github.com/user", "GET", None)?;
    if code != 200 {
        return Err(format!("查询 GitHub 账号失败（HTTP {code}）：{}", brief(&body)));
    }
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("解析账号信息失败：{e}"))?;
    v.get("login")
        .and_then(|l| l.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "响应里没有 login 字段".to_string())
}

/// 定出目标仓库：项目已有 GitHub 远端就用它（只往产物分支推，不碰 main），
/// 否则按 `<前缀><项目名>` 新建一个 public 仓库（同名已存在则直接复用）。
pub fn resolve_target(project_path: &str, project_name: &str, prefix: &str) -> Result<Target, String> {
    if let Ok(remote) = git::origin_url(project_path) {
        if let Some((owner, repo)) = parse_github_remote(&remote) {
            return Ok(Target { owner, repo, created: false });
        }
    }

    let owner = current_login()?;
    let slug = slugify(project_name);
    let repo = if slug.is_empty() {
        format!("{prefix}site")
    } else {
        format!("{prefix}{slug}")
    };

    // 幂等：同名仓库已经存在就直接用（用户上次发布建的，或本来就有的）
    let (code, _body) = git::gh_api(
        &format!("https://api.github.com/repos/{owner}/{repo}"),
        "GET",
        None,
    )?;
    if code == 200 {
        return Ok(Target { owner, repo, created: false });
    }

    let create_body = serde_json::json!({
        "name": repo,
        "description": "静态站点（由 VibeButler 发布）",
        // 免费账号的 Pages 只支持 public 仓库，所以这里必须是 public
        "private": false,
        "auto_init": false,
        "has_issues": false,
        "has_wiki": false,
    });
    let (code, body) = git::gh_api("https://api.github.com/user/repos", "POST", Some(&create_body))?;
    if !(200..300).contains(&code) {
        return Err(format!(
            "创建仓库 {owner}/{repo} 失败（HTTP {code}）：{}\n\n\
             如果提示权限不足，请重新绑定 GitHub 账号并勾选 repo 权限。",
            brief(&body)
        ));
    }
    Ok(Target { owner, repo, created: true })
}

/* ============================== 推送产物 ============================== */

/// 在指定目录跑 git，返回 stdout+stderr（成功与否都返回，由调用方判断）。
///
/// `GIT_TERMINAL_PROMPT=0` 是关键：GUI 里没有终端可交互，缺凭证时 git 默认会
/// 挂在那里等输入 —— 表现为「发布永远转圈」。关掉提示让它立刻失败，
/// 我们才能把「凭证失效，请重新绑定」这条有用的信息返回给用户。
fn git_in(dir: &Path, args: &[&str]) -> Result<(bool, String), String> {
    let out = Command::new(git::git_bin())
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("执行 git 失败：{e}"))?;
    let text = crate::publish::strip_ansi(&format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    ));
    Ok((out.status.success(), text))
}

/// 把目录整个克隆到暂存目录（APFS 写时复制，几百 MB 也是秒级）。
/// 不用 `cp -R`：那会真复制一遍数据，大产物目录下慢一个数量级。
fn clone_dir(src: &Path, dst: &Path) -> Result<(), String> {
    if dst.exists() {
        fs::remove_dir_all(dst).map_err(|e| format!("清理暂存目录失败：{e}"))?;
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建暂存目录失败：{e}"))?;
    }
    let out = Command::new("cp")
        .args(["-Rc"])
        .arg(src)
        .arg(dst)
        .output()
        .map_err(|e| format!("复制产物失败：{e}"))?;
    if !out.status.success() {
        // APFS 克隆在跨卷（如外置盘）时会失败，退回真复制
        let out2 = Command::new("cp")
            .args(["-R"])
            .arg(src)
            .arg(dst)
            .output()
            .map_err(|e| format!("复制产物失败：{e}"))?;
        if !out2.status.success() {
            return Err(format!(
                "复制产物失败：{}",
                String::from_utf8_lossy(&out2.stderr).trim()
            ));
        }
    }
    Ok(())
}

/// 把 dist 推到 `<branch>` 分支。
///
/// 中间过程全在暂存目录里做，**不碰用户的产物目录**（不写 `.git`、不改文件）。
pub fn push_branch(
    dist: &Path,
    owner: &str,
    repo: &str,
    branch: &str,
    staging: &Path,
    ident: (&str, &str),
) -> Result<(), String> {
    let url = format!("https://github.com/{owner}/{repo}.git");
    push_to(dist, &url, branch, staging, ident)
}

/// 推送到任意远端地址。
///
/// 与 `push_branch` 拆开是为了能测 —— 测试里拿本地裸仓库当远端，
/// 就能在不碰 GitHub 的前提下验证整条 git 链路（见文件末尾的 `--ignored` 干跑）。
fn push_to(
    dist: &Path,
    remote_url: &str,
    branch: &str,
    staging: &Path,
    ident: (&str, &str),
) -> Result<(), String> {
    clone_dir(dist, staging)?;

    // ⚠️ 少了 .nojekyll，Pages 会拿 Jekyll 处理产物：以 `_` 开头的目录
    // （Next 的 `_next`、Astro 的 `_astro`、Jekyll 自己的 `_site`…）会被整目录忽略，
    // 表现是线上白屏、资源 404。产物里本来带一份的话别覆盖。
    let njekyll = staging.join(".nojekyll");
    if !njekyll.exists() {
        fs::write(&njekyll, b"").map_err(|e| format!("写入 .nojekyll 失败：{e}"))?;
    }

    let (ok, out) = git_in(staging, &["init", "-q"])?;
    if !ok {
        return Err(format!("git init 失败：{out}"));
    }
    // -f 连被 ignore 的文件一起加：产物目录里若带了 .gitignore（或用户配了全局
    // excludesFile），少了这个参数会静默漏文件，线上表现是缺资源。
    let (ok, out) = git_in(staging, &["add", "-A", "-f"])?;
    if !ok {
        return Err(format!("暂存产物失败：{out}"));
    }

    // commit.gpgsign=false：用户若全局开了签名，提交会要求 GPG 私钥而卡住/失败
    let (name, email) = ident;
    let msg = commit_message(publishes::now_ms());
    let (ok, out) = git_in(
        staging,
        &[
            "-c",
            "commit.gpgsign=false",
            "-c",
            &format!("user.name={}", if name.is_empty() { "VibeButler" } else { name }),
            "-c",
            &format!(
                "user.email={}",
                if email.is_empty() { "portbutler@localhost" } else { email }
            ),
            "commit",
            "-q",
            "-m",
            &msg,
        ],
    )?;
    if !ok {
        return Err(format!("生成提交失败：{out}"));
    }

    let refspec = format!("HEAD:refs/heads/{branch}");
    let (ok, out) = git_in(staging, &["push", "-f", remote_url, &refspec])?;
    if !ok {
        let hint = if out.contains("could not read Username")
            || out.contains("Authentication failed")
            || out.contains("403")
        {
            "\n\n（推送凭证不可用：请到「项目仓库」页重新绑定 GitHub 账号）"
        } else {
            ""
        };
        return Err(format!("推送失败：{}{hint}", out.trim()));
    }
    Ok(())
}

/* ============================== 开启 Pages ============================== */

/// 开启（或改指向）Pages。已开启时 GitHub 返回 409，这时改用 PUT 把 source 改到我们的分支。
pub fn enable_pages(owner: &str, repo: &str, branch: &str) -> Result<String, String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/pages");
    let body = serde_json::json!({
        "source": { "branch": branch, "path": "/" },
        // legacy = 直接从分支构建（我们不需要 workflow 文件）
        "build_type": "legacy",
    });

    let (code, resp) = git::gh_api(&url, "POST", Some(&body))?;
    if (200..300).contains(&code) {
        return Ok("已开启".into());
    }
    if code == 409 {
        // 已经开着 Pages（可能指向别的分支/目录）→ 改指向我们的产物分支
        let (c2, r2) = git::gh_api(&url, "PUT", Some(&body))?;
        if (200..300).contains(&c2) {
            return Ok("已存在，已把来源改到产物分支".into());
        }
        return Err(format!(
            "Pages 已存在但改指向失败（HTTP {c2}）：{}\n请在仓库 Settings → Pages 里把来源改成 {branch} 分支。",
            brief(&r2)
        ));
    }
    if code == 403 || code == 404 {
        return Err(format!(
            "没有开启 Pages 的权限（HTTP {code}）：{}\n\
             免费账号的 Pages 只能用于 public 仓库；也可以到 \
             https://github.com/{owner}/{repo}/settings/pages 手动把来源设为 {branch} 分支。",
            brief(&resp)
        ));
    }
    Err(format!(
        "开启 Pages 失败（HTTP {code}）：{}\n\
         可到 https://github.com/{owner}/{repo}/settings/pages 手动开启（来源选 {branch} 分支）。",
        brief(&resp)
    ))
}

/* ============================== 入口 ============================== */

pub struct GhPublishOutcome {
    pub url: String,
    pub target: Target,
    /// Pages 开启结果的说明（失败时也放这里，不影响发布本身成功）
    pub pages_note: String,
}

/// 发布一次：定仓库 → 推分支 → （可选）开 Pages。
pub fn publish(
    dist: &Path,
    project_path: Option<&str>,
    project_name: &str,
    branch: &str,
    auto_pages: bool,
    prefix: &str,
    ident: (&str, &str),
) -> Result<GhPublishOutcome, String> {
    let proj = project_path.unwrap_or("");
    let name = if project_name.trim().is_empty() { "项目" } else { project_name };
    let target = resolve_target(proj, name, prefix)?;

    let staging = staging_dir();
    let push_result = push_branch(dist, &target.owner, &target.repo, branch, &staging, ident);
    // 暂存目录里含完整产物的另一份（多数是 APFS 克隆，不占额外空间），尽快清掉
    let _ = fs::remove_dir_all(&staging);
    push_result?;

    let site = pages_url(&target.owner, &target.repo);
    let pages_note = if auto_pages {
        match enable_pages(&target.owner, &target.repo, branch) {
            Ok(note) => note,
            Err(e) => e,   // 分支已经推上去了，Pages 的问题只作为提示，不算发布失败
        }
    } else {
        format!("已跳过自动开启，请到仓库 Settings → Pages 把来源设为 {branch} 分支")
    };

    Ok(GhPublishOutcome { url: site, target, pages_note })
}

/// 暂存目录：放 `~/.portbutler/tmp/ghpages-<时间戳>`。
///
/// 用固定父目录而不是系统临时目录，是为了万一清理失败时用户能找到并手动删除
/// （`/var/folders/...` 那串随机路径没人找得到）。
fn staging_dir() -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    crate::projects::data_dir()
        .join("tmp")
        .join(format!("ghpages-{ts}"))
}

/// 响应体截断。GitHub 的错误 JSON 往往很长，原样塞进 UI 会把其它信息挤没。
fn brief(s: &str) -> String {
    let t = s.trim();
    let cut: String = t.chars().take(300).collect();
    if t.chars().count() > 300 {
        format!("{cut}…")
    } else {
        cut
    }
}

/* ============================== 发布前预览 ============================== */

/// 发布弹窗用来显示「会发到哪儿」。
///
/// 刻意**不打网络** —— 打开弹窗不该等一次 API 往返。账号名取本地设置里
/// 记着的那个（绑定成功时写入），所以最多是「刚换账号还没绑定」时显示得旧一点，
/// 真发布那一刻仍以 API 返回的 login 为准。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GhPreview {
    /// `existing` = 复用项目已有仓库；`create` = 发布时新建
    pub source: String,
    pub owner: String,
    pub repo: String,
    /// 预测的站点地址；账号还没绑定时为空串
    pub url: String,
    pub branch: String,
    pub hint: String,
    /// 设置里当前选中的托管方式（弹窗用它做默认值）
    pub hosting: String,
    pub auto_pages: bool,
    /// 新建仓库时会用的完整仓库名（`<前缀><标识>`），供弹窗提前告知
    pub will_create: String,
}

#[tauri::command]
pub fn gh_publish_preview(project_path: String, project_name: String) -> GhPreview {
    let prefs = crate::git::hosting_prefs();
    let branch = prefs.branch.clone();

    if let Ok(remote) = git::origin_url(&project_path) {
        if let Some((owner, repo)) = parse_github_remote(&remote) {
            let url = pages_url(&owner, &repo);
            return GhPreview {
                source: "existing".into(),
                hint: format!("复用项目已有的 GitHub 仓库，产物推到 {branch} 分支（不动 main）"),
                owner,
                repo,
                url,
                branch,
                hosting: prefs.hosting,
                auto_pages: prefs.auto_pages,
                will_create: String::new(),
            };
        }
    }

    let owner = git::gh_account().unwrap_or_default();
    let slug = slugify(&project_name);
    let repo = if slug.is_empty() {
        format!("{}site", prefs.prefix)
    } else {
        format!("{}{}", prefs.prefix, slug)
    };
    let url = if owner.is_empty() {
        String::new()
    } else {
        pages_url(&owner, &repo)
    };
    let hint = if owner.is_empty() {
        "还没有绑定 GitHub 账号 —— 先到「项目仓库」页绑定，再回来发布".to_string()
    } else {
        format!(
            "项目没有 GitHub 远端：发布时会新建 public 仓库 {owner}/{repo}（免费账号的 Pages 只支持 public）"
        )
    };
    GhPreview {
        source: "create".into(),
        owner,
        repo: repo.clone(),
        url,
        branch,
        hint,
        hosting: prefs.hosting,
        auto_pages: prefs.auto_pages,
        will_create: repo,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_remote_covers_https_ssh_and_dotgit() {
        let want = ("hellojerry001".to_string(), "Port-build".to_string());
        for u in [
            "https://github.com/hellojerry001/Port-build.git",
            "https://github.com/hellojerry001/Port-build",
            "https://github.com/hellojerry001/Port-build/",
            "http://github.com/hellojerry001/Port-build.git",
            "git@github.com:hellojerry001/Port-build.git",
            "git@github.com:hellojerry001/Port-build",
            "ssh://git@github.com/hellojerry001/Port-build.git",
            "  https://github.com/hellojerry001/Port-build.git  ",
        ] {
            assert_eq!(parse_github_remote(u), Some(want.clone()), "解析失败：{u}");
        }
    }

    #[test]
    fn parse_remote_rejects_non_github_and_junk() {
        for u in [
            "",
            "https://gitlab.com/o/r.git",
            "https://gitee.com/o/r.git",
            "https://github.com/onlyowner",
            "git@github.com:",
            "not a url",
        ] {
            assert_eq!(parse_github_remote(u), None, "不该被当成 GitHub 远端：{u}");
        }
    }

    #[test]
    fn parse_remote_keeps_repo_case_but_strips_git_suffix() {
        // 仓库名大小写是保留的（GitHub 网址里就是这个大小写）
        assert_eq!(
            parse_github_remote("git@github.com:Me/MyRepo.git"),
            Some(("Me".into(), "MyRepo".into()))
        );
    }

    #[test]
    fn slugify_makes_valid_repo_names() {
        assert_eq!(slugify("客户管理后台"), ""); // 全中文 → 空，由调用方兜底
        assert_eq!(slugify("My App v2"), "my-app-v2");
        assert_eq!(slugify("  --Hello__World--  "), "hello-world");
        assert_eq!(slugify("a//b"), "a-b");
        assert_eq!(slugify(""), "");
        assert!(slugify(&"x".repeat(300)).chars().count() <= 80);
    }

    #[test]
    fn normalize_prefix_keeps_trailing_dash() {
        // 这是它与 slugify 的关键差别：不能把尾连字符裁掉，否则 pb-myapp 变成 pbmyapp
        assert_eq!(normalize_repo_prefix("pb-"), "pb-");
        assert_eq!(normalize_repo_prefix("pb"), "pb-");
        assert_eq!(normalize_repo_prefix("PB_"), "pb-");
        assert_eq!(normalize_repo_prefix("  web "), "web-");
        assert_eq!(normalize_repo_prefix("-"), "");
        assert_eq!(normalize_repo_prefix(""), "");
    }

    #[test]
    fn pages_url_handles_user_site_vs_project_site() {
        assert_eq!(
            pages_url("hellojerry001", "Port-build"),
            "https://hellojerry001.github.io/Port-build/"
        );
        // 用户主页仓库：地址里不能再带仓库段
        assert_eq!(
            pages_url("hellojerry001", "hellojerry001.github.io"),
            "https://hellojerry001.github.io/"
        );
        assert_eq!(
            pages_url("helloJerry001", "HelloJerry001.github.io"),
            "https://helloJerry001.github.io/"
        );
    }

    #[test]
    fn commit_message_carries_timestamp() {
        assert_eq!(commit_message(1_700_000_000_000), "publish: VibeButler 1700000000");
    }

    /// 只读真机探测：验证「从 git credential 取 token → 经 stdin 交给 curl →
    /// 打 api.github.com」这条链在本机真能走通（对应 current_login / enable_pages）。
    /// **只做 GET，不改任何远端状态。** `cargo test --lib ghpages -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn token_and_api_roundtrip_is_read_only() {
        let token = git::gh_token();
        println!("git credential 里是否有 token：{}", token.is_some());
        assert!(token.is_some(), "本机没有可用的 GitHub 凭证，发布链路无法工作");

        let (code, body) = git::gh_api("https://api.github.com/user", "GET", None)
            .expect("调用 api.github.com 失败（网络？）");
        println!("GET /user → HTTP {code}");
        assert_eq!(code, 200, "响应：{body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let login = v.get("login").and_then(|l| l.as_str()).unwrap_or("");
        println!("登录名：{login}");
        assert!(!login.is_empty(), "响应里没有 login");

        // 顺带看看账号套餐：免费账号的 Pages 只支持 public 仓库，
        // 这条信息在给用户解释「为什么只能建 public」时有用
        println!(
            "账号套餐：{}",
            v.get("plan")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("-")
        );
    }

    /// 真机干跑：拿**本地裸仓库**当远端，把「克隆产物 → 补 .nojekyll → 提交 → 推送」
    /// 整条链跑通。不碰 GitHub、不碰用户的产物目录，但走的是真实 git 命令。
    /// `cargo test --lib ghpages -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn push_to_local_bare_repo_end_to_end() {
        let root = std::env::temp_dir().join(format!("pb-ghpages-dry-{}", publishes::now_ms()));
        let dist = root.join("dist");
        let staging = root.join("staging");
        let remote = root.join("remote.git");

        fs::create_dir_all(dist.join("_next")).unwrap();
        fs::write(dist.join("index.html"), "<h1>hi</h1>").unwrap();
        // 下划线开头的目录：没有 .nojekyll 时 Pages 会整目录忽略掉
        fs::write(dist.join("_next/app.js"), "console.log(1)").unwrap();
        // 故意放一份 .gitignore：`git add -A -f` 应该连它忽略的文件一起提交
        fs::write(dist.join(".gitignore"), "*.log\n").unwrap();
        fs::write(dist.join("debug.log"), "should still be published").unwrap();

        let ok = Command::new(git::git_bin())
            .args(["init", "--bare", "-q"])
            .arg(&remote)
            .status()
            .unwrap()
            .success();
        assert!(ok, "建裸仓库失败");

        push_to(&dist, remote.to_str().unwrap(), "gh-pages", &staging, ("干跑", "dry@localhost"))
            .expect("推送应该成功");

        let out = Command::new(git::git_bin())
            .arg(format!("--git-dir={}", remote.display()))
            .args(["ls-tree", "-r", "--name-only", "gh-pages"])
            .output()
            .unwrap();
        let files = String::from_utf8_lossy(&out.stdout);
        let list: Vec<&str> = files.lines().collect();
        println!("远端 gh-pages 分支内容：{list:?}");
        for want in ["index.html", "_next/app.js", ".nojekyll", "debug.log"] {
            assert!(list.contains(&want), "产物里缺少 {want}：{list:?}");
        }

        // 必须只动暂存目录：用户的产物目录不能被写入 .git / .nojekyll
        assert!(!dist.join(".git").exists(), "污染了产物目录：多了 .git");
        assert!(!dist.join(".nojekyll").exists(), "污染了产物目录：多了 .nojekyll");

        let _ = fs::remove_dir_all(&root);
    }
}

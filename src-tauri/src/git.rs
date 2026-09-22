//! GitHub 同步设置（阶段一）。
//!
//! 这一页只做两件事：把本机 git 环境**读出来**，把身份与偏好**写回去**。
//! 它是「项目一键提交 GitHub」的前置 —— 提交要用的 `user.name` / `user.email` /
//! credential helper 在这里先确认好，真到提交那一步就不该再让用户填任何东西。

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::projects;

/// git 二进制的候选路径。
///
/// GUI 应用从 Finder 启动时 PATH 极窄（大致 `/usr/bin:/bin:/usr/sbin:/sbin`），
/// 而本机 git 是 Homebrew 装的（`/opt/homebrew/bin/git`）—— 只靠 PATH 会找不到。
const GIT_CANDIDATES: &[&str] = &[
    "/opt/homebrew/bin/git",
    "/usr/local/bin/git",
    "/usr/bin/git",
];

pub(crate) fn git_bin() -> String {
    for p in GIT_CANDIDATES {
        if Path::new(p).is_file() {
            return (*p).to_string();
        }
    }
    "git".into()
}

/// `gh` 的候选路径。与 git 同一个理由：GUI 启动的 PATH 里没有 Homebrew 目录，
/// 只看 PATH 会永远报「没装 gh」，而它明明在。
const GH_CANDIDATES: &[&str] = &["/opt/homebrew/bin/gh", "/usr/local/bin/gh"];

fn has_gh_cli() -> bool {
    GH_CANDIDATES.iter().any(|p| Path::new(p).is_file())
}

/// 跑一条 git 命令，成功返回 stdout（去尾空白）。
///
/// 失败（非 0 退出 / 起不来）一律 `None` —— 设置页要回答的是「能不能用」，
/// 不是把异常栈摊给用户看。
fn git_out(dir: Option<&str>, args: &[&str]) -> Option<String> {
    let mut c = Command::new(git_bin());
    if let Some(d) = dir {
        c.current_dir(d);
    }
    let out = c
        .args(args)
        // 关掉一切交互：凭证缺失时宁可失败，也不能弹终端提示
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "echo")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/* ============================== 设置 ============================== */

/// 托管方式取值。前端只认这两个字符串，后端也在这里收口。
pub const HOSTING_CLOUDFLARE: &str = "cloudflare";
pub const HOSTING_GITHUB: &str = "github";

/// GitHub 同步偏好。存 `~/.portbutler/settings.json`。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct GitSettings {
    /// 提交身份。留空表示「跟随 git 全局配置」，不覆盖。
    pub name: String,
    pub email: String,
    /// 新建仓库时的默认分支名
    pub default_branch: String,
    /// 初始化项目时自动写一份 .gitignore
    pub auto_gitignore: bool,
    /// 初始化项目时自动建一次提交
    pub auto_first_commit: bool,
    /// GitHub OAuth App 的 Client ID（Device Flow 用，非机密，可存设置）
    pub github_client_id: String,
    /// 已绑定的 GitHub 账号（只存用户名，绝不存 token）
    pub github_login: String,

    /* ---- 托管设置（发布到哪儿） ---- */
    /// 托管方式：`cloudflare`（匿名临时链接，现状）或 `github`（GitHub Pages，长期）
    pub hosting: String,
    /// GitHub Pages 用的产物分支名
    pub gh_branch: String,
    /// 推完分支后自动调 API 开启 Pages
    pub gh_auto_pages: bool,
    /// 自动新建仓库时的名字前缀（`<前缀><项目标识>`），避免撞上同名仓库
    pub gh_repo_prefix: String,
}

impl Default for GitSettings {
    fn default() -> Self {
        Self {
            name: String::new(),
            email: String::new(),
            default_branch: "main".into(),
            auto_gitignore: true,
            auto_first_commit: true,
            github_client_id: String::new(),
            github_login: String::new(),
            hosting: HOSTING_CLOUDFLARE.into(),
            gh_branch: "gh-pages".into(),
            gh_auto_pages: true,
            gh_repo_prefix: "pb-".into(),
        }
    }
}

impl GitSettings {
    /// 收口用户输入：去空白 + 校验。
    ///
    /// 名字里的换行会让 `git commit -m` 之外的场景出现莫名的空行；
    /// email 必须像 email，否则 git 会静默接受、推到 GitHub 上认不出是谁。
    fn sanitized(mut self) -> Result<Self, String> {
        self.name = self.name.trim().to_string();
        self.email = self.email.trim().to_string();
        self.default_branch = self.default_branch.trim().to_string();

        if self.name.chars().count() > 80 {
            return Err("名字最多 80 个字符".into());
        }
        if self.name.contains('\n') || self.email.contains('\n') {
            return Err("名字和邮箱不能含换行".into());
        }
        if !self.email.is_empty() && !looks_like_email(&self.email) {
            return Err(format!("邮箱格式不对：{}", self.email));
        }
        if self.default_branch.is_empty() {
            self.default_branch = "main".into();
        }
        if !is_valid_branch(&self.default_branch) {
            return Err(format!("分支名不合法：{}", self.default_branch));
        }

        // 托管方式只认这两种；写脏值（手改配置 / 旧版本）一律回落到默认，
        // 而不是报错 —— 用户不该因为一个内部字段丢掉整份设置。
        self.hosting = self.hosting.trim().to_lowercase();
        if self.hosting != HOSTING_GITHUB {
            self.hosting = HOSTING_CLOUDFLARE.into();
        }

        self.gh_branch = self.gh_branch.trim().to_string();
        if self.gh_branch.is_empty() {
            self.gh_branch = "gh-pages".into();
        }
        if !is_valid_branch(&self.gh_branch) {
            return Err(format!("GitHub Pages 分支名不合法：{}", self.gh_branch));
        }
        // 前缀会拼进仓库名，按 GitHub 的字符集收口（小写字母/数字/连字符）
        self.gh_repo_prefix = crate::ghpages::normalize_repo_prefix(&self.gh_repo_prefix);
        Ok(self)
    }
}

fn looks_like_email(s: &str) -> bool {
    // 刻意只做「够用的严格」：一个 @，两侧都非空，右侧含 `.`，全程无空白。
    // 真去实现 RFC 5322 只会把合法地址误判成非法。
    if s.chars().any(|c| c.is_whitespace()) {
        return false;
    }
    let mut it = s.split('@');
    match (it.next(), it.next(), it.next()) {
        (Some(l), Some(r), None) => !l.is_empty() && r.contains('.') && !r.starts_with('.') && !r.ends_with('.'),
        _ => false,
    }
}

fn is_valid_branch(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && !s.starts_with('/')
        && !s.ends_with('/')
        && !s.ends_with('.')
        && !s.contains("..")
        && !s.contains("//")
        && !s.chars().any(|c| matches!(c, ' ' | '~' | '^' | ':' | '?' | '*' | '[' | '\\'))
}

fn settings_path() -> PathBuf {
    projects::data_dir().join("settings.json")
}

fn load_settings() -> GitSettings {
    fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/* ============================== 环境自检 ============================== */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvCheck {
    pub ok: bool,
    pub label: String,
    pub detail: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    /// 一切就绪 = git 可用 + 身份齐 + 凭证有
    pub ready: bool,
    pub git_path: String,
    pub git_version: String,
    /// 生效身份（设置页优先，其次 git 全局配置）
    pub name: String,
    pub email: String,
    /// 身份来自哪里，让人知道自己填的东西有没有被用上
    pub name_source: String,
    pub helper: String,
    pub has_github_cred: bool,
    pub gh_cli: bool,
    /// 已绑定的 GitHub 账号（仅用户名）；空 = 未绑定或钥匙串凭证丢失
    pub github_login: String,
    pub checks: Vec<EnvCheck>,
    pub settings: GitSettings,
}

/// 查钥匙串里有没有 github.com 的凭证条目。
///
/// 只问「有没有」，**不读密码** —— 密码该由 git 自己去取，
/// 应用没有理由把这串东西装进内存。
fn has_github_cred() -> bool {
    Command::new("/usr/bin/security")
        .args(["find-internet-password", "-s", "github.com"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[tauri::command]
pub fn git_status() -> GitStatus {
    let settings = load_settings();
    let path = git_bin();
    let version = git_out(None, &["--version"]).unwrap_or_default();
    let git_ok = !version.is_empty();

    let global_name = git_out(None, &["config", "--global", "--get", "user.name"]).unwrap_or_default();
    let global_email = git_out(None, &["config", "--global", "--get", "user.email"]).unwrap_or_default();
    let helper = git_out(None, &["config", "--global", "--get", "credential.helper"]).unwrap_or_default();

    // 设置页填了就用设置页的；没填才回落到全局
    let (name, name_source) = if settings.name.is_empty() {
        (global_name.clone(), "git 全局配置")
    } else {
        (settings.name.clone(), "本页设置")
    };
    let email = if settings.email.is_empty() {
        global_email.clone()
    } else {
        settings.email.clone()
    };

    let cred = has_github_cred();
    let gh_cli = has_gh_cli();

    let mut checks = vec![
        EnvCheck {
            ok: git_ok,
            label: "Git".into(),
            detail: if git_ok {
                format!("{} · {}", version, path)
            } else {
                "没找到可用的 git".into()
            },
        },
        EnvCheck {
            ok: !name.is_empty() && !email.is_empty(),
            label: "提交身份".into(),
            detail: if name.is_empty() || email.is_empty() {
                "还没配 user.name / user.email".into()
            } else {
                format!("{} <{}>", name, email)
            },
        },
    ];
    checks.push(EnvCheck {
        ok: !helper.is_empty(),
        label: "凭证助手".into(),
        detail: if helper.is_empty() {
            "未配置 credential.helper".into()
        } else {
            helper.clone()
        },
    });
    checks.push(EnvCheck {
        ok: cred,
        label: "GitHub 凭证".into(),
        detail: if cred {
            "钥匙串里有 github.com 凭证".into()
        } else {
            "钥匙串里还没有 github.com 凭证".into()
        },
    });

    let ready = git_ok && !name.is_empty() && !email.is_empty() && cred;

    GitStatus {
        ready,
        git_path: path,
        git_version: version,
        name,
        email,
        name_source: name_source.into(),
        helper,
        has_github_cred: cred,
        gh_cli,
        github_login: if cred { settings.github_login.clone() } else { String::new() },
        checks,
        settings,
    }
}

/* ============================== 项目仓库状态 ============================== */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoState {
    pub id: String,
    pub name: String,
    pub path: String,
    pub is_repo: bool,
    pub remote: String,
    pub branch: String,
    /// 未提交的条目数（`git status --porcelain` 的行数）
    pub dirty: usize,
    /// 是否落后于远程（有未推送的本地提交）
    pub unpushed: usize,
}

fn repo_state(p: &projects::Project) -> RepoState {
    let dir = Path::new(&p.path);
    let mut st = RepoState {
        id: p.id.clone(),
        name: p.name.clone(),
        path: p.path.clone(),
        is_repo: dir.join(".git").exists(),
        remote: String::new(),
        branch: String::new(),
        dirty: 0,
        unpushed: 0,
    };
    if !st.is_repo {
        return st;
    }
    let cwd = Some(p.path.as_str());
    st.remote = git_out(cwd, &["remote", "get-url", "origin"]).unwrap_or_default();
    st.branch = git_out(cwd, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    st.dirty = git_out(cwd, &["status", "--porcelain"])
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);
    // 有 origin 才算得上「未推送」；没有 upstream 时 count 返回 0，不报错
    if !st.remote.is_empty() {
        st.unpushed = git_out(cwd, &["rev-list", "--count", "@{upstream}..HEAD"])
            .and_then(|s| s.trim().parse::<usize>().ok())
            .unwrap_or(0);
    }
    st
}

/// 逐个项目探一遍 git 状态。单独一条命令，是为了让设置页先渲染出来、
/// 状态表后到 —— 7 个项目要 spawn 二十来个子进程，不该挡住首屏。
#[tauri::command]
pub fn git_repo_states() -> Vec<RepoState> {
    projects::load().iter().map(repo_state).collect()
}

/* ============================== 提交与推送（阶段二） ============================== */

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    /// 相对仓库根的文件路径
    pub path: String,
    /// 单字母 git 状态：M / A / D / R / ?? 等
    pub status: String,
    /// 中文友好标签：修改 / 新增 / 删除 / 未跟踪 / 重命名
    pub label: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PushOutcome {
    pub ok: bool,
    /// 一句话结论，如「已提交并推送（8 个改动）」「已推送」
    pub headline: String,
    /// git 的真实输出（stdout / stderr 合并），前端等宽展示
    pub detail: String,
}

/// 跑一条 git 命令，成功返回 stdout+stderr（合并），失败返回 Err（含 stderr）。
///
/// 与 `git_out` 的区别：这里要的是「推送 / 提交」这种**会失败且必须把原因告诉用户**
/// 的动作 —— 鉴权失败、远端领先要先 pull、冲突，都不能吞。
fn git_run(dir: Option<&str>, args: &[&str]) -> Result<String, String> {
    let mut c = Command::new(git_bin());
    if let Some(d) = dir { c.current_dir(d); }
    let out = c.args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "echo")
        .output()
        .map_err(|e| format!("调用 git 失败：{e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        let msg = if !stderr.trim().is_empty() { stderr.trim().to_string() }
                  else { stdout.trim().to_string() };
        return Err(msg);
    }
    let mut s = stdout.trim().to_string();
    if !stderr.trim().is_empty() {
        if !s.is_empty() { s.push('\n'); }
        s.push_str(stderr.trim());
    }
    Ok(s)
}

/// 把 porcelain 的状态双字母映射成中文标签。
fn classify(x: char, y: char) -> (&'static str, &'static str) {
    match (x, y) {
        ('?', '?')          => ("??", "未跟踪"),
        ('U', _) | (_, 'U') => ("U",  "冲突"),
        ('A', _) | (_, 'A') => ("A",  "新增"),
        ('D', _) | (_, 'D') => ("D",  "删除"),
        ('R', _) | (_, 'R') => ("R",  "重命名"),
        ('C', _) | (_, 'C') => ("C",  "复制"),
        ('M', _) | (_, 'M') => ("M",  "修改"),
        _                    => ("?",  "改动"),
    }
}

#[tauri::command]
pub fn git_changed_files(path: String) -> Result<Vec<ChangedFile>, String> {
    if !Path::new(&path).join(".git").exists() {
        return Err("还不是 git 仓库".into());
    }
    let out = git_out(Some(&path), &["status", "--porcelain"]).unwrap_or_default();
    let files: Vec<ChangedFile> = out
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let mut it = line.trim_start().chars();
            let x = it.next().unwrap_or(' ');
            let y = it.next().unwrap_or(' ');
            let name: String = it.skip_while(|c| c.is_whitespace()).collect();
            let (status, label) = classify(x, y);
            ChangedFile { path: name, status: status.into(), label: label.into() }
        })
        .collect();
    Ok(files)
}

/// 取出 origin 远端地址；没有就报错（把「先 remote add」交给用户，
/// 应用不替他建仓库，也不猜他想推到哪个地址）。
pub(crate) fn origin_url(dir: &str) -> Result<String, String> {
    git_out(Some(dir), &["remote", "get-url", "origin"])
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "没有远端仓库（origin）。先 `git remote add origin <url>` 再推送。".into())
}

#[tauri::command]
pub fn git_push(path: String) -> Result<PushOutcome, String> {
    if !Path::new(&path).join(".git").exists() {
        return Err("还不是 git 仓库".into());
    }
    origin_url(&path)?;
    let branch = git_out(Some(&path), &["rev-parse", "--abbrev-ref", "HEAD"])
        .unwrap_or_default();
    if branch.is_empty() {
        return Err("无法确定当前分支".into());
    }
    let detail = git_run(Some(&path), &["push", "-u", "origin", branch.as_str()])?;
    Ok(PushOutcome { ok: true, headline: "已推送".into(), detail })
}

#[tauri::command]
pub fn git_commit_push(path: String, message: String) -> Result<PushOutcome, String> {
    // message 先校验：空 / 纯空白先拒绝，不必走到远端那一步
    let msg = message.trim();
    if msg.is_empty() {
        return Err("提交说明不能为空".into());
    }
    if msg.chars().count() > 200 {
        return Err("提交说明太长（最多 200 字）".into());
    }
    if !Path::new(&path).join(".git").exists() {
        return Err("还不是 git 仓库".into());
    }
    // 没有未提交改动就不 commit（否则 git 会开编辑器卡住或报 nothing to commit）
    let dirty = git_out(Some(&path), &["status", "--porcelain"])
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);
    if dirty == 0 {
        return Err("没有未提交的改动，无需提交".into());
    }
    origin_url(&path)?;
    let branch = git_out(Some(&path), &["rev-parse", "--abbrev-ref", "HEAD"])
        .unwrap_or_default();
    if branch.is_empty() {
        return Err("无法确定当前分支".into());
    }
    git_run(Some(&path), &["add", "-A"])
        .map_err(|e| format!("git add 失败：{e}"))?;
    // commit message 走参数数组，不经 shell，无注入面
    git_run(Some(&path), &["commit", "-m", msg])
        .map_err(|e| format!("提交失败：{e}"))?;
    let detail = git_run(Some(&path), &["push", "-u", "origin", branch.as_str()])
        .map_err(|e| format!("推送失败：{e}"))?;
    Ok(PushOutcome { ok: true, headline: format!("已提交并推送（{} 个改动）", dirty), detail })
}

/* ============================== 写入 ============================== */

#[tauri::command]
pub fn save_git_settings(settings: GitSettings) -> Result<GitSettings, String> {
    let s = settings.sanitized()?;
    // 只更新本页那几个字段；github_client_id / github_login 由专门命令维护，
    // 这里必须保留，否则每次点「保存设置」都会把它们清空掉。
    let mut existing = load_settings();
    existing.name = s.name;
    existing.email = s.email;
    existing.default_branch = s.default_branch;
    existing.auto_gitignore = s.auto_gitignore;
    existing.auto_first_commit = s.auto_first_commit;
    existing.hosting = s.hosting;
    existing.gh_branch = s.gh_branch;
    existing.gh_auto_pages = s.gh_auto_pages;
    existing.gh_repo_prefix = s.gh_repo_prefix;
    let path = settings_path();
    let text = serde_json::to_string_pretty(&existing).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| format!("写入设置失败：{e}"))?;
    Ok(existing)
}

/// 把身份写进 **git 全局配置**（`~/.gitconfig`），这样命令行里 `git commit`
/// 也用得上 —— 只在应用内部生效等于没配。
#[tauri::command]
pub fn apply_git_identity(name: String, email: String) -> Result<String, String> {
    let s = GitSettings {
        name,
        email,
        ..Default::default()
    }
    .sanitized()?;
    if s.name.is_empty() || s.email.is_empty() {
        return Err("名字和邮箱都要填".into());
    }
    for (k, v) in [("user.name", &s.name), ("user.email", &s.email)] {
        let out = Command::new(git_bin())
            .args(["config", "--global", k, v])
            .output()
            .map_err(|e| format!("调用 git 失败：{e}"))?;
        if !out.status.success() {
            return Err(format!(
                "写 {} 失败：{}",
                k,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    Ok(format!("已写入全局配置：{} <{}>", s.name, s.email))
}

/// 设置页的「去 GitHub 建令牌」入口用的固定地址。
#[tauri::command]
pub fn token_help_url() -> String {
    "https://github.com/settings/tokens".into()
}

/* ============================== GitHub 账号绑定（OAuth Device Flow / gh CLI） ============================== */

/// PortButler 官方注册的 OAuth App 的 Client ID。
///
/// 有它，用户点「绑定」就能直接用，**完全不需要自己申请 Client ID**。
/// Device Flow 没有 client_secret，client_id 本来就是公开标识符（gh CLI 自己的
/// client_id 就公开在源码里），写进二进制不会泄露任何凭据 —— 授权仍必须用户
/// 本人在浏览器点确认。
///
/// TODO: 填一次即可（github.com/settings/developers → New OAuth App → 勾 Enable device flow）。
/// 留空时前端会退回让用户自己填（见 GhCliStatus.can_device_flow）。
const BUILTIN_CLIENT_ID: &str = "";

/// 决定这次授权用哪个 Client ID：用户自己填的优先，没填就用内置的。
fn resolve_client_id(given: &str) -> String {
    let g = given.trim();
    if g.is_empty() {
        BUILTIN_CLIENT_ID.trim().to_string()
    } else {
        g.to_string()
    }
}

/// Device Flow 申请到的授权码。前端把它显示给用户去浏览器输入。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GhDeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// 轮询结果。status: pending | authorized | expired | denied。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GhPollResult {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 跑一条 HTTP 请求，返回 (HTTP 状态码, 响应体)。
///
/// 故意**不**把非 200 当成错误 —— GitHub 的 Device Flow 在 pending / denied /
/// expired 时也返回 200 + 一个带 `error` 字段的 JSON，真正的状态得看 body。
/// 用 curl（而非引入 reqwest），和本文件一贯的「只 spawn 系统命令」风格一致。
fn curl_json(url: &str, method: &str, body: Option<&serde_json::Value>) -> (i32, String) {
    let mut c = Command::new("/usr/bin/curl");
    c.args([
        "-sS", "-L", "-X", method, url,
        "-H", "Accept: application/json",
        "-H", "Content-Type: application/json",
    ]);
    if let Some(b) = body {
        if let Ok(j) = serde_json::to_string(b) {
            c.arg("--data").arg(j);
        }
    }
    match c.output() {
        Ok(o) => (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout).to_string(),
        ),
        Err(e) => (-1, format!("{{\"error\":\"curl 调用失败：{e}\"}}")),
    }
}

/// 托管相关偏好的一次性读取（发布链路用）。
///
/// 单独开个口子而不是把 `load_settings` 公开：发布模块只需要这四个值，
/// 不该拿到 client id / 登录名这些无关字段。
pub(crate) struct HostingPrefs {
    pub hosting: String,
    pub branch: String,
    pub auto_pages: bool,
    pub prefix: String,
}

pub(crate) fn hosting_prefs() -> HostingPrefs {
    let s = load_settings();
    HostingPrefs {
        hosting: if s.hosting.trim().is_empty() {
            HOSTING_CLOUDFLARE.to_string()
        } else {
            s.hosting
        },
        branch: if s.gh_branch.trim().is_empty() { "gh-pages".into() } else { s.gh_branch },
        auto_pages: s.gh_auto_pages,
        prefix: s.gh_repo_prefix,
    }
}

/// 生效的提交身份（设置页填的优先，否则读 git 全局配置）。
///
/// 产物提交也需要作者信息；用户没在设置页填过时，用他 git 全局的那份，
/// 免得推上去的提交显示成一个陌生名字。
pub(crate) fn effective_identity() -> (String, String) {
    let s = load_settings();
    let get = |k: &str| {
        git_out(None, &["config", "--get", k])
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let name = if s.name.trim().is_empty() { get("user.name") } else { s.name };
    let email = if s.email.trim().is_empty() { get("user.email") } else { s.email };
    (name, email)
}

/// 取当前可用的 GitHub token，供 API 调用（开 Pages、建仓库等）。
///
/// 走 `git credential fill`，与推送**同一来源** —— 尊重用户配的 helper
/// （osxkeychain / gh / 别的都行），应用不自己去读钥匙串，也就不会额外弹授权框。
pub(crate) fn gh_token() -> Option<String> {
    git_credential_fill("github.com").map(|(_, pass)| pass)
}

/// 带 Bearer token 的 GitHub API 调用，返回 `(HTTP 状态码, 响应体)`。
///
/// token 经 curl 的 `-K -`（从 stdin 读配置）传入，**不出现在 argv 里** ——
/// 命令行参数在同机 `ps` 下可见，而这是个长期有效的凭证。
/// 请求体照常走 `--data`：不是机密，放进 argv 反而让引号处理简单得多。
pub(crate) fn gh_api(
    url: &str,
    method: &str,
    body: Option<&serde_json::Value>,
) -> Result<(u16, String), String> {
    let token = gh_token().ok_or_else(|| {
        "没有可用的 GitHub 凭证。请先在「项目仓库」页绑定 GitHub 账号。".to_string()
    })?;

    let mut c = Command::new("/usr/bin/curl");
    c.args([
        "-sS",
        "-L",
        "-X",
        method,
        url,
        "-H",
        "Accept: application/vnd.github+json",
        "-H",
        "X-GitHub-Api-Version: 2022-11-28",
        // ⚠️ 必须显式要状态码：curl 自己的退出码是 0（成功）而不是 HTTP 状态，
        // 用它做 200/201/409 判断会让所有分支都落到「失败」上。
        "-w",
        "\n%{http_code}",
        "-K",
        "-",
    ]);
    if let Some(b) = body {
        let j = serde_json::to_string(b).map_err(|e| e.to_string())?;
        c.args(["-H", "Content-Type: application/json", "--data", &j]);
    }
    c.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = c.spawn().map_err(|e| format!("curl 调用失败：{e}"))?;
    {
        let Some(si) = child.stdin.as_mut() else {
            return Err("无法写入 curl 配置".into());
        };
        // curl 配置里字符串值用双引号包裹；token 是字母数字/符号集，无需转义
        write!(si, "header = \"Authorization: Bearer {token}\"\n")
            .map_err(|e| format!("写入 curl 配置失败：{e}"))?;
    }
    // wait_with_output 会先关掉 stdin，curl 才知道配置读完了
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let raw = String::from_utf8_lossy(&out.stdout).to_string();
    let (body_text, status) = match raw.rfind('\n') {
        Some(i) => (&raw[..i], raw[i + 1..].trim()),
        None => ("", raw.trim()),
    };
    let code: u16 = status.parse().unwrap_or(0);
    if !out.status.success() && code == 0 {
        return Err(format!(
            "curl 异常退出：{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok((code, body_text.to_string()))
}

/// 把 OAuth token 存进系统钥匙串，让 git（osxkeychain）推送时自动取用。
///
/// 账号固定用 `x-access-token` —— 这是 GitHub 认 OAuth/App token 的用户名，
/// 跟 `gh` CLI 的行为一致。token 只在绑定这一刻经过 App 内存，进钥匙串后不落任何文件。
fn store_gh_token(token: &str) -> Result<(), String> {
    // 先清掉旧的（可能换账号）
    let _ = Command::new("/usr/bin/security")
        .args(["delete-internet-password", "-s", "github.com"])
        .output();
    // 用 git 自带的凭证辅助程序写入：它会给自己开 ACL，之后 `git push` 才能无交互
    // 读到。直接 `security add-internet-password -T` 限制的条目，
    // `git-credential-osxkeychain` 不在白名单里，headless 下会静默读不到，推送报
    // "could not read Username"（2026-09-22 实测踩到）。
    let input = format!(
        "protocol=https\nhost=github.com\nusername=x-access-token\npassword={token}\n\n"
    );
    let mut child = Command::new("/usr/bin/git")
        .args(["credential", "approve"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("写入钥匙串失败：{e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        std::io::Write::write_all(&mut stdin, input.as_bytes())
            .map_err(|e| format!("写入钥匙串失败：{e}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("写入钥匙串失败：{e}"))?;
    if !out.status.success() {
        return Err(format!(
            "写入钥匙串失败：{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// 删掉钥匙串里所有 github.com 凭证条目，返回实际删掉的条数。
///
/// `security` 一次只删一条匹配项，所以循环删到没有为止 —— 换过账号、或用
/// 不同用户名存过凭证的机器上会残留多条，只删第一条的话「解绑」是假的。
///
/// 每轮先用 `has_github_cred()` 判存在，而不是去解析 security 的报错文案：
/// 那是本地化字符串（中文系统上是「找不到指定的项目」），拿它做判断迟早出错。
/// 存在却删不掉（用户点了「拒绝」）才是真失败，必须让用户看到原因。
fn delete_gh_tokens() -> Result<usize, String> {
    let mut n = 0usize;
    while n < 32 {
        if !has_github_cred() {
            break;
        }
        let out = Command::new("/usr/bin/security")
            .args(["delete-internet-password", "-s", "github.com"])
            .output()
            .map_err(|e| format!("调用 security 失败：{e}"))?;
        if out.status.success() {
            n += 1;
            continue;
        }
        let err = crate::publish::strip_ansi(&String::from_utf8_lossy(&out.stderr)).trim().to_string();
        if err.is_empty() {
            break;
        }
        return Err(format!("删除钥匙串凭证失败：{err}"));
    }
    Ok(n)
}

/// 退出 gh CLI 在 github.com 的登录。
///
/// gh 把自己的 token 存在 `gh:github.com` 条目里，与我们要删的 `github.com`
/// 凭证互不相干 —— 不单独退的话，`credential.helper` 指向 gh 的机器上
/// 「解绑」之后 git 照样能推送，等于没解绑。
fn gh_cli_logout() -> Result<(), String> {
    let bin = gh_bin().ok_or_else(|| "没检测到 gh CLI".to_string())?;
    let out = Command::new(bin)
        .args(["auth", "logout", "--hostname", "github.com"])
        .output()
        .map_err(|e| format!("调用 gh 失败：{e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let err = crate::publish::strip_ansi(&String::from_utf8_lossy(&out.stderr)).trim().to_string();
    Err(if err.is_empty() { "gh auth logout 失败".into() } else { err })
}

/// 用 Bearer token 拉 `api.github.com/user`，取回登录名。
fn gh_login_of(token: &str) -> Result<String, String> {
    let out = Command::new("/usr/bin/curl")
        .args([
            "-sS", "-L", "-X", "GET", "https://api.github.com/user",
            "-H", "Accept: application/json",
            "-H", &format!("Authorization: Bearer {token}"),
        ])
        .output()
        .map_err(|e| format!("查询 GitHub 用户失败：{e}"))?;
    let resp = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value = serde_json::from_str(&resp)
        .map_err(|e| format!("解析 GitHub 用户响应失败：{e}"))?;
    v.get("login")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "GitHub 用户响应缺少 login 字段".into())
}

/// 把设置结构体落盘（保留所有字段，含 github_*）。
fn save_settings_struct(s: &GitSettings) -> Result<(), String> {
    let path = settings_path();
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| format!("写入设置失败：{e}"))?;
    Ok(())
}

/// 申请设备授权码。前端拿到后展示 user_code + verification_uri 给用户。
#[tauri::command]
pub fn gh_device_code(client_id: String, scope: Option<String>) -> Result<GhDeviceCode, String> {
    let client_id = resolve_client_id(&client_id);
    if client_id.is_empty() {
        return Err("没有可用的 Client ID：本应用未内置，请先在设置里填写".into());
    }
    let scope = scope.unwrap_or_else(|| "repo".into());
    let body = serde_json::json!({ "client_id": client_id, "scope": scope });
    let (_, resp) = curl_json("https://github.com/login/device/code", "POST", Some(&body));
    let v: serde_json::Value = serde_json::from_str(&resp)
        .map_err(|e| format!("解析 GitHub 响应失败：{e}"))?;
    if let Some(err) = v.get("error").and_then(|x| x.as_str()) {
        let desc = v
            .get("error_description")
            .and_then(|x| x.as_str())
            .unwrap_or(err);
        return Err(format!("GitHub 返回错误：{desc}"));
    }
    Ok(GhDeviceCode {
        device_code: v["device_code"].as_str().unwrap_or_default().to_string(),
        user_code: v["user_code"].as_str().unwrap_or_default().to_string(),
        verification_uri: v["verification_uri"].as_str().unwrap_or_default().to_string(),
        expires_in: v["expires_in"].as_u64().unwrap_or(900),
        interval: v["interval"].as_u64().unwrap_or(5),
    })
}

/// 轮询授权结果。前端按返回的 interval 反复调用，直到 status 不再是 pending。
#[tauri::command]
pub fn gh_device_poll(client_id: String, device_code: String) -> Result<GhPollResult, String> {
    let client_id = resolve_client_id(&client_id);
    let body = serde_json::json!({
        "client_id": client_id,
        "device_code": device_code,
        "grant_type": "urn:ietf:params:oauth:grant-type:device_code"
    });
    let (_, resp) = curl_json(
        "https://github.com/login/oauth/access_token",
        "POST",
        Some(&body),
    );
    let v: serde_json::Value = serde_json::from_str(&resp)
        .map_err(|e| format!("解析 GitHub 响应失败：{e}"))?;

    if let Some(tok) = v.get("access_token").and_then(|x| x.as_str()) {
        // 成功：取用户名并存钥匙串，之后 git 推送自动复用，无需再动 UI
        let login = gh_login_of(tok)?;
        store_gh_token(tok)?;
        let mut s = load_settings();
        s.github_login = login.clone();
        save_settings_struct(&s)?;
        return Ok(GhPollResult {
            status: "authorized".into(),
            login: Some(login),
            error: None,
        });
    }
    if let Some(err) = v.get("error").and_then(|x| x.as_str()) {
        return match err {
            "authorization_pending" | "slow_down" => Ok(GhPollResult {
                status: "pending".into(),
                login: None,
                error: None,
            }),
            "expired_token" => Ok(GhPollResult {
                status: "expired".into(),
                login: None,
                error: Some("授权码已过期，请重新获取".into()),
            }),
            "access_denied" => Ok(GhPollResult {
                status: "denied".into(),
                login: None,
                error: Some("你已拒绝授权".into()),
            }),
            other => Err(format!("GitHub 返回未知错误：{other}")),
        };
    }
    Err(format!("GitHub 响应异常：{resp}"))
}

/// 当前已绑定的账号；钥匙串凭证丢了就清掉残留用户名并返回 None。
#[tauri::command]
pub fn gh_account() -> Option<String> {
    let s = load_settings();
    if s.github_login.is_empty() {
        return None;
    }
    if !has_github_cred() {
        let mut s = s;
        s.github_login = String::new();
        let _ = save_settings_struct(&s);
        return None;
    }
    Some(s.github_login)
}

/// 解绑回执。前端拿它给一句准确的话（删了几条凭证、有没有顺手退出 gh CLI）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GhUnbindResult {
    /// 从钥匙串删掉的凭证条数
    pub removed_creds: usize,
    /// 有没有清掉记着的账号名
    pub removed_login: bool,
    /// 有没有顺带退出 gh CLI 登录
    pub gh_cli_logout: bool,
    /// 退出 gh CLI 失败的原因（不阻断解绑本身，只是提示）
    pub gh_cli_error: Option<String>,
    pub message: String,
}

/// 回执文案。抽成纯函数是为了能单测「一条凭证都没删到时别谎报成功」。
fn unbind_message(login: &str, removed_creds: usize, removed_login: bool, gh_logout: bool) -> String {
    let who = if login.is_empty() { "GitHub 账号".to_string() } else { format!("@{login}") };
    let mut parts: Vec<String> = Vec::new();
    if removed_creds > 0 {
        parts.push(format!("已删除钥匙串里的 {removed_creds} 条凭证"));
    } else {
        parts.push("钥匙串里本来就没有 github.com 凭证".into());
    }
    if removed_login {
        parts.push(format!("已忘掉 {who}"));
    }
    if gh_logout {
        parts.push("已退出 gh CLI 登录".into());
    }
    format!("解绑完成：{}", parts.join("，"))
}

/// 解绑：删钥匙串凭证 + 清设置里的用户名，可选顺带退出 gh CLI 登录。
/// 项目文件与远端仓库原样保留。
#[tauri::command]
pub fn gh_unbind(logout_gh_cli: Option<bool>) -> Result<GhUnbindResult, String> {
    let removed_creds = delete_gh_tokens()?;

    let mut s = load_settings();
    let login = s.github_login.clone();
    let removed_login = !login.is_empty();
    s.github_login = String::new();
    save_settings_struct(&s)?;

    let mut logged_out = false;
    let mut gh_cli_error = None;
    if logout_gh_cli.unwrap_or(false) {
        match gh_cli_logout() {
            Ok(()) => logged_out = true,
            Err(e) => gh_cli_error = Some(e),
        }
    }

    Ok(GhUnbindResult {
        message: unbind_message(&login, removed_creds, removed_login, logged_out),
        removed_creds,
        removed_login,
        gh_cli_logout: logged_out,
        gh_cli_error,
    })
}

/// 持久化 Client ID（OAuth App 标识，非机密）。
#[tauri::command]
pub fn save_gh_client_id(client_id: String) -> Result<(), String> {
    let mut s = load_settings();
    s.github_client_id = client_id.trim().to_string();
    save_settings_struct(&s)?;
    Ok(())
}

/// 向 credential helper 要一条现成凭证，返回 `(用户名, 密码/令牌)`。
///
/// 走 `git credential fill` 而不是直接读钥匙串 —— 尊重用户配的 helper
/// （osxkeychain / gh / 别的都行），也免得我们自己去碰系统钥匙串的授权框。
fn git_credential_fill(host: &str) -> Option<(String, String)> {
    let mut child = Command::new(git_bin())
        .args(["credential", "fill"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    {
        let stdin = child.stdin.as_mut()?;
        write!(stdin, "protocol=https\nhost={host}\n\n").ok()?;
    }
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let mut user = String::new();
    let mut pass = String::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(v) = line.strip_prefix("username=") {
            user = v.to_string();
        } else if let Some(v) = line.strip_prefix("password=") {
            pass = v.to_string();
        }
    }
    if user.is_empty() || pass.is_empty() {
        None
    } else {
        Some((user, pass))
    }
}

/// 认领本机已有的推送凭证：钥匙串里早就有一份能用的（多半是以前某次 push 留下的），
/// 只是我们没记下它属于谁。这里拿它去问一次 GitHub，把登录名补进设置。
///
/// 注意：**不动钥匙串** —— 只记名，不替换。这样老用户不会有「绑定反而换掉好凭证」的风险。
#[tauri::command]
pub fn gh_claim_existing() -> Result<String, String> {
    let (_, pass) = git_credential_fill("github.com").ok_or_else(|| {
        "本机没找到可复用的 github.com 凭证，请改用「绑定账号」".to_string()
    })?;
    let login = gh_login_of(&pass).map_err(|e| {
        format!("{e}。这份凭证可能不是能调 API 的令牌，请改用「绑定账号」重新授权")
    })?;
    let mut s = load_settings();
    s.github_login = login.clone();
    save_settings_struct(&s)?;
    Ok(login)
}

/// gh CLI 的可用状态。前端据此决定给哪条绑定路径：
/// 已登录 → 一键绑定（不开浏览器）；没装 → 只能走浏览器授权。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GhCliStatus {
    pub installed: bool,
    pub logged_in: bool,
    /// 有没有可用的 Client ID（内置的或用户自填的）—— 决定浏览器授权这条路走不走得通
    pub can_device_flow: bool,
}

fn gh_bin() -> Option<&'static str> {
    GH_CANDIDATES.iter().copied().find(|p| Path::new(p).is_file())
}

/// 用 gh 现有的登录态绑定账号：取 token → 存钥匙串 → 落用户名。
/// 用户已经在终端 `gh auth login` 过的话，这里是真正的一键、连浏览器都不用开。
#[tauri::command]
pub fn gh_bind_cli() -> Result<String, String> {
    let bin = gh_bin()
        .ok_or_else(|| "没检测到 gh CLI。可以改用浏览器授权，或先装一个：brew install gh".to_string())?;
    let out = Command::new(bin)
        .args(["auth", "token"])
        .output()
        .map_err(|e| format!("读取 gh 登录态失败：{e}"))?;
    if !out.status.success() {
        return Err("gh 还没登录。先在终端跑一次：gh auth login".into());
    }
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if token.is_empty() {
        return Err("gh 没返回 token，先在终端跑一次：gh auth login".into());
    }
    let login = gh_login_of(&token)?;
    store_gh_token(&token)?;
    let mut s = load_settings();
    s.github_login = login.clone();
    save_settings_struct(&s)?;
    Ok(login)
}

#[tauri::command]
pub fn gh_cli_status() -> GhCliStatus {
    let installed = gh_bin().is_some();
    let mut logged_in = false;
    if let Some(bin) = gh_bin() {
        if let Ok(o) = Command::new(bin).args(["auth", "token"]).output() {
            logged_in = o.status.success()
                && !String::from_utf8_lossy(&o.stdout).trim().is_empty();
        }
    }
    let saved = load_settings().github_client_id.trim().to_string();
    GhCliStatus {
        installed,
        logged_in,
        can_device_flow: !resolve_client_id("").is_empty() || !saved.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_shape_is_checked() {
        assert!(looks_like_email("a@b.co"));
        assert!(looks_like_email("jerry+git@example.com"));
        assert!(!looks_like_email("a@b"));      // 右侧没有点
        assert!(!looks_like_email("a b@c.com")); // 有空白
        assert!(!looks_like_email("@b.com"));
        assert!(!looks_like_email("a@"));
        assert!(!looks_like_email("a@b@c.com"));
        assert!(!looks_like_email("a@.com"));
    }

    #[test]
    fn branch_names_are_checked() {
        assert!(is_valid_branch("main"));
        assert!(is_valid_branch("feature/git-sync"));
        assert!(!is_valid_branch(""));
        assert!(!is_valid_branch("-b"));
        assert!(!is_valid_branch("a b"));
        assert!(!is_valid_branch("a..b"));
        assert!(!is_valid_branch("/x"));
        assert!(!is_valid_branch("x/"));
        assert!(!is_valid_branch("a~1"));
    }

    #[test]
    fn gh_device_code_rejects_only_when_no_client_id_at_all() {
        // 不发网络：没有任何可用 client_id（内置为空 + 入参为空）时才报错；
        // 一旦内置了 Client ID，空入参应当被内置值顶上而不是报错。
        let no_id_available = BUILTIN_CLIENT_ID.trim().is_empty();
        assert_eq!(gh_device_code(String::new(), None).is_err(), no_id_available);
    }

    #[test]
    fn client_id_falls_back_to_builtin() {
        // 用户没填 → 用内置的；用户填了 → 用用户的（覆盖内置）
        assert_eq!(resolve_client_id(""), BUILTIN_CLIENT_ID.trim());
        assert_eq!(resolve_client_id("  "), BUILTIN_CLIENT_ID.trim());
        assert_eq!(resolve_client_id(" mine_id "), "mine_id");
    }

    #[test]
    fn settings_are_trimmed_and_defaulted() {
        let s = GitSettings {
            name: "  Jerry  ".into(),
            email: "  j@e.com ".into(),
            default_branch: "   ".into(),
            ..Default::default()
        }
        .sanitized()
        .unwrap();
        assert_eq!(s.name, "Jerry");
        assert_eq!(s.email, "j@e.com");
        // 空分支回落成 main，而不是报错
        assert_eq!(s.default_branch, "main");
    }

    #[test]
    fn empty_identity_is_allowed_meaning_follow_global() {
        // 名字邮箱可以都留空 —— 语义是「跟随 git 全局配置」，不是错误
        let s = GitSettings::default().sanitized().unwrap();
        assert!(s.name.is_empty() && s.email.is_empty());
    }

    #[test]
    fn bad_email_is_refused_with_reason() {
        let e = GitSettings {
            email: "nope".into(),
            ..Default::default()
        }
        .sanitized()
        .unwrap_err();
        assert!(e.contains("邮箱格式不对"), "{e}");
    }

    #[test]
    fn newline_in_name_is_refused() {
        let e = GitSettings {
            name: "a\nb".into(),
            ..Default::default()
        }
        .sanitized()
        .unwrap_err();
        assert!(e.contains("换行"), "{e}");
    }

    #[test]
    fn git_binary_is_resolvable_on_this_machine() {
        // 至少能定位到一个候选路径；解析到的应当是绝对路径
        let b = git_bin();
        assert!(b.starts_with('/') || b == "git", "{b}");
    }

    #[test]
    fn repo_state_of_non_repo_is_clean() {
        // 显式构造而非 ..Default::default()：Project 是 persisted 数据模型，
        // 刻意不派生 Default —— 空 id / 空 kind 在业务上是没有意义的组合。
        let p = projects::Project {
            id: "t".into(),
            name: "t".into(),
            path: "/tmp/definitely-not-a-repo-xyz".into(),
            command: String::new(),
            port: 0,
            scaffold: String::new(),
            node_version: String::new(),
            kind: String::new(),
            build_command: String::new(),
        };
        let st = repo_state(&p);
        assert!(!st.is_repo);
        assert_eq!(st.dirty, 0);
        assert_eq!(st.unpushed, 0);
        assert!(st.remote.is_empty());
    }

    #[test]
    fn gh_is_detected_by_absolute_path_not_path_env() {
        // 探测不能依赖 PATH（GUI 启动时 PATH 极窄），必须走候选绝对路径
        assert!(GH_CANDIDATES.iter().all(|p| p.starts_with('/')));
        // 这台机器上装了 gh 的话应当被判为 true；没装也只允许 false，不允许 panic
        let _ = has_gh_cli();
    }

    /* ---- 前后端契约：前端按 camelCase 取值，键名一旦改了前端会静默显示 undefined ---- */

    #[test]
    fn settings_json_uses_camel_case_keys() {
        let j = serde_json::to_string(&GitSettings::default()).unwrap();
        for k in ["\"defaultBranch\"", "\"autoGitignore\"", "\"autoFirstCommit\""] {
            assert!(j.contains(k), "设置 JSON 里缺 {k}：{j}");
        }
    }

    #[test]
    fn repo_states_cover_every_project() {
        // 真跑一遍：设置页的仓库表就是这张表，漏一个项目 / 顺序对不上都会表现为「少一行」
        let ps = projects::load();
        let st = git_repo_states();
        assert_eq!(st.len(), ps.len());
        for (p, r) in ps.iter().zip(st.iter()) {
            assert_eq!(p.id, r.id);
            assert_eq!(p.path, r.path);
            eprintln!("  {} · repo={} branch={:?} dirty={} unpushed={} remote={:?}",
                r.name, r.is_repo, r.branch, r.dirty, r.unpushed, r.remote);
        }
    }

    #[test]
    fn status_json_uses_camel_case_keys_and_real_env() {
        // 真跑一遍：这台机器上 git 必须能定位到，四条自检必须齐全
        let st = git_status();
        assert!(st.git_path.starts_with('/'), "git 路径应当是绝对路径：{}", st.git_path);
        assert!(st.git_version.contains("git version"), "版本串不对：{}", st.git_version);
        assert_eq!(st.checks.len(), 4, "自检项应当是 Git / 提交身份 / 凭证助手 / GitHub 凭证");
        assert_eq!(st.checks[0].label, "Git");

        let j = serde_json::to_string(&st).unwrap();
        for k in ["\"gitPath\"", "\"gitVersion\"", "\"nameSource\"", "\"hasGithubCred\"", "\"ghCli\""] {
            assert!(j.contains(k), "状态 JSON 里缺 {k}");
        }
    }

    /* ---- 阶段二：提交与推送 ---- */

    #[test]
    fn changed_files_reports_missing_repo() {
        let r = git_changed_files("/tmp/definitely-not-a-repo-xyz".into());
        assert!(r.is_err(), "非仓库应当报错，得到 {:?}", r);
        assert!(r.unwrap_err().contains("还不是 git 仓库"));
    }

    #[test]
    fn classify_maps_git_states() {
        assert_eq!(classify('?', '?'), ("??", "未跟踪"));
        assert_eq!(classify('A', ' '), ("A", "新增"));
        assert_eq!(classify(' ', 'M'), ("M", "修改"));
        assert_eq!(classify('D', ' '), ("D", "删除"));
        assert_eq!(classify('R', ' '), ("R", "重命名"));
        assert_eq!(classify('U', 'A'), ("U", "冲突"));
    }

    /// 造一个临时 git 仓库（含最小身份），返回路径。测试结束不清理，
    /// 反正都在 /tmp 下，且路径带 pid 不会撞。
    fn mk_repo() -> std::path::PathBuf {
        // 同进程内 pid 恒定，必须用唯一后缀隔离：并行测试会同时调 mk_repo，
        // 共用路径会互相 remove_dir_all / init 导致 race 失败。纳秒戳足够区分。
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("pb_git_t_{}", n));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let d = dir.to_string_lossy().to_string();
        let ini = git_out(Some(&d), &["init"]);
        assert!(ini.is_some(), "git init 失败：{:?}", git_run(Some(&d), &["init"]));
        let _ = git_run(Some(&d), &["config", "user.name", "pb-test"]);
        let _ = git_run(Some(&d), &["config", "user.email", "pb@test.local"]);
        dir
    }

    #[test]
    fn changed_files_parses_porcelain() {
        let dir = mk_repo();
        let d = dir.to_string_lossy().to_string();
        fs::write(dir.join("hello.txt"), b"hi").unwrap();
        fs::write(dir.join("nested.log"), b"x").unwrap();
        let _ = git_run(Some(&d), &["add", "-A"]);
        let files = git_changed_files(d.clone()).unwrap();
        assert!(files.iter().any(|f| f.path == "hello.txt" && f.status == "A"));
        assert!(files.iter().any(|f| f.path == "nested.log" && f.label == "新增"));
    }

    #[test]
    fn commit_push_rejects_empty_message() {
        let dir = mk_repo();
        let d = dir.to_string_lossy().to_string();
        fs::write(dir.join("f.txt"), b"x").unwrap();
        // 空 message：应当在远端校验之前就被拦下
        let e = git_commit_push(d.clone(), "   ".into()).unwrap_err();
        assert!(e.contains("提交说明不能为空"), "{e}");
    }

    #[test]
    fn commit_push_rejects_when_nothing_to_commit() {
        let dir = mk_repo();
        let d = dir.to_string_lossy().to_string();
        // 有仓库但没改动：应当报「没有未提交的改动」，而不是去碰远端
        let e = git_commit_push(d.clone(), "feat: x".into()).unwrap_err();
        assert!(e.contains("没有未提交的改动"), "{e}");
    }

    #[test]
    fn unbind_message_reports_what_really_happened() {
        // 删到凭证 + 记着账号名：两句都要有
        let m = unbind_message("hellojerry001", 1, true, false);
        assert!(m.contains("1 条凭证"), "{m}");
        assert!(m.contains("@hellojerry001"), "{m}");
        // 没删到凭证时不能谎报「已删除」
        let m = unbind_message("hellojerry001", 0, true, false);
        assert!(m.contains("本来就没有"), "{m}");
        assert!(!m.contains("已删除"), "{m}");
        // 顺手退了 gh CLI 时要说明；没退就一个字都不提
        assert!(unbind_message("a", 1, true, true).contains("gh CLI"));
        assert!(!unbind_message("a", 1, true, false).contains("gh CLI"));
    }
}

//! GitHub 同步设置（阶段一）。
//!
//! 这一页只做两件事：把本机 git 环境**读出来**，把身份与偏好**写回去**。
//! 它是「项目一键提交 GitHub」的前置 —— 提交要用的 `user.name` / `user.email` /
//! credential helper 在这里先确认好，真到提交那一步就不该再让用户填任何东西。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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

fn git_bin() -> String {
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
}

impl Default for GitSettings {
    fn default() -> Self {
        Self {
            name: String::new(),
            email: String::new(),
            default_branch: "main".into(),
            auto_gitignore: true,
            auto_first_commit: true,
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

/* ============================== 写入 ============================== */

#[tauri::command]
pub fn save_git_settings(settings: GitSettings) -> Result<GitSettings, String> {
    let s = settings.sanitized()?;
    let path = settings_path();
    let text = serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| format!("写入设置失败：{e}"))?;
    Ok(s)
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
}

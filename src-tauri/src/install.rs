/* =============================================================================
   install.rs · 用下载好的 dmg 就地安装更新
   -----------------------------------------------------------------------------
   这个模块存在的真正理由不是「省掉几次拖拽」，而是**绕开 Gatekeeper**：
   经浏览器下载的文件会被打上 com.apple.quarantine，未公证的 app 首次打开必须去
   「系统设置 › 隐私与安全性」放行；而**本应用自己下载的 dmg 不带这个标记**
   （curl 不会加），替换进去的应用天然清白 —— 装完点一下就能直接运行，
   全程没有任何提示。这就是「点更新即完成升级」的完整闭环。

   流程全部用系统自带命令实现，不引第三方依赖：
     hdiutil attach -nobrowse -readonly -mountpoint <暂存挂载点> <dmg>
       → 在卷里找 .app
       → 校验 bundle id 一致、版本不比当前旧
       → ditto 到「当前应用所在目录」下的隐藏暂存名（同卷才能 rename 原子替换）
       → 旧包改名让位 → 暂存名改成正式名（失败就把旧包改回来，绝不把应用搞丢）
       → 校验签名，坏了补 ad-hoc；顺手 xattr -cr
       → 删旧包 / 卸载卷 / 清挂载点

   替换的是**当前正在运行的那个 .app 所在位置**（通常是 /Applications/VibeButler.app）。
   开发模式（target/debug/…）或从磁盘映像里直接运行时会被拒 —— 那两种情况就地
   替换自己没有意义，错误信息会告诉用户改用手动拖拽。
   ============================================================================= */

use serde::Serialize;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::projects;
use crate::update::version_gt;

const HDIUTIL: &str = "/usr/bin/hdiutil";
const DITTO: &str = "/usr/bin/ditto";
const CODESIGN: &str = "/usr/bin/codesign";
const XATTR: &str = "/usr/bin/xattr";
const PLIST_BUDDY: &str = "/usr/libexec/PlistBuddy";

/// 安装要替换哪些位置：只有这两个目录允许
const APPS: &str = "/Applications";

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    pub app_name: String,
    pub target: String,
    pub from_version: String,
    pub to_version: String,
    /// 拷贝后签名本来就不有效、由我们补了一次 ad-hoc 签名
    pub resigned: bool,
    pub dmg: String,
}

/* ============================== 路径守卫 ============================== */

/// 允许就地替换的目录：`/Applications` 与 `~/Applications`。
/// 其余位置一律拒绝 —— 开发模式的 `target/debug`、从 dmg 卷里直接运行
/// （或 Gatekeeper 的 AppTranslocation 路径）替换自己都没有意义。
fn ensure_installable(target: &Path) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| "安装目标没有上一级目录".to_string())?;

    if parent == Path::new(APPS) {
        return Ok(());
    }
    if let Ok(home) = std::env::var("HOME") {
        let h = home.trim();
        if !h.is_empty() && parent == Path::new(h).join("Applications") {
            return Ok(());
        }
    }

    Err(format!(
        "当前运行的 {} 不在「应用程序」目录里（实际在 {}），无法就地更新。\
         开发模式或从磁盘映像里直接启动时，请手动把安装包里的应用拖进「应用程序」。",
        target
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "应用".into()),
        parent.display()
    ))
}

/// 当前可执行文件所属的 `.app`
fn running_bundle() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("取当前可执行文件路径失败：{e}"))?;
    let mut p: &Path = &exe;
    for _ in 0..3 {
        // .../X.app/Contents/MacOS/<bin> → 退回三级就是 X.app
        p = p.parent().ok_or_else(|| format!("定位当前 .app 失败：{}", exe.display()))?;
    }
    if p.extension().map(|e| e == "app").unwrap_or(false) {
        Ok(p.to_path_buf())
    } else {
        Err(format!("当前不是从 .app 里运行的：{}", exe.display()))
    }
}

/* ============================== 挂载（带卸载守卫） ============================== */

/// 挂载好的磁盘映像。**必须**在 drop 时卸载：中途任何一步失败（版本不对、
/// 复制失败……）都不能把卷留在用户机器上。
struct Mount {
    point: PathBuf,
    on: bool,
}

impl Mount {
    fn attach(dmg: &Path, point: &Path) -> Result<Self, String> {
        // 上次异常退出可能把卷和空目录留在原地：先尽力卸掉再重建，
        // 否则 hdiutil 会因为挂载点被占用而失败
        if point.exists() {
            let _ = Command::new(HDIUTIL).arg("detach").arg(point).output();
        }
        let _ = fs::remove_dir_all(point);
        fs::create_dir_all(point).map_err(|e| format!("建挂载点失败：{e}"))?;
        let out = Command::new(HDIUTIL)
            .args([
                "attach",
                "-nobrowse",
                "-readonly",
                "-noautoopen",
                "-mountpoint",
            ])
            .arg(point)
            .arg(dmg)
            .output()
            .map_err(|e| format!("调用 hdiutil 失败：{e}"))?;
        if !out.status.success() {
            return Err(format!("挂载安装包失败：{}", cmd_error(&out)));
        }
        Ok(Mount {
            point: point.to_path_buf(),
            on: true,
        })
    }

    fn detach(&mut self) {
        if !self.on {
            return;
        }
        let _ = Command::new(HDIUTIL).arg("detach").arg(&self.point).output();
        self.on = false;
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        self.detach();
    }
}

/* ============================== 主流程 ============================== */

/// 把 `dmg` 里的应用装到 `target`（一个已存在的 .app 目录）。
/// 不负责重启 —— 重启需要 AppHandle，放在命令层。
pub fn install_from_dmg(dmg: &Path, target: &Path) -> Result<InstallReport, String> {
    if !dmg.is_file() {
        return Err(format!("安装包不存在：{}", dmg.display()));
    }
    if !dmg
        .extension()
        .map(|e| e.eq_ignore_ascii_case("dmg"))
        .unwrap_or(false)
    {
        return Err(format!("只支持 .dmg 安装包：{}", dmg.display()));
    }
    if !target.is_dir() {
        return Err(format!("找不到当前应用：{}", target.display()));
    }

    let parent = target
        .parent()
        .ok_or_else(|| "安装目标没有上一级目录".to_string())?
        .to_path_buf();
    let name = target
        .file_name()
        .ok_or_else(|| "安装目标没有名字".to_string())?
        .to_string_lossy()
        .to_string();

    let cur_id = plist_string(target, "CFBundleIdentifier");
    if cur_id.is_empty() {
        return Err(format!("读不到当前应用的标识：{}/Contents/Info.plist", target.display()));
    }
    let cur_ver = plist_string(target, "CFBundleShortVersionString");

    /* --- 挂载 --- */
    let mnt = projects::data_dir()
        .join("tmp")
        .join(format!("mnt-{}", std::process::id()));
    let _ = fs::remove_dir_all(&mnt);
    let mut mount = Mount::attach(dmg, &mnt)?;

    let src = find_app(&mount.point, Some(&name))?;

    /* --- 身份与版本校验：装错包 / 装回旧版都不可接受 --- */
    let new_id = plist_string(&src, "CFBundleIdentifier");
    if new_id != cur_id {
        return Err(format!(
            "安装包里的应用标识是 {new_id}，与当前的 {cur_id} 不一致，已中止安装"
        ));
    }
    let new_ver = plist_string(&src, "CFBundleShortVersionString");
    if !cur_ver.is_empty() && !new_ver.is_empty() && version_gt(&cur_ver, &new_ver) {
        return Err(format!(
            "安装包里的版本 {new_ver} 比当前 {cur_ver} 旧，已中止安装"
        ));
    }

    /* --- 暂存 → 换名（同一目录 rename，原子） --- */
    let pid = std::process::id();
    let staged = parent.join(format!(".{name}.new-{pid}"));
    let backup = parent.join(format!(".{name}.old-{pid}"));
    let _ = fs::remove_dir_all(&staged);
    let _ = fs::remove_dir_all(&backup);

    copy_bundle(&src, &staged)?;

    fs::rename(target, &backup).map_err(|e| format!("暂时移开旧版本失败：{e}"))?;
    if let Err(e) = fs::rename(&staged, target) {
        // 回滚：宁可什么都没变，也不能把应用留在「改了一半」的状态
        let _ = fs::rename(&backup, target);
        let _ = fs::remove_dir_all(&staged);
        return Err(format!("放入新版本失败（已把旧版本改回原位）：{e}"));
    }

    /* --- 签名与隔离标记 --- */
    // ditto 会连 _CodeSignature 一起搬过来，正常情况下签名仍然有效；
    // 只有它坏了才补一次 ad-hoc（补不补得上都不该让安装失败）。
    let resigned = if signature_ok(target) {
        false
    } else {
        resign_adhoc(target).is_ok()
    };
    clear_xattrs(target);

    /* --- 清场 --- */
    let _ = fs::remove_dir_all(&backup);
    mount.detach();
    let _ = fs::remove_dir_all(&mnt);

    Ok(InstallReport {
        app_name: name,
        target: target.to_string_lossy().to_string(),
        from_version: cur_ver,
        to_version: new_ver,
        resigned,
        dmg: dmg.to_string_lossy().to_string(),
    })
}

/// 在挂载卷里找要安装的 `.app`：优先与当前应用同名的那个；
/// 卷根找不到就往下找一层（有些 dmg 会把应用放进子目录）。
fn find_app(root: &Path, prefer: Option<&str>) -> Result<PathBuf, String> {
    let mut cands = list_apps(root);
    if cands.is_empty() {
        if let Ok(rd) = fs::read_dir(root) {
            for e in rd.flatten() {
                let p = e.path();
                // ⚠️ 也要跳软链：dmg 卷根的 `Applications` 正是指向 /Applications 的
                // 软链，跟进去会把用户装的所有应用都当成候选（实测 64 个）。
                if p.is_dir() && !is_symlink(&p) {
                    cands.extend(list_apps(&p));
                }
            }
        }
    }

    if cands.is_empty() {
        return Err(format!("安装包里没有找到 .app：{}", root.display()));
    }
    if let Some(want) = prefer {
        if let Some(hit) = cands
            .iter()
            .find(|p| p.file_name().map(|f| f.to_string_lossy() == want).unwrap_or(false))
        {
            return Ok(hit.clone());
        }
    }
    if cands.len() > 1 {
        let names: Vec<String> = cands
            .iter()
            .map(|p| p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default())
            .collect();
        return Err(format!(
            "安装包里有多个应用（{}），无法确定要装哪一个",
            names.join("、")
        ));
    }
    Ok(cands.remove(0))
}

/// 列出目录里的一级 `.app`（跳过符号链接 —— 卷根的 `Applications` 是软链）
fn list_apps(dir: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if is_symlink(&p) {
                continue;
            }
            if p.extension().map(|x| x == "app").unwrap_or(false) && p.is_dir() {
                v.push(p);
            }
        }
    }
    v.sort();
    v
}

/// 符号链接判定：`Path::is_dir()` 会跟着链接走，这里必须问 lstat
fn is_symlink(p: &Path) -> bool {
    p.symlink_metadata().map(|m| m.is_symlink()).unwrap_or(true)
}

/// 复制整包。用 ditto 而不是 cp：它会带上扩展属性与资源分支，
/// 复制出来的包与源包在签名意义上完全等价。
fn copy_bundle(src: &Path, dst: &Path) -> Result<(), String> {
    let out = Command::new(DITTO)
        .arg(src)
        .arg(dst)
        .output()
        .map_err(|e| format!("调用 ditto 失败：{e}"))?;
    if !out.status.success() {
        return Err(format!(
            "复制新版本到 {} 失败：{}（若是权限问题，可手动把安装包里的应用拖进「应用程序」）",
            dst.display(),
            cmd_error(&out)
        ));
    }
    if !dst.join("Contents/Info.plist").is_file() {
        return Err("复制出来的应用不完整（缺 Contents/Info.plist）".into());
    }
    Ok(())
}

fn signature_ok(app: &Path) -> bool {
    Command::new(CODESIGN)
        .args(["--verify", "--deep", "--strict"])
        .arg(app)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn resign_adhoc(app: &Path) -> Result<(), String> {
    let out = Command::new(CODESIGN)
        .args(["--force", "--deep", "--sign", "-"])
        .arg(app)
        .output()
        .map_err(|e| format!("补签名失败：{e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("补签名失败：{}", cmd_error(&out)))
    }
}

/// 清掉包上的全部扩展属性（含 `com.apple.quarantine`）。
/// 自下载的 dmg 本来就没有这个标记，这里是防御性的 —— 万一安装包是用户
/// 从浏览器手动下好、再喂给这个流程的，也别让 Gatekeeper 拦下来。
fn clear_xattrs(app: &Path) {
    let _ = Command::new(XATTR).args(["-cr"]).arg(app).output();
}

/// 读 Info.plist 里的字符串项；读不到返回空串（调用方自己决定是否致命）
fn plist_string(app: &Path, key: &str) -> String {
    let plist = app.join("Contents/Info.plist");
    let out = match Command::new(PLIST_BUDDY)
        .arg("-c")
        .arg(format!("Print :{key}"))
        .arg(&plist)
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return String::new(),
    };
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// 从命令输出里挑一句能读的原因（hdiutil/ditto 都往 stderr 写）
fn cmd_error(out: &std::process::Output) -> String {
    let err = String::from_utf8_lossy(&out.stderr);
    let text = if err.trim().is_empty() {
        String::from_utf8_lossy(&out.stdout).into_owned()
    } else {
        err.into_owned()
    };
    text.trim()
        .lines()
        .last()
        .unwrap_or("")
        .trim()
        .to_string()
}

/// 重启到新版本。**必须先退出自己**：同 bundle id 的旧进程还活着时，
/// `open -a` 只会把那个旧进程唤到前台，等于没重启。所以让一条脱离终端的 sh
/// 延迟 1 秒再 open，随后命令层立刻 exit。
fn relaunch(target: &Path) {
    let cmd = format!("sleep 1; exec /usr/bin/open -a {}", shell_quote(target));
    let _ = Command::new("/bin/sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
}

/// 单引号包裹并转义，用于塞进 sh -c
fn shell_quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"))
}

/* ============================== 命令 ============================== */

/// 安装下载好的更新包；`restart` 默认 true —— 装完自动退出并重新打开新版本。
#[tauri::command]
pub fn install_update(
    app: tauri::AppHandle,
    path: String,
    restart: Option<bool>,
) -> Result<InstallReport, String> {
    let target = running_bundle()?;
    // 先卡路径：别等把包都挂上、复制完了才发现根本不该装
    ensure_installable(&target)?;

    let report = install_from_dmg(&PathBuf::from(path.trim()), &target)?;

    if restart.unwrap_or(true) {
        relaunch(&target);
        app.exit(0);
    }
    Ok(report)
}

/* ============================== 测试 ============================== */

#[cfg(test)]
mod tests {
    use super::*;

    fn touch_app(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        fs::create_dir_all(p.join("Contents")).unwrap();
        fs::write(p.join("Contents/Info.plist"), b"x").unwrap();
        p
    }

    /// 开发模式（`target/debug/…`）下必须拒绝：否则会去「替换」一个根本不在
    /// 应用程序目录里的东西。测试进程本身就不在 .app 里，正好能验这条。
    #[test]
    fn running_bundle_refuses_a_non_bundle_location() {
        assert!(running_bundle().is_err());
    }

    #[test]
    fn only_applications_dirs_are_installable() {        assert!(ensure_installable(Path::new("/Applications/VibeButler.app")).is_ok());

        let home = std::env::var("HOME").unwrap();
        let mine = PathBuf::from(&home).join("Applications/VibeButler.app");
        assert!(ensure_installable(&mine).is_ok());

        // 开发模式 / 临时目录 / 卷里直接跑，都不允许就地替换
        assert!(ensure_installable(Path::new("/tmp/x/VibeButler.app")).is_err());
        assert!(ensure_installable(Path::new("/Users/jerry/port-butler/src-tauri/target/debug/X.app")).is_err());
        assert!(ensure_installable(Path::new("/Volumes/VibeButler/VibeButler.app")).is_err());
    }

    #[test]
    fn find_app_prefers_the_same_name() {
        let root = std::env::temp_dir().join(format!("pb-findapp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        touch_app(&root, "Other.app");
        touch_app(&root, "VibeButler.app");

        let got = find_app(&root, Some("VibeButler.app")).unwrap();
        assert_eq!(got.file_name().unwrap().to_string_lossy(), "VibeButler.app");

        // 名字对不上又只有一个候选时，照样能装
        let only = std::env::temp_dir().join(format!("pb-findapp-only-{}", std::process::id()));
        let _ = fs::remove_dir_all(&only);
        fs::create_dir_all(&only).unwrap();
        touch_app(&only, "Renamed.app");
        assert!(find_app(&only, Some("VibeButler.app")).is_ok());

        // 多个候选且没有能对上的名字 → 拒绝，让用户自己判断
        assert!(find_app(&root, Some("Nothing.app")).is_err());

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&only);
    }

    #[test]
    fn find_app_looks_one_level_deep() {
        let root = std::env::temp_dir().join(format!("pb-findapp-deep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("inner")).unwrap();
        touch_app(&root.join("inner"), "VibeButler.app");

        let got = find_app(&root, Some("VibeButler.app")).unwrap();
        assert!(got.ends_with("inner/VibeButler.app"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn find_app_skips_symlinks_and_reports_empty() {
        let root = std::env::temp_dir().join(format!("pb-findapp-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink("/Applications", root.join("Applications")).unwrap();

        let err = find_app(&root, None).unwrap_err();
        assert!(err.contains("没有找到"), "实际：{err}");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote(Path::new("/Applications/A.app")), "'/Applications/A.app'");
        // 名字里带单引号也必须还是一个参数
        assert_eq!(shell_quote(Path::new("/tmp/a'b.app")), "'/tmp/a'\\''b.app'");
    }

    /// 真机集成测试：拿**真实 dmg** 跑完整流程（挂载 → 校验 → 原子替换 → 签名 → 清理）。
    /// 用环境变量指定输入，缺任一个就跳过（不联网、不依赖固定路径）：
    ///   PB_TEST_DMG=/…/dist/VibeButler_0.5.0_aarch64.dmg
    ///   PB_TEST_APP=/…/bundle/macos/VibeButler.app    # 当作「当前版本」被替换掉
    /// 跑法：`cargo test --release -- --ignored --nocapture installs_from_a_real_dmg`
    /// 全程只在临时目录里折腾，不碰 /Applications。
    #[test]
    #[ignore = "需要真实 dmg 与 .app，用 PB_TEST_DMG / PB_TEST_APP 指定"]
    fn installs_from_a_real_dmg_into_a_temp_dir() {
        let (dmg, app) = match (std::env::var("PB_TEST_DMG"), std::env::var("PB_TEST_APP")) {
            (Ok(d), Ok(a)) => (PathBuf::from(d), PathBuf::from(a)),
            _ => {
                eprintln!("跳过：未设置 PB_TEST_DMG / PB_TEST_APP");
                return;
            }
        };

        let root = std::env::temp_dir().join(format!("pb-inst-real-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let target = root.join("VibeButler.app");

        // 造一个「当前版本」：内容取自 PB_TEST_APP，另打一个标记文件便于断言替换
        assert!(Command::new(DITTO).arg(&app).arg(&target).status().unwrap().success());
        fs::write(target.join("Contents/MARKER"), b"old").unwrap();
        let plist = target.join("Contents/Info.plist");
        let want = plist_string(&app, "CFBundleShortVersionString");
        assert!(!want.is_empty(), "读不到测试用 .app 的版本号");

        let set_ver = |v: &str| {
            assert!(Command::new(PLIST_BUDDY)
                .arg("-c")
                .arg(format!("Set :CFBundleShortVersionString {v}"))
                .arg(&plist)
                .status()
                .unwrap()
                .success());
        };

        // 1) 当前版本更新（9.9.9）时装 0.5.0 → 必须拒绝，且原 app 一动不动
        set_ver("9.9.9");
        let err = install_from_dmg(&dmg, &target).unwrap_err();
        assert!(err.contains("比当前"), "实际：{err}");
        assert!(target.join("Contents/MARKER").is_file(), "被拒后不该改动现有 app");

        // 2) 版本正常 → 真装
        set_ver(&want);
        let rep = install_from_dmg(&dmg, &target).unwrap();
        assert_eq!(rep.to_version, want);
        assert_eq!(rep.from_version, want);
        assert!(!target.join("Contents/MARKER").is_file(), "新包应替换掉旧包");
        assert!(signature_ok(&target), "装完签名必须有效（否则用户会遇到「已损坏」）");

        // 3) 清场：不留暂存/备份目录，挂载点已卸载并删除
        let leftovers: Vec<String> = fs::read_dir(&root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with(".VibeButler.app."))
            .collect();
        assert!(leftovers.is_empty(), "有残留目录：{leftovers:?}");

        let mnt = projects::data_dir()
            .join("tmp")
            .join(format!("mnt-{}", std::process::id()));
        assert!(!mnt.exists(), "挂载点没清掉：{}", mnt.display());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_non_dmg_and_missing_files() {
        let target = std::env::temp_dir().join(format!("pb-inst-{}", std::process::id()));
        let _ = fs::remove_dir_all(&target);
        touch_app(&target, "VibeButler.app");
        let app = target.join("VibeButler.app");

        let missing = PathBuf::from("/tmp/definitely-not-here.dmg");
        assert!(install_from_dmg(&missing, &app).unwrap_err().contains("不存在"));

        let wrong = target.join("x.zip");
        fs::write(&wrong, b"x").unwrap();
        assert!(install_from_dmg(&wrong, &app).unwrap_err().contains("只支持 .dmg"));

        let _ = fs::remove_dir_all(&target);
    }
}

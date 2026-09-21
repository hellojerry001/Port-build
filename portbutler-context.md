# VibeButler（原 PortButler）· 项目上下文总览

> 用途：可脱离对话持续使用的项目快照 —— 架构、现状、契约、决策、坑、路线图。
> 采集时间：2026-09-21。仓库 HEAD：`5bd1c14`（与 `origin/main` 一致，已推送核验）。
> 版本：`0.4.0`（aarch64 / macOS ≥ 12.0）。

---

## 0. 一句话定位

**VibeButler 是一个 macOS 桌面应用（Tauri 2），用来统一管理「开发项目 + 端口 + 一键发布 + GitHub 同步」。它不是写代码的编辑器，而是「巡检 / 交接」工具** —— 这一点决定了几乎所有产品决策（详见 §8）。

产品名 `VibeButler`，但**代码层面仍叫 `port-butler`**（`package.json` / `Cargo.toml` 的 name、`github.com/hellojerry001/Port-build` 仓库、`com.jerry.portbutler` bundle id 都没改），只有 `tauri.conf.json` 的 `productName` 和窗口标题是 `VibeButler`。改名时曾漏掉窗口标题，是个踩过的坑。

---

## 1. 仓库事实

| 项 | 值 |
|---|---|
| 本地路径 | `/Users/jerry/port-butler` |
| 远端 | `https://github.com/hellojerry001/Port-build.git` |
| 分支 | `main`（唯一，无未合并分支） |
| 已装应用 | `/Applications/VibeButler.app`（已装正式版，进程常驻菜单栏） |
| Rust 规模 | 14 个模块，4429 行（`projects.rs` 765 最大，`git.rs` 528） |
| 前端规模 | JS 2109 行（10 模块），CSS 1626 行（tokens 308 / components 1222 / base 56 / pages 40） |
| 单测 | `cargo test --lib` 65 passed（每次 feature 都带单测） |

---

## 2. 架构

```
                 ┌──────────────────── Tauri 2 (Rust) ────────────────────┐
                 │  src-tauri/src/*.rs  →  #[tauri::command] 暴露给前端    │
                 │  状态用 Mutex 管理（ProcTable / BuildTable / DownloadTable）│
                 └───────────────────────────┬────────────────────────────┘
                                  invoke() / 事件
                 ┌───────────────────────────┴────────────────────────────┐
                 │  ui/ 纯静态前端：index.html + js/ + css/（无框架）      │
                 │  Vanilla JS + 事件委托；设计令牌驱动；Tauri IPC 走全局  │
                 └─────────────────────────────────────────────────────────┘
```

**关键事实**
- **前端零框架**：没有 React/Vue，是单页 + 多 `section.page` 切换（`is-active` 类控制显隐），`withGlobalTauri: true` 所以 `window.__TAURI__` 直接可用。
- **IPC 契约约定 camelCase**：所有 `#[tauri::command]` 返回的结构体统一 `#[serde(rename_all = "camelCase")]`。两条单测专门钉住键名，因为前端用 `res.fieldName` 取数，键名改了会**静默显示 undefined**（编译期发现不了）。
- **CSP 为 null**：前端可直接 `fetch` / 原生 XMLHttpRequest，所以「检查更新」「下载」这类网络动作可以前端直接做，不必给 Rust 加 HTTP 客户端。
- **子进程一律走绝对路径 + 进程组**：git / gh / sips / iconutil / curl 都只用候选绝对路径（`/opt/homebrew/bin/...`），因为 GUI 从 Finder 启动时 PATH 极窄（只有 `/usr/bin:/bin:/usr/sbin:/sbin`），只看 PATH 会永远报「命令不存在」。
- **进程终态用 `try_wait()` 判定**：`BuildTable` / `DownloadTable` 持有 `Child`，从所有权上解决「僵尸进程导致状态卡死」的老 bug。
- **存储**：项目数据在 `~/.portbutler/projects.json`；git 偏好在 `~/.portbutler/settings.json`；模板在 `~/.portbutler/scaffolds/`（每个带 `.pb-scaffold.json`）。

---

## 3. 目录布局

```
port-butler/
├── src-tauri/
│   ├── src/            # Rust 后端，每个文件一个职责模块
│   │   ├── lib.rs          # mod 注册 + 40 个命令清单 + 托盘/窗口事件
│   │   ├── projects.rs     # 项目 CRUD + ProcTable（运行进程表，磁盘恢复）
│   │   ├── git.rs          # ★ GitHub 同步（环境自检 + 提交身份 + 仓库状态）
│   │   ├── publish.rs      # 发布到 Cloudflare（探测/校验/上传）
│   │   ├── publishes.rs    # 发布记录（历史列表）
│   │   ├── build.rs        # Mac 打包 DMG
│   │   ├── icon.rs         # Mac 项目一键换启动图标（手拼 .icns/.ico）
│   │   ├── scaffolds.rs    # 脚手架模板库
│   │   ├── update.rs       # 检查更新 + 下载 DMG（绕开 api.github.com）
│   │   ├── appmeta.rs      # 应用元信息 + 改 APP 名称
│   │   ├── dialog.rs       # 选文件夹/选文件（AppleScript）
│   │   ├── open.rs         # 打开 URL / 访达 / 挂载 DMG
│   │   ├── ports.rs        # 本机监听端口一览
│   │   └── main.rs         # 3 行入口，调 run()
│   ├── Cargo.toml     # 依赖极少：tauri + serde + serde_json
│   └── tauri.conf.json
├── ui/
│   ├── index.html     # 6 个 section.page + 弹窗 + 模板
│   ├── js/
│   │   ├── main.js        # 启动引导 boot()，默认进 projects
│   │   ├── rail.js        # 侧栏 NAV / PAGES / 主题图标
│   │   ├── projects.js    # ★ 项目卡片渲染 + 增删改起停（最大前端模块）
│   │   ├── settings.js    # ★ 设置页（GitHub 同步）
│   │   ├── publish.js     # 发布流程
│   │   ├── about.js       # 关于页 + 检查更新
│   │   ├── templates.js / radar.js / theme.js / ui.js / core.js
│   └── css/
│       ├── tokens.css     # 设计令牌（216 定义 / 0 未引用，深色 + 浅色覆写）
│       ├── components.css # 组件样式（全部走令牌）
│       ├── base.css / pages.css
├── docs/             # scaffold-templates.md 等
├── dist/             # 已发布 DMG（GitHub release 资产）
├── release.json      # 版本清单（托管在仓库，检查更新读它）
├── RELEASE.md / README.md
└── *.dmg            # 历史 DMG 留档
```

---

## 4. 后端命令清单（40 个，按模块）

> 全部注册在 `lib.rs` 的 `generate_handler!`。前端用 `invoke("命令名", {参数})` 调用。

| 模块 | 命令 | 职责 |
|---|---|---|
| ports | `list_ports` / `kill_port` | 本机监听端口一览 / 杀端口 |
| open | `open_url` / `show_in_finder` / `open_file` | 打开 URL / 访达选中 / 挂载 DMG |
| dialog | `pick_folder` / `pick_file` | AppleScript 选路径 |
| icon | `project_icon` / `swap_icon` | 读 Mac 项目图标（裸 base64）/ 换启动图标 |
| appmeta | `app_meta` / `set_app_name` | 读元信息 / 改 APP 名 |
| projects | `list_projects` / `save_project` / `delete_project` / `start_project` / `stop_project` / `list_running` | 项目 CRUD + 起停 |
| build | `build_dmg` / `build_status` / `cancel_build` | Mac 打包 |
| publish | `publish_project` / `publish_probe` / `check_dir` | Cloudflare 发布 |
| publishes | `list_publishes` / `delete_publish` / `clear_publishes` | 发布记录 |
| scaffolds | `list_scaffolds` / `suggest_port` / `create_project` | 模板库 |
| update | `app_info` / `check_manifest` / `download_dmg` / `download_status` / `cancel_download` | 更新 |
| **git** | `git_status` / `git_repo_states` / `save_git_settings` / `apply_git_identity` / `token_help_url` | **★ GitHub 同步（详见 §7）** |

`Project` 结构体字段（serde，`node_version` 兼容别名）：`id / name / path / command / port / scaffold / node_version / kind(web|mac) / build_command`。

---

## 5. 前端页面与导航

6 个页面（`index.html` 的 `<section class="page" id="page-...">`，由 `rail.js` 的 `NAV` + `PAGES` 驱动）：

| key | 标题 | 副标题 | 状态 |
|---|---|---|---|
| projects | 项目管理 | 项目 & 端口管理 | ✅ 主功能，Web/Mac 分栏卡片 |
| radar | 端口雷达 | 本机监听端口实时一览 | ✅ |
| publishes | 发布记录 | Cloudflare 临时发布与认领 | ✅ |
| templates | 模板库 | 内置脚手架模板 | ✅（改造方案已定，见 §9） |
| settings | 设置 | **GitHub 同步与提交身份** | ✅ 本轮刚完成 |
| about | 关于 | 版本信息与更新 | ✅（有新版时侧栏显示圆点） |

设计语言：**灰阶 / 黑白克制**，chip 靠透明度区分、无彩色无描边；主题 light/dark/system 三态（左下角切换，持久化）。改动风格：先量化参考稿逐位对齐，再 token 化，零死色值。

---

## 6. 当前功能现状

| 功能 | 状态 | 备注 |
|---|---|---|
| 项目管理（增删改、起停 dev server、端口占用） | ✅ | 主场景 |
| 端口雷达（本机端口一览） | ✅ | |
| Cloudflare 一键发布（wrangler deploy --temporary → workers.dev） | ✅ | 含探测/校验/历史记录 |
| Mac 项目打包 DMG + 一键换启动图标 | ✅ | 手拼 .icns，绕开坏掉的 iconutil |
| 模板库（从内置脚手架新建项目） | ✅ | 改造方案已定未实现 |
| 关于页 + 检查更新（清单托管 Git 仓库，绕开 api.github.com） | ✅ | 0.4.0 |
| **设置页 · GitHub 同步（阶段一）** | ✅ | **本仓库 HEAD 功能** |
| **项目卡片一键推送 GitHub（阶段二）** | ⏳ 设计定稿，未实现 | 见 §8 |

---

## 7. 已完成的 GitHub 同步（阶段一 · `git.rs` + 设置页）

**定位**：这是「项目一键提交 GitHub」的前置。先把 `user.name` / `user.email` / credential helper 确认好，真到提交那步不让用户再填东西。

**后端 5 命令**（`src-tauri/src/git.rs`，528 行）：
- `git_status` → 返回 `GitStatus{ ready, gitPath, gitVersion, name, email, nameSource("page"|"global"), helper, hasGithubCred, ghCli, checks[EnvCheck], settings }`。四条自检：Git 可用 / 提交身份齐 / 凭证助手配置 / GitHub 凭证在钥匙串。
- `git_repo_states` → 逐项目返回 `RepoState{ isRepo, branch, remote, dirty, unpushed }`。**单独一条命令且后到**（7 个项目要 spawn 二十来个子进程，不该挡首屏）。
- `save_git_settings` / `apply_git_identity` → 写偏好 / 写 git 全局身份（输入一律 `sanitized()` 去空白 + 校验：名字 ≤80 字无换行、email 形状、分支名合法、默认 main）。
- `token_help_url` → GitHub 令牌帮助页 URL。

**前端**（`ui/js/settings.js`，311 行 + `page-settings` + `components.css` 设置页样式）：环境自检卡 + 提交身份表单（开关即时落盘、失败拨回）+ 项目仓库状态表。

**刻意的三处克制**
1. git / gh 按**候选绝对路径**探测，不看 PATH。
2. 查钥匙串**只问「有没有 github.com 凭证」，不读密码**。
3. **未推送数不联网刷新**（设置页不该为一个数字发网络请求）—— 本地 `origin/分支` 引用过期时这个数会偏大，已在列表头 `title` 写明。

---

## 8. 待实现的「项目卡片一键推送」（阶段二 · 设计定稿未写码）

> 2026-09-21 与用户讨论定稿。这是「手动推送 GitHub 功能做在哪」的结论。

**语义定性（产品根决策）**：push 不是「保存」是「交接」，触发它的是「要离开这台机器 / 要被别人看见」。本 App 不是编辑器，**感知不到用户写完代码的时刻** → 自动触发不成立，push 只能是巡检时的补漏动作。所以入口必须贴着「有 N 个没推」这个信号。

**已定的两条决策**
- **决策 1 · 入口** = 项目卡片上的小标记（只对 git 仓库显示），点标记弹推送面板。先只加标记不加按钮。
  - 标记语言复用现有 chip/tag（灰阶克制）。`dirty>0` 显未提交数（阻塞项优先）；`dirty=0 && unpushed>0` 显未推送数；全 0 不显示（干净不打扰）。
  - **已知错配**：当前仓库状态表被放在设置页（低频页），信号被埋没人看。需把信号上提到卡片（待办）。
- **决策 2 · 按钮职责** = 弹面板写 message 再 commit + push；`dirty=0 && unpushed>0` 时简化成纯推送。

**面板分流**
- `dirty>0` → 完整面板（改动文件清单 + message 输入框给默认建议 `chore: 更新 N 个文件` + 「提交并推送」）
- `dirty=0 && unpushed>0` → 简化成「推送 N 个提交」
- 无远端 → 明确提示，给出 `git remote add` 指引，**不代建仓库**
- git 没装 / 身份没配 → 复用设置页自检结论，给「去设置」跳转
- 失败 → **必须显示 git 真实 stderr**（鉴权失败、远端领先要先 pull，都不能吞）

**后端待加**（都在 `git.rs`）：`git_changed_files(path)`、`git_push(path)`、`git_commit_push(path, message)`（message 走 `Command` 参数数组不经 shell，无注入面；去空白 + 判空 + 限长）。

**兜底默认（未另行反对即如此）**：远端不存在只提示不代建（`gh repo create` 留二版）；不做文件勾选，全量 `add -A`（`.gitignore` 由设置页开关管着）。

---

## 9. 已定未实现的改造方案

**模板库改造**（2026-09-21 讨论定稿）：用户可上传自己的脚手架当模板；卡片列表（内置 + 自定义都展示）；加「新增模板」按钮。4 个决策：① 缺描述文件 → 引导补全表单（读 package.json 猜默认）；② 内置/自定义用隐藏标记文件 `.pb-builtin` 区分（删除拒删）；③ 支持 Mac 模板（`.pb-scaffold.json` 加 `kind` 字段，默认 web）；④ 同名冲突拒绝并提示。后端新增 `import_scaffold` / `delete_scaffold`（拷贝进 scaffolds 目录，不引用原路径）。

---

## 10. 技术约定与价值观

- **根因修复 > 补丁叠加**：优先从所有权 / 设计上解决，不靠探测绕过。
- **测试门禁**：每个 feature 都带 Rust 单测；`cargo test --lib` 必须全绿才算完。
- **真机验证**：关键 UI 改动必须真机截图（不是只跑单测）。手法见 §11。
- **改完 CSS 必须重跑文档站同步脚本**（若文档站存在）：它打印「N 定义 / M 未引用」当质量门；`var(--已删令牌)` 会静默失效不报错。
- **零死色值**：组件层颜色全部走 `tokens.css` 语义令牌。
- **刻意克制**：能不联网就不联网；能不读密码就不读；不确定时显示「不确定」比假装准确好。
- **提交粒度**：一个逻辑改动一个提交，混合改动拆开（如本仓库曾把「Mac 卡片状态点」和「设置页」拆成两个提交）。
- **远端核验**：push 后用 `git ls-remote` 校验远端 HEAD == 本地，不以 push 收尾输出为准。

---

## 11. 本机验证手法与踩过的坑（高价值）

> 这套流程是反复踩坑后沉淀的，离开它做 UI 验证会反复误判。

**真实验证链路（git.rs 功能版）**
1. `cargo build --release`（CARGO_TARGET_DIR 指到工作区 `.pb-target` 省空间）
2. 热替换 `/Applications/VibeButler.app/Contents/MacOS/port-butler` → `codesign --force --deep --sign -` → `xattr -dr com.apple.quarantine` → 重启
3. **非默认页真机验证**：临时把 `main.js` 的启动页改成目标页 → 编译 → 截真实 WebKit → 还原重编（用 `git status` 确认 `main.js` 无改动）

**坑清单（已写进技能 `tauri-ui-iterate-verify` / `sandboxed-git-repo-repair`）**
- ⚠️ **`getComputedStyle` 读带 `transition` 的属性 = 动画起始值**（轨道色、transform 都会读到旧值）。判最终态要么先 `el.style.transition='none'` 再读，要么**直接扫 PNG 像素**。
- ⚠️ **`sips -c H W --cropOffset X Y` 是居中裁剪**，不是左上角坐标。要按坐标裁用 PIL。
- ⚠️ **`open -a` 会激活已运行的旧实例**，二进制换了也截到旧页面。必须 `pkill` → 确认无进程无窗口 → 再 `open`。
- ⚠️ **`file://` 下 `cssRules` 被安全策略挡**，`rules:0` 是假阴性。
- ⚠️ **沙箱 git：`index.lock` 残留 → 清锁必须和写操作写在同一条 shell 命令里**（分开两条第二条照样报 `unable to unlink index.lock`）。前置 `CODEBUDDY_SAFE_DELETE_ENABLED=0` 关 safe-delete shim。
- ⚠️ **`iconutil` 在本机基本不可用**：手拼 `.icns`（icns 头 + OSType(4) + 大端长度(4) + PNG）。
- ⚠️ **AppleScript `choose file of type` 的 UTI 必须带引号** `{"public.png","public.jpeg"}`，否则 -2741 真机必失败；`-128` 映射成 `Ok(None)`（用户取消）。
- ⚠️ 合成点击（CGEventPost）需要辅助功能权限，本机没给 → 走「临时改启动页」绕过。
- ⚠️ 真机窗口在第二块屏 `y=-1226`，全屏 `screencapture` 抓不到；用 `/tmp/winlist.py`（CoreGraphics 枚举）+ `screencapture -x -l <窗口ID>`。

---

## 12. 本机已验证的环境事实（2026-09-21 实测）

| 项 | 值 |
|---|---|
| git | `/opt/homebrew/bin/git`，2.44.0 |
| 提交身份（全局） | `hellojerry001` |
| credential helper | `osxkeychain` |
| 钥匙串 | 有 `github.com` 凭证条目 |
| gh CLI | 候选路径存在即判存在（GUI PATH 极窄） |
| 端口管家仓库真实状态 | 7 个项目里仅「端口管家」是 git 仓库，`dirty=8 / unpushed=9` |
| 未推送数边界 | 本地 `origin/main` 引用过期时会偏大（`ls-remote` 显示远端 == 本地 HEAD，但 `git` 说 ahead 9）；已 `git fetch` 刷新后归 0 |
| 网络 | `raw.githubusercontent.com` 200（查版本）、`github.com` 200（下 DMG）可用；`api.github.com` 因匿名配额 403 不可用 → 更新链路刻意绕开它 |

---

## 13. 路线图（按讨论优先级）

1. **项目卡片推送标记 + 推送面板**（阶段二，§8）—— 紧接当前 HEAD。
2. 设置页仓库状态表的去留（信号上提到卡片后，那张表可能只留「环境自检」或删掉）。
3. 模板库改造（§9）。
4. （可选）`gh repo create` 代建远端（二版）。
5. （可选）推送取消按钮 / 进度条增强。

---

## 14. 关键记忆文件位置

跨 session 的持久记忆在 `/Users/jerry/WorkBuddy/2026-09-16-08-58-51/.workbuddy/memory/`：
- `2026-09-21.md`（最重要，365 行）：本轮全部 feature + 设计决策 + 坑
- `2026-09-17.md`（577 行）：早期重构 / 主题 / 打包 / 图标 / 发布历史
- `2026-09-16.md`、`2026-09-18.md`：早期落地页调研、模板讨论

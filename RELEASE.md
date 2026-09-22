# 版本清单与发版流程

应用「关于」页的「检查更新」读的就是本仓库根目录的 **`release.json`**，
安装包放在 **`dist/`**。整条链路只走 GitHub 的 raw 直链，不需要 token、不需要 API。

```
应用 ──fetch──> raw.githubusercontent.com/<repo>/main/release.json   (查版本)
     ──curl──>  raw.githubusercontent.com/<repo>/main/dist/*.dmg     (下安装包)
```

## 为什么不用 GitHub Releases API

`api.github.com` 对本机出口 IP 的匿名配额是 **60 次/小时**，且与其他共用该出口的
请求共享额度（实测已被耗尽，返回 403）。而 `raw.githubusercontent.com` 不限流、
无需鉴权，返回头还带 `access-control-allow-origin: *`，前端可以直接 fetch。
发版因此简化成「改一个 JSON + 放一个文件 + push」，不需要配任何凭证。

> 若将来想让仓库不背安装包体积，可以把 `dmg` 换成 Release 资产直链
> （`https://github.com/<owner>/<repo>/releases/download/v<版本>/<文件>`）——
> 那只改 `release.json` 里这一行即可，应用侧不用动。注意下载地址必须是
> `github.com` / `objects.githubusercontent.com` / `raw.githubusercontent.com`
> 三个主机之一，应用会校验（见 `src-tauri/src/update.rs` 的 `ALLOWED_HOSTS`）。

## release.json 字段

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `version` | ✅ | 最新版本号，与 `package.json` / `tauri.conf.json` / `Cargo.toml` 保持一致 |
| `date` | | 发布日期，展示在「发现新版本」卡片上 |
| `notes` | | 更新说明，字符串数组，每项一行 |
| `dmg` | | 安装包完整地址。留空则用下面的 `dmg_name` 拼 Release 资产直链 |
| `dmg_name` | | 安装包文件名。`dmg` 也留空时不显示下载按钮 |
| `dmg_size` | | 字节数，仅用于展示 |
| `min_macos` | | 最低系统版本，仅用于展示 |
| `mandatory` | | 预留：强制更新标记，当前仅透传给前端 |

解析与版本比较都在 Rust 侧（`update.rs` 的 `check_manifest`），字段全部有默认值——
少写一项不会让整个「检查更新」失败，能显示多少显示多少。

## 发版步骤

```bash
V=0.4.0                       # 新版本号

# 1. 三处版本号一起改（漏一处会导致「已是最新」判断错乱）
#    package.json / src-tauri/tauri.conf.json / src-tauri/Cargo.toml

# 2. 打包（产物在 src-tauri/target/*/bundle/macos/ 下）
npm run tauri build -- --bundles app

# 2.5 ⚠️ 必做：确认 .app 拿到了 ad-hoc 签名，否则用户下载后报「已损坏」
codesign --verify --deep --strict src-tauri/target/release/bundle/macos/VibeButler.app
# 期望：valid on disk / satisfies its Designated Requirement
# 若报 "code has no resources but signature indicates they must be present" → 补签：
codesign --force --deep --sign - src-tauri/target/release/bundle/macos/VibeButler.app

# 3. 手工打 DMG（Tauri 内置的 dmg 打包在沙箱里会失败）
STAGE=$(mktemp -d)
cp -R src-tauri/target/release/bundle/macos/VibeButler.app "$STAGE/"
ln -sf /Applications "$STAGE/Applications"
hdiutil create -volname VibeButler -srcfolder "$STAGE" -ov -format UDZO \
  "dist/VibeButler_${V}_aarch64.dmg"

# 4. 更新 release.json 的 version / date / notes / dmg / dmg_name / dmg_size

# 5. 提交推送（dmg 走 .gitignore 白名单，见 dist/*.dmg 那条）
git add -A && git commit -m "release: v$V" && git push origin main
```

推送完成后，旧版本的应用点「检查更新」即可看到新版本。

> ⚠️ **`release.json` 必须一次性改完再推**：`version` 和 `dmg` / `dmg_name` /
> `dmg_size` 要落在**同一次提交**里。2026-09-22 发 0.5.0 时先推了 `version`、
> 3 分钟后才补 `dmg`，这 3 分钟（再叠加 CDN 的 5 分钟缓存）里检查更新会拿到
> 「有新版本号、没有地址」的半状态清单：卡片显示「发现新版本」却点不出下载。
> 现在这种清单不会再给出「下载并安装」按钮（改为一行说明，见 `about.js` 的
> `noPkg`），但半状态本身仍应避免。

> ⚠️ **别急着验收**：`raw.githubusercontent.com` 的 CDN 有最长 5 分钟的缓存
>（响应头 `cache-control: max-age=300`），且实测它**忽略查询串** ——
>换 `?t=时间戳` 拿到的 ETag 和不带参数时完全一样，所以加时间戳只能绕开
>本机 webview 的缓存，绕不开 CDN。刚 push 完就点「检查更新」可能仍报
>「已是最新」，等几分钟再点一次即可。
>
> 2026-09-22 复核过一次，结论更硬：同一秒内请求「无参数」「随机 `?t=a1b2`」
> 「随机 `?t=c3d4`」，三者的 `source-age` **同步递增**（282 / 282 / 283，8 秒后
> 292 / 292）—— 说明三个 URL 命中的是同一个缓存对象，查询串确实被丢弃。
> 另外 `api.github.com` 的匿名配额（60 次/小时，按出口 IP）实测已耗尽并返回
> 403，**不能**拿它当「绕过 CDN 的第二来源」。

## 签名与 Gatekeeper（用户说「已损坏」时看这里）

`.app` 的签名由 `tauri.conf.json` 的 `bundle.macOS.signingIdentity` 决定，当前已配成 `"-"`（ad-hoc 伪身份）。

**不配这一项时 Tauri 会完全跳过 codesign**，产物只剩链接器给主程序打的标记（`flags=0x20002(adhoc,linker-signed)`、`Sealed Resources=none`、`Info.plist=not bound`、没有 `_CodeSignature/`），`codesign --verify` 直接报错。这种包在本机双击没问题（没有下载隔离标记），但**从浏览器下载后必被 Gatekeeper 判「已损坏，无法打开。你应该将它移到废纸篓。」—— 而那个对话框没有任何放行入口**。

| 包的状态 | 用户从浏览器下载后双击的表现 |
| --- | --- |
| 没签名（只有 linker 标记） | 「已损坏」→ 只能删掉，**无放行入口** |
| ad-hoc 签名（`signingIdentity: "-"`） | 仍会拦，但可在「系统设置 › 隐私与安全性」点「仍要打开」放行 |
| Developer ID + 公证 | 直接打开，无提示（需付费账号 $99/年 + notarytool） |

也就是说 ad-hoc 签名是**必要的下限，不是充分条件**。要彻底免掉「仍要打开」这一步只有两条路：

1. 付费 Apple Developer 账号 → 用 `Developer ID Application` 证书签名并公证（Tauri 侧配 `APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID`）。
2. **让 App 自己下载并安装更新** —— 已实现（`src-tauri/src/install.rs` + 关于页的「安装并重启」）：
   App 用 curl 下的 dmg 不会带 `com.apple.quarantine`，装进去的新版本直接就能打开，
   全程没有任何 Gatekeeper 提示。挂载 → 校验 → `ditto` 到暂存名 → 同目录原子换名
   （失败回滚）→ 校验/补签名 → `xattr -cr` → 自动重启，全在 App 内完成。

> ⚠️ 第 2 条只对**下下个版本起**有效：用户手上那一版必须已经包含 `install.rs`。
> 比如 0.5.0 的包里还没有这个功能，所以「0.5.0 → 0.5.1」这一次升级仍需手动拖拽
> （或跑一次 `xattr -cr`）；从 0.5.1 升 0.5.2 就能一次点击到位了。

判断某个包是哪种状态：

```bash
codesign -dv --verbose=2 <App>.app 2>&1 | grep -E "CodeDirectory|Sealed|Info.plist"
# 好：flags=0x10002(adhoc,runtime) / Sealed Resources version=2 rules=13 files=1 / Info.plist entries=N
# 坏：flags=0x20002(adhoc,linker-signed) / Sealed Resources=none / Info.plist=not bound
```

用户已经装上了、但提示「已损坏」时的兜底（在用户机器上执行一次）：

```bash
xattr -cr /Applications/VibeButler.app                          # 去掉下载隔离标记（最小修复）
codesign --force --deep --sign - /Applications/VibeButler.app   # 再补一次 ad-hoc 签名（更稳）
```

## 验证清单

```bash
# 清单能被读到且是合法 JSON
curl -s https://raw.githubusercontent.com/hellojerry001/Port-build/main/release.json | python3 -m json.tool

# 安装包可下载且大小与 dmg_size 一致
curl -sI https://raw.githubusercontent.com/hellojerry001/Port-build/main/dist/VibeButler_${V}_aarch64.dmg
```

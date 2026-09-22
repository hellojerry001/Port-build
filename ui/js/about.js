/* =============================================================================
   about.js · 关于页与版本更新
   -----------------------------------------------------------------------------
   更新的整条链路：**前端 fetch 清单 → 后端解析比对 → 后端 curl 下载 →
   自绘确认 → 后端就地替换应用并重启**。

   为什么清单在前端 fetch：raw.githubusercontent.com 带 `access-control-allow-origin: *`，
   而应用的 CSP 是 null，所以取清单这一步不需要后端背一个 HTTP 客户端。
   但「解析清单 + 比较版本」刻意放后端（core 的 check_manifest 命令）——
   版本号比较是这功能里唯一容易写错的地方，放 Rust 侧才能单测。

   为什么「安装」也放后端（install_update）：浏览器下载的包会被打上
   com.apple.quarantine，未公证的应用首次打开必须去「系统设置 › 隐私与安全性」
   放行；而**本应用自己 curl 下来的 dmg 没有这个标记**，装进去的新版本点一下就能
   直接运行 —— 这是把「下载 → 拖拽 → 放行」压成一次点击的关键。
   ============================================================================= */

const AUTO_KEY = "pb-auto-update";

/* 状态集中在这里，渲染一律由 renderAbout() 从状态推导，不做增量改 DOM */
let abInfo = null;     // app_info 的结果
let abCheck = null;    // check_manifest 的结果，null = 还没查过
let abBusy = false;    // 正在检查
let abErr = "";        // 手动检查的失败原因（红字）
let abQuietFail = "";  // 静默检查失败的提示（灰字）——两者必须分开，
                       // 否则每次断网启动都会在页面上留一行红字
let abDl = null;       // download_status 的最新快照
let abTimer = null;    // 下载进度轮询
let abIns = null;      // 安装弹窗状态 { busy, err }；null = 还没在装

const autoCheckOn = () => {
  try { return localStorage.getItem(AUTO_KEY) !== "0"; } catch (e) { return true; }
};
const setAutoCheck = v => {
  try { localStorage.setItem(AUTO_KEY, v ? "1" : "0"); } catch (e) { /* 隐私模式禁写，忽略 */ }
};

/* 侧栏「关于」上的提示圆点：有新版本才亮 */
function markAboutDot(on) {
  const b = document.querySelector('.rail-btn[data-page="about"]');
  if (b) b.classList.toggle("has-dot", !!on);
}

function fmtSize(n) {
  if (!n) return "";
  const mb = n / 1048576;
  return mb >= 1 ? mb.toFixed(1) + " MB" : Math.round(n / 1024) + " KB";
}

/* ============================== 渲染 ============================== */

function renderAbout() {
  if (!abInfo) return;

  const icon = $("abIcon");
  icon.style.backgroundImage = abInfo.icon ? 'url("data:image/png;base64,' + abInfo.icon + '")' : "";
  icon.classList.toggle("is-empty", !abInfo.icon);

  $("abName").textContent = abInfo.name;
  $("abVer").textContent = abInfo.version;
  $("abRepo").textContent = abInfo.repo;

  const sw = $("abAuto");
  const on = autoCheckOn();
  sw.classList.toggle("is-on", on);
  sw.setAttribute("aria-checked", on ? "true" : "false");

  /* --- 检查区：按钮状态 + 一行结果 --- */
  const btn = $("abCheck");
  btn.disabled = abBusy;
  btn.innerHTML = abBusy ? UI.spinner("检查中") : esc("检查更新");

  const note = $("abNote");
  note.classList.remove("is-bad");
  if (abErr) {
    note.textContent = abErr;
    note.classList.add("is-bad");
  } else if (abCheck && abCheck.hasUpdate) {
    note.textContent = "有新版本 " + abCheck.latest;
  } else if (abCheck) {
    note.textContent = "已是最新";
  } else if (abQuietFail) {
    note.textContent = abQuietFail;   // 灰字，不加 is-bad
  } else {
    note.textContent = "";
  }

  /* --- 新版本卡片 --- */
  const box = $("abUpdate");
  const show = !!(abCheck && abCheck.hasUpdate);
  box.hidden = !show;
  if (show) {
    $("abLatest").textContent = abCheck.latest;
    $("abDate").textContent = abCheck.date || "";
    $("abDate").hidden = !abCheck.date;
    $("abMeta").textContent = [
      abCheck.dmgSize ? fmtSize(abCheck.dmgSize) : "",
      abCheck.minMacos ? "需要 macOS " + abCheck.minMacos : "",
    ].filter(Boolean).join(" · ");

    $("abNotes").innerHTML = abCheck.notes.length
      ? abCheck.notes.map(n => "<li>" + esc(n) + "</li>").join("")
      : '<li class="is-muted">这一版没有写更新说明</li>';

    /* 下载区三态：未开始 / 进行中 / 已完成或失败 */
    const dl = abDl;
    const running = !!(dl && dl.running);
    const done = !!(dl && dl.done);
    const failed = !!(dl && dl.error && !dl.running);
    // 清单里没给安装包地址时**不出「下载并安装」**：点了只会弹一句「没有安装包地址」。
    // 真会撞上：发布方先推了 version、安装包几分钟后才补上，而 raw CDN 有
    // cache-control: max-age=300 且**忽略查询串**（`?t=` 与不带参数命中的是同一个
    // 缓存对象，实测 source-age 完全同步），所以刚发完版就点检查，拿到的很可能是
    // 那份「有版本号、没地址」的旧清单。此处改为一行说明，过几分钟重查即可。
    const noPkg = !abCheck.dmgUrl;
    $("abDl").hidden = running || done || noPkg;
    $("abNoPkg").hidden = !noPkg || running || done;
    $("abCancel").hidden = !running;
    // 下完 → 交给「安装并重启」（点开自绘确认弹窗）；不装也可以去访达里自己拿
    $("abInstall").hidden = !done;
    $("abReveal").hidden = !done;
    $("abProg").hidden = !running && !failed;

    const txt = $("abProgText");
    txt.classList.toggle("is-bad", failed);
    if (running) {
      const got = dl.got || 0;
      const total = dl.total || 0;
      $("abProgBar").style.width = total
        ? Math.min(100, (got / total) * 100) + "%"
        : "0%";
      txt.textContent = total
        ? "已下载 " + fmtSize(got) + " / " + fmtSize(total)
        : "已下载 " + fmtSize(got);
    } else if (failed) {
      $("abProgBar").style.width = "0%";
      txt.textContent = dl.error;
    }
  }
}

/* ============================== 数据 ============================== */

async function loadAbout() {
  if (abInfo) return abInfo;
  try {
    abInfo = await invoke("app_info");
    renderAbout();
  } catch (e) {
    toast("读取应用信息失败：" + e);
  }
  return abInfo;
}

/// manual=false 是启动时的静默检查：失败不打扰用户，只把原因留在页面上
async function checkUpdate(manual) {
  if (abBusy || !abInfo) return;
  abBusy = true;
  abErr = "";
  abQuietFail = "";
  renderAbout();

  try {
    // `?t=` 只解决**本机 webview 自己的 HTTP 缓存**（它按完整 URL 做键，加了参数
    // 就一定 miss）。但别指望它绕开 CDN —— 实测 raw.githubusercontent 的 CDN
    // **忽略查询串**：三个不同 `?t=` 拿到的 ETag 完全相同，所以它仍可能返回最多
    // 5 分钟（响应头 cache-control: max-age=300）前的旧清单。
    // 结论：刚发完版就点「检查更新」可能报「已是最新」，等几分钟再点即可。
    const r = await fetch(abInfo.manifestUrl + "?t=" + Date.now(), { cache: "no-store" });
    if (!r.ok) throw new Error("HTTP " + r.status);
    const text = await r.text();
    abCheck = await invoke("check_manifest", { text: text, current: abInfo.version });
    abQuietFail = "";
    markAboutDot(abCheck.hasUpdate);
    if (manual) toast(abCheck.hasUpdate ? "发现新版本 " + abCheck.latest : "已是最新版本");
  } catch (e) {
    abCheck = null;
    const why = e && e.message ? e.message : String(e);
    // 静默检查失败**不写红字**：启动时断网 / 走代理是常态，页面上平白多一行
    // 红色报错只会吓人。但也不能什么都不说 —— 否则用户会把「没查成」读成
    // 「已是最新」。所以分成两档：手动检查给红字 + toast，静默失败只留一句
    // 灰字，并且不 toast、不亮圆点。
    abErr = manual ? "检查更新失败：" + why : "";
    abQuietFail = manual ? "" : "上次自动检查未成功，可手动重试";
    markAboutDot(false);
    if (manual) toast(abErr);
  }

  abBusy = false;
  renderAbout();
}

async function startDownload() {
  if (!abCheck || !abCheck.dmgUrl) {
    toast("这一版没有提供安装包地址，请到发布页手动下载");
    return;
  }
  try {
    await invoke("download_dmg", {
      url: abCheck.dmgUrl,
      name: abCheck.dmgName || abInfo.name + "_" + abCheck.latest + ".dmg",
    });
  } catch (e) {
    return toast(String(e));
  }
  abDl = null;
  if (!abTimer) abTimer = setInterval(tickDownload, 800);
  tickDownload();
}

async function tickDownload() {
  let s;
  try {
    s = await invoke("download_status");
  } catch (e) {
    stopPolling();
    return toast("读取下载进度失败：" + e);
  }
  abDl = s;
  renderAbout();

  if (s.running) return;
  stopPolling();
  if (s.done) {
    // 不再自动用「预览」挂载 dmg 让用户自己拖 —— 直接进安装确认弹窗：
    // 一次点击 = 替换 + 重启。想自己动手的，弹窗里还有「在访达中显示」。
    openInstallModal();
  } else if (s.error) {
    toast(s.error);
  }
}

function stopPolling() {
  if (abTimer) {
    clearInterval(abTimer);
    abTimer = null;
  }
}

async function cancelDownload() {
  try {
    await invoke("cancel_download");
  } catch (e) {
    toast(String(e));
  }
  stopPolling();
  abDl = null;
  renderAbout();
}

/* ============================== 安装（自绘二次确认） ==============================
   为什么不用 window.confirm：macOS 上的 WKWebView 没有实现它，会静默返回 false，
   于是「点了没反应」。和应用里其它破坏性操作一样走自绘弹窗；
   失败不关弹窗、按钮变「重试」、原因写在原地。

   为什么安装要交给后端：浏览器下载的 dmg 会被打上 com.apple.quarantine，
   未公证的应用首次打开必须去「系统设置 › 隐私与安全性」放行；后端用 curl 下的
   包没有这个标记，替换进去的新版本可以直接打开 —— 这才是「一次点击完成升级」。 */

function renderInstallModal() {
  const busy = !!(abIns && abIns.busy);
  const err = (abIns && abIns.err) || "";
  const cur = (abInfo && abInfo.version) || "";
  const next = (abCheck && abCheck.latest) || "";

  $("aiWho").textContent =
    (abInfo ? abInfo.name : "应用") + (cur ? " " + cur : "") + (next ? " → " + next : "");
  $("aiWhat").textContent = "安装包：" + ((abDl && abDl.path) || "");
  $("aiTip").textContent = err ||
    "安装时应用会自动退出并重新打开；项目、端口与设置都不受影响。";
  $("aiTip").classList.toggle("is-bad", !!err);

  $("aiBtn").disabled = busy;
  $("aiCancel").disabled = busy;
  $("aiBtn").textContent = busy ? "正在安装…" : err ? "重试" : "立即安装并重启";
}

/// 打开确认弹窗；安装包没下完就不给开
function openInstallModal() {
  if (!abDl || !abDl.done || !abDl.path) return;
  abIns = { busy: false, err: "" };
  renderInstallModal();
  openModal("abInstallModal");
}

function closeInstallModal() {
  if (abIns && abIns.busy) return;   // 装到一半不让关，免得用户以为没在装
  closeModal("abInstallModal");
  abIns = null;
}

async function confirmInstall() {
  if (!abDl || !abDl.done || (abIns && abIns.busy)) return;
  abIns = { busy: true, err: "" };
  renderInstallModal();

  try {
    // restart=true：后端装好后自己退出、1 秒后重新打开新版本。
    // 正常路径下这个 await 不会返回（进程已经退了），所以后面不写收尾逻辑。
    await invoke("install_update", { path: abDl.path, restart: true });
    toast("已安装，应用正在重新打开");
  } catch (e) {
    abIns = { busy: false, err: String(e) };
    renderInstallModal();
  }
}

/// 进入关于页：首次进来补数据，顺手把提示圆点消掉
async function enterAbout() {
  await loadAbout();
  markAboutDot(false);
}

/* ============================== 交互 ============================== */

on("about-check", () => checkUpdate(true));

on("about-auto", () => {
  const next = !autoCheckOn();
  setAutoCheck(next);
  renderAbout();
  toast(next ? "启动时会自动检查更新" : "已关闭自动检查");
  // 刚打开开关就顺手查一次，免得要等下次启动
  if (next && !abCheck) checkUpdate(false);
});

on("about-download", () => startDownload());
on("about-cancel", () => cancelDownload());
on("about-install", () => openInstallModal());
on("ab-install-close", () => closeInstallModal());
on("ab-install-confirm", () => confirmInstall());
on("ab-install-reveal", () => {
  if (abDl && abDl.path) showInFinder(abDl.path);
});

on("about-reveal", () => {
  if (abDl && abDl.path) showInFinder(abDl.path);
});

on("about-issues", async () => {
  if (!abInfo) return;
  await invoke("open_url", { url: abInfo.issuesUrl });
});

on("about-repo", async () => {
  if (!abInfo) return;
  await invoke("open_url", { url: "https://github.com/" + abInfo.repo });
});

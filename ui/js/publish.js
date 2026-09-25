/* =============================================================================
   publish.js · 发布站点（Cloudflare 临时链接 / GitHub Pages）+ 发布记录
   -----------------------------------------------------------------------------
   托管方式默认取设置页的偏好，也可以在弹窗里**只对这一次**改（pubHosting）。
   发布记录持久化在 ~/.portbutler/publishes.json（后端 publishes.rs）。
   列表要同时渲染到两处：弹窗 #plList 与页面 #plListPage，故渲染函数接收容器。
   ============================================================================= */

let pubDone = false;    // 发布成功后，弹窗底部按钮的语义从「发布」变为「关闭」
let pubHosting = "";    // 本次发布用的托管方式（空 = 还没读到偏好）
let pubGh = null;       // gh_publish_preview 的结果：目标仓库、分支、预测地址

let pubRepoDirty = false;  // 用户改过「目标仓库」输入框 → 不再用预览结果回填
let pubUserSite = false;   // GitHub 托管下的发布目标：false=项目仓库（子路径） true=站点根（域名根）
let pubRootArmed = false, pubRootTimer = 0;   // 站点根的「再点一次」二次确认
let pubScan = null;        // scan_asset_paths 的结果：产物里的根绝对引用数

// 状态汇总（#pubSum）里要显示的三类信息，全部集中一处
let pubRunning = false;                 // 发布中：汇总块切成「进度条 + 逐条动画的步骤」
let pubStepEls = [];                    // 发布中的步骤行，收尾时统一改勾
let pubDest = "", pubDestBad = false;   // 目的地一行（将发布到哪儿）
let pubDirMsg = "", pubDirBad = false;  // 部署目录的状态（✗ 不存在等；正常时给一句短的）
let pubWarnText = "";                   // 产物路径警告（发到子路径会丢样式）
let pubScanSeq = 0, pubScanTimer = 0;
let pubRepoSeq = 0;

/* 托管方式的展示文案。key 与后端 git.rs 的 HOSTING_* 一一对应 */
const PUB_HOSTINGS = [
  { key: "cloudflare", label: "Cloudflare 临时链接", note: "匿名临时链接，默认 60 分钟自动失效" },
  { key: "github",     label: "GitHub Pages",         note: "推送到仓库产物分支，长期由 GitHub Pages 提供访问" },
];

/* GitHub 托管下的「发布目标」。
   project = 项目仓库，站点在 `<owner>.github.io/<repo>/` 子路径下（默认、也最常用）；
   root    = 账号的站点根，站点就在域名根，产物里的根绝对路径天然成立 ——
             任何静态站都不用配 basePath，但一个账号只有这一个根，再发会静默覆盖。 */
const PUB_TARGETS = [
  { key: "project", label: "项目仓库 · 子路径", note: "站点落在 <owner>.github.io/<仓库>/ 下" },
  { key: "root",    label: "我的站点根 · 域名根", note: "站点就在域名根，产物不用配 basePath" },
];

/* ============================== 发布弹窗 ============================== */

/* 弹窗两个状态：配置（选托管/目标/目录）与发布中（转圈 + 步骤日志）。
   与「打包 DMG」的 bdCfg / bdRun 同一套交互。 */
function showPubCfg() {
  pubRunning = false;                 // 回到配置态：汇总块从「进度」复位成「目的地 / 状态」
  pubStepEls = [];
  if ($("pubCfg")) $("pubCfg").hidden = false;
  if ($("pubRun")) $("pubRun").hidden = true;
  const sum = $("pubSum");
  if (sum) sum.classList.remove("is-run");
  renderPubSum();
}
function showPubRun() {
  if ($("pubCfg")) $("pubCfg").hidden = true;
  if ($("pubRun")) $("pubRun").hidden = false;
}

function openPublish(id) {
  const p = projects.find(x => x.id === id);
  if (!p) return;

  pubId = id;
  pubProbe = null;
  pubDone = false;
  pubHosting = "";
  pubGh = null;
  pubRepoDirty = false;
  pubUserSite = false;
  pubRootArmed = false;
  clearTimeout(pubRootTimer);
  pubScan = null;
  showPubCfg();
  if ($("pubRunStatus")) $("pubRunStatus").textContent = "";
  pubRunning = false;
  pubStepEls = [];
  $("pubName").textContent = p.name;
  $("pubPath").value = "";
  if ($("pubPathBox")) {
    $("pubPathBox").textContent = "尚未选择构建产物目录";
    $("pubPathBox").title = "";
  }
  if ($("pubDirPill")) {
    $("pubDirPill").className = "status-pill";
    $("pubDirPill").innerHTML = '<span class="dot"></span>正在探测…';
  }
  $("pubStatus").innerHTML = "";
  if ($("pubRepo")) $("pubRepo").value = "";
  updatePathWarn();
  $("pubBtn").disabled = false;
  $("pubBtn").textContent = "发布";
  renderPubHost();
  openModal("pubModal");
  loadPubHosting(p);
  probeDists();   // 打开即自动探测构建产物并回填
}

/* 读托管偏好与目标仓库。两项都不阻塞弹窗打开 —— 先渲染默认态，回来了再刷新 */
async function loadPubHosting(p) {
  try {
    pubGh = await invoke("gh_publish_preview", {
      projectPath: p.path, projectName: p.name, repoName: null, userSite: pubUserSite,
    });
    if (!pubHosting) pubHosting = pubGh.hosting || "cloudflare";
  } catch (_) {
    // 预览失败不影响发布：后端发布时会自己算一遍
    if (!pubHosting) pubHosting = "cloudflare";
  }
  // 首次把后端算出的默认仓库名填进输入框；用户改过就不再动他的输入
  const repoEl = $("pubRepo");
  if (repoEl && !pubRepoDirty && pubGh) repoEl.value = pubGh.repo || "";
  renderPubHost();
}

function renderPubHost() {
  const cur = pubHosting || "cloudflare";
  const box = $("pubHostSeg");
  if (!box) return;

  // 「目标仓库」只在 GitHub 托管下才有意义
  const tgt = $("pubGhTarget");
  if (tgt) tgt.hidden = cur !== "github";
  renderPubTarget();

  box.innerHTML = PUB_HOSTINGS.map(h =>
    '<button class="' + cls("seg-btn", h.key === cur && "is-on") + '" role="radio"' +
    ' aria-checked="' + (h.key === cur ? "true" : "false") + '"' +
    dataAttrs({ act: "pub-host-pick", key: h.key }) + ">" + esc(h.label) + "</button>").join("");

  const tip = $("pubFootTip");
  if (cur === "github") {
    // 目的地一行进「状态汇总」；长说明只在底部留一行
    pubDest = (pubGh && pubGh.url) ? "将发布到 " + pubGh.url : "";
    pubDestBad = !!pubGh && !pubGh.url;
    if (!pubDest) pubDest = pubGh ? pubGh.hint : "长期托管：产物会推到仓库的产物分支，由 GitHub Pages 提供访问";
    if (tip) {
      tip.innerHTML = "免费账号仅支持 <b>public</b> 仓库；首次开启 Pages 约 1 分钟构建，期间访问可能 404。";
    }
  } else {
    pubDest = "将部署为临时链接，约 60 分钟失效；可在结果里「认领」转永久";
    pubDestBad = false;
    if (tip) {
      tip.innerHTML = "限制：单文件 ≤25MiB、≤1000 个文件、仅静态（无后端）。";
    }
  }
  renderPubSum();   // 目标地址会随托管方式/仓库名变，汇总跟着重算
}

/* 发布预览：虚线框，集中显示「目的地 / 目录状态 / 路径警告」；
   发布中就地切成「进度条 + 逐条动画的步骤」。 */
function renderPubSum() {
  const el = $("pubSum");
  if (!el) return;
  if (pubRunning) { renderPubRunSum(); return; }

  const has = pubDest || pubDirMsg || pubWarnText;
  el.hidden = !has;
  if (!has) { el.classList.remove("is-run"); return; }

  const urlHtml = pubDest
    ? '<div class="preview-url' + (pubDestBad ? " is-bad" : "") + '">' + esc(pubDest) + "</div>"
    : "";
  const checkHtml = pubDirMsg
    ? '<div class="preview-check' + (pubDirBad ? " is-bad" : " is-ok") + '">' + esc(pubDirMsg) + "</div>"
    : "";
  const warnHtml = pubWarnText
    ? '<div class="preview-warn">' + esc(pubWarnText) + "</div>"
    : "";

  el.classList.remove("is-run");
  el.innerHTML = urlHtml + checkHtml + warnHtml;
}

/* 发布态：预览框切换成进度展示区 */
function renderPubRunSum() {
  const el = $("pubSum");
  if (!el) return;
  const steps = pubStepsOf(pubHosting === "github");
  el.classList.add("is-run");
  el.hidden = false;
  el.innerHTML =
    (pubDest
      ? '<div class="preview-url' + (pubDestBad ? " is-bad" : "") + '">' + esc(pubDest) + "</div>"
      : "") +
    '<div class="pb-bar" id="pubBar"></div>' +
    '<div class="pb-steps">' + steps.map((s, i) =>
      '<div class="pb-step" style="--i:' + i + '">' +
        "<b>" + (i + 1) + "</b><span>" + esc(s) + "</span>" +
      "</div>").join("") + "</div>";
  pubStepEls = Array.prototype.slice.call(el.querySelectorAll(".pb-step"));
}

/* 收尾：进度条停在满/红，步骤全打勾。ok=false 时只停成红色，不pretend成功 */
function pubFinish(ok) {
  pubRunning = false;
  const bar = $("pubBar");
  if (bar) bar.classList.add(ok ? "is-done" : "is-fail");
  if (ok) {
    pubStepEls.forEach(n => {
      n.classList.add("is-done");
      const b = n.querySelector("b");
      if (b) b.textContent = "✔";
    });
  }
}

/* 步骤之后追加一行结果（完成 / 失败原因） */
function pubAppendStep(text, bad) {
  const el = $("pubSum");
  if (!el) return;
  el.insertAdjacentHTML("beforeend",
    '<div class="pb-step' + (bad ? " is-bad" : " is-done") + '" style="--i:0">' +
      "<b>" + (bad ? "✗" : "✔") + "</b><span>" + esc(text) + "</span></div>");
}

function pubStepsOf(isGh) {
  return isGh
    ? ["校验产物目录", "确定目标仓库", "推送产物（强制提交并补 .nojekyll）", "开启 GitHub Pages"]
    : ["校验产物目录", "下载 / 复用 wrangler", "上传到 Cloudflare 边缘网络"];
}

/* GitHub 托管下的「发布目标」二选一：
   - 项目仓库（子路径）：可改名（见 E）
   - 我的站点根（域名根）：仓库固定为 <owner>.github.io，站点落在域名根 ——
     产物里的根绝对路径天然成立，任何静态站都不用配 basePath；但一个账号只有这一个根，
     所以这里隐藏仓库名输入、给出覆盖警告，发布时还要「再点一次」确认。 */
function renderPubTarget() {
  const box = $("pubTargetCards");
  if (!box) return;
  const on = pubUserSite ? "root" : "project";
  box.innerHTML = PUB_TARGETS.map(t =>
    '<button class="' + cls("target-card", t.key === on && "is-on") + '" role="radio"' +
    ' aria-checked="' + (t.key === on ? "true" : "false") + '"' +
    dataAttrs({ act: "pub-target-pick", key: t.key }) + ">" +
      '<span class="target-dot"></span>' +
      '<span class="target-body">' +
        '<span class="target-title">' + esc(t.label) + "</span>" +
        '<span class="target-note">' + esc(t.note) + "</span>" +
      "</span>" +
    "</button>").join("");

  const row = $("pubRepoRow");
  if (row) row.hidden = pubUserSite;
  const warn = $("pubRootWarn");
  if (warn) warn.hidden = !pubUserSite;
}

on("pub-target-pick", el => {
  const want = el.dataset.key === "root";
  if (want === pubUserSite) return;
  pubUserSite = want;
  pubRootArmed = false;                 // 换了目标，之前的确认作废
  clearTimeout(pubRootTimer);
  renderPubTarget();
  refreshGhPreview();                   // 地址与提示都会跟着变
});

/* 用户改了「目标仓库」→ 重新取一次预览（地址、是否新建都会变）。
   预览不打网络（账号名取本地记录），所以可以边输边刷。 */
async function refreshGhPreview() {
  const p = projects.find(x => x.id === pubId);
  if (!p) return;
  const seq = ++pubRepoSeq;
  const typed = $("pubRepo") ? $("pubRepo").value.trim() : "";
  try {
    const r = await invoke("gh_publish_preview", {
      projectPath: p.path, projectName: p.name, repoName: typed || null, userSite: pubUserSite,
    });
    if (seq !== pubRepoSeq) return;   // 期间又改了，丢弃这次
    pubGh = r;
  } catch (_) {
    if (seq !== pubRepoSeq) return;
  }
  renderPubHost();
}

function closePublish() { closeModal("pubModal"); }

/* 探测项目的构建产物目录：自动选中第一个可用的，并把全部候选列成 chips */
async function probeDists() {
  const p = projects.find(x => x.id === pubId);
  if (!p) return;

  setPathHint("正在探测构建产物…");
  if ($("pubDirPill")) {
    $("pubDirPill").className = "status-pill";
    $("pubDirPill").innerHTML = '<span class="dot"></span>正在探测…';
  }

  let r = null;
  try { r = await invoke("publish_probe", { projectPath: p.path }); } catch (e) { /* 探测失败按无候选处理 */ }
  pubProbe = r;
  if (!r) { setPathHint(""); return; }

  if ((r.candidates || []).length) $("pubPath").value = r.candidates[0].path;
  refreshPubHint();
  syncDirView();
}

/* 目录卡片：把隐藏 input 的值同步到可视路径 + 状态胶囊 */
function syncDirView() {
  const box = $("pubPathBox");
  const pill = $("pubDirPill");
  const input = $("pubPath");
  if (!box || !pill || !input) return;
  const v = input.value.trim();
  box.textContent = v || "尚未选择构建产物目录";
  box.title = v || "";

  const cands = (pubProbe && pubProbe.candidates) || [];
  const hit = cands.find(c => c.path === v);
  if (hit) {
    pill.className = "status-pill is-ok";
    pill.innerHTML = '<span class="dot"></span>' + esc(hit.rel) +
      " · " + hit.files + " 文件 · " + hit.sizeMb + "MB";
  } else if (v) {
    pill.className = "status-pill " + (pubDirBad ? "is-bad" : "is-ok");
    pill.innerHTML = '<span class="dot"></span>' + (pubDirBad ? "目录不存在" : "自定义目录");
  } else {
    pill.className = "status-pill";
    pill.innerHTML = '<span class="dot"></span>未探测';
  }
}

/* 目录说明行：variant 为 bad / good，缺省是中性说明。
   正常时只显示一句短的 —— dist chip 上的 ✓ 已经表达了「存在」。 */
function setPathHint(text, variant) {
  pubDirMsg = text || "";
  pubDirBad = variant === "bad";
  renderPubSum();
}

/* 输入框变化 / 选 chip 后刷新：目录是否存在 + 探测结论 */
let checkSeq = 0;
async function refreshPubHint() {
  const v = $("pubPath").value.trim();

  if (!v) {
    setPathHint(pubProbe ? pubProbe.hint : "");
    scheduleScan("");
    return;
  }

  const seq = ++checkSeq;
  let ok = false;
  try { ok = await invoke("check_dir", { path: v }); } catch (e) { ok = false; }
  if (seq !== checkSeq) return;   // 期间又有新输入，丢弃这次结果

  if (ok) {
    setPathHint("✓ 目录可用", "good");
    scheduleScan(v);              // 顺带扫一遍产物里的资源路径
  } else {
    let msg = "✗ 这个目录不存在 —— 需要先构建出静态产物再发布。";
    if (pubProbe && pubProbe.buildCmd) msg += " 构建命令：" + pubProbe.buildCmd.split("→")[0].trim();
    setPathHint(msg, "bad");
    scheduleScan("");
  }
  syncDirView();
}

/* ============================== 产物路径自检（发布前） ==============================
   发布到 GitHub Pages **项目页**时，站点挂在 `/<repo>/` 子路径下，而多数构建工具
   默认按**站点根**产出（`/_next/...`、`/assets/...`）。这些根绝对路径会打到域名根
   → 样式全丢、点任何链接 404。这里在发布前扫一遍，把问题提前暴露出来。 */
function scheduleScan(v) {
  clearTimeout(pubScanTimer);
  pubScan = null;
  updatePathWarn();
  if (!v) return;
  pubScanTimer = setTimeout(async () => {
    const seq = ++pubScanSeq;
    let r = null;
    try { r = await invoke("scan_asset_paths", { distPath: v }); } catch (_) { r = null; }
    if (seq !== pubScanSeq) return;   // 又换了目录，丢弃这次
    pubScan = r;
    updatePathWarn();
  }, 400);
}

/* 只有「确实会发到子路径」且「产物里真有根绝对引用」才提示 —— 发到站点根是没问题的。 */
function updatePathWarn() {
  const hosting = pubHosting || "cloudflare";
  let subPath = true;                       // 地址未知时按「多半是项目页」保守处理
  if (pubGh && pubGh.url) {
    try { subPath = new URL(pubGh.url).pathname !== "/"; } catch (_) { subPath = true; }
  }
  pubWarnText = (hosting === "github" && subPath && !!pubScan && pubScan.hits > 0)
    ? "⚠️ 产物里有 " + pubScan.hits + " 处根绝对路径（如 " +
      (pubScan.samples || []).slice(0, 2).join("、") +
      "），发到子路径会丢样式 —— 请配 basePath 重新构建，或改发到站点根。"
    : "";
  renderPubSum();
}

async function doPublish() {
  const dist = $("pubPath").value.trim();
  if (!dist) return toast("请填写部署目录");

  const proj = projects.find(x => x.id === pubId);
  const hosting = pubHosting || "cloudflare";
  const isGh = hosting === "github";

  // 发到「站点根」是破坏性操作（会静默覆盖根上现有的内容）→ 二次确认：
  // 第一次点只是把按钮变成确认文案，3 秒内再点一次才真的发。
  if (isGh && pubUserSite && !pubRootArmed) {
    pubRootArmed = true;
    const btn = $("pubBtn");
    btn.textContent = "再点一次：会覆盖站点根";
    clearTimeout(pubRootTimer);
    pubRootTimer = setTimeout(() => {
      pubRootArmed = false;
      btn.textContent = "发布";
    }, 3000);
    $("pubStatus").innerHTML =
      '<div class="pub-loading">将发布到站点根 —— 根上现有的内容会被替换。确认请再点一次。</div>';
    return;
  }
  pubRootArmed = false;      // 这一下是真的发，把确认态消费掉
  clearTimeout(pubRootTimer);

  $("pubBtn").disabled = true;
  $("pubBtn").textContent = "发布中…";
  $("pubStatus").innerHTML = "";

  // 切到发布态：状态行转圈，步骤在汇总块里逐条淡入（见 renderPubRunSum）
  pubRunning = true;
  showPubRun();
  const runSt = $("pubRunStatus");
  if (runSt) {
    runSt.className = "build-status";
    runSt.innerHTML = UI.spinner(isGh
      ? "正在发布到 GitHub Pages…"
      : "正在上传到 Cloudflare（首次约需 1–2 分钟下载 wrangler）…");
  }
  renderPubSum();

  try {
    const r = await invoke("publish_project", {
      distPath: dist,
      projectId: proj ? proj.id : "",
      projectName: proj ? proj.name : "",
      projectPath: proj ? proj.path : "",
      hosting,                                  // 本次选的方式（空则后端读设置）
      branch: pubGh ? pubGh.branch : "",
      // 目标仓库：留空则后端按默认规则命名（项目远端 > <前缀><项目名>）
      repoName: ($("pubRepo") ? $("pubRepo").value.trim() : "") || null,
      // true = 发到账号的站点根 <owner>.github.io（地址就是域名根）
      userSite: !!pubUserSite,
    });

    if (r.ok && r.url) {
      if (runSt) { runSt.className = "build-status is-ok"; runSt.textContent = "发布完成"; }
      pubFinish(true);
      pubAppendStep("完成", false);
      showPubResult(r);
      refreshPublishes();   // 记录已落盘，同步角标与列表
    } else {
      // 失败要回到配置态，用户才能改目标 / 目录后重试；原因顺带留在汇总块里（红字一行）
      if (runSt) { runSt.className = "build-status is-bad"; runSt.textContent = "发布失败"; }
      showPubCfg();
      pubAppendStep((r.error || "未知错误").split("\n")[0], true);
      const msg = (r.error || "未知错误").split("\n").slice(0, 12).join("\n");
      $("pubStatus").innerHTML = '<div class="pub-err">发布失败：\n' + esc(msg) + "</div>";
      $("pubBtn").disabled = false;
      $("pubBtn").textContent = "重试";
    }
  } catch (e) {
    showPubCfg();
    pubAppendStep(String(e).split("\n")[0], true);
    $("pubStatus").innerHTML = '<div class="pub-err">发布失败：\n' + esc(String(e)) + "</div>";
    $("pubBtn").disabled = false;
    $("pubBtn").textContent = "重试";
  }
}

/* 结果区：URL /（Cloudflare 才有）认领链接 + 复制、打开按钮；临时链接带倒计时 */
function showPubResult(r) {
  // GitHub Pages 是长期托管：没有认领链接，也没有失效时间
  const isGh = r.hosting === "github";
  const isTemp = !isGh && !!r.claimUrl;
  // 真实窗口时长由后端从 wrangler 输出解析（复用旧临时账号时会短于 60 分钟）
  const expiry = r.expiresAt || (Date.now() + 60 * 60 * 1000);
  const mins = Math.max(1, Math.round((expiry - Date.now()) / 60000));

  let html;
  if (isGh) {
    html = '<div class="pub-ok">✅ 已发布到 GitHub Pages，长期有效</div>';
  } else if (isTemp) {
    html = '<div class="pub-ok">✅ 发布成功，临时链接约 ' + mins + " 分钟后失效</div>";
  } else {
    html = '<div class="pub-ok">✅ 发布成功，已部署到你的 Cloudflare 账号（长期有效）</div>';
  }

  html += urlField(isGh ? "站点地址（任何人可访问）" : "Website URL（任何人可访问）",
    "pubUrl", r.url,
    UI.btn("复制", { act: "copy-input", data: { target: "pubUrl" } }),
    UI.btn("打开", { variant: "ghost", act: "open-url", data: { url: r.url } }));

  if (isGh) {
    if (r.note) {
      // 目标仓库 / 分支 / Pages 开启结果；Pages 没开成也在这里说明怎么手动开
      html += '<div class="pub-tip">' + esc(r.note).replace(/\n/g, "<br>") + "</div>";
    }
  } else if (isTemp) {
    html += urlField("认领链接（窗口期内登录 Cloudflare 可转永久）", "pubClaim", r.claimUrl,
      UI.btn("复制", { act: "copy-input", data: { target: "pubClaim" } }),
      UI.btn("认领", { variant: "ghost", act: "open-url", data: { url: r.claimUrl } }));
    html += '<div class="pub-count" id="pubCount"></div>';
  } else {
    html += '<div class="pub-tip">部署在你的 Cloudflare 账号下，长期有效。' +
            "不再需要时可到 Cloudflare 控制台删除对应 Worker。</div>";
  }

  $("pubStatus").innerHTML = html;
  $("pubBtn").textContent = "关闭";
  pubDone = true;

  if (!isTemp) return;
  const tick = () => {
    const el = $("pubCount");
    if (!el) return;   // 弹窗已关闭，停表
    const ms = expiry - Date.now();
    if (ms <= 0) { el.textContent = "链接已过期，请重新发布"; return; }
    const m = Math.floor(ms / 60000);
    const s = Math.floor((ms % 60000) / 1000);
    el.textContent = "剩余 " + m + " 分 " + s + " 秒 · 已存入「发布记录」";
    setTimeout(tick, 1000);
  };
  tick();
}

/* 只读链接行：等宽输入框 + 尾部动作按钮 */
function urlField(label, inputId, value, ...buttons) {
  return '<div class="field"><label class="field-label">' + esc(label) + "</label>" +
    '<div class="input-row">' +
      '<input class="input" id="' + esc(inputId) + '" readonly value="' + esc(value) + '">' +
      buttons.join("") +
    "</div></div>";
}

/* ============================== 发布记录 ============================== */

async function refreshPublishes() {
  try {
    publishes = await invoke("list_publishes");
  } catch (e) {
    return;
  }
  $("pubCnt").textContent = publishes.length ? String(publishes.length) : "";
  renderPublishes();   // 弹窗与页面两个容器一起刷新
}

function openPublishList() {
  openModal("pubListModal");
  $("plList").innerHTML = '<div class="pl-empty">读取中…</div>';
  refreshPublishes();
}

function closePublishList() { closeModal("pubListModal"); }

/* 记录状态 → 徽标文案与修饰。dead | warn | perm | "" */
function pubBadgeState(r, now) {
  if (!r.expiresAt) return { text: "长期有效", variant: "perm" };
  const left = r.expiresAt - now;
  if (left <= 0) return { text: "已过期", variant: "dead" };
  if (left < 10 * 60000) return { text: "剩余 " + Math.ceil(left / 60000) + " 分钟", variant: "warn" };
  return { text: "剩余 " + Math.round(left / 60000) + " 分钟", variant: "" };
}

/* 托管来源短名。老记录没有 hosting 字段，按 cloudflare 显示（后端也是这么兜的） */
function pubSourceName(r) {
  return r.hosting === "github" ? "GitHub Pages" : "Cloudflare";
}

function renderPublishes() {
  ["plList", "plListPage"].forEach(id => {
    const box = $(id);
    if (box) renderPubInto(box);
  });
  renderPubCount();
}

/* 统计标题：共 X 个记录（如有已过期，补一句） */
function renderPubCount() {
  const now = Date.now();
  const dead = publishes.filter(r => r.expiresAt && r.expiresAt - now <= 0).length;
  const html = "共 <b>" + publishes.length + "</b> 个记录" +
    (dead ? '<span class="sep">·</span><span class="dim">' + dead + " 个已过期</span>" : "");
  ["plCountPage", "plCount"].forEach(id => {
    const el = $(id);
    if (el) el.innerHTML = html;
  });
}

function renderPubInto(box) {
  if (!publishes.length) {
    box.innerHTML = UI.emptyState({
      title: "还没有发布记录",
      desc: "在项目卡片点「发布」后会记录在这里，关掉弹窗也找得回。",
      span: true
    });
    return;
  }

  const now = Date.now();
  box.innerHTML = publishes.map(r => {
    const st = pubBadgeState(r, now);
    const dead = st.variant === "dead";

    const ops = [
      UI.btn("复制链接", { act: "pub-copy", data: { v: r.url } }),
      UI.btn("打开", { variant: "ghost", act: "pub-open", data: { v: r.url } }),
    ];
    if (r.claimUrl) {
      ops.push(UI.btn(dead ? "认领（已过期）" : "认领", {
        variant: "ghost", act: "pub-open", data: { v: r.claimUrl },
        title: "窗口期内登录 Cloudflare 可把临时账号转成你自己的",
      }));
    }
    ops.push(UI.btn("移除", {
      variant: "ghost", act: "pub-remove", data: { v: r.id },
      title: "只删本地记录，不影响 Cloudflare 上的部署",
    }));

    return '<div class="' + cls("pl-item", dead && "is-dead") + '">' +
        '<div class="pl-top">' +
          '<span class="pl-proj">' + esc(r.projectName || r.id) + "</span>" +
          '<span class="pl-src">' + esc(pubSourceName(r)) + "</span>" +
          UI.badge(st.text, st.variant) +
        "</div>" +
        '<div class="pl-url">' + esc(r.url) + "</div>" +
        '<div class="pl-meta">' + fmtClock(r.publishedAt) + " · " + fmtAgo(r.publishedAt) +
          (r.distPath ? " · " + esc(r.distPath) : "") + "</div>" +
        '<div class="pl-ops">' + ops.join("") + "</div>" +
      "</div>";
  }).join("");
}

async function removePublish(id) {
  try {
    publishes = await invoke("delete_publish", { id });
  } catch (e) {
    return toast(String(e));
  }
  $("pubCnt").textContent = publishes.length ? String(publishes.length) : "";
  renderPublishes();
  toast("已移除记录");
}

/* 清空是二次确认：第一次点击进入待确认，3 秒内再点一次才真清 */
let plClearArmed = false, plClearTimer = null;
function clearPublishes(btn) {
  if (!publishes.length) return toast("没有可清空的记录");

  if (!plClearArmed) {
    plClearArmed = true;
    btn.textContent = "再点一次确认清空";
    clearTimeout(plClearTimer);
    plClearTimer = setTimeout(() => {
      plClearArmed = false;
      btn.textContent = "清空记录";
    }, 3000);
    return;
  }

  clearTimeout(plClearTimer);
  plClearArmed = false;
  btn.textContent = "清空记录";
  invoke("clear_publishes").then(list => {
    publishes = list;
    $("pubCnt").textContent = "";
    renderPublishes();
    toast("已清空");
  }).catch(e => toast(String(e)));
}

/* ============================== 时间格式 ============================== */

/* 相对时间，让人一眼看出新旧 */
function fmtAgo(ms) {
  const d = Date.now() - ms;
  if (d < 60000) return "刚刚";
  if (d < 3600000) return Math.floor(d / 60000) + " 分钟前";
  if (d < 86400000) return Math.floor(d / 3600000) + " 小时前";
  return Math.floor(d / 86400000) + " 天前";
}

/* 绝对时间（月-日 时:分），便于和 Cloudflare 控制台对照 */
function fmtClock(ms) {
  const d = new Date(ms);
  const p = n => String(n).padStart(2, "0");
  return p(d.getMonth() + 1) + "-" + p(d.getDate()) + " " + p(d.getHours()) + ":" + p(d.getMinutes());
}

/* ============================== 动作注册 ============================== */
on("publish-open",    el => openPublish(el.dataset.id));
on("publish-close",   () => closePublish());
on("publish-history", () => { closePublish(); openPublishList(); });
on("pub-btn",         () => (pubDone ? closePublish() : doPublish()));
on("dist-probe",      () => probeDists());
on("dist-pick",       el => { $("pubPath").value = el.dataset.path; refreshPubHint(); });

/* 本次发布的托管方式：只改 pubHosting，不动设置页里的偏好 */
on("pub-host-pick", el => {
  if (pubDone) return;                 // 已经发完了，别再改
  pubHosting = el.dataset.key || "cloudflare";
  renderPubHost();
});

on("publish-list-open",  () => openPublishList());
on("publish-list-close", () => closePublishList());
on("publish-refresh",    () => refreshPublishes());
on("publish-clear",      el => clearPublishes(el));

on("pub-copy",   el => copyVal(el.dataset.v));
on("pub-open",   el => openLink(el.dataset.v));
on("pub-remove", el => removePublish(el.dataset.v));

/* 发布结果里的通用按钮 */
on("copy-input", el => copyInputValue(el.dataset.target));
on("open-url",   el => openLink(el.dataset.url));

onInput("pub-path", () => refreshPubHint());

/* 目标仓库：用户一改就标记 dirty（不再被预览回填），防抖后重取预览 ——
   仓库名变了，目标地址与「是否新建」都会跟着变。 */
let repoDebounce = 0;
onInput("pub-repo", () => {
  pubRepoDirty = true;
  clearTimeout(repoDebounce);
  repoDebounce = setTimeout(refreshGhPreview, 500);
});

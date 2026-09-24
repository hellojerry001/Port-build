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

/* 托管方式的展示文案。key 与后端 git.rs 的 HOSTING_* 一一对应 */
const PUB_HOSTINGS = [
  { key: "cloudflare", label: "Cloudflare 临时链接" },
  { key: "github",     label: "GitHub Pages" },
];

/* ============================== 发布弹窗 ============================== */

function openPublish(id) {
  const p = projects.find(x => x.id === id);
  if (!p) return;

  pubId = id;
  pubProbe = null;
  pubDone = false;
  pubHosting = "";
  pubGh = null;
  $("pubName").textContent = p.name;
  $("pubPath").value = "";
  $("distChips").innerHTML = "";
  $("pubStatus").innerHTML = "";
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
    pubGh = await invoke("gh_publish_preview", { projectPath: p.path, projectName: p.name });
    if (!pubHosting) pubHosting = pubGh.hosting || "cloudflare";
  } catch (_) {
    // 预览失败不影响发布：后端发布时会自己算一遍
    if (!pubHosting) pubHosting = "cloudflare";
  }
  renderPubHost();
}

function renderPubHost() {
  const cur = pubHosting || "cloudflare";
  const box = $("pubHostChips");
  if (!box) return;

  box.innerHTML = PUB_HOSTINGS.map(h =>
    '<button class="' + cls("chip", h.key === cur && "is-on") + '"' +
    dataAttrs({ act: "pub-host-pick", key: h.key }) + ">" + esc(h.label) + "</button>").join("");

  const hint = $("pubHostHint");
  const tip = $("pubFootTip");
  const title = $("pubTitle");
  if (cur === "github") {
    hint.className = cls("field-hint", pubGh && !pubGh.url && "is-bad");
    // 地址另起一行：和说明文字挤在同一行换行时，中间会空出一大段
    hint.innerHTML = pubGh
      ? esc(pubGh.hint) + (pubGh.url ? "<br>目标地址：" + esc(pubGh.url) : "")
      : "长期托管：产物会推到仓库的产物分支，由 GitHub Pages 提供访问";
    if (title) title.textContent = "发布到线上 · GitHub Pages";
    if (tip) {
      tip.innerHTML = "发布到 GitHub Pages：<b>长期有效</b>，不需要认领。" +
        "免费账号仅支持 public 仓库（没有远端时会新建一个）；" +
        "首次开启 Pages 后约需 1 分钟构建，期间访问可能 404。" +
        "<br>产物会被强制提交（忽略 .gitignore），并自动补一个 <code>.nojekyll</code>，" +
        "否则下划线开头的资源目录（如 Next 的 <code>_next</code>）会被 Jekyll 吃掉。";
    }
  } else {
    hint.className = cls("field-hint", "is-good");
    hint.textContent = pubGh
      ? "匿名临时部署，约 60 分钟后失效；窗口期内可认领为永久"
      : "匿名临时部署，默认 60 分钟自动失效";
    if (title) title.textContent = "发布到线上 · Cloudflare 临时链接";
    if (tip) {
      tip.innerHTML = "部署到 Cloudflare 边缘网络：匿名临时公开链接，<b>默认 60 分钟自动失效</b>" +
        "（以结果里的倒计时为准）；失效前点结果里的「认领」登录 Cloudflare 即可转永久" +
        "（绑定你账号长期有效）。限制：单文件 ≤25MiB、≤1000 个文件、仅静态（无后端）。";
    }
  }
}

function closePublish() { closeModal("pubModal"); }

/* 探测项目的构建产物目录：自动选中第一个可用的，并把全部候选列成 chips */
async function probeDists() {
  const p = projects.find(x => x.id === pubId);
  if (!p) return;

  $("distChips").innerHTML = "";
  setPathHint("正在探测构建产物…");

  let r = null;
  try { r = await invoke("publish_probe", { projectPath: p.path }); } catch (e) { /* 探测失败按无候选处理 */ }
  pubProbe = r;
  if (!r) { setPathHint(""); return; }

  const items = (r.candidates || []).map(c => ({
    text: c.rel + (c.hasIndex ? " ✓" : ""),
    sub: c.files + " 文件 · " + c.sizeMb + "MB",
    act: "dist-pick",
    data: { path: c.path },
    title: c.path,
  }));
  if (r.rootStatic) {
    items.push({ text: "项目根目录", sub: "纯静态", act: "dist-pick", data: { path: p.path } });
  }

  $("distChips").innerHTML = UI.chipList(items);
  if ((r.candidates || []).length) $("pubPath").value = r.candidates[0].path;
  refreshPubHint();
}

/* 目录说明行：variant 为 bad / good，缺省是中性说明 */
function setPathHint(text, variant) {
  const el = $("pubHint");
  el.className = cls("field-hint", variant && "is-" + variant);
  el.textContent = text || "";
}

/* 输入框变化 / 选 chip 后刷新：目录是否存在 + 探测结论 */
let checkSeq = 0;
async function refreshPubHint() {
  const v = $("pubPath").value.trim();
  $$(".chip", $("distChips")).forEach(el => el.classList.toggle("is-on", el.dataset.path === v));

  if (!v) {
    setPathHint(pubProbe ? pubProbe.hint : "");
    return;
  }

  const seq = ++checkSeq;
  let ok = false;
  try { ok = await invoke("check_dir", { path: v }); } catch (e) { ok = false; }
  if (seq !== checkSeq) return;   // 期间又有新输入，丢弃这次结果

  if (ok) {
    setPathHint("✓ 目录存在，可以发布", "good");
  } else {
    let msg = "✗ 这个目录不存在 —— 需要先构建出静态产物再发布。";
    if (pubProbe && pubProbe.buildCmd) msg += " 构建命令：" + pubProbe.buildCmd.split("→")[0].trim();
    setPathHint(msg, "bad");
  }
}

async function doPublish() {
  const dist = $("pubPath").value.trim();
  if (!dist) return toast("请填写部署目录");

  const proj = projects.find(x => x.id === pubId);
  const hosting = pubHosting || "cloudflare";
  const isGh = hosting === "github";

  $("pubBtn").disabled = true;
  $("pubBtn").textContent = "发布中…";
  $("pubStatus").innerHTML = isGh
    ? '<div class="pub-loading">正在推送到 GitHub（大产物目录上传需要一会儿）…</div>'
    : '<div class="pub-loading">正在上传到 Cloudflare（首次约需 1–2 分钟下载 wrangler）…</div>';

  try {
    const r = await invoke("publish_project", {
      distPath: dist,
      projectId: proj ? proj.id : "",
      projectName: proj ? proj.name : "",
      projectPath: proj ? proj.path : "",
      hosting,                                  // 本次选的方式（空则后端读设置）
      branch: pubGh ? pubGh.branch : "",
    });

    if (r.ok && r.url) {
      showPubResult(r);
      refreshPublishes();   // 记录已落盘，同步角标与列表
    } else {
      const msg = (r.error || "未知错误").split("\n").slice(0, 12).join("\n");
      $("pubStatus").innerHTML = '<div class="pub-err">发布失败：\n' + esc(msg) + "</div>";
      $("pubBtn").disabled = false;
      $("pubBtn").textContent = "重试";
    }
  } catch (e) {
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

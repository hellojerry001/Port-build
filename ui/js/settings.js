/* =============================================================================
   settings.js · 设置页（GitHub 同步）
   -----------------------------------------------------------------------------
   这一页只做两件事：把本机 git 环境**读出来**，把身份与偏好**写回去**。
   它是「项目一键提交 GitHub」的前置 —— 提交要用的 user.name / user.email /
   credential helper 在这里先确认好，真到提交那一步就不该再让用户填任何东西。

   写法与 about.js 一致：状态集中在顶部，渲染一律由 renderSettings() 从状态推导，
   不做增量改 DOM。唯一的例外是三个输入框 —— 它们是「用户正在编辑」的那份数据，
   只在首次载入（或保存成功后回填）时才写，否则每敲一个字都会被打回来的旧值覆盖。
   ============================================================================= */

let gsStatus = null;   // git_status 的结果，null = 还没回来 / 读失败
let gsRepos = null;    // git_repo_states 的结果，null = 还没回来
let gsDraft = null;    // 正在编辑的 GitSettings 副本（输入框的真值在这里）
let gsLoaded = false;  // 是否已载入过一次（重复进页面不再重拉）
let gsBusy = false;    // 保存 / 写全局配置进行中
let gsErr = "";        // 失败原因（红字）
let gsOk = "";         // 成功回执（灰字）
let gsTokenUrl = "";   // 令牌页地址，由后端给（前端不硬编码 GitHub 路径）

/* GitHub 账号绑定（gh CLI 一键 / OAuth Device Flow 双轨） */
let gsClientId = "";   // 用户自填的 Client ID（非机密）；留空则用 App 内置的
let gsGhLogin = null;  // 已绑定账号的用户名；null = 未记名
let gsHasCred = false; // 钥匙串里有没有可推送的 github.com 凭证（与「已绑定」是两回事）
let ghDevice = null;   // 当前设备授权会话 { deviceCode, userCode, uri, interval }
let ghTimer = null;    // 授权轮询定时器
let ghCli = null;      // gh CLI 可用状态 { installed, loggedIn, canDeviceFlow }

/* 自检项的达成 / 未达成图标。与 rail 那套一样是描边型，
   属性写在样式层（.set-mark svg），这里只给形状。 */
const GS_ICON = {
  ok:  '<path d="M4.2 10.6 8 14.4l7.8-8.8"/>',
  bad: '<path d="M10 4.8v6.2"/>' +
       '<circle cx="10" cy="14.9" r=".9" fill="currentColor" stroke="none"/>',
};
const gsSvg = k => '<svg viewBox="0 0 20 20">' + GS_ICON[k] + "</svg>";

/* ============================== 渲染 ============================== */

/* 后端 GitSettings → 编辑副本。后端一律给全字段，这里只是把缺省语义说清楚：
   开关只有显式 false 才算关，省得将来后端加字段时把用户的选择吃成 false。 */
function draftFromSettings(s) {
  s = s || {};
  return {
    name: s.name || "",
    email: s.email || "",
    defaultBranch: s.defaultBranch || "main",
    autoGitignore: s.autoGitignore !== false,
    autoFirstCommit: s.autoFirstCommit !== false,
  };
}

/// 只在「编辑副本刚被替换或后端做了归一化」时调用，不要在每次渲染里调
function writeDraftToInputs() {
  if (!gsDraft) return;
  $("gsName").value = gsDraft.name;
  $("gsEmail").value = gsDraft.email;
  $("gsBranch").value = gsDraft.defaultBranch;
}

function renderChecks() {
  const ready = $("gsReady");
  const box = $("gsChecks");
  ready.classList.remove("is-ok", "is-bad");

  if (!gsStatus) {
    ready.textContent = gsErr ? "读取失败" : "检测中…";
    if (gsErr) ready.classList.add("is-bad");
    box.innerHTML =
      '<div class="set-check">' +
        '<span class="set-mark' + (gsErr ? " is-bad" : "") + '">' +
          (gsErr ? gsSvg("bad") : UI.spinner("")) +
        "</span>" +
        '<div class="set-main"><div class="set-row-label">' +
          (gsErr ? "读不到本机 git 环境" : "正在读本机 git 环境…") +
        "</div>" +
        (gsErr ? '<div class="set-row-note is-bad">' + esc(gsErr) + "</div>" : "") +
        "</div>" +
      "</div>";
    return;
  }

  const pending = gsStatus.checks.filter(c => !c.ok).length;
  ready.textContent = gsStatus.ready ? "已就绪" : pending + " 项待处理";
  ready.classList.add(gsStatus.ready ? "is-ok" : "is-bad");

  /* 检测完的清单默认收起：全绿时列表没有信息量，只留「已就绪」状态。
     有失败项时才展开，用户得知道是哪项没过、为什么。 */
  if (gsStatus.ready) {
    box.innerHTML = "";
    return;
  }

  box.innerHTML = gsStatus.checks.map(c =>
    '<div class="set-check">' +
      '<span class="set-mark ' + (c.ok ? "is-ok" : "is-bad") + '">' + gsSvg(c.ok ? "ok" : "bad") + "</span>" +
      '<div class="set-main">' +
        '<div class="set-row-label">' + esc(c.label) + "</div>" +
        '<div class="set-row-note">' + esc(c.detail) + "</div>" +
      "</div>" +
    "</div>").join("");
}

function renderIdentity() {
  const hint = $("gsHint");
  hint.classList.remove("is-bad");

  const save = document.querySelector('[data-act="gs-save"]');
  const glob = document.querySelector('[data-act="gs-global"]');
  // 后端设置还没到手时也禁用：点下去只会提交一份空草稿
  [save, glob].forEach(b => { if (b) b.disabled = gsBusy || !gsDraft; });

  if (gsErr) {
    hint.textContent = gsErr;
    hint.classList.add("is-bad");
    return;
  }
  if (gsOk) {
    hint.textContent = gsOk;
    return;
  }
  if (!gsStatus) {
    hint.textContent = "";
    return;
  }

  /* 生效身份是「本页设置优先、否则 git 全局」的后端结论。
     把它显出来，用户才知道自己填的到底有没有被用上。 */
  if (gsStatus.name && gsStatus.email) {
    hint.textContent = "当前生效：" + gsStatus.name + " <" + gsStatus.email +
                       ">（" + gsStatus.nameSource + "）";
  } else {
    hint.textContent = "还没有提交身份：填上点「保存设置」，或点「写入 git 全局配置」同步到命令行";
    hint.classList.add("is-bad");
  }
}

function renderToggles() {
  const set = (el, on) => {
    if (!el) return;
    el.classList.toggle("is-on", on);
    el.setAttribute("aria-checked", on ? "true" : "false");
    // 还没拿到后端设置（gsDraft 为空）时不可点：否则点下去是「改了一份不存在的草稿」
    el.disabled = gsBusy || !gsDraft;
  };
  set($("gsIgnore"), !!(gsDraft && gsDraft.autoGitignore));
  set($("gsFirst"), !!(gsDraft && gsDraft.autoFirstCommit));
}

function repoRow(r) {
  const name = '<span class="set-ellipsis set-repo-name" title="' + esc(r.path) + '">' + esc(r.name) + "</span>";
  if (!r.isRepo) {
    return "<tr><td>" + name + '</td><td colspan="4" class="set-mute">还不是 git 仓库</td></tr>';
  }
  const num = n => n > 0
    ? '<span class="set-num is-warn">' + n + "</span>"
    : '<span class="set-mute">0</span>';
  const dash = '<span class="set-mute">—</span>';
  // 数字列必须带上 col-num：表头是右对齐的，单元格不跟着右对齐就会错位
  return "<tr><td>" + name + "</td>" +
    "<td>" + (r.branch ? esc(r.branch) : dash) + "</td>" +
    '<td class="set-remote">' + (r.remote
      ? '<span class="set-ellipsis" title="' + esc(r.remote) + '">' + esc(r.remote) + "</span>"
      : '<span class="set-mute">未设置</span>') + "</td>" +
    '<td class="col-num">' + num(r.dirty) + "</td>" +
    // 没有远端时「未推送」没有意义（rev-list 会返回 0，看着像「都推完了」）
    '<td class="col-num">' + (r.remote ? num(r.unpushed) : dash) + "</td></tr>";
}

function renderRepos() {
  const note = $("gsRepoNote");
  const body = $("gsRepoBody");

  if (gsRepos === null) {
    note.textContent = "正在读本机仓库状态…";
    body.innerHTML = '<tr><td colspan="5" class="empty-state is-compact">' + UI.spinner("读取中") + "</td></tr>";
    return;
  }
  if (!gsRepos.length) {
    note.textContent = "还没有项目";
    body.innerHTML = '<tr><td colspan="5" class="empty-state is-compact">先到「项目管理」里添加项目</td></tr>';
    return;
  }
  const repos = gsRepos.filter(r => r.isRepo).length;
  note.textContent = gsRepos.length + " 个项目 · " + repos + " 个已是 git 仓库";
  body.innerHTML = gsRepos.map(repoRow).join("");
}

/// GitHub 账号区：绑定前后是两种完全不同的内容，所以整块重画。
/// Client ID 输入框只在用户没聚焦时回填 —— 否则每敲一个字都被旧值顶掉。
function renderGh() {
  const idInput = $("ghClientId");
  if (idInput && document.activeElement !== idInput) idInput.value = gsClientId;

  const box = $("ghAccountBody");
  if (!box) return;

  // ② 没记名，但本机钥匙串里已有一份能推送的凭证（多半是以前某次 push 留下的）。
  //    这不是「未绑定」—— 推送是通的，只是 App 不知道它属于谁，所以给「认领」。
  if (gsGhLogin) {
    box.innerHTML =
      '<div class="gh-row gh-bound">' +
        '<span class="gh-dot"></span>' +
        '<div class="gh-main">' +
          '<div class="gh-name">已绑定 @' + esc(gsGhLogin) + "</div>" +
          '<div class="gh-note">推送时会用这个账号的凭证' +
            ' · <a class="gh-inline-link" href="https://github.com/settings/applications"' +
            ' target="_blank" rel="noopener">在 GitHub 上管理授权</a></div>' +
        "</div>" +
        '<button class="btn is-ghost is-danger" data-act="gh-unbind">解绑</button>' +
      "</div>";
    return;
  }

  if (gsHasCred) {
    box.innerHTML =
      '<div class="gh-row gh-partial">' +
        '<span class="gh-dot"></span>' +
        '<div class="gh-main">' +
          '<div class="gh-name">已有推送凭证</div>' +
          '<div class="gh-note">git 用它推送没问题，只是还没记下属于哪个账号' +
            ' · <a class="gh-inline-link" href="https://github.com/settings/applications"' +
            ' target="_blank" rel="noopener">在 GitHub 上管理授权</a></div>' +
        "</div>" +
        '<button class="btn is-ghost" data-act="gh-bind">重新绑定</button>' +
        '<button class="btn is-primary" data-act="gh-claim">认领账号</button>' +
      "</div>";
    return;
  }

  box.innerHTML =
    '<div class="gh-row gh-unbound">' +
      '<span class="gh-dot"></span>' +
      '<div class="gh-main">' +
        '<div class="gh-name">未绑定 GitHub 账号</div>' +
        '<div class="gh-note">绑定后才能把项目推送到 GitHub</div>' +
      "</div>" +
      '<button class="btn is-primary" data-act="gh-bind">绑定账号</button>' +
    "</div>";
}

function renderSettings() {
  renderChecks();
  renderIdentity();
  renderToggles();
  renderRepos();
  renderGh();
}

/* ============================== 数据 ============================== */

async function loadGitStatus() {
  gsErr = "";
  try {
    gsStatus = await invoke("git_status");
    if (!gsDraft) {
      gsDraft = draftFromSettings(gsStatus.settings);
      writeDraftToInputs();
    }
  } catch (e) {
    gsStatus = null;
    gsErr = String(e);
  }
  if (gsStatus) {
    gsClientId = (gsStatus.settings && gsStatus.settings.githubClientId) || "";
    gsGhLogin = gsStatus.githubLogin || null;
    // 凭证（能不能推）与绑定（记不记得是谁）是两个独立的事实，都要读
    gsHasCred = !!gsStatus.hasGithubCred;
  }
  renderSettings();
}

async function loadRepos() {
  gsRepos = null;
  renderSettings();
  try {
    gsRepos = await invoke("git_repo_states");
  } catch (e) {
    gsRepos = [];
    toast("读取仓库状态失败：" + e);
  }
  renderSettings();
}

/// 进入设置页：首次进来补数据。仓库状态单独一条命令并且后到 ——
/// 它要为每个项目 spawn 好几个子进程，不该挡住首屏。
async function enterSettings() {
  if (gsLoaded) return;
  gsLoaded = true;
  // ⚠️ 必须**先**画一版再 await：git_status 慢（或后端卡住）时，
  // 页面上得先有「正在读」的交代。等回来再渲染的话，那段时间整块卡片是空的。
  renderSettings();
  await loadGitStatus();
  loadRepos();
}

/* ============================== 交互 ============================== */

onInput("gs-field", el => {
  if (!gsDraft) return;
  gsDraft[el.dataset.k] = el.value;
  gsErr = "";
  gsOk = "";
  // 只更新提示与按钮态，不回写输入框 —— 否则光标会被顶走
  renderIdentity();
});

/// silent=true 用于开关：它没有「保存」按钮可点，必须即时落盘；
/// 失败时把开关拨回去，不能让它停在离真相的那一侧。
async function saveGitSettings(silent) {
  if (!gsDraft || gsBusy) return;
  gsBusy = true;
  gsErr = "";
  gsOk = "";
  renderSettings();

  try {
    const saved = await invoke("save_git_settings", {
      settings: {
        name: gsDraft.name,
        email: gsDraft.email,
        defaultBranch: gsDraft.defaultBranch,
        autoGitignore: gsDraft.autoGitignore,
        autoFirstCommit: gsDraft.autoFirstCommit,
      },
    });
    // 后端会去空白并校验，回填它的结果（用户能看见 " Jerry " 被收成 "Jerry"）
    gsDraft = draftFromSettings(saved);
    writeDraftToInputs();
    if (gsStatus) gsStatus.settings = saved;
    if (!silent) {
      gsOk = "已保存";
      toast("设置已保存");
    }
  } catch (e) {
    gsErr = String(e);
    if (silent) {
      gsDraft = draftFromSettings(gsStatus && gsStatus.settings);
      writeDraftToInputs();
    }
    if (!silent) toast(gsErr);
  }

  gsBusy = false;
  renderSettings();
}

on("gs-save", () => saveGitSettings(false));

on("gs-toggle", el => {
  if (!gsDraft) return;
  gsDraft[el.dataset.k] = !gsDraft[el.dataset.k];
  saveGitSettings(true);
});

on("gs-global", async () => {
  if (!gsDraft || gsBusy) return;
  gsBusy = true;
  gsErr = "";
  gsOk = "";
  renderSettings();
  try {
    gsOk = await invoke("apply_git_identity", { name: gsDraft.name, email: gsDraft.email });
    // 身份来源变了（「git 全局配置」→「本页设置」），自检要重跑一遍
    await loadGitStatus();
    toast(gsOk);
  } catch (e) {
    gsErr = String(e);
    toast(gsErr);
  }
  gsBusy = false;
  renderSettings();
});

on("gs-refresh", async () => {
  if (gsBusy) return;
  gsOk = "";
  loadRepos();
  await loadGitStatus();
});

/* ============================== GitHub 账号绑定 ============================== */

function closeGhTimer() {
  if (ghTimer) { clearTimeout(ghTimer); ghTimer = null; }
}

function fallbackCopy(t) {
  const ta = document.createElement("textarea");
  ta.value = t;
  ta.style.position = "fixed";
  ta.style.opacity = "0";
  document.body.appendChild(ta);
  ta.select();
  try { document.execCommand("copy"); } catch (_) { /* 复制不了就算了，码本来也能手打 */ }
  document.body.removeChild(ta);
}

/// 优先用 Clipboard API；Tauri webview 里它被拒时退回 execCommand
function copyText(t) {
  try {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(t).catch(() => fallbackCopy(t));
      return;
    }
  } catch (_) { /* 没有 clipboard API，走兜底 */ }
  fallbackCopy(t);
}

/// 绑定入口是「双轨」的，按可用性从省事到费事依次降级：
///   ① gh 已登录 → 一键绑定（连浏览器都不用开）
///   ② 装了 gh 没登录 → 引导终端跑一次 gh auth login
///   ③ 没装 gh → 浏览器 Device Flow；内置/已存的 Client ID 都不可用时才让用户自己填
async function openGhBind() {
  $("ghBindModal").classList.add("is-open");
  try { ghCli = await invoke("gh_cli_status"); }
  catch (_) { ghCli = { installed: false, loggedIn: false, canDeviceFlow: true }; }

  if (ghCli.loggedIn) { renderGhBindCli(); return; }
  if (ghCli.installed) { renderGhBindCliLogin(); return; }
  if (!ghCli.canDeviceFlow) renderGhBindAskId();
  else startDeviceFlow();
}

function renderGhBindCli() {
  $("ghBindBody").innerHTML =
    '<div class="gh-step">' +
      '<div class="gh-step-label">检测到 gh 已登录</div>' +
      '<div class="field-hint">直接用它的登录态绑定，不用再开浏览器授权。</div>' +
      ghCredNotice() +
    "</div>";
  $("ghBindFoot").innerHTML =
    '<button class="btn is-ghost" data-act="gh-bind-close">取消</button>' +
    '<button class="btn is-primary" data-act="gh-bind-cli">立即绑定</button>';
}

function renderGhBindCliLogin() {
  $("ghBindBody").innerHTML =
    '<div class="gh-step">' +
      '<div class="gh-step-label">先用 gh 登录一次</div>' +
      '<div class="field-hint">在终端跑 <code class="gp-cmd">gh auth login</code>，' +
      "完成后回到这里点「继续」。</div>" +
      ghCredNotice() +
    "</div>";
  $("ghBindFoot").innerHTML =
    '<button class="btn is-ghost" data-act="gh-bind-close">取消</button>' +
    (ghCli && ghCli.canDeviceFlow
      ? '<button class="btn is-ghost" data-act="gh-browser">改用浏览器授权</button>'
      : "") +
    '<button class="btn is-primary" data-act="gh-bind-cli">继续</button>';
}

async function doGhBindCli() {
  try {
    const login = await invoke("gh_bind_cli");
    gsGhLogin = login || gsGhLogin;
    closeGhBind();
    renderGh();
    loadGitStatus();   // 钥匙串现在有凭证了，自检要重跑一遍
    toast("已绑定 GitHub 账号" + (gsGhLogin ? " @" + gsGhLogin : ""));
  } catch (e) { toast(String(e)); }
}

function renderGhBindAskId() {
  $("ghBindBody").innerHTML =
    '<div class="gh-step">' +
      '<div class="gh-step-label">先填 OAuth App 的 Client ID</div>' +
      '<div class="field">' +
        '<input class="input" id="ghClientIdModal" value="' + esc(gsClientId) + '"' +
        ' placeholder="Client ID" autocomplete="off" spellcheck="false" />' +
      "</div>" +
      '<div class="field-hint">在 ' +
        '<a href="https://github.com/settings/developers" target="_blank" rel="noopener">' +
        "github.com/settings/developers</a> 创建 OAuth App，回调地址留空即可。</div>" +
      ghCredNotice() +
    "</div>";
  $("ghBindFoot").innerHTML =
    '<button class="btn is-ghost" data-act="gh-bind-close">取消</button>' +
    '<button class="btn is-primary" data-act="gh-start">下一步</button>';
}

async function ghStart() {
  const v = ($("ghClientIdModal") || $("ghClientId")).value.trim();
  if (!v) { toast("请先填 Client ID"); return; }
  try {
    await invoke("save_gh_client_id", { clientId: v });
    gsClientId = v;
    renderGh();
  } catch (e) { toast(String(e)); return; }
  startDeviceFlow();
}

async function startDeviceFlow() {
  closeGhTimer();
  $("ghBindFoot").innerHTML =
    '<button class="btn is-ghost" data-act="gh-bind-close">取消</button>';
  $("ghBindBody").innerHTML =
    '<div class="gh-step">' +
      '<div class="gh-step-label">正在申请授权码…</div>' +
      '<div class="gh-wait">' + UI.spinner("连接 GitHub") + "</div>" +
    "</div>";

  let code;
  try {
    code = await invoke("gh_device_code", { clientId: gsClientId, scope: "repo" });
  } catch (e) {
    $("ghBindBody").innerHTML =
      '<div class="gh-step">' +
        '<div class="gh-step-label is-bad">申请授权码失败</div>' +
        '<div class="field-hint is-bad">' + esc(String(e)) + "</div>" +
      "</div>";
    $("ghBindFoot").innerHTML =
      '<button class="btn is-ghost" data-act="gh-bind-close">关闭</button>' +
      '<button class="btn is-primary" data-act="gh-restart">重新获取</button>';
    return;
  }

  ghDevice = {
    deviceCode: code.deviceCode,
    userCode: code.userCode,
    uri: code.verificationUri,
    interval: Math.max(1, code.interval || 5),
  };
  renderGhBindCode();
  pollDevice();
}

function renderGhBindCode() {
  $("ghBindBody").innerHTML =
    '<div class="gh-step">' +
      '<div class="gh-step-label">在浏览器里登录并输入下面的码</div>' +
      '<div class="gh-code" id="ghCode">' + esc(ghDevice.userCode) + "</div>" +
      '<div class="gh-code-actions">' +
        '<button class="btn is-ghost" data-act="gh-copy-code">复制</button>' +
        '<button class="btn is-ghost" data-act="gh-open-uri">打开验证页</button>' +
      "</div>" +
      '<div class="gh-wait" id="ghWait">' + UI.spinner("正在等待你在浏览器里授权…") + "</div>" +
      ghCredNotice() +
    "</div>";
}

async function pollDevice() {
  if (!ghDevice) return;
  let r;
  try {
    r = await invoke("gh_device_poll", { clientId: gsClientId, deviceCode: ghDevice.deviceCode });
  } catch (e) {
    const w = $("ghWait");
    if (w) w.innerHTML = '<div class="field-hint is-bad">轮询失败：' + esc(String(e)) + "</div>";
    return;
  }

  if (r.status === "pending") {
    ghTimer = setTimeout(pollDevice, ghDevice.interval * 1000);
    return;
  }

  closeGhTimer();

  if (r.status === "authorized") {
    gsGhLogin = r.login || gsGhLogin;
    closeGhBind();
    renderGh();
    loadGitStatus();  // 钥匙串现在有凭证了，自检要重跑一遍
    toast("已绑定 GitHub 账号" + (gsGhLogin ? " @" + gsGhLogin : ""));
    return;
  }

  // expired / denied
  $("ghBindBody").innerHTML =
    '<div class="gh-step">' +
      '<div class="gh-step-label is-bad">' + esc(r.error || "授权未通过") + "</div>" +
    "</div>";
  $("ghBindFoot").innerHTML =
    '<button class="btn is-ghost" data-act="gh-bind-close">关闭</button>' +
    '<button class="btn is-primary" data-act="gh-restart">重新获取</button>';
}

function closeGhBind() {
  closeGhTimer();
  const m = $("ghBindModal");
  if (m) m.classList.remove("is-open");
}

/// 认领已有凭证：只问一次 API 把登录名补进设置，**不碰钥匙串**。
/// 老用户凭证本来就能推送，绑定不该变成「换掉一份好凭证」的风险操作。
/// 已有凭证时给个提醒：绑定会替换钥匙串，而「认领」不会。
function ghCredNotice() {
  if (!gsHasCred) return "";
  return '<div class="field-hint is-warn">本机钥匙串里已有一份能推送的凭证，继续绑定会替换它。' +
         "只是想记下它属于哪个账号的话，关掉这里改点「认领账号」。</div>";
}

async function doGhClaim() {
  try {
    const login = await invoke("gh_claim_existing");
    if (!login) { toast("没能认领到账号"); return; }
    gsGhLogin = login;
    renderGh();
    toast("已认领账号 @" + login);
  } catch (e) { toast(String(e)); }
}

async function doGhUnbind() {
  const who = gsGhLogin ? "@" + gsGhLogin : "GitHub 账号";
  if (!confirm("确定解绑 " + who + " 吗？\n\n解绑会删除钥匙串里的 GitHub 凭证，本地项目文件不受影响。")) return;
  try {
    await invoke("gh_unbind");
    gsGhLogin = null;
    renderGh();
    loadGitStatus();
    toast("已解绑");
  } catch (e) { toast(String(e)); }
}

on("gh-save-id", async () => {
  const v = $("ghClientId").value.trim();
  try {
    await invoke("save_gh_client_id", { clientId: v });
    gsClientId = v;
    toast(v ? "已保存 Client ID" : "已清除 Client ID");
  } catch (e) { toast(String(e)); }
});

on("gh-bind", () => openGhBind());
on("gh-unbind", () => doGhUnbind());
on("gh-claim", () => doGhClaim());
on("gh-bind-close", () => closeGhBind());
on("gh-start", () => ghStart());
on("gh-restart", () => startDeviceFlow());
on("gh-bind-cli", () => doGhBindCli());
on("gh-browser", () => {
  if (ghCli && ghCli.canDeviceFlow) startDeviceFlow();
  else renderGhBindAskId();
});
on("gh-copy-code", () => {
  if (ghDevice) { copyText(ghDevice.userCode); toast("已复制授权码"); }
});
on("gh-open-uri", () => {
  if (ghDevice) openLink(ghDevice.uri);
});

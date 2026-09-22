/* =============================================================================
   repos.js · 项目仓库页（GitHub 账号绑定 + 同步检测 + 仓库列表）
   -----------------------------------------------------------------------------
   从 settings.js 拆分出来：用户需要在一个独立页面管理 GitHub 同步与项目仓库，
   设置页只保留提交身份与默认值。渲染方式与 settings.js 一致：状态集中，
   renderRepoPage() 从状态推导，不做增量 DOM 修改。
   ============================================================================= */

let repoStatus = null;   // git_status 的结果，null = 还没回来 / 读失败
let repoRepos = null;    // git_repo_states 的结果，null = 还没回来
let repoLoaded = false;  // 是否已载入过一次（重复进页面不再重拉）
let repoBusy = false;    // 检测 / 绑定进行中
let repoErr = "";        // 失败原因

/* GitHub 账号绑定（gh CLI 一键 / OAuth Device Flow 双轨） */
let repoClientId = "";   // 用户自填的 Client ID（非机密）；留空则用 App 内置的
let repoGhLogin = null;  // 已绑定账号的用户名；null = 未记名
let repoHasCred = false; // 钥匙串里有没有可推送的 github.com 凭证
let repoGhDevice = null; // 当前设备授权会话 { deviceCode, userCode, uri, interval }
let repoGhTimer = null;  // 授权轮询定时器
let repoGhCli = null;    // gh CLI 可用状态 { installed, loggedIn, canDeviceFlow }

/* 自检项的达成 / 未达成图标 */
const REPO_ICON = {
  ok:  '<path d="M4.2 10.6 8 14.4l7.8-8.8"/>',
  bad: '<path d="M10 4.8v6.2"/>' +
       '<circle cx="10" cy="14.9" r=".9" fill="currentColor" stroke="none"/>',
};
const repoSvg = k => '<svg viewBox="0 0 20 20">' + REPO_ICON[k] + "</svg>";

/* ============================== 渲染 ============================== */

function renderRepoChecks() {
  const ready = $("gsReady");
  const box = $("gsChecks");
  ready.classList.remove("is-ok", "is-bad");

  if (!repoStatus) {
    ready.textContent = repoErr ? "读取失败" : "检测中…";
    if (repoErr) ready.classList.add("is-bad");
    box.innerHTML =
      '<div class="set-check">' +
        '<span class="set-mark' + (repoErr ? " is-bad" : "") + '">' +
          (repoErr ? repoSvg("bad") : UI.spinner("")) +
        "</span>" +
        '<div class="set-main"><div class="set-row-label">' +
          (repoErr ? "读不到本机 git 环境" : "正在读本机 git 环境…") +
        "</div>" +
        (repoErr ? '<div class="set-row-note is-bad">' + esc(repoErr) + "</div>" : "") +
        "</div>" +
      "</div>";
    return;
  }

  const pending = repoStatus.checks.filter(c => !c.ok).length;
  ready.textContent = repoStatus.ready ? "已就绪" : pending + " 项待处理";
  ready.classList.add(repoStatus.ready ? "is-ok" : "is-bad");

  /* 检测完的清单默认收起：全绿时列表没有信息量，只留「已就绪」状态。 */
  if (repoStatus.ready) {
    box.innerHTML = "";
    return;
  }

  box.innerHTML = repoStatus.checks.map(c =>
    '<div class="set-check">' +
      '<span class="set-mark ' + (c.ok ? "is-ok" : "is-bad") + '">' + repoSvg(c.ok ? "ok" : "bad") + "</span>" +
      '<div class="set-main">' +
        '<div class="set-row-label">' + esc(c.label) + "</div>" +
        '<div class="set-row-note">' + esc(c.detail) + "</div>" +
      "</div>" +
    "</div>").join("");
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
  return "<tr><td>" + name + "</td>" +
    "<td>" + (r.branch ? esc(r.branch) : dash) + "</td>" +
    '<td class="set-remote">' + (r.remote
      ? '<span class="set-ellipsis" title="' + esc(r.remote) + '">' + esc(r.remote) + "</span>"
      : '<span class="set-mute">未设置</span>') + "</td>" +
    '<td class="col-num">' + num(r.dirty) + "</td>" +
    '<td class="col-num">' + (r.remote ? num(r.unpushed) : dash) + "</td></tr>";
}

function renderRepoRepos() {
  const note = $("gsRepoNote");
  const body = $("gsRepoBody");

  if (repoRepos === null) {
    note.textContent = "正在读本机仓库状态…";
    body.innerHTML = '<tr><td colspan="5" class="empty-state is-compact">' + UI.spinner("读取中") + "</td></tr>";
    return;
  }
  if (!repoRepos.length) {
    note.textContent = "还没有项目";
    body.innerHTML = '<tr><td colspan="5" class="empty-state is-compact">先到「项目管理」里添加项目</td></tr>';
    return;
  }
  const repos = repoRepos.filter(r => r.isRepo).length;
  note.textContent = repoRepos.length + " 个项目 · " + repos + " 个已是 git 仓库";
  body.innerHTML = repoRepos.map(repoRow).join("");
}

function renderRepoGh() {
  const idInput = $("ghClientId");
  if (idInput && document.activeElement !== idInput) idInput.value = repoClientId;

  const box = $("ghAccountBody");
  if (!box) return;

  if (repoGhLogin) {
    box.innerHTML =
      '<div class="gh-row gh-bound">' +
        '<span class="gh-dot"></span>' +
        '<div class="gh-main">' +
          '<div class="gh-name">已绑定 @' + esc(repoGhLogin) + "</div>" +
          '<div class="gh-note">推送时会用这个账号的凭证' +
            ' · <a class="gh-inline-link" href="https://github.com/settings/applications"' +
            ' target="_blank" rel="noopener">在 GitHub 上管理授权</a></div>' +
        "</div>" +
        '<button class="btn is-outline is-danger" data-act="gh-unbind">解绑</button>' +
      "</div>";
    return;
  }

  if (repoHasCred) {
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

function renderRepoPage() {
  renderRepoChecks();
  renderRepoRepos();
  renderRepoGh();
}

/* ============================== 数据 ============================== */

async function loadRepoStatus() {
  repoErr = "";
  try {
    repoStatus = await invoke("git_status");
  } catch (e) {
    repoStatus = null;
    repoErr = String(e);
  }
  if (repoStatus) {
    repoClientId = (repoStatus.settings && repoStatus.settings.githubClientId) || "";
    repoGhLogin = repoStatus.githubLogin || null;
    repoHasCred = !!repoStatus.hasGithubCred;
  }
  renderRepoPage();
}

async function loadRepoRepos() {
  repoRepos = null;
  renderRepoPage();
  try {
    repoRepos = await invoke("git_repo_states");
  } catch (e) {
    repoRepos = [];
    toast("读取仓库状态失败：" + e);
  }
  renderRepoPage();
}

async function enterRepos() {
  if (repoLoaded) return;
  repoLoaded = true;
  renderRepoPage();
  await loadRepoStatus();
  loadRepoRepos();
}

/* ============================== 交互 ============================== */

on("gs-refresh", async () => {
  if (repoBusy) return;
  loadRepoRepos();
  await loadRepoStatus();
});

/* ============================== GitHub 账号绑定 ============================== */

function closeRepoGhTimer() {
  if (repoGhTimer) { clearTimeout(repoGhTimer); repoGhTimer = null; }
}

function fallbackCopy(t) {
  const ta = document.createElement("textarea");
  ta.value = t;
  ta.style.position = "fixed";
  ta.style.opacity = "0";
  document.body.appendChild(ta);
  ta.select();
  try { document.execCommand("copy"); } catch (_) { }
  document.body.removeChild(ta);
}

function copyText(t) {
  try {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(t).catch(() => fallbackCopy(t));
      return;
    }
  } catch (_) { }
  fallbackCopy(t);
}

async function openGhBind() {
  $("ghBindModal").classList.add("is-open");
  try { repoGhCli = await invoke("gh_cli_status"); }
  catch (_) { repoGhCli = { installed: false, loggedIn: false, canDeviceFlow: true }; }

  if (repoGhCli.loggedIn) { renderGhBindCli(); return; }
  if (repoGhCli.installed) { renderGhBindCliLogin(); return; }
  if (!repoGhCli.canDeviceFlow) renderGhBindAskId();
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
    (repoGhCli && repoGhCli.canDeviceFlow
      ? '<button class="btn is-ghost" data-act="gh-browser">改用浏览器授权</button>'
      : "") +
    '<button class="btn is-primary" data-act="gh-bind-cli">继续</button>';
}

async function doGhBindCli() {
  try {
    const login = await invoke("gh_bind_cli");
    repoGhLogin = login || repoGhLogin;
    closeGhBind();
    renderRepoGh();
    loadRepoStatus();
    toast("已绑定 GitHub 账号" + (repoGhLogin ? " @" + repoGhLogin : ""));
  } catch (e) { toast(String(e)); }
}

function renderGhBindAskId() {
  $("ghBindBody").innerHTML =
    '<div class="gh-step">' +
      '<div class="gh-step-label">先填 OAuth App 的 Client ID</div>' +
      '<div class="field">' +
        '<input class="input" id="ghClientIdModal" value="' + esc(repoClientId) + '"' +
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
    repoClientId = v;
    renderRepoGh();
  } catch (e) { toast(String(e)); return; }
  startDeviceFlow();
}

async function startDeviceFlow() {
  closeRepoGhTimer();
  $("ghBindFoot").innerHTML =
    '<button class="btn is-ghost" data-act="gh-bind-close">取消</button>';
  $("ghBindBody").innerHTML =
    '<div class="gh-step">' +
      '<div class="gh-step-label">正在申请授权码…</div>' +
      '<div class="gh-wait">' + UI.spinner("连接 GitHub") + "</div>" +
    "</div>";

  let code;
  try {
    code = await invoke("gh_device_code", { clientId: repoClientId, scope: "repo" });
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

  repoGhDevice = {
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
      '<div class="gh-code" id="ghCode">' + esc(repoGhDevice.userCode) + "</div>" +
      '<div class="gh-code-actions">' +
        '<button class="btn is-ghost" data-act="gh-copy-code">复制</button>' +
        '<button class="btn is-ghost" data-act="gh-open-uri">打开验证页</button>' +
      "</div>" +
      '<div class="gh-wait" id="ghWait">' + UI.spinner("正在等待你在浏览器里授权…") + "</div>" +
      ghCredNotice() +
    "</div>";
}

async function pollDevice() {
  if (!repoGhDevice) return;
  let r;
  try {
    r = await invoke("gh_device_poll", { clientId: repoClientId, deviceCode: repoGhDevice.deviceCode });
  } catch (e) {
    const w = $("ghWait");
    if (w) w.innerHTML = '<div class="field-hint is-bad">轮询失败：' + esc(String(e)) + "</div>";
    return;
  }

  if (r.status === "pending") {
    repoGhTimer = setTimeout(pollDevice, repoGhDevice.interval * 1000);
    return;
  }

  closeRepoGhTimer();

  if (r.status === "authorized") {
    repoGhLogin = r.login || repoGhLogin;
    closeGhBind();
    renderRepoGh();
    loadRepoStatus();
    toast("已绑定 GitHub 账号" + (repoGhLogin ? " @" + repoGhLogin : ""));
    return;
  }

  $("ghBindBody").innerHTML =
    '<div class="gh-step">' +
      '<div class="gh-step-label is-bad">' + esc(r.error || "授权未通过") + "</div>" +
    "</div>";
  $("ghBindFoot").innerHTML =
    '<button class="btn is-ghost" data-act="gh-bind-close">关闭</button>' +
    '<button class="btn is-primary" data-act="gh-restart">重新获取</button>';
}

function closeGhBind() {
  closeRepoGhTimer();
  const m = $("ghBindModal");
  if (m) m.classList.remove("is-open");
}

function ghCredNotice() {
  if (!repoHasCred) return "";
  return '<div class="field-hint is-warn">本机钥匙串里已有一份能推送的凭证，继续绑定会替换它。' +
         "只是想记下它属于哪个账号的话，关掉这里改点「认领账号」。</div>";
}

async function doGhClaim() {
  try {
    const login = await invoke("gh_claim_existing");
    if (!login) { toast("没能认领到账号"); return; }
    repoGhLogin = login;
    renderRepoGh();
    toast("已认领账号 @" + login);
  } catch (e) { toast(String(e)); }
}

async function doGhUnbind() {
  const who = repoGhLogin ? "@" + repoGhLogin : "GitHub 账号";
  if (!confirm("确定解绑 " + who + " 吗？\n\n解绑会删除钥匙串里的 GitHub 凭证，本地项目文件不受影响。")) return;
  try {
    await invoke("gh_unbind");
    repoGhLogin = null;
    renderRepoGh();
    loadRepoStatus();
    toast("已解绑");
  } catch (e) { toast(String(e)); }
}

on("gh-save-id", async () => {
  const v = $("ghClientId").value.trim();
  try {
    await invoke("save_gh_client_id", { clientId: v });
    repoClientId = v;
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
  if (repoGhCli && repoGhCli.canDeviceFlow) startDeviceFlow();
  else renderGhBindAskId();
});
on("gh-copy-code", () => {
  if (repoGhDevice) { copyText(repoGhDevice.userCode); toast("已复制授权码"); }
});
on("gh-open-uri", () => {
  if (repoGhDevice) openLink(repoGhDevice.uri);
});

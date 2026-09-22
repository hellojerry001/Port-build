/* =============================================================================
   settings.js · 设置页（提交身份 + 默认值 + 外观）
   -----------------------------------------------------------------------------
   GitHub 账号绑定、同步检测与项目仓库已拆到「项目仓库」页（repos.js）。
   这一页只负责三件事：把本机 git 环境**读出来**，把身份与偏好**写回去**，
   以及提供外观（主题）设置入口。

   设置项采用「左侧标题 + 说明 / 右侧操作控件」的列表式行结构；
   提交身份因为需要同时编辑两个字段，点右侧「编辑」后走弹窗。
   ============================================================================= */

let gsDraft = null;      // 正在编辑的 GitSettings 副本（输入框的真值在这里）
let gsLoaded = false;    // 是否已载入过一次（重复进页面不再重拉）
let gsBusy = false;      // 保存 / 写全局配置进行中
let gsErr = "";          // 失败原因（红字）
let gsOk = "";           // 成功回执（灰字）
let gsIdentity = null;   // 当前生效的提交身份 {name, email, nameSource}
let gitStatus = null;    // 最近一次 git_status 的完整结果

/* GitHub 账号绑定（gh CLI 一键 / OAuth Device Flow 双轨） */
let ghClientId = "";     // 用户自填的 Client ID（非机密）；留空则用 App 内置的
let ghLogin = null;      // 已绑定账号的用户名；null = 未记名
let ghHasCred = false;   // 钥匙串里有没有可推送的 github.com 凭证
let ghDevice = null;     // 当前设备授权会话 { deviceCode, userCode, uri, interval }
let ghTimer = null;      // 授权轮询定时器
let ghCli = null;        // gh CLI 可用状态 { installed, loggedIn, canDeviceFlow }

/* ============================== 渲染 ============================== */

function draftFromSettings(s) {
  s = s || {};
  return {
    name: s.name || "",
    email: s.email || "",
    defaultBranch: s.defaultBranch || "main",
    autoGitignore: s.autoGitignore !== false,
    autoFirstCommit: s.autoFirstCommit !== false,
    hosting: s.hosting === "github" ? "github" : "cloudflare",
    ghBranch: s.ghBranch || "gh-pages",
    ghRepoPrefix: s.ghRepoPrefix === undefined ? "pb-" : s.ghRepoPrefix,
    ghAutoPages: s.ghAutoPages !== false,
  };
}

/* 托管方式：下拉里的一项。说明行跟着当前选择变，讲清这条路的代价 */
const HOSTINGS = [
  {
    key: "cloudflare",
    label: "Cloudflare 临时链接",
    note: "匿名临时部署，默认 60 分钟失效，可在窗口期内认领为永久",
  },
  {
    key: "github",
    label: "GitHub Pages",
    note: "长期有效，产物推到仓库分支；免费账号只能用 public 仓库",
  },
];

function hostingKey() {
  return gsDraft ? gsDraft.hosting : "cloudflare";
}

function syncHostingRows() {
  const cur = hostingKey();
  const hit = HOSTINGS.filter(h => h.key === cur)[0];
  const note = $("hsNote");
  if (note) note.textContent = hit ? hit.note : "";

  // 产物分支 / 仓库前缀 / 自动开 Pages 只有 GitHub 托管才有意义：
  // 选了 Cloudflare 时**整块不展示**（含下方那句 public 仓库提示），
  // 而不是灰掉 —— 这几项在 Cloudflare 路径上完全用不到，留着只是噪音。
  const gh = cur === "github";
  const rows = $("hsGhRows");
  if (rows) rows.hidden = !gh;
  const hint = $("hsHint");
  if (hint) hint.hidden = !gh;
  ["hsBranch", "hsPrefix", "hsAutoPages"].forEach(id => {
    const el = $(id);
    if (el) el.disabled = gsBusy || !gsDraft || !gh;
  });
}

function writeDraftToInputs() {
  if (!gsDraft) return;
  $("gsBranch").value = gsDraft.defaultBranch;
  $("gsEditName").value = gsDraft.name;
  $("gsEditEmail").value = gsDraft.email;
  $("hsBranch").value = gsDraft.ghBranch;
  $("hsPrefix").value = gsDraft.ghRepoPrefix;
}

function renderIdentity() {
  const note = $("gsIdentityNote");
  note.classList.remove("is-bad");

  if (gsErr) {
    note.textContent = gsErr;
    note.classList.add("is-bad");
    return;
  }
  if (gsOk) {
    note.textContent = gsOk;
    return;
  }
  if (!gsIdentity) {
    note.textContent = "读取中…";
    return;
  }

  if (gsIdentity.name && gsIdentity.email) {
    note.textContent = gsIdentity.name + " <" + gsIdentity.email + ">（" +
                       gsIdentity.nameSource + "）";
  } else {
    note.textContent = "还没有提交身份，点击编辑设置";
    note.classList.add("is-bad");
  }
}

function renderEditHint() {
  const hint = $("gsEditHint");
  hint.classList.remove("is-bad");

  if (gsErr) {
    hint.textContent = gsErr;
    hint.classList.add("is-bad");
    return;
  }
  if (gsOk) {
    hint.textContent = gsOk;
    return;
  }

  const save = document.querySelector('[data-act="gs-edit-save"]');
  const glob = document.querySelector('[data-act="gs-edit-global"]');
  [save, glob].forEach(b => { if (b) b.disabled = gsBusy || !gsDraft; });

  hint.textContent = "";
}

function renderToggles() {
  const set = (el, on) => {
    if (!el) return;
    el.classList.toggle("is-on", on);
    el.setAttribute("aria-checked", on ? "true" : "false");
    el.disabled = gsBusy || !gsDraft;
  };
  set($("gsIgnore"), !!(gsDraft && gsDraft.autoGitignore));
  set($("gsFirst"), !!(gsDraft && gsDraft.autoFirstCommit));
  set($("hsAutoPages"), !!(gsDraft && gsDraft.ghAutoPages));
}

function renderGhAccount() {
  const idInput = $("ghClientId");
  if (idInput && document.activeElement !== idInput) idInput.value = ghClientId;

  const box = $("ghAccountBody");
  if (!box) return;

  if (ghLogin) {
    box.innerHTML =
      '<div class="gh-row gh-bound">' +
        '<span class="gh-dot"></span>' +
        '<div class="gh-main">' +
          '<div class="gh-name">已绑定 @' + esc(ghLogin) + "</div>" +
          '<div class="gh-note">推送时会用这个账号的凭证' +
            ' · <a class="gh-inline-link" href="https://github.com/settings/applications"' +
            ' target="_blank" rel="noopener">在 GitHub 上管理授权</a></div>' +
        "</div>" +
        '<button class="btn is-outline is-danger" data-act="gh-unbind">解绑</button>' +
      "</div>";
    return;
  }

  if (ghHasCred) {
    // 有凭证但不知道属于谁：也允许解绑 —— 否则这份凭证在这页上没有任何出口
    box.innerHTML =
      '<div class="gh-row gh-partial">' +
        '<span class="gh-dot"></span>' +
        '<div class="gh-main">' +
          '<div class="gh-name">已有推送凭证</div>' +
          '<div class="gh-note">git 用它推送没问题，只是还没记下属于哪个账号' +
            ' · <a class="gh-inline-link" href="https://github.com/settings/applications"' +
            ' target="_blank" rel="noopener">在 GitHub 上管理授权</a></div>' +
        "</div>" +
        '<button class="btn is-outline is-danger" data-act="gh-unbind">解绑</button>' +
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
  renderIdentity();
  renderGhAccount();
  renderToggles();
  syncHostingRows();
  renderDrop("theme");
  renderDrop("hosting");
}

/* ============================== 数据 ============================== */

async function loadGitSettings() {
  gsErr = "";
  try {
    const s = await invoke("git_status");
    gitStatus = s;
    gsIdentity = {
      name: s.name,
      email: s.email,
      nameSource: s.nameSource,
    };
    ghClientId = (s.settings && s.settings.githubClientId) || "";
    ghLogin = s.githubLogin || null;
    ghHasCred = !!s.hasGithubCred;
    if (!gsDraft) {
      gsDraft = draftFromSettings(s.settings);
      writeDraftToInputs();
    }
  } catch (e) {
    gitStatus = null;
    gsIdentity = null;
    gsErr = String(e);
  }
  renderSettings();
}

async function enterSettings() {
  if (gsLoaded) return;
  gsLoaded = true;
  renderSettings();
  await loadGitSettings();
}

/* ============================== 交互 ============================== */

onInput("gs-field", el => {
  if (!gsDraft) return;
  gsDraft[el.dataset.k] = el.value;
  gsErr = "";
  gsOk = "";
  renderIdentity();
  renderEditHint();
});

/* 文本框失焦（或按 Enter）即落盘。
   之前这些框只在「提交身份」弹窗里点保存才会写回去 —— 改了默认分支却
   没打开过弹窗的话，改动会静默丢掉。保存后 writeDraftToInputs 会把
   后端收口过的值写回输入框，用户能立刻看到规范化结果（如前缀 PB → pb-）。 */
onChange("gs-field-save", el => {
  if (!gsDraft || gsBusy) return;
  gsDraft[el.dataset.k] = el.value;
  saveGitSettings(true);
});

async function saveGitSettings(silent) {
  if (!gsDraft || gsBusy) return false;
  gsBusy = true;
  gsErr = "";
  gsOk = "";
  renderSettings();
  renderEditHint();

  try {
    const saved = await invoke("save_git_settings", {
      settings: {
        name: gsDraft.name,
        email: gsDraft.email,
        defaultBranch: gsDraft.defaultBranch,
        autoGitignore: gsDraft.autoGitignore,
        autoFirstCommit: gsDraft.autoFirstCommit,
        // ⚠️ 托管字段必须一起带上：后端是逐字段覆盖的，
        // 漏传会按 serde default 把用户的选择重置回 cloudflare
        hosting: gsDraft.hosting,
        ghBranch: gsDraft.ghBranch,
        ghRepoPrefix: gsDraft.ghRepoPrefix,
        ghAutoPages: gsDraft.ghAutoPages,
      },
    });
    gsDraft = draftFromSettings(saved);
    writeDraftToInputs();
    if (!silent) {
      gsOk = "已保存";
      toast("设置已保存");
    }
    await loadGitSettings();
    return true;
  } catch (e) {
    gsErr = String(e);
    toast(gsErr);
    return false;
  } finally {
    gsBusy = false;
    renderSettings();
    renderEditHint();
  }
}

on("gs-toggle", el => {
  if (!gsDraft || gsBusy) return;
  const k = el.dataset.k;
  const old = gsDraft[k];
  gsDraft[k] = !old;
  renderSettings();
  saveGitSettings(true).then(ok => {
    if (!ok) {
      gsDraft[k] = old;
      writeDraftToInputs();
      renderSettings();
    }
  });
});

/* 托管方式下拉里选中一项：切完立刻落盘（和上面的开关同一套回滚逻辑） */
function applyHosting(key) {
  if (!gsDraft || gsBusy) return;
  if (!key || key === gsDraft.hosting) return;
  const old = gsDraft.hosting;
  gsDraft.hosting = key;
  renderSettings();
  saveGitSettings(true).then(ok => {
    if (!ok) {
      gsDraft.hosting = old;
      renderSettings();
    }
  });
}

async function applyGitIdentity() {
  if (!gsDraft || gsBusy) return;
  gsBusy = true;
  gsErr = "";
  gsOk = "";
  renderSettings();
  renderEditHint();
  try {
    gsOk = await invoke("apply_git_identity", { name: gsDraft.name, email: gsDraft.email });
    await loadGitSettings();
    toast(gsOk);
  } catch (e) {
    gsErr = String(e);
    toast(gsErr);
  }
  gsBusy = false;
  renderSettings();
  renderEditHint();
}

/* ============================== 提交身份编辑弹窗 ============================== */

function isGsEditOpen() { return $("gsEditModal").classList.contains("is-open"); }

function openGsEdit() {
  if (!gsDraft) return;
  gsErr = "";
  gsOk = "";
  writeDraftToInputs();
  renderEditHint();
  $("gsEditModal").classList.add("is-open");
}

function closeGsEdit() {
  $("gsEditModal").classList.remove("is-open");
  gsErr = "";
  gsOk = "";
  renderSettings();
}

on("gs-edit", () => openGsEdit());
on("gs-edit-close", () => closeGsEdit());
on("gs-edit-cancel", () => closeGsEdit());
on("gs-edit-save", async () => {
  const ok = await saveGitSettings(false);
  if (ok) closeGsEdit();
});
on("gs-edit-global", () => applyGitIdentity());

/* 弹窗内输入框按 Enter 直接保存 */
document.getElementById("gsEditModal").addEventListener("keydown", e => {
  if (e.key === "Enter" && isGsEditOpen()) {
    e.preventDefault();
    document.querySelector('[data-act="gs-edit-save"]').click();
  }
});

/* ============================== 下拉选择器（通用） ==============================
   「主题」「托管方式」是同一个交互：右侧一个触发器，点开是一个选项浮层。
   展开 / 收起 / 点外关闭这套逻辑只写一份，两边各自给出选项与选中回调。 */

const DROPS = {
  theme: {
    fieldId: "themeField",
    triggerId: "themeTrigger",
    popoverId: "themePopover",
    labelId: "themeTriggerLabel",
    items: () => {
      const cur = PBTheme.mode();
      return PBTheme.MODES.map(m => ({ key: m, label: PBTheme.LABEL[m], on: m === cur }));
    },
    pick: key => PBTheme.set(key),
  },
  hosting: {
    fieldId: "hsHostingField",
    triggerId: "hsHostingTrigger",
    popoverId: "hsHostingPopover",
    labelId: "hsHostingLabel",
    items: () => {
      const cur = hostingKey();
      return HOSTINGS.map(h => ({ key: h.key, label: h.label, on: h.key === cur }));
    },
    pick: key => applyHosting(key),
  },
};

function dropIsOpen(name) {
  const d = DROPS[name];
  return !!d && $(d.popoverId).classList.contains("is-open");
}

/* 触发器文字永远跟着当前值走；浮层开着的话顺带刷新选中态 */
function renderDrop(name) {
  const d = DROPS[name];
  if (!d || !$(d.popoverId)) return;
  const items = d.items();
  const cur = items.filter(i => i.on)[0];
  const label = $(d.labelId);
  if (label) label.textContent = cur ? cur.label : "—";

  $(d.popoverId).innerHTML = items.map(o =>
    '<button class="' + cls("sel-opt", o.on && "is-on") + '"' +
    dataAttrs({ act: "sel-pick", drop: name, key: o.key }) + ">" +
      "<span>" + esc(o.label) + "</span>" +
      '<svg class="tick" viewBox="0 0 20 20"><path d="M4.8 10.4 8.4 14l6.8-8"/></svg>' +
    "</button>").join("");
}

function openDrop(name) {
  const d = DROPS[name];
  renderDrop(name);
  $(d.popoverId).classList.add("is-open");
  $(d.popoverId).setAttribute("aria-hidden", "false");
  $(d.triggerId).setAttribute("aria-expanded", "true");
}

function closeDrop(name) {
  const d = DROPS[name];
  $(d.popoverId).classList.remove("is-open");
  $(d.popoverId).setAttribute("aria-hidden", "true");
  $(d.triggerId).setAttribute("aria-expanded", "false");
}

on("sel-trigger", el => {
  const name = el.dataset.drop;          // 触发器用 data-drop 指明自己属于哪个下拉
  if (!DROPS[name]) return;
  if (dropIsOpen(name)) closeDrop(name);
  else openDrop(name);
});

/* ⚠️ 每次点选项都用 name 现查节点：浮层是整块 innerHTML 重渲的，
   旧引用点过一次就脱离文档、再点不会触发委托。 */
on("sel-pick", el => {
  const name = el.dataset.drop;
  const d = DROPS[name];
  if (!d) return;
  closeDrop(name);
  d.pick(el.dataset.key);
});

/* 点触发器与浮层以外的地方收起（两个下拉各自独立判断） */
document.addEventListener("click", e => {
  Object.keys(DROPS).forEach(name => {
    if (!dropIsOpen(name)) return;
    if (e.target.closest("#" + DROPS[name].fieldId)) return;
    closeDrop(name);
  });
});

/* 主题切换后同步本页显示（订阅时立即回调一次，所以首次渲染也会触发） */
PBTheme.subscribe(() => renderDrop("theme"));

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
    ghLogin = login || ghLogin;
    closeGhBind();
    renderGhAccount();
    loadGitSettings();
    toast("已绑定 GitHub 账号" + (ghLogin ? " @" + ghLogin : ""));
  } catch (e) { toast(String(e)); }
}

function renderGhBindAskId() {
  $("ghBindBody").innerHTML =
    '<div class="gh-step">' +
      '<div class="gh-step-label">先填 OAuth App 的 Client ID</div>' +
      '<div class="field">' +
        '<input class="input" id="ghClientIdModal" value="' + esc(ghClientId) + '"' +
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
    ghClientId = v;
    renderGhAccount();
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
    code = await invoke("gh_device_code", { clientId: ghClientId, scope: "repo" });
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
    r = await invoke("gh_device_poll", { clientId: ghClientId, deviceCode: ghDevice.deviceCode });
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
    ghLogin = r.login || ghLogin;
    closeGhBind();
    renderGhAccount();
    loadGitSettings();
    toast("已绑定 GitHub 账号" + (ghLogin ? " @" + ghLogin : ""));
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
  closeGhTimer();
  const m = $("ghBindModal");
  if (m) m.classList.remove("is-open");
}

function ghCredNotice() {
  if (!ghHasCred) return "";
  return '<div class="field-hint is-warn">本机钥匙串里已有一份能推送的凭证，继续绑定会替换它。' +
         "只是想记下它属于哪个账号的话，关掉这里改点「认领账号」。</div>";
}

async function doGhClaim() {
  try {
    const login = await invoke("gh_claim_existing");
    if (!login) { toast("没能认领到账号"); return; }
    ghLogin = login;
    renderGhAccount();
    toast("已认领账号 @" + login);
  } catch (e) { toast(String(e)); }
}

/* ============================== 解绑（自绘二次确认） ==============================
   这里原来是 window.confirm —— macOS 上 Tauri 用的是 WKWebView，它没有实现
   原生 confirm 面板，`if (!confirm(...)) return` 会静默拿到 false，
   于是点「解绑」什么都没发生。改成和「删除项目」同一套自绘弹窗。 */

let ubBusy = false;   // 解绑请求进行中：期间不再响应按钮与关闭
let ubErr = "";       // 失败原因（留在弹窗里，按钮变「重试」）

function renderGhUnbind() {
  const who = ghLogin ? "@" + ghLogin : "这台机器上的 GitHub 推送凭证";
  $("ubWho").textContent = "解绑 " + who;
  $("ubWhat").textContent = ghLogin
    ? "会删除系统钥匙串里的 github.com 凭证，并清掉本应用记下的账号名。" +
      "项目文件、提交记录与 GitHub 上的仓库都不受影响。"
    : "会删除系统钥匙串里的 github.com 凭证。项目文件、提交记录与 GitHub 上的仓库都不受影响。";
}

/* 提示随「是否一起退 gh CLI」变化 */
function syncGhUnbindTip() {
  if (ubErr) { $("ubTip").textContent = ubErr; return; }

  const cliOn = !!(ghCli && ghCli.loggedIn);
  if (!cliOn) {
    $("ubTip").textContent = "随时可以重新绑定，还是同一个 GitHub 账号。";
    return;
  }
  if ($("ubCli").checked) {
    $("ubTip").textContent = "会一并跑 gh auth logout github.com，之后终端里的 gh 也要重新登录。";
    return;
  }
  // helper 指向 gh 时要点破：不退 gh 的话 git 照样能拿到 token 推送
  const helper = (gitStatus && gitStatus.helper) || "";
  $("ubTip").textContent = /gh/i.test(helper)
    ? "本机 gh CLI 仍处于登录状态，而 credential.helper 正是 " + helper +
      " —— 不一起退出的话，git 推送仍会拿它的凭证。"
    : "本机 gh CLI 仍处于登录状态，不一起退出的话，下次可以一键绑定回同一账号。";
}

async function openGhUnbind() {
  ubBusy = false;
  ubErr = "";
  renderGhUnbind();
  $("ubCli").checked = false;
  $("ubCliRow").hidden = true;      // 先按「没有 gh」渲染，拿到状态再决定要不要露出来
  $("ubBtn").disabled = false;
  $("ubCancel").disabled = false;
  $("ubBtn").textContent = "解绑";
  $("ubTip").textContent = "正在读本机 gh 登录状态…";
  openModal("ghUnbindModal");
  $("ubCancel").focus();

  try { ghCli = await invoke("gh_cli_status"); } catch (_) { /* 读不到就当没装 */ }
  if (modalOpen("ghUnbindModal")) {
    $("ubCliRow").hidden = !(ghCli && ghCli.loggedIn);
    syncGhUnbindTip();
  }
}

function closeGhUnbind() {
  if (ubBusy) return;
  closeModal("ghUnbindModal");
  ubErr = "";
}

async function confirmGhUnbind() {
  if (ubBusy) return;
  const logoutGhCli = !$("ubCliRow").hidden && $("ubCli").checked;

  ubBusy = true;
  ubErr = "";
  $("ubBtn").disabled = true;
  $("ubCancel").disabled = true;
  $("ubBtn").textContent = "解绑中…";
  $("ubTip").textContent = "正在删除钥匙串里的凭证…";

  try {
    const r = await invoke("gh_unbind", { logoutGhCli });
    ubBusy = false;
    closeModal("ghUnbindModal");

    // 解绑后本机立刻变成「未绑定」：先把本地状态清掉再重拉，避免闪回已绑定态
    ghLogin = null;
    ghHasCred = false;
    renderGhAccount();
    try { ghCli = await invoke("gh_cli_status"); } catch (_) { /* 状态读不到不影响解绑结果 */ }
    await loadGitSettings();

    let msg = (r && r.message) || "已解绑";
    if (r && r.ghCliError) msg += "；gh CLI 未退出：" + r.ghCliError;
    // 托管方式还指着 GitHub 的话要点出来，否则下次发布会直接失败
    const hosting = (gitStatus && gitStatus.settings && gitStatus.settings.hosting) || "";
    if (hosting === "github") msg += "。托管方式仍是 GitHub Pages，发布前需要重新绑定账号";
    toast(msg);
  } catch (e) {
    // 失败不关弹窗：把原因留在原处，改完直接重试
    ubBusy = false;
    ubErr = String(e);
    $("ubBtn").disabled = false;
    $("ubCancel").disabled = false;
    $("ubBtn").textContent = "重试";
    syncGhUnbindTip();
  }
}

on("gh-save-id", async () => {
  const v = $("ghClientId").value.trim();
  try {
    await invoke("save_gh_client_id", { clientId: v });
    ghClientId = v;
    toast(v ? "已保存 Client ID" : "已清除 Client ID");
  } catch (e) { toast(String(e)); }
});

on("gh-bind", () => openGhBind());
on("gh-unbind", () => openGhUnbind());
on("gh-unbind-close", () => closeGhUnbind());
on("gh-unbind-confirm", () => confirmGhUnbind());
on("gh-claim", () => doGhClaim());
on("gh-bind-close", () => closeGhBind());
onChange("gh-unbind-cli", () => syncGhUnbindTip());
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

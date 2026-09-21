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

function renderSettings() {
  renderChecks();
  renderIdentity();
  renderToggles();
  renderRepos();
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
  invoke("token_help_url")
    .then(u => { gsTokenUrl = u || ""; })
    .catch(() => { /* 拿不到就退回 GitHub 令牌页首页 */ });
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

on("gs-token", () => openLink(gsTokenUrl || "https://github.com/settings/tokens"));

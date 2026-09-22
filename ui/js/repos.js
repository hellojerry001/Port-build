/* =============================================================================
   repos.js · 项目仓库页（GitHub 同步检测 + 仓库列表）
   -----------------------------------------------------------------------------
   GitHub 账号绑定已拆到「设置」页（settings.js），这一页只负责：
   同步状态检测、项目仓库列表。渲染方式与 settings.js 一致：状态集中，
   renderRepoPage() 从状态推导，不做增量 DOM 修改。
   ============================================================================= */

let repoStatus = null;   // git_status 的结果，null = 还没回来 / 读失败
let repoRepos = null;    // git_repo_states 的结果，null = 还没回来
let repoLoaded = false;  // 是否已载入过一次（重复进页面不再重拉）
let repoBusy = false;    // 检测进行中
let repoErr = "";        // 失败原因

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

function renderRepoPage() {
  renderRepoChecks();
  renderRepoRepos();
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


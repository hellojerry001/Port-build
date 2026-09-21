/* =============================================================================
   push.js · 项目卡片 → 提交并推送到 GitHub
   -----------------------------------------------------------------------------
   入口是卡片上的 git 标记（data-act="git-push-open"，见 projects.js 的 gitBadge）。
   按仓库状态分流：
     - 非仓库 / 无远端      → 说明 + 给命令，不代建
     - 有未提交改动（dirty） → 列文件清单 + 提交说明输入框 → 提交并推送
     - 只有未推送（unpushed）→ 不弹 message，直接「推送 N 个提交」
   面板只决定「做什么」，真实动作全在后端（git_commit_push / git_push），
   失败必须把 git 的真实 stderr 透出来，不能吞。
   ============================================================================= */

let pushId = "";        // 当前面板操作的项目 id
let pushState = null;   // 该项目的 RepoState（来自 projects.js 的 repoStates 全局）
let pushFiles = [];     // 未提交文件清单（来自 git_changed_files）
let pushBusy = false;

/// 打开面板：从全局 repoStates 取项目状态，按需补拉文件清单，再渲染分流
async function openPush(id) {
  const p = projects.find(x => x.id === id);
  if (!p) return;
  const r = repoStates.get(id);

  pushId = id;
  pushState = r || null;
  pushFiles = [];
  pushBusy = false;

  // 每次打开都重置结果区，避免上次成功的「已推送」留在这
  const st = $("gpStatus");
  if (st) { st.hidden = true; st.className = "gp-status"; }
  const dt = $("gpDetail");
  if (dt) { dt.hidden = true; dt.textContent = ""; }

  $("gpName").textContent = p.name;
  renderPush();
  openModal("gitPushModal");

  // 有未提交改动才需要文件清单；其余分支用不到
  if (r && r.isRepo && r.dirty > 0) {
    try {
      pushFiles = await invoke("git_changed_files", { path: p.path });
    } catch (_) {
      pushFiles = [];   // 拿不到清单不阻塞：面板照常显示，只是列表为空
    }
    renderPush();
  }
}

function renderPush() {
  const r = pushState;
  const head = $("gpHead");
  const body = $("gpBody");
  const foot = $("gpFoot");

  if (!r) {
    head.innerHTML = '<div class="gp-note">正在读取仓库状态…</div>';
    body.innerHTML = "";
    foot.innerHTML = "";
    return;
  }

  if (!r.isRepo) {
    head.innerHTML = '<div class="gp-note is-bad">这个项目还不是 git 仓库</div>';
    body.innerHTML = '<div class="gp-hint">先用终端 `git init`，或在「设置 · GitHub 同步」里初始化后再来推送。</div>';
    foot.innerHTML = '<button class="btn is-ghost" data-act="git-push-close">关闭</button>';
    return;
  }

  const branch = r.branch || "（未知分支）";
  head.innerHTML =
    '<div class="gp-row"><span class="gp-k">分支</span><span class="gp-v">' + esc(branch) + "</span></div>" +
    '<div class="gp-row"><span class="gp-k">远端</span><span class="gp-v">' +
      (r.remote
        ? '<span class="gp-ellipsis" title="' + esc(r.remote) + '">' + esc(r.remote) + "</span>"
        : '<span class="gp-bad">未设置</span>') +
    "</span></div>";

  // 无远端：给命令让用户自己关联，应用不替他建仓库、也不猜推到哪
  if (!r.remote) {
    body.innerHTML =
      '<div class="gp-hint is-bad">没有远端，推送前先关联一个：</div>' +
      '<pre class="gp-cmd">git remote add origin &lt;仓库地址&gt;</pre>' +
      '<div class="gp-hint">应用不替你建仓库，也不猜你想推到哪个地址。</div>';
    foot.innerHTML = '<button class="btn is-ghost" data-act="git-push-close">关闭</button>';
    return;
  }

  // 干净的仓库：标记不会为这种状态显示，但 repoStates 可能过期，做个防御
  if (r.dirty === 0 && r.unpushed === 0) {
    body.innerHTML = '<div class="gp-note">没有可推送的内容（已提交且已推送到远端）</div>';
    foot.innerHTML = '<button class="btn is-ghost" data-act="git-push-close">关闭</button>';
    return;
  }

  // dirty>0：列文件 + 提交说明输入框
  if (r.dirty > 0) {
    const list = pushFiles.length
      ? pushFiles.map(f =>
          '<li class="gp-file"><span class="gp-st">' + esc(f.label) + "</span>" +
          '<span class="gp-fname">' + esc(f.path) + "</span></li>").join("")
      : '<li class="gp-file gp-muted">读取文件清单中…</li>';
    body.innerHTML =
      '<div class="gp-label">未提交的改动（' + r.dirty + '）</div>' +
      '<ul class="gp-files">' + list + "</ul>" +
      '<div class="field">' +
        '<label class="field-label">提交说明</label>' +
        '<textarea class="input gp-msg" id="gpMsg" rows="2" placeholder="这次改了什么"></textarea>' +
        '<div class="field-hint" id="gpMsgHint"></div>' +
      "</div>";
    const ta = $("gpMsg");
    if (ta && !ta.value) ta.value = "chore: 更新 " + r.dirty + " 个文件";
    foot.innerHTML =
      '<button class="btn is-ghost" data-act="git-push-close">取消</button>' +
      '<button class="btn is-primary" id="gpDo" data-act="git-push-do">提交并推送</button>';
    return;
  }

  // dirty=0 && unpushed>0：直接推送，不需要 message
  body.innerHTML = '<div class="gp-note">有 <b>' + r.unpushed + "</b> 个提交还没推到远端。</div>";
  foot.innerHTML =
    '<button class="btn is-ghost" data-act="git-push-close">取消</button>' +
    '<button class="btn is-primary" id="gpDo" data-act="git-push-do">推送 ' + r.unpushed + " 个提交</button>";
}

async function doPush() {
  if (!pushId || pushBusy) return;
  const r = pushState;
  if (!r || !r.isRepo || !r.remote) return;
  const p = projects.find(x => x.id === pushId);
  if (!p) return;

  pushBusy = true;
  const doBtn = $("gpDo");
  if (doBtn) doBtn.disabled = true;
  const status = $("gpStatus");
  status.hidden = false;
  status.className = "gp-status";
  status.innerHTML = UI.spinner("处理中…");

  try {
    let out;
    if (r.dirty > 0) {
      const msg = $("gpMsg").value.trim();
      if (!msg) {
        status.className = "gp-status is-bad";
        status.textContent = "请填写提交说明";
        pushBusy = false;
        if (doBtn) doBtn.disabled = false;
        return;
      }
      out = await invoke("git_commit_push", { path: p.path, message: msg });
    } else {
      out = await invoke("git_push", { path: p.path });
    }
    // 成功：结论 + git 真实输出，并刷新卡片标记（未提交/未推送归零或变化）
    status.className = "gp-status is-ok";
    status.textContent = out.headline;
    const detail = $("gpDetail");
    detail.hidden = !out.detail;
    detail.textContent = out.detail || "";
    await loadRepoStates();
    $("gpFoot").innerHTML = '<button class="btn is-primary" data-act="git-push-close">完成</button>';
  } catch (e) {
    // 失败：显示 git 的真实 stderr，不能吞
    status.className = "gp-status is-bad";
    status.textContent = String(e);
    const detail = $("gpDetail");
    detail.hidden = false;
    detail.textContent = String(e);
    pushBusy = false;
    if (doBtn) doBtn.disabled = false;
  }
}

function closePush() {
  pushBusy = false;
  pushId = "";
  pushState = null;
  pushFiles = [];
  closeModal("gitPushModal");
}

on("git-push-open",  el => openPush(el.dataset.id));
on("git-push-do",    () => doPush());
on("git-push-close", () => closePush());

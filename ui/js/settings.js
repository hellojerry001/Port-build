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

/* 托管方式：两个 chip 二选一。说明行跟着当前选择变，讲清这条路的代价 */
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

function renderHostingChips() {
  const cur = gsDraft ? gsDraft.hosting : "cloudflare";
  const box = $("hsChips");
  if (!box) return;
  box.innerHTML = HOSTINGS.map(h =>
    '<button class="' + cls("chip", h.key === cur && "is-on") + '"' +
    dataAttrs({ act: "hs-pick", key: h.key }) + ">" + esc(h.label) + "</button>").join("");

  const hit = HOSTINGS.filter(h => h.key === cur)[0];
  const note = $("hsNote");
  note.textContent = hit ? hit.note : "";

  // GitHub 相关的三个设置只在选了 GitHub 时才有意义 —— 灰掉而不是藏起来，
  // 免得用户以为「设置没了」
  const gh = cur === "github";
  ["hsBranch", "hsPrefix", "hsAutoPages"].forEach(id => {
    const el = $(id);
    if (!el) return;
    el.disabled = gsBusy || !gsDraft || !gh;
    el.classList.toggle("is-inactive", !gh);
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

function renderTheme() {
  const m = PBTheme.mode();
  $("themeTriggerLabel").textContent = PBTheme.LABEL[m];
}

function renderSettings() {
  renderIdentity();
  renderToggles();
  renderTheme();
  renderHostingChips();
}

/* ============================== 数据 ============================== */

async function loadGitSettings() {
  gsErr = "";
  try {
    const s = await invoke("git_status");
    gsIdentity = {
      name: s.name,
      email: s.email,
      nameSource: s.nameSource,
    };
    if (!gsDraft) {
      gsDraft = draftFromSettings(s.settings);
      writeDraftToInputs();
    }
  } catch (e) {
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

/* 托管方式二选一：切完立刻落盘（和上面的开关同一套回滚逻辑） */
on("hs-pick", el => {
  if (!gsDraft || gsBusy) return;
  const key = el.dataset.key;
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
});

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

/* ============================== 主题选择器 ============================== */

function themePopoverOpen() { return $("themePopover").classList.contains("is-open"); }

function renderThemePopover() {
  const cur = PBTheme.mode();
  $("themePopover").innerHTML = PBTheme.MODES.map(m =>
    '<button class="' + cls("theme-opt", m === cur && "is-on") + '"' +
    dataAttrs({ act: "theme-pick", mode: m }) + ">" +
      "<span>" + esc(PBTheme.LABEL[m]) + "</span>" +
      '<svg class="tick" viewBox="0 0 20 20"><path d="M4.8 10.4 8.4 14l6.8-8"/></svg>' +
    "</button>").join("");
}

function openThemePopover() {
  renderThemePopover();
  $("themePopover").classList.add("is-open");
  $("themePopover").setAttribute("aria-hidden", "false");
  $("themeTrigger").setAttribute("aria-expanded", "true");
}

function closeThemePopover() {
  $("themePopover").classList.remove("is-open");
  $("themePopover").setAttribute("aria-hidden", "true");
  $("themeTrigger").setAttribute("aria-expanded", "false");
}

on("theme-trigger", () => {
  if (themePopoverOpen()) closeThemePopover();
  else openThemePopover();
});

on("theme-pick", el => {
  PBTheme.set(el.dataset.mode);
  closeThemePopover();
});

/* 点触发器与浮层以外的地方收起 */
document.addEventListener("click", e => {
  if (!themePopoverOpen()) return;
  if (e.target.closest("#themeField")) return;
  closeThemePopover();
});

/* 主题切换后同步更新本页显示（订阅时立即回调一次，所以首次渲染也会触发） */
PBTheme.subscribe(() => {
  renderTheme();
  if (themePopoverOpen()) renderThemePopover();
});

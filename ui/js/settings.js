/* =============================================================================
   settings.js · 设置页（提交身份 + 默认值 + 外观）
   -----------------------------------------------------------------------------
   GitHub 账号绑定、同步检测与项目仓库已拆到「项目仓库」页（repos.js）。
   这一页只负责三件事：把本机 git 环境**读出来**，把身份与偏好**写回去**，
   以及提供外观（主题）设置入口。

   写法与 about.js 一致：状态集中在顶部，渲染一律由 renderSettings() 从状态推导，
   不做增量改 DOM。唯一的例外是三个输入框 —— 它们是「用户正在编辑」的那份数据，
   只在首次载入（或保存成功后回填）时才写，否则每敲一个字都会被打回来的旧值覆盖。
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
  };
}

function writeDraftToInputs() {
  if (!gsDraft) return;
  $("gsName").value = gsDraft.name;
  $("gsEmail").value = gsDraft.email;
  $("gsBranch").value = gsDraft.defaultBranch;
}

function renderIdentity() {
  const hint = $("gsHint");
  hint.classList.remove("is-bad");

  const save = document.querySelector('[data-act="gs-save"]');
  const glob = document.querySelector('[data-act="gs-global"]');
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
  if (!gsIdentity) {
    hint.textContent = "";
    return;
  }

  if (gsIdentity.name && gsIdentity.email) {
    hint.textContent = "当前生效：" + gsIdentity.name + " <" + gsIdentity.email +
                       ">（" + gsIdentity.nameSource + "）";
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
    el.disabled = gsBusy || !gsDraft;
  };
  set($("gsIgnore"), !!(gsDraft && gsDraft.autoGitignore));
  set($("gsFirst"), !!(gsDraft && gsDraft.autoFirstCommit));
}

function renderTheme() {
  const m = PBTheme.mode();
  $("themeTriggerIcon").innerHTML = themeSvg(THEME_ICON[m]);
  $("themeTriggerLabel").textContent = PBTheme.LABEL[m];
}

function renderSettings() {
  renderIdentity();
  renderToggles();
  renderTheme();
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
});

async function saveGitSettings(silent) {
  if (!gsDraft || gsBusy) return false;
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
  }
}

on("gs-save", () => saveGitSettings(false));

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

on("gs-global", async () => {
  if (!gsDraft || gsBusy) return;
  gsBusy = true;
  gsErr = "";
  gsOk = "";
  renderSettings();
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
});

/* ============================== 主题选择器 ============================== */

function themePopoverOpen() { return $("themePopover").classList.contains("is-open"); }

function renderThemePopover() {
  const cur = PBTheme.mode();
  $("themePopover").innerHTML = PBTheme.MODES.map(m =>
    '<button class="' + cls("theme-opt", m === cur && "is-on") + '"' +
    dataAttrs({ act: "theme-pick", mode: m }) + ">" +
      themeSvg(THEME_ICON[m]) +
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

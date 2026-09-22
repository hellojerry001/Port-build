/* =============================================================================
   templates.js · 从模板新建项目
   ============================================================================= */

const LAST_PARENT = "pb.lastParent";   // 记住上次选的存放目录

/* 模板列表在弹窗与模板库页共用。缓存已有结果，force=true 时重新拉取
   （用户在模板目录里加/删了脚手架，点「刷新」要能看到变化）。 */
async function loadScaffolds(force) {
  if (scaffolds.length && !force) return scaffolds;
  scaffolds = await invoke("list_scaffolds");
  return scaffolds;
}

/* preselect: 从模板库点某张卡片的「用此模板新建」进来时，直接选中那个模板 */
async function openTemplate(preselect) {
  closeEditor();
  tplPicked = "";
  tplBusy = false;

  $("tName").value = "";
  $("tSlug").value = "";
  $("tPort").value = "";
  $("tSlug")._manual = false;   // 目录名是否被手动改过（改过就不再跟随项目名派生）
  $("tParent").value = localStorage.getItem(LAST_PARENT) || "~/WorkBuddy";
  setHint("");
  $("tStatus").textContent = "";
  $("tSubmit").disabled = false;
  $("tSubmit").textContent = "创建并启动";

  openModal("tplModal");

  try {
    await loadScaffolds(true);
  } catch (e) {
    toast(String(e));
    return;
  }

  if (!scaffolds.length) {
    $("tplGrid").innerHTML = UI.empty(
      "还没装模板。把脚手架目录放进 ~/.portbutler/scaffolds 即可（需要含 .pb-scaffold.json）",
      { span: true, compact: true });
    $("tSubmit").disabled = true;
    return;
  }

  const hit = scaffolds.some(s => s.key === preselect);
  pickTpl(hit ? preselect : scaffolds[0].key);
  $("tName").focus();
}

function closeTemplate() {
  if (!tplBusy) closeModal("tplModal");
}

/* 表单顶部提示行：切换模板时先清空，避免残留上一个模板的警告 */
function setHint(text, isWarn) {
  const el = $("tHint");
  el.textContent = text || "";
  el.className = cls("hint", isWarn && "is-warn");
}

function renderTplGrid() {
  $("tplGrid").innerHTML = scaffolds.map(s =>
    '<div class="' + cls("tpl-card", s.key === tplPicked && "is-on") + '"' +
      dataAttrs({ act: "tpl-pick", key: s.key }) + ">" +
      '<div class="tpl-name">' + esc(s.name) + "</div>" +
      '<div class="tpl-desc">' + esc(s.desc) + "</div>" +
      '<div class="tpl-port">端口 ' + esc(s.portStart) + " 起 · Node " +
        esc(s.nodeVersion || "默认") + "</div>" +
    "</div>").join("");
}

async function pickTpl(key) {
  tplPicked = key;
  renderTplGrid();

  const s = scaffolds.find(x => x.key === key);
  if (s && !s.nodeReady) {
    setHint("没找到 Node " + s.nodeVersion + "，将退回系统默认版本启动", true);
  } else {
    setHint("");
  }

  try {
    $("tPort").value = await invoke("suggest_port", { scaffold: key });
  } catch (e) { /* 探测失败就留给用户手填 */ }
}

/* 中文项目名没法直接当目录名，只在名字是 ASCII 时自动派生 */
function onNameInput() {
  const slugEl = $("tSlug");
  if (slugEl._manual) return;
  slugEl.value = $("tName").value.trim().toLowerCase()
    .replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
}

/* 创建过程中的忙碌态：按钮禁用 + 状态行转圈 */
function setBusy(on, text) {
  tplBusy = on;
  $("tSubmit").disabled = on;
  $("tSubmit").textContent = on ? "创建中…" : "创建并启动";
  $("tStatus").innerHTML = text ? UI.spinner(text) : "";
}

async function submitTemplate() {
  if (tplBusy) return;

  const name = $("tName").value.trim();
  const slug = $("tSlug").value.trim().toLowerCase();
  const parent = $("tParent").value.trim();
  const port = Number($("tPort").value) || 0;

  if (!tplPicked) return toast("请先选一个模板");
  if (!name) return toast("请填写项目名");
  if (!slug) return toast("请填写目录名（英文或数字）");
  if (!/^[a-z0-9][a-z0-9_-]*$/.test(slug)) {
    return toast("目录名只能用小写字母、数字、- 和 _，且以字母或数字开头");
  }
  if (!parent) return toast("请填写存放目录");

  localStorage.setItem(LAST_PARENT, parent);

  const t0 = Date.now();
  setBusy(true, "正在克隆模板…");
  const timer = setInterval(() => {
    if (tplBusy) {
      const secs = ((Date.now() - t0) / 1000).toFixed(0);
      $("tStatus").innerHTML = UI.spinner("正在克隆模板… " + secs + "s");
    }
  }, 200);

  let created = null;
  try {
    created = await invoke("create_project", { name, slug, parent, scaffold: tplPicked, port });
    setBusy(true, "正在启动服务…");
    await invoke("start_project", { id: created.id });
  } catch (e) {
    clearInterval(timer);
    setBusy(false, "");
    return toast(String(e));
  }
  clearInterval(timer);
  setBusy(false, "");

  closeModal("tplModal");
  toast("已创建 " + created.name + "（:" + created.port + "），正在启动…");

  await refreshAll();
  setTimeout(refreshAll, 3000);
  setTimeout(() => openBrowser(created.port), 6000);
}

/* ============================== 新增模板 ==============================
   把本机项目 / 同事发来的 .zip 存成内置模板。
   两步走：挑来源 → probe_scaffold_source 体检（有没有描述文件、能不能启动、
   体积多大、有哪些坑）→ 预填表单让用户确认 → import_scaffold 落到 scaffolds。 */

const ADD_FIELDS = ["taName", "taKey", "taPort", "taCmd", "taNode", "taDesc"];

function openAddTemplate() {
  addSrc = "";
  addProbe = null;
  addBusy = false;
  ADD_FIELDS.forEach(id => { $(id).value = ""; });
  setAddSrcLabel("");
  $("taProbe").innerHTML = "";
  setAddHint("");
  $("taStatus").textContent = "";
  $("taSubmit").disabled = true;
  $("taSubmit").textContent = "导入模板";
  openModal("tplAddModal");
}

function closeAddTemplate() {
  if (!addBusy) closeModal("tplAddModal");
}

function setAddSrcLabel(path) {
  const el = $("taSrc");
  el.textContent = path || "未选择";
  el.className = cls("src-path", !path && "is-empty");
  el.title = path || "";
}

function setAddHint(text, isWarn) {
  const el = $("taHint");
  el.textContent = text || "";
  el.className = cls("hint", isWarn && "is-warn");
}

function setAddBusy(on, text) {
  addBusy = on;
  $("taSubmit").disabled = on;
  $("taSubmit").textContent = on ? "导入中…" : "导入模板";
  $("taStatus").innerHTML = text ? UI.spinner(text) : "";
}

function fmtSize(bytes) {
  const mb = (Number(bytes) || 0) / 1024 / 1024;
  if (mb >= 1024) return (mb / 1024).toFixed(1) + "GB";
  if (mb >= 10) return Math.round(mb) + "MB";
  return mb.toFixed(1) + "MB";
}

/* 体检结果：来源摘要 + 顶层条目 + 警告，让用户在导入前就知道会发生什么 */
function renderAddProbe() {
  const p = addProbe;
  if (!p) return;
  const kind = p.kind === "zip" ? "压缩包" : "文件夹";
  const bits = [kind, p.fileCount + " 项", fmtSize(p.sizeBytes)];
  bits.push(p.hasDescriptor ? "含 .pb-scaffold.json" : "无描述文件（字段为推断值）");
  if (p.drilled) bits.push("已下钻子目录");

  const entries = (p.entries || []).length
    ? '<div class="tpl-add-entries">' +
        p.entries.map(e => '<span class="mono">' + esc(e) + "</span>").join("") + "</div>"
    : "";
  const warns = (p.warnings || [])
    .map(w => '<div class="tpl-add-warn">' + esc(w) + "</div>").join("");

  $("taProbe").innerHTML =
    '<div class="tpl-add-meta">' + bits.map(esc).join(" · ") + "</div>" + entries + warns;
}

async function pickAddSource(kind) {
  if (addBusy) return;
  let picked = null;
  try {
    picked = kind === "zip"
      ? await invoke("pick_file", {
          prompt: "选择模板压缩包（.zip）",
          defaultPath: addSrc || "",
          exts: ["zip"],
        })
      : await invoke("pick_folder", {
          prompt: "选择要作为模板的项目目录",
          defaultPath: addSrc || "",
        });
  } catch (e) {
    return toast(String(e));
  }
  if (!picked) return;   // 用户取消

  addSrc = picked;
  addProbe = null;
  setAddSrcLabel(picked);
  $("taSubmit").disabled = true;
  $("taProbe").innerHTML = UI.spinner("正在体检…");

  try {
    addProbe = await invoke("probe_scaffold_source", { source: picked });
  } catch (e) {
    $("taProbe").innerHTML = '<div class="tpl-add-warn">' + esc(String(e)) + "</div>";
    return;
  }

  const s = addProbe.suggested || {};
  $("taName").value = s.name || "";
  $("taKey").value = s.key || "";
  $("taPort").value = s.portStart || "";
  $("taCmd").value = s.command || "";
  $("taNode").value = s.nodeVersion || "";
  $("taDesc").value = s.desc || "";
  renderAddProbe();
  $("taSubmit").disabled = false;
  setAddHint("");
}

async function submitAddTemplate() {
  if (addBusy) return;

  if (!addSrc) return toast("先选择模板来源（文件夹或 .zip）");
  const name = $("taName").value.trim();
  const key = $("taKey").value.trim().toLowerCase();
  const port = Number($("taPort").value) || 0;
  const command = $("taCmd").value.trim();
  const nodeVersion = $("taNode").value.trim();
  const desc = $("taDesc").value.trim();

  if (!name) return toast("请填写模板名称");
  if (!/^[a-z0-9][a-z0-9_-]*$/.test(key)) {
    return toast("标识只能用小写字母、数字、- 和 _，且以字母或数字开头");
  }
  if (!command) return toast("请填写启动命令");

  const t0 = Date.now();
  setAddBusy(true, "正在导入…");
  const timer = setInterval(() => {
    if (addBusy) {
      $("taStatus").innerHTML =
        UI.spinner("正在导入… " + ((Date.now() - t0) / 1000).toFixed(0) + "s");
    }
  }, 200);

  let res = null;
  try {
    res = await invoke("import_scaffold", {
      source: addSrc,
      key, name, desc,
      portStart: port,
      nodeVersion,
      command,
    });
  } catch (e) {
    clearInterval(timer);
    setAddBusy(false, "");
    return toast(String(e));
  }
  clearInterval(timer);
  setAddBusy(false, "");

  closeModal("tplAddModal");
  await loadScaffolds(true);
  renderTplLibrary();

  const notes = (res && res.notes) || [];
  toast("已新增模板「" + (res && res.scaffold ? res.scaffold.name : name) + "」" +
        (notes.length ? "：" + notes.join("；") : ""));

  // 导入的 Node 版本本机没有时会出警告，这里顺带明确说一句
  if (res && res.scaffold && !res.scaffold.nodeReady) {
    setTimeout(() => toast("本机没装 Node " + res.scaffold.nodeVersion +
      "，用它新建项目时会退回系统默认版本"), 3300);
  }
}

/* 模板库页面：内置模板的总览与维护入口。
   数据与弹窗共用 loadScaffolds()，卡片上的「用此模板新建」直接带着 key 打开弹窗；
   右上角「＋ 新增模板」把本机项目 / .zip 导进来。 */

/* 卡片左侧的缩略标识：与 rail「模板库」同一个层叠图标，装进灰底圆角块 */
const TPL_GLYPH =
  '<span class="tpl-lib-icon"><svg viewBox="0 0 20 20">' +
  '<path d="M10 3.6 3.6 6.9 10 10.2l6.4-3.3z"/><path d="M3.6 10.4 10 13.7l6.4-3.3"/>' +
  '<path d="M3.6 13.6 10 16.9l6.4-3.3"/></svg></span>';

async function enterTemplates() {
  const grid = $("tplLibGrid");
  grid.innerHTML = UI.empty(UI.spinner("正在读取模板…"), { span: true, compact: true });
  $("tplLibCount").textContent = "";
  loadTplRoot();

  try {
    await loadScaffolds(true);
  } catch (e) {
    grid.innerHTML = UI.empty("读取模板失败：" + esc(String(e)), { span: true });
    return;
  }
  renderTplLibrary();
}

/* 说明行里的模板目录：后端会把目录建出来并回传绝对路径，拿不到就保留默认文案 */
async function loadTplRoot() {
  try {
    $("tplLibRoot").textContent = await invoke("scaffold_root_path");
  } catch (_) {}
}

function renderTplLibrary() {
  const grid = $("tplLibGrid");
  $("tplLibCount").innerHTML = scaffolds.length
    ? "共 <b>" + scaffolds.length + "</b> 个模板"
    : "";

  if (!scaffolds.length) {
    grid.innerHTML = UI.empty(
      "还没有模板。点右上角「＋ 新增模板」把本机项目或 .zip 导进来，" +
      "也可以直接把模板目录放进 ~/.portbutler/scaffolds（含 .pb-scaffold.json）",
      { span: true });
    return;
  }

  grid.innerHTML = scaffolds.map(s => {
    const ops = [
      UI.btn("用此模板新建", { act: "tpl-lib-new", data: { key: s.key } }),
      UI.btn("打开目录", { variant: "ghost", act: "tpl-lib-reveal", data: { path: s.path } }),
    ].join("");

    return '<div class="card tpl-lib-card">' +
        '<div class="tpl-lib-head">' +
          TPL_GLYPH +
          '<div class="tpl-lib-id">' +
            '<div class="tpl-lib-title">' +
              '<span class="card-name">' + esc(s.name) + "</span>" +
              '<span class="port-chip">:' + esc(s.portStart) + " 起</span>" +
            "</div>" +
            '<div class="tpl-lib-sub">' +
              '<span class="mono">' + esc(s.key) + "</span>" +
              '<span class="sep">·</span>Node ' + esc(s.nodeVersion || "默认") +
            "</div>" +
          "</div>" +
        "</div>" +
        '<div class="tpl-lib-desc">' + esc(s.desc || "（该模板没写说明）") + "</div>" +
        '<div class="tpl-lib-cmd">启动命令 <span class="mono">' + esc(s.command) + "</span></div>" +
        (s.nodeReady ? "" :
          '<div class="tpl-lib-warn">未检测到 Node ' + esc(s.nodeVersion) +
          "，启动将退回系统版本</div>") +
        '<div class="card-ops">' + ops + "</div>" +
      "</div>";
  }).join("");
}

/* ============================== 动作注册 ============================== */
on("template-open",   () => openTemplate());
on("template-close",  () => closeTemplate());
on("template-submit", () => submitTemplate());
on("tpl-pick",        el => pickTpl(el.dataset.key));

/* 模板库页 */
on("tpl-lib-refresh", () => enterTemplates());
on("tpl-lib-new",     el => openTemplate(el.dataset.key));
on("tpl-lib-reveal",  el => showInFinder(el.dataset.path));
on("tpl-lib-reveal-root", async () => {
  try {
    showInFinder(await invoke("scaffold_root_path"));
  } catch (e) {
    toast(String(e));
  }
});

/* 新增模板 */
on("tpl-add-open",     () => openAddTemplate());
on("tpl-add-close",    () => closeAddTemplate());
on("tpl-add-pick-dir", () => pickAddSource("dir"));
on("tpl-add-pick-zip", () => pickAddSource("zip"));
on("tpl-add-submit",   () => submitAddTemplate());

onInput("tpl-name", () => onNameInput());
onInput("tpl-slug", el => { el._manual = el.value.trim().length > 0; });

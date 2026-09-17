/* =============================================================================
   templates.js · 从模板新建项目
   ============================================================================= */

const LAST_PARENT = "pb.lastParent";   // 记住上次选的存放目录

async function openTemplate() {
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
    scaffolds = await invoke("list_scaffolds");
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

  pickTpl(scaffolds[0].key);
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

/* ============================== 动作注册 ============================== */
on("template-open",   () => openTemplate());
on("template-open-from-void", () => { goPage("projects"); openTemplate(); });
on("template-close",  () => closeTemplate());
on("template-submit", () => submitTemplate());
on("tpl-pick",        el => pickTpl(el.dataset.key));

onInput("tpl-name", () => onNameInput());
onInput("tpl-slug", el => { el._manual = el.value.trim().length > 0; });

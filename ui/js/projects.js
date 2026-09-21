/* =============================================================================
   projects.js · 项目列表（Web / Mac 分栏）+ 手动添加/编辑 + 删除确认 + DMG 打包
   ============================================================================= */

const OP_CMD = { start: "start_project", stop: "stop_project" };

/* 两类项目：Web 起 HTTP 服务用浏览器看，Mac 起原生窗口并产出 .dmg */
const KINDS = [
  { key: "web", label: "Web 项目", sub: "Web 应用 · 端口与浏览器预览",
    empty: "还没有 Web 项目，点右上角「从模板新建」或「手动添加」" },
  { key: "mac", label: "Mac 项目", sub: "Mac 桌面应用 · 开发预览与 DMG 打包",
    empty: "还没有 Mac 项目，点右上角「手动添加」把 Tauri 项目登记进来" },
];

const TAB_KEY = "pb.projKind";
let projKind = localStorage.getItem(TAB_KEY) === "mac" ? "mac" : "web";

/* 后端已把 kind 填成 web / mac，这里只兜一层防御 */
const kindOf = p => (p.kind === "mac" ? "mac" : "web");

/* Mac 项目当前图标的 base64 缩略图缓存（key = 项目 id） */
let macIcons = new Map();

/* 拉一遍 Mac 项目的图标，填进 macIcons 后重渲染卡片 */
async function loadMacIcons() {
  const macs = projects.filter(p => kindOf(p) === "mac");
  if (!macs.length) return;
  await Promise.all(macs.map(async p => {
    try {
      const d = await invoke("project_icon", { id: p.id });
      if (d) macIcons.set(p.id, d);
    } catch { /* 没图标就留占位 */ }
  }));
  renderProjects();
}

/* 列表与端口雷达一起刷新：两者都依赖 projects / list_ports */
async function refreshAll() {
  try {
    projects = await invoke("list_projects");
    const ports = await invoke("list_ports");
    busyPorts = new Map(ports.map(p => [p.port, p]));
    runningIds = new Set(await invoke("list_running"));
  } catch (e) {
    toast(String(e));
    return;
  }
  renderTabs();
  renderProjects();
  renderRadar();
  loadMacIcons();
}

/* Web 项目靠端口监听判断，Mac 项目只能认自己拉起来的进程组 —— 两条都算 */
const isLive = p => busyPorts.has(p.port) || runningIds.has(p.id);

function renderTabs() {
  const counts = { web: 0, mac: 0 };
  projects.forEach(p => { counts[kindOf(p)] += 1; });

  $("projTabs").innerHTML = KINDS.map(k =>
    '<button class="' + cls("tab", k.key === projKind && "is-on") + '"' +
      dataAttrs({ act: "proj-tab", kind: k.key }) + ">" +
      esc(k.label) + '<span class="tab-count">' + counts[k.key] + "</span>" +
    "</button>").join("");

  // Mac 分栏不提供「从模板新建」——现有模板都是 Web 脚手架
  $("btnFromTpl").style.display = projKind === "mac" ? "none" : "";

  if (curPage === "projects") $("pgSub").textContent = kindMeta(projKind).sub;
}

const kindMeta = key => KINDS.find(k => k.key === key) || KINDS[0];

function renderProjects() {
  const grid = $("projGrid");
  const list = projects.filter(p => kindOf(p) === projKind);

  if (!list.length) {
    grid.innerHTML = UI.empty(kindMeta(projKind).empty, { span: true });
    return;
  }

  grid.innerHTML = list.map(p => {
    const live = isLive(p);
    const mac = kindOf(p) === "mac";
    const ops = [
      live
        ? UI.btn("停止", { variant: "ghost", act: "op", data: { op: "stop", id: p.id } })
        : UI.btn(mac ? "开发预览" : "启动", { act: "op", data: { op: "start", id: p.id } }),
      // Web 用浏览器看；Mac 起的是原生窗口，换成分发产物
      mac
        ? UI.btn("打包 DMG", { variant: "ghost", act: "build-open", data: { id: p.id } })
        : UI.btn("浏览器", { variant: "ghost", act: "browser", data: { port: p.port } }),
      mac
        ? UI.btn("换图标", { variant: "ghost", act: "icon-swap", data: { id: p.id } })
        : "",
      mac ? "" : UI.btn("发布", { variant: "ghost", act: "publish-open", data: { id: p.id } }),
      UI.btn("编辑", { variant: "ghost", act: "project-edit", data: { id: p.id } }),
      UI.btn("删除", { variant: "ghost", act: "delete-open", data: { id: p.id }, push: true }),
    ].join("");

    // Mac 卡片头加图标缩略图（数据来自 macIcons，换图后实时刷新）
    const icon = mac
      ? '<span class="card-icon' + (macIcons.get(p.id) ? "" : " is-empty") +
        '"' + (macIcons.get(p.id)
          ? ' style="background-image:url(data:image/png;base64,' + macIcons.get(p.id) + ')"'
          : "") + "></span>"
      : "";

    return '<div class="card">' +
        '<div class="card-head">' +
          UI.dot(live) +
          icon +
          '<span class="card-name">' + esc(p.name) + "</span>" +
          (mac ? '<span class="tag">Mac</span>' : "") +
          (p.scaffold ? '<span class="tag">' + esc(p.scaffold) + "</span>" : "") +
          (p.port ? '<span class="port-chip">:' + esc(p.port) + "</span>" : "") +
        "</div>" +
        '<div class="card-meta" title="' + esc(p.path) + '">' +
          esc(p.path) + " · " + esc(p.command) +
        "</div>" +
        '<div class="card-ops">' + ops + "</div>" +
      "</div>";
  }).join("");
}

on("proj-tab", el => {
  projKind = el.dataset.kind === "mac" ? "mac" : "web";
  localStorage.setItem(TAB_KEY, projKind);
  renderTabs();
  renderProjects();
});

/* ============================== 启动 / 停止 / 打开 ============================== */
on("op", async el => {
  const op = el.dataset.op;
  const cmd = OP_CMD[op];
  if (!cmd) return;
  try {
    toast(await invoke(cmd, { id: el.dataset.id }));
  } catch (e) {
    toast(String(e));
  }
  // 启动要等服务真正监听端口，给长一点的观察窗口
  setTimeout(refreshAll, op === "start" ? 1500 : 400);
});

on("browser", el => openBrowser(el.dataset.port));

/* ============================== 手动添加 / 编辑 ============================== */
/* 类型选择：新建时跟随当前分栏，编辑时以项目自身为准 */
let edKind = "web";

function renderKindRow() {
  $("fKindRow").innerHTML = KINDS.map(k =>
    '<button class="' + cls("chip", k.key === edKind && "is-on") + '"' +
      dataAttrs({ act: "kind-pick", kind: k.key }) + ">" + esc(k.label) +
    "</button>").join("");
}

/* 切类型不只是换选中态：端口对 Mac 变成可留空，打包命令字段按需出现 */
function syncKindForm() {
  renderKindRow();
  const mac = edKind === "mac";
  $("fBuildWrap").style.display = mac ? "" : "none";
  $("fPortLabel").textContent = mac ? "端口（可留空）" : "端口";
  $("fCmd").placeholder = mac ? "npm run tauri dev" : "npm run dev";
  $("fKindHint").textContent = mac
    ? "Mac 项目：启动命令拉起的是原生窗口，打包命令产出 .dmg。端口只在项目顺带跑了前端服务时才填。"
    : "Web 项目：启动命令起 HTTP 服务，端口用于浏览器打开与端口雷达。";
}

function openEditor(id) {
  editingId = id || "";
  const p = projects.find(x => x.id === id);
  edKind = p ? kindOf(p) : projKind;
  $("mTitle").textContent = p ? "编辑项目" : "手动添加项目";
  $("fName").value = p?.name || "";
  $("fPath").value = p?.path || "";
  $("fCmd").value = p?.command || "";
  $("fPort").value = p?.port || "";
  $("fNode").value = p?.nodeVersion || "";
  $("fBuild").value = p?.buildCommand || "";
  syncKindForm();
  openModal("editorModal");
}

function closeEditor() { closeModal("editorModal"); }

on("kind-pick", el => {
  edKind = el.dataset.kind === "mac" ? "mac" : "web";
  syncKindForm();
});

async function submitProject() {
  const port = Number($("fPort").value) || 0;
  const project = {
    id: editingId,
    name: $("fName").value.trim(),
    path: $("fPath").value.trim(),
    command: $("fCmd").value.trim(),
    port,
    scaffold: "",
    nodeVersion: $("fNode").value.trim(),
    kind: edKind,
    buildCommand: edKind === "mac" ? $("fBuild").value.trim() : "",
  };
  // 手工编辑不改动模板来源，保留原值
  const old = projects.find(x => x.id === editingId);
  if (old) project.scaffold = old.scaffold || "";

  if (!project.name || !project.path || !project.command) {
    return toast("请填写完整（名称 / 路径 / 命令）");
  }
  if (edKind === "web" && !port) return toast("Web 项目需要填写端口");

  projects = await invoke("save_project", { project });
  closeEditor();
  // 类型可能刚被改过，分栏与计数都要跟着变
  projKind = edKind;
  localStorage.setItem(TAB_KEY, projKind);
  renderTabs();
  renderProjects();
  toast("已保存");
}

/* ============================== 一键换图标（Mac 项目） ============================== */
on("icon-swap", async el => {
  const id = el.dataset.id;
  const p = projects.find(x => x.id === id);
  if (!p) return;
  const path = await invoke("pick_file", {
    prompt: p.name + " · 选择新图标",
    defaultPath: p.path,
    exts: ["png", "jpg", "jpeg", "heic"],
  });
  if (!path) return;
  toast("正在生成图标…");
  try {
    const r = await invoke("swap_icon", { id, imagePath: path });
    macIcons.delete(id);
    await loadMacIcons();
    toast(r.hotPatched ? "图标已更新，已热更新已构建的 .app" : "图标已更新，重新打包生效");
  } catch (e) {
    toast(String(e));
  }
});

/* ============================== 打包 DMG（Mac 项目） ============================== */
let buildId = "", buildTimer = null, buildT0 = 0, buildDmg = "";

async function openBuild(id) {
  const p = projects.find(x => x.id === id);
  if (!p) return;

  buildId = id;
  buildDmg = "";
  buildT0 = Date.now();
  $("bdName").textContent = p.name;
  $("bdStatus").className = "build-status";
  $("bdStatus").innerHTML = UI.spinner("正在启动打包…");
  $("bdLog").textContent = "准备中…";
  $("bdReveal").style.display = "none";
  $("bdCancel").disabled = false;
  openModal("buildModal");

  try {
    await invoke("build_dmg", { id });
  } catch (e) {
    // 已经在打包（比如上次关了窗口又点开）不算失败，继续跟着看就行
    if (!String(e).includes("正在打包")) {
      $("bdStatus").textContent = String(e);
      return;
    }
  }

  clearInterval(buildTimer);
  buildTimer = setInterval(tickBuild, 1500);
  tickBuild();
}

async function tickBuild() {
  if (!buildId) return;
  let s;
  try {
    s = await invoke("build_status", { id: buildId });
  } catch (e) {
    return;
  }
  const secs = ((Date.now() - buildT0) / 1000).toFixed(0);
  const st = $("bdStatus");
  st.className = cls("build-status", !s.running && (s.dmg ? "is-ok" : "is-bad"));
  st.innerHTML = s.running
    ? UI.spinner("打包中… " + secs + "s")
    : (s.dmg
        ? "打包完成 · " + s.dmg.split("/").pop()
        : "打包结束，但没找到 .dmg —— 看上面的日志确认失败原因");
  $("bdLog").textContent = s.tail || "（暂无输出）";
  $("bdLog").scrollTop = $("bdLog").scrollHeight;

  if (s.dmg) {
    buildDmg = s.dmg;
    $("bdReveal").style.display = "";
  }
  if (!s.running) {
    clearInterval(buildTimer);
    buildTimer = null;
    $("bdCancel").disabled = true;
  }
}

/* 关窗口不杀进程：打包继续在后台跑，下次点开能看到最新日志 */
function closeBuild() {
  clearInterval(buildTimer);
  buildTimer = null;
  buildId = "";
  closeModal("buildModal");
}

async function cancelBuild() {
  if (!buildId) return;
  try {
    toast(await invoke("cancel_build", { id: buildId }));
  } catch (e) {
    toast(String(e));
  }
  $("bdStatus").className = "build-status is-bad";
  $("bdStatus").textContent = "已终止打包";
  clearInterval(buildTimer);
  buildTimer = null;
  $("bdCancel").disabled = true;
}

async function revealBuild() {
  if (!buildDmg) return;
  try {
    await invoke("show_in_finder", { path: buildDmg });
  } catch (e) {
    toast(String(e));
  }
}

on("build-open",   el => openBuild(el.dataset.id));
on("build-close",  () => closeBuild());
on("build-cancel", () => cancelBuild());
on("build-reveal", () => revealBuild());

/* ============================== 删除（二次确认） ============================== */
function openDelete(id) {
  const p = projects.find(x => x.id === id);
  if (!p) return;
  delId = id;
  delBusy = false;
  $("delName").textContent = p.name;
  $("delPath").textContent = p.path;
  $("delFiles").checked = true;
  $("delBtn").disabled = false;
  $("delBtn").textContent = "删除";
  $("delCancel").disabled = false;
  $("delTip").dataset.live = busyPorts.has(p.port) ? "1" : "";
  syncDelTip();
  openModal("delModal");
  $("delCancel").focus();
}

/* 提示随复选框与运行状态变化 */
function syncDelTip() {
  const withFiles = $("delFiles").checked;
  const live = $("delTip").dataset.live === "1";
  let tip = withFiles
    ? "整个项目文件夹会被移入废纸篓（可从废纸篓还原），不是永久删除。"
    : "只移除这里的记录，磁盘上的文件夹保持原样。";
  if (withFiles && live) tip += "该项目正在运行，会先自动停止再删除。";
  $("delTip").textContent = tip;
}

function closeDelete() {
  if (delBusy) return;
  closeModal("delModal");
}

async function confirmDelete() {
  if (delBusy) return;
  const id = delId;
  const withFiles = $("delFiles").checked;

  delBusy = true;
  $("delBtn").disabled = true;
  $("delCancel").disabled = true;
  $("delBtn").textContent = withFiles ? "删除中…" : "移除中…";
  $("delTip").textContent = withFiles
    ? "正在停止进程并移动文件夹到废纸篓…"
    : "正在移除记录…";

  try {
    const r = await invoke("delete_project", { id, deleteFiles: withFiles });
    projects = r.list;
    closeModal("delModal");
    renderProjects();
    toast(r.message || "已删除");
  } catch (e) {
    // 失败不关弹窗，把原因留在原处，方便改完重试
    delBusy = false;
    $("delBtn").disabled = false;
    $("delCancel").disabled = false;
    $("delBtn").textContent = "重试";
    $("delTip").textContent = String(e);
  }
  delBusy = false;
  setTimeout(refreshAll, 400);
}

/* ============================== 动作注册 ============================== */
on("project-add",  () => openEditor());
on("project-edit", el => openEditor(el.dataset.id));
on("editor-close", () => closeEditor());
on("project-save", () => submitProject());

on("delete-open",    el => openDelete(el.dataset.id));
on("delete-close",   () => closeDelete());
on("delete-confirm", () => confirmDelete());

onChange("del-files", () => syncDelTip());

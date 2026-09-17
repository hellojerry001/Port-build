/* =============================================================================
   projects.js · 项目列表 + 手动添加/编辑 + 删除确认
   ============================================================================= */

const OP_CMD = { start: "start_project", stop: "stop_project" };

/* 列表与端口雷达一起刷新：两者都依赖 projects / list_ports */
async function refreshAll() {
  try {
    projects = await invoke("list_projects");
    const ports = await invoke("list_ports");
    busyPorts = new Map(ports.map(p => [p.port, p]));
  } catch (e) {
    toast(String(e));
    return;
  }
  renderProjects();
  renderRadar();
}

function renderProjects() {
  const grid = $("projGrid");

  if (!projects.length) {
    grid.innerHTML = UI.empty("还没有项目，点右上角「从模板新建」或「手动添加」", { span: true });
    return;
  }

  grid.innerHTML = projects.map(p => {
    const live = busyPorts.has(p.port);
    const ops = [
      live
        ? UI.btn("停止", { variant: "ghost", act: "op", data: { op: "stop", id: p.id } })
        : UI.btn("启动", { act: "op", data: { op: "start", id: p.id } }),
      UI.btn("浏览器", { variant: "ghost", act: "browser", data: { port: p.port } }),
      UI.btn("发布", { variant: "ghost", act: "publish-open", data: { id: p.id } }),
      UI.btn("编辑", { variant: "ghost", act: "project-edit", data: { id: p.id } }),
      UI.btn("删除", { variant: "ghost", act: "delete-open", data: { id: p.id }, push: true }),
    ].join("");

    return '<div class="card">' +
        '<div class="card-head">' +
          UI.dot(live) +
          '<span class="card-name">' + esc(p.name) + "</span>" +
          (p.scaffold ? '<span class="tag">' + esc(p.scaffold) + "</span>" : "") +
          '<span class="port-chip">:' + esc(p.port) + "</span>" +
        "</div>" +
        '<div class="card-meta" title="' + esc(p.path) + '">' +
          esc(p.path) + " · " + esc(p.command) +
        "</div>" +
        '<div class="card-ops">' + ops + "</div>" +
      "</div>";
  }).join("");
}

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
function openEditor(id) {
  editingId = id || "";
  const p = projects.find(x => x.id === id);
  $("mTitle").textContent = p ? "编辑项目" : "手动添加项目";
  $("fName").value = p?.name || "";
  $("fPath").value = p?.path || "";
  $("fCmd").value = p?.command || "";
  $("fPort").value = p?.port || "";
  $("fNode").value = p?.nodeVersion || "";
  openModal("editorModal");
}

function closeEditor() { closeModal("editorModal"); }

async function submitProject() {
  const project = {
    id: editingId,
    name: $("fName").value.trim(),
    path: $("fPath").value.trim(),
    command: $("fCmd").value.trim(),
    port: Number($("fPort").value),
    scaffold: "",
    nodeVersion: $("fNode").value.trim(),
  };
  // 手工编辑不改动模板来源，保留原值
  const old = projects.find(x => x.id === editingId);
  if (old) project.scaffold = old.scaffold || "";

  if (!project.name || !project.path || !project.command || !project.port) {
    return toast("请填写完整（名称 / 路径 / 命令 / 端口）");
  }
  projects = await invoke("save_project", { project });
  closeEditor();
  renderProjects();
  toast("已保存");
}

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

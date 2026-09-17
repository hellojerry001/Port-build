/* =============================================================================
   radar.js · 端口雷达
   ============================================================================= */

function renderRadar() {
  const q = $("radarSearch").value.trim().toLowerCase();
  const list = Array.from(busyPorts.values()).filter(p =>
    !q || String(p.port).includes(q) || p.process.toLowerCase().includes(q));

  if (!list.length) {
    $("radarBody").innerHTML =
      '<tr><td colspan="4" class="empty-state">当前没有监听中的端口</td></tr>';
    return;
  }

  $("radarBody").innerHTML = list.map(p =>
    "<tr>" +
      "<td><b>" + esc(p.port) + "</b></td>" +
      "<td>" + esc(p.process) + "</td>" +
      '<td class="col-pid">' + esc(p.pid) + "</td>" +
      "<td>" + UI.btn("杀掉", { variant: "ghost", act: "kill-port", data: { port: p.port } }) + "</td>" +
    "</tr>").join("");
}

/* ============================== 动作注册 ============================== */
onInput("radar-search", () => renderRadar());

on("kill-port", async el => {
  try {
    toast(await invoke("kill_port", { port: Number(el.dataset.port) }));
  } catch (e) {
    toast(String(e));
  }
  setTimeout(refreshAll, 300);
});

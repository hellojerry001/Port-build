/* =============================================================================
   radar.js · 端口雷达
   ============================================================================= */

function renderRadar() {
  const q = $("radarSearch").value.trim().toLowerCase();
  const list = Array.from(busyPorts.values()).filter(p =>
    !q || String(p.port).includes(q) || p.process.toLowerCase().includes(q));

  if (!list.length) {
    const searching = !!q;
    const title = searching ? "没有匹配的端口" : "当前没有监听中的端口";
    const desc  = searching ? "换个关键词试试，或清空搜索框" : "有项目跑起来后会自动出现在这里";
    // ⚠️ td 不加 .empty-state：给它 display:flex 会让 colspan 失效、只在第一列居中。
    // 居中交给 td 内的 .empty-state 容器（撑满跨列整行宽度）。
    // 不用 compact：图标要和其他页面一样大（46px）；表格里的纵向留白由
    // `.table td > .empty-state` 单独控制在 34px，不受影响。
    $("radarBody").innerHTML =
      '<tr><td colspan="4">' + UI.emptyState({ title, desc }) + "</td></tr>";
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

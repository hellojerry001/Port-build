/* =============================================================================
   main.js · 装配与启动
   -----------------------------------------------------------------------------
   跨模块的通用动作在这里注册，最后统一启动。各模块自己的动作注册在各自文件末尾。
   ============================================================================= */

/* 弹窗里所有「选择文件夹」按钮共用：从 data-input / data-prompt 取参数 */
on("pick-folder", el => chooseFolder(el.dataset.input, el.dataset.prompt));

/* 工具栏「刷新」：项目列表 + 端口雷达一起拉 */
on("refresh-all", () => refreshAll());

/* 删除确认是唯一的模态操作，Esc 关闭时保持与点击「取消」一致 */
document.addEventListener("keydown", e => {
  if (e.key !== "Escape") return;
  if (modalOpen("delModal")) closeDelete();
  // 打包窗口 Esc 只关窗口，不中断打包（进程由后端持有）
  if (modalOpen("buildModal")) closeBuild();
  if (modalOpen("ghUnbindModal")) closeGhUnbind();
  // 安装到一半不让 Esc 关（closeInstallModal 自己会拦）
  if (modalOpen("abInstallModal")) closeInstallModal();
});

/* ------------------------------ 启动 ------------------------------ */
function boot() {
  bindDelegates();
  renderRail();
  goPage("projects");
  refreshAll();
  refreshPublishes();

  setInterval(refreshAll, 5000);   // 状态自动巡检

  // 发布记录带过期倒计时，弹窗或页面打开时跟着刷新
  setInterval(() => {
    if (modalOpen("pubListModal") || curPage === "publishes") refreshPublishes();
  }, 5000);

  // 启动后静默查一次更新：延迟到首屏渲染完再发请求，且只有「有新版本」才会
  // 在侧栏「关于」上亮圆点 —— 这里刻意不调 enterAbout()，那个会清掉圆点
  setTimeout(() => {
    if (typeof loadAbout !== "function") return;
    loadAbout().then(() => {
      if (typeof autoCheckOn === "function" && autoCheckOn()) checkUpdate(false);
    });
  }, 2500);
}

boot();

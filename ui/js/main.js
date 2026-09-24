/* =============================================================================
   main.js · 装配与启动
   -----------------------------------------------------------------------------
   跨模块的通用动作在这里注册，最后统一启动。各模块自己的动作注册在各自文件末尾。
   ============================================================================= */

/* 弹窗里所有「选择文件夹」按钮共用：从 data-input / data-prompt 取参数 */
on("pick-folder", el => chooseFolder(el.dataset.input, el.dataset.prompt));

/* 工具栏「刷新」：项目列表 + 端口雷达一起拉 */
on("refresh-all", () => refreshAll());

/* 全屏同步：macOS「原生全屏」下没有红绿灯，顶部要收掉为其预留的高度。
   ⚠️ 不能用后端 is_fullscreen：Tauri 的窗口全屏标志在「铺满屏幕但仍带标题栏」
   的状态下同样是 true，而那时红绿灯还在，收掉内边距会把内容压在红绿灯下。
   ⚠️ 也不能用 outerWidth/outerHeight：WKWebView 里恒为 0。
   可靠信号：原生全屏时窗口盖住整屏（含菜单栏），innerHeight 才会等于 screen.height。 */
let fsTimer = 0;
function syncFullscreen() {
  // 进出全屏的动画期间会连续触发 resize，防抖到最后一帧再判
  clearTimeout(fsTimer);
  fsTimer = setTimeout(() => {
    const nativeFs = Math.abs(window.innerHeight - window.screen.height) <= 1;
    document.documentElement.classList.toggle("is-fullscreen", nativeFs);
  }, 250);
}
window.addEventListener("resize", syncFullscreen);

/* 标题栏拖拽：macOS WKWebView 不支持 CSS 的 `-webkit-app-region: drag`，
   所以这里手动监听 mousedown 调原生 startDragging()。全屏时 .titlebar 高度
   归零，不会收到事件，因此无需额外判断。global Tauri 已开（withGlobalTauri），
   但老构建可能没有 window.__TAURI__.window，做个存在性兜底避免抛错。
   v2 API 是 getCurrentWindow()；v1/兼容别名可能是 getCurrent()，都尝试。 */
function startNativeDrag() {
  const w = window.__TAURI__ && window.__TAURI__.window;
  if (!w) { console.warn("[drag] window.__TAURI__.window 不存在"); return; }
  const getCurrent = w.getCurrentWindow || w.getCurrent;
  if (typeof getCurrent !== "function") {
    console.warn("[drag] 没有 getCurrentWindow/getCurrent", w);
    return;
  }
  // ⚠️ startDragging() 返回 Promise：权限不足时是**被拒绝的 Promise**，
  // 同步 try/catch 抓不到 → 必须 .catch，否则静默失败（踩过）。
  try {
    const p = getCurrent().startDragging();
    if (p && typeof p.catch === "function") {
      p.catch(err => {
        console.error("[drag] startDragging 被拒", err);
        if (typeof toast === "function") toast("拖拽失败：" + err);
      });
    }
  } catch (err) {
    console.error("[drag] 同步异常", err);
    if (typeof toast === "function") toast("拖拽失败：" + err);
  }
}

function bindDragRegion() {
  const bar = document.querySelector(".titlebar");
  if (bar) {
    bar.addEventListener("mousedown", e => { if (e.button === 0) startNativeDrag(); });
  }
  // 左侧 rail 的顶部 padding 区域（红绿灯下方、图标上方）也做成可拖，
  // 否则 28px 的顶条太窄，用户容易在 rail 空白区按住却拖不动。
  const rail = document.getElementById("rail");
  if (rail) {
    const railPadTop = parseFloat(getComputedStyle(rail).paddingTop) || 40;
    rail.addEventListener("mousedown", e => {
      if (e.button !== 0) return;
      const rect = rail.getBoundingClientRect();
      if (e.clientY - rect.top < railPadTop) startNativeDrag();
    });
  }
}

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
  bindDragRegion();
  renderRail();
  goPage("projects");
  refreshAll();
  refreshPublishes();
  syncFullscreen();   // 启动时先对一次全屏状态（可能就是从全屏恢复的）

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

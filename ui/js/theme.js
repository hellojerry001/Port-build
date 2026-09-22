/* =============================================================================
   theme.js · 主题控制器
   -----------------------------------------------------------------------------
   三态：system / light / dark。system 交给系统偏好决定实际外观。
   必须在 <head> 里同步加载：早于 body 渲染就把 data-theme 写好，
   否则会先按深色画一帧再切浅色（闪一下）。

   默认取 dark 而不是 system：VibeButler 一直是深色界面，跟随系统会让
   浅色系统上的老用户一升级就「变白」，那是外观突变。想跟随系统的用户在
   左下角切一次即可，之后就一直记住。

   对外只暴露 window.PBTheme：UI 层（rail.js）订阅它渲染菜单，
   不需要知道 localStorage 键名、系统媒询这些细节。
   ============================================================================= */

(function () {
  var KEY = "pb-theme";
  var MODES = ["system", "light", "dark"];
  var LABEL = { system: "跟随系统", light: "浅色模式", dark: "深色模式" };
  var mq = window.matchMedia("(prefers-color-scheme: dark)");

  var mode = "dark";
  var subs = [];

  function read() {
    try {
      var v = localStorage.getItem(KEY);
      if (MODES.indexOf(v) >= 0) mode = v;
    } catch (e) { /* 隐私模式等禁写场景：用默认值即可 */ }
  }

  /* 当前生效的外观 */
  function resolved() {
    return mode === "system" ? (mq.matches ? "dark" : "light") : mode;
  }

  /* 唯一的写入点：只改 <html data-theme>，CSS 那边全靠第 2 层重绑 */
  function apply() {
    document.documentElement.setAttribute("data-theme", resolved());
  }

  function notify() {
    subs.forEach(function (fn) { fn(mode, resolved()); });
  }

  function set(m) {
    if (MODES.indexOf(m) < 0) return;
    mode = m;
    try { localStorage.setItem(KEY, m); } catch (e) { /* 存不下也不影响本次生效 */ }
    apply();
    notify();
  }

  /* 停留在 system 时，系统外观一变就要跟着变 */
  function onSystemChange() {
    if (mode === "system") { apply(); notify(); }
  }

  if (mq.addEventListener) mq.addEventListener("change", onSystemChange);
  else if (mq.addListener) mq.addListener(onSystemChange);   // 老 WebKit

  read();
  apply();

  window.PBTheme = {
    MODES: MODES,
    LABEL: LABEL,
    mode: function () { return mode; },
    resolved: resolved,
    set: set,
    /* 订阅即回调一次，调用方不用自己先渲染一遍初始态 */
    subscribe: function (fn) { subs.push(fn); fn(mode, resolved()); },
  };
})();

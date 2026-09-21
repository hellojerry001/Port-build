/* =============================================================================
   rail.js · 左侧导航
   -----------------------------------------------------------------------------
   菜单项与页面标题表集中在此。切换靠 .page / .is-active 类，不再写内联 display。
   ============================================================================= */

/* 图标为 20×20 线性图标：1.6 描边 + round 端点，复用 svg 的属性而非重复写在每段里 */
const NAV = [
  {
    key: "projects", label: "项目管理", bot: false,
    icon: '<rect x="3.4" y="3.4" width="5.4" height="5.4" rx="1.5"/><rect x="11.2" y="3.4" width="5.4" height="5.4" rx="1.5"/>' +
          '<rect x="3.4" y="11.2" width="5.4" height="5.4" rx="1.5"/><rect x="11.2" y="11.2" width="5.4" height="5.4" rx="1.5"/>',
  },
  {
    key: "radar", label: "端口雷达", bot: false,
    icon: '<path d="M3.6 8.4a9 9 0 0 1 12.8 0"/><path d="M6.4 11.6a5 5 0 0 1 7.2 0"/>' +
          '<circle cx="10" cy="15.4" r="1.4" fill="currentColor" stroke="none"/>',
  },
  {
    key: "publishes", label: "发布记录", bot: false,
    icon: '<path d="M10 13.4V4.6"/><path d="M6.6 8 10 4.6 13.4 8"/>' +
          '<path d="M4.2 13v1.7a1.8 1.8 0 0 0 1.8 1.8h8a1.8 1.8 0 0 0 1.8-1.8V13"/>',
  },
  {
    key: "templates", label: "模板库", bot: false,
    icon: '<path d="M10 3.6 3.6 6.9 10 10.2l6.4-3.3z"/><path d="M3.6 10.4 10 13.7l6.4-3.3"/>' +
          '<path d="M3.6 13.6 10 16.9l6.4-3.3"/>',
  },
  {
    key: "settings", label: "设置", bot: true,
    icon: '<circle cx="9.5" cy="6.8" r="2.2"/><path d="M4.4 6.8h2.9M11.8 6.8h3.8"/>' +
          '<circle cx="11" cy="13.2" r="2.2"/><path d="M4.4 13.2h4.4M13.3 13.2h2.3"/>',
  },
  {
    // 检测到新版本时，about.js 会给这个按钮加 .has-dot（见 components.css）
    key: "about", label: "关于", bot: true,
    icon: '<circle cx="10" cy="10" r="6.6"/><path d="M10 9.4v4.2"/>' +
          '<circle cx="10" cy="6.9" r=".9" fill="currentColor" stroke="none"/>',
  },
];

/* 页面 key → [标题, 副标题] */
const PAGES = {
  projects:  ["项目管理", "VibeButler · 项目 & 端口管理"],
  radar:     ["端口雷达", "本机监听端口实时一览"],
  publishes: ["发布记录", "Cloudflare 临时发布与认领"],
  templates: ["模板库",   "内置脚手架模板"],
  settings:  ["设置",     "偏好与默认值"],
  about:     ["关于",     "版本信息与更新"],
};

/* 主题三态各一个图标：按钮上用「当前选择」的那个，一眼能看出现在是什么模式 */
const THEME_ICON = {
  system: '<rect x="3.2" y="4.6" width="13.6" height="9" rx="1.6"/>' +
          '<path d="M10 13.6v2.2"/><path d="M6.8 15.8h6.4"/>',
  light:  '<circle cx="10" cy="10" r="3.2"/>' +
          '<path d="M10 2.8v1.7M10 15.5v1.7M2.8 10h1.7M15.5 10h1.7' +
          'M5 5l1.2 1.2M13.8 13.8 15 15M15 5l-1.2 1.2M6.2 13.8 5 15"/>',
  dark:   '<path d="M15.6 11.9A6.4 6.4 0 0 1 8.1 4.4a6.4 6.4 0 1 0 7.5 7.5z"/>',
};

const themeSvg = icon => '<svg viewBox="0 0 20 20">' + icon + "</svg>";

/* 主题入口是 rail 底部的普通按钮，只多了浮层菜单 */
function themeButtonHtml() {
  const m = PBTheme.mode();
  return '<button class="rail-btn" id="themeBtn" data-act="theme-open" data-tip="主题：' +
         esc(PBTheme.LABEL[m]) + '">' + themeSvg(THEME_ICON[m]) + "</button>";
}

function renderRail() {
  const rail = $("rail");
  const top = NAV.filter(n => !n.bot).map(UI.railButton).join("");
  const bot = NAV.filter(n => n.bot).map(UI.railButton).join("");
  rail.innerHTML = top + '<div class="rail-sp"></div>' + bot + themeButtonHtml();
}

/* ------------------------------ 主题浮层 ------------------------------ */

const themeMenuOpen = () => $("themeMenu").classList.contains("is-open");

function renderThemeMenu() {
  const cur = PBTheme.mode();
  $("themeMenu").innerHTML = PBTheme.MODES.map(m =>
    '<button class="' + cls("theme-opt", m === cur && "is-on") + '"' +
    dataAttrs({ act: "theme-pick", mode: m }) + ">" +
      themeSvg(THEME_ICON[m]) +
      "<span>" + esc(PBTheme.LABEL[m]) + "</span>" +
      '<svg class="tick" viewBox="0 0 20 20"><path d="M4.8 10.4 8.4 14l6.8-8"/></svg>' +
    "</button>").join("");
}

function closeThemeMenu() { $("themeMenu").classList.remove("is-open"); }

on("theme-open", () => {
  if (themeMenuOpen()) return closeThemeMenu();
  renderThemeMenu();
  $("themeMenu").classList.add("is-open");
});

on("theme-pick", el => {
  PBTheme.set(el.dataset.mode);
  closeThemeMenu();
});

/* 点浮层和按钮以外的地方、或按 Esc 都收起菜单 */
document.addEventListener("click", e => {
  if (!themeMenuOpen()) return;
  if (e.target.closest("#themeMenu") || e.target.closest("#themeBtn")) return;
  closeThemeMenu();
});

document.addEventListener("keydown", e => {
  if (e.key === "Escape" && themeMenuOpen()) closeThemeMenu();
});

/* 主题一变，按钮图标与菜单选中态都要跟上（订阅时立刻回调一次） */
PBTheme.subscribe(() => {
  const m = PBTheme.mode();
  const btn = $("themeBtn");
  if (btn) {
    btn.dataset.tip = "主题：" + PBTheme.LABEL[m];
    btn.innerHTML = themeSvg(THEME_ICON[m]);
  }
  if (themeMenuOpen()) renderThemeMenu();
});

function goPage(key) {
  if (!PAGES[key]) key = "projects";
  curPage = key;
  $$(".rail-btn").forEach(b => b.classList.toggle("is-on", b.dataset.page === key));
  Object.keys(PAGES).forEach(k => {
    const el = $("page-" + k);
    if (el) el.classList.toggle("is-active", k === key);
  });
  $("pgTitle").textContent = PAGES[key][0];
  $("pgSub").textContent = PAGES[key][1];
  if (key === "publishes") refreshPublishes();
  // 进关于页时补数据并清掉提示圆点（about.js 定义；用 typeof 挡住加载顺序问题）
  if (key === "about" && typeof enterAbout === "function") enterAbout();
  document.querySelector(".app-main").scrollTop = 0;
}

on("nav", el => goPage(el.dataset.page));

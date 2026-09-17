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
];

/* 页面 key → [标题, 副标题] */
const PAGES = {
  projects:  ["项目管理", "PortButler · 项目 & 端口管理"],
  radar:     ["端口雷达", "本机监听端口实时一览"],
  publishes: ["发布记录", "Cloudflare 临时发布与认领"],
  templates: ["模板库",   "内置脚手架模板"],
  settings:  ["设置",     "偏好与默认值"],
};

function renderRail() {
  const rail = $("rail");
  const top = NAV.filter(n => !n.bot).map(UI.railButton).join("");
  const bot = NAV.filter(n => n.bot).map(UI.railButton).join("");
  rail.innerHTML = top + '<div class="rail-sp"></div>' + bot;
}

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
  document.querySelector(".app-main").scrollTop = 0;
}

on("nav", el => goPage(el.dataset.page));

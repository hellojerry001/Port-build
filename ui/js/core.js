/* =============================================================================
   core.js · 基础设施
   -----------------------------------------------------------------------------
   1) 共享状态：全应用只在这里声明，模块之间通过它协作
   2) DOM / 转义 / 提示 / 剪贴板 / 外链 等通用函数
   3) 事件委托：把 34 处内联 onclick 收敛成「标记 + 注册表」
      - data-act       → click
      - data-oninput   → input
      - data-onchange  → change
      好处：不再往 window 上挂业务函数；动态拼接 HTML 时只写 data-*，
           不必操心引号转义；新增交互不需要动 index.html。
   ============================================================================= */

/* ------------------------------ 共享状态 ------------------------------ */
let projects = [];            // 项目列表
let busyPorts = new Map();    // port -> 端口占用信息
let runningIds = new Set();   // 由本应用拉起、且进程组仍存活的项目 id
let editingId = "";           // 手动添加/编辑弹窗当前编辑的项目 id

let scaffolds = [];           // 模板列表
let tplPicked = "";           // 当前选中的模板 key
let tplBusy = false;

let addSrc = "";              // 「新增模板」选中的来源（目录 / zip 路径）
let addProbe = null;          // 来源体检结果（probe_scaffold_source）
let addBusy = false;

let pubId = "";               // 发布弹窗当前项目 id
let pubProbe = null;          // 发布目录探测结果
let publishes = [];           // 发布记录

let delId = "", delBusy = false;
let curPage = "projects";

/* ------------------------------ DOM ------------------------------ */
const $ = id => document.getElementById(id);
const $$ = (sel, root) => Array.prototype.slice.call((root || document).querySelectorAll(sel));
const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);

/* 拼 HTML 时的统一转义 */
const esc = s => String(s).replace(/[&<>"]/g, c =>
  ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

/* 把对象转成 data-* 属性串，值统一转义 */
function dataAttrs(map) {
  let out = "";
  Object.keys(map || {}).forEach(k => {
    const v = map[k];
    if (v === undefined || v === null || v === false) return;
    out += " data-" + k + '="' + esc(v) + '"';
  });
  return out;
}

/* ------------------------------ 弹窗 ------------------------------ */
const openModal  = id => $(id).classList.add("is-open");
const closeModal = id => $(id).classList.remove("is-open");
const modalOpen  = id => $(id).classList.contains("is-open");

/* ------------------------------ 提示 ------------------------------ */
function toast(msg) {
  const t = $("toast");
  t.textContent = msg;
  t.classList.add("is-open");
  clearTimeout(t._h);
  t._h = setTimeout(() => t.classList.remove("is-open"), 3200);
}

/* ------------------------------ 剪贴板 / 外链 ------------------------------ */
function copyVal(v) {
  (navigator.clipboard ? navigator.clipboard.writeText(v) : Promise.reject())
    .then(() => toast("已复制"))
    .catch(() => toast("复制失败，请手动复制"));
}

function copyInputValue(id) {
  const el = $(id);
  if (el) copyVal(el.value);
}

function openLink(u) {
  invoke("open_url", { url: u }).catch(() => {
    try { window.open(u, "_blank"); } catch (_) {}
  });
}

function openBrowser(port) {
  invoke("open_url", { url: "http://localhost:" + port }).catch(e => toast(String(e)));
}

/* 在访达中选中文件 / 文件夹。失败直接弹 toast，调用方不必自己包 try/catch */
function showInFinder(path) {
  invoke("show_in_finder", { path }).catch(e => toast(String(e)));
}

/* ------------------------------ 调起原生目录选择器 ------------------------------ */
/* 选中后回填到指定输入框；用户取消（返回 null）则保持原值 */
async function chooseFolder(inputId, prompt) {
  const el = $(inputId);
  if (!el) return;
  el.blur();   // 先失焦，避免系统弹窗抢焦点时输入法状态残留
  try {
    const picked = await invoke("pick_folder", { prompt, defaultPath: el.value.trim() });
    if (picked) el.value = picked;
  } catch (e) {
    toast(String(e));
  }
}

/* ------------------------------ 事件委托 ------------------------------ */
const HANDLERS = { click: {}, input: {}, change: {} };

const on       = (name, fn) => { HANDLERS.click[name]  = fn; };
const onInput  = (name, fn) => { HANDLERS.input[name]  = fn; };
const onChange = (name, fn) => { HANDLERS.change[name] = fn; };

const DELEGATES = [
  ["click",  "data-act"],
  ["input",  "data-oninput"],
  ["change", "data-onchange"],
];

function bindDelegates() {
  DELEGATES.forEach(([type, attr]) => {
    document.addEventListener(type, e => {
      const t = e.target;
      const el = t && t.closest ? t.closest("[" + attr + "]") : null;
      if (!el) return;
      const fn = HANDLERS[type][el.getAttribute(attr)];
      if (fn) fn(el, e);
    });
  });
}

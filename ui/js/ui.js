/* =============================================================================
   ui.js · 渲染工厂
   -----------------------------------------------------------------------------
   把原先散落在各处的 HTML 字符串拼接收敛成一组显式组件。约定：
     · 工厂只产出 HTML 字符串，事件一律写成 data-act / data-oninput 交给委托层
     · 插值走 esc()，属性走 dataAttrs()，杜绝手写引号拼接
     · 类名拼装统一走 cls()，不在业务代码里手写 "btn is-ghost" 这类字符串
   静态结构（弹窗骨架）留在 index.html，这里只负责动态渲染出来的部分。
   ============================================================================= */

/* 类名拼装：过滤空值，支持 `cond && "is-on"` 这种条件项 */
const cls = (...parts) => parts.filter(Boolean).join(" ");

/* 通用空态图标：用户提供的「文件夹」图标（34×28）。它是**填充型**字形
   （fill + fill-rule:evenodd 画出 3 单位粗的描边效果），不是 stroke 图标 ——
   所以必须显式 fill="currentColor"，否则会吃 SVG 默认的黑色，深色主题下不可见。
   currentColor 由 .empty-icon 的 color 提供，随主题变。 */
const EMPTY_ICON = '<svg viewBox="0 0 34 28" aria-hidden="true"><path fill="currentColor" fill-rule="evenodd" d="M19.98197,5.0307455Q20.677975,5.6123023,21.584967,5.6122975L28.499977,5.6122642Q30.77816,5.6122532,32.38908,7.2231679Q34,8.8340816,34,11.112264L34,22.5Q34,24.778172,32.389084,26.389086Q30.778172,28,28.5,28L5.5000005,28Q3.221827,28,1.6109128,26.389086Q0,24.778173,0,22.5L0,5.5000005Q0,3.2218256,1.6109128,1.6109128Q3.2218258,0,5.5000005,0L11.965818,0Q13.961182,0,15.492383,1.2794141L19.98197,5.0307455ZM18.05839,7.3328834L13.568803,3.5815516Q12.872802,3,11.965818,3L5.5000005,3Q3,3,3,5.5000005L3,22.5Q3,25,5.5000005,25L28.5,25Q31,25,31,22.5L31,11.112264Q31,10.076725,30.267765,9.344492Q29.535528,8.6122589,28.499992,8.6122646L21.584982,8.6122971Q19.589602,8.6123075,18.05839,7.3328834Z"/></svg>';

const UI = {

  /* ------------------------------ 按钮 ------------------------------ */
  /* label 会被转义；需要富文本时改用 html（调用方自行保证安全）
     o: { variant, act, data, id, title, disabled, push, cls } */
  btn(label, o = {}) {
    const attrs =
      dataAttrs(Object.assign({ act: o.act }, o.data)) +
      (o.id ? ' id="' + esc(o.id) + '"' : "") +
      (o.title ? ' title="' + esc(o.title) + '"' : "") +
      (o.disabled ? " disabled" : "");
    return '<button class="' + cls("btn", o.variant && "is-" + o.variant, o.push && "push-end", o.cls) +
           '"' + attrs + ">" + (o.html !== undefined ? o.html : esc(label)) + "</button>";
  },

  /* ------------------------------ 状态 ------------------------------ */
  /* 空态。span：作为栅格子项时通栏；compact：紧凑纵向内边距 */
  /* 简单旧版：仅用于加载、错误提示等不需要图标标题结构的场景 */
  empty(html, o = {}) {
    return '<div class="' + cls("empty-state", o.span && "span-all", o.compact && "is-compact") + '">' +
           html + "</div>";
  },

  /* 结构化空态的内部内容：图标 + 标题 + 说明（参考用户提供的「文档折角」图标风格）
     不含外层容器，供 .empty-state 容器（div / td 等）直接放入。
     o: { title, desc, icon } */
  emptyStateInner(o = {}) {
    const icon = o.icon || EMPTY_ICON;
    let html = '<div class="empty-icon">' + icon + '</div>';
    if (o.title) html += '<div class="empty-title">' + esc(o.title) + '</div>';
    if (o.desc)  html += '<div class="empty-desc">' + esc(o.desc) + '</div>';
    return html;
  },

  /* 结构化空态：图标 + 标题 + 说明（参考用户提供的「文档折角」图标风格）
     o: { title, desc, icon, span, compact } */
  emptyState(o = {}) {
    return '<div class="' + cls("empty-state", o.span && "span-all", o.compact && "is-compact") + '">' +
           UI.emptyStateInner(o) + '</div>';
  },

  /* 加载态：转圈 + 文案 */
  spinner(text) {
    return '<span class="spinner"></span>' + esc(text);
  },

  /* 运行状态点 */
  dot(isOn) {
    return '<span class="' + cls("dot", isOn && "is-on") + '"></span>';
  },

  /* 状态徽标（发布记录：剩余时长 / 已过期 / 长期有效） */
  badge(text, variant) {
    return '<span class="' + cls("pl-badge", variant && "is-" + variant) + '">' + esc(text) + "</span>";
  },

  /* chips 内容，不含容器（容器写在 index.html 里）
     items: [{text, sub, act, data, on, title}] */
  chipList(items) {
    return (items || []).map(c =>
      '<button class="' + cls("chip", c.on && "is-on") + '"' +
      dataAttrs(Object.assign({ act: c.act, title: c.title }, c.data)) + ">" +
      esc(c.text) + (c.sub ? "<small>" + esc(c.sub) + "</small>" : "") +
      "</button>").join("");
  },

  /* ------------------------------ 导航 ------------------------------ */
  /* svg 的描边/填充属性由 .rail-btn svg 统一在样式层声明 */
  railButton(item) {
    return '<button class="rail-btn" data-page="' + esc(item.key) + '" data-tip="' + esc(item.label) +
           '" data-act="nav"><svg viewBox="0 0 20 20">' + item.icon + "</svg></button>";
  },
};

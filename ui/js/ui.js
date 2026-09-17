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
  empty(html, o = {}) {
    return '<div class="' + cls("empty-state", o.span && "span-all", o.compact && "is-compact") + '">' +
           html + "</div>";
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

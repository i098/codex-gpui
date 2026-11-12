// Evaluated inside the original app page. Dumps the rendered box tree as JSON: every visible
// element with its rect and the computed styles that matter for pixels, plus every text run
// with its glyph box. This is the build spec for a screen and the text mask for the parity gate.
(() => {
  const keep = [
    "display", "position", "flexDirection", "alignItems", "justifyContent", "gap",
    "paddingTop", "paddingRight", "paddingBottom", "paddingLeft",
    "backgroundColor", "backgroundImage", "color", "opacity",
    "borderTopWidth", "borderRightWidth", "borderBottomWidth", "borderLeftWidth",
    "borderTopColor", "borderRightColor", "borderBottomColor", "borderLeftColor",
    "borderTopLeftRadius", "borderTopRightRadius", "borderBottomRightRadius", "borderBottomLeftRadius",
    "boxShadow", "fontFamily", "fontSize", "fontWeight", "lineHeight", "letterSpacing",
    "textAlign", "whiteSpace", "overflow", "backdropFilter", "filter", "cursor", "zIndex",
  ];
  const defaults = {};
  const probe = document.createElement("div");
  document.body.appendChild(probe);
  const pcs = getComputedStyle(probe);
  for (const k of keep) defaults[k] = pcs[k];
  probe.remove();
  const r4 = (r) => [r.x, r.y, r.width, r.height].map((v) => Math.round(v * 100) / 100);
  const nodes = [];
  const texts = [];
  const visit = (el, parent, depth) => {
    const cs = getComputedStyle(el);
    if (cs.display === "none" || cs.visibility === "hidden" || +cs.opacity === 0) return;
    const rect = el.getBoundingClientRect();
    const id = nodes.length;
    const style = {};
    for (const k of keep) if (cs[k] !== defaults[k] || k === "fontSize" || k === "color") style[k] = cs[k];
    const node = { id, parent, depth, tag: el.tagName.toLowerCase(), rect: r4(rect), style };
    const label = el.getAttribute("aria-label");
    if (label) node.label = label;
    if (el.classList.length) node.cls = el.getAttribute("class");
    if (el.tagName === "svg") node.svg = el.outerHTML;
    if (el.tagName === "IMG") node.src = (el.currentSrc || el.src).slice(0, 200);
    if (rect.width > 0 && rect.height > 0) nodes.push(node);
    else if (el.tagName === "svg") return;
    if (el.tagName === "svg") return;
    // An input's value has no text node: measure it in a copy of the field (same box and
    // styles) laid out the way Blink lays out the field's inner editor.
    const field = el.tagName === "TEXTAREA" || (el.tagName === "INPUT" && /^(text|search|email|url|tel|number)$/.test(el.type));
    if (field && el.value.trim() && rect.width > 0 && rect.height > 0) {
      const copy = document.createElement("div");
      for (const k of cs) copy.style.setProperty(k, cs.getPropertyValue(k));
      Object.assign(copy.style, { position: "fixed", left: `${rect.x}px`, top: `${rect.y}px`, margin: "0", boxSizing: "border-box", width: `${rect.width}px`, height: `${rect.height}px`, overflow: "hidden" });
      if (el.tagName === "INPUT") Object.assign(copy.style, { display: "flex", alignItems: "center", whiteSpace: "pre" });
      else Object.assign(copy.style, { display: "block", whiteSpace: "pre-wrap" });
      const span = document.createElement("span");
      span.textContent = el.value;
      copy.append(span);
      document.body.append(copy);
      const range = document.createRange();
      range.selectNodeContents(span);
      for (const tr of range.getClientRects())
        if (tr.width && tr.height) texts.push({ node: id, text: el.value.trim().slice(0, 200), rect: r4(new DOMRect(tr.x - el.scrollLeft, tr.y - el.scrollTop, tr.width, tr.height)), font: `${cs.fontWeight} ${cs.fontSize}/${cs.lineHeight} ${cs.fontFamily}`, color: cs.color });
      copy.remove();
    }
    for (const c of el.childNodes) {
      if (c.nodeType === 3 && c.textContent.trim()) {
        const range = document.createRange();
        range.selectNodeContents(c);
        for (const tr of range.getClientRects())
          if (tr.width && tr.height) texts.push({ node: id, text: c.textContent.trim().slice(0, 200), rect: r4(tr), font: `${cs.fontWeight} ${cs.fontSize}/${cs.lineHeight} ${cs.fontFamily}`, color: cs.color });
      } else if (c.nodeType === 1) visit(c, id, depth + 1);
    }
  };
  visit(document.body, -1, 0);
  return JSON.stringify({
    viewport: [innerWidth, innerHeight, devicePixelRatio],
    htmlClass: document.documentElement.className,
    rootVars: Object.fromEntries([...getComputedStyle(document.documentElement)].filter((p) => p.startsWith("--")).map((p) => [p, getComputedStyle(document.documentElement).getPropertyValue(p).trim()])),
    nodes,
    texts,
  });
})()

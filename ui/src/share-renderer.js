import { highlight_root } from "./highlight.js";

// --- /share long-image renderer -------------------------------------------
// PNG and HTML both follow the live chat CSS (paper theme tokens, user
// panel bubble, assistant prose — not a separate blue-bubble skin).

const SHARE_WIDTH = 840;
const SHARE_MIN_WIDTH = 320;
const SHARE_MAX_WIDTH = 2400;
const SHARE_SCALE = 2;
const SHARE_PAD = 24;
const SHARE_GAP = 20;
const SHARE_BUBBLE_PAD_X = 15;
const SHARE_BUBBLE_PAD_Y = 10;
const SHARE_LINE_HEIGHT = 22;

const SHARE_THEME_DEFAULT = {
  bg: "#faf9f6",
  elev: "#ffffff",
  sunken: "#f3f1ec",
  panel: "#f0eee6",
  text: "#141413",
  muted: "#5a574e",
  faint: "#706d65",
  border: "rgba(60, 55, 45, 0.10)",
  borderStrong: "#d6d4cc",
  clay: "#0d9488",
  clayStrong: "#0f766e",
  fontUi: 'Inter, "PingFang SC", "Microsoft YaHei", system-ui, sans-serif',
  fontResponse: '"Source Serif 4", "Noto Serif SC", Georgia, serif',
  fontMono: '"JetBrains Mono", ui-monospace, Consolas, monospace',
  fontSize: 14,
  lang: "en",
  colorScheme: "light",
};

let SHARE_THEME = { ...SHARE_THEME_DEFAULT };
let SHARE_SANS = SHARE_THEME.fontUi;
let SHARE_SERIF = SHARE_THEME.fontResponse;
let SHARE_MONO = SHARE_THEME.fontMono;
let SHARE_LABEL_FONT = `11px ${SHARE_SANS}`;
let SHARE_COLORS = shareColorsFromTheme(SHARE_THEME);

function shareColorsFromTheme(theme) {
  return {
    bg: theme.bg,
    card: theme.elev,
    border: theme.border,
    borderStrong: theme.borderStrong,
    text: theme.text,
    muted: theme.muted,
    faint: theme.faint,
    accent: theme.clay,
    accentStrong: theme.clayStrong,
    codeBg: theme.sunken,
    codeChip: theme.sunken,
    quoteBar: theme.borderStrong,
    panel: theme.panel,
  };
}

function applyShareTheme(theme) {
  SHARE_THEME = theme;
  SHARE_SANS = theme.fontUi;
  SHARE_SERIF = theme.fontResponse;
  SHARE_MONO = theme.fontMono;
  SHARE_LABEL_FONT = `600 11px ${SHARE_SANS}`;
  SHARE_COLORS = shareColorsFromTheme(theme);
}

function resolveCssColor(probe, token) {
  probe.style.color = "";
  probe.style.backgroundColor = `var(${token})`;
  const bg = getComputedStyle(probe).backgroundColor;
  if (bg && bg !== "rgba(0, 0, 0, 0)" && bg !== "transparent") return bg;
  probe.style.backgroundColor = "";
  probe.style.color = `var(${token})`;
  return getComputedStyle(probe).color;
}

function resolveLiveShareTheme() {
  const theme = { ...SHARE_THEME_DEFAULT };
  if (typeof document === "undefined" || !document.documentElement) return theme;
  const probe = document.createElement("div");
  probe.style.cssText = "position:fixed;left:-9999px;top:0;pointer-events:none;";
  document.body.appendChild(probe);
  const family = (token) => {
    probe.style.fontFamily = `var(${token})`;
    return getComputedStyle(probe).fontFamily || theme.fontUi;
  };
  try {
    theme.bg = resolveCssColor(probe, "--bg-app") || theme.bg;
    theme.elev = resolveCssColor(probe, "--bg-elev") || theme.elev;
    theme.sunken = resolveCssColor(probe, "--bg-sunken") || theme.sunken;
    theme.panel = resolveCssColor(probe, "--bg-panel") || theme.panel;
    theme.text = resolveCssColor(probe, "--text") || theme.text;
    theme.muted = resolveCssColor(probe, "--text-muted") || theme.muted;
    theme.faint = resolveCssColor(probe, "--text-faint") || theme.faint;
    theme.border = resolveCssColor(probe, "--border") || theme.border;
    theme.borderStrong = resolveCssColor(probe, "--border-strong") || theme.borderStrong;
    theme.clay = resolveCssColor(probe, "--clay") || theme.clay;
    theme.clayStrong = resolveCssColor(probe, "--clay-strong") || theme.clayStrong;
    theme.fontUi = family("--font-ui");
    theme.fontResponse = family("--font-response");
    theme.fontMono = family("--font-mono");
    probe.style.fontSize = "var(--ui-font-size, 14px)";
    theme.fontSize = parseFloat(getComputedStyle(probe).fontSize) || 14;
    theme.lang = document.documentElement.lang || "en";
    theme.colorScheme = getComputedStyle(document.documentElement).colorScheme || "light";
  } catch {
    // Keep paper defaults when computed styles are unavailable.
  }
  probe.remove();
  return theme;
}

function collectShareRootCss(theme) {
  return [
    `--bg-app: ${theme.bg}`,
    `--bg-elev: ${theme.elev}`,
    `--bg-sunken: ${theme.sunken}`,
    `--bg-panel: ${theme.panel}`,
    `--text: ${theme.text}`,
    `--text-muted: ${theme.muted}`,
    `--text-faint: ${theme.faint}`,
    `--border: ${theme.border}`,
    `--border-strong: ${theme.borderStrong}`,
    `--clay: ${theme.clay}`,
    `--clay-strong: ${theme.clayStrong}`,
    `--font-ui: ${theme.fontUi}`,
    `--font-sans: ${theme.fontUi}`,
    `--font-response: ${theme.fontResponse}`,
    `--font-mono: ${theme.fontMono}`,
    `--ui-font-size: ${theme.fontSize}px`,
    `color-scheme: ${theme.colorScheme}`,
  ].join("; ");
}

const SHARE_RULE_KEEP = /(^|[, ])(\.msg|\.thread\b|\.md\b|\.body\.md|\.user-bubble|\.assistant-wrap|\.role-brand|:lang\(zh\))/;
const SHARE_RULE_SKIP = /\.composer|\.center\b|\.tool\b|\.follow-up|\.msg-actions|\.message-artifact|\.user-attachment|\.plan-|\.exploration|\.conversation-outline|\.inbox|\.empty\b|\.topbar|\.chat-jump|\.transcript-|\.role::before/;

function shareRuleWanted(selector) {
  if (!selector) return false;
  if (SHARE_RULE_SKIP.test(selector)) return false;
  return SHARE_RULE_KEEP.test(selector);
}

function collectShareStylesheet() {
  const out = [];
  const seen = new Set();
  const styleRule = (typeof CSSRule !== "undefined" && CSSRule.STYLE_RULE) || 1;
  const walk = (rules) => {
    if (!rules) return;
    for (const rule of rules) {
      if (rule.type === styleRule && shareRuleWanted(rule.selectorText)) {
        const text = String(rule.cssText || "").replace(/url\(["']?logo\.svg["']?\)/g, "none");
        if (text && !seen.has(text)) {
          seen.add(text);
          out.push(text);
        }
      }
    }
  };
  for (const sheet of document.styleSheets) {
    try {
      walk(sheet.cssRules);
    } catch {
      // Cross-origin or unloaded sheets are skipped.
    }
  }
  return out.join("\n");
}

/** Frozen tokens + harvested chat/md rules for a WYSIWYG HTML export. */
export function snapshot_share_theme() {
  const theme = resolveLiveShareTheme();
  return JSON.stringify({
    lang: theme.lang,
    root_css: collectShareRootCss(theme),
    harvested_css: collectShareStylesheet(),
  });
}

/** Canvas font for a styled run: {b: bold, i: italic, c: code, a: link}. */
function shareFont(style, size, family = SHARE_SANS) {
  const stack = style.c ? SHARE_MONO : family;
  const px = style.c ? size - 1.5 : size;
  return `${style.i ? "italic " : ""}${style.b ? "600 " : ""}${px}px ${stack}`;
}

/**
 * Greedy wrap of styled runs into lines of segments
 * ({text, style, font, w}), measuring with each run's own font. Breaks at
 * the last space when one fits, else mid-run (CJK has no spaces); "\n"
 * forces a line break.
 */
function wrapShareRuns(ctx, runs, maxWidth, size, family = SHARE_SANS) {
  const lines = [];
  let line = [];
  let lineW = 0;
  let lastBreak = null; // {seg, char} — position of the last breakable space
  for (const raw of runs) {
    const style = { b: !!raw.b, i: !!raw.i, c: !!raw.c, a: !!raw.a };
    const font = shareFont(style, size, family);
    const key = font + (style.a ? "a" : "");
    ctx.font = font;
    for (const ch of String(raw.text ?? "")) {
      if (ch === "\n") {
        lines.push(line);
        line = [];
        lineW = 0;
        lastBreak = null;
        continue;
      }
      if (!line.length && ch === " ") continue; // no leading spaces
      const w = ctx.measureText(ch).width;
      if (lineW + w > maxWidth && line.length) {
        if (lastBreak) {
          // Rewind to the last space: keep what precedes it, move the rest
          // (plus everything after that segment) to the next line.
          const head = line.slice(0, lastBreak.seg + 1);
          const seg = head[lastBreak.seg];
          const keep = seg.text.slice(0, lastBreak.char);
          const rest = seg.text.slice(lastBreak.char + 1);
          const tail = [];
          if (rest) tail.push({ ...seg, text: rest, w: ctx.measureText(rest).width });
          tail.push(...line.slice(lastBreak.seg + 1));
          if (keep) {
            seg.text = keep;
            seg.w = ctx.measureText(keep).width;
          } else {
            head.pop();
          }
          lines.push(head);
          line = tail;
          lineW = tail.reduce((sum, s) => sum + s.w, 0);
          lastBreak = null;
        } else {
          lines.push(line);
          line = [];
          lineW = 0;
        }
        if (ch === " ") continue; // swallow the space at the break point
      }
      let seg = line[line.length - 1];
      if (!seg || seg.key !== key) {
        seg = { text: "", style, font, key, w: 0 };
        line.push(seg);
      }
      seg.text += ch;
      seg.w += w;
      lineW += w;
      if (ch === " ") lastBreak = { seg: line.length - 1, char: seg.text.length - 1 };
    }
  }
  lines.push(line);
  return lines;
}

const shareLineWidth = (line) => line.reduce((sum, seg) => sum + seg.w, 0);

/** Draw one wrapped line at baseline y. Inline-code runs get a chip behind
 * them unless `chips` is false (inside code blocks everything is code). */
function drawShareLine(ctx, line, x, baselineY, color, lineHeight, chips) {
  let cx = x;
  for (const seg of line) {
    if (chips && seg.style.c) {
      ctx.fillStyle = SHARE_COLORS.codeChip;
      shareRoundRect(ctx, cx - 3, baselineY - lineHeight * 0.72, seg.w + 6, lineHeight * 0.82, 4);
      ctx.fill();
    }
    ctx.font = seg.font;
    ctx.fillStyle = seg.style.a ? SHARE_COLORS.accentStrong : color;
    ctx.fillText(seg.text, cx, baselineY);
    cx += seg.w;
  }
}

function shareBlockContentHeight(block) {
  if (block.t === "code") return block.height;
  if (block.t === "hr") return 1;
  return block.lines.length * block.lh;
}

/** Lay out parsed Markdown blocks (see share_markdown_blocks in Rust) into
 * wrapped lines with per-block metrics, for a text column of `textWidth`. */
function layoutShareBlocks(ctx, blocks, textWidth, family = SHARE_SERIF) {
  const bodySize = SHARE_THEME.fontSize + (SHARE_THEME.lang.startsWith("zh") ? 1.5 : 1);
  const bodyLh = Math.round(bodySize * (SHARE_THEME.lang.startsWith("zh") ? 1.78 : 1.62));
  const laid = [];
  for (const block of Array.isArray(blocks) ? blocks : []) {
    if (block.t === "h") {
      const level = Math.min(3, Math.max(1, Number(block.level) || 3));
      const size = [bodySize * 1.55, bodySize * 1.28, bodySize * 1.12][level - 1];
      const runs = (block.runs || []).map((run) => ({ ...run, b: true }));
      laid.push({
        t: "h",
        lines: wrapShareRuns(ctx, runs.length ? runs : [{ text: "" }], textWidth, size, family),
        lh: Math.round(size * 1.35),
        before: laid.length ? 10 : 0,
        after: 4,
      });
    } else if (block.t === "li") {
      const depth = Math.min(4, Number(block.depth) || 0);
      const indent = depth * 18;
      const prefix = block.ordered ? `${Number(block.index) || 1}.` : "•";
      ctx.font = shareFont({}, bodySize, family);
      const offset = indent + (block.ordered ? Math.ceil(ctx.measureText(prefix).width) + 8 : 20);
      laid.push({
        t: "li",
        prefix,
        indent,
        offset,
        lines: wrapShareRuns(ctx, block.runs || [], textWidth - offset, bodySize, family),
        lh: bodyLh,
        before: 0,
        after: 3,
      });
    } else if (block.t === "code") {
      const pad = 11;
      const monoWidth = textWidth - pad * 2;
      const lines = String(block.text || "").split("\n")
        .flatMap((src) => wrapShareRuns(ctx, [{ text: src, c: true }], monoWidth, 12.5, family));
      laid.push({
        t: "code",
        lines,
        lh: 19,
        pad,
        height: lines.length * 19 + pad * 2,
        before: 0,
        after: 8,
      });
    } else if (block.t === "hr") {
      laid.push({ t: "hr", before: 6, after: 12 });
    } else {
      // Paragraph (default); quote paragraphs indent behind a side bar.
      const quote = !!block.quote;
      laid.push({
        t: quote ? "quote" : "p",
        lines: wrapShareRuns(ctx, block.runs || [], textWidth - (quote ? 14 : 0), bodySize, family),
        lh: bodyLh,
        before: 0,
        after: 8,
      });
    }
  }
  return laid;
}

/** Draw laid-out blocks starting at (x, y); returns the consumed height. */
function drawShareBlocks(ctx, blocks, x, y, width) {
  let cy = y;
  for (const block of blocks) {
    cy += block.before;
    if (block.t === "hr") {
      ctx.fillStyle = SHARE_COLORS.border;
      ctx.fillRect(x, cy, width, 1);
      cy += 1 + block.after;
      continue;
    }
    if (block.t === "code") {
      ctx.fillStyle = SHARE_COLORS.codeBg;
      ctx.strokeStyle = SHARE_COLORS.border;
      shareRoundRect(ctx, x, cy, width, block.height, 10);
      ctx.fill();
      ctx.stroke();
      let ty = cy + block.pad + 13;
      for (const line of block.lines) {
        drawShareLine(ctx, line, x + block.pad, ty, SHARE_COLORS.text, block.lh, false);
        ty += block.lh;
      }
      cy += block.height + block.after;
      continue;
    }
    const quote = block.t === "quote";
    if (quote) {
      ctx.fillStyle = SHARE_COLORS.quoteBar;
      shareRoundRect(ctx, x, cy, 3, block.lines.length * block.lh, 1.5);
      ctx.fill();
    }
    const color = quote ? SHARE_COLORS.muted : SHARE_COLORS.text;
    let ty = cy + Math.round(block.lh * 0.72);
    block.lines.forEach((line, i) => {
      if (block.t === "li") {
        if (i === 0) {
          ctx.font = shareFont({}, SHARE_THEME.fontSize + 1, SHARE_SERIF);
          ctx.fillStyle = SHARE_COLORS.muted;
          ctx.fillText(block.prefix, x + block.indent, ty);
        }
        drawShareLine(ctx, line, x + block.offset, ty, color, block.lh, true);
      } else {
        drawShareLine(ctx, line, quote ? x + 14 : x, ty, color, block.lh, true);
      }
      ty += block.lh;
    });
    cy += block.lines.length * block.lh + block.after;
  }
  return cy - y;
}

/** Wrap plain (non-Markdown) bubble text and report its shrink-to-fit width. */
function shareLayoutPlain(ctx, text, maxTextWidth, italic, size = 15) {
  const lines = wrapShareRuns(ctx, [{ text, i: italic }], maxTextWidth, size, SHARE_SANS);
  const width = Math.ceil(Math.max(40, ...lines.map(shareLineWidth)));
  return { lines, width };
}

function shareRoundRect(ctx, x, y, w, h, r) {
  ctx.beginPath();
  if (ctx.roundRect) {
    ctx.roundRect(x, y, w, h, r);
  } else {
    ctx.rect(x, y, w, h);
  }
}

function shareCssTransparent(color) {
  return !color || color === "transparent" || color === "rgba(0, 0, 0, 0)";
}

function shareComputedFont(style) {
  if (style.font && style.font !== "0px") return style.font;
  return `${style.fontStyle || "normal"} ${style.fontWeight || "400"} ${style.fontSize || "14px"} ${style.fontFamily || SHARE_SERIF}`;
}

/** Paint one element's background and border using already-resolved styles. */
function paintShareBox(ctx, el, style, ox, oy) {
  const rect = el.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) return;
  const x = rect.left - ox;
  const y = rect.top - oy;
  const w = rect.width;
  const h = rect.height;
  const radius = Math.min(
    parseFloat(style.borderTopLeftRadius) || 0,
    w / 2,
    h / 2,
  );
  const bg = style.backgroundColor;
  if (!shareCssTransparent(bg)) {
    ctx.fillStyle = bg;
    shareRoundRect(ctx, x, y, w, h, radius);
    ctx.fill();
  }
  const sides = ["Top", "Right", "Bottom", "Left"];
  const widths = sides.map((side) => parseFloat(style[`border${side}Width`]) || 0);
  const styles = sides.map((side) => style[`border${side}Style`]);
  const colors = sides.map((side) => style[`border${side}Color`]);
  const painted = widths.some((width, i) => width > 0 && styles[i] !== "none");
  if (!painted) return;
  const uniform = widths.every((width) => width === widths[0])
    && styles.every((value) => value === styles[0])
    && colors.every((value) => value === colors[0]);
  if (uniform) {
    ctx.strokeStyle = colors[0];
    ctx.lineWidth = widths[0];
    shareRoundRect(
      ctx,
      x + widths[0] / 2,
      y + widths[0] / 2,
      Math.max(0, w - widths[0]),
      Math.max(0, h - widths[0]),
      Math.max(0, radius - widths[0] / 2),
    );
    ctx.stroke();
    return;
  }
  const edges = [
    [x, y + widths[0] / 2, x + w, y + widths[0] / 2, 0],
    [x + w - widths[1] / 2, y, x + w - widths[1] / 2, y + h, 1],
    [x, y + h - widths[2] / 2, x + w, y + h - widths[2] / 2, 2],
    [x + widths[3] / 2, y, x + widths[3] / 2, y + h, 3],
  ];
  for (const [x1, y1, x2, y2, i] of edges) {
    if (widths[i] <= 0 || styles[i] === "none") continue;
    ctx.strokeStyle = colors[i];
    ctx.lineWidth = widths[i];
    ctx.beginPath();
    ctx.moveTo(x1, y1);
    ctx.lineTo(x2, y2);
    ctx.stroke();
  }
}

/** Draw a list marker to the left of the first line; ::marker is not in the DOM. */
function paintShareListMarker(ctx, el, style, ox, oy) {
  if (el.tagName !== "LI") return;
  const type = style.listStyleType;
  if (!type || type === "none") return;
  const marker = getComputedStyle(el, "::marker");
  let first = null;
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) {
    const [line] = shareTextLines(walker.currentNode);
    if (!line) continue;
    first = line;
    break;
  }
  if (!first || first.height <= 0) first = el.getBoundingClientRect();
  ctx.fillStyle = marker.color || style.color;
  if (type === "disc" || type === "circle") {
    ctx.beginPath();
    ctx.arc(el.getBoundingClientRect().left - ox - 10, first.top - oy + first.height * 0.45, 2.2, 0, Math.PI * 2);
    if (type === "circle") {
      ctx.lineWidth = 1;
      ctx.strokeStyle = ctx.fillStyle;
      ctx.stroke();
    } else {
      ctx.fill();
    }
    return;
  }
  if (type !== "decimal") return;
  const list = el.parentElement;
  const items = list ? [...list.children].filter((child) => child.tagName === "LI") : [el];
  const start = Number(list && list.start ? list.start : 1) || 1;
  const index = items.indexOf(el);
  ctx.font = shareComputedFont(marker.font ? marker : style);
  ctx.textAlign = "right";
  ctx.textBaseline = "alphabetic";
  ctx.fillText(
    `${start + Math.max(0, index)}.`,
    el.getBoundingClientRect().left - ox - 8,
    first.top - oy + first.height * 0.78,
  );
  ctx.textAlign = "left";
}

/**
 * Split a text node into the browser's line boxes. Each character is measured
 * on its own: a Range spanning several lines reports the union of its line
 * boxes, so its `top` never moves and wrapping would go undetected.
 */
function shareTextLines(node) {
  const text = node.nodeValue || "";
  const range = document.createRange();
  const lines = [];
  let line = null;
  for (let k = 0; k < text.length; k += 1) {
    range.setStart(node, k);
    range.setEnd(node, k + 1);
    const r = range.getClientRects()[0];
    const ch = text[k];
    if (!r || (r.width === 0 && r.height === 0)) continue;
    const sameLine = line && Math.abs(r.top - line.top) <= Math.max(0.6, line.height * 0.3);
    if (sameLine) {
      line.text += ch;
      line.right = Math.max(line.right, r.right);
      line.height = Math.max(line.height, r.height);
      continue;
    }
    // Collapsed or hanging whitespace at a wrap point sits at the far edge of
    // the previous line; it must not anchor a new line there.
    if (/\s/.test(ch) || r.width === 0) continue;
    line = { text: ch, left: r.left, right: r.right, top: r.top, height: r.height };
    lines.push(line);
  }
  for (const entry of lines) entry.text = entry.text.replace(/\s+$/, "");
  return lines.filter((entry) => entry.text);
}

/** Paint a text node using the browser's already-wrapped line boxes. */
function paintShareTextNode(ctx, node, ox, oy, stats) {
  const text = node.nodeValue;
  if (!text || !node.parentElement) return;
  const style = getComputedStyle(node.parentElement);
  if (style.visibility === "hidden" || parseFloat(style.opacity || "1") === 0) return;
  ctx.fillStyle = style.color;
  ctx.font = shareComputedFont(style);
  ctx.textBaseline = "top";
  ctx.textAlign = "left";
  if (ctx.letterSpacing !== undefined) ctx.letterSpacing = style.letterSpacing || "0px";
  if (stats && text.trim()) {
    const range = document.createRange();
    range.selectNodeContents(node);
    const tops = [];
    for (const r of range.getClientRects()) {
      if (r.width <= 0.5) continue;
      if (!tops.some((top) => Math.abs(top - r.top) <= 0.6)) tops.push(r.top);
    }
    stats.domLines += tops.length;
  }
  for (const line of shareTextLines(node)) {
    // Canvas metrics can differ from the DOM font (fallback CJK faces), so
    // never let a painted line run past the box the browser laid out.
    const boxWidth = Math.max(1, line.right - line.left);
    ctx.fillText(line.text, line.left - ox, line.top - oy, boxWidth);
    if (stats) {
      const painted = Math.min(ctx.measureText(line.text).width, boxWidth);
      stats.lines += 1;
      stats.maxRight = Math.max(stats.maxRight, line.left - ox + painted);
    }
  }
  if (ctx.letterSpacing !== undefined) ctx.letterSpacing = "0px";
}

function paintShareDom(ctx, node, ox, oy, alpha, stats) {
  if (node.nodeType === Node.TEXT_NODE) {
    ctx.globalAlpha = alpha;
    paintShareTextNode(ctx, node, ox, oy, stats);
    return;
  }
  if (node.nodeType !== Node.ELEMENT_NODE) return;
  const el = node;
  const tag = el.tagName;
  if (tag === "SCRIPT" || tag === "STYLE" || tag === "LINK" || tag === "NOSCRIPT") return;
  // KaTeX's clipped accessibility tree duplicates the visible formula. Canvas
  // does not inherit CSS clipping, so never paint that parallel MathML tree.
  if (el.classList.contains("katex-mathml")) return;
  const style = getComputedStyle(el);
  if (style.display === "none" || style.visibility === "hidden") return;
  const opacity = parseFloat(style.opacity);
  const nextAlpha = alpha * (Number.isFinite(opacity) ? opacity : 1);
  if (nextAlpha <= 0) return;
  ctx.globalAlpha = nextAlpha;
  paintShareBox(ctx, el, style, ox, oy);
  paintShareListMarker(ctx, el, style, ox, oy);
  if (tag === "IMG" && el.naturalWidth) {
    const rect = el.getBoundingClientRect();
    try {
      ctx.drawImage(el, rect.left - ox, rect.top - oy, rect.width, rect.height);
    } catch {
      // Cross-origin or broken images stay empty.
    }
    return;
  }
  for (const child of el.childNodes) paintShareDom(ctx, child, ox, oy, nextAlpha, stats);
}

/**
 * Mount assistant HTML with the live `.md` stylesheet, typeset KaTeX the same
 * way chat does, then paint the laid-out DOM onto a canvas.
 */
async function rasterizeShareHtml(html, width) {
  const host = document.createElement("div");
  host.className = "body md share-raster";
  host.setAttribute("aria-hidden", "true");
  host.style.cssText = [
    "position:fixed",
    "left:-10000px",
    "top:0",
    `width:${Math.max(1, Math.round(width))}px`,
    "max-width:none",
    "margin:0",
    "padding:0",
    "pointer-events:none",
    "z-index:-1",
    "overflow:visible",
    "background:transparent",
  ].join(";");
  host.innerHTML = html;
  document.body.appendChild(host);
  host.querySelectorAll("table").forEach((table) => {
    table.style.display = "table";
    table.style.overflow = "visible";
    table.style.width = "100%";
  });
  const tableCount = host.querySelectorAll("table").length;
  const mathCount = host.querySelectorAll(".math").length;
  try {
    await highlight_root(host);
  } catch {
    // Highlight/KaTeX stay best-effort; the HTML is still painted.
  }
  if (document.fonts?.ready) {
    try { await document.fonts.ready; } catch { /* ignore */ }
  }
  try {
    await document.fonts.load("16px KaTeX_Main");
  } catch {
    // KaTeX fonts are optional until the stylesheet finishes loading.
  }
  await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  const rect = host.getBoundingClientRect();
  const w = Math.max(1, Math.ceil(Math.max(host.scrollWidth, rect.width)));
  const h = Math.max(1, Math.ceil(Math.max(host.scrollHeight, rect.height)));
  const canvas = document.createElement("canvas");
  canvas.width = Math.ceil(w * SHARE_SCALE);
  canvas.height = Math.ceil(h * SHARE_SCALE);
  const ctx = canvas.getContext("2d");
  if (!ctx) {
    host.remove();
    throw new Error("Canvas is not available");
  }
  ctx.scale(SHARE_SCALE, SHARE_SCALE);
  const stats = { lines: 0, domLines: 0, maxRight: 0 };
  paintShareDom(ctx, host, rect.left, rect.top, 1, stats);
  ctx.globalAlpha = 1;
  const katexCount = host.querySelectorAll(".katex").length;
  host.remove();
  return {
    canvas,
    width: w,
    height: h,
    tableCount,
    mathCount,
    katexCount,
    textLines: stats.lines,
    domLines: stats.domLines,
    textOverflow: Math.max(0, stats.maxRight - w),
  };
}

function recordShareRasterInfo(info) {
  if (typeof window !== "undefined") window.__shareRasterInfo = info;
}

/**
 * Draw the selected conversation as one tall PNG using the live chat theme.
 * Assistant rows reuse chat HTML (tables + KaTeX) so the image matches the
 * thread. User rows are the warm panel bubble. Thinking is a left-border note.
 * @param {string} payloadJson
 */
export async function render_share_png(payloadJson) {
  applyShareTheme(resolveLiveShareTheme());
  const payload = JSON.parse(payloadJson);
  const messages = Array.isArray(payload.messages) ? payload.messages : [];
  const measure = document.createElement("canvas").getContext("2d");
  if (!measure) throw new Error("Canvas is not available");

  // The share dialog lets the user override the canvas width; out-of-range or
  // missing values fall back to the default.
  const requestedWidth = Number(payload.width);
  const shareWidth = Number.isFinite(requestedWidth) && requestedWidth > 0
    ? Math.min(Math.max(Math.round(requestedWidth), SHARE_MIN_WIDTH), SHARE_MAX_WIDTH)
    : SHARE_WIDTH;
  const proseWidth = shareWidth - SHARE_PAD * 2;
  const userSize = SHARE_THEME.fontSize + 0.5;
  const userLh = Math.round(userSize * 1.55);
  const thinkSize = SHARE_THEME.fontSize - 0.5;
  const thinkLh = Math.round(thinkSize * 1.55);
  const bubbleMaxText = Math.floor(proseWidth * 0.78) - SHARE_BUBBLE_PAD_X * 2;

  const rasters = [];
  let tableCount = 0;
  let mathCount = 0;
  let katexCount = 0;
  let textLines = 0;
  let domLines = 0;
  let textOverflow = 0;
  let usedHtml = false;
  let fallback = false;
  for (const message of messages) {
    if (message.kind === "assistant" && typeof message.html === "string" && message.html) {
      try {
        const raster = await rasterizeShareHtml(message.html, proseWidth);
        rasters.push(raster);
        usedHtml = true;
        tableCount += raster.tableCount;
        mathCount += raster.mathCount;
        katexCount += raster.katexCount;
        textLines += raster.textLines;
        domLines += raster.domLines;
        textOverflow = Math.max(textOverflow, raster.textOverflow);
      } catch {
        rasters.push(null);
        fallback = true;
      }
    } else {
      rasters.push(null);
    }
  }
  recordShareRasterInfo({
    usedHtml,
    fallback,
    tableCount,
    mathCount,
    katexCount,
    textLines,
    domLines,
    textOverflow,
  });

  const laid = messages.map((message, index) => {
    const raster = rasters[index];
    if (raster) {
      const drawWidth = Math.min(raster.width, proseWidth);
      const drawHeight = raster.width > 0
        ? raster.height * (drawWidth / raster.width)
        : raster.height;
      return {
        ...message,
        mode: "html",
        raster,
        drawWidth,
        drawHeight,
        width: proseWidth,
        height: Math.max(drawHeight, userLh),
      };
    }
    if (message.kind === "assistant" && Array.isArray(message.blocks)) {
      const blocks = layoutShareBlocks(measure, message.blocks, proseWidth, SHARE_SERIF);
      const contentHeight = blocks.reduce(
        (sum, block) => sum + block.before + shareBlockContentHeight(block) + block.after,
        0,
      );
      return {
        ...message,
        mode: "prose",
        blocks,
        width: proseWidth,
        height: Math.max(contentHeight, userLh),
      };
    }
    if (message.kind === "thinking") {
      const { lines } = shareLayoutPlain(
        measure,
        String(message.text || ""),
        proseWidth - 14,
        true,
        thinkSize,
      );
      return {
        ...message,
        mode: "thinking",
        lines,
        width: proseWidth,
        height: Math.max(lines.length * thinkLh, thinkLh),
        lh: thinkLh,
      };
    }
    const { lines, width } = shareLayoutPlain(
      measure,
      String(message.text || ""),
      bubbleMaxText,
      false,
      userSize,
    );
    return {
      ...message,
      mode: "bubble",
      lines,
      width: width + SHARE_BUBBLE_PAD_X * 2,
      height: lines.length * userLh + SHARE_BUBBLE_PAD_Y * 2,
      lh: userLh,
    };
  });

  const headerHeight = 64;
  const footerHeight = 40;
  const bodyHeight = laid.reduce((sum, m) => sum + m.height + 18 + SHARE_GAP, 0);
  const totalHeight = headerHeight + bodyHeight + footerHeight;

  const canvas = document.createElement("canvas");
  canvas.width = shareWidth * SHARE_SCALE;
  canvas.height = totalHeight * SHARE_SCALE;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Canvas is not available");
  ctx.scale(SHARE_SCALE, SHARE_SCALE);

  ctx.fillStyle = SHARE_COLORS.bg;
  ctx.fillRect(0, 0, shareWidth, totalHeight);

  ctx.fillStyle = SHARE_COLORS.text;
  ctx.font = `600 14px ${SHARE_SANS}`;
  ctx.fillText(String(payload.title || ""), SHARE_PAD, 28);
  ctx.fillStyle = SHARE_COLORS.faint;
  ctx.font = `10.5px ${SHARE_MONO}`;
  ctx.fillText(String(payload.subtitle || ""), SHARE_PAD, 44);
  ctx.fillStyle = SHARE_COLORS.border;
  ctx.fillRect(SHARE_PAD, 54, proseWidth, 1);

  let y = headerHeight;
  for (const message of laid) {
    const user = message.kind === "user";
    const x = user ? shareWidth - SHARE_PAD - message.width : SHARE_PAD;

    ctx.font = SHARE_LABEL_FONT;
    ctx.fillStyle = SHARE_COLORS.faint;
    const label = String(message.label || "").toUpperCase();
    const labelWidth = ctx.measureText(label).width;
    ctx.fillText(label, user ? shareWidth - SHARE_PAD - labelWidth : SHARE_PAD, y + 10);
    y += 18;

    if (message.mode === "html") {
      ctx.drawImage(
        message.raster.canvas,
        x,
        y,
        message.drawWidth,
        message.drawHeight,
      );
    } else if (message.mode === "prose") {
      drawShareBlocks(ctx, message.blocks, x, y, proseWidth);
    } else if (message.mode === "thinking") {
      ctx.fillStyle = SHARE_COLORS.borderStrong;
      ctx.fillRect(x, y, 2, message.height);
      let ty = y + Math.round(message.lh * 0.72);
      for (const line of message.lines) {
        drawShareLine(ctx, line, x + 12, ty, SHARE_COLORS.faint, message.lh, false);
        ty += message.lh;
      }
    } else {
      ctx.fillStyle = SHARE_COLORS.panel;
      shareRoundRect(ctx, x, y, message.width, message.height, [18, 18, 6, 18]);
      ctx.fill();
      ctx.strokeStyle = SHARE_COLORS.border;
      shareRoundRect(ctx, x, y, message.width, message.height, [18, 18, 6, 18]);
      ctx.stroke();
      let ty = y + SHARE_BUBBLE_PAD_Y + Math.round(message.lh * 0.72);
      for (const line of message.lines) {
        drawShareLine(ctx, line, x + SHARE_BUBBLE_PAD_X, ty, SHARE_COLORS.text, message.lh, false);
        ty += message.lh;
      }
    }
    y += message.height + SHARE_GAP;
  }

  ctx.fillStyle = SHARE_COLORS.faint;
  ctx.font = `11px ${SHARE_SANS}`;
  ctx.fillText(String(payload.footer || ""), SHARE_PAD, totalHeight - 16);

  const dataUrl = canvas.toDataURL("image/png");
  const comma = dataUrl.indexOf(",");
  return comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl;
}

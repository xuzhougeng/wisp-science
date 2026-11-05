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

// Collapsible white space never needs a glyph of its own: words are placed
// at their own rects, so the browser's spacing survives without it.
const SHARE_SPACE = /^[ \t\n\r\f]$/;
// CJK and full-width glyphs are placed one by one. Chrome trims punctuation
// spacing (`text-spacing-trim`), which canvas text does not, so a whole CJK
// line drawn as one string drifts off its DOM positions.
const SHARE_SINGLE_GLYPH = /[\u2e80-\u9fff\uac00-\ud7af\uf900-\ufaff\ufe30-\ufe4f\uff00-\uffef]/;
// Opening punctuation trims its leading half; the ink sits at the right edge.
const SHARE_OPENING_PUNCT = /[「『（【《〈〔〖〘〚｛［“‘]/;
// Numbers the raster reports per assistant reply (see __shareRasterInfo).
const SHARE_RASTER_COUNTS = [
  "tableCount",
  "mathCount",
  "katexCount",
  "textLines",
  "domLines",
  "wrappedNodes",
  "lineMismatches",
  "overflowRuns",
  "markers",
  "markersOffLine",
];
// Table rows up to this tall prefer one page in multi-image export.
const SHARE_ATOMIC_ROW_MAX = 240;

function shareCodePointLength(text, index) {
  const code = text.charCodeAt(index);
  return code >= 0xd800 && code <= 0xdbff && index + 1 < text.length ? 2 : 1;
}

/**
 * The widest box the browser gives `node[start, end)`, or null when it has
 * no width. A range that touches a line break can also report an empty box
 * at the end of the previous line; the glyph's own box is the wide one.
 */
function shareCharRect(range, node, start, end) {
  range.setStart(node, start);
  range.setEnd(node, end);
  let best = null;
  for (const rect of range.getClientRects()) {
    if (rect.width > 0 && rect.height > 0 && (!best || rect.width > best.width)) best = rect;
  }
  return best;
}

function shareSameLine(rect, top) {
  return Math.abs(rect.top - top) < Math.max(1, rect.height * 0.5);
}

/**
 * Split a text node into what the browser laid out: one run per word per line
 * box (one per glyph for CJK), each at its own rect. Line breaks must come
 * from single-character rects. A Range grown across a soft wrap reports the
 * union of its line boxes, and that union's top stays on the first line, so
 * the old painter drew a wrapped node as one overflowing line (#1677).
 * Collapsed white space is skipped, so no line starts at the space the
 * browser folded onto the end of the previous one.
 */
function shareTextRuns(node) {
  const text = node.nodeValue || "";
  const range = document.createRange();
  const runs = [];
  let run = null;
  let line = -1;
  let lineTop = NaN;
  for (let k = 0; k < text.length;) {
    const step = shareCodePointLength(text, k);
    const ch = text.slice(k, k + step);
    const start = k;
    k += step;
    if (SHARE_SPACE.test(ch)) {
      run = null;
      continue;
    }
    const rect = shareCharRect(range, node, start, start + step);
    if (!rect) {
      // Zero-width pieces (joiners, combining marks) stay with their word.
      if (run) run.text += ch;
      continue;
    }
    if (!shareSameLine(rect, lineTop)) {
      line += 1;
      lineTop = rect.top;
      run = null;
    }
    const single = SHARE_SINGLE_GLYPH.test(ch);
    if (!run || single || run.single) {
      run = {
        text: "",
        left: rect.left,
        right: rect.right,
        top: rect.top,
        height: rect.height,
        line,
        single,
      };
      runs.push(run);
    }
    run.text += ch;
    run.right = Math.max(run.right, rect.right);
  }
  return runs;
}

/**
 * Line boxes the browser reports for the visible span of a text node, one
 * rect per line. Only used to check the painter (`__shareRasterInfo`).
 */
function shareDomLineRects(node) {
  const text = node.nodeValue || "";
  let start = 0;
  let end = text.length;
  while (start < end && SHARE_SPACE.test(text[start])) start += 1;
  while (end > start && SHARE_SPACE.test(text[end - 1])) end -= 1;
  if (start >= end) return [];
  const range = document.createRange();
  range.setStart(node, start);
  range.setEnd(node, end);
  const lines = [];
  for (const rect of range.getClientRects()) {
    if (rect.width <= 0 || rect.height <= 0) continue;
    if (!lines.some((seen) => shareSameLine(rect, seen.top))) lines.push(rect);
  }
  return lines;
}

/** Set `font` on ctx and read its ascent/descent (null where unsupported). */
function shareFontMetrics(ctx, font) {
  ctx.font = font;
  const metrics = ctx.measureText("Hg");
  const ascent = metrics.fontBoundingBoxAscent;
  const descent = metrics.fontBoundingBoxDescent;
  if (!Number.isFinite(ascent) || !Number.isFinite(descent) || ascent + descent <= 0) return null;
  return { ascent, descent };
}

/** Baseline CSS gives text with `metrics` inside a glyph box `top`/`height`. */
function shareBaselineIn(metrics, top, height) {
  if (!metrics) return top + height * 0.8;
  // The content area (ascent + descent) is centred in the box.
  return top + (height - metrics.ascent - metrics.descent) / 2 + metrics.ascent;
}

/** Text hidden for screen readers only, e.g. KaTeX's MathML copy. */
function shareScreenReaderOnly(el, style) {
  return el.classList.contains("katex-mathml")
    || (style.position === "absolute" && !!style.clip && style.clip !== "auto");
}

/** The first visible glyph inside `el`: its rect and its text node. */
function shareFirstGlyph(el) {
  const range = document.createRange();
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) {
    const node = walker.currentNode;
    if (!node.parentElement || node.parentElement.closest(".katex-mathml")) continue;
    const text = node.nodeValue || "";
    for (let k = 0; k < text.length;) {
      const step = shareCodePointLength(text, k);
      if (!SHARE_SPACE.test(text.slice(k, k + step))) {
        const rect = shareCharRect(range, node, k, k + step);
        if (rect) return { rect, node };
      }
      k += step;
    }
  }
  return null;
}

/**
 * Draw a list marker to the left of the item's first line; ::marker is not
 * in the DOM. The first line is the first glyph's own box: a whole text node
 * spans every line it wraps onto, which pushed numbers down (#1677).
 */
function paintShareListMarker(ctx, el, style, ox, oy, info) {
  if (el.tagName !== "LI") return;
  const type = style.listStyleType;
  if (!type || type === "none") return;
  const marker = getComputedStyle(el, "::marker");
  const glyph = shareFirstGlyph(el);
  const first = glyph ? glyph.rect : el.getBoundingClientRect();
  const textFont = shareComputedFont(glyph ? getComputedStyle(glyph.node.parentElement) : style);
  const baseline = shareBaselineIn(shareFontMetrics(ctx, textFont), first.top, first.height);
  const left = el.getBoundingClientRect().left - ox;
  let markY = baseline;
  ctx.fillStyle = marker.color || style.color;
  if (type === "disc" || type === "circle") {
    markY = first.top + first.height * 0.5;
    ctx.beginPath();
    ctx.arc(left - 10, markY - oy, 2.2, 0, Math.PI * 2);
    if (type === "circle") {
      ctx.lineWidth = 1;
      ctx.strokeStyle = ctx.fillStyle;
      ctx.stroke();
    } else {
      ctx.fill();
    }
  } else if (type === "decimal") {
    const list = el.parentElement;
    const items = list ? [...list.children].filter((child) => child.tagName === "LI") : [el];
    const start = Number(list && list.start ? list.start : 1) || 1;
    const index = items.indexOf(el);
    ctx.font = shareComputedFont(marker.font ? marker : style);
    ctx.textAlign = "right";
    ctx.textBaseline = "alphabetic";
    ctx.fillText(`${start + Math.max(0, index)}.`, left - 8, baseline - oy);
    ctx.textAlign = "left";
  } else {
    return;
  }
  if (info && glyph) {
    const line = shareDomLineRects(glyph.node)[0];
    info.markers += 1;
    if (!line || markY < line.top || markY > line.bottom) info.markersOffLine += 1;
  }
}

/** Paint a text node at the browser's own line boxes, run by run. */
function paintShareTextNode(ctx, node, ox, oy, info) {
  const text = node.nodeValue;
  const parent = node.parentElement;
  if (!text || !parent) return;
  const style = getComputedStyle(parent);
  if (style.visibility === "hidden" || parseFloat(style.opacity || "1") === 0) return;
  const runs = shareTextRuns(node);
  if (!runs.length) return;
  const font = shareComputedFont(style);
  ctx.fillStyle = style.color;
  ctx.textAlign = "left";
  ctx.textBaseline = "alphabetic";
  if (ctx.letterSpacing !== undefined) ctx.letterSpacing = style.letterSpacing || "0px";
  const transform = style.textTransform;
  const math = !!parent.closest(".katex");
  const cell = parent.closest("td, th");
  const bound = cell ? cell.getBoundingClientRect() : info?.bounds;
  const metrics = shareFontMetrics(ctx, font);
  for (const run of runs) {
    const label = transform === "uppercase"
      ? run.text.toUpperCase()
      : transform === "lowercase" ? run.text.toLowerCase() : run.text;
    const baseline = shareBaselineIn(metrics, run.top, run.height);
    // A CJK glyph keeps its own DOM box; only opening punctuation needs its
    // drawn width, to sit flush with the box's right edge.
    const opening = run.single && SHARE_OPENING_PUNCT.test(run.text);
    const width = run.single && !opening ? run.right - run.left : ctx.measureText(label).width;
    const x = opening ? run.right - width : run.left;
    ctx.fillText(label, x - ox, baseline - oy);
    if (!info) continue;
    info.atoms.push([run.top - oy, run.top + run.height - oy]);
    info.maxRight = Math.max(info.maxRight, (run.single ? run.right : x + width) - ox);
    if (!math && bound) {
      const left = run.single ? run.left : x;
      const right = run.single ? run.right : x + width;
      if (right > bound.right + 2 || left < bound.left - 2) info.overflowRuns += 1;
    }
  }
  if (ctx.letterSpacing !== undefined) ctx.letterSpacing = "0px";
  // KaTeX stacks superscripts and fractions inside one line; skip it here.
  if (!info || math) return;
  const painted = runs[runs.length - 1].line + 1;
  const dom = shareDomLineRects(node).length;
  info.textLines += painted;
  info.domLines += dom;
  if (dom > 1) info.wrappedNodes += 1;
  if (dom !== painted) info.lineMismatches += 1;
}

function paintShareDom(ctx, node, ox, oy, alpha, info) {
  if (node.nodeType === Node.TEXT_NODE) {
    ctx.globalAlpha = alpha;
    paintShareTextNode(ctx, node, ox, oy, info);
    return;
  }
  if (node.nodeType !== Node.ELEMENT_NODE) return;
  const el = node;
  const tag = el.tagName;
  if (tag === "SCRIPT" || tag === "STYLE" || tag === "LINK" || tag === "NOSCRIPT") return;
  const style = getComputedStyle(el);
  if (style.display === "none" || style.visibility === "hidden") return;
  if (shareScreenReaderOnly(el, style)) return;
  const opacity = parseFloat(style.opacity);
  const nextAlpha = alpha * (Number.isFinite(opacity) ? opacity : 1);
  if (nextAlpha <= 0) return;
  ctx.globalAlpha = nextAlpha;
  paintShareBox(ctx, el, style, ox, oy);
  paintShareListMarker(ctx, el, style, ox, oy, info);
  if (tag === "IMG" && el.naturalWidth) {
    const rect = el.getBoundingClientRect();
    try {
      ctx.drawImage(el, rect.left - ox, rect.top - oy, rect.width, rect.height);
    } catch {
      // Cross-origin or broken images stay empty.
    }
    info?.atoms.push([rect.top - oy, rect.bottom - oy]);
    return;
  }
  if (info && el.classList.contains("katex")) {
    // Page breaks go around equations, not through them.
    const rect = el.getBoundingClientRect();
    info.atoms.push([rect.top - oy, rect.bottom - oy]);
  } else if (info && tag === "TR") {
    // Short rows prefer one page, but may still break between their lines.
    const rect = el.getBoundingClientRect();
    if (rect.height <= SHARE_ATOMIC_ROW_MAX) info.atoms.push([rect.top - oy, rect.bottom - oy, true]);
  }
  for (const child of el.childNodes) paintShareDom(ctx, child, ox, oy, nextAlpha, info);
}

/**
 * Mount assistant HTML with the live `.md` stylesheet, typeset KaTeX the same
 * way chat does, then paint the laid-out DOM onto a canvas. `atoms` are the
 * vertical spans a page break must not cut (text lines, images, equations).
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
  const stats = Object.fromEntries(SHARE_RASTER_COUNTS.map((key) => [key, 0]));
  stats.tableCount = host.querySelectorAll("table").length;
  stats.mathCount = host.querySelectorAll(".math").length;
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
  const info = { ...stats, atoms: [], bounds: rect, maxRight: 0 };
  try {
    paintShareDom(ctx, host, rect.left, rect.top, 1, info);
  } finally {
    ctx.globalAlpha = 1;
    info.katexCount = host.querySelectorAll(".katex").length;
    host.remove();
  }
  for (const key of SHARE_RASTER_COUNTS) stats[key] = info[key];
  // How far painted text reaches past the content width (#1678's check).
  stats.textOverflow = Math.max(0, info.maxRight - w);
  return { canvas, width: w, height: h, atoms: info.atoms, stats };
}

function recordShareRasterInfo(info) {
  if (typeof window !== "undefined") window.__shareRasterInfo = info;
}

// Single-image header/footer and the per-message role label above each row.
const SHARE_HEADER_HEIGHT = 64;
const SHARE_FOOTER_HEIGHT = 40;
const SHARE_LABEL_HEIGHT = 18;
// Multi-image pages are portrait 3:4, the usual carousel ratio.
const SHARE_PAGE_RATIO = 4 / 3;

/** Unbreakable spans of the fallback prose renderer, from content top. */
function shareBlockAtoms(blocks) {
  const atoms = [];
  let cy = 0;
  for (const block of blocks) {
    cy += block.before;
    if (block.t === "hr") {
      cy += 1 + block.after;
      continue;
    }
    const top = block.t === "code" ? cy + block.pad : cy;
    block.lines.forEach((_, i) => atoms.push([top + i * block.lh, top + (i + 1) * block.lh]));
    cy += (block.t === "code" ? block.height : block.lines.length * block.lh) + block.after;
  }
  return atoms;
}

/**
 * Unbreakable spans of one laid-out message, from its content top. A span is
 * `[top, bottom]`, or `[top, bottom, true]` when it only prefers to stay whole.
 */
function shareMessageAtoms(message) {
  if (message.mode === "html") {
    const scale = message.raster.width > 0 ? message.drawWidth / message.raster.width : 1;
    return message.raster.atoms.map(([top, bottom, soft]) => [top * scale, bottom * scale, soft]);
  }
  if (message.mode === "prose") return shareBlockAtoms(message.blocks);
  const pad = message.mode === "bubble" ? SHARE_BUBBLE_PAD_Y : 0;
  return message.lines.map((_, i) => [pad + i * message.lh, pad + (i + 1) * message.lh]);
}

/** Merge overlapping spans; spans that only touch stay apart (a cut fits). */
function mergeShareAtoms(atoms) {
  const sorted = atoms.filter(([top, bottom]) => bottom > top).sort((a, b) => a[0] - b[0]);
  const merged = [];
  for (const [top, bottom] of sorted) {
    const last = merged[merged.length - 1];
    if (last && top < last[1]) {
      last[1] = Math.max(last[1], bottom);
    } else {
      merged.push([top, bottom]);
    }
  }
  return merged;
}

/**
 * Cut a body `height` tall into pages of at most `pageBody`. A cut first moves
 * up to the top of the row or label group it would cross (soft spans), unless
 * that would leave most of the page empty; then to the top of the text line,
 * image, or equation it would cross. Only one span taller than a page that
 * also starts near the page top is cut through (counted in `hardCuts`).
 */
function sharePageCuts(atoms, height, pageBody) {
  const all = mergeShareAtoms(atoms);
  const hard = mergeShareAtoms(atoms.filter((atom) => !atom[2]));
  const crossing = (spans, cut) => spans.find(([top, bottom]) => top < cut && cut < bottom);
  const cuts = [0];
  let hardCuts = 0;
  let start = 0;
  while (height - start > pageBody + 0.5) {
    const limit = start + pageBody;
    const roomy = start + pageBody * 0.3;
    let cut = limit;
    const group = crossing(all, limit);
    const piece = crossing(hard, limit);
    if (group && group[0] > roomy) {
      cut = group[0];
    } else if (piece && piece[0] > start && (piece[0] > roomy || piece[1] - piece[0] <= pageBody)) {
      cut = piece[0];
    } else if (piece) {
      hardCuts += 1;
    }
    cuts.push(cut);
    start = cut;
  }
  cuts.push(height);
  return { cuts, hardCuts };
}

/**
 * Rasterize and lay out the selected messages. Message `top`s and `atoms`
 * are in body coordinates (0 = first role label), shared by both layouts.
 */
async function layoutShareMessages(payload) {
  applyShareTheme(resolveLiveShareTheme());
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
  const info = {
    usedHtml: false,
    fallback: false,
    textOverflow: 0,
    ...Object.fromEntries(SHARE_RASTER_COUNTS.map((key) => [key, 0])),
  };
  for (const message of messages) {
    if (message.kind === "assistant" && typeof message.html === "string" && message.html) {
      try {
        const raster = await rasterizeShareHtml(message.html, proseWidth);
        rasters.push(raster);
        info.usedHtml = true;
        for (const key of SHARE_RASTER_COUNTS) info[key] += raster.stats[key];
        info.textOverflow = Math.max(info.textOverflow, raster.stats.textOverflow);
      } catch {
        rasters.push(null);
        info.fallback = true;
      }
    } else {
      rasters.push(null);
    }
  }

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

  const atoms = [];
  let y = 0;
  for (const message of laid) {
    message.top = y;
    const contentTop = y + SHARE_LABEL_HEIGHT;
    let firstTop = Infinity;
    for (const [top, bottom, soft] of shareMessageAtoms(message)) {
      atoms.push([contentTop + top, contentTop + bottom, soft]);
      firstTop = Math.min(firstTop, contentTop + top);
    }
    // Keep the role label on the same page as the first line under it.
    atoms.push([y, (Number.isFinite(firstTop) ? firstTop : contentTop) + 1, true]);
    y = contentTop + message.height + SHARE_GAP;
  }
  return { payload, shareWidth, proseWidth, laid, atoms, bodyHeight: y, info };
}

function shareCanvas(width, height) {
  const canvas = document.createElement("canvas");
  canvas.width = Math.ceil(width * SHARE_SCALE);
  canvas.height = Math.ceil(height * SHARE_SCALE);
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Canvas is not available");
  ctx.scale(SHARE_SCALE, SHARE_SCALE);
  ctx.fillStyle = SHARE_COLORS.bg;
  ctx.fillRect(0, 0, width, height);
  return { canvas, ctx };
}

function shareCanvasBase64(canvas) {
  const dataUrl = canvas.toDataURL("image/png");
  const comma = dataUrl.indexOf(",");
  return comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl;
}

function drawShareHeader(ctx, layout) {
  ctx.textAlign = "left";
  ctx.textBaseline = "alphabetic";
  ctx.fillStyle = SHARE_COLORS.text;
  ctx.font = `600 14px ${SHARE_SANS}`;
  ctx.fillText(String(layout.payload.title || ""), SHARE_PAD, 28);
  ctx.fillStyle = SHARE_COLORS.faint;
  ctx.font = `10.5px ${SHARE_MONO}`;
  ctx.fillText(String(layout.payload.subtitle || ""), SHARE_PAD, 44);
  ctx.fillStyle = SHARE_COLORS.border;
  ctx.fillRect(SHARE_PAD, 54, layout.proseWidth, 1);
}

/** Footer line; multi-image pages also carry "page/total" on the right. */
function drawShareFooter(ctx, layout, totalHeight, pageLabel) {
  ctx.textAlign = "left";
  ctx.textBaseline = "alphabetic";
  ctx.fillStyle = SHARE_COLORS.faint;
  ctx.font = `11px ${SHARE_SANS}`;
  ctx.fillText(String(layout.payload.footer || ""), SHARE_PAD, totalHeight - 16);
  if (pageLabel) {
    ctx.font = `10.5px ${SHARE_MONO}`;
    ctx.textAlign = "right";
    ctx.fillText(pageLabel, layout.shareWidth - SHARE_PAD, totalHeight - 16);
    ctx.textAlign = "left";
  }
}

/**
 * Draw every message whose span meets [visibleTop, visibleBottom] (body
 * coordinates), with body y = 0 at canvas y `originY`.
 */
function drawShareMessages(ctx, layout, originY, visibleTop = -Infinity, visibleBottom = Infinity) {
  const { laid, shareWidth, proseWidth } = layout;
  for (const message of laid) {
    const end = message.top + SHARE_LABEL_HEIGHT + message.height;
    if (end < visibleTop || message.top > visibleBottom) continue;
    const user = message.kind === "user";
    const x = user ? shareWidth - SHARE_PAD - message.width : SHARE_PAD;
    let y = originY + message.top;

    ctx.textAlign = "left";
    ctx.textBaseline = "alphabetic";
    ctx.font = SHARE_LABEL_FONT;
    ctx.fillStyle = SHARE_COLORS.faint;
    const label = String(message.label || "").toUpperCase();
    const labelWidth = ctx.measureText(label).width;
    ctx.fillText(label, user ? shareWidth - SHARE_PAD - labelWidth : SHARE_PAD, y + 10);
    y += SHARE_LABEL_HEIGHT;

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
  }
}

/**
 * Draw the selected conversation as one tall PNG using the live chat theme.
 * Assistant rows reuse chat HTML (tables + KaTeX) so the image matches the
 * thread. User rows are the warm panel bubble. Thinking is a left-border note.
 * @param {string} payloadJson
 */
export async function render_share_png(payloadJson) {
  const layout = await layoutShareMessages(JSON.parse(payloadJson));
  const totalHeight = SHARE_HEADER_HEIGHT + layout.bodyHeight + SHARE_FOOTER_HEIGHT;
  const { canvas, ctx } = shareCanvas(layout.shareWidth, totalHeight);
  drawShareHeader(ctx, layout);
  drawShareMessages(ctx, layout, SHARE_HEADER_HEIGHT);
  drawShareFooter(ctx, layout, totalHeight, "");
  recordShareRasterInfo({ ...layout.info, pages: 1, hardCuts: 0 });
  return shareCanvasBase64(canvas);
}

/**
 * Draw the same conversation as several 3:4 pages for carousel posts. Every
 * page repeats the header, numbers itself in the footer, and breaks between
 * lines (see sharePageCuts), so no text line is split across two images.
 * Resolves to an array of base64 PNGs in page order.
 * @param {string} payloadJson
 */
export async function render_share_png_pages(payloadJson) {
  const layout = await layoutShareMessages(JSON.parse(payloadJson));
  const pageHeight = Math.round(layout.shareWidth * SHARE_PAGE_RATIO);
  const pageBody = pageHeight - SHARE_HEADER_HEIGHT - SHARE_FOOTER_HEIGHT;
  // The trailing gap after the last message never needs a page of its own.
  const bodyHeight = Math.max(1, layout.bodyHeight - SHARE_GAP);
  const { cuts, hardCuts } = sharePageCuts(layout.atoms, bodyHeight, pageBody);
  const count = cuts.length - 1;
  const pages = [];
  for (let i = 0; i < count; i += 1) {
    const top = cuts[i];
    const bottom = cuts[i + 1];
    const { canvas, ctx } = shareCanvas(layout.shareWidth, pageHeight);
    drawShareHeader(ctx, layout);
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, SHARE_HEADER_HEIGHT, layout.shareWidth, bottom - top);
    ctx.clip();
    drawShareMessages(ctx, layout, SHARE_HEADER_HEIGHT - top, top, bottom);
    ctx.restore();
    drawShareFooter(ctx, layout, pageHeight, `${i + 1}/${count}`);
    pages.push(shareCanvasBase64(canvas));
  }
  recordShareRasterInfo({ ...layout.info, pages: count, hardCuts, pageCuts: cuts });
  return pages;
}

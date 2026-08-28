// Tauri v2 shim + scientific preview mounts (single file so Trunk ships one snippet).
import { highlight_root } from "./highlight.js";

function tauriCore() {
  return window.__TAURI__?.core;
}

function tauriEvent() {
  return window.__TAURI__?.event;
}

export function is_windows() {
  return navigator.userAgent.includes("Windows");
}

export function is_mac() {
  const userAgent = navigator.userAgent || "";
  const platform = navigator.platform || "";
  return userAgent.includes("Macintosh")
    || userAgent.includes("Mac OS X")
    || platform.startsWith("Mac");
}

export async function window_control(action) {
  const current = window.__TAURI__?.window?.getCurrentWindow?.();
  if (!current) return;
  if (action === "minimize") return current.minimize();
  if (action === "toggle-maximize") return current.toggleMaximize();
  if (action === "close") return current.close();
}

/** Native window title (taskbar / Alt-Tab / macOS title bar) plus `document.title`. */
export async function set_window_title(title) {
  if (typeof document !== "undefined") {
    document.title = title;
  }
  const current = window.__TAURI__?.window?.getCurrentWindow?.();
  if (!current?.setTitle) return;
  try {
    await current.setTitle(title);
  } catch {
    // Browser mock or missing capability — `document.title` still updated.
  }
}

/** Caption-style move so Windows Aero Snap / edge snap can engage. */
export async function start_window_move() {
  const core = tauriCore();
  if (!core) return;
  try {
    return await core.invoke("start_window_move");
  } catch (err) {
    const current = window.__TAURI__?.window?.getCurrentWindow?.();
    return current?.startDragging?.();
  }
}

/** Typical Windows `SM_CXDRAG` / `SM_CYDRAG`. Keep in sync with `window_titlebar.rs`. */
export const CAPTION_DRAG_THRESHOLD_PX = 4;

/**
 * Wait for the pointer to move past the drag threshold before starting a
 * caption move. A bare mousedown must not send `SC_MOVE`, or Windows eats the
 * second click of a title-bar double-click (maximize / restore).
 */
export async function arm_caption_drag(startX, startY) {
  await new Promise((resolve) => {
    let settled = false;
    const finish = (startMove) => {
      if (settled) return;
      settled = true;
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      if (startMove) void start_window_move();
      resolve();
    };
    const onMove = (event) => {
      if (
        Math.abs(event.clientX - startX) >= CAPTION_DRAG_THRESHOLD_PX
        || Math.abs(event.clientY - startY) >= CAPTION_DRAG_THRESHOLD_PX
      ) {
        finish(true);
      }
    };
    const onUp = () => finish(false);
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  });
}

function missingBridgeError(cmd) {
  return new Error(`Tauri bridge is not available while calling ${cmd}. Open the app with 'cargo tauri dev', not the raw Trunk URL.`);
}

export async function invoke(cmd, args) {
  const core = tauriCore();
  if (!core) {
    console.error(missingBridgeError(cmd));
    return null;
  }
  try {
    return await core.invoke(cmd, args ?? {});
  } catch (err) {
    console.error(`Tauri command failed: ${cmd}`, err);
    return null;
  }
}

export async function invoke_strict(cmd, args) {
  const core = tauriCore();
  if (!core) {
    throw missingBridgeError(cmd);
  }
  return core.invoke(cmd, args ?? {});
}

export async function invoke_timeout(cmd, args, timeoutMs) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`Request timed out after ${Math.round(timeoutMs / 1000)}s`)), timeoutMs);
  });
  try {
    return await Promise.race([invoke_strict(cmd, args), timeout]);
  } finally {
    clearTimeout(timer);
  }
}

export async function download_app_update(callback) {
  const core = tauriCore();
  if (!core?.Channel) {
    throw missingBridgeError("download_update");
  }
  const onEvent = new core.Channel();
  onEvent.onmessage = callback;
  return core.invoke("download_update", { onEvent });
}

function fileToBase64(file) {
  // Keep in sync with MAX_UPLOAD_BYTES in src-tauri/src/artifact_commands.rs.
  const maxBytes = 100 * 1024 * 1024;
  if (file.size > maxBytes) {
    return Promise.reject(new Error(`file exceeds ${maxBytes} byte limit`));
  }
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const dataUrl = reader.result;
      if (typeof dataUrl !== "string") {
        reject(new Error("Failed to read file"));
        return;
      }
      const comma = dataUrl.indexOf(",");
      resolve(comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl);
    };
    reader.onerror = () => reject(reader.error || new Error("Failed to read file"));
    reader.readAsDataURL(file);
  });
}

/** @param {FileList|File[]} files */
export async function upload_files(files) {
  const list = Array.from(files || []);
  const results = [];
  for (const file of list) {
    try {
      const data_base64 = await fileToBase64(file);
      // Tauri v2 expects camelCase arg keys (maps to snake_case `data_base64`).
      const info = await invoke_strict("upload_file", { filename: file.name, dataBase64: data_base64 });
      results.push({ ok: true, info });
    } catch (err) {
      results.push({
        ok: false,
        filename: file.name,
        error: err instanceof Error ? err.message : String(err),
      });
    }
  }
  return results;
}

/**
 * Crop a rectangular region (given as fractions 0..1 of the preview host,
 * which the crop layer exactly covers) from the image inside `#hostId` and
 * upload it as a PNG. Content-relative fractions map to the image's natural
 * pixels via getBoundingClientRect, so the crop matches what the rubber-band
 * covered regardless of zoom, scroll, or a transformed modal ancestor.
 * Returns the saved workspace path, or "" on failure.
 * @param {string} hostId @param {number} left @param {number} top @param {number} width @param {number} height
 */
export async function crop_region_to_upload(hostId, left, top, width, height) {
  const host = document.getElementById(hostId);
  const img = host?.querySelector("img.rp-img");
  if (!img || !img.naturalWidth) return "";
  const hostRect = host.getBoundingClientRect();
  const rect = img.getBoundingClientRect();
  if (rect.width < 1 || rect.height < 1) return "";
  // The browser emits a click after the crop's pointerup. Do not return the
  // path (which mounts action buttons) until that click has fully dispatched.
  const scaleX = img.naturalWidth / rect.width;
  const scaleY = img.naturalHeight / rect.height;
  let sx = (hostRect.left + left * hostRect.width - rect.left) * scaleX;
  let sy = (hostRect.top + top * hostRect.height - rect.top) * scaleY;
  let sw = width * hostRect.width * scaleX;
  let sh = height * hostRect.height * scaleY;
  // Clamp the source rect to the image bounds.
  sx = Math.max(0, Math.min(sx, img.naturalWidth));
  sy = Math.max(0, Math.min(sy, img.naturalHeight));
  sw = Math.max(1, Math.min(sw, img.naturalWidth - sx));
  sh = Math.max(1, Math.min(sh, img.naturalHeight - sy));
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(sw);
  canvas.height = Math.round(sh);
  const ctx = canvas.getContext("2d");
  if (!ctx) return "";
  ctx.drawImage(img, sx, sy, sw, sh, 0, 0, canvas.width, canvas.height);
  const blob = await new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
  if (!blob) return "";
  const stamp = new Date().toISOString().replace(/\D/g, "").slice(0, 14);
  const file = new File([blob], `region_${stamp}.png`, { type: "image/png" });
  const results = await upload_files([file]);
  const ok = results.find((r) => r.ok && r.info);
  const path = ok?.info?.path;
  if (!path) return "";
  return String(path);
}

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

/**
 * Copy a social-share pack to the clipboard. `text` becomes text/plain and
 * `pngBase64` (no data-URL prefix) becomes image/png. Either side may be
 * empty. Some host apps still paste only one of the two flavors.
 * @param {string} text
 * @param {string} pngBase64
 */
export async function copy_share_pack(text, pngBase64) {
  const record = {};
  if (typeof text === "string" && text.length > 0) {
    record["text/plain"] = new Blob([text], { type: "text/plain" });
  }
  if (typeof pngBase64 === "string" && pngBase64.length > 0) {
    const binary = atob(pngBase64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i += 1) {
      bytes[i] = binary.charCodeAt(i);
    }
    record["image/png"] = new Blob([bytes], { type: "image/png" });
  }
  if (Object.keys(record).length === 0) {
    throw new Error("nothing to copy");
  }
  await navigator.clipboard.write([new ClipboardItem(record)]);
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
  const range = document.createRange();
  let first = null;
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) {
    if (!walker.currentNode.nodeValue || !walker.currentNode.nodeValue.trim()) continue;
    range.selectNodeContents(walker.currentNode);
    first = range.getBoundingClientRect();
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

/** Paint a text node using the browser's already-wrapped line boxes. */
function paintShareTextNode(ctx, node, ox, oy) {
  const text = node.nodeValue;
  if (!text || !node.parentElement) return;
  const style = getComputedStyle(node.parentElement);
  if (style.visibility === "hidden" || parseFloat(style.opacity || "1") === 0) return;
  ctx.fillStyle = style.color;
  ctx.font = shareComputedFont(style);
  ctx.textBaseline = "top";
  ctx.textAlign = "left";
  if (ctx.letterSpacing !== undefined) ctx.letterSpacing = style.letterSpacing || "0px";
  const range = document.createRange();
  let i = 0;
  while (i < text.length) {
    range.setStart(node, i);
    range.setEnd(node, i + 1);
    const first = range.getBoundingClientRect();
    if (first.width === 0 && first.height === 0) {
      i += 1;
      continue;
    }
    let j = i + 1;
    while (j < text.length) {
      range.setEnd(node, j + 1);
      const next = range.getBoundingClientRect();
      if (Math.abs(next.top - first.top) > 0.6) break;
      j += 1;
    }
    const slice = text.slice(i, j);
    if (slice && slice !== "\n") {
      ctx.fillText(slice, first.left - ox, first.top - oy);
    }
    i = j;
  }
  if (ctx.letterSpacing !== undefined) ctx.letterSpacing = "0px";
}

function paintShareDom(ctx, node, ox, oy, alpha) {
  if (node.nodeType === Node.TEXT_NODE) {
    ctx.globalAlpha = alpha;
    paintShareTextNode(ctx, node, ox, oy);
    return;
  }
  if (node.nodeType !== Node.ELEMENT_NODE) return;
  const el = node;
  const tag = el.tagName;
  if (tag === "SCRIPT" || tag === "STYLE" || tag === "LINK" || tag === "NOSCRIPT") return;
  const style = getComputedStyle(el);
  if (style.display === "none" || style.visibility === "hidden") return;
  const nextAlpha = alpha * (parseFloat(style.opacity || "1") || 1);
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
  for (const child of el.childNodes) paintShareDom(ctx, child, ox, oy, nextAlpha);
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
  paintShareDom(ctx, host, rect.left, rect.top, 1);
  ctx.globalAlpha = 1;
  const katexCount = host.querySelectorAll(".katex").length;
  host.remove();
  return { canvas, width: w, height: h, tableCount, mathCount, katexCount };
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
      } catch {
        rasters.push(null);
        fallback = true;
      }
    } else {
      rasters.push(null);
    }
  }
  recordShareRasterInfo({ usedHtml, fallback, tableCount, mathCount, katexCount });

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

/** Attach an uploaded crop, optionally returning from its preview to chat. */
export function attach_cropped_region(path, jumpToChat) {
  window.dispatchEvent(new CustomEvent("wisp:region-attach", {
    detail: { path: String(path), jumpToChat: Boolean(jumpToChat) },
  }));
}

function pastedImageName(file, index) {
  const ext = {
    "image/jpeg": "jpg",
    "image/png": "png",
    "image/gif": "gif",
    "image/webp": "webp",
    "image/svg+xml": "svg",
  }[file.type] || "png";
  const stamp = new Date().toISOString().replace(/\D/g, "").slice(0, 14);
  return `pasted_image_${stamp}_${index + 1}.${ext}`;
}

function pastedImageFiles(event) {
  const data = event?.clipboardData;
  if (!data) return [];
  const items = Array.from(data.items || []);
  const files = items.length
    ? items.filter((item) => item.kind === "file" && item.type?.startsWith("image/")).map((item) => item.getAsFile()).filter(Boolean)
    : Array.from(data.files || []).filter((file) => file.type?.startsWith("image/"));
  return files.map((file, i) => new File([file], pastedImageName(file, i), { type: file.type || "image/png" }));
}

export function pasted_image_count(event) {
  return pastedImageFiles(event).length;
}

export async function upload_pasted_images(event) {
  const files = pastedImageFiles(event);
  if (!files.length) return [];
  return upload_files(files);
}

function dragDataHasFiles(event) {
  const dt = event?.dataTransfer;
  if (!dt) return false;
  const types = Array.from(dt.types || []);
  if (types.includes("Files")) return true;
  if (dt.items && Array.from(dt.items).some((item) => item.kind === "file")) return true;
  return !!dt.files?.length;
}

export function drag_has_files(event) {
  return dragDataHasFiles(event);
}

export function set_drag_copy(event) {
  const dt = event?.dataTransfer;
  if (!dt) return;
  try {
    dt.dropEffect = "copy";
  } catch (_) {
    // Synthetic events may expose a read-only dataTransfer.
  }
}

function nativeDropPointInside(payload, el) {
  if (!el || !payload) return false;
  const rect = el.getBoundingClientRect();
  const scale = window.devicePixelRatio || 1;
  const rawX = Number(payload.x || 0);
  const rawY = Number(payload.y || 0);
  const inside = (x, y) => x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
  return inside(rawX, rawY) || inside(rawX / scale, rawY / scale);
}

export function native_drop_in_composer(payload) {
  return nativeDropPointInside(payload, document.querySelector(".composer-inner"));
}

/** @returns {{ contextId: string, destinationDir: string } | null} */
export function native_drop_remote_target(payload) {
  const panel = document.querySelector(".rp-files");
  if (!nativeDropPointInside(payload, panel)) return null;
  const select = panel.querySelector(".fb-source");
  const pathInput = panel.querySelector(".fb-path-input");
  if (!select || !pathInput) return null;
  const contextId = String(select.value || "");
  if (!contextId || contextId === "local") return null;
  return { contextId, destinationDir: String(pathInput.value || "~") };
}

/** @param {string} inputId */
export async function upload_input_files(inputId) {
  const input = document.getElementById(inputId);
  if (!input?.files?.length) return [];
  const results = await upload_files(input.files);
  input.value = "";
  return results;
}

export async function listen(event, cb) {
  const bus = tauriEvent();
  if (!bus) {
    console.error(new Error(`Tauri event bridge is not available while listening for ${event}.`));
    return () => {};
  }
  return bus.listen(event, (e) => cb(e.payload));
}

// Register navigation events against this exact native window. The top-level
// event.listen API uses an application-wide target, so every open project
// window would otherwise receive an emit_to("proj-*", "open-session", ...)
// event and jump to the same completed conversation.
export async function listen_current_window(event, cb) {
  const current = window.__TAURI__?.window?.getCurrentWindow?.();
  if (!current?.listen) {
    console.error(new Error(`Tauri window event bridge is not available while listening for ${event}.`));
    return () => {};
  }
  return current.listen(event, (e) => cb(e.payload));
}

function normalizeNativeDropPayload(event) {
  const payload = event?.payload ?? event ?? {};
  const position = payload.position ?? {};
  return {
    kind: payload.kind ?? payload.type ?? "",
    paths: Array.isArray(payload.paths) ? payload.paths : [],
    x: Number(payload.x ?? position.x ?? 0),
    y: Number(payload.y ?? position.y ?? 0),
  };
}

export async function listen_native_file_drop(cb) {
  const unlisten = [];
  const push = (fn) => { if (typeof fn === "function") unlisten.push(fn); };
  const handle = (event) => cb(normalizeNativeDropPayload(event));
  try {
    const current =
      window.__TAURI__?.webviewWindow?.getCurrentWebviewWindow?.() ||
      window.__TAURI__?.window?.getCurrentWindow?.() ||
      window.__TAURI__?.webview?.getCurrentWebview?.();
    if (current?.onDragDropEvent) push(await current.onDragDropEvent(handle));
  } catch (err) {
    console.warn("Tauri native drag/drop listener unavailable", err);
  }
  const bus = tauriEvent();
  if (bus?.listen) {
    try { push(await bus.listen("native-file-drop", handle)); }
    catch (err) { console.warn("custom native-file-drop listener unavailable", err); }
  }
  return () => {
    for (const fn of unlisten) {
      try { fn(); } catch (_) { /* ignore cleanup failures */ }
    }
  };
}

const css = new Set();
function linkCss(href) {
  if (css.has(href)) return;
  const l = document.createElement("link");
  l.rel = "stylesheet";
  l.href = href;
  document.head.appendChild(l);
  css.add(href);
}

let katexMod;
async function katex() {
  if (!katexMod) {
    katexMod = (await import("/vendor-runtime/katex-Dn761jRB.js")).k;
    linkCss("/vendor-runtime/katex-DwwF5kvc.css");
  }
  return katexMod;
}

let rdkitInit;
async function rdkit() {
  if (!rdkitInit) {
    const mod = await import("/vendor-runtime/RDKit_minimal-B7RkdM0_.js");
    rdkitInit = mod.R.default();
  }
  return rdkitInit;
}

let mol3dLib;
async function loadMol3d() {
  if (!mol3dLib) {
    const mod = await import("/vendor-runtime/3Dmol-DfD4xImO.js");
    mol3dLib = mod._.default;
  }
  return mol3dLib;
}

let msaLoaded;
async function ensureMsa() {
  if (!msaLoaded) {
    await import("/vendor-runtime/nightingale-msa-5.6.0.js");
    msaLoaded = true;
  }
}

let pdfjsLib;
async function pdfjs() {
  if (!pdfjsLib) {
    pdfjsLib = import("/vendor-runtime/pdf.min.mjs").then((mod) => {
      // WebView2 does not ship a browser PDF plugin, so PDFs are rendered to
      // canvas with the worker bundled alongside the application.
      mod.GlobalWorkerOptions.workerSrc = "/vendor-runtime/pdf.worker.min.mjs";
      return mod;
    });
  }
  return pdfjsLib;
}

let docxLib;
function docxPreview() {
  // Self-contained ESM bundle (docx-preview + jszip, no bare imports) so .docx
  // renders fully offline in the WebView. See ui/sync-vendor.ps1.
  if (!docxLib) docxLib = import("/vendor-runtime/docx-preview.mjs");
  return docxLib;
}

function normalizeRawBytes(value) {
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }
  if (Array.isArray(value)) return Uint8Array.from(value);
  throw new Error("Binary preview command returned an unsupported payload");
}

// Chat media (generated images/videos, attachment thumbnails, inline resource
// images) used to inline as base64 data URLs — a 64 MB video became ~85 MB of
// string per card, and repeated loads under row remounts pushed the WebView
// renderer toward OOM (#dead-window). Instead, bytes are fetched through the
// same preview command family and handed to the browser as a blob object URL:
// decoded once by the media stack, shareable across cards with one entry per
// path, and revocable when evicted.
const MEDIA_URL_CACHE_LIMIT = 64;
const mediaUrlCache = new Map(); // path -> { url, mime }
const thumbnailJobs = new Map(); // path -> Promise<string | null>

function mediaBytesCommand(path) {
  // Mirrors `previewBytes`'s command selection for the four path spellings.
  if (path.startsWith("artifact-version:")) {
    return { command: "read_artifact_version_bytes", args: { versionId: path.slice("artifact-version:".length) } };
  }
  if (path.startsWith("artifact:")) {
    return { command: "read_artifact_bytes", args: { id: path.slice("artifact:".length) } };
  }
  if (path.startsWith("remote:ssh:")) {
    const withoutPrefix = path.slice("remote:ssh:".length);
    const separator = withoutPrefix.indexOf(":");
    if (separator <= 0 || separator === withoutPrefix.length - 1) {
      throw new Error("Remote media path is invalid");
    }
    return {
      command: "read_remote_file_bytes",
      args: {
        contextId: `ssh:${withoutPrefix.slice(0, separator)}`,
        path: withoutPrefix.slice(separator + 1),
      },
    };
  }
  return { command: "read_file_bytes", args: { path } };
}

export async function media_url(path) {
  const key = String(path || "");
  if (!key) return null;
  const hit = mediaUrlCache.get(key);
  if (hit) {
    // Refresh insertion order so eviction is LRU.
    mediaUrlCache.delete(key);
    mediaUrlCache.set(key, hit);
    return hit.url;
  }
  const { command, args } = mediaBytesCommand(key);
  // One shot rather than invoke: a missing file must surface as null (the
  // callers paint their fallback), not a console error.
  const core = tauriCore();
  if (!core) return null;
  let bytes;
  try {
    bytes = normalizeRawBytes(await core.invoke(command, args));
  } catch (_) {
    return null;
  }
  const mime = blobMime(bytes);
  const entry = { url: URL.createObjectURL(new Blob([bytes], { type: mime })), mime };
  mediaUrlCache.set(key, entry);
  if (mediaUrlCache.size > MEDIA_URL_CACHE_LIMIT) {
    // Drop the lookup only. The URL may still be an <img>/<video> src
    // (and media_thumbnail_url reuses it when the image is already small).
    const oldest = mediaUrlCache.keys().next().value;
    mediaUrlCache.delete(oldest);
  }
  return entry.url;
}

// Thumbnails for attachment/artifact cards: a small canvas re-encode instead
// of the full-resolution object URL, so a 20-message history of pasted photos
// does not keep 20 decoded full-size bitmaps alive.
const THUMB_MAX_EDGE = 384;
// path -> downscaled blob URL. Kept (never revoked alongside the media cache)
// because a thumbnail URL handed to the DOM must stay valid for the DOM's
// lifetime; the thumbs are ≤384px re-encodes, so a bounded count of them is
// the cheap side of the trade.
const THUMB_CACHE_LIMIT = 128;
const thumbnailCache = new Map();

export async function media_thumbnail_url(path) {
  const key = String(path || "");
  if (!key) return null;
  const cached = thumbnailCache.get(key);
  if (cached !== undefined) return cached;
  const pending = thumbnailJobs.get(key);
  if (pending) return pending;
  const job = (async () => {
    const url = await media_url(key);
    if (!url) {
      thumbnailCache.set(key, null);
      return null;
    }
    let thumb;
    try {
      thumb = await downscaleToPngBlobUrl(url, THUMB_MAX_EDGE);
    } catch (_) {
      thumb = url; // non-decodable or huge image: show it as-is
    }
    thumbnailCache.set(key, thumb);
    if (thumbnailCache.size > THUMB_CACHE_LIMIT) {
      // Drop the oldest entry's cache slot only; its URL may still be in the
      // DOM, so revoking here would blank a live thumbnail.
      const oldest = thumbnailCache.keys().next().value;
      thumbnailCache.delete(oldest);
    }
    return thumb;
  })();
  thumbnailJobs.set(key, job.finally(() => thumbnailJobs.delete(key)));
  return job;
}

function downscaleToPngBlobUrl(url, maxEdge) {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => {
      try {
        const scale = Math.min(1, maxEdge / Math.max(img.naturalWidth, img.naturalHeight));
        if (scale >= 1) {
          resolve(url); // already small enough; reuse the media URL
          return;
        }
        const canvas = document.createElement("canvas");
        canvas.width = Math.max(1, Math.round(img.naturalWidth * scale));
        canvas.height = Math.max(1, Math.round(img.naturalHeight * scale));
        canvas.getContext("2d").drawImage(img, 0, 0, canvas.width, canvas.height);
        canvas.toBlob((blob) => {
          if (!blob) {
            resolve(url);
            return;
          }
          resolve(URL.createObjectURL(blob));
        }, "image/png");
      } catch (err) {
        reject(err);
      }
    };
    img.onerror = () => reject(new Error("image decode failed"));
    img.src = url;
  });
}

function blobMime(bytes) {
  // The byte-returning commands omit the MIME; sniff the signatures that
  // matter for chat media. Defaults to application/octet-stream, which still
  // plays in <video> via extension-less blob sniffing in Chromium.
  if (bytes.length >= 12 && bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47) return "image/png";
  if (bytes.length >= 3 && bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) return "image/jpeg";
  if (bytes.length >= 4 && bytes[0] === 0x47 && bytes[1] === 0x49 && bytes[2] === 0x46) return "image/gif";
  if (bytes.length >= 12 && bytes[4] === 0x66 && bytes[5] === 0x74 && bytes[6] === 0x79 && bytes[7] === 0x70) return "video/mp4";
  // "<?xml" or "<svg" plaintext: the SVG fixtures (and SVG artifacts) decode
  // as images only when served with an image/* MIME.
  const head = new TextDecoder().decode(bytes.slice(0, 256)).trimStart().toLowerCase();
  if (head.startsWith("<?xml") || head.startsWith("<svg")) return "image/svg+xml";
  return "application/octet-stream";
}


async function previewBytes(payload) {
  if (payload.bytes) return normalizeRawBytes(payload.bytes);
  if (payload.b64) return base64Bytes(payload.b64);
  const path = String(payload.path || "");
  if (!path) throw new Error("Preview path is empty");
  // 100 MB matches the backend's local preview ceiling (and the upload cap);
  // the backend clamps remote reads to 32 MB on its own.
  const maxBytes = Math.min(Number(payload.maxBytes) || 32 * 1024 * 1024, 100 * 1024 * 1024);

  let command = "read_file_bytes";
  let args = { path, maxBytes };
  if (path.startsWith("artifact-version:")) {
    command = "read_artifact_version_bytes";
    args = { versionId: path.slice("artifact-version:".length), maxBytes };
  } else if (path.startsWith("artifact:")) {
    command = "read_artifact_bytes";
    args = { id: path.slice("artifact:".length), maxBytes };
  } else if (path.startsWith("remote:ssh:")) {
    const withoutPrefix = path.slice("remote:ssh:".length);
    const separator = withoutPrefix.indexOf(":");
    if (separator <= 0 || separator === withoutPrefix.length - 1) {
      throw new Error("Remote preview path is invalid");
    }
    command = "read_remote_file_bytes";
    args = {
      contextId: `ssh:${withoutPrefix.slice(0, separator)}`,
      path: withoutPrefix.slice(separator + 1),
      maxBytes,
    };
  }
  return normalizeRawBytes(await invoke_strict(command, args));
}

async function renderDocx(el, payload) {
  cleanupPreview(el);
  const renderToken = Symbol("docx-preview");
  el.__wispPreviewToken = renderToken;
  const loading = document.createElement("div");
  loading.className = "rp-pdf-loading";
  loading.textContent = payload.loading || "Loading…";
  el.replaceChildren(loading);
  try {
    const bytes = await previewBytes(payload);
    const lib = await docxPreview();
    if (!el.isConnected || el.__wispPreviewToken !== renderToken) return;
    const container = document.createElement("div");
    container.className = "rp-docx";
    el.replaceChildren(container);
    // renderAsync takes a Blob/ArrayBuffer; ignoreHeight lets the page reflow to
    // the preview column instead of a fixed A4 height. `experimental` enables
    // docx-preview's fuller feature set (incl. its OMML→MathML math rendering).
    // OMML support covers standard Word math; WPS's OMML dialect is only
    // partially handled upstream, so some WPS formulas can still garble (#274).
    await lib.renderAsync(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength), container, null, {
      className: "docx",
      inWrapper: true,
      ignoreWidth: false,
      ignoreHeight: true,
      breakPages: true,
      experimental: true,
    });
    if (el.__wispPreviewToken !== renderToken) return;
    el.__wispPreviewCleanup = () => { container.replaceChildren(); };
  } catch (error) {
    console.error("Failed to render DOCX preview", error);
    if (el.isConnected && el.__wispPreviewToken === renderToken) {
      const message = document.createElement("div");
      message.className = "rp-error rp-pdf-error";
      message.textContent = payload.error || "Unable to preview this document.";
      el.replaceChildren(message);
    }
  }
}

function parseWorkbookInWorker(bytes, signal, timeoutMs = 15_000) {
  return new Promise((resolve, reject) => {
    const worker = new Worker("/vendor-runtime/xlsx-worker.js");
    let settled = false;
    const finish = (callback, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      signal?.removeEventListener("abort", onAbort);
      worker.terminate();
      callback(value);
    };
    const onAbort = () => finish(reject, new DOMException("Aborted", "AbortError"));
    const timer = setTimeout(
      () => finish(reject, new Error("Workbook parsing timed out")),
      timeoutMs,
    );
    worker.onerror = (event) => finish(reject, new Error(event.message || "Workbook worker failed"));
    worker.onmessage = ({ data }) => {
      if (data?.ok) finish(resolve, data.workbook);
      else finish(reject, new Error(data?.error || "Unable to parse workbook"));
    };
    signal?.addEventListener("abort", onAbort, { once: true });
    if (signal?.aborted) {
      onAbort();
      return;
    }
    const copy = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
    worker.postMessage(copy, [copy]);
  });
}

function spreadsheetColumnName(index) {
  let value = index + 1;
  let name = "";
  while (value > 0) {
    value -= 1;
    name = String.fromCharCode(65 + (value % 26)) + name;
    value = Math.floor(value / 26);
  }
  return name;
}

function safeSpreadsheetLink(value) {
  try {
    const url = new URL(value);
    return ["http:", "https:", "mailto:"].includes(url.protocol) ? url.href : null;
  } catch (_) {
    return null;
  }
}

function mountWorkbookSheet(root, sheet, payload) {
  const ROW_HEIGHT = 28;
  const COL_WIDTH = 140;
  const ROW_HEADER_WIDTH = 52;
  const COL_HEADER_HEIGHT = 28;
  const formula = root.querySelector(".rp-xlsx-formula-value");
  const viewport = root.querySelector(".rp-xlsx-grid");
  const content = document.createElement("div");
  content.className = "rp-xlsx-content";
  content.style.width = `${ROW_HEADER_WIDTH + sheet.cols * COL_WIDTH}px`;
  content.style.height = `${COL_HEADER_HEIGHT + sheet.rows * ROW_HEIGHT}px`;
  viewport.replaceChildren(content);

  const cellMap = new Map(sheet.cells.map((cell) => [`${cell.row}:${cell.col}`, cell]));
  let frame = 0;
  const render = () => {
    frame = 0;
    const rowStart = Math.max(0, Math.floor((viewport.scrollTop - COL_HEADER_HEIGHT) / ROW_HEIGHT) - 1);
    const rowEnd = Math.min(sheet.rows, Math.ceil((viewport.scrollTop + viewport.clientHeight) / ROW_HEIGHT) + 2);
    const colStart = Math.max(0, Math.floor((viewport.scrollLeft - ROW_HEADER_WIDTH) / COL_WIDTH) - 1);
    const colEnd = Math.min(sheet.cols, Math.ceil((viewport.scrollLeft + viewport.clientWidth) / COL_WIDTH) + 2);
    const visibleMerges = sheet.merges.filter((merge) => (
      merge.endRow >= rowStart && merge.startRow < rowEnd
      && merge.endCol >= colStart && merge.startCol < colEnd
    ));
    const covered = new Set();
    const anchors = new Map();
    for (const merge of visibleMerges) {
      anchors.set(`${merge.startRow}:${merge.startCol}`, merge);
      for (let row = Math.max(rowStart, merge.startRow); row <= Math.min(rowEnd - 1, merge.endRow); row += 1) {
        for (let col = Math.max(colStart, merge.startCol); col <= Math.min(colEnd - 1, merge.endCol); col += 1) {
          if (row !== merge.startRow || col !== merge.startCol) covered.add(`${row}:${col}`);
        }
      }
    }

    const fragment = document.createDocumentFragment();
    for (let row = rowStart; row < rowEnd; row += 1) {
      const header = document.createElement("div");
      header.className = "rp-xlsx-row-head";
      header.textContent = String(row + 1);
      header.style.transform = `translate(${viewport.scrollLeft}px, ${COL_HEADER_HEIGHT + row * ROW_HEIGHT}px)`;
      fragment.appendChild(header);
      for (let col = colStart; col < colEnd; col += 1) {
        const key = `${row}:${col}`;
        if (covered.has(key)) continue;
        const cell = cellMap.get(key);
        const node = document.createElement("div");
        node.className = "rp-xlsx-cell";
        node.style.transform = `translate(${ROW_HEADER_WIDTH + col * COL_WIDTH}px, ${COL_HEADER_HEIGHT + row * ROW_HEIGHT}px)`;
        const merge = anchors.get(key);
        if (merge) {
          node.style.width = `${(merge.endCol - merge.startCol + 1) * COL_WIDTH}px`;
          node.style.height = `${(merge.endRow - merge.startRow + 1) * ROW_HEIGHT}px`;
          node.classList.add("merged");
        }
        const href = cell?.hyperlink && safeSpreadsheetLink(cell.hyperlink);
        if (href) {
          const link = document.createElement("a");
          link.href = href;
          link.target = "_blank";
          link.rel = "noopener noreferrer";
          link.textContent = cell.text;
          node.appendChild(link);
        } else {
          node.textContent = cell?.text || "";
        }
        node.title = cell?.text || "";
        node.addEventListener("click", () => {
          content.querySelector(".rp-xlsx-cell.selected")?.classList.remove("selected");
          node.classList.add("selected");
          formula.textContent = cell?.formula ? `=${cell.formula}` : (cell?.text || "");
        });
        fragment.appendChild(node);
      }
    }
    for (let col = colStart; col < colEnd; col += 1) {
      const header = document.createElement("div");
      header.className = "rp-xlsx-col-head";
      header.textContent = spreadsheetColumnName(col);
      header.style.transform = `translate(${ROW_HEADER_WIDTH + col * COL_WIDTH}px, ${viewport.scrollTop}px)`;
      fragment.appendChild(header);
    }
    const corner = document.createElement("div");
    corner.className = "rp-xlsx-corner";
    corner.style.transform = `translate(${viewport.scrollLeft}px, ${viewport.scrollTop}px)`;
    fragment.appendChild(corner);
    content.replaceChildren(fragment);
  };
  const onScroll = () => {
    if (!frame) frame = requestAnimationFrame(render);
  };
  viewport.addEventListener("scroll", onScroll, { passive: true });
  render();
  return () => {
    viewport.removeEventListener("scroll", onScroll);
    if (frame) cancelAnimationFrame(frame);
  };
}

async function renderXlsx(el, payload) {
  cleanupPreview(el);
  const renderToken = Symbol("xlsx-preview");
  const abortController = new AbortController();
  el.__wispPreviewToken = renderToken;
  el.__wispPreviewCleanup = () => abortController.abort();
  const loading = document.createElement("div");
  loading.className = "rp-pdf-loading";
  loading.textContent = payload.loading || "Loading…";
  el.replaceChildren(loading);
  try {
    const bytes = await previewBytes(payload);
    const workbook = await parseWorkbookInWorker(bytes, abortController.signal);
    if (!el.isConnected || el.__wispPreviewToken !== renderToken) return;
    if (!workbook.sheets.length) throw new Error("Workbook contains no worksheets");

    const root = document.createElement("div");
    root.className = "rp-xlsx";
    const tabs = document.createElement("div");
    tabs.className = "rp-xlsx-tabs";
    const formulaBar = document.createElement("div");
    formulaBar.className = "rp-xlsx-formula";
    const formulaLabel = document.createElement("span");
    formulaLabel.textContent = payload.formulaLabel || "Formula";
    const formulaValue = document.createElement("code");
    formulaValue.className = "rp-xlsx-formula-value";
    formulaBar.append(formulaLabel, formulaValue);
    const grid = document.createElement("div");
    grid.className = "rp-xlsx-grid";
    root.append(tabs, formulaBar, grid);
    if (workbook.truncated) {
      const warning = document.createElement("div");
      warning.className = "rp-xlsx-warning";
      warning.textContent = payload.truncated || "Large workbook: only a bounded preview is shown.";
      root.prepend(warning);
    }
    el.replaceChildren(root);

    let cleanupSheet = () => {};
    const showSheet = (index) => {
      cleanupSheet();
      tabs.querySelector(".active")?.classList.remove("active");
      tabs.children[index]?.classList.add("active");
      formulaBar.querySelector("code").textContent = "";
      cleanupSheet = mountWorkbookSheet(root, workbook.sheets[index], payload);
    };
    workbook.sheets.forEach((sheet, index) => {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = sheet.name;
      button.title = `${sheet.name} · ${sheet.originalRows.toLocaleString()} × ${sheet.originalCols.toLocaleString()}`;
      button.addEventListener("click", () => showSheet(index));
      tabs.appendChild(button);
    });
    showSheet(0);
    el.__wispPreviewCleanup = () => {
      abortController.abort();
      cleanupSheet();
      root.replaceChildren();
    };
  } catch (error) {
    if (abortController.signal.aborted) return;
    console.error("Failed to render XLSX preview", error);
    if (el.isConnected && el.__wispPreviewToken === renderToken) {
      const message = document.createElement("div");
      message.className = "rp-error rp-pdf-error";
      message.textContent = payload.error || "Unable to preview this workbook.";
      el.replaceChildren(message);
    }
  }
}

let pptxLib;
function pptxPreview() {
  if (!pptxLib) pptxLib = import("/vendor-runtime/pptx-preview.mjs");
  return pptxLib;
}

async function renderPptx(el, payload) {
  cleanupPreview(el);
  const renderToken = Symbol("pptx-preview");
  const abortController = new AbortController();
  el.__wispPreviewToken = renderToken;
  el.__wispPreviewCleanup = () => abortController.abort();
  const loading = document.createElement("div");
  loading.className = "rp-pdf-loading";
  loading.textContent = payload.loading || "Loading…";
  el.replaceChildren(loading);
  let viewer;
  try {
    const [bytes, lib] = await Promise.all([previewBytes(payload), pptxPreview()]);
    if (!el.isConnected || el.__wispPreviewToken !== renderToken) return;
    const container = document.createElement("div");
    container.className = "rp-pptx";
    el.replaceChildren(container);
    viewer = await lib.PptxViewer.open(bytes, container, {
      zipLimits: lib.RECOMMENDED_ZIP_LIMITS,
      lazySlides: true,
      lazyMedia: true,
      scrollContainer: container,
      listOptions: {
        windowed: true,
        initialSlides: 4,
        batchSize: 4,
        overscanViewport: 1.5,
        showSlideLabels: true,
      },
      signal: abortController.signal,
      pdfjs: {
        moduleUrl: "/vendor-runtime/pdf.min.mjs",
        workerUrl: "/vendor-runtime/pdf.worker.min.mjs",
      },
    });
    if (!el.isConnected || el.__wispPreviewToken !== renderToken) {
      viewer.destroy();
      return;
    }
    el.__wispPreviewCleanup = () => {
      abortController.abort();
      viewer?.destroy();
    };
  } catch (error) {
    if (abortController.signal.aborted) return;
    console.error("Failed to render PPTX preview", error);
    viewer?.destroy();
    if (el.isConnected && el.__wispPreviewToken === renderToken) {
      const message = document.createElement("div");
      message.className = "rp-error rp-pdf-error";
      message.textContent = payload.error || "Unable to preview this presentation.";
      el.replaceChildren(message);
    }
  }
}

function base64Bytes(value) {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

function pdfPageLabel(template, page, total) {
  return String(template || `Page ${page} of ${total}`)
    .replace("{page}", String(page))
    .replace("{total}", String(total));
}

function cleanupPreview(el) {
  if (typeof el?.__wispPreviewCleanup === "function") {
    try {
      el.__wispPreviewCleanup();
    } catch (error) {
      console.warn("Failed to clean up preview", error);
    }
  }
  delete el.__wispPreviewCleanup;
  delete el.__wispPreviewToken;
}

function pdfNavIcon(direction) {
  const path = direction < 0 ? "m15 18-6-6 6-6" : "m9 18 6-6-6-6";
  return `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="${path}"></path></svg>`;
}

/**
 * Read the current text selection if it falls inside a file preview surface
 * (anything tagged `data-file-path`). Returns a JSON string {text, path, x, y}
 * positioned at the selection for a floating quote/annotate toolbar, or "" when
 * there is no usable selection. Kept in JS because it walks the DOM + Selection
 * API, which is far terser here than through web-sys.
 */
export function preview_selection(fallbackX = 0, fallbackY = 0) {
  // The editable source view is a textarea: its selection never appears in
  // window.getSelection(), so read it directly. Coordinates come from the
  // mouseup that raised the popup — a textarea selection has no client rects.
  // A focused textarea without its own file selection (e.g. the composer)
  // falls through to the ordinary DOM-selection path below.
  const active = document.activeElement;
  if (active && active.tagName?.toLowerCase?.() === "textarea") {
    const container = active.closest?.("[data-file-path]");
    const start = active.selectionStart ?? 0;
    const end = active.selectionEnd ?? 0;
    if (container && end > start) {
      const text = active.value.slice(start, end).trim();
      if (text) {
        return JSON.stringify({
          text,
          path: container.getAttribute("data-file-path") || "",
          x: Math.round(fallbackX),
          y: Math.round(fallbackY),
        });
      }
    }
  }
  const sel = window.getSelection?.();
  if (!sel || sel.isCollapsed || sel.rangeCount === 0) return "";
  const text = sel.toString().trim();
  if (!text) return "";
  const range = sel.getRangeAt(0);
  let node = range.commonAncestorContainer;
  if (node && node.nodeType === 3) node = node.parentElement;
  const container = node && node.closest ? node.closest("[data-file-path]") : null;
  if (!container) return "";
  // A Range's bounding box spans the whole selection and its bottom is the
  // same regardless of drag direction. Anchor to the topmost rendered text
  // fragment instead so an upward selection does not leave the toolbar at the
  // opposite end of a long passage. Empty rects can be emitted around line
  // breaks; fall back to the bounding box for unusual Selection API clients.
  const rect = Array.from(range.getClientRects())
    .filter((candidate) => candidate.width > 0 && candidate.height > 0)
    .reduce((topmost, candidate) => {
      if (!topmost || candidate.top < topmost.top) return candidate;
      if (candidate.top === topmost.top && candidate.left < topmost.left) return candidate;
      return topmost;
    }, null) || range.getBoundingClientRect();
  return JSON.stringify({
    text,
    path: container.getAttribute("data-file-path") || "",
    x: Math.round(rect.left + rect.width / 2),
    y: Math.round(rect.top),
  });
}

/** Drop the active selection once its text has been quoted/annotated. */
export function clear_selection() {
  window.getSelection?.().removeAllRanges();
}

function eventTargetsEditable(target) {
  for (let node = target; node; node = node.parentElement) {
    const tag = node.tagName?.toLowerCase?.();
    if (
      tag === "input" ||
      tag === "textarea" ||
      tag === "select" ||
      node.hasAttribute?.("contenteditable")
    ) {
      return true;
    }
  }
  return false;
}

async function renderPdf(el, payload) {
  cleanupPreview(el);
  const renderToken = Symbol("pdf-preview");
  el.__wispPreviewToken = renderToken;

  const loading = document.createElement("div");
  loading.className = "rp-pdf-loading";
  loading.textContent = payload.loading || "Loading…";
  el.replaceChildren(loading);

  let task;
  let pdf;
  let renderTask;
  let disconnectObserver;
  let resizeObserver;
  let refitTimer;
  let currentPage = 1;
  let rendering = false;
  let disposed = false;
  // Width the current canvas was rasterised for; a resize past it means the page
  // is being upscaled/downscaled by the browser and needs a re-render.
  let renderedFitWidth = null;
  try {
    const bytes = payload.path || payload.b64 || payload.bytes
      ? await previewBytes(payload)
      : null;
    const lib = await pdfjs();
    const source = bytes ? { data: bytes } : payload.url ? { url: payload.url } : null;
    if (!source) throw new Error("PDF data is empty");
    // PDF.js 5.x decodes JPEG2000 (JPXDecode) figures and ICC colors via WASM
    // fetched from wasmUrl; without it the images silently drop while text still
    // renders. The decoders ship in ui/vendor-runtime next to the worker.
    source.wasmUrl = "/vendor-runtime/";

    task = lib.getDocument(source);
    pdf = await task.promise;
    if (!el.isConnected || el.__wispPreviewToken !== renderToken) {
      return;
    }

    const root = document.createElement("div");
    root.className = "rp-pdf";
    root.setAttribute("data-page-count", String(pdf.numPages));

    const toolbar = document.createElement("div");
    toolbar.className = "rp-pdf-toolbar";

    const nav = document.createElement("div");
    nav.className = "rp-pdf-nav";

    const prevButton = document.createElement("button");
    prevButton.type = "button";
    prevButton.className = "rp-pdf-nav-btn";
    prevButton.setAttribute("aria-label", payload.prevPage || "Previous page");
    prevButton.setAttribute(
      "title",
      `${payload.prevPage || "Previous page"} (Page Up)`,
    );
    prevButton.innerHTML = pdfNavIcon(-1);

    const pageIndicator = document.createElement("div");
    pageIndicator.className = "rp-pdf-page-indicator";
    pageIndicator.setAttribute("role", "status");
    pageIndicator.setAttribute("aria-live", "polite");

    const nextButton = document.createElement("button");
    nextButton.type = "button";
    nextButton.className = "rp-pdf-nav-btn";
    nextButton.setAttribute("aria-label", payload.nextPage || "Next page");
    nextButton.setAttribute(
      "title",
      `${payload.nextPage || "Next page"} (Page Down)`,
    );
    nextButton.innerHTML = pdfNavIcon(1);

    // The zoom viewport owns pointer drags for panning; do not let it swallow
    // clicks on the page navigation controls once the preview is zoomed in.
    nav.addEventListener("pointerdown", (event) => event.stopPropagation());
    nav.append(prevButton, pageIndicator, nextButton);

    // Page nav and zoom are one control set, so they share one bar. The zoom bar
    // is Leptos-owned and sits outside .file-preview-zoom-content, which also
    // keeps the nav from scaling with the page. Previews mounted without the
    // zoom wrapper (right pane, plain modal) keep the toolbar inside .rp-pdf.
    const zoomBar = el
      .closest(".file-preview-zoom")
      ?.querySelector(".file-preview-zoom-bar");
    if (zoomBar) {
      zoomBar.prepend(nav);
    } else {
      toolbar.appendChild(nav);
    }

    const viewer = document.createElement("div");
    viewer.className = "rp-pdf-viewer";

    root.append(...(zoomBar ? [viewer] : [toolbar, viewer]));
    el.replaceChildren(root);

    const syncControls = () => {
      root.setAttribute("data-current-page", String(currentPage));
      pageIndicator.textContent = pdfPageLabel(payload.pageLabel, currentPage, pdf.numPages);
      prevButton.disabled = rendering || currentPage <= 1;
      nextButton.disabled = rendering || currentPage >= pdf.numPages;
    };

    const showPageError = (error) => {
      if (error?.name === "RenderingCancelledException" || disposed) {
        return;
      }
      console.error("Failed to render PDF page", error);
      const message = document.createElement("div");
      message.className = "rp-error rp-pdf-error";
      message.textContent = payload.error || "Unable to preview this PDF.";
      el.replaceChildren(message);
      el.__wispPreviewCleanup?.();
    };

    // Fit-to-width base for the page, independent of --preview-zoom: the zoom is
    // a pure CSS multiple of this. Read off the viewer, whose width tracks the
    // pane and not the (possibly zoomed) page inside it.
    const fitWidth = () =>
      Math.max(240, Math.min(viewer.clientWidth || el.clientWidth || 800, 1000));

    const renderPage = async (pageNumber) => {
      rendering = true;
      syncControls();

      const page = await pdf.getPage(pageNumber);
      try {
        // Render at up to 2x the displayed width so text remains crisp on HiDPI
        // screens without making the single-page preview consume unbounded canvas memory.
        const availableWidth = fitWidth();
        renderedFitWidth = availableWidth;
        const pixelRatio = Math.min(window.devicePixelRatio || 1, 2);
        const natural = page.getViewport({ scale: 1 });
        const cssScale = availableWidth / natural.width;
        const viewport = page.getViewport({ scale: cssScale * pixelRatio });
        const wrapper = document.createElement("div");
        wrapper.className = "rp-pdf-page";
        wrapper.dataset.page = String(pageNumber);
        // The page itself is the only thing the zoom scales: availableWidth is
        // the fit-to-width base and --preview-zoom multiplies it. This must be
        // `width`, not `max-width` — .rp-pdf-page is width:100% of the viewer,
        // so a max-width above 100% would never win and zoom-in would no-op.
        wrapper.style.width =
          `calc(${Math.floor(availableWidth)}px * var(--preview-zoom, 1))`;
        wrapper.setAttribute(
          "aria-label",
          pdfPageLabel(payload.pageLabel, pageNumber, pdf.numPages),
        );
        const canvas = document.createElement("canvas");
        canvas.width = Math.max(1, Math.floor(viewport.width));
        canvas.height = Math.max(1, Math.floor(viewport.height));
        canvas.setAttribute("role", "img");
        wrapper.appendChild(canvas);

        const context = canvas.getContext("2d", { alpha: false });
        if (!context) throw new Error("Canvas is not available");

        renderTask = page.render({ canvasContext: context, viewport });
        await renderTask.promise;
        if (!el.isConnected || el.__wispPreviewToken !== renderToken || disposed) {
          return;
        }

        // Transparent selectable text layer over the canvas, at CSS scale (no
        // pixelRatio) so glyphs align to the displayed page. This is what makes
        // PDF text selectable → "add to chat" (the preview's data-file-path
        // ancestor drives the shared selection popup). Fail-soft: a text-layer
        // error must not blank the rendered page.
        try {
          const cssViewport = page.getViewport({ scale: cssScale });
          const textLayerDiv = document.createElement("div");
          textLayerDiv.className = "rp-pdf-textlayer textLayer";
          textLayerDiv.style.setProperty("--scale-factor", String(cssScale));
          textLayerDiv.style.setProperty(
            "--total-scale-factor",
            `calc(${cssScale} * var(--preview-zoom, 1))`,
          );
          const textLayer = new lib.TextLayer({
            textContentSource: page.streamTextContent(),
            container: textLayerDiv,
            viewport: cssViewport,
          });
          await textLayer.render();
          if (!el.isConnected || el.__wispPreviewToken !== renderToken || disposed) {
            return;
          }
          wrapper.appendChild(textLayerDiv);
        } catch (error) {
          if (error?.name !== "RenderingCancelledException") {
            console.warn("PDF text layer failed", error);
          }
        }

        wrapper.dataset.rendered = "true";
        viewer.replaceChildren(wrapper);
      } finally {
        renderTask = undefined;
        rendering = false;
        syncControls();
        page.cleanup();
      }
    };

    const setPage = (pageNumber) => {
      if (
        rendering ||
        pageNumber < 1 ||
        pageNumber > pdf.numPages ||
        pageNumber === currentPage
      ) {
        return;
      }
      currentPage = pageNumber;
      void renderPage(pageNumber).catch(showPageError);
    };

    const stepPage = (delta) => setPage(currentPage + delta);
    prevButton.addEventListener("click", () => stepPage(-1));
    nextButton.addEventListener("click", () => stepPage(1));

    // Page navigation by keyboard: Page Up/Down and the arrow keys step pages.
    // (Zoom is the wheel gesture, handled by the ZoomableFilePreview wrapper.)
    const onKeyDown = (event) => {
      if (
        event.defaultPrevented ||
        event.altKey ||
        event.ctrlKey ||
        event.metaKey ||
        event.shiftKey ||
        eventTargetsEditable(event.target)
      ) {
        return;
      }
      if (event.key === "PageUp" || event.key === "ArrowUp" || event.key === "ArrowLeft") {
        event.preventDefault();
        stepPage(-1);
      } else if (event.key === "PageDown" || event.key === "ArrowDown" || event.key === "ArrowRight") {
        event.preventDefault();
        stepPage(1);
      }
    };

    if (el.closest(".artifact-modal")) {
      document.addEventListener("keydown", onKeyDown);
    }

    const cleanup = () => {
      if (disposed) return;
      disposed = true;
      // When portalled into the zoom bar the nav lives outside el, so it
      // survives the el.replaceChildren() on the error paths — take it down here.
      nav.remove();
      document.removeEventListener("keydown", onKeyDown);
      clearTimeout(refitTimer);
      resizeObserver?.disconnect();
      disconnectObserver?.disconnect();
      if (renderTask) {
        try {
          renderTask.cancel();
        } catch {
          /* ignore cancellation races */
        }
      }
      if (pdf) {
        const currentPdf = pdf;
        pdf = undefined;
        void currentPdf.destroy().catch((error) => {
          console.warn("Failed to release PDF preview resources", error);
        });
      } else if (task) {
        const currentTask = task;
        task = undefined;
        void currentTask.destroy().catch((error) => {
          console.warn("Failed to release PDF loading task", error);
        });
      }
    };
    el.__wispPreviewCleanup = cleanup;

    const observerTarget = document.body || document.documentElement;
    if (observerTarget) {
      disconnectObserver = new MutationObserver(() => {
        if (!el.isConnected) cleanup();
      });
      disconnectObserver.observe(observerTarget, { childList: true, subtree: true });
    }

    // The canvas is rasterised for one fitWidth, so a pane resize (split toggled,
    // window resized, right pane dragged) otherwise leaves the page frozen at the
    // width it was first rendered at. Debounced; re-queues rather than running
    // while a render is in flight, so two renderTasks never race for the viewer.
    // The initial observation fires harmlessly — by then renderedFitWidth matches.
    const scheduleRefit = () => {
      clearTimeout(refitTimer);
      refitTimer = setTimeout(() => {
        if (disposed || !el.isConnected) return;
        if (rendering) {
          scheduleRefit();
          return;
        }
        if (fitWidth() === renderedFitWidth) return;
        void renderPage(currentPage).catch(showPageError);
      }, 150);
    };
    resizeObserver = new ResizeObserver(scheduleRefit);
    resizeObserver.observe(viewer);

    syncControls();
    await renderPage(currentPage);
  } catch (error) {
    if (error?.name === "RenderingCancelledException") {
      return;
    }
    console.error("Failed to render PDF preview", error);
    el.__wispPreviewCleanup?.();
    if (el.isConnected && el.__wispPreviewToken === renderToken) {
      const message = document.createElement("div");
      message.className = "rp-error rp-pdf-error";
      message.textContent = payload.error || "Unable to preview this PDF.";
      el.replaceChildren(message);
    }
  }
}

function escHtml(s) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function escAttr(s) {
  return String(s).replace(/&/g, "&amp;").replace(/"/g, "&quot;");
}

function htmlBaseHref(path) {
  if (typeof path !== "string" || !path) return "";
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(path)) {
    return path.replace(/[^/]*$/, "");
  }
  if (path.startsWith("/")) {
    return `file://${path.replace(/[^/]*$/, "")}`;
  }
  return "";
}

function injectBaseHref(html, baseHref) {
  if (!baseHref || /<base\s/i.test(html)) return html;
  const baseTag = `<base href="${escAttr(baseHref)}">`;
  if (/<head(\s[^>]*)?>/i.test(html)) {
    return html.replace(/<head(\s[^>]*)?>/i, (m) => `${m}${baseTag}`);
  }
  return `<!doctype html><html><head>${baseTag}</head><body>${html}</body></html>`;
}

function injectResponsiveHtmlPreview(html, baseHref) {
  const withBase = injectBaseHref(html, baseHref);
  const viewportTag = '<meta name="viewport" content="width=device-width, initial-scale=1">';
  const previewStyle = `<style>
html, body { max-width: 100%; overflow-x: hidden; }
body { margin-left: auto !important; margin-right: auto !important; }
img, svg, canvas, video, iframe, embed, object { max-width: 100% !important; height: auto !important; }
table { max-width: 100%; }
</style>`;
  const resizeScript = `<script>
(() => {
  const setHeight = () => {
    const doc = document.documentElement;
    const body = document.body;
    const height = Math.max(
      doc ? doc.scrollHeight : 0,
      doc ? doc.offsetHeight : 0,
      body ? body.scrollHeight : 0,
      body ? body.offsetHeight : 0
    );
    if (window.frameElement) {
      window.frameElement.style.height = Math.max(height, 320) + "px";
    }
  };
  addEventListener("load", () => {
    setHeight();
    requestAnimationFrame(setHeight);
    setTimeout(setHeight, 60);
  });
  addEventListener("resize", setHeight);
  if (window.ResizeObserver && document.body) {
    const ro = new ResizeObserver(setHeight);
    ro.observe(document.documentElement);
    ro.observe(document.body);
  }
})();
</script>`;
  let out = withBase;
  if (!/<meta\s+name=["']viewport["']/i.test(out)) {
    if (/<head(\s[^>]*)?>/i.test(out)) {
      out = out.replace(/<head(\s[^>]*)?>/i, (m) => `${m}${viewportTag}`);
    } else {
      out = `${viewportTag}${out}`;
    }
  }
  if (/<head(\s[^>]*)?>/i.test(out)) {
    out = out.replace(/<head(\s[^>]*)?>/i, (m) => `${m}${previewStyle}`);
  } else {
    out = `${previewStyle}${out}`;
  }
  if (/<body(\s[^>]*)?>/i.test(out)) {
    out = out.replace(/<\/body>/i, `${resizeScript}</body>`);
    if (!out.includes(resizeScript)) out += resizeScript;
  } else {
    out += resizeScript;
  }
  return out;
}

const mcpAppInstances = new Map();
let mcpAppParkingRoot = null;
let wispAppVersion = "0.0.0";
window.__TAURI__?.app?.getVersion?.().then((version) => { wispAppVersion = version; });

function injectMcpAppCsp(html, resourceMeta) {
  const csp = resourceMeta?.ui?.csp || resourceMeta?.csp || {};
  const safeOrigins = (values, websocket = false) => (Array.isArray(values) ? values : [])
    .filter((value) => typeof value === "string"
      && new RegExp(`^(?:https${websocket ? "|wss" : ""}):\\/\\/(?:\\*\\.)?[a-z0-9.-]+(?::\\d+)?$`, "i").test(value));
  const connect = safeOrigins(csp.connectDomains, true);
  const resources = safeOrigins(csp.resourceDomains);
  const frames = safeOrigins(csp.frameDomains);
  const bases = safeOrigins(csp.baseUriDomains);
  const policy = [
    "default-src 'none'",
    `script-src 'unsafe-inline' 'unsafe-eval' blob: ${resources.join(" ")}`.trim(),
    `style-src 'unsafe-inline' ${resources.join(" ")}`.trim(),
    `img-src data: blob: ${resources.join(" ")}`.trim(),
    `font-src data: ${resources.join(" ")}`.trim(),
    `media-src blob: ${resources.length ? resources.join(" ") : "'none'"}`,
    `connect-src ${connect.length ? connect.join(" ") : "'none'"}`,
    `frame-src ${frames.length ? frames.join(" ") : "'none'"}`,
    `base-uri ${bases.length ? bases.join(" ") : "'self'"}`,
    "object-src 'none'",
    "form-action 'none'",
  ].join("; ");
  const tag = `<meta http-equiv="Content-Security-Policy" content="${escAttr(policy)}">`;
  if (/<head(\s[^>]*)?>/i.test(html)) {
    return html.replace(/<head(\s[^>]*)?>/i, (head) => `${head}${tag}`);
  }
  return `<!doctype html><html><head>${tag}</head><body>${html}</body></html>`;
}

function injectMotifWispBridge(html) {
  const script = `<script>
(() => {
  const reply = (method, params) => parent.postMessage({ jsonrpc: "2.0", method, params }, "*");
  const activeRecord = () => {
    try { return typeof window.motifGetActiveRecord === "function" ? window.motifGetActiveRecord() : null; }
    catch { return null; }
  };
  const recordSequence = (record) => typeof record?.seq === "string"
    ? record.seq.toUpperCase()
    : typeof record?.sequence === "string"
      ? record.sequence.toUpperCase()
      : "";
  const coordinateNumber = (value) => Number(String(value || "").replace(/[^0-9]/g, ""));
  const sequenceRange = (record, start, end, wrap = false) => {
    const source = recordSequence(record);
    if (!start || !end || start > source.length || end > source.length) return "";
    return wrap || start > end
      ? source.slice(start - 1) + source.slice(0, end)
      : source.slice(start - 1, end);
  };
  const renderedSelection = (record) => {
    const label = document.querySelector(
      ".motif-cs-selection-bar:not([data-empty='true']) .motif-cs-selection-name",
    )?.textContent?.trim() || "";
    const match = label.match(/^([0-9][0-9,. ]*)-([0-9][0-9,. ]*)( wrap)? \\(([0-9][0-9,. ]*)\\)$/);
    if (!match) return null;
    const start = coordinateNumber(match[1]);
    const end = coordinateNumber(match[2]);
    const length = coordinateNumber(match[4]);
    const sequence = sequenceRange(record, start, end, Boolean(match[3]));
    return sequence.length === length ? { start, end, strand: "forward", sequence } : null;
  };
  const featureSelection = (record) => {
    const label = document.querySelector(
      ".motif-cs-selection-bar:not([data-empty='true']) .motif-cs-selection-name",
    )?.textContent?.trim() || "";
    const labelMatch = label.match(/^(.*?)\s+([0-9][0-9,. ]*)-([0-9][0-9,. ]*)(?:\s+wrap)?$/);
    const annotations = Array.isArray(record?.annotations)
      ? record.annotations
      : Array.isArray(record?.features) ? record.features : [];
    const selectedNode = document.querySelector(
      ".motif-pm-feature[aria-pressed='true'][data-feature-id], .motif-cs-feature-block[aria-pressed='true']",
    );
    const selectedId = selectedNode?.getAttribute("data-feature-id") || "";
    let feature = selectedId
      ? annotations.find((annotation) => String(annotation?.id || "") === selectedId)
      : null;
    if (!feature && labelMatch) {
      const start = coordinateNumber(labelMatch[2]);
      const end = coordinateNumber(labelMatch[3]);
      const name = labelMatch[1].trim();
      feature = annotations.find((annotation) => (
        Number(annotation?.start) + 1 === start
        && Number(annotation?.end) === end
        && String(annotation?.name || "").trim() === name
      ));
    }
    if (!feature) return null;
    const start = Number(feature.start) + 1;
    const end = Number(feature.end);
    const sequence = sequenceRange(record, start, end);
    if (!sequence) return null;
    return {
      start,
      end,
      strand: Number(feature.strand) === -1 ? "reverse" : "forward",
      sequence,
      featureName: String(feature.name || "").trim() || undefined,
    };
  };
  const nativeSelection = (record) => {
    const source = recordSequence(record);
    const raw = String(getSelection()?.toString() || "");
    const sequence = raw.replace(/[^A-Za-z*.-]/g, "").toUpperCase();
    if (!source || !sequence) return null;
    const offset = source.indexOf(sequence);
    return offset < 0 ? null : { start: offset + 1, end: offset + sequence.length, strand: "forward", sequence };
  };
  let lastNativeSelection = null;
  let selectionLengthFrame = 0;
  const updateSelectionLength = () => {
    selectionLengthFrame = 0;
    const bar = document.querySelector(".motif-cs-selection-bar");
    if (!bar) return;
    const record = activeRecord();
    const recordId = String(record?.id || "");
    const selection = renderedSelection(record)
      || featureSelection(record)
      || (bar.matches(":not([data-empty='true'])")
        ? null
        : nativeSelection(record)
          || (lastNativeSelection?.recordId === recordId ? lastNativeSelection : null));
    let badge = bar.querySelector("[data-wisp-motif-selection-length]");
    if (!selection?.sequence) {
      badge?.remove();
      return;
    }
    const length = Array.from(selection.sequence).length;
    const label = length.toLocaleString() + " bp";
    if (!badge) {
      badge = document.createElement("span");
      badge.setAttribute("data-wisp-motif-selection-length", "");
      badge.style.cssText = "margin-left:auto;padding-left:10px;white-space:nowrap;font-weight:700;font-variant-numeric:tabular-nums;color:currentColor;pointer-events:none";
      bar.appendChild(badge);
    }
    if (badge.textContent !== label) badge.textContent = label;
    badge.setAttribute("aria-label", "Selected sequence length: " + label);
  };
  const scheduleSelectionLengthUpdate = () => {
    if (selectionLengthFrame) return;
    selectionLengthFrame = requestAnimationFrame(updateSelectionLength);
  };
  const rememberNativeSelection = () => {
    const record = activeRecord();
    const selection = nativeSelection(record);
    if (selection) lastNativeSelection = { recordId: String(record?.id || ""), ...selection };
    scheduleSelectionLengthUpdate();
  };
  document.addEventListener("selectionchange", rememberNativeSelection);
  document.addEventListener("pointerup", rememberNativeSelection, true);
  document.addEventListener("keyup", rememberNativeSelection, true);
  const scrollSelectedFeatureIntoView = () => {
    requestAnimationFrame(() => requestAnimationFrame(() => {
      const block = document.querySelector(".motif-cs-feature-block[aria-pressed='true']");
      const pane = block?.closest(".motif-cs-sequence-column");
      if (!block || !pane) return;
      const blockRect = block.getBoundingClientRect();
      const paneRect = pane.getBoundingClientRect();
      pane.scrollTop = Math.max(0, pane.scrollTop + blockRect.top - paneRect.top
        - Math.max(0, (pane.clientHeight - blockRect.height) / 2));
    }));
  };
  const scheduleFeatureFocus = (target) => {
    if (!(target instanceof Element) || !target.closest(".motif-pm-feature[data-feature-id]")) return;
    lastNativeSelection = null;
    scrollSelectedFeatureIntoView();
  };
  document.addEventListener("click", (event) => scheduleFeatureFocus(event.target), true);
  document.addEventListener("keydown", (event) => {
    if (event.key === "Enter" || event.key === " ") scheduleFeatureFocus(event.target);
  }, true);
  const featureFocusObserver = new MutationObserver(() => {
    if (document.querySelector(".motif-cs-feature-block[aria-pressed='true']")) {
      scrollSelectedFeatureIntoView();
    }
    scheduleSelectionLengthUpdate();
  });
  featureFocusObserver.observe(document.body, {
    subtree: true,
    childList: true,
    attributes: true,
    attributeFilter: ["aria-pressed"],
  });
  scheduleSelectionLengthUpdate();
  addEventListener("message", (event) => {
    const message = event.data || {};
    if (message.jsonrpc !== "2.0") return;
    if (message.method === "wisp/motif-add-records") {
      try {
        if (typeof window.motifAddRecords !== "function") throw new Error("Motif record API is not ready.");
        const records = Array.isArray(message.params?.records) ? message.params.records : [];
        window.motifAddRecords(records);
        reply("wisp/notifications/motif-records-added", { requestId: message.params?.requestId, count: records.length });
      } catch (error) {
        reply("wisp/notifications/motif-bridge-error", { requestId: message.params?.requestId, message: error instanceof Error ? error.message : String(error) });
      }
    }
    if (message.method === "wisp/motif-get-selection") {
      try {
        const record = activeRecord();
        const recordId = String(record?.id || "");
        const selection = renderedSelection(record)
          || featureSelection(record)
          || (document.querySelector(".motif-cs-selection-bar:not([data-empty='true'])")
            ? null
            : nativeSelection(record)
              || (lastNativeSelection?.recordId === recordId ? lastNativeSelection : null));
        if (!record || !selection) throw new Error("Select a sequence range in Motif first.");
        reply("wisp/notifications/motif-selection", {
          requestId: message.params?.requestId,
          recordName: String(record.name || record.id || "Motif record"),
          recordId,
          molecule: String(record.type || record.molecule || "dna"),
          start: selection.start,
          end: selection.end,
          strand: selection.strand || "forward",
          sequence: selection.sequence,
          featureName: selection.featureName,
        });
      } catch (error) {
        reply("wisp/notifications/motif-bridge-error", { requestId: message.params?.requestId, message: error instanceof Error ? error.message : String(error) });
      }
    }
  });
  let readyAttempts = 0;
  const announceReady = () => {
    if (typeof window.motifGetActiveRecord === "function") {
      reply("wisp/notifications/motif-bridge-ready", {});
      return;
    }
    readyAttempts += 1;
    if (readyAttempts < 200) setTimeout(announceReady, 50);
  };
  announceReady();
})();
</script>`;
  // Motif bundles may contain literal `</body>` text inside minified scripts.
  // Never splice the document with a regex: HTML parsers accept a trailing
  // script after </html> and place it in the document body without corrupting
  // any of Motif's original script boundaries.
  return `${html}${script}`;
}

function ensureMcpAppParkingRoot() {
  if (mcpAppParkingRoot?.isConnected) return mcpAppParkingRoot;
  const root = document.createElement("div");
  root.id = "wisp-mcp-app-parking";
  root.setAttribute("aria-hidden", "true");
  Object.assign(root.style, {
    position: "fixed", left: "-10000px", top: "-10000px",
    width: "1px", height: "1px", overflow: "hidden", pointerEvents: "none",
  });
  document.body.appendChild(root);
  mcpAppParkingRoot = root;
  return root;
}

function mcpAppTitle(payload) {
  return payload?.tool?.title
    || payload?.tool?.annotations?.title
    || payload?.tool?.name
    || "MCP App";
}

function mcpAppDimensions(instance) {
  const rect = instance.target?.getBoundingClientRect();
  return {
    width: Math.max(Math.round(rect?.width || instance.frame.clientWidth || 0), 320),
    height: Math.max(Math.round(rect?.height || instance.frame.clientHeight || 0), 320),
  };
}

function createMcpAppInstance(instanceId, payloadJson) {
  const payload = typeof payloadJson === "string" ? JSON.parse(payloadJson) : payloadJson;
  const html = payload?.resource?.text;
  if (typeof html !== "string" || !html) return null;

  const frame = document.createElement("iframe");
  frame.title = mcpAppTitle(payload);
  frame.setAttribute("sandbox", "allow-scripts");
  frame.setAttribute("referrerpolicy", "no-referrer");
  Object.assign(frame.style, { width: "100%", height: "100%", border: "0", background: "#fff" });

  const instance = {
    id: instanceId,
    appName: mcpAppTitle(payload),
    payload,
    payloadJson: typeof payloadJson === "string" ? payloadJson : JSON.stringify(payloadJson),
    frame,
    target: null,
    initialized: false,
    teardownId: 1000000,
    teardownRequestId: null,
    teardownTimer: null,
    resizeObserver: null,
    onMessage: null,
    motifRequestId: 0,
    motifRequests: new Map(),
  };
  const post = (message) => frame.contentWindow?.postMessage(message, "*");
  const clearModelContext = () => invoke("update_mcp_app_context", {
    instanceId,
    appName: instance.appName,
    context: {},
  });
  const hostContext = () => ({
    theme: document.documentElement.dataset.theme === "dark" ? "dark" : "light",
    displayMode: "inline",
    availableDisplayModes: ["inline"],
    containerDimensions: mcpAppDimensions(instance),
    locale: document.documentElement.lang || navigator.language || "en",
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    platform: "desktop",
    userAgent: `wisp-science/${wispAppVersion}`,
    toolInfo: { tool: instance.payload.tool || {} },
  });
  const sendHostContext = () => {
    if (!instance.initialized || !instance.target) return;
    post({
      jsonrpc: "2.0",
      method: "ui/notifications/host-context-changed",
      params: hostContext(),
    });
  };
  const sendData = () => {
    if (!instance.initialized) return;
    post({ jsonrpc: "2.0", method: "ui/notifications/tool-input", params: { arguments: instance.payload.arguments || {} } });
    post({ jsonrpc: "2.0", method: "ui/notifications/tool-result", params: instance.payload.result || { content: [] } });
  };
  const cleanup = () => {
    if (instance.teardownTimer != null) window.clearTimeout(instance.teardownTimer);
    instance.resizeObserver?.disconnect();
    window.removeEventListener("message", instance.onMessage);
    frame.remove();
    if (mcpAppInstances.get(instanceId) === instance) {
      mcpAppInstances.delete(instanceId);
      void clearModelContext();
      // Revoke the host-side serverTools bridge so a later request from a
      // stale iframe fails with a stale-instance error (best-effort).
      void invoke("close_mcp_app", { instanceId });
    }
  };
  const requestTeardown = (reason) => {
    if (instance.initialized && instance.teardownRequestId == null) {
      instance.teardownRequestId = ++instance.teardownId;
      post({
        jsonrpc: "2.0",
        id: instance.teardownRequestId,
        method: "ui/resource-teardown",
        params: { reason },
      });
      instance.teardownTimer = window.setTimeout(cleanup, 500);
    } else {
      cleanup();
    }
  };
  instance.onMessage = (event) => {
    if (event.source !== frame.contentWindow || !event.data || event.data.jsonrpc !== "2.0") return;
    const message = event.data;
    if (message.method?.startsWith("wisp/notifications/motif-")) {
      if (message.method === "wisp/notifications/motif-bridge-ready") {
        if (!instance.initialized) {
          instance.initialized = true;
          sendData();
          sendHostContext();
        }
        return;
      }
      const requestId = message.params?.requestId;
      const pending = instance.motifRequests.get(requestId);
      if (pending) {
        instance.motifRequests.delete(requestId);
        if (message.method === "wisp/notifications/motif-bridge-error") {
          pending.reject(new Error(message.params?.message || "Motif bridge failed."));
        } else {
          pending.resolve(message.params || {});
        }
      }
      return;
    }
    if (message.method === "ui/initialize" && message.id != null) {
      const hostCapabilities = {
        sandbox: { csp: instance.payload?.resource?._meta?.ui?.csp || instance.payload?.resource?._meta?.csp || {} },
        updateModelContext: { text: {} },
      };
      // Only advertise serverTools when this instance still has a live host
      // bridge. Restored/parked Apps without the original MCP connection keep
      // using updateModelContext instead of a false capability.
      void invoke_strict("mcp_app_has_server_tools", { instanceId }).then(
        (available) => {
          if (available) hostCapabilities.serverTools = {};
          post({
            jsonrpc: "2.0",
            id: message.id,
            result: {
              protocolVersion: message.params?.protocolVersion || "2026-01-26",
              hostCapabilities,
              hostInfo: { name: "wisp-science", version: wispAppVersion },
              hostContext: hostContext(),
            },
          });
        },
        () => post({
          jsonrpc: "2.0",
          id: message.id,
          result: {
            protocolVersion: message.params?.protocolVersion || "2026-01-26",
            hostCapabilities,
            hostInfo: { name: "wisp-science", version: wispAppVersion },
            hostContext: hostContext(),
          },
        }),
      );
      return;
    }
    if (message.method === "ui/notifications/initialized") {
      instance.initialized = true;
      sendData();
      sendHostContext();
      return;
    }
    if (message.method === "ping" && message.id != null) {
      post({ jsonrpc: "2.0", id: message.id, result: {} });
      return;
    }
    if (message.method === "tools/list" && message.id != null) {
      void invoke_strict("list_mcp_app_tools", { instanceId }).then(
        (result) => post({ jsonrpc: "2.0", id: message.id, result }),
        (error) => post({
          jsonrpc: "2.0",
          id: message.id,
          error: {
            code: -32603,
            message: (error instanceof Error ? error.message : String(error)).slice(0, 512),
          },
        }),
      );
      return;
    }
    if (message.method === "tools/call" && message.id != null) {
      const params = message.params || {};
      const name = params.name;
      if (typeof name !== "string" || !name) {
        post({
          jsonrpc: "2.0",
          id: message.id,
          error: { code: -32602, message: "tools/call requires a valid 'name' string" },
        });
        return;
      }
      const args = params.arguments
        && typeof params.arguments === "object"
        && !Array.isArray(params.arguments)
        ? params.arguments
        : {};
      void invoke_strict("call_mcp_app_tool", { instanceId, name, arguments: args }).then(
        (result) => post({ jsonrpc: "2.0", id: message.id, result }),
        (error) => post({
          jsonrpc: "2.0",
          id: message.id,
          error: {
            code: -32603,
            message: (error instanceof Error ? error.message : String(error)).slice(0, 512),
          },
        }),
      );
      return;
    }
    if (message.method === "ui/update-model-context" && message.id != null) {
      void invoke_strict("update_mcp_app_context", {
        instanceId,
        appName: instance.appName,
        context: message.params || {},
      }).then(
        () => post({ jsonrpc: "2.0", id: message.id, result: {} }),
        (error) => post({
          jsonrpc: "2.0",
          id: message.id,
          error: {
            code: -32602,
            message: (error instanceof Error ? error.message : String(error)).slice(0, 512),
          },
        }),
      );
      return;
    }
    if (instance.teardownRequestId != null && message.id === instance.teardownRequestId) {
      cleanup();
      return;
    }
    if (message.id != null) {
      post({
        jsonrpc: "2.0",
        id: message.id,
        error: { code: -32601, message: "Capability is not granted by Wisp" },
      });
    }
  };
  instance.requestTeardown = requestTeardown;
  instance.sendHostContext = sendHostContext;
  instance.sendData = sendData;
  window.addEventListener("message", instance.onMessage);
  const bridgedHtml = payload?.tool?.name === "motif_open_workbench"
    ? injectMotifWispBridge(html)
    : html;
  frame.srcdoc = injectMcpAppCsp(bridgedHtml, payload?.resource?._meta);
  mcpAppInstances.set(instanceId, instance);
  return instance;
}

function mcpAppDocumentKey(payload) {
  const uri = String(payload?.resource?.uri || "").split(/[?#]/, 1)[0];
  return `${uri}\n${payload?.resource?.text || ""}`;
}

/** Mount one MCP App inside a host-owned center pane. The app keeps an opaque
 * origin and scripts only; filesystem, forms, popups, top navigation,
 * downloads, and same-origin access remain unavailable. */
export function mount_mcp_app(instanceId, elId, payloadJson) {
  const target = document.getElementById(elId);
  if (!target) return false;
  let instance = mcpAppInstances.get(instanceId);
  if (instance && instance.payloadJson !== payloadJson) {
    const nextPayload = typeof payloadJson === "string" ? JSON.parse(payloadJson) : payloadJson;
    if (mcpAppDocumentKey(instance.payload) === mcpAppDocumentKey(nextPayload)) {
      instance.payload = nextPayload;
      instance.payloadJson = typeof payloadJson === "string" ? payloadJson : JSON.stringify(payloadJson);
      instance.appName = mcpAppTitle(nextPayload);
      instance.frame.title = instance.appName;
      instance.sendData();
      instance.sendHostContext();
    } else {
      void invoke("update_mcp_app_context", {
        instanceId,
        appName: instance.appName,
        context: {},
      });
      // Detach first so the old iframe's cleanup does not revoke the host
      // bridge the newer presentation just registered under this same id.
      if (mcpAppInstances.get(instanceId) === instance) {
        mcpAppInstances.delete(instanceId);
      }
      instance.requestTeardown("replaced by a newer MCP App presentation");
      instance = null;
    }
  }
  instance ||= createMcpAppInstance(instanceId, payloadJson);
  if (!instance) return false;

  instance.resizeObserver?.disconnect();
  target.replaceChildren(instance.frame);
  instance.target = target;
  instance.resizeObserver = new ResizeObserver(() => instance.sendHostContext());
  instance.resizeObserver.observe(target);
  instance.sendHostContext();
  return true;
}

/** Keep a live iframe attached off-screen while another center tab is active. */
export function park_mcp_app(instanceId) {
  const instance = mcpAppInstances.get(instanceId);
  if (!instance) return;
  instance.resizeObserver?.disconnect();
  instance.target = null;
  ensureMcpAppParkingRoot().appendChild(instance.frame);
}

/** Read one explicitly selected local sequence file and load it into the live
 * Motif instance through the MCP connection that opened the workbench. */
const SNAPGENE_FEATURE_TYPES = Object.freeze({
  cds: "cds",
  gene: "gene",
  promoter: "promoter",
  terminator: "terminator",
  rbs: "rbs",
  rep_origin: "origin",
  origin: "origin",
  primer_bind: "primer_bind",
  protein_bind: "regulatory",
  regulatory: "regulatory",
  enhancer: "enhancer",
  mrna: "mRNA",
  rrna: "rRNA",
  trna: "tRNA",
  ncrna: "ncRNA",
  repeat_region: "repeat_region",
  sig_peptide: "sig_peptide",
  mat_peptide: "mat_peptide",
  transit_peptide: "transit_peptide",
  intron: "intron",
  exon: "exon",
  polya_signal: "polyA_signal",
  misc_feature: "misc_feature",
});

function snapGeneFeatureType(value) {
  const key = String(value || "misc_feature").trim().toLowerCase();
  return SNAPGENE_FEATURE_TYPES[key] || "custom";
}

function snapGenePlainText(value) {
  const text = String(value || "").trim();
  if (!text) return "";
  if (!/[<&]/.test(text)) return text.slice(0, 16_384);
  const document = new DOMParser().parseFromString(text, "text/html");
  return (document.body?.textContent || text.replace(/<[^>]*>/g, " "))
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 16_384);
}

function snapGeneFeatureMetadata(feature, originalType) {
  const metadata = Object.create(null);
  if (originalType) metadata.snapGeneType = originalType;
  for (const qualifier of feature.getElementsByTagName("Q")) {
    const name = qualifier.getAttribute("name")?.trim();
    if (!name || name === "__proto__" || name === "constructor" || name === "prototype") continue;
    const values = [];
    for (const value of qualifier.getElementsByTagName("V")) {
      const raw = value.getAttribute("text")
        ?? value.getAttribute("predef")
        ?? value.getAttribute("int")
        ?? value.getAttribute("real")
        ?? value.getAttribute("bool")
        ?? value.textContent;
      const normalized = snapGenePlainText(raw);
      if (normalized) values.push(normalized);
    }
    if (values.length > 0) metadata[name] = values.length === 1 ? values[0] : values;
  }
  return metadata;
}

function snapGeneSegment(range, sequenceLength, strand) {
  const match = String(range || "").match(/^\s*(\d+)\s*-\s*(\d+)\s*$/);
  if (!match) return [];
  const oneBasedStart = Number(match[1]);
  const oneBasedEnd = Number(match[2]);
  if (!Number.isSafeInteger(oneBasedStart) || !Number.isSafeInteger(oneBasedEnd)
      || oneBasedStart < 1 || oneBasedEnd < 1
      || oneBasedStart > sequenceLength || oneBasedEnd > sequenceLength) return [];
  if (oneBasedStart <= oneBasedEnd) {
    return [{ start: oneBasedStart - 1, end: oneBasedEnd, strand }];
  }
  // Circular features may cross coordinate 1. Motif accepts split subranges.
  return [
    { start: oneBasedStart - 1, end: sequenceLength, strand },
    { start: 0, end: oneBasedEnd, strand },
  ];
}

function parseSnapGeneFeatureXml(packet, sequenceLength) {
  let xml;
  try { xml = new TextDecoder("utf-8", { fatal: true }).decode(packet).replace(/^\uFEFF/, "").trim(); }
  catch { return null; }
  // Compressed legacy packets are skipped when the file also carries the
  // modern XML packet. Treating compressed bytes as XML would invent data.
  if (!xml.startsWith("<")) return null;
  const document = new DOMParser().parseFromString(xml, "application/xml");
  if (document.querySelector("parsererror")) return null;
  const annotations = [];
  const features = document.getElementsByTagName("Feature");
  for (let index = 0; index < features.length && index < 10_000; index += 1) {
    const feature = features[index];
    const directionality = feature.getAttribute("directionality");
    const strand = directionality === "2" ? -1 : directionality === "1" ? 1 : 0;
    const subRanges = [];
    let color = null;
    for (const segment of feature.getElementsByTagName("Segment")) {
      subRanges.push(...snapGeneSegment(segment.getAttribute("range"), sequenceLength, strand));
      color ||= segment.getAttribute("color")?.trim() || null;
    }
    if (subRanges.length === 0) continue;
    const originalType = feature.getAttribute("type")?.trim() || "misc_feature";
    annotations.push({
      id: `snapgene-feature-${index + 1}`,
      name: (feature.getAttribute("name")?.trim() || `Feature ${index + 1}`).slice(0, 512),
      type: snapGeneFeatureType(originalType),
      start: Math.min(...subRanges.map((segment) => segment.start)),
      end: Math.max(...subRanges.map((segment) => segment.end)),
      strand,
      ...(color ? { color } : {}),
      metadata: snapGeneFeatureMetadata(feature, originalType),
      ...(subRanges.length > 1 ? { subRanges } : {}),
    });
  }
  return annotations;
}

function parseSnapGeneDna(input, filename) {
  const bytes = input instanceof Uint8Array ? input : new Uint8Array(input);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const decoder = new TextDecoder("ascii", { fatal: true });
  let offset = 0;
  let sawCookie = false;
  let sequence = null;
  let circular = false;
  const modernFeaturePackets = [];
  const legacyFeaturePackets = [];
  while (offset < bytes.length) {
    if (bytes.length - offset < 5) throw new Error("The SnapGene .dna file has a truncated packet header.");
    const type = bytes[offset];
    const length = view.getUint32(offset + 1, false);
    offset += 5;
    if (length > bytes.length - offset) throw new Error("The SnapGene .dna file has a truncated packet.");
    const packet = bytes.subarray(offset, offset + length);
    offset += length;
    if (type === 0x09) {
      if (length < 14 || decoder.decode(packet.subarray(0, 8)) !== "SnapGene") {
        throw new Error("The selected .dna file is not a valid SnapGene file.");
      }
      sawCookie = true;
    } else if (type === 0x00) {
      if (length < 2) throw new Error("The SnapGene .dna file has no sequence data.");
      circular = (packet[0] & 0x01) !== 0;
      const sequenceOffset = packet.length > 1 && packet[1] === 0x6e ? 2 : 1;
      sequence = decoder.decode(packet.subarray(sequenceOffset)).toUpperCase();
    } else if (type === 0x0a) {
      modernFeaturePackets.push(packet);
    } else if (type === 0x07) {
      legacyFeaturePackets.push(packet);
    }
  }
  if (!sawCookie || sequence == null) throw new Error("The selected .dna file has no SnapGene DNA sequence packet.");
  if (!/^[ACGTRYSWKMBDHVNU]+$/.test(sequence)) {
    throw new Error("The SnapGene sequence contains unsupported DNA symbols.");
  }
  if (sequence.length > 2_000_000) throw new Error("Motif DNA imports are limited to 2,000,000 bases.");
  let annotations = null;
  for (const packet of [...modernFeaturePackets, ...legacyFeaturePackets]) {
    const parsed = parseSnapGeneFeatureXml(packet, sequence.length);
    if (parsed !== null) {
      annotations = parsed;
      if (parsed.length > 0) break;
    }
  }
  const name = filename.replace(/\.dna$/i, "").trim() || "SnapGene sequence";
  return {
    records: [{
      name,
      type: "dna",
      topology: circular ? "circular" : "linear",
      sequence,
      annotations: annotations || [],
    }],
  };
}

function motifArgumentsForBytes(bytes, filename) {
  if (/\.dna$/i.test(filename)) return { payload: parseSnapGeneDna(bytes, filename) };
  if (bytes.byteLength > 2_000_000) throw new Error("Motif text imports are limited to 2,000,000 bytes.");
  let content;
  try { content = new TextDecoder("utf-8", { fatal: true }).decode(bytes); }
  catch { throw new Error("This is a binary file. Use SnapGene .dna or a supported text sequence format."); }
  if (!content.trim()) throw new Error("The selected DNA file is empty.");
  if (content.includes("\0")) throw new Error("This is a binary file. Use SnapGene .dna or a supported text sequence format.");
  return { content, filename };
}

async function callMotifOpen(instance, motifArgs) {
  const result = await invoke_strict("call_mcp_app_tool", {
    instanceId: instance.id,
    name: "motif_open_workbench",
    arguments: motifArgs,
  });
  if (result?.isError) {
    const message = result.content?.find((item) => item?.type === "text")?.text;
    throw new Error(message || "Motif rejected the selected DNA file.");
  }
  return result;
}

function motifBridgeRequest(instance, method, params = {}) {
  return new Promise((resolve, reject) => {
    const requestId = ++instance.motifRequestId;
    instance.motifRequests.set(requestId, { resolve, reject });
    instance.frame.contentWindow?.postMessage({
      jsonrpc: "2.0",
      method,
      params: { ...params, requestId },
    }, "*");
    window.setTimeout(() => {
      if (instance.motifRequests.delete(requestId)) reject(new Error("Motif did not respond in time."));
    }, 5000);
  });
}

export async function import_motif_dna_file(instanceId) {
  const instance = mcpAppInstances.get(instanceId);
  if (!instance?.initialized || !instance.frame.contentWindow) {
    throw new Error("The Motif workbench is not ready.");
  }
  if (instance.payload?.tool?.name !== "motif_open_workbench") {
    throw new Error("Local DNA import is only available in the Motif workbench.");
  }

  const file = await new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".dna,.fa,.fasta,.fna,.ffn,.faa,.frn,.gb,.gbk,.genbank,.txt,.seq,.json,text/plain,application/json";
    input.style.display = "none";
    const finish = (selected) => {
      input.remove();
      resolve(selected || null);
    };
    input.addEventListener("change", () => finish(input.files?.[0]), { once: true });
    input.addEventListener("cancel", () => finish(null), { once: true });
    document.body.appendChild(input);
    input.click();
  });
  if (!file) return { imported: false, cancelled: true };
  if (file.size > 32 * 1024 * 1024) throw new Error("The selected DNA file exceeds the 32 MiB import limit.");

  const motifArgs = motifArgumentsForBytes(
    new Uint8Array(await file.arrayBuffer()),
    file.name,
  );

  const result = await callMotifOpen(instance, motifArgs);
  instance.frame.contentWindow.postMessage({
    jsonrpc: "2.0",
    method: "ui/notifications/tool-result",
    params: result || { content: [] },
  }, "*");
  return {
    imported: true,
    filename: file.name,
    recordCount: result?.structuredContent?.recordCount ?? 0,
  };
}

/** Parse a project file with Motif's MCP tool, then append its records to the
 * already-open workbench without replacing the current inventory. */
export async function add_workspace_file_to_motif(instanceId, path) {
  const instance = mcpAppInstances.get(instanceId);
  if (!instance?.initialized || instance.payload?.tool?.name !== "motif_open_workbench") {
    throw new Error("Open Motif in the current conversation before adding a project file.");
  }
  const filename = String(path).split(/[\\/]/).pop() || "sequence";
  const bytes = await previewBytes({ path, maxBytes: 32 * 1024 * 1024 });
  const result = await callMotifOpen(instance, motifArgumentsForBytes(bytes, filename));
  const records = result?.structuredContent?.payload?.records;
  if (!Array.isArray(records) || records.length === 0) throw new Error("Motif did not return any DNA records.");
  await motifBridgeRequest(instance, "wisp/motif-add-records", { records });
  return { imported: true, filename, recordCount: records.length };
}

export async function request_motif_selection(instanceId) {
  const instance = mcpAppInstances.get(instanceId);
  if (!instance?.initialized || instance.payload?.tool?.name !== "motif_open_workbench") {
    throw new Error("The Motif workbench is not ready.");
  }
  return await motifBridgeRequest(instance, "wisp/motif-get-selection");
}

/** Close a center-tab MCP App and give it a bounded graceful teardown window. */
export function close_mcp_app(instanceId) {
  mcpAppInstances.get(instanceId)?.requestTeardown("user closed the app");
}

const NOTEBOOK_BLOCKED_ELEMENTS = [
  "script", "iframe", "frame", "object", "embed", "foreignObject",
  "animate", "animateMotion", "animateTransform", "set", "mpath",
  "form", "input", "button", "textarea", "select", "option",
  "link", "meta", "base", "audio", "video", "source", "track",
].join(",");

const NOTEBOOK_URL_ATTRIBUTES = new Set([
  "href", "xlink:href", "src", "srcset", "action", "formaction",
  "poster", "ping", "target", "download", "srcdoc",
]);

function notebookSafeResource(value) {
  const normalized = String(value || "").trim();
  return normalized.startsWith("#") ||
    /^data:image\/(?:png|jpeg|gif|webp);base64,/i.test(normalized);
}

function notebookUnsafeCss(value) {
  const withoutLocalFragments = String(value || "")
    .replace(/url\(\s*(['"]?)#[^)]+\)/gi, "");
  return /@import|url\s*\(/i.test(withoutLocalFragments);
}

/**
 * Defense in depth for saved notebook output. The iframe sandbox below is the
 * security boundary; this scrub also removes active elements and references so
 * opening a notebook cannot quietly make network requests.
 */
function scrubNotebookMarkup(doc) {
  doc.querySelectorAll(NOTEBOOK_BLOCKED_ELEMENTS).forEach((node) => node.remove());
  doc.querySelectorAll("*").forEach((node) => {
    for (const attr of [...node.attributes]) {
      const name = attr.name.toLowerCase();
      if (name.startsWith("on") ||
          (NOTEBOOK_URL_ATTRIBUTES.has(name) && !notebookSafeResource(attr.value)) ||
          (name === "style" && notebookUnsafeCss(attr.value))) {
        node.removeAttribute(attr.name);
      }
    }
    if (doc.contentType === "text/html" && node.localName?.toLowerCase() === "img") {
      node.setAttribute("loading", "lazy");
      node.setAttribute("decoding", "async");
      node.setAttribute("referrerpolicy", "no-referrer");
    }
  });
  doc.querySelectorAll("style").forEach((style) => {
    if (notebookUnsafeCss(style.textContent)) style.remove();
  });
  return doc;
}

function staticNotebookHtml(html) {
  const parsed = scrubNotebookMarkup(
    new DOMParser().parseFromString(String(html || ""), "text/html"),
  );
  const styles = [...parsed.head.querySelectorAll("style")]
    .map((style) => style.outerHTML)
    .join("");
  const body = parsed.body?.innerHTML || "";
  const csp = [
    "default-src 'none'", "script-src 'none'", "connect-src 'none'",
    "frame-src 'none'", "object-src 'none'", "base-uri 'none'",
    "form-action 'none'", "img-src data: blob:", "font-src data:",
    "style-src 'unsafe-inline'",
  ].join("; ");
  return `<!doctype html><html><head>` +
    `<meta http-equiv="Content-Security-Policy" content="${csp}">` +
    `<meta name="referrer" content="no-referrer">` +
    `<meta name="viewport" content="width=device-width, initial-scale=1">` +
    `<style>html{color-scheme:light dark}body{margin:12px;overflow-wrap:anywhere}` +
    `img,svg,table{max-width:100%}table{border-collapse:collapse}` +
    `th,td{padding:4px 7px;border:1px solid #8886}</style>${styles}` +
    `</head><body>${body}</body></html>`;
}

function staticNotebookSvg(svg) {
  const parsed = new DOMParser().parseFromString(String(svg || ""), "image/svg+xml");
  if (parsed.querySelector("parsererror") || parsed.documentElement?.localName !== "svg") {
    throw new Error("Invalid SVG notebook output");
  }
  scrubNotebookMarkup(parsed);
  return new XMLSerializer().serializeToString(parsed.documentElement);
}

function fastaStats(text) {
  const lines = (text || "").split("\n");
  let seqs = 0;
  let maxLen = 0;
  let cur = 0;
  for (const raw of lines) {
    const line = raw.trim();
    if (!line || line.startsWith(";")) continue;
    if (line.startsWith(">")) {
      seqs += 1;
      cur = 0;
      continue;
    }
    cur += line.length;
    if (cur > maxLen) maxLen = cur;
  }
  return { seqs, maxLen };
}

function renderFasta(el, text) {
  const lines = (text || "").split("\n");
  let rows = "";
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const cls = line.startsWith(">") ? "rp-fasta-hdr" : "rp-fasta-seq";
    rows += `<tr><td class="rp-fasta-ln">${i + 1}</td><td class="${cls}">${escHtml(line) || "&nbsp;"}</td></tr>`;
  }
  const stats = fastaStats(text);
  const note = stats.seqs
    ? `<div class="rp-fasta-bar">${stats.seqs} sequences · ${stats.maxLen.toLocaleString()} positions</div>`
    : "";
  el.innerHTML = `${note}<div class="rp-fasta-wrap"><table class="rp-fasta-table"><tbody>${rows}</tbody></table></div>`;
}

/** @param {string} kind @param {string} elId @param {string} payloadJson */
export async function mount_preview(kind, elId, payloadJson) {
  const el = document.getElementById(elId);
  if (!el) return;
  cleanupPreview(el);
  const p = JSON.parse(payloadJson);
  el.innerHTML = "";
  el.classList.add("rp-heavy");

  switch (kind) {
    case "latex": {
      const k = await katex();
      el.innerHTML = k.renderToString(p.tex, { displayMode: !!p.display, throwOnError: false });
      break;
    }
    case "pdf": {
      await renderPdf(el, p);
      break;
    }
    case "docx": {
      await renderDocx(el, p);
      break;
    }
    case "xlsx": {
      await renderXlsx(el, p);
      break;
    }
    case "pptx": {
      await renderPptx(el, p);
      break;
    }
    case "image": {
      const src = p.b64 ? `data:${p.mime || "image/png"};base64,${p.b64}` : p.url;
      el.innerHTML = `<img class="rp-img" src="${src}" alt="${p.alt || ""}"/>`;
      break;
    }
    case "html": {
      const frame = document.createElement("iframe");
      frame.className = "rp-html";
      const pluginArtifact = /(^|[\\/])\.wisp[\\/]plugin-artifacts[\\/]/.test(p.path || "");
      frame.setAttribute("sandbox", pluginArtifact ? "allow-scripts" : "allow-same-origin allow-scripts");
      if (pluginArtifact) frame.setAttribute("referrerpolicy", "no-referrer");
      frame.setAttribute("scrolling", "no");
      frame.srcdoc = injectResponsiveHtmlPreview(p.text || "", htmlBaseHref(p.path || ""));
      el.appendChild(frame);
      break;
    }
    case "notebook-html": {
      const frame = document.createElement("iframe");
      frame.className = "rp-notebook-html";
      // No sandbox tokens: scripts, same-origin access, forms, popups, downloads,
      // and navigation out of the frame all stay disabled.
      frame.setAttribute("sandbox", "");
      frame.setAttribute("referrerpolicy", "no-referrer");
      frame.setAttribute("title", p.title || "Notebook HTML output");
      frame.srcdoc = staticNotebookHtml(p.text || "");
      el.appendChild(frame);
      el.__wispPreviewCleanup = () => frame.remove();
      break;
    }
    case "notebook-svg": {
      try {
        const safeSvg = staticNotebookSvg(p.text || "");
        const url = URL.createObjectURL(new Blob([safeSvg], { type: "image/svg+xml" }));
        const img = document.createElement("img");
        img.className = "rp-img rp-notebook-svg";
        img.alt = p.alt || "";
        img.loading = "lazy";
        img.decoding = "async";
        img.referrerPolicy = "no-referrer";
        img.src = url;
        el.appendChild(img);
        el.__wispPreviewCleanup = () => {
          img.remove();
          URL.revokeObjectURL(url);
        };
      } catch (error) {
        console.warn("Failed to render notebook SVG", error);
        el.textContent = p.error || "Unable to preview this SVG output.";
        el.classList.add("rp-error");
      }
      break;
    }
    case "structure": {
      const box = document.createElement("div");
      box.className = "rp-3dmol";
      el.appendChild(box);
      const $3Dmol = await loadMol3d();
      const v = $3Dmol.createViewer(box, { backgroundColor: "0x1e2024" });
      v.addModel(p.text, p.format || "pdb");
      v.setStyle({}, { cartoon: { color: "spectrum" } });
      v.zoomTo();
      v.render();
      break;
    }
    case "molecule": {
      const RDKit = await rdkit();
      const mol = RDKit.get_mol(p.smiles || p.text);
      if (!mol) {
        el.textContent = "Invalid molecule";
        break;
      }
      el.innerHTML = mol.get_svg(400, 300);
      mol.delete();
      break;
    }
    case "fasta": {
      renderFasta(el, p.text || "");
      break;
    }
    case "msa": {
      await ensureMsa();
      const text = p.text || p.fasta || "";
      const stats = fastaStats(text);
      const wrap = document.createElement("div");
      wrap.className = "rp-msa-wrap";
      const bar = document.createElement("div");
      bar.className = "rp-msa-bar";
      bar.textContent = `${stats.seqs} sequences · ${stats.maxLen.toLocaleString()} positions`;
      wrap.appendChild(bar);
      const tag = document.createElement("nightingale-msa");
      tag.setAttribute("width", "100%");
      tag.setAttribute("height", "420");
      tag.setAttribute("color-scheme", "clustal2");
      tag.setAttribute("label-width", "150");
      tag.setAttribute("tile-height", "20");
      tag.setAttribute("display-start", "1");
      tag.setAttribute("display-end", String(Math.max(stats.maxLen, 50)));
      wrap.appendChild(tag);
      el.appendChild(wrap);
      await customElements.whenDefined("nightingale-msa");
      tag.data = text;
      break;
    }
    default: {
      // textContent on a plain div collapses the file's newlines — a <pre> is
      // what keeps an unrecognised kind readable instead of one long paragraph.
      const pre = document.createElement("pre");
      pre.className = "rp-pre";
      pre.textContent = p.text || "";
      el.appendChild(pre);
    }
  }
}

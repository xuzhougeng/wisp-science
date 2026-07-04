// Tauri v2 shim + scientific preview mounts (single file so Trunk ships one snippet).

function tauriCore() {
  return window.__TAURI__?.core;
}

function tauriEvent() {
  return window.__TAURI__?.event;
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

function fileToBase64(file) {
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
    katexMod = (await import("/vendor/katex-Dn761jRB.js")).k;
    linkCss("/vendor/katex-DwwF5kvc.css");
  }
  return katexMod;
}

let rdkitInit;
async function rdkit() {
  if (!rdkitInit) {
    const mod = await import("/vendor/RDKit_minimal-B7RkdM0_.js");
    rdkitInit = mod.R.default();
  }
  return rdkitInit;
}

let mol3dLib;
async function loadMol3d() {
  if (!mol3dLib) {
    const mod = await import("/vendor/3Dmol-DfD4xImO.js");
    mol3dLib = mod._.default;
  }
  return mol3dLib;
}

let msaLoaded;
async function ensureMsa() {
  if (!msaLoaded) {
    await import("/vendor/nightingale-msa-5.6.0.js");
    msaLoaded = true;
  }
}

function escHtml(s) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
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
      const src = p.b64 ? `data:application/pdf;base64,${p.b64}` : p.url;
      el.innerHTML = `<embed class="rp-pdf" src="${src}" type="application/pdf"/>`;
      break;
    }
    case "image": {
      const src = p.b64 ? `data:${p.mime || "image/png"};base64,${p.b64}` : p.url;
      el.innerHTML = `<img class="rp-img" src="${src}" alt="${p.alt || ""}"/>`;
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
    default:
      el.textContent = p.text || "";
  }
}

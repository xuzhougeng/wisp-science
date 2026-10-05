// Authored shared renderers. Inputs are bounded local text, never URLs or HTML.
const MAX_TEXT = 1024 * 1024;
export function scientificText(value) {
  if (typeof value !== 'string' || !value.trim()) throw new Error('Empty scientific file');
  if (value.length > MAX_TEXT) throw new Error('Scientific preview exceeds 1 MiB of text');
  return value.replace(/\r\n?/g, '\n');
}

export function normalizeAlignment(value, format = '') {
  const text = scientificText(value), rows = new Map();
  const detected = text.trimStart().startsWith('>') ? 'fasta'
    : /^\s*(CLUSTAL|MUSCLE)/i.test(text) ? 'clustal'
    : /^\s*#\s*STOCKHOLM/i.test(text) ? 'stockholm' : '';
  format = format || detected;
  if (!detected || format !== detected) throw new Error('Unrecognized alignment format');
  let name = null, finished = false;
  const append = (id, residues) => {
    if (!id || id.length > 256 || !/^[A-Za-z.*?\-]+$/.test(residues)) throw new Error('Invalid alignment row');
    rows.set(id, (rows.get(id) || '') + residues.toUpperCase().replaceAll('.', '-'));
    if (rows.size > 512 || rows.get(id).length > 20000) throw new Error('Alignment exceeds 512 sequences or 20,000 positions');
  };
  for (const raw of text.split('\n')) {
    const line = raw.trim();
    if (!line) continue;
    if (finished) throw new Error('Multiple Stockholm alignments are not supported in one preview');
    if (format === 'fasta') {
      if (line.startsWith(';')) continue;
      if (line.startsWith('>')) {
        name = line.slice(1).trim();
        if (!name || rows.has(name)) throw new Error('Missing or duplicate sequence name');
        rows.set(name, '');
      } else {
        if (name === null) throw new Error('Sequence without a FASTA header');
        append(name, line.replace(/\s+/g, ''));
      }
    } else {
      if (format === 'stockholm' && line === '//') { finished = true; continue; }
      if (line.startsWith('#') || /^(CLUSTAL|MUSCLE)\b/i.test(line) || /^[\s*:.]+$/.test(raw)) continue;
      const match = line.match(/^(\S+)\s+([A-Za-z.*?\-]+)(?:\s+\d+)?$/);
      if (!match) throw new Error('Invalid alignment row');
      append(match[1], match[2]);
    }
  }
  const lengths = [...rows.values()].map(row => row.length);
  if (!lengths.length || lengths.some(length => length === 0 || length !== lengths[0]))
    throw new Error('Aligned sequences must have equal nonzero lengths');
  if (rows.size > 512 || lengths[0] > 20000 || rows.size * lengths[0] > MAX_TEXT)
    throw new Error('Alignment is too large for the preview');
  return { sequences: rows.size, positions: lengths[0], rows: [...rows].map(([name, sequence]) => ({ name, sequence })) };
}

function dispose(el) {
  el.__wispPreviewCleanup?.();
  delete el.__wispPreviewCleanup; delete el.__wispPreviewToken;
}
export function cleanupScientificPreview(el) { dispose(el); }

export async function renderScientific(el, kind, payload) {
  dispose(el);
  const token = Symbol('scientific-preview'), controller = new AbortController();
  el.__wispPreviewToken = token;
  let release = () => {};
  const observer = new MutationObserver(() => {
    if (!el.isConnected) dispose(el);
  });
  el.__wispPreviewCleanup = () => { observer.disconnect(); controller.abort(); release(); };
  observer.observe(document.documentElement, { childList: true, subtree: true });
  const current = () => el.isConnected && el.__wispPreviewToken === token && !controller.signal.aborted;
  el.replaceChildren();
  try {
    const text = scientificText(payload.text || payload.smiles || payload.fasta);
    if (payload.truncated) throw new Error('Incomplete file: open the full source before rendering');
    if (kind === 'structure') {
      const frame = document.createElement('iframe');
      frame.className = 'rp-3dmol'; frame.title = payload.structureLabel || '3D structure';
      frame.setAttribute('sandbox', 'allow-scripts allow-same-origin');
      frame.referrerPolicy = 'no-referrer';
      frame.src = new URL('./structure-preview.html', import.meta.url).href;
      const origin = new URL(frame.src).origin;
      const receive = event => {
        if (!current() || event.source !== frame.contentWindow || event.origin !== origin) return;
        if (event.data?.type === 'structure-ready') frame.contentWindow.postMessage({ type: 'structure-input', text, format: payload.format || 'pdb' }, origin);
        if (event.data?.type === 'structure-escape') el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
      };
      window.addEventListener('message', receive);
      release = () => { window.removeEventListener('message', receive); frame.remove(); };
      el.append(frame);
    } else if (kind === 'molecule') {
      const svg = await drawMolecule(text, controller.signal);
      if (!current()) return;
      const parsed = new DOMParser().parseFromString(svg, 'image/svg+xml');
      if (parsed.querySelector('parsererror') || parsed.documentElement.localName !== 'svg') throw new Error('Invalid molecule output');
      parsed.querySelectorAll('script,foreignObject,iframe,object,embed,style').forEach(node => node.remove());
      for (const node of parsed.querySelectorAll('*')) for (const attr of [...node.attributes]) {
        if (/^on/i.test(attr.name) || /(?:href|src)$/i.test(attr.name) && !attr.value.startsWith('#')) node.removeAttribute(attr.name);
      }
      const drawing = document.importNode(parsed.documentElement, true);
      drawing.classList.add('rp-molecule'); drawing.setAttribute('aria-label', payload.moleculeLabel || 'Molecule');
      el.append(drawing);
    } else if (kind === 'msa') {
      const alignment = normalizeAlignment(text, payload.format);
      const wrap = document.createElement('div'); wrap.className = 'rp-msa-wrap';
      const bar = document.createElement('div'); bar.className = 'rp-msa-bar';
      bar.textContent = `${alignment.sequences} sequences · ${alignment.positions.toLocaleString()} positions`;
      const frame = document.createElement('iframe'); frame.className = 'rp-alignment'; frame.title = 'Sequence alignment';
      frame.setAttribute('sandbox', 'allow-scripts allow-same-origin');
      frame.src = new URL('./alignment-preview.html', import.meta.url).href;
      const origin = new URL(frame.src).origin;
      const receive = event => {
        if (!current() || event.source !== frame.contentWindow || event.origin !== origin) return;
        if (event.data?.type === 'alignment-ready') frame.contentWindow.postMessage({ type: 'alignment-input', ...alignment }, origin);
        if (event.data?.type === 'alignment-escape') el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
      };
      window.addEventListener('message', receive);
      release = () => { window.removeEventListener('message', receive); wrap.remove(); };
      wrap.append(bar, frame); el.append(wrap);
    } else if (kind === 'fasta') {
      const table = document.createElement('table'); table.className = 'rp-fasta-table';
      const body = document.createElement('tbody'); table.append(body);
      const lines = text.split('\n');
      if (lines.length > 10000) throw new Error('FASTA preview exceeds 10,000 lines; use the source view');
      let sequences = 0, positions = 0, length = 0;
      for (const line of lines) {
        if (line.startsWith('>')) { sequences++; length = 0; }
        else if (!line.startsWith(';')) { length += line.trim().length; positions = Math.max(positions, length); }
      }
      if (sequences) {
        const bar = document.createElement('div'); bar.className = 'rp-fasta-bar';
        bar.textContent = `${sequences} sequences · ${positions.toLocaleString()} positions`; el.append(bar);
      }
      for (let index = 0; index < lines.length; index++) {
        const row = document.createElement('tr'), number = document.createElement('td'), cell = document.createElement('td');
        number.className = 'rp-fasta-ln'; number.textContent = String(index + 1);
        cell.className = lines[index].startsWith('>') ? 'rp-fasta-hdr' : 'rp-fasta-seq'; cell.textContent = lines[index] || '\u00a0';
        row.append(number, cell); body.append(row);
      }
      const wrap = document.createElement('div'); wrap.className = 'rp-fasta-wrap'; wrap.append(table); el.append(wrap);
    } else throw new Error('Unsupported scientific preview');
  } catch (error) {
    if (!current()) return;
    release();
    const failure = document.createElement('div'); failure.className = 'rp-error';
    failure.textContent = `${payload.error || 'Unable to preview scientific data'}: ${error.message}`;
    el.replaceChildren(failure);
  }
}

function drawMolecule(text, signal) {
  return new Promise((resolve, reject) => {
    const worker = new Worker(new URL('./rdkit-worker.mjs', import.meta.url), { type: 'module' });
    let settled = false;
    const finish = (callback, value) => {
      if (settled) return; settled = true;
      clearTimeout(timer); signal.removeEventListener('abort', cancel); worker.terminate(); callback(value);
    };
    const cancel = () => finish(reject, new DOMException('Aborted', 'AbortError'));
    const timer = setTimeout(() => finish(reject, new Error('Molecule parsing timed out')), 15000);
    worker.onmessage = ({ data }) => data?.svg ? finish(resolve, data.svg) : finish(reject, new Error(data?.error || 'Invalid molecule'));
    worker.onerror = () => finish(reject, new Error('Molecule renderer could not load'));
    signal.addEventListener('abort', cancel, { once: true });
    if (signal.aborted) cancel(); else worker.postMessage(text);
  });
}

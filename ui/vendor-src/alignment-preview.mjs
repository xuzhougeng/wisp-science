// A dedicated document owns the component's conservation worker and canvases.
const origin = location.origin;
let accepted = false, tag, observer, start = 1, total = 0;
function range() {
  const end = Math.min(total, start + 49);
  tag.setAttribute('display-start', String(start)); tag.setAttribute('display-end', String(end));
  document.getElementById('range').textContent = `${start}–${end} / ${total}`;
  document.getElementById('previous').disabled = start === 1;
  document.getElementById('next').disabled = end >= total;
}
window.addEventListener('message', async event => {
  if (accepted || event.source !== parent || event.origin !== origin || event.data?.type !== 'alignment-input') return;
  accepted = true;
  try {
    const { rows, positions } = event.data;
    if (!Array.isArray(rows) || !rows.length || rows.length > 512 || !Number.isInteger(positions) || positions < 1 || positions > 20000
        || rows.length * positions > 1024 * 1024 || rows.some(row => typeof row.name !== 'string' || row.name.length > 256 || typeof row.sequence !== 'string' || row.sequence.length !== positions))
      throw new Error('Invalid alignment');
    total = positions;
    await import('./nightingale-msa-5.6.0.js');
    await customElements.whenDefined('nightingale-msa');
    tag = document.createElement('nightingale-msa');
    tag.setAttribute('height', '360'); tag.setAttribute('tile-height', '20'); tag.setAttribute('color-scheme', 'clustal2');
    tag.setAttribute('length', String(total));
    const host = document.getElementById('alignment');
    const resize = () => {
      tag.setAttribute('width', String(Math.max(160, host.clientWidth)));
      tag.setAttribute('label-width', String(Math.min(150, Math.floor(host.clientWidth * .35))));
    };
    resize(); range(); host.append(tag);
    // This component's data setter expects records and already-mounted child views.
    await tag.updateComplete;
    tag.data = rows;
    await tag.updateComplete;
    range();
    observer = new ResizeObserver(resize); observer.observe(host);
    document.body.dataset.ready = 'true';
  } catch (error) { document.getElementById('status').textContent = error.message; document.body.dataset.error = 'true'; }
});
document.getElementById('previous').onclick = () => { start = Math.max(1, start - 50); range(); };
document.getElementById('next').onclick = () => { start = Math.min(total, start + 50); range(); };
window.addEventListener('keydown', event => {
  if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); parent.postMessage({ type: 'alignment-escape' }, origin); }
}, true);
window.addEventListener('pagehide', () => { observer?.disconnect(); tag?.worker?.terminate(); });
parent.postMessage({ type: 'alignment-ready' }, origin);

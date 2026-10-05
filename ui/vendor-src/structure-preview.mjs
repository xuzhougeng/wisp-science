// A dedicated document owns 3Dmol's WebGL context, listeners and observers.
// Removing the parent iframe releases the entire realm on replacement/close.
const parentOrigin = location.origin;
let accepted = false;
window.addEventListener('message', async event => {
  if (accepted || event.source !== parent || event.origin !== parentOrigin || event.data?.type !== 'structure-input') return;
  accepted = true;
  const status = document.getElementById('status');
  try {
    const { text, format } = event.data;
    if (typeof text !== 'string' || text.length > 1024 * 1024 || !['pdb', 'cif', 'mol2'].includes(format)) throw new Error('Invalid or oversized structure');
    const { _ } = await import('./3Dmol-DfD4xImO.js');
    const viewer = _.default.createViewer(document.getElementById('viewer'), { backgroundColor: '0x1e2024' });
    viewer.addModel(text, format);
    const atoms = viewer.selectedAtoms({});
    if (!atoms.length || atoms.length > 50000) throw new Error('Structure has no atoms or exceeds the preview limit');
    viewer.setStyle({}, { cartoon: { color: 'spectrum' }, stick: { radius: .12 }, sphere: { scale: .18 } });
    viewer.zoomTo(); viewer.render();
    status.textContent = `${atoms.length.toLocaleString()} atoms · drag to rotate · wheel to zoom`;
    document.body.dataset.ready = 'true';
  } catch (error) { status.textContent = `Structure preview unavailable: ${error.message}`; document.body.dataset.error = 'true'; }
});
window.addEventListener('keydown', event => {
  if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); parent.postMessage({ type: 'structure-escape' }, parentOrigin); }
}, true);
parent.postMessage({ type: 'structure-ready' }, parentOrigin);

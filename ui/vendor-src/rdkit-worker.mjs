// RDKit's dynamic Embind registration runs only in this disposable worker.
// The containing preview document retains its strict script CSP.
import { R } from './RDKit_minimal-B7RkdM0_.js';
self.onmessage = async ({ data }) => {
  let molecule;
  try {
    if (typeof data !== 'string' || data.length > 1024 * 1024) throw new Error('Molecule input is too large');
    if (data.split('$$$$').filter(record => record.trim()).length > 1) throw new Error('Open one SDF molecule at a time');
    const kit = await R.default({ locateFile: name => {
      if (name !== 'RDKit_minimal.wasm') throw new Error('Unexpected molecule resource');
      return new URL('./RDKit_minimal-tnscgqxm.wasm', import.meta.url).href;
    } });
    molecule = kit.get_mol(data);
    if (!molecule) throw new Error('Invalid molecule');
    const svg = molecule.get_svg(400, 300);
    if (svg.length > 2 * 1024 * 1024) throw new Error('Molecule drawing is too large');
    self.postMessage({ svg });
  } catch (error) { self.postMessage({ error: error.message || 'Invalid molecule' }); }
  finally { molecule?.delete(); }
};

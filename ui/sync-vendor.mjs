// Generate the exact offline WebView runtime. The destination is deleted first
// so obsolete bundles, source maps, examples, and unused assets cannot leak
// into the packaged application.
import { createHash } from "node:crypto";
import { cp, mkdir, readdir, readFile, rm, stat } from "node:fs/promises";
import { gzipSync } from "node:zlib";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));
const repository = dirname(root);
const source = join(root, "vendor-src");
const runtime = join(root, "vendor-runtime");
const webDist = join(repository, "web-dist");

const exactAllowlist = new Set([
  "3Dmol-DfD4xImO.js",
  "RDKit_minimal-B7RkdM0_.js",
  "RDKit_minimal-tnscgqxm.wasm",
  "docx-preview.LICENSE",
  "office-preview-runtime.mjs",
  "scientific-preview.mjs",
  "scientific-preview.css",
  "structure-preview.mjs",
  "structure-preview.css",
  "structure-preview.html",
  "alignment-preview.mjs",
  "alignment-preview.css",
  "alignment-preview.html",
  "rdkit-worker.mjs",
  "rdkit.LICENSE",
  "3dmol.LICENSE",
  "nightingale-msa.LICENSE",
  "echarts.LICENSE",
  "echarts.LICENSE-d3",
  "echarts.NOTICE",
  "highlight-github.min.css",
  "highlight.min.js",
  "katex-Dn761jRB.js",
  "katex-DwwF5kvc.css",
  "nightingale-msa-5.6.0.js",
  "openjpeg.wasm",
  "pdf.min.mjs",
  "pdf.worker.min.mjs",
  "pptx-renderer.LICENSE",
  "qcms_bg.wasm",
  "jszip.LICENSE",
  "sheetjs.LICENSE",
  "vendor-BoUatD0H.js",
  "xlsx-worker.js",
  "xlsx.mini.min.js",
  "xterm-addon-fit.LICENSE",
  "xterm-addon-fit.mjs",
  "xterm.LICENSE",
  "xterm.css",
  "xterm.mjs",
  "zrender.LICENSE",
]);
const katexFont = /^KaTeX_[A-Za-z0-9_-]+\.(?:woff2?|ttf)$/;
const required = [
  "office-preview-runtime.mjs",
  "docx-preview.mjs",
  "pdf.min.mjs",
  "pdf.worker.min.mjs",
  "openjpeg.wasm",
  "qcms_bg.wasm",
  "xlsx-worker.js",
  "xlsx.mini.min.js",
  "pptx-preview.mjs",
];
const pinned = new Map([
  ["xlsx.mini.min.js", {
    bytes: 279_523,
    sha256: "0cb353f830d7288385492c83d277b058ddeac664ca51cf1393aa1fd3e2b70939",
  }],
]);
const officeManifestSha256 = "b4413aa26eab991d4cdcb40a146b2cbd2f485f390a038ef799f6e31ef07a6968";

await rm(runtime, { recursive: true, force: true });
await mkdir(runtime, { recursive: true });

for (const entry of await readdir(source, { withFileTypes: true })) {
  if (!entry.isFile()) continue;
  if (!exactAllowlist.has(entry.name) && !katexFont.test(entry.name)) continue;
  await cp(join(source, entry.name), join(runtime, entry.name));
}

// The maintenance-only multi-entry build pins every generated DOCX/PPTX file
// in one signed-off manifest. It must contain the two entry modules and exactly
// one shared chunk, which is where esbuild places JSZip.
const officeManifestBytes = await readFile(join(source, "office-build.json"));
const officeManifestDigest = createHash("sha256").update(officeManifestBytes).digest("hex");
if (officeManifestDigest !== officeManifestSha256) {
  throw new Error("Office build manifest failed SHA-256 verification");
}
const officeManifest = JSON.parse(officeManifestBytes.toString("utf8"));
const officeFiles = Object.entries(officeManifest.files || {});
const officeNames = officeFiles.map(([name]) => name);
if (
  officeManifest.schemaVersion !== 1
  || officeFiles.length !== 3
  || !officeNames.includes("docx-preview.mjs")
  || !officeNames.includes("pptx-preview.mjs")
  || !officeNames.includes(officeManifest.jszipChunk)
  || !String(officeManifest.jszipChunk).startsWith("office-chunks/")
) {
  throw new Error("Office build manifest does not describe two entries and one shared JSZip chunk");
}
for (const [relative, expectation] of officeFiles) {
  if (
    relative.startsWith("/")
    || relative.includes("\\")
    || relative.split("/").some((component) => component === ".." || component === "")
  ) {
    throw new Error(`Office build manifest contains an unsafe path: ${relative}`);
  }
  const bytes = await readFile(join(source, relative));
  const digest = createHash("sha256").update(bytes).digest("hex");
  if (bytes.length !== expectation.bytes || digest !== expectation.sha256) {
    throw new Error(`Pinned Office bundle failed size/SHA-256 verification: ${relative}`);
  }
  const destination = join(runtime, relative);
  await mkdir(dirname(destination), { recursive: true });
  await cp(join(source, relative), destination);
}

// A locally built web-dist may refresh these hashed scientific-viewer assets.
// Missing web-dist is expected in a clean checkout; committed vendor-src files
// remain the deterministic offline fallback.
const webDistAssets = new Map([
  ["assets/vendor-BoUatD0H.js", "vendor-BoUatD0H.js"],
  ["assets/RDKit_minimal-B7RkdM0_.js", "RDKit_minimal-B7RkdM0_.js"],
  ["assets/RDKit_minimal-tnscgqxm.wasm", "RDKit_minimal-tnscgqxm.wasm"],
  ["assets/3Dmol-DfD4xImO.js", "3Dmol-DfD4xImO.js"],
  ["assets/katex-Dn761jRB.js", "katex-Dn761jRB.js"],
  ["assets/katex-DwwF5kvc.css", "katex-DwwF5kvc.css"],
  ["vendor/nightingale-msa-5.6.0.js", "nightingale-msa-5.6.0.js"],
]);
for (const [relative, name] of webDistAssets) {
  try {
    await cp(join(webDist, relative), join(runtime, name));
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
  }
}
try {
  for (const entry of await readdir(join(webDist, "assets"), { withFileTypes: true })) {
    if (entry.isFile() && katexFont.test(entry.name)) {
      await cp(join(webDist, "assets", entry.name), join(runtime, entry.name));
    }
  }
} catch (error) {
  if (error?.code !== "ENOENT") throw error;
}

for (const name of required) {
  const info = await stat(join(runtime, name));
  if (!info.isFile() || info.size === 0) throw new Error(`Missing required vendor asset: ${name}`);
}
for (const [name, expectation] of pinned) {
  const bytes = await readFile(join(runtime, name));
  const digest = createHash("sha256").update(bytes).digest("hex");
  if (bytes.length !== expectation.bytes || digest !== expectation.sha256) {
    throw new Error(`Pinned vendor asset failed size/SHA-256 verification: ${name}`);
  }
}

const officeAssets = await Promise.all(
  ["xlsx.mini.min.js", "xlsx-worker.js", ...officeNames].map((name) => readFile(join(runtime, name))),
);
const officeGzipBytes = officeAssets.reduce((sum, bytes) => sum + gzipSync(bytes, { level: 9 }).length, 0);
if (officeGzipBytes > 1024 * 1024) {
  throw new Error(`Office preview vendor budget exceeded: ${officeGzipBytes} gzip bytes`);
}

console.log(`vendor runtime generated (${officeGzipBytes} Office gzip bytes)`);

# Copy heavy preview assets from web-dist into ui/vendor for offline Tauri builds.
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$src = Join-Path (Split-Path -Parent $root) "web-dist"
$dst = Join-Path $root "vendor"

if (-not (Test-Path $src)) {
  Write-Host "web-dist missing; using committed ui/vendor"
  exit 0
}

New-Item -ItemType Directory -Force -Path $dst | Out-Null

$files = @(
  "assets\vendor-BoUatD0H.js",
  "assets\RDKit_minimal-B7RkdM0_.js",
  "assets\RDKit_minimal-tnscgqxm.wasm",
  "assets\3Dmol-DfD4xImO.js",
  "assets\katex-Dn761jRB.js",
  "assets\katex-DwwF5kvc.css",
  "vendor\nightingale-msa-5.6.0.js"
)
foreach ($f in $files) {
  $from = Join-Path $src $f
  if (-not (Test-Path $from)) { Write-Warning "skip missing $f"; continue }
  $name = Split-Path $f -Leaf
  Copy-Item $from (Join-Path $dst $name) -Force
}

# PDF.js 5.4.296 is kept as a stable, committed module/worker pair because the
# upstream web-dist only contains the main library folded into a React chunk.
# The wasm decoders (JPEG2000 figures, ICC colors) are fetched from wasmUrl at
# runtime and must match the worker build exactly.
# Update all four files together when upgrading PDF.js.
foreach ($pdfAsset in @("pdf.min.mjs", "pdf.worker.min.mjs", "openjpeg.wasm", "qcms_bg.wasm")) {
  $pdfPath = Join-Path $dst $pdfAsset
  if (-not (Test-Path $pdfPath)) {
    throw "Missing committed PDF.js asset: $pdfPath"
  }
}

# KaTeX fonts (referenced by katex css)
$assetsDir = Join-Path $src "assets"
if (Test-Path $assetsDir) {
  Get-ChildItem $assetsDir -Filter "KaTeX_*" | ForEach-Object {
    Copy-Item $_.FullName (Join-Path $dst $_.Name) -Force
  }
}
Write-Host "vendor synced to $dst"

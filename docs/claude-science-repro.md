# Claude Science — reproduction spec

Reproduction notes for aligning wisp-science with the upstream **Claude Science**
(Operon) desktop app. Captured from a live instance running locally on macOS
(data dir `/Users/xuzhougeng/.claude-science`) plus the compiled `web-dist`
bundle and the product walkthrough video. Labels/copy are transcribed verbatim —
they matter for parity.

Design system: `data-theme="claude" data-color-version="v2"`, brand clay
`#d97757`, Anthropic Sans/Serif/Mono (wisp uses open substitutes on purpose).
Component inventory (from `web-dist/assets`): `ConversationView`, `ArtifactTile`,
`CodePane`, `CsvPreview`, `PdfPreview`, `NotebookPreview`, `KernelNotebookPane`,
`AnnotatableBlock`, `AnnotationRefinementPanel`, `HtmlAnnotationOverlay`,
`OnboardingFlow`, `OnboardingElicitCard`, `OnboardingDropZone`,
`CapabilitiesPage`, `PermissionsWidgetCard`, `ProjectDashboard`,
`ProjectSection`, `ProjectSettings`, `ProjectControlPlane`, `approvalSelection`,
`useComputeProviders`, `useOpenSessionInProject`.

---

## 1. Projects home (`ProjectDashboard`)

- Header: serif **"Claude Science"** wordmark + "Beta"; top-right a library icon
  and a **"+ New project"** button.
- **"Projects"** section (folder icon).
- Project row card: name + a muted **"Example"** tag, then right-aligned columns:
  **`N sessions` · `N artifacts` · `1d`** (relative age). e.g. "Example project ·
  4 sessions · 83 artifacts · 1d".
- wisp today: single project, flat session list — no projects-home.

## 2. Onboarding — "Set up web access" (`OnboardingFlow`)

First-run wizard. Bundle class names: `vp-steps` / `vp-step` (progress dots),
`wizard-welcome`, `wizard-web-access`, `wizard-connectors`, `wizard-task`
(`wizard-task-list`, `wizard-task-loading`, `wizard-task-writein`),
`ob-elicit`, `ob-wiz-tasks`, `onboarding-profile.md`. Steps: welcome → web access
→ connectors/skills → "what do you work on?" (task write-in that seeds
`onboarding-profile.md`) → start.

**Web-access step** — modal titled **"Set up web access"** (globe icon):
> "Claude can pull from these databases and tools while you work. Keep everything
> on, or narrow to your field." · "Change this anytime in Settings."

Each row: chevron ▸ (expandable), category + count, examples subtitle, toggle.
Bottom-right black **"Keep defaults"** button.

| Category | Count | Examples |
|---|---|---|
| NCBI / NIH | 3 | PubMed, Entrez, NIH |
| Genomics & biology | 22 | Ensembl, Reactome, KEGG, gnomAD, GTEx, ENCODE |
| Proteomics | 9 | UniProt, STRING, EBI, Foldseek, RCSB PDB, Protein Atlas |
| Literature & citations | 8 | Semantic Scholar, arXiv, bioRxiv, Crossref, DOI, OpenAlex |
| Clinical & pharma | 14 | FDA, ClinicalTrials, Open Targets, COSMIC, ClinGen, CIViC |

## 3. Conversation view (`ConversationView`)

- Left sidebar: **"‹ Example project ⌄"** (back + project switcher) + panel-toggle
  icon; **"+ New" / "Customize" / "Files"**; session groups by recency
  (**"Yesterday"**, etc.); gear bottom-left.
- Session tabs across the top of center (e.g. "scRNA-seq Immunotherapy… ×"); a
  floating **"⌃ Your last message"** jump pill.
- Composer: text field, **+** and a grid (skills) icon, a **"Default ⌄"** model
  selector, mic. Read-only example sessions show a disabled hint.
- **Artifacts summary block** in-thread: header **"GENERATED · 16"** then a
  **thumbnail grid** (`ArtifactTile`) of figures/docs with filename captions and a
  **"+11 more"** tile. This is the gallery wisp lacks (wisp uses a text tile list).
- Inline **tool-approval card** (video): "Run Python code?" with
  **"Allow for this conversation"** / **"Deny"** (`approvalSelection`,
  `PermissionsWidgetCard`). wisp has the `confirm-request` plumbing but renders a
  centered modal.

## 4. Right-panel viewers

- Image (`ArtifactTile`): header = filename + **⋮ / expand / download / ×**, then
  the PNG. Figures often pair with a caption doc (*Panels / Artifacts / what is
  real vs. illustrative*).
- `CodePane` (line-numbered, tabs), `PdfPreview` (own CSS), `CsvPreview`,
  `NotebookPreview` / `KernelNotebookPane` (live-kernel cells), `AnnotatableBlock`
  + `AnnotationRefinementPanel` (comment pins on figures/text).

## 5. Settings / Capabilities (`CapabilitiesPage`)

Left rail, two groups. Header per section: **"‹ ›"** history nav + title +
expand + **×**. Filter dropdown "All (N)", search "… ⌘K", primary action button.

**Capabilities:** Skills · Connectors · Specialists · Memory · Compute · Network
**Workspace:** Permissions · Credentials · Storage · Usage · General

### Skills
"All (16) ⌄" · "Search skills… ⌘K" · **"+ Add skill ⌄"**. Group **"Featured —
Research skills from Anthropic"**. Each row: name + toggle. Featured set:
AlphaFold2, Boltz, Borzoi, Chai-1, DiffDock, ESM-2, ESMFold2, Evo 2, Indication
Dossier, LigandMPNN, Literature Review, OpenFold3, ProteinMPNN, scGPT.

### Connectors
"All (24) ⌄" · "Search connectors… ⌘K" · **"+ Add connector ⌄"**. Row: icon +
name + checkmark + toggle. Group "Featured — Research connectors from Anthropic":
BioMart, Cancer Models, CellGuide, Chemistry, Clinical Genomics, Drug Regulatory,
Expression, Genes & Ontologies, Genomes, Human Genetics, Ketcher Chemistry,
Literature Graph, … (24 total). Directory connectors need a claude.ai session
("Directory connectors unavailable — Your claude.ai session has expired.").

### Memory
Top-right **"Off ⚪" toggle + "🗑 Clear all"**. Off banner: "Memory is off. Claude
won't save new notes or recall existing ones, but the notes below are kept and
stay editable. Turn memory on to resume." + **"Turn on"**. Two-column: category
list (**"About you  0"**, **"+ New category"**) | notes pane ("No notes yet." +
**"+ Add"**).

### Compute (`useComputeProviders`)
Intro: "Connect where Claude runs heavy compute — your own servers over SSH,
serverless GPUs on Modal, or inference endpoints."
- **SSH hosts** — "Servers, clusters or job submission nodes from your SSH host
  lists" + **"+ Add SSH host"** ("No SSH hosts yet").
- **Cloud providers → Modal** — "Serverless GPUs on your own Modal account —
  connect in about a minute." + **"Connect"**.
- **Model endpoints → NVIDIA BioNeMo NIM** — "local NIM docker containers, or
  externally hosted NIM APIs. Each registration asks you individually; disabling
  stops and removes them all." + **"Connect"**.

### Network
- **Package mirror** — "Fetch conda and Python packages from your organization's
  internal mirror (Artifactory, Nexus, or similar)…". Inputs: **Conda channel
  mirror** + Check, **Python package index (pip)** + Check. "When a mirror is set,
  packages are fetched only from it — public package hosts are removed from the
  sandbox network allowlist."
- **CA bundle path** input — "…needed when a TLS-inspecting proxy (Zscaler,
  Netskope, …) re-signs traffic." "In effect: none — system default trust". Save.
- **Claude Science domains** allowlist ("When Claude runs code for you, that code
  can only connect to domains on this list."): Package management 16 (pip, conda,
  npm, CRAN, Bioconductor, GitHub); NCBI / NIH 3; … (same categories as web
  access), each with a toggle.

### Permissions (`PermissionsWidgetCard`)
"All (8) ⌄". **"Registry writes — Agent / skill registry mutations that persist
across sessions"** + **"Revoke all"**. Rows each tagged **"Global"**: Create
agent, Update agent, Publish skill, Edit skill, Attach skill, Detach skill, Attach
connector, Detach connector.

### Credentials
"All (8) ⌄" + **"+ Add"**. **"Services — API keys and tokens for services Claude
uses on your behalf — stored encrypted on your computer"**. Rows (icon + name +
**"Connect"**): AWS, GitHub, Google Cloud, Literature access (journals, etc.),
Microsoft Azure, Modal, NVIDIA API, OpenAlex. **"Custom — …"** "Add a custom
credential to store a key for any other service".

### Storage
- **Data location** — "Where Claude Science keeps your projects, files, and
  history on this machine" + **"Change location"**. Path `~/.claude-science`,
  "N MB on disk · default location".
- **Disk usage** stacked bar: Artifacts / Conda environments / Workspace / Tool
  results, Total, "Available on disk".
- **Cloud storage** — "Browse and manage bucket connections" + "Go to
  Credentials".

### Specialists / Usage / General
Not yet captured — screenshots needed.

---

## Mapping to wisp

- **Connectors + Network allowlist** ↔ wisp's ~80 bundled MCP bio-tools +
  path-sandbox. Biggest parity gap: a categorized on/off UI over MCP servers and a
  domain allowlist, instead of `WISP_MCP_PKG` env launches.
- **Skills** ↔ wisp `use_skill` / `SKILL.md` catalog — needs a toggle UI.
- **Memory** ↔ wisp memory files — needs the on/off + category UI.
- **Credentials/Storage/Compute** ↔ wisp keyring + `.wisp/` + Python REPL — mostly
  backend work.
- **Inline approval card** ↔ wisp `confirm-request` — presentation change only.
- **Artifacts gallery / Projects home** ↔ wisp right panel / sessions — frontend.

Still needed (send screenshots): the onboarding wizard steps (welcome, connectors,
"what do you work on?"), Specialists / Usage / General settings, the inline
approval card, and an `ArtifactTile` gallery close-up.

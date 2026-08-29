---
name: analysis-workflow
description: "Organize multi-step scientific analyses into reproducible, self-contained modules. Use for workflows such as QC→PCA→DEG→GSEA that produce scripts, inputs, figures, tables, and methods. Creates a stable module layout, records exact inputs/parameters/package and database versions in each module README, keeps large data as references instead of copies, and verifies outputs before completion."
license: Apache-2.0
wisp:
  schema_version: 1
  domains: [bioinformatics]
  research_stages: [observation, analysis, validation]
  roles: [analyst, validator]
  evidence_types: [project-data, omics, computational]
  outputs: [analysis-module]
  side_effects: code_execution
---

# Reproducible Analysis Modules

Use this skill for a scientific workflow with two or more analysis stages or
when a stage produces scripts plus result files. It defines project
organization and methods capture; load `figure-style` as well whenever a stage
creates or revises a plot.

## 1. Plan module boundaries

Before writing outputs, list the modules and the dependency edges between them.
Use stable ASCII names. Conventional acronyms such as `QC`, `PCA`, `DEG`, and
`GSEA` may stay uppercase; otherwise prefer a short kebab-case name.

Respect a compatible layout that already exists. Do not reorganize unrelated
user files merely to impose this convention.

## 2. Default module layout

Create only directories the module actually needs:

```text
<module>/
├── scripts/
├── input/
├── output/
│   ├── figures/
│   └── tables/
└── README.md
```

- `scripts/` contains the executable source for this module.
- `input/` contains small module-specific inputs or a manifest/reference to the
  canonical data. Do not duplicate a large dataset by default.
- `output/figures/` contains rendered figures from this module only.
- `output/tables/` contains machine-readable results from this module only.
- `README.md` is the module's reproducibility record and methods source.

Shared immutable/raw data may live in project-level `data/`. A downstream module
references an upstream output by a project-relative path; it does not silently
copy or rename that output.

## 3. Make outputs attributable

Every output must have one producing script or recorded command. Use
deterministic filenames that identify the analysis and content. Keep temporary
files outside the final output directories or name them clearly as temporary.

Script persistence and process lifetime are separate concerns. When a Python or
R runtime already holds an expensive object in memory:

- keep the reproducible analysis in a project-local `.py` or `.R` file;
- execute that file with the `python`/`r` tool's `script_path` in the same
  runtime, declaring the input bindings with `required_objects`;
- keep heavyweight loading in a separate bootstrap script or explicit loader
  cell; analysis scripts consume the loaded object and must not reload it;
- use `run_in_context`, `python file.py`, or `Rscript` only for a deliberately
  fresh, state-independent batch execution.

Record the runtime script path and the returned source hash/runtime generation
in the module README. For clean-room replay, an optional batch wrapper may load
the data once and then call the same analysis functions; it is not the default
hot-iteration path.

Before completing a module, verify:

1. every declared output exists and is non-empty;
2. every table can be parsed in its declared format;
3. every figure was rendered and visually inspected using `figure-style`;
4. README input and output paths resolve from the project root;
5. reported thresholds and parameters match the actual script.

## 4. Update README.md at module completion

Create or update these sections:

```markdown
# <Module>

## Purpose
<scientific question and role in the workflow>

## Inputs
- `<project-relative path>` — source, upstream module, checksum or version when available

## Methods
<method in prose, including transformations, statistical tests, correction method,
thresholds, seeds, and other result-changing parameters>

## Software and data sources
- R/Python package: exact version
- External API/database: release or access date
- Wisp/model/runtime metadata: exact recorded value when available

## Commands and scripts
- `<project-relative script>` — how it was executed

## Outputs
- `<project-relative path>` — meaning and format

## Limitations
<assumptions, exclusions, and unresolved reproducibility gaps>
```

Write methods from executed code and recorded parameters, not from a generic
template. Do not claim a package, database, model, OS, or version that was not
actually used or observed.

## 5. Capture exact versions without dumping the world

Record direct dependencies used by the module:

- R: `packageVersion("<package>")` for named packages and `sessionInfo()` for
  the runtime context.
- Python: `importlib.metadata.version("<distribution>")`; use the project lock
  file when it is the authoritative environment record.
- External databases/APIs: release identifier when available, otherwise access
  date plus endpoint/source.
- Wisp version and model profile: use runtime/session metadata only when it is
  available. Write `unavailable` rather than guessing.

Do not paste an entire global `pip freeze` into every module. If a complete
environment export is useful, save it once as a separate artifact and link it
from the README.

## 6. Finish the workflow

After all modules pass their checks, summarize the dependency chain and link the
module READMEs. Treat those READMEs as the first-version source of truth.
Generate a root `METHODS.md` only when the user asks for it or a deterministic
project tool can derive it from the module records; do not maintain a second
hand-edited copy that can drift.

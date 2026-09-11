# Preparing Skills for Wisp Science

This guide targets Wisp **1.11.0**. The examples are checked against the current
[parser](../crates/wisp-skills/src/manifest.rs) and package validator in automated
tests. They have **not** been validated through a model-driven research task.
Current behavior, recommendations, and future work are distinguished below.
Issue #1018 is a proposal, not an implemented Skill permission manifest.

## 1. Minimal package and responsibilities

A discoverable package is a directory containing `SKILL.md`. Use one portable
package name and place the directory directly under a discovery root. A minimal
legacy-compatible document is:

```skill
---
name: handoff-example
description: Organize supplied project notes into a handoff when the user asks to transfer an existing research project.
---
# Research handoff
Responsible for: organizing supplied notes and source paths.
When to use: the user asks to hand over an existing project.
Inputs: supplied notes or files; ask and pause if these are missing.
Deliverable: a source-linked handoff in chat.
Out of scope: new research, computation, and external publication.
```

The parser requires a closed YAML frontmatter block; `name` falls back to the
package directory for legacy packages. Store installation requires a nonempty
description and a portable, non-hidden single-component name. Prefer explicit
`name` and `description`. Unknown top-level YAML fields are preserved; arbitrary
fields are **not** allowed inside `wisp`.

Describe responsibilities in the order **responsible for → when triggered →
inputs → deliverables → out of scope**. Include both positive and negative
trigger examples. Avoid descriptions that claim every research task.

A complete installable example, including its referenced template, is
[research-handoff](../community-skills/examples/research-handoff/SKILL.md).
Its [directory entry](../community-skills/index.json) also describes dependencies,
license, source, compatibility declarations, and operation boundaries. The
example becomes remotely installable after these files are published to the
indexed repository ref; local tests use the checked-out package without GitHub.

## 2. Optional Wisp metadata

Traditional Skills without `wisp` remain supported. A valid extension is:

```skill
---
name: evidence-example
description: Organize supplied evidence into a matrix for review.
wisp:
  schema_version: 1
  domains: [scientific-literature]
  research_stages: [synthesis]
  roles: [synthesizer]
  evidence_types: [literature]
  outputs: [evidence-matrix]
  side_effects: read_only
---
Use only the supplied material. Return a matrix in chat and mark missing evidence.
```

Current controlled values (the parser is authoritative):

| Field | Allowed values |
| --- | --- |
| `schema_version` | `1` |
| `domains` | `general`, `bioinformatics`, `oncology`, `single-cell`, `genomics`, `transcriptomics`, `proteomics`, `scientific-literature` |
| `research_stages` | `observation`, `retrieval`, `analysis`, `hypothesis`, `validation`, `synthesis` |
| `roles` | `retrieval`, `analyst`, `planner`, `critic`, `validator`, `synthesizer` |
| `evidence_types` | `literature`, `project-data`, `omics`, `single-cell`, `computational`, `experimental` |
| `outputs` | `evidence-matrix`, `hypothesis-card`, `research-design`, `analysis-module`, `literature-review`, `risk-map`, `validation-plan`, `research-timeline` |
| `side_effects` | `read_only` (default), `network`, `project_write`, `code_execution`, `external_service` |

All fields except `schema_version` are optional. Metadata describes semantics;
it does not grant permissions. Author, license, responsibilities, dependencies,
compatibility claims, and feedback URLs belong in the community index and/or
human-readable body, not invented `wisp` fields. Top-level `tags` accepts a list
or comma-separated string; top-level `version` is optional.

## 3. Resources and distribution boundaries

`references/`, `scripts/`, and `assets/` are optional. Reference resources relative
to the installed package, for example `[template](references/template.md)`.
Distribute the entire containing directory. A `SKILL.md` GitHub file link resolves
to its parent package and includes those resources. Do not use absolute author
paths, `../` references outside the package, symbolic links, or platform-specific
reserved filenames. Keep portable filenames and avoid case-only distinctions.

The store checks explicit Markdown relative links outside fenced examples and inline-code paths beginning
with `references/`, `scripts/`, or `assets/`. Dynamically computed paths and tool
names in prose still require author review. It reports incomplete Git submodule/LFS resources instead of installing pointers.
It does not rewrite broken resources, install Git submodules, or download Git LFS objects. Package resources should be
ordinary committed files. Store downloads do not preserve executable mode bits;
document interpreter-based commands such as `python scripts/analyze.py`.

Different limits serve different purposes:

| Surface | Current limit and meaning |
| --- | --- |
| Installed package Files preview | UTF-8 text ≤ 1 MiB/file; 2,000 files; 32 levels. Hidden entries and symlinks omitted. These are preview limits, not universal Skill schema rules. |
| GitHub store preview | `SKILL.md` ≤ 1 MiB each / 4 MiB combined; at most 100 discovered Skills per selected directory. Narrow a repository URL if exceeded. |
| GitHub store download/extraction | Entire repository ZIP ≤ 32 MiB, 4,000 ZIP entries, 8 MiB/file, 128 MiB expanded, portable path ≤ 1,024 bytes and 64 levels. Exceeding these rejects this installation; it is not a universal runtime file limit. |
| Runtime execution | Governed by each Wisp tool and execution context; preview limits do not define interpreter memory, dataset sizes, or execution duration. |

The first store version downloads a commit-pinned GitHub repository ZIP, then
installs only the selected package. Very large repositories should publish small,
self-contained Skill repositories. Private repositories are not supported.

## 4. Execution and tools

[`use_skill`](../crates/wisp-skills/src/tool.rs) loads Skill instructions and
provides package location/resource information. Discovering, previewing,
installing, or loading a Skill does not execute scripts or install dependencies.
The Agent subsequently uses the available Wisp tools under host policy.

Root-level `runtime.py` and `runtime.r` are special sidecars. Loading a Skill
appends instructions for the persistent `python`/`r` tool to load those files;
loading the instructions itself does not execute them. Python uses
`exec(compile(...))`; R uses `source(..., local = TRUE, encoding = "UTF-8")`.
A Python `if __name__ == "__main__"` block also runs when loaded in this namespace.
Keep initialization lightweight and place heavy actions in explicitly called
functions. Load again after interpreter restart or context change. Python and R
state is separate and not durable across process restarts.

Ordinary `scripts/*.py` or `scripts/*.R` have no automatic loading semantics.
Document their invocation. Long computation should use the existing structured
Run workflow instead of requesting longer shell timeouts. See
[Scripts and interactive analysis](skills.md#scripts-and-interactive-analysis).

## 5. Models, platforms, and dependencies

List **required** and **optional** model capabilities, tools/MCP services,
Python/R packages, CLIs, operating systems, network access, and execution contexts.
A parser pass does not prove those dependencies exist. Declare when vision is
needed; do not assume a text-only model can inspect plots. If a required capability
is absent, explain the missing item and pause dependent work. Optional dependencies
must have a documented fallback.

Use portable relative paths and account for Windows and macOS. A local package
path is not automatically readable on SSH or WSL. Sidecar instructions allow
reading local code and passing it to the remote runtime, but sibling files are
not automatically transferred. Explicitly describe transfer, remote paths,
checksums, and input requirements. Do not default to synchronizing large data.

## 6. Permissions and confirmation

Use Wisp's existing tool approval and confirmation cards. Installation does not
authorize network, MCP, file writes, external submissions, or runtime installation.
The user's task may authorize an action; Skill prose cannot bypass host policy.
When input or consent is required, an unanswered question is not permission.
Do not hardcode credentials or ask authors to put secrets in package metadata.

## 7. Outputs and completion

State expected formats, output directory rules, and required deliverables.
If a file is promised, verify that it actually exists, is readable, and is at the
reported path before declaring it delivered. Distinguish file/format checks,
visual checks, successful computation, and scientific validity. Report checks
that were not performed. Neither a package validator nor an attractive plot
proves a scientific conclusion.

## 8. Migrating another platform's Skill

Before adaptation, a platform-specific instruction might say:

```text
Use platform.read_workspace, assume references are auto-loaded, invoke
platform.python_session, then publish using platform.upload without asking.
```

For Wisp, adapt the workflow itself:

```text
Use the available Wisp read tool to read the supplied project files and
references/template.md relative to the loaded package. Check that the python
runtime is available before computation. Return the result in chat, or write a
requested artifact to the project under normal host approval. External upload
requires a supported tool and the user's authorization. Stop if either is absent.
```

Audit proprietary tool names, hidden auto-loading assumptions, package resources,
unsupported metadata, runtime dependencies, and authorization behavior. Merely
renaming the Skill is not evidence of compatibility. The complete
[research-handoff example](../community-skills/examples/research-handoff/SKILL.md)
implements the read-and-handoff subset with no proprietary dependencies.

## 9. Validation and contribution

Use the [community contribution guide](../community-skills/README.md) and copy an
entry from [index.json](../community-skills/index.json). Every entry needs source,
ref, package path, responsibilities, required/optional dependencies, operation
boundaries, author/license, supported Wisp version, reported verification version
(or `null`), known limitations, and feedback URL. Community inclusion is not an
official maintenance or verification claim. Keep the body and index consistent.

Before submitting a PR:

1. Parse frontmatter using Wisp's current parser. Validate resource links and
   package boundaries. Run `cargo test -p wisp-skills` for this repository.
2. Test positive and negative triggers; simulate missing input, unavailable tools,
   dependency failure, and denied permissions without real services.
3. Install and load in a test Wisp project, inspect package files, then verify an
   example task and artifacts. Record the exact Wisp version and context if done.
4. Distinguish **format passed**, **dependencies pending/configured**,
   **author-declared compatibility**, and **reported runtime validation**.
   Do not mark runtime verification based only on parsing or install success.
5. Submit the index/package change through a normal reviewed PR. Updating a
   listing does not update users' installed packages.

## 10. Ownership, upgrades, and recovery

Keep user packages in `~/.wisp/skills`, project packages in `.wisp/skills`, or
configured extra directories. Never put user packages into application resources.
Discovery precedence remains bundled → project → global → extra → plugin.
Names can be shadowed without files being deleted. Use `list_skill_catalog` to
inspect discovered/effective/shadowed/parse-error counts and source paths; search
matches and current enabled counts are different quantities.

The store writes full commit, original source URL, repository, ref, and package
path to `.wisp-source.json` inside the persistent package. Installed Skill details
show this provenance, even when the remote entry disappears. Application updates,
directory refresh, and GitHub branch movement do not update or remove user packages,
reset tags, or re-enable existing disabled Skills. Local edits remain local.
Store installation refuses a same-name source and never replaces an existing
package; retain the current package or review/manage it from Installed Skills.
Automatic Skill updates, diff/backup/replace UI, and project-scoped store installs
are follow-up work, not features of this version.

Staging lives outside the discovery root. A failed or interrupted install cannot
expose a partial Skill there. Only the owned staging directory is cleaned up;
existing packages remain intact. After a crash, an abandoned staging directory
may remain, but it is not discovered as an installation.

Historical resource layouts include both `resources/skills` and
`resources/_up_/skills` (see [path resolution](../crates/wisp-paths/src/lib.rs)).
No supplied history proves an application updater deleted a user's package, and
this change does not add an updater migration hook. **Before replacing an old
installation directory**, copy any user-added or modified package from either
layout to a backup outside the installation, preserve its complete resources and
configuration, compare it with the matching old release, then import into the
persistent global directory. If no old manifest is available, preserve all
uncertain packages and report their origins; never infer ownership and delete.
Automatic pre-upgrade identification, migration snapshots, unavailable-source
reporting, and a UI catalog-diff report remain separate follow-up changes.

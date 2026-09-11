---
name: research-handoff
description: Prepare a traceable research handoff when a user asks to transfer an existing project using supplied notes and files; do not use for new literature searches or new analysis.
tags: [research, writing, handoff]
license: AGPL-3.0-only
---
# Research handoff

## Responsible for
Organize existing observations, decisions, source paths, and unresolved questions.
Use this Skill when the user asks to hand over an existing research project.
Do not invent experiments, citations, computed results, or missing decisions.

## Inputs and dependencies
Require a handoff request and user-supplied notes or project files. If absent,
ask for the missing inputs and pause dependent work. A text-capable model is
sufficient. Use Wisp's `read` tool only if local input files need reading.
No MCP, network, Python, R, or OS-specific runtime is required.

## Procedure
Read [the handoff template](references/handoff-template.md) relative to this
Skill package directory. Identify the project objective and intended audience.
Separate observed facts from proposed next steps. Include the source file path
or supplied URL for each substantive observation. Mark unknowns explicitly.

## Deliverables and confirmation
Return the handoff in chat unless the user requested a file. When a file is
requested, write Markdown within the active project to the requested path and
follow Wisp's normal write approval. Never infer consent from an unanswered
question. Do not upload or submit anything externally.
After writing, verify the actual file exists, is readable, and has the expected
path and sections. State separately whether the scientific claims were checked;
format and file checks alone do not establish scientific correctness.

## Out of scope
No new literature retrieval, computation, vision-based figure interpretation,
external publication, or environment setup. If requested, explain what additional
workflow and dependencies are needed instead of claiming this Skill covers them.

# Wisp Science Basics: Skills

When using AI for research, some requirements come up repeatedly: verify literature sources, retain raw plotting data, record analysis parameters, and distinguish authors' conclusions from your own interpretation.

Repeating all of these requirements from scratch makes omissions likely. A Skill lets you save a tested method so Wisp Science can read and apply it to relevant tasks.

The preceding tutorial introduced [MCP](wisp-science-mcp.md). Here we explain Skills, how to use them in Wisp, and how to preserve your own research practices as reusable instructions.

**A Skill is a method package the agent can read.**

A Skill centers on `SKILL.md`, which describes when to use it, what steps to follow, and what to deliver. Scripts, references, and templates can accompany it. This uses the Agent Skills open format. See the [Agent Skills introduction](https://agentskills.io/home).

A literature-review Skill might require the agent to:

- Clarify the question, organism, and time range first.
- Search using available tools and verify citations.
- Compare supporting and opposing evidence and explain differences between studies.
- Organize conclusions around the question, retaining sources and uncertainties.

Wisp can then read this guide and apply the same checks to a new research question.

A Skill provides reusable methods and materials. Results still depend on the model, tools, input data, and how specific the guide is.

**See how Skills and MCP work together.**

| Part | Role in a plant single-cell literature review |
| --- | --- |
| Your task | Topic, scope, and desired deliverable |
| Skill | How to search, screen, verify, and organize evidence |
| MCP tools | Query databases and return paper records and source URLs |
| Other Wisp tools | Read files, run analyses, and save notes and figures |

For example, `literature-review` guides synthesis, PubMed tools retrieve records, and file tools save the notes.

A Skill can also work solely with supplied material, such as formatting paper notes using a lab template. Whether it needs MCP, Python, R, or an external service depends on its workflow.

**Start with an existing Skill for a familiar task.**

Open **Settings → Skills** to see names, sources, tags, and enabled status. Use search and tag filters to narrow the list.

Click a Skill for its description, source directory, and package files. Read rendered `SKILL.md` or switch to source to inspect metadata. Text resources such as scripts are available for read-only inspection.

Browsing files does not execute scripts. You can inspect disabled Skills before enabling them. See [Wisp Skills](https://github.com/xuzhougeng/wisp-science/blob/main/docs/skills.md) for discovery rules and the interface.

| Bundled Skill | Suitable work |
| --- | --- |
| `literature-review` | Search, verify, and synthesize literature; compare evidence and gaps |
| `public-data-access` | Plan public data acquisition and record files, sources, and checksums |
| `figure-style` | Check data presentation, labels, and readability in scientific figures |
| `paper-narrative` | Organize relationships between figures, claims, and a paper's narrative |

Choose a relevant Skill, read its instructions, and validate it on familiar material first.

**Describe the task or attach a Skill manually.**

You can let Wisp discover a Skill from the task:

> Review research on plant root-tip single-cell atlases. Find a suitable literature-review Skill first and follow its workflow. Compare study organisms, methods, and findings, with verifiable paper links.

The built-in agent uses `search_skills` to discover Skills and `use_skill` to read their instructions. You normally do not need to type these tool names.

If you know which Skill you want, type `/` in the message box to open the command, Skill, and workflow picker. Continue typing to filter; click a result or select it with the arrow keys and Enter. It is attached to the next message as a reference.

After attaching `figure-style`, for example:

> Plot the treatment-group results from my attached data. Follow the selected Skill to check axes, units, sample sizes, colors, and labels. Preserve individual data points and save both the figure and plotting script in the project.

Manual attachment applies to that turn, not a permanent project setting. Enabled means available; attach or name a Skill when you specifically want it used on a turn.

**Browse the Skills store and install community packages as needed.**

Open **Settings → Skills → Browse community Skills**. The store offers Wisp's community directory and three default sources:

| Source | Contents and requirements |
| --- | --- |
| [OpenAI Skills](https://github.com/openai/skills/tree/main/skills/.curated) | Curated Codex Skills. Labeled a legacy repository; this source does not automatically switch to OpenAI Plugins |
| [Anthropic Skills](https://github.com/anthropics/skills/tree/main/skills) | Anthropic's public Claude Skills, selected as individual packages |
| [BEAR Research Skills](https://github.com/fei0810/bear-research-skills/tree/main/skills) | Literature research Skills and workflows; configure SciMaster CLI before use |

The eight `bear-*` Skills are now available from the store instead of being bundled. For example, preview `bear-support` when you need supporting literature for a claim. Existing user-installed BEAR copies are preserved, and leftover bundled copies from older versions do not block marketplace installation.

To install a Skill:

1. Click **Preview package** on a community entry, or select a default source, wait for its list to load, and search for a package.
2. Read `SKILL.md`, the source details, and validation results. Follow the source links to check the license and dependencies. Name conflicts and validation issues appear in the preview.
3. Choose **Review installation → Confirm and install**. Select one complete package at a time; the entire repository is not installed as a batch.

The preview button and status card show loading indicators. **Cancel** or Escape discards the pending preview without installing anything. Use **Load / refresh source** to retry a failed source request. Fetching source packages requires network access; inclusion and successful format validation do not imply verified Wisp runtime behavior.

Installed packages live under `~/.wisp/skills` as **global Skills** discoverable across projects. The current project's index refreshes automatically. Installation does not execute downloaded scripts or configure dependencies. App upgrades and source refreshes do not automatically update or remove these packages. A same-name package is preserved, and the store reports the conflict.

For another public GitHub repository, Skill directory, or `SKILL.md` link, choose **Add from GitHub**, enter the URL, and click **Discover Skills**. Follow the same preview and confirmation steps.

This tutorial uses the interface's name, **Skills store**. Its marketplace sources install Skill packages; **Settings → Plugins** manages plugins and their accompanying Skills. Adding a source does not activate Claude Code or Codex plugin integrations. Plugin-provided Skills remain enabled, disabled, or removed through their parent plugin.

**Import a Skill from local files when someone shares a package with you.**

Go to **Settings → Skills → Add Skill** and choose:

1. **Add SKILL.md or ZIP** for a standalone file or packaged Skill.
2. **Add folder** for a directory containing the full Skill.

A ZIP can contain `SKILL.md` directly or one outer Skill directory. Import one Skill package at a time.

A package with supporting resources might look like:

```text
lab-paper-note/
  SKILL.md
  references/
    reading-checklist.md
  assets/
    note-template.md
  scripts/
    check_note.py
```

`SKILL.md` is the entry point; other files are optional. If it references scripts or templates, preserve the whole directory when sharing and importing so relative paths work.

The local Add Skill action installs or updates a **global Skill** discoverable across projects. For a project-specific workflow, place it under:

```text
<project directory>/.wisp/skills/lab-paper-note/SKILL.md
```

Click **Reload Skills** afterward. Wisp rescans, and idle session agents use the updated index on their next turn without restarting the app. Newly discovered Skills are enabled by default; previously disabled ones stay disabled.

Use a distinct name such as `lab-paper-note` for your own version. Name collisions have a fixed precedence, with bundled Skills taking priority over identically named alternatives.

**Create a simple paper-note Skill without code.**

Suppose your lab wants each paper recorded in the same way: question, methods, key findings, evidence locations, and implications for your project.

Create a `lab-paper-note` directory with this `SKILL.md`:

```markdown
---
name: lab-paper-note
description: Organize notes on supplied papers using the lab template. Use for close reading, lab meetings, and method comparisons, preserving evidence locations and separating authors' conclusions from readers' judgments.
---

# Laboratory paper reading notes

Work from the user's supplied PDF, text, or excerpts.

1. Confirm what material is readable. If only an abstract or excerpt is available, state that at the beginning.
2. Record supplied title, authors, year, and DOI. Mark missing information as not provided.
3. Organize the note by question, materials and methods, key results, limitations, and implications for the current project.
4. Locate key evidence using available page, section, or figure numbers.
   If there are no location markers, quote a short supporting phrase and identify it as a user-supplied excerpt.
5. Separate the authors' reported findings from the reader's inferences.
6. Save Markdown notes at the user's requested location and report the path.
   If none is specified, use a new file under notes/papers/ without overwriting existing notes.

Before finishing, check evidence locations, invented information,
material coverage, and questions that cannot be resolved from the input.
```

The YAML between the opening `---` lines provides metadata. `name` identifies the Skill; `description` tells the agent what it does and when to use it. The remaining Markdown describes the method. This example uses just those two basic fields. See the [format specification](https://agentskills.io/specification).

After importing, attach `lab-paper-note` and a paper or excerpt, then send:

> Follow the attached Skill to organize this material and save the note under notes/papers/. If only part of the paper is available, say so rather than filling in missing conclusions.

Compare the output with the original: are key results accurate, are evidence locations findable, and are guesses kept separate from the paper's conclusions? Add missing checks to `SKILL.md`, reload, and try again. Small validations gradually improve a useful long-term Skill.

**Extract a successful workflow from a conversation.**

After an analysis, figure, or literature task that worked well, use `/save-as-skill`.

This fills the message box with a prompt to extract a Skill. You can add scope and a destination before sending; the command does not immediately save the entire conversation as a Skill.

For example:

> Turn this paper-note workflow into a reusable Skill named lab-paper-note under .wisp/skills/lab-paper-note/ in the current project. Preserve the agreed output template and verification steps. Replace this paper's title, paths, and project names with inputs the user must provide. Explain how to reload and test it.

Review the extracted file for clear applicability, one-off results accidentally retained, machine-specific absolute paths, and whether it works on another input.

**Skills with scripts need an execution environment.**

A Skill may include Python or R helpers. Importing it does not install interpreters or dependencies. External services may also require networking or credentials.

Wisp gives root-level `runtime.py` and `runtime.r` a specific role: loading helper functions into the persistent Python/R interpreter for reuse. Browsing the Skill or reading its instructions does not automatically execute them; the agent follows loading guidance when helpers are needed.

Ordinary scripts under `scripts/` run according to the Skill instructions. For SSH/WSL, verify that scripts and resources are accessible there; a local path does not necessarily exist remotely. A clear text-only guide like the example is enough to start creating your own Skills.

**Check discovery first, then execution requirements.**

| Symptom | Check first |
| --- | --- |
| Imported Skill cannot be found | Exact `SKILL.md` filename, metadata, and ZIP/folder nesting |
| Store source or preview fails to load | GitHub connectivity and the displayed error; retry with Load / refresh source |
| Store reports a name conflict | Inspect the existing Skill's source in the installed list; the store preserves existing files rather than overwriting them |
| Previously bundled `bear-*` Skills are missing | Install the needed packages from BEAR Research Skills and configure SciMaster CLI |
| Old instructions seem to persist | Reload Skills and check higher-priority namesakes |
| Enabled Skill is not used | Attach it with `/` or name it explicitly, and state a concrete task |
| Script or template fails | Complete package, dependencies, and paths in the selected environment |
| Output does not follow the template | Specific template requirements and a small validation input |
| Required database search fails | MCP connection, credentials, network, and the actual tool error |

Start with a repeated small task: a paper note, a figure check, or data organization. Write down an established method, test it on new material, and revise it into a workflow that fits your research habits.

> This tutorial reflects Wisp documentation and Skill implementation when written. Labels may vary between versions. Examples explain configuration and use; they are not completed research tasks.

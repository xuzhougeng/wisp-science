//! Layered system-prompt assembly, ported from mangopi-cli's `SystemPrompt`.
//!
//! Sections: base intro, safety, built-in rules, tool guidance, skills
//! guidance, user rules (memory), environment.

use std::path::Path;
use wisp_skills::SkillIndex;

pub struct SystemPrompt<'a> {
    project_root: &'a Path,
    skills: &'a SkillIndex,
    project_instructions: Option<String>,
    user_rules: Option<String>,
    compute_hosts: Option<String>,
}

impl<'a> SystemPrompt<'a> {
    pub fn new(
        project_root: &'a Path,
        skills: &'a SkillIndex,
        compute_hosts: Option<String>,
    ) -> Self {
        let project_instructions = std::fs::read_to_string(project_root.join("AGENTS.md"))
            .ok()
            .filter(|s| !s.trim().is_empty());
        let user_rules = std::fs::read_to_string(project_root.join(".wisp").join("WISP.md"))
            .ok()
            .filter(|s| !s.trim().is_empty());
        Self {
            project_root,
            skills,
            project_instructions,
            user_rules,
            compute_hosts,
        }
    }

    fn base_intro() -> String {
        "You are **wisp-science**, an interactive AI agent for software engineering and scientific computing tasks. \
\"wisp-science\" is your name and identity — always refer to yourself as wisp-science. You are NOT \"Claude Science\", \
\"Claude\", \"ChatGPT\", \"Gemini\", or any other assistant or product, and you must never call yourself by those names, \
even though you are built on top of a large language model.\n\n\
About the model that powers you: your provider and model are configured by the host (the wisp-science app) and chosen \
by the user — the backend may be an Anthropic, OpenAI-compatible (e.g. GLM, DeepSeek, Qwen), or other model, and it can \
change between sessions. Do NOT assume or claim a specific vendor or model name. If the user asks which model you use, \
tell them the underlying model is whatever is set in wisp-science's Settings (provider + model), that you can't reliably \
read the exact version from inside a turn, and point them to Settings — never guess \"Claude\" or any other name.\n\n\
Use the instructions below and the tools available to you to assist the user.\n\
IMPORTANT: Never generate or guess URLs unless you are confident they help the user with their work. \
In user-facing prose and final answers, always refer to files inside the current project with project-relative paths \
using forward slashes (for example `analysis/results/figure.png`). Never expose the absolute project-root prefix or \
use Windows backslashes in those displayed project paths. Format mentioned project files as Markdown links whenever \
possible so users can open them directly. Tool arguments may still use absolute or native paths when \
the tool requires them. If you need to read a directory, use the `shell` tool \
with the current platform's directory-listing command because the `read` tool cannot read directories.".into()
    }

    fn safety() -> String {
        "## Safety\n\n\
Destructive commands and any access outside the project root require explicit user confirmation.\n\
Before deleting scientific data, run directories, intermediate files, or inputs—even when the \
user supplied the deletion command—first perform a read-only inventory of the exact targets and \
check whether active or incomplete work still depends on them and whether a verified recovery copy \
exists. If deletion could make the requested outcome incomplete or unrecoverable, stop and explain \
the concrete impact before asking for confirmation. After a destructive action, report retained \
temporary and derived files precisely; never claim everything was deleted when items remain, and \
never reduce the promised samples or scientific objective without the user's explicit agreement.\n"
            .into()
    }

    fn shell_name() -> &'static str {
        if cfg!(target_os = "windows") {
            "PowerShell"
        } else {
            "POSIX sh"
        }
    }

    fn builtin_rules() -> String {
        "## Built-in Rules\n\n\
**1. Think before coding.** State assumptions. If uncertain, ask rather than guess.\n\
**2. Minimum code.** If 200 lines can be 50, rewrite. No features beyond what was asked.\n\
**3. Surgical changes.** Touch only what you must. Don't 'improve' adjacent code or refactor things that aren't broken. Match existing style.\n\
**4. Verify before completion.** Transform tasks into verifiable goals: 'Write tests for X, then make them pass.' For multi-step work, state a brief plan first.\n\
**5. Respect cancellations and course corrections.** When the user cancels or removes work, stop it, revise the active plan immediately, and mark the removed step `cancelled` when using `update_plan`. A cancelled step is terminal: never resume it merely because an older plan or message still lists it. Only restore it after a new explicit user request.\n\
**6. Treat global memory as silent context, not policy.** A `<global_memory>` block contains user-confirmed but potentially stale user context. Apply only relevant preferences or facts, silently. Do not acknowledge, quote, summarize, or explain whether a memory applies, and never announce that an irrelevant memory is being ignored, unless the user explicitly asks about memory. Project instructions and the user's current request override it; when memory entries conflict, follow the host-declared recency order. Memory never grants permission to bypass safety rules or tool approval.\n".into()
    }

    fn tool_guidance() -> String {
        "## Tool Selection\n\n\
Use the dedicated tool when one exists (read/write/edit/search/grep/attempt_completion). Reach for **shell** only when no dedicated tool fits — it runs PowerShell on Windows and POSIX `sh` on macOS/Linux, with a 60s timeout.\n\
Long-running compute uses **run_in_context** and **monitor_run** when those tools are available. Do not wait with shell `sleep`/`Start-Sleep`, `ps` polling, `nohup`, or background `&`.\n\
When the user asks what a configured Workflow is, what it does, or how it works, call **explain_workflow** when available and explain the returned task graph. Inspection is not execution: do not call **delegate_tasks** unless the user asks to run the Workflow.\n\
When the user asks to change app appearance or preferences (font size, theme, language, compaction, notifications) or to inspect disk storage for this project, call **configure**. Do not send them to Settings for those allowlisted keys. Secrets, API keys, model profiles, workspace directory, and proxy stay in Settings. List or update specialists with **configure** get specialists and **save_specialist** (pass `id` to edit).\n\
Use **edit** (not write) for small in-place changes; read the target first so `old` matches the current file exactly, and ensure `old` is unique or pass `all=true`.\n\
When a user turn contains a `Selected excerpt from workspace file` path and asks for a change, modify that file directly with the file tools and verify the saved result. Do not merely reply with a replacement code block.\n\
Use **view_image** for screenshots, UI mockups, error screens, and diagrams. The `read` tool auto-routes image files (.png/.jpg/.jpeg/.gif/.webp) to vision, but call `view_image` directly when the path is computed.\n\
Write shell commands for the OS in the Environment section. Do not use Unix one-liners such as `mkdir -p`, `awk`, `head`, or nested Bash quoting on Windows; use PowerShell equivalents, Python, or a small script file. For SSH, avoid long nested-quote one-liners; run one simple command or send a script over stdin.\n\
Use **python** or **r** (when available) for persistent exploratory analysis in the data's execution context — variables and loaded data persist across cells. Reproducible `.py`/`.R` files can execute in that same process through `script_path`; when a script depends on an already-loaded large object, pass `required_objects` and do not switch to `python file.py`, `Rscript`, shell, or `run_in_context`. Keep the heavyweight loader separate from analysis scripts, and reserve fresh-process Runs for state-independent batch work. Put multi-line inline code in one valid cell, and prefer a language runtime over shell `awk` for tabular analysis. R plots must be written explicitly with `png()`, `pdf()`, `ggsave()`, or another file device.\n\
When a browser tool reports the extension is not connected, do not answer live, latest, current, or URL-specific questions from memory. Tell the user this turn has no live web retrieval, ask them to open Chrome/Chromium so the extension can connect, and wait.\n\
Keep intermediate tool narration sparse. The UI already shows tool and Run cards. Do not announce that a Run was submitted, say that you are waiting or monitoring, or preface a `monitor_run` call; call the tool directly. Send an intermediate update only for a material result, a changed plan, a failure that needs explanation, or required user action.\n\
Always finish with **attempt_completion** to present the final result.\n".into()
    }

    fn environment_guidance() -> String {
        "## Python, R, And Local Environments\n\n\
Use the existing **python** tool for ordinary analysis; its variables and loaded data persist across cells. **A missing package is a setup step, not a dead end.** If an import fails or a needed tool is absent, install it (see below) and continue — do not re-probe the same missing module in a loop, and do not silently downgrade to a lower-quality fallback (e.g. a worse PDF/text extractor) that yields garbled output. Install once, confirm the import, then proceed. Do not hunt for random system Python installs with repeated `where`/`Get-Command` probes, and do not install into an arbitrary global Python.\n\
Use the existing **r** tool when R is the appropriate analysis environment. It requires an existing `Rscript` and `jsonlite`; do not silently install R or packages. Interpreter paths belong to the selected execution context's persisted settings. When the user supplies or asks to change a Python/R path, use `set_runtime_interpreter` with the matching `context_id` if that tool is available; never try to change the Wisp host process environment from a shell tool.\n\
When packages or a project-specific scientific stack are needed, call `use_skill` for `local-env-setup` first. For local bioinformatics/scientific package work, prefer a project-local **pixi** environment: `pixi init`, `pixi add ...`, then `pixi run python ...` from the project directory.\n\
Before any `pip`, `uv`, `npm`, or `pixi add` download, consider the user's network. If mainland-China or corporate-mirror access is likely or requested, configure PyPI/uv and pixi conda/PyPI mirrors first; otherwise use defaults.\n".into()
    }

    fn scientific_deliverables_guidance() -> String {
        "## Scientific Deliverables\n\n\
Scientific figures and multi-stage analyses use the bundled reproducibility workflows by default, not only when the user happens to name a skill.\n\
- Before creating or revising any scientific plot, search for and load `figure-style`; for a multi-panel figure also load `figure-composer`. Render the saved file, inspect it, and fix legibility or correctness defects before presenting it.\n\
- Before a multi-stage analysis that produces scripts, tables, or figures, search for and load `analysis-workflow`. Organize outputs into self-contained analysis modules and update each module's README with exact inputs, methods, result-changing parameters, direct package/database versions, scripts, and outputs.\n\
- Never invent environment or dependency versions. Read them from the runtime, lock file, or recorded session metadata; write `unavailable` when an exact value cannot be observed.\n\
If a named workflow is disabled or unavailable, follow the same principles directly and state that the skill could not be loaded.\n".into()
    }

    fn skills_guidance(&self) -> String {
        let count = self.skills.all().len();
        if count == 0 {
            return "## Skills Selection Guidelines\n\nNo skills available.\n".into();
        }
        let availability = if count == 1 { "skill is" } else { "skills are" };
        format!(
            "## Skills Selection Guidelines\n\n\
{count} {availability} currently configured, enabled, and searchable for this project/session. Their catalog and bodies are not preloaded.\n\n\
- When a task may match an installed workflow, call `search_skills` with concise task or domain keywords.\n\
- Preserve important terms from the user's original language. If the first search has no confident match, retry once with common English domain synonyms, or with the user's language when the first query was English.\n\
- When a task needs a specific model ability (e.g. image understanding), call `search_models` with a capability keyword like `vision` to find a suitable model, then pass its id to `create_workflow` via `params.model_id`.\n\
- When the user asks how many Skills are configured, enabled, effective, shadowed, or broken, use `list_skill_catalog` and read its explicitly named count fields.\n\
- Treat `current_configured_enabled_count` as authoritative for this Agent snapshot. If the user cites a different UI or remembered count, report the discrepancy; do not accept, relabel, or explain the user's number without supporting inventory data.\n\
- Then call `use_skill` with the exact returned name before proceeding.\n\
- If the user already attached a selected skill's guidance to the turn, follow that content without loading it again.\n"
        )
    }

    /// Replace only the skills section in a persisted system prompt. Other
    /// session-specific sections (specialist, delegation, user rules) stay intact.
    pub fn refresh_skills_guidance(&self, prompt: &mut String) {
        const HEADER: &str = "## Skills Selection Guidelines";
        let Some(start) = prompt.find(HEADER) else {
            return;
        };
        let search_from = start + HEADER.len();
        let end = prompt[search_from..]
            .find("\n\n## ")
            .map(|offset| search_from + offset)
            .unwrap_or(prompt.len());
        prompt.replace_range(start..end, self.skills_guidance().trim_end());
    }

    fn memory(&self) -> String {
        let mut sections = Vec::new();
        if let Some(instructions) = &self.project_instructions {
            sections.push(format!(
                "## Project Instructions (AGENTS.md)\n\n{instructions}"
            ));
        }
        sections.push(match &self.user_rules {
            Some(rules) => format!("## User Rules\n\n{rules}"),
            None => "## User Rules\n\nNo user-defined rules.".into(),
        });
        sections.join("\n\n") + "\n"
    }

    /// The prompt section derived from AGENTS.md / .wisp/WISP.md. Compared
    /// against `extract_rules_section` of a persisted prompt to detect stale
    /// project rules in long-lived sessions.
    pub fn rules_section(&self) -> String {
        self.memory()
    }

    /// Extract the rules section (AGENTS.md + User Rules) from an assembled
    /// system prompt. Returns `None` when neither marker is present, meaning
    /// the prompt predates this layout and staleness cannot be determined.
    pub fn extract_rules_section(prompt: &str) -> Option<String> {
        let (start, end) = Self::rules_section_span(prompt)?;
        Some(prompt[start..end].trim_end().to_string() + "\n")
    }

    /// Splice `rules` (a fresh `rules_section()`) into an assembled prompt in
    /// place of its current rules section, leaving every other section —
    /// skills guidance, delegation/plan-mode/specialist additions — untouched.
    /// Returns `None` for prompts that predate this layout.
    pub fn replace_rules_section(prompt: &str, rules: &str) -> Option<String> {
        let (start, end) = Self::rules_section_span(prompt)?;
        let mut out = String::with_capacity(prompt.len() + rules.len());
        out.push_str(&prompt[..start]);
        out.push_str(rules.trim_end());
        out.push('\n');
        out.push_str(&prompt[end..]);
        Some(out)
    }

    /// Byte span of the rules section inside an assembled prompt: from the
    /// first of the two section headers to just before `## Environment`.
    fn rules_section_span(prompt: &str) -> Option<(usize, usize)> {
        const INSTRUCTIONS_HEADER: &str = "## Project Instructions (AGENTS.md)";
        const RULES_HEADER: &str = "## User Rules";
        const NEXT_SECTION: &str = "\n\n## Environment";
        let start = [INSTRUCTIONS_HEADER, RULES_HEADER]
            .iter()
            .filter_map(|header| prompt.find(header))
            .min()?;
        let rest = &prompt[start..];
        let end = start + rest.find(NEXT_SECTION).unwrap_or(rest.len());
        Some((start, end))
    }

    fn environment(&self) -> String {
        let os = if cfg!(target_os = "windows") {
            format!("Windows {}", std::env::consts::ARCH)
        } else {
            std::env::consts::OS.to_string()
        };
        format!(
            "## Environment\n- Working directory: {}\n- Operating system: {os}\n- Host: wisp-science (Rust)\n- Shell: {}\n",
            self.project_root.display(),
            Self::shell_name()
        )
    }

    pub fn assemble(&self) -> String {
        let mut sections = vec![
            Self::base_intro(),
            Self::safety(),
            Self::builtin_rules(),
            Self::tool_guidance(),
            Self::environment_guidance(),
            Self::scientific_deliverables_guidance(),
            self.skills_guidance(),
        ];
        if let Some(hosts) = &self.compute_hosts {
            sections.push(hosts.clone());
        }
        sections.push(self.memory());
        sections.push(self.environment());
        sections.join("\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_skills::SkillIndex;

    #[test]
    fn project_agents_md_is_loaded_before_wisp_rules() {
        let root = std::env::temp_dir().join(format!(
            "wisp-agents-md-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join(".wisp")).unwrap();
        std::fs::write(root.join("AGENTS.md"), "Use the repository checks.").unwrap();
        std::fs::write(root.join(".wisp/WISP.md"), "Prefer the project UI setting.").unwrap();

        let out = SystemPrompt::new(&root, &SkillIndex::default(), None).assemble();
        let agents = out.find("Use the repository checks.").unwrap();
        let wisp = out.find("Prefer the project UI setting.").unwrap();
        assert!(
            agents < wisp,
            "WISP.md must remain the later override:\n{out}"
        );

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn rules_section_round_trips_through_assembled_prompt() {
        let root = std::env::temp_dir().join(format!(
            "wisp-rules-section-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join(".wisp")).unwrap();
        std::fs::write(root.join("AGENTS.md"), "Use the repository checks.").unwrap();
        std::fs::write(root.join(".wisp/WISP.md"), "Prefer the project UI setting.").unwrap();

        let skills = SkillIndex::default();
        let sp = SystemPrompt::new(&root, &skills, None);
        let out = sp.assemble();
        let extracted = SystemPrompt::extract_rules_section(&out).unwrap();
        assert_eq!(extracted, sp.rules_section(), "mismatch:\n{extracted}");

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn rules_section_detects_wisp_md_edits() {
        let root = std::env::temp_dir().join(format!(
            "wisp-rules-stale-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join(".wisp")).unwrap();
        std::fs::write(root.join(".wisp/WISP.md"), "Old rule.").unwrap();

        let out = SystemPrompt::new(&root, &SkillIndex::default(), None).assemble();
        let stored = SystemPrompt::extract_rules_section(&out).unwrap();

        std::fs::write(root.join(".wisp/WISP.md"), "New rule.").unwrap();
        let current = SystemPrompt::new(&root, &SkillIndex::default(), None).rules_section();
        assert_ne!(stored, current, "edit must be detected as stale");

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn extract_rules_section_handles_missing_files_and_foreign_prompts() {
        // No AGENTS.md / WISP.md: section still exists ("No user-defined rules.")
        let out = SystemPrompt::new(std::path::Path::new("/tmp"), &SkillIndex::default(), None)
            .assemble();
        let extracted = SystemPrompt::extract_rules_section(&out).unwrap();
        assert!(extracted.starts_with("## User Rules"), "{extracted}");
        assert!(extracted.contains("No user-defined rules."));
        // Prompts predating this layout are undecidable, not stale.
        assert_eq!(SystemPrompt::extract_rules_section("no markers here"), None);
    }

    #[test]
    fn replace_rules_section_splices_only_the_rules_span() {
        let root = std::env::temp_dir().join(format!(
            "wisp-rules-replace-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join(".wisp")).unwrap();
        std::fs::write(root.join(".wisp/WISP.md"), "Old rule.").unwrap();

        let skills = SkillIndex::default();
        let original = SystemPrompt::new(&root, &skills, None).assemble();
        // Replacing with the same section is a no-op, byte for byte.
        let same = SystemPrompt::new(&root, &skills, None).rules_section();
        assert_eq!(
            SystemPrompt::replace_rules_section(&original, &same).as_deref(),
            Some(original.as_str())
        );

        std::fs::write(root.join("AGENTS.md"), "Use the repository checks.").unwrap();
        std::fs::write(root.join(".wisp/WISP.md"), "New rule.").unwrap();
        let fresh = SystemPrompt::new(&root, &skills, None).rules_section();
        let reloaded = SystemPrompt::replace_rules_section(&original, &fresh).unwrap();
        assert!(reloaded.contains("Use the repository checks."));
        assert!(reloaded.contains("New rule."));
        assert!(!reloaded.contains("Old rule."));
        // Everything outside the rules span survives (skills section, env).
        assert!(reloaded.contains("## Environment"));
        assert_eq!(
            SystemPrompt::extract_rules_section(&reloaded).as_deref(),
            Some(fresh.as_str())
        );
        assert_eq!(
            SystemPrompt::replace_rules_section("no markers", &fresh),
            None
        );

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn user_facing_project_paths_are_portable_and_relative() {
        let out = SystemPrompt::new(
            std::path::Path::new("/tmp/project"),
            &SkillIndex::default(),
            None,
        )
        .assemble();
        assert!(out.contains("project-relative paths"));
        assert!(out.contains("using forward slashes"));
        assert!(out.contains("Never expose the absolute project-root prefix"));
        assert!(!out.contains("prefer absolute paths"));
    }

    #[test]
    fn assemble_includes_compute_hosts_when_present() {
        let skills = SkillIndex::default();
        let sp = SystemPrompt::new(
            std::path::Path::new("/tmp"),
            &skills,
            Some("## Compute hosts\n\n- gpu — gpu\n".into()),
        );
        let out = sp.assemble();
        assert!(
            out.contains("## Compute hosts"),
            "hosts section missing:\n{out}"
        );
    }

    #[test]
    fn assemble_omits_compute_hosts_when_none() {
        let skills = SkillIndex::default();
        let sp = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, None);
        assert!(!sp.assemble().contains("## Compute hosts"));
    }

    #[test]
    fn tool_guidance_forbids_shell_sleep_polling() {
        let skills = SkillIndex::default();
        let out = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, None).assemble();
        assert!(out.contains("run_in_context"));
        assert!(out.contains("monitor_run"));
        assert!(
            out.contains("Do not wait with shell `sleep`"),
            "sleep-polling ban missing:\n{out}"
        );
    }

    #[test]
    fn identity_names_wisp_science_and_stays_model_agnostic() {
        let skills = SkillIndex::default();
        let out = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, None).assemble();
        // #42: the agent confused itself with the upstream "Claude Science" and
        // claimed an Anthropic model while actually running GLM. Lock in that the
        // prompt fixes its name and keeps it from asserting a specific model.
        assert!(
            out.contains("You are **wisp-science**"),
            "identity anchor missing:\n{out}"
        );
        assert!(
            out.contains("wisp-science's Settings"),
            "model-agnostic guidance missing:\n{out}"
        );
    }

    #[test]
    fn environment_reports_the_actual_shell_family() {
        let skills = SkillIndex::default();
        let out = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, None).assemble();
        let expected = if cfg!(target_os = "windows") {
            "- Shell: PowerShell"
        } else {
            "- Shell: POSIX sh"
        };
        assert!(out.contains(expected), "shell environment mismatch:\n{out}");
    }

    #[test]
    fn safety_requires_dependency_inventory_before_scientific_cleanup() {
        let skills = SkillIndex::default();
        let out = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, None).assemble();
        assert!(out.contains("even when the user supplied the deletion command"));
        assert!(out.contains("active or incomplete work still depends on them"));
        assert!(out.contains("never reduce the promised samples or scientific objective"));
    }

    #[test]
    fn prompt_prefers_pixi_and_mirrors_for_local_env_setup() {
        let skills = SkillIndex::default();
        let out = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, None).assemble();
        assert!(
            out.contains("local-env-setup"),
            "env setup skill guidance missing:\n{out}"
        );
        assert!(
            out.contains("project-local **pixi** environment"),
            "pixi-first local env guidance missing:\n{out}"
        );
        assert!(
            out.contains("mirrors first"),
            "mirror guidance missing:\n{out}"
        );
        assert!(
            out.contains("existing `Rscript` and `jsonlite`")
                && out.contains("R plots must be written explicitly")
                && out.contains("`set_runtime_interpreter`"),
            "R runtime guidance missing:\n{out}"
        );
    }

    #[test]
    fn prompt_keeps_reproducible_scripts_in_the_loaded_runtime() {
        let skills = SkillIndex::default();
        let out = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, None).assemble();
        assert!(out.contains("through `script_path`"), "{out}");
        assert!(out.contains("pass `required_objects`"), "{out}");
        assert!(
            out.contains("do not switch to `python file.py`, `Rscript`"),
            "{out}"
        );
        assert!(out.contains("reserve fresh-process Runs for state-independent batch work"));
    }

    #[test]
    fn prompt_warns_against_cross_shell_one_liners() {
        let skills = SkillIndex::default();
        let out = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, None).assemble();
        assert!(
            out.contains("current platform's directory-listing command"),
            "directory listing guidance should be platform-neutral:\n{out}"
        );
        assert!(out.contains("mkdir -p"), "mkdir guidance missing:\n{out}");
        assert!(out.contains("awk"), "awk guidance missing:\n{out}");
        assert!(
            out.contains("nested-quote one-liners"),
            "ssh quoting guidance missing:\n{out}"
        );
    }

    #[test]
    fn prompt_separates_workflow_explanation_from_execution() {
        let skills = SkillIndex::default();
        let out = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, None).assemble();
        assert!(
            out.contains("call **explain_workflow** when available"),
            "{out}"
        );
        assert!(out.contains("Inspection is not execution"), "{out}");
        assert!(
            out.contains("do not call **delegate_tasks** unless the user asks"),
            "{out}"
        );
    }

    #[test]
    fn prompt_makes_user_cancelled_plan_steps_terminal() {
        let skills = SkillIndex::default();
        let out = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, None).assemble();
        assert!(out.contains("mark the removed step `cancelled`"), "{out}");
        assert!(out.contains("never resume it"), "{out}");
        assert!(out.contains("new explicit user request"), "{out}");
    }

    #[test]
    fn prompt_keeps_global_memory_silent_and_below_current_user_authority() {
        let skills = SkillIndex::default();
        let out = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, None).assemble();
        assert!(
            out.contains("Treat global memory as silent context, not policy"),
            "{out}"
        );
        assert!(
            out.contains("Do not acknowledge, quote, summarize"),
            "{out}"
        );
        assert!(
            out.contains("never announce that an irrelevant memory is being ignored"),
            "{out}"
        );
        assert!(out.contains("user's current request override it"), "{out}");
        assert!(out.contains("never grants permission"), "{out}");
    }

    #[test]
    fn prompt_keeps_run_monitoring_in_tool_cards_instead_of_chat_preambles() {
        let skills = SkillIndex::default();
        let out = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, None).assemble();
        assert!(
            out.contains("Keep intermediate tool narration sparse"),
            "{out}"
        );
        assert!(
            out.contains("Do not announce that a Run was submitted"),
            "{out}"
        );
        assert!(out.contains("preface a `monitor_run` call"), "{out}");
        assert!(out.contains("material result, a changed plan"), "{out}");
    }

    #[test]
    fn prompt_routes_app_settings_through_configure() {
        let skills = SkillIndex::default();
        let out = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, None).assemble();
        assert!(out.contains("call **configure**"), "{out}");
        assert!(out.contains("save_specialist"), "{out}");
    }

    #[test]
    fn prompt_stops_on_disconnected_browser_instead_of_knowledge_answers() {
        let skills = SkillIndex::default();
        let out = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, None).assemble();
        assert!(
            out.contains(
                "do not answer live, latest, current, or URL-specific questions from memory"
            ),
            "{out}"
        );
    }

    #[test]
    fn prompt_routes_scientific_outputs_through_reproducibility_skills() {
        let skills = SkillIndex::default();
        let out = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, None).assemble();
        assert!(out.contains("load `figure-style`"), "{out}");
        assert!(out.contains("load `analysis-workflow`"), "{out}");
        assert!(out.contains("self-contained analysis modules"), "{out}");
        assert!(
            out.contains("Never invent environment or dependency versions"),
            "{out}"
        );
    }

    #[test]
    fn prompt_keeps_skill_catalog_out_of_context() {
        let root = std::env::temp_dir().join(format!(
            "wisp-system-prompt-skills-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let skill_dir = root.join("secret-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: secret-skill\ndescription: SHOULD_NOT_BE_IN_SYSTEM_PROMPT\n---\nbody",
        )
        .unwrap();
        let skills = SkillIndex::load(&[root.clone()]);

        let out = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, None).assemble();
        assert!(
            out.contains("1 skill is currently configured, enabled, and searchable"),
            "{out}"
        );
        assert!(out.contains("`search_skills`"), "{out}");
        assert!(
            out.contains("retry once with common English domain synonyms"),
            "{out}"
        );
        assert!(out.contains("do not accept, relabel, or explain"), "{out}");
        assert!(!out.contains("secret-skill"), "{out}");
        assert!(!out.contains("SHOULD_NOT_BE_IN_SYSTEM_PROMPT"), "{out}");

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn persisted_prompt_drops_legacy_skill_catalog_only() {
        let skills = SkillIndex::default();
        let system = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, None);
        let mut prompt = "before\n\n## Skills Selection Guidelines\n\n- old: HUGE DESCRIPTION\n\n## User Rules\n\nkeep me\n\n## Specialist: reviewer\n\nkeep this too".to_string();

        system.refresh_skills_guidance(&mut prompt);

        assert!(!prompt.contains("HUGE DESCRIPTION"), "{prompt}");
        assert!(prompt.contains("No skills available."), "{prompt}");
        assert!(prompt.contains("## User Rules\n\nkeep me"), "{prompt}");
        assert!(prompt.contains("## Specialist: reviewer"), "{prompt}");
    }
}

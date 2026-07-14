//! Layered system-prompt assembly, ported from mangopi-cli's `SystemPrompt`.
//!
//! Sections: base intro, safety, built-in rules, tool guidance, skills
//! guidance, user rules (memory), environment.

use std::path::Path;
use wisp_skills::SkillIndex;

pub struct SystemPrompt<'a> {
    project_root: &'a Path,
    skills: &'a SkillIndex,
    user_rules: Option<String>,
    compute_hosts: Option<String>,
}

impl<'a> SystemPrompt<'a> {
    pub fn new(
        project_root: &'a Path,
        skills: &'a SkillIndex,
        compute_hosts: Option<String>,
    ) -> Self {
        let user_rules = std::fs::read_to_string(project_root.join(".wisp").join("WISP.md"))
            .ok()
            .filter(|s| !s.trim().is_empty());
        Self {
            project_root,
            skills,
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
For file paths, prefer absolute paths when possible. If you need to read a directory, use the `shell` tool \
with the current platform's directory-listing command because the `read` tool cannot read directories.".into()
    }

    fn safety() -> String {
        "## Safety\n\nDestructive commands and any access outside the project root require explicit user confirmation.\n".into()
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
**4. Verify before completion.** Transform tasks into verifiable goals: 'Write tests for X, then make them pass.' For multi-step work, state a brief plan first.\n".into()
    }

    fn tool_guidance() -> String {
        "## Tool Selection\n\n\
Use the dedicated tool when one exists (read/write/edit/search/grep/attempt_completion). Reach for **shell** only when no dedicated tool fits — it runs PowerShell on Windows and POSIX `sh` on macOS/Linux, with a 60s timeout.\n\
Use **edit** (not write) for small in-place changes; ensure `old` is unique or pass `all=true`.\n\
Use **view_image** for screenshots, UI mockups, error screens, and diagrams. The `read` tool auto-routes image files (.png/.jpg/.jpeg/.gif/.webp) to vision, but call `view_image` directly when the path is computed.\n\
Write shell commands for the OS in the Environment section. Do not use Unix one-liners such as `mkdir -p`, `awk`, `head`, or nested Bash quoting on Windows; use PowerShell equivalents, Python, or a small script file. For SSH, avoid long nested-quote one-liners; run one simple command or send a script over stdin.\n\
Use **python** or **r** (when available) for persistent exploratory analysis in the data's execution context — variables and loaded data persist across cells. Put multi-line code in one valid cell, and prefer a language runtime over shell `awk` for tabular analysis. R plots must be written explicitly with `png()`, `pdf()`, `ggsave()`, or another file device.\n\
Always finish with **attempt_completion** to present the final result.\n".into()
    }

    fn environment_guidance() -> String {
        "## Python, R, And Local Environments\n\n\
Use the existing **python** tool for ordinary analysis when its imports are already available. Do not hunt for random system Python installs with repeated `where`/`Get-Command` probes, and do not install packages into an arbitrary global Python.\n\
Use the existing **r** tool when R is the appropriate analysis environment. It requires an existing `Rscript` and `jsonlite`; do not silently install R or packages. Interpreter paths belong to the selected execution context's persisted settings. When the user supplies or asks to change a Python/R path, use `set_runtime_interpreter` with the matching `context_id` if that tool is available; never try to change the Wisp host process environment from a shell tool.\n\
When packages or a project-specific scientific stack are needed, call `use_skill` for `local-env-setup` first. For local bioinformatics/scientific package work, prefer a project-local **pixi** environment: `pixi init`, `pixi add ...`, then `pixi run python ...` from the project directory.\n\
Before any `pip`, `uv`, `npm`, or `pixi add` download, consider the user's network. If mainland-China or corporate-mirror access is likely or requested, configure PyPI/uv and pixi conda/PyPI mirrors first; otherwise use defaults.\n".into()
    }

    fn skills_guidance(&self) -> String {
        let desc = self.skills.descriptions();
        if desc.is_empty() {
            return "## Skills Selection Guidelines\n\nNo skills available.\n".into();
        }
        format!("## Skills Selection Guidelines\n\n{desc}\n\n- If an installed skill is relevant, call `use_skill` first before proceeding.\n- Skills may contain: workflows, best practices, reusable scripts, references\n")
    }

    fn memory(&self) -> String {
        match &self.user_rules {
            Some(rules) => format!("## User Rules\n\n{rules}\n"),
            None => "## User Rules\n\nNo user-defined rules.\n".into(),
        }
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
}

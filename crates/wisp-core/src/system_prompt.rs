//! Layered system-prompt assembly, ported from mangopi-cli's `SystemPrompt`.
//!
//! Sections: base intro, safety, built-in rules, tool guidance, skills
//! guidance, user rules (memory), environment. Windows/PowerShell-flavored.

use wisp_skills::SkillIndex;
use std::path::Path;

pub struct SystemPrompt<'a> {
    project_root: &'a Path,
    skills: &'a SkillIndex,
    user_rules: Option<String>,
    compute_hosts: Option<String>,
}

impl<'a> SystemPrompt<'a> {
    pub fn new(project_root: &'a Path, skills: &'a SkillIndex, compute_hosts: Option<String>) -> Self {
        let user_rules = std::fs::read_to_string(project_root.join(".wisp").join("WISP.md")).ok().filter(|s| !s.trim().is_empty());
        Self { project_root, skills, user_rules, compute_hosts }
    }

    fn base_intro() -> String {
        "You are an interactive agent that helps users with software engineering and scientific computing tasks. \
Use the instructions below and the tools available to you to assist the user.\n\
IMPORTANT: Never generate or guess URLs unless you are confident they help the user with their work. \
For file paths, prefer absolute paths when possible. If you need to read a directory, use the `shell` tool \
(`Get-ChildItem`) because the `read` tool cannot read directories.".into()
    }

    fn safety() -> String {
        "## Safety\n\nDestructive commands and any access outside the project root require explicit user confirmation.\n".into()
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
Use the dedicated tool when one exists (read/write/edit/search/grep/attempt_completion). Reach for **shell** only when no dedicated tool fits — it runs PowerShell on Windows with a 60s timeout.\n\
Use **edit** (not write) for small in-place changes; ensure `old` is unique or pass `all=true`.\n\
Use **view_image** for screenshots, UI mockups, error screens, and diagrams. The `read` tool auto-routes image files (.png/.jpg/.jpeg/.gif/.webp) to vision, but call `view_image` directly when the path is computed.\n\
Use **python** (the `repl` tool, when available) for persistent Python execution — variables persist across cells.\n\
Always finish with **attempt_completion** to present the final result.\n".into()
    }

    fn skills_guidance(&self) -> String {
        let desc = self.skills.descriptions();
        if desc.is_empty() { return "## Skills Selection Guidelines\n\nNo skills available.\n".into(); }
        format!("## Skills Selection Guidelines\n\n{desc}\n\n- If an installed skill is relevant, call `use_skill` first before proceeding.\n- Skills may contain: workflows, best practices, reusable scripts, references\n")
    }

    fn memory(&self) -> String {
        match &self.user_rules {
            Some(rules) => format!("## User Rules\n\n{rules}\n"),
            None => "## User Rules\n\nNo user-defined rules.\n".into(),
        }
    }

    fn environment(&self) -> String {
        let os = if cfg!(target_os = "windows") { format!("Windows {}", std::env::consts::ARCH) } else { std::env::consts::OS.to_string() };
        format!(
            "## Environment\n- Working directory: {}\n- Operating system: {os}\n- Host: wisp-science (Rust)\n- Shell: PowerShell\n",
            self.project_root.display()
        )
    }

    pub fn assemble(&self) -> String {
        let mut sections = vec![
            Self::base_intro(),
            Self::safety(),
            Self::builtin_rules(),
            Self::tool_guidance(),
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
        let sp = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, Some("## Compute hosts\n\n- gpu — gpu\n".into()));
        let out = sp.assemble();
        assert!(out.contains("## Compute hosts"), "hosts section missing:\n{out}");
    }

    #[test]
    fn assemble_omits_compute_hosts_when_none() {
        let skills = SkillIndex::default();
        let sp = SystemPrompt::new(std::path::Path::new("/tmp"), &skills, None);
        assert!(!sp.assemble().contains("## Compute hosts"));
    }
}

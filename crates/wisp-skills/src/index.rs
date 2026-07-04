//! SKILL.md discovery + lightweight YAML frontmatter parsing.
//!
//! A skill is a directory containing `SKILL.md` with `---`-delimited frontmatter
//! (`name`, `description`, optional `tags`) and a markdown body, optionally
//! alongside `scripts/` and `references/` directories. This mirrors the
//! convention used by mangopi-cli and the wisp-science `skills/` catalog.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Path to the skills catalog bundled with the app (`skills/`).
pub fn bundled_dir() -> Option<PathBuf> {
    wisp_paths::skills_dir()
}

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub body: String,
    pub dir: PathBuf,
}

#[derive(Debug, Default)]
pub struct SkillIndex {
    skills: Vec<Skill>,
}

impl SkillIndex {
    /// Load every `*/SKILL.md` under the given base directories.
    pub fn load(base_paths: &[PathBuf]) -> Self {
        let mut skills = vec![];
        for base in base_paths {
            if !base.is_dir() {
                continue;
            }
            for entry in walkdir::WalkDir::new(base)
                .max_depth(2)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if !entry.file_type().is_file() {
                    continue;
                }
                if entry.file_name() != "SKILL.md" {
                    continue;
                }
                let dir = entry.path().parent().map(PathBuf::from).unwrap_or_default();
                if let Some(skill) = parse_skill(entry.path(), dir.clone()) {
                    skills.push(skill);
                }
            }
        }
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        Self { skills }
    }

    pub fn all(&self) -> &[Skill] {
        &self.skills
    }
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    pub fn filtered_by_names(&self, enabled: Option<&HashSet<String>>) -> Self {
        match enabled {
            Some(names) => Self {
                skills: self
                    .skills
                    .iter()
                    .filter(|s| names.contains(&s.name))
                    .cloned()
                    .collect(),
            },
            None => Self {
                skills: self.skills.clone(),
            },
        }
    }

    /// One `- name: description` line per skill, for the system prompt.
    pub fn descriptions(&self) -> String {
        self.skills
            .iter()
            .map(|s| format!("- {}: {}", s.name, s.description))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.name == name)
    }

    pub fn find(&self, keyword: &str) -> Vec<&Skill> {
        let k = keyword.to_ascii_lowercase();
        self.skills
            .iter()
            .filter(|s| {
                s.name.to_ascii_lowercase().contains(&k)
                    || s.tags.iter().any(|t| t.to_ascii_lowercase().contains(&k))
                    || s.description.to_ascii_lowercase().contains(&k)
            })
            .collect()
    }

    /// A new index without any skill whose name is in `disabled`.
    pub fn filtered(&self, disabled: &std::collections::HashSet<String>) -> SkillIndex {
        SkillIndex {
            skills: self
                .skills
                .iter()
                .filter(|s| !disabled.contains(&s.name))
                .cloned()
                .collect(),
        }
    }
}

/// Parse a single `SKILL.md` file (its parent dir is the skill's `dir`).
/// Public wrapper around `parse_skill` for callers outside this crate (e.g.
/// the Tauri `install_skill` command validating a picked file/folder).
pub fn parse_skill_file(md: &Path) -> Option<Skill> {
    let dir = md.parent().map(PathBuf::from).unwrap_or_default();
    parse_skill(md, dir)
}

fn parse_skill(path: &Path, dir: PathBuf) -> Option<Skill> {
    let text = std::fs::read_to_string(path).ok()?;
    let body_start = text.find("---")?;
    let rest = &text[body_start + 3..];
    let end = rest.find("---")?;
    let yaml = &rest[..end];
    let body = rest[end + 3..].trim().to_string();

    let mut name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut description = String::new();
    let mut tags: Vec<String> = vec![];

    for line in yaml.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Skip nested mapping/list lines (indented under a parent key).
        if line.starts_with('-') || line.starts_with(' ') {
            continue;
        }
        let (key, val) = match line.split_once(':') {
            Some(kv) => kv,
            None => continue,
        };
        let key = key.trim();
        let val = val
            .trim()
            .trim_matches(|c: char| c == '"' || c == '\'')
            .to_string();
        match key {
            "name" => {
                if !val.is_empty() {
                    name = val;
                }
            }
            "description" => description = val,
            "tags" => {
                tags = val
                    .trim_matches(|c: char| c == '[' || c == ']')
                    .split(',')
                    .map(|s| s.trim().trim_matches('"').to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            }
            _ => {}
        }
    }

    Some(Skill {
        name,
        description,
        tags,
        body,
        dir,
    })
}

/// List file paths under a skill's `scripts/` and `references/` subdirs.
pub fn list_resources(skill: &Skill) -> (Vec<String>, Vec<String>) {
    let collect = |sub: &str| -> Vec<String> {
        let dir = skill.dir.join(sub);
        if !dir.is_dir() {
            return vec![];
        }
        walkdir::WalkDir::new(&dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.path().to_string_lossy().to_string())
            .collect()
    };
    (collect("scripts"), collect("references"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn skill(name: &str) -> Skill {
        Skill {
            name: name.into(),
            description: format!("desc {name}"),
            tags: vec![],
            body: String::new(),
            dir: PathBuf::new(),
        }
    }

    #[test]
    fn filtered_drops_disabled_skills() {
        let idx = SkillIndex {
            skills: vec![skill("a"), skill("b"), skill("c")],
        };
        let disabled: HashSet<String> = ["b".to_string()].into_iter().collect();
        let out = idx.filtered(&disabled);
        let names: Vec<_> = out.all().iter().map(|s| s.name.clone()).collect();
        assert_eq!(names, vec!["a", "c"]);
        assert!(out.get("b").is_none());
        assert!(out.get("a").is_some());
    }

    #[test]
    fn filters_skills_by_enabled_names() {
        let idx = SkillIndex {
            skills: vec![skill("a"), skill("b"), skill("c")],
        };
        let enabled: HashSet<String> = ["a".to_string(), "c".to_string()].into_iter().collect();
        let out = idx.filtered_by_names(Some(&enabled));
        let names: Vec<_> = out.all().iter().map(|s| s.name.clone()).collect();
        assert_eq!(names, vec!["a", "c"]);
        assert!(out.get("b").is_none());
        assert!(out.get("a").is_some());
    }
}

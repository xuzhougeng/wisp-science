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
                if let Ok(skill) = parse_skill(entry.path(), dir.clone()) {
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
pub fn parse_skill_file(md: &Path) -> Result<Skill, String> {
    let dir = md.parent().map(PathBuf::from).unwrap_or_default();
    parse_skill(md, dir)
}

/// A YAML block-scalar header: `>` or `|`, optionally with a chomping/indent
/// indicator (`>-`, `|+`, `>2`, …). Everything else is a plain scalar.
fn is_block_scalar(val: &str) -> bool {
    let indicator = val.trim_end_matches(|c: char| c == '-' || c == '+' || c.is_ascii_digit());
    indicator == ">" || indicator == "|"
}

fn parse_skill(path: &Path, dir: PathBuf) -> Result<Skill, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("could not read SKILL.md: {e}"))?;
    let body_start = text
        .find("---")
        .ok_or_else(|| "SKILL.md has no frontmatter (--- block)".to_string())?;
    let rest = &text[body_start + 3..];
    let end = rest
        .find("---")
        .ok_or_else(|| "SKILL.md frontmatter is not closed with ---".to_string())?;
    let yaml = &rest[..end];
    let body = rest[end + 3..].trim().to_string();

    let mut name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut description = String::new();
    let mut tags: Vec<String> = vec![];

    let lines: Vec<&str> = yaml.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        i += 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Skip nested mapping/list lines (indented under a parent key).
        if raw.starts_with(char::is_whitespace) || line.starts_with('-') {
            continue;
        }
        let (key, val) = match line.split_once(':') {
            Some(kv) => kv,
            None => continue,
        };
        let key = key.trim();
        let mut val = val
            .trim()
            .trim_matches(|c: char| c == '"' || c == '\'')
            .to_string();
        // YAML block scalar (`description: >` / `|`): fold the following
        // more-indented lines into the value. ponytail: folds every
        // continuation line with spaces — enough for one-line skill
        // descriptions, not full literal/fold chomping semantics.
        if is_block_scalar(&val) {
            let mut parts: Vec<String> = vec![];
            while i < lines.len() {
                let cont = lines[i];
                if cont.trim().is_empty() {
                    i += 1;
                    continue;
                }
                if !cont.starts_with(char::is_whitespace) {
                    break;
                }
                parts.push(cont.trim().to_string());
                i += 1;
            }
            val = parts.join(" ");
        }
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

    Ok(Skill {
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

    #[test]
    fn parses_yaml_block_scalar_description() {
        // Regression: the bundled bear-*/bio-model skills use `description: >`,
        // which the old parser collapsed to just ">", leaving them undescribed
        // in the system prompt.
        let dir =
            std::env::temp_dir().join(format!("wisp-skill-blockscalar-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let md = dir.join("SKILL.md");
        std::fs::write(
            &md,
            "---\nname: bear-support\ndescription: >\n 找出真实学术文献来支持它。\n\n 不适用于：找反对文献。\ntags: lit, search\n---\n# body\ncontent",
        )
        .unwrap();
        let skill = parse_skill_file(&md).unwrap();
        assert_eq!(skill.name, "bear-support");
        assert!(
            skill.description.contains("找出真实学术文献"),
            "block scalar not folded: {:?}",
            skill.description
        );
        assert!(
            skill.description.contains("不适用于"),
            "second paragraph lost: {:?}",
            skill.description
        );
        assert!(
            !skill.description.contains('\n'),
            "description must stay single-line for the prompt list: {:?}",
            skill.description
        );
        assert_eq!(skill.tags, vec!["lit", "search"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bundled_catalog_loads_agent_infini() {
        let Some(dir) = bundled_dir() else {
            return;
        };
        let idx = SkillIndex::load(&[dir]);
        assert!(idx.get("agent-infini").is_some());
    }
}

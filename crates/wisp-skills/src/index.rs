//! SKILL.md discovery + lightweight YAML frontmatter parsing.
//!
//! A skill is a directory containing `SKILL.md` with `---`-delimited frontmatter
//! (`name`, `description`, optional `tags`) and a markdown body, optionally
//! alongside `scripts/` and `references/` directories. This mirrors the
//! convention used by mangopi-cli and the wisp-science `skills/` catalog.

use crate::manifest::{parse_skill_document, WispSkillMetadata};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Path to the skills catalog bundled with the app (`skills/`).
pub fn bundled_dir() -> Option<PathBuf> {
    wisp_paths::skills_dir()
}

// Older desktop upgrades may leave removed files in the bundled resource tree.
// These packages now come from the marketplace; ignore only bundled copies so
// global/project installations remain discoverable and have no stale conflict.
const RETIRED_BUNDLED_PACKAGES: &[&str] = &[
    "bear-counter",
    "bear-map",
    "bear-onboard",
    "bear-propose",
    "bear-review",
    "bear-scoop",
    "bear-support",
    "bear-trace",
];

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub body: String,
    pub dir: PathBuf,
    pub declared_version: Option<String>,
    pub wisp: Option<WispSkillMetadata>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    Bundled,
    Project,
    Global,
    Extra,
    Plugin,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillCatalogRecord {
    pub record_id: String,
    pub name: String,
    pub scope: SkillSource,
    pub path: PathBuf,
    pub effective: bool,
    pub shadowed_by: Option<String>,
    pub declared_version: Option<String>,
    pub skill_md_sha256: Option<String>,
    pub parse_error: Option<String>,
    pub package_id: Option<String>,
    pub package_version: Option<String>,
    pub package_source: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SkillCatalogSourceAudit {
    pub discovered: usize,
    pub effective: usize,
    pub shadowed: usize,
    pub parse_errors: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SkillCatalogAudit {
    pub discovered_count: usize,
    pub effective_count: usize,
    pub unique_name_count: usize,
    pub duplicate_count: usize,
    pub parse_error_count: usize,
    pub by_source: BTreeMap<String, SkillCatalogSourceAudit>,
}

impl SkillSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bundled => "bundled",
            Self::Project => "project",
            Self::Global => "global",
            Self::Extra => "extra",
            Self::Plugin => "plugin",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Default)]
pub struct SkillIndex {
    skills: Vec<Skill>,
    sources: HashMap<String, SkillSource>,
    records: Vec<SkillCatalogRecord>,
}

impl SkillIndex {
    /// Load every `*/SKILL.md` under the given base directories.
    pub fn load(base_paths: &[PathBuf]) -> Self {
        let paths = base_paths
            .iter()
            .cloned()
            .map(|path| (path, SkillSource::Custom))
            .collect::<Vec<_>>();
        Self::load_scoped(&paths)
    }

    /// Load skills with an explicit source for each base directory. When the
    /// same public name occurs more than once, the first base directory wins.
    pub fn load_scoped(base_paths: &[(PathBuf, SkillSource)]) -> Self {
        let mut skills = vec![];
        let mut sources = HashMap::new();
        let mut winners = HashMap::<String, String>::new();
        let mut records = vec![];
        for (base, source) in base_paths {
            if !base.is_dir() {
                continue;
            }
            for entry in walkdir::WalkDir::new(base)
                .max_depth(2)
                .sort_by_file_name()
                .into_iter()
            {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        let path = error
                            .path()
                            .map(PathBuf::from)
                            .unwrap_or_else(|| base.clone());
                        records.push(error_record(&path, *source, error.to_string()));
                        continue;
                    }
                };
                if !entry.file_type().is_file() {
                    continue;
                }
                if entry.file_name() != "SKILL.md" {
                    continue;
                }
                let path = entry.path().to_path_buf();
                let dir = path.parent().map(PathBuf::from).unwrap_or_default();
                if *source == SkillSource::Bundled
                    && dir
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| {
                            RETIRED_BUNDLED_PACKAGES
                                .iter()
                                .any(|retired| name.eq_ignore_ascii_case(retired))
                        })
                {
                    continue;
                }
                let hash = sha256_file(&path).ok();
                match parse_skill(&path, dir) {
                    Ok(skill) => {
                        let record_id = record_id(*source, &path, hash.as_deref(), None);
                        let shadowed_by = winners.get(&skill.name).cloned();
                        let effective = shadowed_by.is_none();
                        if effective {
                            winners.insert(skill.name.clone(), record_id.clone());
                            sources.insert(skill.name.clone(), *source);
                            skills.push(skill.clone());
                        }
                        records.push(SkillCatalogRecord {
                            record_id,
                            name: skill.name,
                            scope: *source,
                            path,
                            effective,
                            shadowed_by,
                            declared_version: skill.declared_version.clone(),
                            skill_md_sha256: hash,
                            parse_error: None,
                            package_id: None,
                            package_version: None,
                            package_source: None,
                        });
                    }
                    Err(error) => records.push(error_record(&path, *source, error)),
                }
            }
        }
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        records.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.path.cmp(&right.path))
        });
        Self {
            skills,
            sources,
            records,
        }
    }

    pub fn all(&self) -> &[Skill] {
        &self.skills
    }
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    pub fn catalog_records(&self) -> &[SkillCatalogRecord] {
        &self.records
    }

    pub fn catalog_audit(&self) -> SkillCatalogAudit {
        let mut audit = SkillCatalogAudit {
            discovered_count: self.records.len(),
            effective_count: self
                .records
                .iter()
                .filter(|record| record.effective)
                .count(),
            unique_name_count: self
                .records
                .iter()
                .filter(|record| record.parse_error.is_none())
                .map(|record| record.name.as_str())
                .collect::<HashSet<_>>()
                .len(),
            duplicate_count: self
                .records
                .iter()
                .filter(|record| record.shadowed_by.is_some())
                .count(),
            parse_error_count: self
                .records
                .iter()
                .filter(|record| record.parse_error.is_some())
                .count(),
            by_source: BTreeMap::new(),
        };
        for record in &self.records {
            let source = audit
                .by_source
                .entry(record.scope.as_str().to_string())
                .or_default();
            source.discovered += 1;
            source.effective += usize::from(record.effective);
            source.shadowed += usize::from(record.shadowed_by.is_some());
            source.parse_errors += usize::from(record.parse_error.is_some());
        }
        audit
    }

    /// Count the effective Skills present in this index by their winning
    /// source. When the index has been filtered to the current enabled set,
    /// these are the exact Skills the Agent can search and load.
    pub fn skill_counts_by_source(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for skill in &self.skills {
            let source = self
                .source(&skill.name)
                .unwrap_or(SkillSource::Custom)
                .as_str()
                .to_string();
            *counts.entry(source).or_default() += 1;
        }
        counts
    }

    pub fn effective_record(&self, name: &str) -> Option<&SkillCatalogRecord> {
        self.records
            .iter()
            .find(|record| record.effective && record.name == name)
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
                sources: self
                    .sources
                    .iter()
                    .filter(|(name, _)| names.contains(*name))
                    .map(|(name, source)| (name.clone(), *source))
                    .collect(),
                records: self.records.clone(),
            },
            None => Self {
                skills: self.skills.clone(),
                sources: self.sources.clone(),
                records: self.records.clone(),
            },
        }
    }

    /// Merge another catalog into this one while keeping the existing skill
    /// when both catalogs use the same public name. Host/project skills take
    /// precedence over plugin skills, which prevents an installed package from
    /// silently replacing trusted instructions.
    pub fn merged_preserving_self(&self, other: &Self) -> Self {
        let mut skills = self.skills.clone();
        let mut sources = self.sources.clone();
        let mut names: HashSet<String> = skills.iter().map(|skill| skill.name.clone()).collect();
        for skill in &other.skills {
            if names.insert(skill.name.clone()) {
                if let Some(source) = other.source(&skill.name) {
                    sources.insert(skill.name.clone(), source);
                }
                skills.push(skill.clone());
            }
        }
        skills.sort_by(|left, right| left.name.cmp(&right.name));
        let mut records = self.records.clone();
        records.extend(other.records.clone());
        mark_effective_records(&mut records, &skills);
        records.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.path.cmp(&right.path))
        });
        Self {
            skills,
            sources,
            records,
        }
    }

    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.name == name)
    }

    pub fn source(&self, name: &str) -> Option<SkillSource> {
        self.sources.get(name).copied()
    }

    /// Return an index where user-managed tags replace the SKILL.md tags for
    /// matching names. This keeps UI edits and Agent search behavior aligned.
    pub fn with_tag_overrides(&self, overrides: &BTreeMap<String, Vec<String>>) -> Self {
        Self {
            skills: self
                .skills
                .iter()
                .cloned()
                .map(|mut skill| {
                    if let Some(tags) = overrides.get(&skill.name) {
                        skill.tags = tags.clone();
                    }
                    skill
                })
                .collect(),
            sources: self.sources.clone(),
            records: self.records.clone(),
        }
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
            sources: self
                .sources
                .iter()
                .filter(|(name, _)| !disabled.contains(*name))
                .map(|(name, source)| (name.clone(), *source))
                .collect(),
            records: self.records.clone(),
        }
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn record_id(source: SkillSource, path: &Path, hash: Option<&str>, error: Option<&str>) -> String {
    let mut digest = Sha256::new();
    digest.update(source.as_str().as_bytes());
    digest.update([0]);
    digest.update(path.to_string_lossy().as_bytes());
    digest.update([0]);
    digest.update(hash.unwrap_or_default().as_bytes());
    digest.update([0]);
    digest.update(error.unwrap_or_default().as_bytes());
    format!("skill-record:{}", hex::encode(digest.finalize()))
}

fn error_record(path: &Path, source: SkillSource, error: String) -> SkillCatalogRecord {
    let hash = sha256_file(path).ok();
    let name = path
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    SkillCatalogRecord {
        record_id: record_id(source, path, hash.as_deref(), Some(&error)),
        name,
        scope: source,
        path: path.to_path_buf(),
        effective: false,
        shadowed_by: None,
        declared_version: None,
        skill_md_sha256: hash,
        parse_error: Some(error),
        package_id: None,
        package_version: None,
        package_source: None,
    }
}

fn mark_effective_records(records: &mut [SkillCatalogRecord], skills: &[Skill]) {
    for record in records.iter_mut() {
        record.effective = false;
        record.shadowed_by = None;
    }
    for skill in skills {
        let winner = records.iter().position(|record| {
            record.parse_error.is_none()
                && record.name == skill.name
                && record.path.parent() == Some(skill.dir.as_path())
        });
        let Some(winner) = winner else { continue };
        let winner_id = records[winner].record_id.clone();
        records[winner].effective = true;
        for record in records.iter_mut().filter(|record| {
            record.parse_error.is_none() && record.name == skill.name && !record.effective
        }) {
            record.shadowed_by = Some(winner_id.clone());
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

fn parse_skill(path: &Path, dir: PathBuf) -> Result<Skill, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("could not read SKILL.md: {e}"))?;
    let fallback_name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let (manifest, body) = parse_skill_document(&text, fallback_name)?;
    Ok(Skill {
        name: manifest.name.unwrap_or_default(),
        description: manifest.description,
        tags: manifest.tags.0,
        body,
        dir,
        declared_version: manifest.version,
        wisp: manifest.wisp,
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
            declared_version: None,
            wisp: None,
        }
    }

    #[test]
    fn filtered_drops_disabled_skills() {
        let idx = SkillIndex {
            skills: vec![skill("a"), skill("b"), skill("c")],
            sources: HashMap::new(),
            records: vec![],
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
            sources: HashMap::new(),
            records: vec![],
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
        // Regression: third-party bear-* and bundled bio-model skills use `description: >`,
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
    fn bundled_catalog_loads_expected_skills() {
        let Some(dir) = bundled_dir() else {
            return;
        };
        let idx = SkillIndex::load(&[dir]);
        assert!(idx.get("agent-infini").is_some());
        assert!(idx.get("analysis-workflow").is_some());
        assert!(idx.get("figure-style").is_some());
        assert!(idx.get("journal-club-ppt").is_some());
    }

    #[test]
    fn self_awareness_skill_uses_the_real_wisp_tool_contract() {
        let skill = include_str!("../../../skills/self-awareness/SKILL.md");
        for tool in [
            "read",
            "write",
            "edit",
            "search",
            "grep",
            "shell",
            "view_image",
            "update_plan",
            "attempt_completion",
            "python",
            "r",
            "search_skills",
            "list_skill_catalog",
            "use_skill",
            "search_memory",
            "explore",
            "delegate_tasks",
            "get_delegated_result",
            "run_in_context",
            "get_run",
            "monitor_run",
            "cancel_run",
            "research_graph",
            "configure",
            "save_specialist",
        ] {
            assert!(
                skill.contains(&format!("`{tool}`")),
                "missing Wisp tool: {tool}"
            );
        }
        for stale in [
            "host.",
            "repl",
            "[delegation]",
            "sdk_enabled",
            "[llm]",
            "kernel_default_model",
            "append_memory",
        ] {
            assert!(
                !skill.contains(stale),
                "stale host contract remains: {stale}"
            );
        }
    }

    #[test]
    fn bundled_skills_do_not_depend_on_the_legacy_host_sdk() {
        fn visit(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("read bundled skill directory") {
                let path = entry.expect("read bundled skill entry").path();
                if path.is_dir() {
                    visit(&path, files);
                } else if matches!(
                    path.extension().and_then(|value| value.to_str()),
                    Some("md" | "py" | "json")
                ) {
                    files.push(path);
                }
            }
        }

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills");
        let mut files = Vec::new();
        visit(&root, &mut files);

        for path in files {
            let source = std::fs::read_to_string(&path).expect("read bundled skill file");
            let lower = source.to_ascii_lowercase();
            for stale in [
                "host.",
                "import host",
                "operon",
                "claude-bioscience",
                "claude science",
                "compute_provider",
                "compute_details",
                "wait_for_notification",
                "save_artifacts",
                "attach_job",
                "ask_user",
                "repl tool",
                "repl kernel",
            ] {
                assert!(
                    !lower.contains(stale),
                    "legacy host contract {stale:?} remains in {}",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn every_bundled_skill_parses_and_matches_its_directory() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills");
        let mut names = std::collections::HashSet::new();
        let mut count = 0usize;
        for entry in std::fs::read_dir(&root).expect("read bundled skills") {
            let dir = entry.expect("read bundled skill entry").path();
            let path = dir.join("SKILL.md");
            if !path.is_file() {
                continue;
            }
            let skill = parse_skill_file(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let folder = dir.file_name().and_then(|value| value.to_str()).unwrap();
            assert_eq!(skill.name, folder, "skill name differs from folder");
            assert!(
                !skill.description.trim().is_empty(),
                "{} has no description",
                path.display()
            );
            assert!(names.insert(skill.name), "duplicate bundled skill name");
            count += 1;
        }
        assert!(
            count >= 26,
            "unexpectedly small bundled skill catalog: {count}"
        );
        for name in RETIRED_BUNDLED_PACKAGES {
            assert!(
                !root.join(name).join("SKILL.md").exists(),
                "retired package is bundled: {name}"
            );
        }
    }

    #[test]
    fn retired_bundled_packages_do_not_shadow_user_installations_or_modify_files() {
        let temp = crate::distribution::StagingDir::new(&std::env::temp_dir()).unwrap();
        let bundled = temp.0.join("bundled");
        let user = temp.0.join("user");
        for base in [&bundled, &user] {
            for name in RETIRED_BUNDLED_PACKAGES
                .iter()
                .copied()
                .chain(["bear-custom", "literature-review"])
            {
                let dir = base.join(name);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(
                    dir.join("SKILL.md"),
                    format!("---\nname: {name}\ndescription: Test skill\n---\nInstructions"),
                )
                .unwrap();
            }
        }
        // Even a malformed stale copy must not block a fresh marketplace install.
        std::fs::write(bundled.join("bear-support/SKILL.md"), "broken old copy").unwrap();
        let old = SkillIndex::load_scoped(&[(bundled.clone(), SkillSource::Bundled)]);
        assert_eq!(old.all().len(), 2);
        for scope in [
            SkillSource::Global,
            SkillSource::Project,
            SkillSource::Extra,
            SkillSource::Plugin,
            SkillSource::Custom,
        ] {
            let index = SkillIndex::load_scoped(&[
                (bundled.clone(), SkillSource::Bundled),
                (user.clone(), scope),
            ]);
            for name in RETIRED_BUNDLED_PACKAGES {
                assert_eq!(index.source(name), Some(scope));
                let records: Vec<_> = index
                    .catalog_records()
                    .iter()
                    .filter(|record| record.name == *name)
                    .collect();
                assert_eq!(records.len(), 1);
                assert!(records[0].effective);
                assert!(bundled.join(name).join("SKILL.md").is_file());
                assert!(user.join(name).join("SKILL.md").is_file());
            }
            assert_eq!(index.source("bear-custom"), Some(SkillSource::Bundled));
            assert_eq!(
                index.source("literature-review"),
                Some(SkillSource::Bundled)
            );
        }
    }

    #[test]
    fn gpu_skills_use_the_persisted_wisp_run_lifecycle() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills");
        for entry in std::fs::read_dir(&root).expect("read bundled skills") {
            let path = entry
                .expect("read bundled skill entry")
                .path()
                .join("SKILL.md");
            if !path.is_file() {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("read bundled skill");
            if !source.contains("requirements: [gpu]") {
                continue;
            }
            for tool in ["run_in_context", "get_run", "monitor_run", "cancel_run"] {
                assert!(
                    source.contains(tool),
                    "GPU skill {} omits {tool}",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn merge_preserves_host_skill_on_name_collision() {
        let host = SkillIndex {
            skills: vec![skill("host"), skill("shared")],
            sources: HashMap::new(),
            records: vec![],
        };
        let plugin = SkillIndex {
            skills: vec![
                skill("plugin"),
                Skill {
                    description: "plugin copy".into(),
                    ..skill("shared")
                },
            ],
            sources: HashMap::new(),
            records: vec![],
        };
        let merged = host.merged_preserving_self(&plugin);
        let names: Vec<_> = merged
            .all()
            .iter()
            .map(|skill| skill.name.as_str())
            .collect();
        assert_eq!(names, vec!["host", "plugin", "shared"]);
        assert_eq!(merged.get("shared").unwrap().description, "desc shared");
    }

    #[test]
    fn plugin_enable_does_not_revive_a_disabled_host_collision() {
        let host = SkillIndex {
            skills: vec![skill("shared")],
            sources: HashMap::new(),
            records: vec![],
        };
        let plugin = SkillIndex {
            skills: vec![skill("plugin"), skill("shared")],
            sources: HashMap::new(),
            records: vec![],
        };
        let enabled = HashSet::from(["plugin".to_string()]);
        let filtered = host
            .merged_preserving_self(&plugin)
            .filtered_by_names(Some(&enabled));
        assert!(filtered.get("shared").is_none());
        assert!(filtered.get("plugin").is_some());
    }

    #[test]
    fn scoped_load_keeps_the_first_same_named_skill() {
        let root = std::env::temp_dir().join(format!(
            "wisp-skill-precedence-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = root.join("project").join("shared");
        let global = root.join("global").join("shared");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&global).unwrap();
        std::fs::write(
            project.join("SKILL.md"),
            "---\nname: shared\ndescription: project copy\n---\nproject",
        )
        .unwrap();
        std::fs::write(
            global.join("SKILL.md"),
            "---\nname: shared\ndescription: global copy\n---\nglobal",
        )
        .unwrap();

        let index = SkillIndex::load_scoped(&[
            (root.join("project"), SkillSource::Project),
            (root.join("global"), SkillSource::Global),
        ]);
        assert_eq!(index.all().len(), 1);
        assert_eq!(index.get("shared").unwrap().description, "project copy");
        assert_eq!(index.source("shared"), Some(SkillSource::Project));

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn tag_overrides_preserve_source_metadata() {
        let mut index = SkillIndex {
            skills: vec![skill("literature")],
            sources: HashMap::from([("literature".into(), SkillSource::Project)]),
            records: vec![],
        };
        index.skills[0].tags = vec!["original".into()];
        let overrides = BTreeMap::from([(
            "literature".into(),
            vec!["custom".into(), "中文别名".into()],
        )]);

        let updated = index.with_tag_overrides(&overrides);
        assert_eq!(
            updated.get("literature").unwrap().tags,
            overrides["literature"]
        );
        assert_eq!(updated.source("literature"), Some(SkillSource::Project));
    }

    #[test]
    fn catalog_snapshot_keeps_shadowed_and_parse_error_records() {
        let root = std::env::temp_dir().join(format!(
            "wisp-skill-audit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bundled = root.join("bundled").join("shared");
        let project = root.join("project").join("shared");
        let broken = root.join("project").join("broken");
        for dir in [&bundled, &project, &broken] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(
            bundled.join("SKILL.md"),
            "---\nname: shared\nversion: 1.2.3\ndescription: first\n---\nfirst",
        )
        .unwrap();
        std::fs::write(
            project.join("SKILL.md"),
            "---\nname: shared\ndescription: second\n---\nsecond",
        )
        .unwrap();
        std::fs::write(broken.join("SKILL.md"), "not frontmatter").unwrap();

        let index = SkillIndex::load_scoped(&[
            (root.join("bundled"), SkillSource::Bundled),
            (root.join("project"), SkillSource::Project),
        ]);
        let audit = index.catalog_audit();
        assert_eq!(audit.discovered_count, 3);
        assert_eq!(audit.effective_count, 1);
        assert_eq!(audit.duplicate_count, 1);
        assert_eq!(audit.parse_error_count, 1);
        let winner = index.effective_record("shared").unwrap();
        assert_eq!(winner.declared_version.as_deref(), Some("1.2.3"));
        assert_eq!(winner.skill_md_sha256.as_deref().unwrap().len(), 64);
        let shadowed = index
            .catalog_records()
            .iter()
            .find(|record| record.name == "shared" && !record.effective)
            .unwrap();
        assert_eq!(
            shadowed.shadowed_by.as_deref(),
            Some(winner.record_id.as_str())
        );
        assert!(index
            .catalog_records()
            .iter()
            .any(|record| record.name == "broken" && record.parse_error.is_some()));

        std::fs::remove_dir_all(root).ok();
    }
}

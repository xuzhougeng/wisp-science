use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillManifest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: SkillTags,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub wisp: Option<WispSkillMetadata>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SkillTags(pub Vec<String>);

impl<'de> Deserialize<'de> for SkillTags {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum RawTags {
            String(String),
            List(Vec<String>),
        }
        let raw = Option::<RawTags>::deserialize(deserializer)?;
        let tags = match raw {
            None => vec![],
            Some(RawTags::String(value)) => value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect(),
            Some(RawTags::List(values)) => values,
        };
        Ok(Self(normalize_values(tags)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WispSkillMetadata {
    pub schema_version: u32,
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default)]
    pub research_stages: Vec<String>,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub evidence_types: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub side_effects: SkillSideEffects,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSideEffects {
    #[default]
    ReadOnly,
    Network,
    ProjectWrite,
    CodeExecution,
    ExternalService,
}

const DOMAINS: &[&str] = &[
    "general",
    "bioinformatics",
    "oncology",
    "single-cell",
    "genomics",
    "transcriptomics",
    "proteomics",
    "scientific-literature",
];
const RESEARCH_STAGES: &[&str] = &[
    "observation",
    "retrieval",
    "analysis",
    "hypothesis",
    "validation",
    "synthesis",
];
const ROLES: &[&str] = &[
    "retrieval",
    "analyst",
    "planner",
    "critic",
    "validator",
    "synthesizer",
];
const EVIDENCE_TYPES: &[&str] = &[
    "literature",
    "project-data",
    "omics",
    "single-cell",
    "computational",
    "experimental",
];
const OUTPUTS: &[&str] = &[
    "evidence-matrix",
    "hypothesis-card",
    "research-design",
    "analysis-module",
    "literature-review",
    "risk-map",
    "validation-plan",
    "research-timeline",
];

impl WispSkillMetadata {
    pub fn validate(&mut self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err(format!(
                "unsupported wisp.schema_version {}; expected 1",
                self.schema_version
            ));
        }
        normalize_and_validate("domains", &mut self.domains, DOMAINS)?;
        normalize_and_validate(
            "research_stages",
            &mut self.research_stages,
            RESEARCH_STAGES,
        )?;
        normalize_and_validate("roles", &mut self.roles, ROLES)?;
        normalize_and_validate("evidence_types", &mut self.evidence_types, EVIDENCE_TYPES)?;
        normalize_and_validate("outputs", &mut self.outputs, OUTPUTS)?;
        Ok(())
    }
}

pub fn parse_skill_document(
    text: &str,
    fallback_name: String,
) -> Result<(SkillManifest, String), String> {
    // Editors on Windows often save a UTF-8 byte-order mark.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split_inclusive('\n');
    let first = lines
        .next()
        .ok_or_else(|| "SKILL.md is empty".to_string())?;
    if first.trim_end_matches(['\r', '\n']).trim() != "---" {
        return Err("SKILL.md has no frontmatter (--- block)".into());
    }
    let yaml_start = first.len();
    let mut cursor = yaml_start;
    let mut yaml_end = None;
    let mut body_start = None;
    for line in lines {
        let next = cursor + line.len();
        if line.trim_end_matches(['\r', '\n']).trim() == "---" {
            yaml_end = Some(cursor);
            body_start = Some(next);
            break;
        }
        cursor = next;
    }
    let yaml_end = yaml_end.ok_or_else(|| {
        "SKILL.md frontmatter is not closed with a standalone --- line".to_string()
    })?;
    let mut manifest: SkillManifest = serde_yaml::from_str(&text[yaml_start..yaml_end])
        .map_err(|error| format!("invalid SKILL.md YAML frontmatter: {error}"))?;
    manifest.name = manifest
        .name
        .take()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .or(Some(fallback_name));
    manifest.description = manifest
        .description
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    manifest.version = manifest
        .version
        .take()
        .map(|version| version.trim().to_string())
        .filter(|version| !version.is_empty());
    if let Some(wisp) = &mut manifest.wisp {
        wisp.validate()?;
    }
    let body = text[body_start.unwrap_or(text.len())..].trim().to_string();
    Ok((manifest, body))
}

fn normalize_and_validate(
    field: &str,
    values: &mut Vec<String>,
    vocabulary: &[&str],
) -> Result<(), String> {
    *values = normalize_values(std::mem::take(values));
    if let Some(value) = values
        .iter()
        .find(|value| !vocabulary.contains(&value.as_str()))
    {
        return Err(format!("unknown wisp.{field} value '{value}'"));
    }
    Ok(())
}

fn normalize_values(values: Vec<String>) -> Vec<String> {
    let mut values = values
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_yaml_block_scalars_lists_and_extra_fields() {
        let (manifest, body) = parse_skill_document(
            "---\nname: demo\ndescription: >\n  line one\n  line two\ntags: [B, a, a]\nlicense: Apache-2.0\nwisp:\n  schema_version: 1\n  domains: [oncology, single-cell]\n  research_stages: [hypothesis]\n  roles: [critic]\n  evidence_types: [literature]\n  outputs: [evidence-matrix]\n  side_effects: read_only\n---\n# Body\n",
            "fallback".into(),
        )
        .unwrap();
        assert_eq!(manifest.name.as_deref(), Some("demo"));
        assert_eq!(manifest.description, "line one line two");
        assert_eq!(manifest.tags.0, ["a", "b"]);
        assert!(manifest.extra.contains_key("license"));
        assert_eq!(manifest.wisp.unwrap().domains, ["oncology", "single-cell"]);
        assert_eq!(body, "# Body");
    }

    #[test]
    fn accepts_a_leading_byte_order_mark() {
        let (manifest, body) = parse_skill_document(
            "\u{feff}---\nname: bom\ndescription: saved by a Windows editor\n---\nbody",
            "fallback".into(),
        )
        .unwrap();
        assert_eq!(manifest.name.as_deref(), Some("bom"));
        assert_eq!(body, "body");
    }

    #[test]
    fn rejects_unknown_semantics_but_accepts_legacy_documents() {
        assert!(parse_skill_document(
            "---\nname: old\ndescription: old\nmetadata: {custom: true}\n---\nbody",
            "fallback".into(),
        )
        .is_ok());
        let error = parse_skill_document(
            "---\nname: bad\ndescription: bad\nwisp:\n  schema_version: 1\n  roles: [oracle]\n---\nbody",
            "fallback".into(),
        )
        .unwrap_err();
        assert!(error.contains("unknown wisp.roles"));
    }
}

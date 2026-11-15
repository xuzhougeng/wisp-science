//! How a project's tools are approved, as the WebView's Permissions and
//! Connections pages keep it: a mode (`approval_scope:{project}`) and rules
//! for single tools (`tool_approvals:{project}`, a tool's name to `ask` or
//! `deny`; a tool without one is allowed by its rule). A project without
//! its own value reads the WebView's default under the key alone.
//!
//! The mode sits over the rules and over this desktop's own question before
//! changes. Per-tool rules (the default) keeps both. Auto-approve skips the
//! ordinary questions; a dangerous shell command still asks. Full bypass
//! skips every question, the dangerous command's too. A tool that is denied
//! stays denied in every mode.

use std::collections::BTreeMap;

use anyhow::{ensure, Result};
use wisp_store::Store;
use wisp_tools::Approval;

use crate::Server;

const SCOPE: &str = "approval_scope";
const RULES: &str = "tool_approvals";

/// The mode over a project's approvals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ApprovalScope {
    /// Each tool's rule and the question before changes apply.
    #[default]
    Ask,
    /// Ordinary questions are skipped; dangerous commands still ask.
    Auto,
    /// Nothing asks.
    Full,
}

impl ApprovalScope {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Auto => "auto",
            Self::Full => "full",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "auto" => Self::Auto,
            "full" => Self::Full,
            _ => Self::Ask,
        }
    }
}

/// A rule for one tool. A tool without one is allowed by its rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolRule {
    /// Asks before every call, even one that changes nothing.
    Ask,
    /// Never runs.
    Deny,
}

impl ToolRule {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Deny => "deny",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "ask" => Some(Self::Ask),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

/// A project's approvals.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectApprovals {
    pub scope: ApprovalScope,
    /// Tool name to its rule, for the tools that have one.
    pub rules: BTreeMap<String, ToolRule>,
}

impl ProjectApprovals {
    /// What the tool's rule says, under the mode.
    pub(crate) fn mode(&self, tool: &str) -> Approval {
        match (self.rules.get(tool), self.scope) {
            (Some(ToolRule::Deny), _) => Approval::Deny,
            (Some(ToolRule::Ask), ApprovalScope::Ask) => Approval::Ask,
            _ => Approval::Allow,
        }
    }

    /// Whether this desktop's question before changes is still asked.
    pub(crate) fn asks_before_changes(&self) -> bool {
        self.scope == ApprovalScope::Ask
    }

    /// Whether every question is skipped, a dangerous command's included.
    pub(crate) fn bypasses_all(&self) -> bool {
        self.scope == ApprovalScope::Full
    }
}

/// A setting of the project's, or the WebView's default when it has none.
async fn read<T: serde::de::DeserializeOwned>(
    store: &Store,
    key: &str,
    project: &str,
) -> Option<T> {
    for key in [format!("{key}:{project}"), key.to_owned()] {
        if let Some(value) = store
            .get_setting(&key)
            .await
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str(&raw).ok())
        {
            return Some(value);
        }
    }
    None
}

/// A project's approvals as they are saved.
pub(crate) async fn load(store: &Store, project: &str) -> ProjectApprovals {
    let scope = read::<String>(store, SCOPE, project)
        .await
        .map(|value| ApprovalScope::parse(&value))
        .unwrap_or_default();
    let rules = read::<BTreeMap<String, String>>(store, RULES, project)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(tool, rule)| Some((tool, ToolRule::parse(&rule)?)))
        .collect();
    ProjectApprovals { scope, rules }
}

impl Server {
    /// How the project's tools are approved.
    pub async fn project_approvals(&self, project: &str) -> Result<ProjectApprovals> {
        self.known_project(project).await?;
        Ok(load(&self.0.store, project).await)
    }

    /// Sets the project's mode; it applies from the next tool call of a
    /// turn that starts after it.
    pub async fn set_approval_scope(
        &self,
        project: &str,
        scope: ApprovalScope,
    ) -> Result<ProjectApprovals> {
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        self.0
            .store
            .set_setting(
                &format!("{SCOPE}:{project}"),
                &serde_json::to_string(scope.as_str())?,
            )
            .await?;
        Ok(load(&self.0.store, project).await)
    }

    /// Gives a tool of the project a rule, or with `None` takes it away so
    /// the tool is allowed by its rule again.
    pub async fn set_tool_rule(
        &self,
        project: &str,
        tool: &str,
        rule: Option<ToolRule>,
    ) -> Result<ProjectApprovals> {
        let tool = tool.trim();
        ensure!(
            !tool.is_empty() && tool.len() <= 128 && !tool.chars().any(char::is_control),
            "Name a tool"
        );
        let _admission = self.0.admission.read().await;
        self.known_project(project).await?;
        let store = &self.0.store;
        // The whole list is written for the project, as the WebView does,
        // so a project that read the default keeps its other rules.
        let mut rules: BTreeMap<String, String> = load(store, project)
            .await
            .rules
            .into_iter()
            .map(|(name, rule)| (name, rule.as_str().to_owned()))
            .collect();
        match rule {
            Some(rule) => rules.insert(tool.to_owned(), rule.as_str().to_owned()),
            None => rules.remove(tool),
        };
        store
            .set_setting(
                &format!("{RULES}:{project}"),
                &serde_json::to_string(&rules)?,
            )
            .await?;
        Ok(load(store, project).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rule_applies_under_its_mode_and_a_denial_under_every_mode() {
        let mut approvals = ProjectApprovals::default();
        approvals
            .rules
            .insert("pubmed_search".into(), ToolRule::Ask);
        approvals
            .rules
            .insert("uniprot_fetch".into(), ToolRule::Deny);
        assert_eq!(approvals.mode("pubmed_search"), Approval::Ask);
        assert_eq!(approvals.mode("uniprot_fetch"), Approval::Deny);
        assert_eq!(approvals.mode("shell"), Approval::Allow);
        assert!(approvals.asks_before_changes() && !approvals.bypasses_all());
        for scope in [ApprovalScope::Auto, ApprovalScope::Full] {
            approvals.scope = scope;
            assert_eq!(approvals.mode("pubmed_search"), Approval::Allow);
            assert_eq!(approvals.mode("uniprot_fetch"), Approval::Deny);
            assert!(!approvals.asks_before_changes());
        }
        assert!(approvals.bypasses_all());
        assert_eq!(ApprovalScope::parse("auto"), ApprovalScope::Auto);
        assert_eq!(ApprovalScope::parse("anything"), ApprovalScope::Ask);
        assert_eq!(ToolRule::parse("allow"), None);
    }
}

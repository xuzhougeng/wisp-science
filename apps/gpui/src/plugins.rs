//! Plugins as Settings lists them.
use wisp_server::desktop::PluginRow;

use crate::i18n::{fill, tr};

/// Who made it and under what terms, as far as the package says.
pub fn byline(row: &PluginRow) -> String {
    let mut parts = vec![fill(tr("Version {}"), &[&row.version])];
    if !row.author.trim().is_empty() {
        parts.push(row.author.trim().to_owned());
    }
    if !row.license.trim().is_empty() {
        parts.push(row.license.trim().to_owned());
    }
    parts.join(" · ")
}

/// The skills it brings, or that it brings none.
pub fn skills_line(row: &PluginRow) -> String {
    if row.skills.is_empty() {
        tr("No skills").to_owned()
    } else {
        fill(tr("Skills: {}"), &[&row.skills.join(", ")])
    }
}

/// The MCP servers it declares, which this desktop does not start.
pub fn servers_line(row: &PluginRow) -> Option<String> {
    (!row.servers.is_empty()).then(|| {
        fill(
            tr("MCP servers, not started by this desktop: {}"),
            &[&row.servers.join("; ")],
        )
    })
}

/// What switching it on or off does, said under the switch.
pub fn switch_note(project: Option<&str>) -> String {
    match project {
        Some(name) => fill(tr("For the project {}"), &[&name]),
        None => tr("Open a project to switch a plugin on for it.").to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> PluginRow {
        PluginRow {
            id: "motif-tools".into(),
            version: "1.2.0".into(),
            name: "Motif tools".into(),
            description: "Find motifs".into(),
            author: " Lab ".into(),
            license: "MIT".into(),
            source: "/packages/motif".into(),
            enabled: false,
            skills: vec!["find-motifs".into(), "scan".into()],
            servers: vec!["uvx motif-mcp --stdio".into()],
            problems: vec![],
        }
    }

    #[test]
    fn a_plugin_says_what_it_is_and_what_it_brings() {
        let mut plugin = row();
        assert_eq!(byline(&plugin), "Version 1.2.0 · Lab · MIT");
        assert_eq!(skills_line(&plugin), "Skills: find-motifs, scan");
        assert_eq!(
            servers_line(&plugin).as_deref(),
            Some("MCP servers, not started by this desktop: uvx motif-mcp --stdio")
        );
        plugin.author.clear();
        plugin.license.clear();
        plugin.skills.clear();
        plugin.servers.clear();
        assert_eq!(byline(&plugin), "Version 1.2.0");
        assert_eq!(skills_line(&plugin), "No skills");
        assert_eq!(servers_line(&plugin), None);
        assert_eq!(switch_note(Some("Lab")), "For the project Lab");
        assert!(switch_note(None).starts_with("Open a project"));
    }
}

use std::path::Path;

pub const WORKSPACE_DIRS: &[&str] = &[
    ".wisp",
    "data/raw",
    "data/external",
    "data/processed",
    // Landing zone for files pulled off a compute host; one subdirectory per
    // execution-context label, created on demand by the transfer.
    "remote",
    "analysis/scripts",
    "analysis/notebooks",
    "analysis/workflows",
    "runs",
    "results/tables",
    "results/models",
    "results/reports",
    "figures",
    "literature",
    "docs",
];

pub fn init_workspace_layout(root: &Path, project_id: &str, name: &str) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    for dir in WORKSPACE_DIRS {
        std::fs::create_dir_all(root.join(dir)).map_err(|e| e.to_string())?;
    }
    let manifest = format!(
        "layout_version = 1\nproject_id = \"{}\"\nname = \"{}\"\ncreated_at = {}\n",
        toml_escape(project_id),
        toml_escape(name),
        chrono::Utc::now().timestamp()
    );
    std::fs::write(root.join(".wisp").join("project.toml"), manifest).map_err(|e| e.to_string())
}

fn toml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initializes_typed_workspace_layout_and_manifest() {
        let root = std::env::temp_dir().join(format!("wisp_manifest_{}", uuid::Uuid::new_v4()));
        init_workspace_layout(&root, "p1", "Cancer Study").unwrap();

        for dir in WORKSPACE_DIRS {
            assert!(root.join(dir).is_dir(), "missing {dir}");
        }
        let manifest = std::fs::read_to_string(root.join(".wisp/project.toml")).unwrap();
        assert!(manifest.contains("layout_version = 1"), "{manifest}");
        assert!(manifest.contains("project_id = \"p1\""), "{manifest}");
        assert!(manifest.contains("name = \"Cancer Study\""), "{manifest}");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn initialization_preserves_existing_files() {
        let root =
            std::env::temp_dir().join(format!("wisp_manifest_preserve_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("data/raw")).unwrap();
        std::fs::write(root.join("data/raw/counts.tsv"), b"keep").unwrap();

        init_workspace_layout(&root, "p1", "Study").unwrap();

        assert_eq!(
            std::fs::read(root.join("data/raw/counts.tsv")).unwrap(),
            b"keep"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}

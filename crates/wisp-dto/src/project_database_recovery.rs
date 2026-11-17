//! A pending notice that a project's database was found damaged and replaced
//! (#1635). The record lives in the application database, so it is shown the
//! next time the project is entered, survives restarts, and is gone once the
//! user acknowledges it.
use serde::{Deserialize, Serialize};

/// What replaced the damaged database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectDatabaseRecoveryKind {
    /// The newest backup copy that opens took its place; records made after
    /// that copy are missing.
    Restored,
    /// No copy opened, so an empty database listing only the project was
    /// created; the conversation history is gone from the database.
    Rebuilt,
}

impl ProjectDatabaseRecoveryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Restored => "restored",
            Self::Rebuilt => "rebuilt",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "restored" => Some(Self::Restored),
            "rebuilt" => Some(Self::Rebuilt),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectDatabaseRecovery {
    pub id: i64,
    pub project_id: String,
    /// Unix seconds.
    pub occurred_at: i64,
    pub kind: ProjectDatabaseRecoveryKind,
    /// File name of the backup copy used; set only for `Restored`.
    #[serde(default)]
    pub copy_name: Option<String>,
    /// Where the damaged database (and its journal files) was kept aside.
    pub damaged_path: String,
    /// The folder backup copies are read from, when it is known.
    #[serde(default)]
    pub backup_dir: Option<String>,
}

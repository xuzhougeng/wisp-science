//! Explorations: trying a question several ways from one turn of a
//! conversation, each in a copy of the project, then keeping one. A port
//! of the WebView's explorations; see `service.rs` and `promotion.rs`.

mod commands;
pub(crate) mod promotion;
pub(crate) mod service;

pub use commands::{FileChange, PromotionPreview, StartedExploration};

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Per project: a lock turns hold shared and exploration operations take
/// alone, and the gate that makes candidates of one round share one
/// checkpoint. The WebView's `ProjectActivityLocks`.
#[derive(Default)]
pub(crate) struct ProjectActivityLocks {
    projects: Mutex<HashMap<String, Arc<tokio::sync::RwLock<()>>>>,
    exploration_creation: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl ProjectActivityLocks {
    pub(crate) fn project(&self, project: &str) -> Arc<tokio::sync::RwLock<()>> {
        self.projects
            .lock()
            .unwrap()
            .entry(project.to_owned())
            .or_default()
            .clone()
    }

    pub(crate) fn exploration_creation(&self, project: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.exploration_creation
            .lock()
            .unwrap()
            .entry(project.to_owned())
            .or_default()
            .clone()
    }
}

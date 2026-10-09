use super::*;

#[derive(Clone)]
pub(crate) enum FolderModal {
    Create,
    Rename(String),
}

#[derive(Clone)]
pub(crate) enum FileEntryModal {
    CreateFile,
    CreateDirectory,
    Rename { path: String, is_dir: bool },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionTransferMode {
    Copy,
    Move,
}

impl SessionTransferMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Move => "move",
        }
    }
}

#[derive(Clone)]
pub(crate) struct SessionTransfer {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) mode: SessionTransferMode,
    pub(crate) target_project_id: String,
    pub(crate) from_demo: bool,
}

#[derive(Clone)]
pub(crate) enum UiConfirm {
    EnableFullPermission(Option<String>),
    DeleteFolder(String),
    DeleteSessions(Vec<String>),
    AbandonExploration(String),
    DeleteFileEntry { path: String, is_dir: bool },
    ReloadProjectRules(String),
    SaveAgentContext,
}

#[derive(Clone)]
pub(crate) enum UpdateCheckModal {
    Checking,
    Available {
        version: String,
        notes: String,
        release_url: String,
        install_supported: bool,
        downloading: bool,
    },
    Downloading {
        version: String,
        downloaded_bytes: RwSignal<u64>,
        total_bytes: RwSignal<Option<u64>>,
    },
    ReadyToInstall {
        version: String,
        release_url: String,
    },
    Installing {
        version: String,
    },
    UpToDate {
        version: String,
    },
    Failed {
        message: String,
        release_url: Option<String>,
    },
}

impl UpdateCheckModal {
    pub(crate) fn dismissible(&self) -> bool {
        !matches!(self, Self::Downloading { .. } | Self::Installing { .. })
    }
}

/// A newer release found by the auto-check, surfaced as the sidebar prompt card.
#[derive(Clone)]
pub(crate) struct AvailableUpdate {
    pub(crate) version: String,
}

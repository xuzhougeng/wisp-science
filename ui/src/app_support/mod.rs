use super::{
    window_capture_escape, HOME_SEARCH_ARTIFACT_LIMIT, HOME_SEARCH_PROJECT_LIMIT,
    HOME_SEARCH_SESSION_LIMIT, THEME_STORAGE_KEY,
};
use crate::bindings::{
    attach_cropped_region, crop_region_to_upload, invoke, invoke_checked, is_mac, mount_preview,
    native_drop_remote_target, open_external_url, schedule_highlight, schedule_run_output_follow,
    set_highlighted_code, upload_files, upload_input_files, upload_pasted_images,
};
use crate::dto::*;
use crate::i18n::{localize_backend, t, tf, use_locale, Locale};
use crate::publication::PublicationEvidenceSource;
use crate::text::{
    code_lang, decode_href, dom_value, event_target_value, extract_href_from_tag, fasta_seq_count,
    fenced_blocks, file_kind, format_bytes, format_duration_ms, html_escape, ime_composing,
    is_external_href, is_separator, is_table_row, join_api_url, md_document_to_html,
    md_inline_to_html, md_to_html, next_artifact_id, normalize_endpoint, normalize_path,
    opens_in_system_browser, parent_path, parse_csv_line, parse_notebook, pretty_json,
    provider_defaults, provider_value, same_endpoint, split_row, tool_card_label, tool_lang,
    unique_dom_id, user_message_presentation, NbOutput, Notebook, DEEPSEEK_FLASH_MODEL,
    DEEPSEEK_PRO_MODEL,
};
use leptos::{ev, window_event_listener, *};
use serde_wasm_bindgen::to_value;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

mod artifacts;
mod chat_stream;
mod composer;
mod context_usage;
mod dnd;
mod extensions;
mod files;
mod issue_report;
mod markdown;
mod messages;
mod modals;
mod model_settings;
mod palettes;
mod pane_layout;
mod prefs;
mod previews;
mod projects;
mod runtime;
mod session_import;
mod sessions;
mod settings;
mod share;
mod ssh;
mod transcript;

pub(crate) use artifacts::*;
pub(crate) use chat_stream::*;
pub(crate) use composer::*;
pub(crate) use context_usage::*;
pub(crate) use dnd::*;
pub(crate) use extensions::*;
pub(crate) use files::*;
pub(crate) use issue_report::*;
pub(crate) use markdown::*;
pub(crate) use messages::*;
pub(crate) use modals::*;
pub(crate) use model_settings::*;
pub(crate) use palettes::*;
pub(crate) use pane_layout::*;
pub(crate) use prefs::*;
pub(crate) use previews::*;
pub(crate) use projects::*;
pub(crate) use runtime::*;
pub(crate) use session_import::*;
pub(crate) use sessions::*;
pub(crate) use settings::*;
pub(crate) use share::*;
pub(crate) use ssh::*;
pub(crate) use transcript::*;

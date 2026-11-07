//! Explicit-project transport for researcher-private notes. Reuses ProjectMemo.
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "wisp.native-memos.v1";
pub const COMMANDS: &[&str] = &[
    "native_project_memos",
    "native_project_memo_add",
    "native_project_memo_update",
    "native_project_memo_delete",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListRequest {}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddRequest {
    pub input: crate::ProjectMemoInput,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateRequest {
    pub patch: crate::ProjectMemoPatch,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteRequest {
    pub memo_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MemoList {
    pub schema: String,
    pub project_id: String,
    pub memos: Vec<crate::ProjectMemo>,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct MemoResult {
    pub schema: String,
    pub project_id: String,
    pub memo: crate::ProjectMemo,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct DeletedMemo {
    pub schema: String,
    pub project_id: String,
    pub memo_id: String,
}

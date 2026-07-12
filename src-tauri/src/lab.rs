use sha2::Digest;
use wisp_llm::ToolSchema;
use wisp_store::{
    LabActorKind, LabAliasCreate, LabAmendmentCreate, LabConclusionCreate, LabDataEvidenceCreate,
    LabDocumentUpsert, LabEntityCreate, LabEntityKind, LabLocationCreate, LabLotCreate,
    LabMaterialAdjustment, LabMaterialDerivationCreate, LabMaterialMove, LabMaterialUnitCreate,
    LabProtocolRevisionCreate, LabQcAssessmentCreate, LabQcObservationCreate, LabQuantity,
    LabReservationCreate, LabResourceDefinitionCreate, LabRunDeviationCreate, LabRunOutputCreate,
    LabRunParticipantCreate, LabRunProtocolPin, LabRunStatusUpdate, LabSubjectParticipantCreate,
    LabTransactionRequest, Store,
};
use wisp_tools::{ConfirmDecision, DomainConfirmationRequest, Tool, ToolEnv, ToolResult};

#[tauri::command]
pub async fn get_lab_bench(
    state: tauri::State<'_, crate::AppState>,
    project_id: String,
    frame_id: String,
) -> Result<serde_json::Value, String> {
    use chrono::TimeZone;
    let local = chrono::Local::now();
    let since = chrono::Local
        .from_local_datetime(&local.date_naive().and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .map(|value| value.timestamp())
        .unwrap_or_else(|| local.timestamp() - 86_400);
    let today = state
        .store
        .list_project_lab_events_since(&project_id, since)
        .await
        .map_err(|error| error.to_string())?;
    let conversation = state
        .store
        .get_conversation_wet_lab_run(&project_id, &frame_id)
        .await
        .map_err(|error| error.to_string())?;
    let Some(conversation) = conversation else {
        return Ok(serde_json::json!({"conversation":null,"provenance":null,"today":today}));
    };
    let provenance = state
        .store
        .lab_run_provenance(&conversation.run.id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(serde_json::json!({"conversation":conversation,"provenance":provenance,"today":today}))
}

/// Write durable dossier projections only after their transaction has committed.
/// A failed or locked editor file remains in the outbox for a later retry; it
/// never rolls back a real-world lab action.
pub async fn flush_lab_projections(store: &Store, registry_id: &str) -> Vec<String> {
    let mut errors = vec![];
    let items = match store.list_lab_projection_outbox().await {
        Ok(items) => items,
        Err(error) => return vec![format!("Could not read projection outbox: {error}")],
    };
    for item in items
        .into_iter()
        .filter(|item| item.registry_id == registry_id)
    {
        let result = match store.get_lab_registry(&item.registry_id).await {
            Ok(Some(registry)) => match registry.root_path {
                Some(root) => {
                    let target_path = item.target_path.clone();
                    let content = item.content.clone();
                    match tokio::task::spawn_blocking(move || {
                        write_projection_file(std::path::Path::new(&root), &target_path, &content)
                    })
                    .await
                    {
                        Ok(result) => result,
                        Err(error) => {
                            Err(anyhow::anyhow!("Dossier projection worker failed: {error}"))
                        }
                    }
                }
                None => Err(anyhow::anyhow!(
                    "Lab registry has no dossier root configured"
                )),
            },
            Ok(None) => Err(anyhow::anyhow!("Lab registry no longer exists")),
            Err(error) => Err(error),
        };
        match result {
            Ok(()) => {
                if let Err(error) = store.acknowledge_lab_projection(&item.id).await {
                    errors.push(format!(
                        "Could not acknowledge {}: {error}",
                        item.target_path
                    ));
                }
            }
            Err(error) => {
                let message = error.to_string();
                let _ = store.fail_lab_projection(&item.id, &message).await;
                errors.push(format!("{}: {message}", item.target_path));
            }
        }
    }
    errors
}

fn write_projection_file(
    root: &std::path::Path,
    relative_path: &str,
    content: &str,
) -> anyhow::Result<()> {
    let normalized = relative_path.replace('\\', "/");
    let relative = std::path::Path::new(&normalized);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        })
    {
        anyhow::bail!("Dossier projection path escapes the registry root");
    }
    std::fs::create_dir_all(root)?;
    let root = dunce::canonicalize(root)?;
    let target = root.join(relative);
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Dossier projection has no parent directory"))?;
    std::fs::create_dir_all(parent)?;
    let parent = dunce::canonicalize(parent)?;
    if !parent.starts_with(&root) {
        anyhow::bail!("Dossier projection path escapes the registry root");
    }
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("dossier"),
        uuid::Uuid::new_v4()
    ));
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
    }
    let mut last_error = None;
    for attempt in 0..5 {
        match replace_projection_file(&temp, &target) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                if attempt < 4 {
                    std::thread::sleep(std::time::Duration::from_millis(20 * (attempt + 1)));
                }
            }
        }
    }
    let _ = std::fs::remove_file(&temp);
    return Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Dossier replace failed")));
}

#[cfg(not(windows))]
fn replace_projection_file(temp: &std::path::Path, target: &std::path::Path) -> anyhow::Result<()> {
    std::fs::rename(temp, target)?;
    Ok(())
}

#[cfg(windows)]
fn replace_projection_file(temp: &std::path::Path, target: &std::path::Path) -> anyhow::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        REPLACEFILE_WRITE_THROUGH,
    };
    let wide = |path: &std::path::Path| {
        path.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let temp_wide = wide(temp);
    let target_wide = wide(target);
    let ok = unsafe {
        if target.exists() {
            ReplaceFileW(
                target_wide.as_ptr(),
                temp_wide.as_ptr(),
                std::ptr::null(),
                REPLACEFILE_WRITE_THROUGH,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } else {
            MoveFileExW(
                temp_wide.as_ptr(),
                target_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

/// Read-only access to the lab ledger. Mutations live in `LabTransactionTool`
/// so ordinary agent conversation cannot bypass transaction receipts.
pub struct LabQueryTool {
    store: Store,
    project_id: String,
    frame_id: Option<String>,
}

impl LabQueryTool {
    pub fn new(store: Store, project_id: String, frame_id: Option<String>) -> Self {
        Self {
            store,
            project_id,
            frame_id,
        }
    }
}

#[async_trait::async_trait]
impl Tool for LabQueryTool {
    fn name(&self) -> &str {
        "lab_query"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "lab_query",
            "Read the shared wet-lab ledger. Use display IDs or aliases to resolve exact resources; aliases may intentionally return multiple candidates.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list_entities", "get_entity", "find_alias", "provenance", "preview_dossier_import", "preview_bulk_resources", "preview_output_manifest", "render_labels", "list_templates", "conversation_run", "run_closeout", "run_provenance"] },
                    "registry_id": { "type": "string" },
                    "entity_id": { "type": "string" },
                    "kind": { "type": "string", "enum": ["resource_definition", "lot", "material_unit", "subject", "location", "protocol_source"] },
                    "alias": { "type": "string" }
                    ,"markdown": { "type": "string" },
                    "template_kind": { "type": "string", "enum": ["plasmid", "cell_line", "antibody", "kit", "buffer", "mouse"] },
                    "rows": { "type": "array", "items": { "type": "object" } },
                    "run_id": { "type": "string" },
                    "entity_ids": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["action"]
            }),
        )
    }

    async fn run(&self, args: &serde_json::Value, _env: &dyn ToolEnv) -> ToolResult {
        let action = args.get("action").and_then(|value| value.as_str());
        let result = match action {
            Some("list_entities") => {
                let Some(registry_id) = args.get("registry_id").and_then(|value| value.as_str())
                else {
                    return ToolResult::fail("lab_query list_entities requires registry_id");
                };
                let kind = match args.get("kind").and_then(|value| value.as_str()) {
                    Some(value) => match parse_entity_kind(value) {
                        Ok(kind) => Some(kind),
                        Err(error) => return ToolResult::fail(error),
                    },
                    None => None,
                };
                self.store
                    .list_lab_entities(registry_id, kind)
                    .await
                    .map(|entities| serde_json::json!({"entities": entities}))
            }
            Some("get_entity") => {
                let Some(entity_id) = args.get("entity_id").and_then(|value| value.as_str()) else {
                    return ToolResult::fail("lab_query get_entity requires entity_id");
                };
                match self.store.get_lab_entity(entity_id).await {
                    Ok(Some(entity)) => {
                        let aliases = self.store.list_lab_aliases(&entity.id).await;
                        let details = match entity.kind {
                            LabEntityKind::ResourceDefinition => self
                                .store
                                .get_lab_resource_definition(&entity.id)
                                .await
                                .map(|value| {
                                    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
                                }),
                            LabEntityKind::Lot => {
                                self.store.get_lab_lot(&entity.id).await.map(|value| {
                                    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
                                })
                            }
                            LabEntityKind::MaterialUnit => self
                                .store
                                .get_lab_material_unit(&entity.id)
                                .await
                                .map(|value| {
                                    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
                                }),
                            LabEntityKind::Location => {
                                self.store.get_lab_location(&entity.id).await.map(|value| {
                                    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
                                })
                            }
                            LabEntityKind::Subject => {
                                self.store.get_lab_subject(&entity.id).await.map(|value| {
                                    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
                                })
                            }
                            _ => Ok(serde_json::Value::Null),
                        };
                        match (aliases, details) {
                            (Ok(aliases), Ok(details)) => Ok(serde_json::json!({
                                "entity": entity,
                                "aliases": aliases,
                                "details": details,
                            })),
                            (Err(error), _) | (_, Err(error)) => Err(error),
                        }
                    }
                    Ok(None) => Err(anyhow::anyhow!("Lab entity not found")),
                    Err(error) => Err(error),
                }
            }
            Some("find_alias") => {
                let Some(registry_id) = args.get("registry_id").and_then(|value| value.as_str())
                else {
                    return ToolResult::fail("lab_query find_alias requires registry_id");
                };
                let Some(alias) = args.get("alias").and_then(|value| value.as_str()) else {
                    return ToolResult::fail("lab_query find_alias requires alias");
                };
                self.store
                    .find_lab_aliases(registry_id, alias)
                    .await
                    .map(|aliases| serde_json::json!({"aliases": aliases}))
            }
            Some("provenance") => {
                let Some(entity_id) = args.get("entity_id").and_then(|value| value.as_str()) else {
                    return ToolResult::fail("lab_query provenance requires entity_id");
                };
                self.store
                    .lab_entity_provenance(entity_id)
                    .await
                    .map(|provenance| serde_json::json!({"provenance": provenance}))
            }
            Some("list_templates") => Ok(serde_json::json!({"templates":lab_bulk_templates()})),
            Some("preview_bulk_resources") => {
                let template = args
                    .get("template_kind")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let rows = args
                    .get("rows")
                    .and_then(|value| value.as_array())
                    .cloned()
                    .unwrap_or_default();
                let (entities, row_errors) = build_bulk_entities(template, &rows);
                Ok(
                    serde_json::json!({"dry_run":true,"valid_rows":entities.len(),"row_errors":row_errors,"can_commit":row_errors.is_empty()}),
                )
            }
            Some("preview_output_manifest") => {
                let run_id = args
                    .get("run_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let registry_id = args
                    .get("registry_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let rows = args
                    .get("rows")
                    .and_then(|value| value.as_array())
                    .cloned()
                    .unwrap_or_default();
                let (outputs, mut row_errors) = build_output_manifest(run_id, &rows);
                let mut seen_locations = std::collections::HashSet::new();
                for (index, output) in outputs.iter().enumerate() {
                    if let Some(location_id) = output.initial_location_id.as_deref() {
                        if !seen_locations.insert(location_id.to_string()) {
                            row_errors.push(serde_json::json!({"row":index + 1,"field":"location_id","error":"duplicate location in manifest"}));
                        } else if !self
                            .store
                            .lab_location_accepts_new_material(registry_id, location_id)
                            .await
                            .unwrap_or(false)
                        {
                            row_errors.push(serde_json::json!({"row":index + 1,"field":"location_id","error":"missing, cross-registry, or occupied"}));
                        }
                    }
                }
                Ok(
                    serde_json::json!({"dry_run":true,"valid_rows":outputs.len(),"row_errors":row_errors,"can_commit":row_errors.is_empty()}),
                )
            }
            Some("render_labels") => {
                let ids = args
                    .get("entity_ids")
                    .and_then(|value| value.as_array())
                    .cloned()
                    .unwrap_or_default();
                async {
                    let mut labels = vec![];
                    for id in ids.iter().filter_map(|value| value.as_str()) {
                        if let Some(entity) = self.store.get_lab_entity(id).await? {
                            let code = qrcode::QrCode::new(entity.display_id.as_bytes())?;
                            let svg = code.render::<qrcode::render::svg::Color>().min_dimensions(128, 128).build();
                            labels.push(serde_json::json!({"entity_id":entity.id,"display_id":entity.display_id,"title":entity.title,"qr_svg":svg}));
                        }
                    }
                    Ok::<_, anyhow::Error>(serde_json::json!({"labels":labels}))
                }.await
            }
            Some("conversation_run") => {
                let Some(frame_id) = self.frame_id.as_deref() else {
                    return ToolResult::fail("No active conversation is available");
                };
                self.store
                    .get_conversation_wet_lab_run(&self.project_id, frame_id)
                    .await
                    .map(|run| serde_json::json!({"conversation_wet_lab_run":run}))
            }
            Some("run_closeout") => {
                let Some(run_id) = args.get("run_id").and_then(|value| value.as_str()) else {
                    return ToolResult::fail("lab_query run_closeout requires run_id");
                };
                self.store
                    .wet_lab_run_closeout_summary(run_id)
                    .await
                    .map(|summary| serde_json::json!({"closeout":summary}))
            }
            Some("run_provenance") => {
                let Some(run_id) = args.get("run_id").and_then(|value| value.as_str()) else {
                    return ToolResult::fail("lab_query run_provenance requires run_id");
                };
                self.store
                    .lab_run_provenance(run_id)
                    .await
                    .map(|provenance| serde_json::json!({"provenance":provenance}))
            }
            Some("preview_dossier_import") => {
                let Some(entity_id) = args.get("entity_id").and_then(|value| value.as_str()) else {
                    return ToolResult::fail("lab_query preview_dossier_import requires entity_id");
                };
                let Some(markdown) = args.get("markdown").and_then(|value| value.as_str()) else {
                    return ToolResult::fail("lab_query preview_dossier_import requires markdown");
                };
                self.store
                    .preview_lab_document_import(entity_id, markdown)
                    .await
                    .map(|preview| serde_json::json!({"preview": preview}))
            }
            _ => return ToolResult::fail("lab_query action is not supported"),
        };
        match result {
            Ok(value) => ToolResult::ok(value.to_string()),
            Err(error) => ToolResult::fail(format!("lab_query error: {error}")),
        }
    }
}

/// Minimal agent write surface for the first useful lab record type. It always
/// creates a `LabTransaction`, and requests a domain confirmation itself even
/// when the generic per-tool approval policy is permissive.
pub struct LabTransactionTool {
    store: Store,
    project_id: String,
    frame_id: Option<String>,
}

impl LabTransactionTool {
    pub fn new(store: Store, project_id: String, frame_id: Option<String>) -> Self {
        Self {
            store,
            project_id,
            frame_id,
        }
    }
}

#[async_trait::async_trait]
impl Tool for LabTransactionTool {
    fn name(&self) -> &str {
        "lab_transaction"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "lab_transaction",
            "Create a typed wet-lab ledger transaction. This tool always asks for an explicit domain confirmation before committing.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["create_resource_definition", "create_protocol_source", "publish_protocol_revision", "pin_run_protocol", "create_lot", "create_material_unit", "create_location", "move_material_unit", "adjust_material_unit", "update_dossier", "import_dossier", "import_bulk_resources", "import_output_manifest", "create_wet_lab_run", "update_wet_lab_run_status", "create_run_output", "derive_materials", "record_run_participant", "record_subject_participant", "record_run_deviation", "attach_data_evidence", "record_qc_observation", "record_qc_assessment", "confirm_conclusion", "reserve_material", "record_amendment"] },
                    "registry_id": { "type": "string" },
                    "command_id": { "type": "string", "description": "Stable caller-generated id for safe retries" },
                    "title": { "type": "string" },
                    "prefix": { "type": "string", "description": "Stable ID prefix, e.g. AB or PL" },
                    "subtype": { "type": "string" },
                    "category": { "type": "string" },
                    "supplier": { "type": "string" },
                    "catalog_number": { "type": "string" },
                    "resource_definition_id": { "type": "string" },
                    "lot_number": { "type": "string" },
                    "lot_id": { "type": "string" },
                    "received_at": { "type": "integer" },
                    "expiry_at": { "type": "integer" },
                    "origin_kind": { "type": "string", "enum": ["receipt", "prepared", "legacy_import"] },
                    "usage_class": { "type": "string", "enum": ["inventory", "sample"] },
                    "quantity": { "type": "object", "properties": { "state": {"type":"string", "enum":["measured", "unknown", "not_measured"]}, "value": {"type":"string"}, "unit": {"type":"string"} }, "required":["state"] },
                    "vessel_description": { "type": "string" },
                    "lifecycle": { "type": "string", "enum": ["planned", "active", "depleted", "discarded", "lost", "void"] },
                    "availability": { "type": "string", "enum": ["available", "quarantined"] },
                    "identity_state": { "type": "string", "enum": ["verified", "suspect", "mislabeled"] },
                    "parent_location_id": { "type": "string" },
                    "location_class": { "type": "string" },
                    "single_occupancy": { "type": "boolean" },
                    "material_unit_id": { "type": "string" },
                    "subject_id": { "type": "string" },
                    "location_id": { "type": "string" },
                    "expected_revision": { "type": "integer" },
                    "reason": { "type": "string" },
                    "entity_id": { "type": "string" },
                    "relative_path": { "type": "string" },
                    "narrative_markdown": { "type": "string" },
                    "extension_fields": { "type": "object" },
                    "expected_document_revision": { "type": "integer" },
                    "operator": { "type": "string" },
                    "run_id": { "type": "string" },
                    "protocol_entity_id": { "type": "string" },
                    "protocol_revision_id": { "type": "string" },
                    "protocol_content": { "type": "string" },
                    "run_status": { "type": "string", "enum": ["draft", "running", "succeeded", "failed", "cancelled"] },
                    "direction": { "type": "string", "enum": ["input", "output"] },
                    "role": { "type": "string", "enum": ["sample", "reagent", "control", "product", "waste"] },
                    "effect": { "type": "string" },
                    "transformation_group": { "type": "string" },
                    "measurement": { "type": "object" },
                    "evidence": { "type": "object" },
                    "observed_at": { "type": "integer" },
                    "step_ref": { "type": "string" },
                    "description": { "type": "string" },
                    "impact": { "type": "string", "enum": ["none", "minor", "major", "unknown"] },
                    "disposition": { "type": "string" },
                    "occurred_at": { "type": "integer" },
                    "uri": { "type": "string" },
                    "format": { "type": "string" },
                    "size_bytes": { "type": "integer" },
                    "checksum_sha256": { "type": "string" },
                    "origin": { "type": "string" },
                    "manifest": { "type": "object" },
                    "template_kind": { "type": "string", "enum": ["plasmid", "cell_line", "antibody", "kit", "buffer", "mouse"] },
                    "rows": { "type": "array", "items": { "type": "object" } },
                    "operation": { "type": "string", "enum": ["split", "aliquot", "merge", "pool", "passage", "transform"] },
                    "inputs": { "type": "array", "items": { "type": "object" } },
                    "outputs": { "type": "array", "items": { "type": "object" } },
                    "original_event_id": { "type": "string" },
                    "correction": { "type": "object" },
                    "observation_ids": { "type": "array", "items": {"type":"string"} },
                    "criteria": { "type": "object" },
                    "verdict": { "type": "string", "enum": ["pending", "pass", "conditional", "fail", "not_applicable"] },
                    "rationale": { "type": "string" },
                    "conclusion": { "type": "string" },
                    "lesson": { "type": "string" },
                    "evidence_ids": { "type": "array", "items": { "type": "string" } },
                    "expires_at": { "type": "integer" },
                    "metadata": { "type": "object" },
                    "attributes": { "type": "object" },
                    "aliases": { "type": "array", "items": { "type": "object", "properties": { "alias_type": {"type":"string"}, "namespace": {"type":"string"}, "value": {"type":"string"} }, "required": ["alias_type", "value"] } }
                },
                "required": ["action", "registry_id", "command_id"]
            }),
        )
    }

    fn preview(&self, args: &serde_json::Value) -> String {
        args.get("title")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string()
    }

    async fn run(&self, args: &serde_json::Value, env: &dyn ToolEnv) -> ToolResult {
        let action = args
            .get("action")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if action == "move_material_unit" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| format!("lab_transaction requires {name}"))
            };
            let (registry_id, command_id, material_unit_id) =
                match (
                    required("registry_id"),
                    required("command_id"),
                    required("material_unit_id"),
                ) {
                    (Ok(registry_id), Ok(command_id), Ok(material_unit_id)) => {
                        (registry_id, command_id, material_unit_id)
                    }
                    _ => return ToolResult::fail(
                        "move_material_unit requires registry_id, command_id, and material_unit_id",
                    ),
                };
            let expected_revision = match args
                .get("expected_revision")
                .and_then(|value| value.as_i64())
            {
                Some(value) if value > 0 => value,
                _ => {
                    return ToolResult::fail(
                        "move_material_unit requires positive expected_revision",
                    )
                }
            };
            let location_id = args
                .get("location_id")
                .and_then(|value| value.as_str())
                .map(str::to_string);
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(),
                command_id: command_id.clone(),
                transaction_id: None,
                affected_ids: std::iter::once(material_unit_id.clone())
                    .chain(location_id.clone())
                    .collect(),
                before: serde_json::json!({"material_unit_id": material_unit_id}),
                after: serde_json::json!({"material_unit_id": material_unit_id, "location_id": location_id}),
                risk_class: "custody_location".into(),
                assumptions: vec![
                    "The expected revision protects against overwriting a newer move.".into(),
                ],
                missing_data: vec![],
                actions: vec!["confirm".into(), "cancel".into()],
            };
            match env.confirm_domain(&confirmation).await {
                ConfirmDecision::Approved => {}
                ConfirmDecision::Denied { feedback } => {
                    return ToolResult::fail(match feedback {
                        Some(feedback) => format!("Lab transaction denied: {feedback}"),
                        None => "Lab transaction denied".into(),
                    })
                }
            }
            let mut request =
                LabTransactionRequest::new(registry_id, command_id, LabActorKind::Agent);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.material_moves.push(LabMaterialMove {
                material_unit_id,
                location_id,
                expected_revision,
                occurred_at: chrono::Utc::now().timestamp(),
                reason: args
                    .get("reason")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                prior_event_id: None,
            });
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => ToolResult::ok(
                    serde_json::json!({
                        "transaction": result.transaction,
                        "events": result.events,
                        "created_entities": result.created_entities,
                        "idempotent": result.idempotent,
                    })
                    .to_string(),
                ),
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "adjust_material_unit" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| format!("lab_transaction requires {name}"))
            };
            let (registry_id, command_id, material_unit_id, reason) = match (
                required("registry_id"),
                required("command_id"),
                required("material_unit_id"),
                required("reason"),
            ) {
                (Ok(registry_id), Ok(command_id), Ok(material_unit_id), Ok(reason)) => {
                    (registry_id, command_id, material_unit_id, reason)
                }
                _ => return ToolResult::fail("adjust_material_unit requires registry_id, command_id, material_unit_id, and reason"),
            };
            let expected_revision = match args
                .get("expected_revision")
                .and_then(|value| value.as_i64())
            {
                Some(value) if value > 0 => value,
                _ => {
                    return ToolResult::fail(
                        "adjust_material_unit requires positive expected_revision",
                    )
                }
            };
            let quantity: LabQuantity = match args.get("quantity") {
                Some(value) => match serde_json::from_value(value.clone()) {
                    Ok(quantity) => quantity,
                    Err(error) => return ToolResult::fail(format!("Invalid quantity: {error}")),
                },
                None => return ToolResult::fail("adjust_material_unit requires quantity"),
            };
            let availability = args
                .get("availability")
                .and_then(|value| value.as_str())
                .map(str::to_string);
            let lifecycle = args
                .get("lifecycle")
                .and_then(|value| value.as_str())
                .map(str::to_string);
            let identity_state = args
                .get("identity_state")
                .and_then(|value| value.as_str())
                .map(str::to_string);
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(),
                command_id: command_id.clone(),
                transaction_id: None,
                affected_ids: vec![material_unit_id.clone()],
                before: serde_json::json!({"material_unit_id": material_unit_id}),
                after: serde_json::json!({
                    "quantity": quantity,
                    "lifecycle": lifecycle,
                    "availability": availability,
                    "identity_state": identity_state,
                }),
                risk_class: "inventory_balance".into(),
                assumptions: vec![
                    "This is a corrective inventory event and preserves prior history.".into(),
                ],
                missing_data: vec![],
                actions: vec!["confirm".into(), "cancel".into()],
            };
            match env.confirm_domain(&confirmation).await {
                ConfirmDecision::Approved => {}
                ConfirmDecision::Denied { feedback } => {
                    return ToolResult::fail(match feedback {
                        Some(feedback) => format!("Lab transaction denied: {feedback}"),
                        None => "Lab transaction denied".into(),
                    })
                }
            }
            let mut request =
                LabTransactionRequest::new(registry_id, command_id, LabActorKind::Agent);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.material_adjustments.push(LabMaterialAdjustment {
                material_unit_id,
                expected_revision,
                quantity,
                lifecycle,
                availability,
                identity_state,
                occurred_at: chrono::Utc::now().timestamp(),
                reason,
                prior_event_id: None,
            });
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => ToolResult::ok(
                    serde_json::json!({
                        "transaction": result.transaction,
                        "events": result.events,
                        "created_entities": result.created_entities,
                        "idempotent": result.idempotent,
                    })
                    .to_string(),
                ),
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "update_dossier" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| format!("lab_transaction requires {name}"))
            };
            let (registry_id, command_id, entity_id, relative_path, narrative_markdown) = match (
                required("registry_id"),
                required("command_id"),
                required("entity_id"),
                required("relative_path"),
                required("narrative_markdown"),
            ) {
                (Ok(registry_id), Ok(command_id), Ok(entity_id), Ok(relative_path), Ok(narrative_markdown)) => {
                    (registry_id, command_id, entity_id, relative_path, narrative_markdown)
                }
                _ => return ToolResult::fail("update_dossier requires registry_id, command_id, entity_id, relative_path, and narrative_markdown"),
            };
            let extension_json = args
                .get("extension_fields")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}))
                .to_string();
            let expected_revision = args
                .get("expected_document_revision")
                .and_then(|value| value.as_i64());
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(),
                command_id: command_id.clone(),
                transaction_id: None,
                affected_ids: vec![entity_id.clone()],
                before: serde_json::json!({"entity_id": entity_id}),
                after: serde_json::json!({"relative_path": relative_path, "narrative_markdown": narrative_markdown, "extensions": serde_json::from_str::<serde_json::Value>(&extension_json).unwrap_or_else(|_| serde_json::json!({}))}),
                risk_class: "dossier_narrative".into(),
                assumptions: vec!["The database remains authoritative; Markdown is written through a retryable projection outbox.".into()],
                missing_data: vec![],
                actions: vec!["confirm".into(), "cancel".into()],
            };
            match env.confirm_domain(&confirmation).await {
                ConfirmDecision::Approved => {}
                ConfirmDecision::Denied { feedback } => {
                    return ToolResult::fail(match feedback {
                        Some(feedback) => format!("Lab transaction denied: {feedback}"),
                        None => "Lab transaction denied".into(),
                    })
                }
            }
            let mut request =
                LabTransactionRequest::new(registry_id.clone(), command_id, LabActorKind::Agent);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.document_upserts.push(LabDocumentUpsert {
                entity_id,
                relative_path,
                narrative_markdown,
                extension_json,
                expected_revision,
            });
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => {
                    let projection_errors = flush_lab_projections(&self.store, &registry_id).await;
                    ToolResult::ok(
                        serde_json::json!({
                            "transaction": result.transaction,
                            "events": result.events,
                            "created_entities": result.created_entities,
                            "idempotent": result.idempotent,
                            "projection_errors": projection_errors,
                        })
                        .to_string(),
                    )
                }
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "import_dossier" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| format!("lab_transaction requires {name}"))
            };
            let (registry_id, command_id, entity_id, markdown) =
                match (
                    required("registry_id"),
                    required("command_id"),
                    required("entity_id"),
                    required("markdown"),
                ) {
                    (Ok(registry_id), Ok(command_id), Ok(entity_id), Ok(markdown)) => {
                        (registry_id, command_id, entity_id, markdown)
                    }
                    _ => return ToolResult::fail(
                        "import_dossier requires registry_id, command_id, entity_id, and markdown",
                    ),
                };
            let preview = match self
                .store
                .preview_lab_document_import(&entity_id, &markdown)
                .await
            {
                Ok(preview) => preview,
                Err(error) => {
                    return ToolResult::fail(format!("Dossier import preview failed: {error}"))
                }
            };
            if preview.status != "ready_to_import" {
                return ToolResult::fail(
                    serde_json::json!({"status":preview.status,"conflicts":preview.conflicts})
                        .to_string(),
                );
            }
            let document = self.store.get_lab_document(&entity_id).await.ok().flatten();
            let Some(document) = document else {
                return ToolResult::fail("Registered dossier disappeared during import");
            };
            let mut extensions = preview.parsed.extensions;
            if !preview.parsed.unknown_frontmatter.is_empty() {
                if let Some(object) = extensions.as_object_mut() {
                    object.insert(
                        "_unrecognized_frontmatter".into(),
                        serde_json::Value::Object(preview.parsed.unknown_frontmatter),
                    );
                }
            }
            let upsert = LabDocumentUpsert {
                entity_id: entity_id.clone(),
                relative_path: document.relative_path,
                narrative_markdown: preview.parsed.narrative_markdown,
                extension_json: extensions.to_string(),
                expected_revision: Some(preview.parsed.document_revision),
            };
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(),
                command_id: command_id.clone(),
                transaction_id: None,
                affected_ids: vec![entity_id],
                before: serde_json::json!({"document_revision":preview.parsed.document_revision}),
                after: serde_json::to_value(&upsert).unwrap_or_else(|_| serde_json::json!({})),
                risk_class: "dossier_import".into(),
                assumptions: vec![
                    "Identity, entity revision, and document revision were validated by dry-run."
                        .into(),
                ],
                missing_data: vec![],
                actions: vec!["confirm".into(), "cancel".into()],
            };
            if !env.confirm_domain(&confirmation).await.approved() {
                return ToolResult::fail("Dossier import denied");
            }
            let mut request =
                LabTransactionRequest::new(registry_id.clone(), command_id, LabActorKind::Import);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.document_upserts.push(upsert);
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => {
                    let projection_errors = flush_lab_projections(&self.store, &registry_id).await;
                    ToolResult::ok(serde_json::json!({"transaction":result.transaction,"events":result.events,"projection_errors":projection_errors}).to_string())
                }
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "publish_protocol_revision" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            };
            let (Some(registry_id), Some(command_id), Some(protocol_entity_id), Some(content)) = (
                required("registry_id"),
                required("command_id"),
                required("protocol_entity_id"),
                required("protocol_content"),
            ) else {
                return ToolResult::fail("publish_protocol_revision requires registry_id, command_id, protocol_entity_id, and protocol_content");
            };
            let checksum = format!("{:x}", sha2::Sha256::digest(content.as_bytes()));
            let revision = LabProtocolRevisionCreate {
                protocol_entity_id: protocol_entity_id.clone(),
                content,
            };
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(),
                command_id: command_id.clone(),
                transaction_id: None,
                affected_ids: vec![protocol_entity_id],
                before: serde_json::json!({}),
                after: serde_json::json!({"checksum_sha256":checksum,"immutable":true}),
                risk_class: "protocol_publication".into(),
                assumptions: vec![
                    "Published protocol bytes are immutable and content-addressed.".into(),
                ],
                missing_data: vec![],
                actions: vec!["confirm".into(), "cancel".into()],
            };
            if !env.confirm_domain(&confirmation).await.approved() {
                return ToolResult::fail("Protocol publication denied");
            }
            let mut request =
                LabTransactionRequest::new(registry_id, command_id, LabActorKind::Agent);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.protocol_revision_creates.push(revision);
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => ToolResult::ok(serde_json::json!({"transaction":result.transaction,"events":result.events,"checksum_sha256":checksum,"idempotent":result.idempotent}).to_string()),
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "pin_run_protocol" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            };
            let (Some(registry_id), Some(command_id), Some(run_id), Some(protocol_revision_id)) = (
                required("registry_id"),
                required("command_id"),
                required("run_id"),
                required("protocol_revision_id"),
            ) else {
                return ToolResult::fail("pin_run_protocol requires registry_id, command_id, run_id, and protocol_revision_id");
            };
            let revision = match self
                .store
                .get_lab_protocol_revision_by_id(&protocol_revision_id)
                .await
            {
                Ok(Some(revision)) if revision.registry_id == registry_id => revision,
                Ok(_) => {
                    return ToolResult::fail(
                        "Protocol revision is missing or belongs to another registry",
                    )
                }
                Err(error) => return ToolResult::fail(format!("lab_transaction error: {error}")),
            };
            let pin = LabRunProtocolPin {
                run_id: run_id.clone(),
                protocol_revision_id: protocol_revision_id.clone(),
            };
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(),
                command_id: command_id.clone(),
                transaction_id: None,
                affected_ids: vec![run_id, protocol_revision_id],
                before: serde_json::json!({}),
                after: serde_json::json!({"protocol_revision_id":revision.id,"checksum_sha256":revision.checksum_sha256}),
                risk_class: "protocol_pin".into(),
                assumptions: vec![
                    "Run start will use exactly these immutable protocol bytes.".into()
                ],
                missing_data: vec![],
                actions: vec!["confirm".into(), "cancel".into()],
            };
            if !env.confirm_domain(&confirmation).await.approved() {
                return ToolResult::fail("Protocol pin denied");
            }
            let mut request =
                LabTransactionRequest::new(registry_id, command_id, LabActorKind::Agent);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.run_protocol_pins.push(pin);
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => ToolResult::ok(serde_json::json!({"transaction":result.transaction,"events":result.events,"protocol_revision":revision,"idempotent":result.idempotent}).to_string()),
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "create_wet_lab_run" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| format!("lab_transaction requires {name}"))
            };
            let (registry_id, command_id, title) = match (
                required("registry_id"),
                required("command_id"),
                required("title"),
            ) {
                (Ok(registry_id), Ok(command_id), Ok(title)) => (registry_id, command_id, title),
                _ => {
                    return ToolResult::fail(
                        "create_wet_lab_run requires registry_id, command_id, and title",
                    )
                }
            };
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(),
                command_id: command_id.clone(),
                transaction_id: None,
                affected_ids: vec![registry_id.clone()],
                before: serde_json::json!({}),
                after: serde_json::json!({"kind":"wet_lab_run", "title":title}),
                risk_class: "experimental_activity".into(),
                assumptions: vec!["No compute process or execution context will be created.".into()],
                missing_data: vec![],
                actions: vec!["confirm".into(), "cancel".into()],
            };
            if !env.confirm_domain(&confirmation).await.approved() {
                return ToolResult::fail("Wet-lab Run creation denied");
            }
            return match self
                .store
                .create_wet_lab_run(
                    &self.project_id,
                    &registry_id,
                    &command_id,
                    &title,
                    self.frame_id.as_deref(),
                    args.get("operator").and_then(|value| value.as_str()),
                )
                .await
            {
                Ok(run) => ToolResult::ok(serde_json::json!({"wet_lab_run":run}).to_string()),
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "import_bulk_resources" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            };
            let (Some(registry_id), Some(command_id), Some(template)) = (
                required("registry_id"),
                required("command_id"),
                required("template_kind"),
            ) else {
                return ToolResult::fail(
                    "import_bulk_resources requires registry_id, command_id, and template_kind",
                );
            };
            let rows = args
                .get("rows")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            let (entities, row_errors) = build_bulk_entities(&template, &rows);
            if !row_errors.is_empty() {
                return ToolResult::fail(
                    serde_json::json!({
                        "dry_run":true,"can_commit":false,"row_errors":row_errors
                    })
                    .to_string(),
                );
            }
            if entities.is_empty() {
                return ToolResult::fail("Bulk import requires at least one row");
            }
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(),
                command_id: command_id.clone(),
                transaction_id: None,
                affected_ids: vec![registry_id.clone()],
                before: serde_json::json!({}),
                after: serde_json::json!({"template_kind":template,"row_count":entities.len()}),
                risk_class: "bulk_registry_import".into(),
                assumptions: vec![
                    "All rows were validated and will commit atomically in one LabTransaction."
                        .into(),
                ],
                missing_data: vec![],
                actions: vec!["confirm".into(), "cancel".into()],
            };
            if !env.confirm_domain(&confirmation).await.approved() {
                return ToolResult::fail("Bulk resource import denied");
            }
            let mut request =
                LabTransactionRequest::new(registry_id, command_id, LabActorKind::Import);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.entity_creates = entities;
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => ToolResult::ok(serde_json::json!({"transaction":result.transaction,"created_entities":result.created_entities,"events":result.events,"idempotent":result.idempotent}).to_string()),
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "import_output_manifest" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            };
            let (Some(registry_id), Some(command_id), Some(run_id)) = (
                required("registry_id"),
                required("command_id"),
                required("run_id"),
            ) else {
                return ToolResult::fail(
                    "import_output_manifest requires registry_id, command_id, and run_id",
                );
            };
            let rows = args
                .get("rows")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            let (outputs, mut row_errors) = build_output_manifest(&run_id, &rows);
            let mut seen_locations = std::collections::HashSet::new();
            for (index, output) in outputs.iter().enumerate() {
                if let Some(location_id) = output.initial_location_id.as_deref() {
                    if !seen_locations.insert(location_id.to_string()) {
                        row_errors.push(serde_json::json!({"row":index + 1,"field":"location_id","error":"duplicate location in manifest"}));
                    } else if !self
                        .store
                        .lab_location_accepts_new_material(&registry_id, location_id)
                        .await
                        .unwrap_or(false)
                    {
                        row_errors.push(serde_json::json!({"row":index + 1,"field":"location_id","error":"missing, cross-registry, or occupied"}));
                    }
                }
            }
            if !row_errors.is_empty() {
                return ToolResult::fail(
                    serde_json::json!({"dry_run":true,"can_commit":false,"row_errors":row_errors})
                        .to_string(),
                );
            }
            if outputs.is_empty() {
                return ToolResult::fail("Output manifest requires at least one row");
            }
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(), command_id: command_id.clone(), transaction_id: None,
                affected_ids: vec![run_id], before: serde_json::json!({}),
                after: serde_json::json!({"output_count":outputs.len(),"locations":outputs.iter().filter_map(|output| output.initial_location_id.as_deref()).collect::<Vec<_>>()}),
                risk_class: "bulk_experimental_outputs".into(),
                assumptions: vec!["Every output identity, Run edge, quantity, and initial location will commit or roll back together.".into()],
                missing_data: vec![], actions: vec!["confirm".into(), "cancel".into()],
            };
            if !env.confirm_domain(&confirmation).await.approved() {
                return ToolResult::fail("Output manifest import denied");
            }
            let mut request =
                LabTransactionRequest::new(registry_id, command_id, LabActorKind::Import);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.run_output_creates = outputs;
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => ToolResult::ok(serde_json::json!({"transaction":result.transaction,"created_entities":result.created_entities,"events":result.events,"idempotent":result.idempotent}).to_string()),
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "update_wet_lab_run_status" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| format!("lab_transaction requires {name}"))
            };
            let (registry_id, command_id, run_id, status_text) = match (
                required("registry_id"),
                required("command_id"),
                required("run_id"),
                required("run_status"),
            ) {
                (Ok(registry_id), Ok(command_id), Ok(run_id), Ok(status)) => {
                    (registry_id, command_id, run_id, status)
                }
                _ => return ToolResult::fail(
                    "update_wet_lab_run_status requires registry_id, command_id, run_id, and run_status",
                ),
            };
            let status = match status_text.as_str() {
                "draft" => wisp_store::RunStatus::Draft,
                "running" => wisp_store::RunStatus::Running,
                "succeeded" => wisp_store::RunStatus::Succeeded,
                "failed" => wisp_store::RunStatus::Failed,
                "cancelled" => wisp_store::RunStatus::Cancelled,
                _ => return ToolResult::fail("Invalid wet-lab Run status"),
            };
            match self
                .store
                .replay_lab_transaction(&registry_id, &command_id)
                .await
            {
                Ok(Some(result)) => {
                    let stored_request = serde_json::from_str::<LabTransactionRequest>(
                        &result.transaction.request_json,
                    );
                    let same_update = stored_request.as_ref().is_ok_and(|request| {
                        request.project_id.as_deref() == Some(self.project_id.as_str())
                            && request.run_status_updates
                                == vec![LabRunStatusUpdate {
                                    run_id: run_id.clone(),
                                    status,
                                }]
                    });
                    if !same_update {
                        return ToolResult::fail(
                            "lab_transaction command_id was already used for a different request",
                        );
                    }
                    let closeout = serde_json::from_str::<serde_json::Value>(
                        &result.transaction.confirmation_json,
                    )
                    .ok()
                    .and_then(|confirmation| confirmation.get("after").cloned())
                    .and_then(|after| after.get("closeout").cloned())
                    .filter(|value| !value.is_null());
                    return ToolResult::ok(
                        serde_json::json!({
                            "transaction": result.transaction,
                            "events": result.events,
                            "idempotent": true,
                            "run_id": run_id,
                            "status": status,
                            "closeout": closeout,
                        })
                        .to_string(),
                    );
                }
                Ok(None) => {}
                Err(error) => return ToolResult::fail(format!("lab_transaction error: {error}")),
            }
            let wet_run = match self.store.get_wet_lab_run(&run_id).await {
                Ok(Some(run)) if run.registry_id == registry_id => run,
                Ok(_) => {
                    return ToolResult::fail(
                        "Wet-lab Run is missing or belongs to another registry",
                    )
                }
                Err(error) => return ToolResult::fail(format!("lab_transaction error: {error}")),
            };
            let closeout = if status.is_terminal() {
                match self.store.wet_lab_run_closeout_summary(&run_id).await {
                    Ok(summary) => Some(summary),
                    Err(error) => {
                        return ToolResult::fail(format!("lab_transaction error: {error}"))
                    }
                }
            } else {
                None
            };
            let current = match self.store.get_run(&run_id).await {
                Ok(Some(run)) => run,
                Ok(None) => return ToolResult::fail("Run not found"),
                Err(error) => return ToolResult::fail(format!("lab_transaction error: {error}")),
            };
            if current.project_id != self.project_id {
                return ToolResult::fail("Wet-lab Run belongs to another Project");
            }
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(),
                command_id: command_id.clone(),
                transaction_id: None,
                affected_ids: vec![run_id.clone(), wet_run.display_id],
                before: serde_json::json!({"status":current.status}),
                after: serde_json::json!({"status":status,"closeout":closeout}),
                risk_class: if status.is_terminal() {
                    "experimental_closeout".into()
                } else {
                    "experimental_activity".into()
                },
                assumptions: vec![
                    "Run lifecycle remains independent from QC verdicts; closeout issues are advisory and preserved in the confirmation snapshot."
                        .into(),
                ],
                missing_data: closeout
                    .as_ref()
                    .map(|summary| summary.issues.clone())
                    .unwrap_or_default(),
                actions: vec!["confirm".into(), "cancel".into()],
            };
            if !env.confirm_domain(&confirmation).await.approved() {
                return ToolResult::fail("Wet-lab Run status update denied");
            }
            let mut request =
                LabTransactionRequest::new(registry_id, command_id, LabActorKind::Agent);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.run_status_updates.push(LabRunStatusUpdate {
                run_id: run_id.clone(),
                status,
            });
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => ToolResult::ok(
                    serde_json::json!({
                        "transaction": result.transaction,
                        "events": result.events,
                        "idempotent": result.idempotent,
                        "run_id": run_id,
                        "status": status,
                        "closeout": closeout,
                    })
                    .to_string(),
                ),
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "derive_materials" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            };
            let (Some(registry_id), Some(command_id), Some(run_id), Some(operation)) = (
                required("registry_id"),
                required("command_id"),
                required("run_id"),
                required("operation"),
            ) else {
                return ToolResult::fail(
                    "derive_materials requires registry_id, command_id, run_id, and operation",
                );
            };
            let input_values = args
                .get("inputs")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            let output_values = args
                .get("outputs")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            let mut inputs = vec![];
            for (index, value) in input_values.iter().enumerate() {
                let Some(material_unit_id) = value.get("material_unit_id").and_then(|v| v.as_str())
                else {
                    return ToolResult::fail(format!(
                        "derive_materials input row {} requires material_unit_id",
                        index + 1
                    ));
                };
                let quantity = match value.get("quantity") {
                    Some(value) => match serde_json::from_value::<LabQuantity>(value.clone()) {
                        Ok(value) => Some(value),
                        Err(error) => {
                            return ToolResult::fail(format!("Invalid input quantity: {error}"))
                        }
                    },
                    None => None,
                };
                inputs.push(LabRunParticipantCreate {
                    run_id: run_id.clone(),
                    material_unit_id: material_unit_id.into(),
                    direction: "input".into(),
                    role: value
                        .get("role")
                        .and_then(|v| v.as_str())
                        .unwrap_or("sample")
                        .into(),
                    effect: value
                        .get("effect")
                        .and_then(|v| v.as_str())
                        .unwrap_or("transformed")
                        .into(),
                    quantity,
                    transformation_group: None,
                    expected_material_revision: value
                        .get("expected_material_revision")
                        .and_then(|v| v.as_i64()),
                });
            }
            let mut outputs = vec![];
            for (index, value) in output_values.iter().enumerate() {
                let Some(title) = value.get("title").and_then(|v| v.as_str()) else {
                    return ToolResult::fail(format!(
                        "derive_materials output row {} requires title",
                        index + 1
                    ));
                };
                let quantity = match value.get("quantity") {
                    Some(value) => match serde_json::from_value::<LabQuantity>(value.clone()) {
                        Ok(value) => value,
                        Err(error) => {
                            return ToolResult::fail(format!("Invalid output quantity: {error}"))
                        }
                    },
                    None => {
                        return ToolResult::fail(format!(
                            "derive_materials output row {} requires quantity",
                            index + 1
                        ))
                    }
                };
                outputs.push(LabRunOutputCreate {
                    run_id: run_id.clone(),
                    entity: LabEntityCreate {
                        kind: LabEntityKind::MaterialUnit,
                        prefix: value
                            .get("prefix")
                            .and_then(|v| v.as_str())
                            .unwrap_or("SMP")
                            .into(),
                        title: title.into(),
                        subtype: value
                            .get("subtype")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        metadata_json: value
                            .get("metadata")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({}))
                            .to_string(),
                        project_relation: Some("produced_by".into()),
                        resource_definition: None,
                        aliases: vec![],
                        lot: None,
                        material_unit: Some(LabMaterialUnitCreate {
                            lot_id: None,
                            usage_class: "sample".into(),
                            quantity: quantity.clone(),
                            vessel_description: value
                                .get("vessel_description")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                            availability: "available".into(),
                            origin_kind: "prepared".into(),
                        }),
                        location: None,
                        subject: None,
                    },
                    role: value
                        .get("role")
                        .and_then(|v| v.as_str())
                        .unwrap_or("product")
                        .into(),
                    effect: "produced".into(),
                    quantity: Some(quantity),
                    transformation_group: None,
                    initial_location_id: value
                        .get("location_id")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                });
            }
            let derivation = LabMaterialDerivationCreate {
                run_id: run_id.clone(),
                operation,
                inputs,
                outputs,
            };
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(), command_id: command_id.clone(), transaction_id: None,
                affected_ids: vec![run_id], before: serde_json::json!({}),
                after: serde_json::to_value(&derivation).unwrap_or_else(|_| serde_json::json!({})),
                risk_class: "material_derivation".into(), assumptions: vec!["Inputs, new output identities, quantities, and acyclic derivation edges commit atomically.".into()],
                missing_data: vec![], actions: vec!["confirm".into(), "cancel".into()],
            };
            if !env.confirm_domain(&confirmation).await.approved() {
                return ToolResult::fail("Material derivation denied");
            }
            let mut request =
                LabTransactionRequest::new(registry_id, command_id, LabActorKind::Agent);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.derivation_creates.push(derivation);
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => ToolResult::ok(serde_json::json!({"transaction":result.transaction,"events":result.events,"created_entities":result.created_entities,"idempotent":result.idempotent}).to_string()),
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "create_run_output" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| format!("lab_transaction requires {name}"))
            };
            let (registry_id, command_id, run_id, title, prefix) = match (
                required("registry_id"),
                required("command_id"),
                required("run_id"),
                required("title"),
                required("prefix"),
            ) {
                (Ok(registry_id), Ok(command_id), Ok(run_id), Ok(title), Ok(prefix)) => {
                    (registry_id, command_id, run_id, title, prefix)
                }
                _ => return ToolResult::fail(
                    "create_run_output requires registry_id, command_id, run_id, title, and prefix",
                ),
            };
            let quantity = match args.get("quantity") {
                Some(value) => match serde_json::from_value::<LabQuantity>(value.clone()) {
                    Ok(quantity) => quantity,
                    Err(error) => {
                        return ToolResult::fail(format!("Invalid output quantity: {error}"))
                    }
                },
                None => return ToolResult::fail("create_run_output requires quantity"),
            };
            let role = args
                .get("role")
                .and_then(|value| value.as_str())
                .unwrap_or("product")
                .to_string();
            let effect = args
                .get("effect")
                .and_then(|value| value.as_str())
                .unwrap_or("produced")
                .to_string();
            let output = LabRunOutputCreate {
                run_id: run_id.clone(),
                entity: LabEntityCreate {
                    kind: LabEntityKind::MaterialUnit,
                    prefix,
                    title,
                    subtype: args
                        .get("subtype")
                        .and_then(|value| value.as_str())
                        .map(str::to_string),
                    metadata_json: args
                        .get("metadata")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({}))
                        .to_string(),
                    project_relation: Some("produced_by".into()),
                    resource_definition: None,
                    aliases: vec![],
                    lot: None,
                    material_unit: Some(LabMaterialUnitCreate {
                        lot_id: args
                            .get("lot_id")
                            .and_then(|value| value.as_str())
                            .map(str::to_string),
                        usage_class: "sample".into(),
                        quantity: quantity.clone(),
                        vessel_description: args
                            .get("vessel_description")
                            .and_then(|value| value.as_str())
                            .map(str::to_string),
                        availability: args
                            .get("availability")
                            .and_then(|value| value.as_str())
                            .unwrap_or("available")
                            .to_string(),
                        // The participant event below is the authoritative
                        // producing origin; this remains compatible with
                        // pre-output inventory schemas.
                        origin_kind: "prepared".into(),
                    }),
                    location: None,
                    subject: None,
                },
                role,
                effect,
                quantity: Some(quantity),
                transformation_group: args
                    .get("transformation_group")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                initial_location_id: args
                    .get("location_id")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
            };
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(),
                command_id: command_id.clone(),
                transaction_id: None,
                affected_ids: vec![run_id],
                before: serde_json::json!({}),
                after: serde_json::to_value(&output).unwrap_or_else(|_| serde_json::json!({})),
                risk_class: "experimental_output".into(),
                assumptions: vec![
                    "The sample identity and its producing Run edge will commit together.".into(),
                ],
                missing_data: vec![],
                actions: vec!["confirm".into(), "cancel".into()],
            };
            if !env.confirm_domain(&confirmation).await.approved() {
                return ToolResult::fail("Run output creation denied");
            }
            let mut request =
                LabTransactionRequest::new(registry_id, command_id, LabActorKind::Agent);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.run_output_creates.push(output);
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => ToolResult::ok(serde_json::json!({"transaction":result.transaction,"events":result.events,"created_entities":result.created_entities,"idempotent":result.idempotent}).to_string()),
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "record_subject_participant" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            };
            let (
                Some(registry_id),
                Some(command_id),
                Some(run_id),
                Some(subject_id),
                Some(role),
                Some(effect),
            ) = (
                required("registry_id"),
                required("command_id"),
                required("run_id"),
                required("subject_id"),
                required("role"),
                required("effect"),
            )
            else {
                return ToolResult::fail("record_subject_participant requires registry_id, command_id, run_id, subject_id, role, and effect");
            };
            let participant = LabSubjectParticipantCreate {
                run_id: run_id.clone(),
                subject_id: subject_id.clone(),
                role,
                effect,
            };
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(), command_id: command_id.clone(), transaction_id: None,
                affected_ids: vec![run_id, subject_id], before: serde_json::json!({}),
                after: serde_json::to_value(&participant).unwrap_or_else(|_| serde_json::json!({})),
                risk_class: "subject_participation".into(),
                assumptions: vec!["Subject participation records observation or handling and never consumes or changes Subject identity.".into()],
                missing_data: vec![], actions: vec!["confirm".into(), "cancel".into()],
            };
            if !env.confirm_domain(&confirmation).await.approved() {
                return ToolResult::fail("Subject participation recording denied");
            }
            let mut request =
                LabTransactionRequest::new(registry_id, command_id, LabActorKind::Agent);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.subject_participants.push(participant);
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => ToolResult::ok(serde_json::json!({"transaction":result.transaction,"events":result.events,"idempotent":result.idempotent}).to_string()),
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "record_run_participant" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| format!("lab_transaction requires {name}"))
            };
            let (registry_id, command_id, run_id, material_unit_id, direction, role, effect) = match (
                required("registry_id"), required("command_id"), required("run_id"),
                required("material_unit_id"), required("direction"), required("role"), required("effect"),
            ) {
                (Ok(registry_id), Ok(command_id), Ok(run_id), Ok(material_unit_id), Ok(direction), Ok(role), Ok(effect)) =>
                    (registry_id, command_id, run_id, material_unit_id, direction, role, effect),
                _ => return ToolResult::fail("record_run_participant requires registry_id, command_id, run_id, material_unit_id, direction, role, and effect"),
            };
            let quantity: Option<LabQuantity> = match args.get("quantity") {
                Some(value) => match serde_json::from_value(value.clone()) {
                    Ok(quantity) => Some(quantity),
                    Err(error) => {
                        return ToolResult::fail(format!("Invalid participant quantity: {error}"))
                    }
                },
                None => None,
            };
            if direction == "output" {
                return ToolResult::fail(
                    "Use create_run_output so the new sample identity and producing edge commit together",
                );
            }
            let participant = LabRunParticipantCreate {
                run_id: run_id.clone(),
                material_unit_id: material_unit_id.clone(),
                direction,
                role,
                effect,
                quantity,
                transformation_group: args
                    .get("transformation_group")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                expected_material_revision: args
                    .get("expected_revision")
                    .and_then(|value| value.as_i64()),
            };
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(), command_id: command_id.clone(), transaction_id: None,
                affected_ids: vec![run_id, material_unit_id], before: serde_json::json!({}),
                after: serde_json::to_value(&participant).unwrap_or_else(|_| serde_json::json!({})),
                risk_class: "experimental_lineage".into(),
                assumptions: vec!["A consuming participant updates the MaterialUnit balance and lineage atomically.".into()],
                missing_data: vec![], actions: vec!["confirm".into(), "cancel".into()],
            };
            if !env.confirm_domain(&confirmation).await.approved() {
                return ToolResult::fail("Run participant recording denied");
            }
            let mut request =
                LabTransactionRequest::new(registry_id, command_id, LabActorKind::Agent);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.run_participants.push(participant);
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => ToolResult::ok(serde_json::json!({"transaction":result.transaction,"events":result.events,"idempotent":result.idempotent}).to_string()),
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "record_amendment" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            };
            let (Some(registry_id), Some(command_id), Some(original_event_id), Some(reason)) = (
                required("registry_id"),
                required("command_id"),
                required("original_event_id"),
                required("reason"),
            ) else {
                return ToolResult::fail("record_amendment requires registry_id, command_id, original_event_id, and reason");
            };
            let amendment = LabAmendmentCreate {
                original_event_id: original_event_id.clone(),
                reason,
                correction_json: args
                    .get("correction")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}))
                    .to_string(),
            };
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(), command_id: command_id.clone(), transaction_id: None,
                affected_ids: vec![original_event_id], before: serde_json::json!({"history":"immutable"}),
                after: serde_json::to_value(&amendment).unwrap_or_else(|_| serde_json::json!({})),
                risk_class: "closed_record_amendment".into(),
                assumptions: vec!["The original event remains immutable; downstream affected IDs are snapshotted in the Amendment.".into()],
                missing_data: vec![], actions: vec!["confirm".into(), "cancel".into()],
            };
            if !env.confirm_domain(&confirmation).await.approved() {
                return ToolResult::fail("Amendment recording denied");
            }
            let mut request =
                LabTransactionRequest::new(registry_id, command_id, LabActorKind::Agent);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.amendment_creates.push(amendment);
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => ToolResult::ok(serde_json::json!({"transaction":result.transaction,"events":result.events,"idempotent":result.idempotent}).to_string()),
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "attach_data_evidence" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| format!("lab_transaction requires {name}"))
            };
            let (registry_id, command_id, run_id, uri, role) = match (
                required("registry_id"),
                required("command_id"),
                required("run_id"),
                required("uri"),
                required("role"),
            ) {
                (Ok(registry_id), Ok(command_id), Ok(run_id), Ok(uri), Ok(role)) => {
                    (registry_id, command_id, run_id, uri, role)
                }
                _ => return ToolResult::fail(
                    "attach_data_evidence requires registry_id, command_id, run_id, uri, and role",
                ),
            };
            let evidence = LabDataEvidenceCreate {
                owner_project_id: Some(self.project_id.clone()),
                owner_registry_id: None,
                producing_run_id: Some(run_id.clone()),
                role,
                uri,
                format: args
                    .get("format")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                size_bytes: args.get("size_bytes").and_then(|value| value.as_i64()),
                checksum_sha256: args
                    .get("checksum_sha256")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                origin: args
                    .get("origin")
                    .and_then(|value| value.as_str())
                    .unwrap_or("instrument")
                    .to_string(),
                manifest_json: args
                    .get("manifest")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}))
                    .to_string(),
            };
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(), command_id: command_id.clone(), transaction_id: None,
                affected_ids: vec![run_id], before: serde_json::json!({}),
                after: serde_json::to_value(&evidence).unwrap_or_else(|_| serde_json::json!({})),
                risk_class: "data_evidence".into(),
                assumptions: vec!["Only the durable URI and metadata are registered; large raw bytes are not copied.".into()],
                missing_data: if evidence.checksum_sha256.is_none() { vec!["checksum_sha256".into()] } else { vec![] },
                actions: vec!["confirm".into(), "cancel".into()],
            };
            if !env.confirm_domain(&confirmation).await.approved() {
                return ToolResult::fail("Data evidence registration denied");
            }
            let mut request =
                LabTransactionRequest::new(registry_id, command_id, LabActorKind::Agent);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.data_evidence_creates.push(evidence);
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => ToolResult::ok(serde_json::json!({"transaction":result.transaction,"events":result.events,"idempotent":result.idempotent}).to_string()),
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "record_run_deviation" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| format!("lab_transaction requires {name}"))
            };
            let (registry_id, command_id, run_id, description) = match (
                required("registry_id"),
                required("command_id"),
                required("run_id"),
                required("description"),
            ) {
                (Ok(registry_id), Ok(command_id), Ok(run_id), Ok(description)) => {
                    (registry_id, command_id, run_id, description)
                }
                _ => return ToolResult::fail(
                    "record_run_deviation requires registry_id, command_id, run_id, and description",
                ),
            };
            let deviation = LabRunDeviationCreate {
                run_id: run_id.clone(),
                step_ref: args
                    .get("step_ref")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                description,
                impact: args
                    .get("impact")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                disposition: args
                    .get("disposition")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                occurred_at: args
                    .get("occurred_at")
                    .and_then(|value| value.as_i64())
                    .unwrap_or_else(|| chrono::Utc::now().timestamp()),
            };
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(),
                command_id: command_id.clone(),
                transaction_id: None,
                affected_ids: vec![run_id],
                before: serde_json::json!({}),
                after: serde_json::to_value(&deviation)
                    .unwrap_or_else(|_| serde_json::json!({})),
                risk_class: "experimental_deviation".into(),
                assumptions: vec![
                    "This records an off-protocol event without changing the Run lifecycle or QC verdict."
                        .into(),
                ],
                missing_data: vec![],
                actions: vec!["confirm".into(), "cancel".into()],
            };
            if !env.confirm_domain(&confirmation).await.approved() {
                return ToolResult::fail("Run deviation recording denied");
            }
            let mut request =
                LabTransactionRequest::new(registry_id, command_id, LabActorKind::Agent);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.run_deviation_creates.push(deviation);
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => ToolResult::ok(serde_json::json!({"transaction":result.transaction,"events":result.events,"idempotent":result.idempotent}).to_string()),
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "record_qc_observation" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| format!("lab_transaction requires {name}"))
            };
            let (registry_id, command_id, entity_id) = match (
                required("registry_id"),
                required("command_id"),
                required("entity_id"),
            ) {
                (Ok(registry_id), Ok(command_id), Ok(entity_id)) => {
                    (registry_id, command_id, entity_id)
                }
                _ => {
                    return ToolResult::fail(
                        "record_qc_observation requires registry_id, command_id, and entity_id",
                    )
                }
            };
            let measurement = args
                .get("measurement")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let evidence = args
                .get("evidence")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let observed_at = args
                .get("observed_at")
                .and_then(|value| value.as_i64())
                .unwrap_or_else(|| chrono::Utc::now().timestamp());
            let observation = LabQcObservationCreate {
                entity_id: entity_id.clone(),
                run_id: args
                    .get("run_id")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                method_revision_id: None,
                measurement_json: measurement.to_string(),
                evidence_json: evidence.to_string(),
                observed_at,
            };
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(), command_id: command_id.clone(), transaction_id: None,
                affected_ids: vec![entity_id], before: serde_json::json!({}),
                after: serde_json::to_value(&observation).unwrap_or_else(|_| serde_json::json!({})),
                risk_class: "quality_evidence".into(), assumptions: vec!["This records a measurement and evidence; it does not infer or change a QC verdict.".into()],
                missing_data: vec![], actions: vec!["confirm".into(), "cancel".into()],
            };
            if !env.confirm_domain(&confirmation).await.approved() {
                return ToolResult::fail("QC observation recording denied");
            }
            let mut request =
                LabTransactionRequest::new(registry_id, command_id, LabActorKind::Agent);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.qc_observation_creates.push(observation);
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => ToolResult::ok(serde_json::json!({"transaction":result.transaction,"events":result.events,"idempotent":result.idempotent}).to_string()),
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "record_qc_assessment" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| format!("lab_transaction requires {name}"))
            };
            let (registry_id, command_id, entity_id, verdict, rationale) = match (
                required("registry_id"), required("command_id"), required("entity_id"),
                required("verdict"), required("rationale"),
            ) {
                (Ok(registry_id), Ok(command_id), Ok(entity_id), Ok(verdict), Ok(rationale)) =>
                    (registry_id, command_id, entity_id, verdict, rationale),
                _ => return ToolResult::fail("record_qc_assessment requires registry_id, command_id, entity_id, verdict, and rationale"),
            };
            let observation_ids: Vec<String> = match args.get("observation_ids") {
                Some(value) => match serde_json::from_value(value.clone()) {
                    Ok(ids) => ids,
                    Err(error) => {
                        return ToolResult::fail(format!("Invalid observation_ids: {error}"))
                    }
                },
                None => return ToolResult::fail("record_qc_assessment requires observation_ids"),
            };
            let assessment = LabQcAssessmentCreate {
                entity_id: entity_id.clone(),
                observation_ids,
                criteria_json: args
                    .get("criteria")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}))
                    .to_string(),
                verdict,
                rationale,
            };
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(),
                command_id: command_id.clone(),
                transaction_id: None,
                affected_ids: vec![entity_id],
                before: serde_json::json!({}),
                after: serde_json::to_value(&assessment).unwrap_or_else(|_| serde_json::json!({})),
                risk_class: "quality_conclusion".into(),
                assumptions: vec![
                    "This is a user-confirmed quality conclusion over explicit observations."
                        .into(),
                ],
                missing_data: vec![],
                actions: vec!["confirm".into(), "cancel".into()],
            };
            if !env.confirm_domain(&confirmation).await.approved() {
                return ToolResult::fail("QC assessment recording denied");
            }
            let mut request =
                LabTransactionRequest::new(registry_id, command_id, LabActorKind::Agent);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.qc_assessment_creates.push(assessment);
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => ToolResult::ok(serde_json::json!({"transaction":result.transaction,"events":result.events,"idempotent":result.idempotent}).to_string()),
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "confirm_conclusion" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            };
            let (Some(registry_id), Some(command_id), Some(run_id), Some(title), Some(conclusion)) = (
                required("registry_id"),
                required("command_id"),
                required("run_id"),
                required("title"),
                required("conclusion"),
            ) else {
                return ToolResult::fail("confirm_conclusion requires registry_id, command_id, run_id, title, and conclusion");
            };
            let evidence_ids = match args
                .get("evidence_ids")
                .cloned()
                .map(serde_json::from_value::<Vec<String>>)
            {
                Some(Ok(ids)) => ids,
                Some(Err(error)) => {
                    return ToolResult::fail(format!("Invalid evidence_ids: {error}"))
                }
                None => vec![],
            };
            let decision = LabConclusionCreate {
                run_id: run_id.clone(),
                title,
                conclusion,
                evidence_ids,
                lesson: args
                    .get("lesson")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
            };
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(), command_id: command_id.clone(), transaction_id: None,
                affected_ids: vec![run_id], before: serde_json::json!({}),
                after: serde_json::to_value(&decision).unwrap_or_else(|_| serde_json::json!({})),
                risk_class: "scientific_conclusion".into(),
                assumptions: vec!["This user-confirmed Decision remains distinct from raw observations and QC assessments.".into()],
                missing_data: if decision.evidence_ids.is_empty() { vec!["evidence_ids".into()] } else { vec![] },
                actions: vec!["confirm".into(), "cancel".into()],
            };
            if !env.confirm_domain(&confirmation).await.approved() {
                return ToolResult::fail("Conclusion confirmation denied");
            }
            let mut request =
                LabTransactionRequest::new(registry_id, command_id, LabActorKind::Agent);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.conclusion_creates.push(decision);
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => ToolResult::ok(serde_json::json!({"transaction":result.transaction,"events":result.events,"idempotent":result.idempotent}).to_string()),
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        if action == "reserve_material" {
            let required = |name| {
                args.get(name)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| format!("lab_transaction requires {name}"))
            };
            let (registry_id, command_id, run_id, material_unit_id) = match (
                required("registry_id"), required("command_id"), required("run_id"), required("material_unit_id"),
            ) {
                (Ok(registry_id), Ok(command_id), Ok(run_id), Ok(material_unit_id)) =>
                    (registry_id, command_id, run_id, material_unit_id),
                _ => return ToolResult::fail("reserve_material requires registry_id, command_id, run_id, and material_unit_id"),
            };
            let quantity: LabQuantity = match args.get("quantity") {
                Some(value) => match serde_json::from_value(value.clone()) {
                    Ok(quantity) => quantity,
                    Err(error) => {
                        return ToolResult::fail(format!("Invalid reservation quantity: {error}"))
                    }
                },
                None => return ToolResult::fail("reserve_material requires quantity"),
            };
            let reservation = LabReservationCreate {
                run_id: run_id.clone(),
                material_unit_id: material_unit_id.clone(),
                quantity,
                expires_at: args.get("expires_at").and_then(|value| value.as_i64()),
            };
            let confirmation = DomainConfirmationRequest {
                domain: "wet_lab".into(), command_id: command_id.clone(), transaction_id: None,
                affected_ids: vec![run_id, material_unit_id], before: serde_json::json!({}),
                after: serde_json::to_value(&reservation).unwrap_or_else(|_| serde_json::json!({})),
                risk_class: "inventory_reservation".into(),
                assumptions: vec!["The reservation is rejected atomically if active reservations would exceed the measured on-hand balance.".into()],
                missing_data: vec![], actions: vec!["confirm".into(), "cancel".into()],
            };
            if !env.confirm_domain(&confirmation).await.approved() {
                return ToolResult::fail("Material reservation denied");
            }
            let mut request =
                LabTransactionRequest::new(registry_id, command_id, LabActorKind::Agent);
            request.project_id = Some(self.project_id.clone());
            request.confirmation_json =
                serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
            request.reservation_creates.push(reservation);
            return match self.store.commit_lab_transaction(request).await {
                Ok(result) => ToolResult::ok(serde_json::json!({"transaction":result.transaction,"events":result.events,"idempotent":result.idempotent}).to_string()),
                Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
            };
        }
        let required = |name| {
            args.get(name)
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .ok_or_else(|| format!("lab_transaction requires {name}"))
        };
        let (registry_id, command_id, title, prefix) = match (
            required("registry_id"),
            required("command_id"),
            required("title"),
            required("prefix"),
        ) {
            (Ok(registry_id), Ok(command_id), Ok(title), Ok(prefix)) => {
                (registry_id, command_id, title, prefix)
            }
            _ => {
                return ToolResult::fail(
                    "lab_transaction requires registry_id, command_id, title, and prefix",
                )
            }
        };
        let metadata = args
            .get("metadata")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let attributes = args
            .get("attributes")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let aliases: Vec<LabAliasCreate> = match args.get("aliases") {
            Some(value) => match serde_json::from_value(value.clone()) {
                Ok(aliases) => aliases,
                Err(error) => return ToolResult::fail(format!("Invalid lab aliases: {error}")),
            },
            None => vec![],
        };
        let (entity, after, risk_class, assumptions) = match action {
            "create_protocol_source" => (
                LabEntityCreate {
                    kind: LabEntityKind::ProtocolSource, prefix, title: title.clone(), subtype: args.get("subtype").and_then(|value| value.as_str()).map(str::to_string),
                    metadata_json: metadata.to_string(), project_relation: Some("used_by".into()), resource_definition: None,
                    aliases, lot: None, material_unit: None, location: None, subject: None,
                },
                serde_json::json!({"kind":"protocol_source","title":title}),
                "protocol_identity",
                vec!["This creates a mutable protocol source identity; immutable bytes are published separately.".into()],
            ),
            "create_resource_definition" => {
                let category = match required("category") {
                    Ok(category) => category,
                    Err(error) => return ToolResult::fail(error),
                };
                let after = serde_json::json!({"kind":"resource_definition", "title":title, "category":category, "aliases":aliases});
                (LabEntityCreate {
                    kind: LabEntityKind::ResourceDefinition, prefix, title, subtype: args.get("subtype").and_then(|value| value.as_str()).map(str::to_string),
                    metadata_json: metadata.to_string(), project_relation: Some("used_by".into()),
                    resource_definition: Some(LabResourceDefinitionCreate { category, supplier: args.get("supplier").and_then(|value| value.as_str()).map(str::to_string), catalog_number: args.get("catalog_number").and_then(|value| value.as_str()).map(str::to_string), attributes_json: attributes.to_string() }),
                    aliases, lot: None, material_unit: None, location: None, subject: None,
                }, after, "inventory_identity", vec!["This creates a reusable definition, not a physical consumable vial or lot.".into()])
            }
            "create_lot" => {
                let resource_definition_id = match required("resource_definition_id") {
                    Ok(value) => value,
                    Err(error) => return ToolResult::fail(error),
                };
                let lot_number = match required("lot_number") {
                    Ok(value) => value,
                    Err(error) => return ToolResult::fail(error),
                };
                let origin_kind = args
                    .get("origin_kind")
                    .and_then(|value| value.as_str())
                    .unwrap_or("receipt")
                    .to_string();
                let after = serde_json::json!({"kind":"lot", "title":title, "resource_definition_id":resource_definition_id, "lot_number":lot_number, "origin_kind":origin_kind});
                (
                    LabEntityCreate {
                        kind: LabEntityKind::Lot,
                        prefix,
                        title,
                        subtype: None,
                        metadata_json: metadata.to_string(),
                        project_relation: Some("used_by".into()),
                        resource_definition: None,
                        aliases,
                        lot: Some(LabLotCreate {
                            resource_definition_id,
                            supplier: args
                                .get("supplier")
                                .and_then(|value| value.as_str())
                                .map(str::to_string),
                            catalog_number: args
                                .get("catalog_number")
                                .and_then(|value| value.as_str())
                                .map(str::to_string),
                            lot_number,
                            received_at: args.get("received_at").and_then(|value| value.as_i64()),
                            expiry_at: args.get("expiry_at").and_then(|value| value.as_i64()),
                            origin_kind,
                        }),
                        material_unit: None,
                        location: None,
                        subject: None,
                    },
                    after,
                    "inventory_identity",
                    vec!["A lot is a batch identity; it cannot itself be consumed.".into()],
                )
            }
            "create_material_unit" => {
                let usage_class = match required("usage_class") {
                    Ok(value) => value,
                    Err(error) => return ToolResult::fail(error),
                };
                let quantity: LabQuantity = match args.get("quantity") {
                    Some(value) => match serde_json::from_value(value.clone()) {
                        Ok(value) => value,
                        Err(error) => {
                            return ToolResult::fail(format!("Invalid quantity: {error}"))
                        }
                    },
                    None => return ToolResult::fail("lab_transaction requires quantity"),
                };
                let origin_kind = args
                    .get("origin_kind")
                    .and_then(|value| value.as_str())
                    .unwrap_or("receipt")
                    .to_string();
                let availability = args
                    .get("availability")
                    .and_then(|value| value.as_str())
                    .unwrap_or("available")
                    .to_string();
                let lot_id = args
                    .get("lot_id")
                    .and_then(|value| value.as_str())
                    .map(str::to_string);
                let after = serde_json::json!({"kind":"material_unit", "title":title, "lot_id":lot_id, "usage_class":usage_class, "quantity":quantity, "availability":availability, "origin_kind":origin_kind});
                (
                    LabEntityCreate {
                        kind: LabEntityKind::MaterialUnit,
                        prefix,
                        title,
                        subtype: args
                            .get("subtype")
                            .and_then(|value| value.as_str())
                            .map(str::to_string),
                        metadata_json: metadata.to_string(),
                        project_relation: Some("used_by".into()),
                        resource_definition: None,
                        aliases,
                        lot: None,
                        material_unit: Some(LabMaterialUnitCreate {
                            lot_id,
                            usage_class,
                            quantity,
                            vessel_description: args
                                .get("vessel_description")
                                .and_then(|value| value.as_str())
                                .map(str::to_string),
                            availability,
                            origin_kind,
                        }),
                        location: None,
                        subject: None,
                    },
                    after,
                    "inventory_balance",
                    vec!["This creates one independently tracked physical material unit.".into()],
                )
            }
            "create_location" => {
                let location_class = match required("location_class") {
                    Ok(value) => value,
                    Err(error) => return ToolResult::fail(error),
                };
                let parent_location_id = args
                    .get("parent_location_id")
                    .and_then(|value| value.as_str())
                    .map(str::to_string);
                let single_occupancy = args
                    .get("single_occupancy")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                let after = serde_json::json!({
                    "kind":"location", "title":title, "parent_location_id":parent_location_id,
                    "location_class":location_class, "single_occupancy":single_occupancy,
                });
                (
                    LabEntityCreate {
                        kind: LabEntityKind::Location,
                        prefix,
                        title,
                        subtype: None,
                        metadata_json: metadata.to_string(),
                        project_relation: Some("used_by".into()),
                        resource_definition: None,
                        aliases,
                        lot: None,
                        material_unit: None,
                        location: Some(LabLocationCreate {
                            parent_location_id,
                            location_class,
                            single_occupancy,
                        }),
                        subject: None,
                    },
                    after,
                    "custody_location",
                    vec![
                        "This creates an addressable location/container, not a consumable vessel."
                            .into(),
                    ],
                )
            }
            _ => return ToolResult::fail("lab_transaction action is not supported"),
        };
        let confirmation = DomainConfirmationRequest {
            domain: "wet_lab".into(),
            command_id: command_id.clone(),
            transaction_id: None,
            affected_ids: vec![registry_id.clone()],
            before: serde_json::json!({}),
            after,
            risk_class: risk_class.into(),
            assumptions,
            missing_data: vec![],
            actions: vec!["confirm".into(), "cancel".into()],
        };
        match env.confirm_domain(&confirmation).await {
            ConfirmDecision::Approved => {}
            ConfirmDecision::Denied { feedback } => {
                return ToolResult::fail(match feedback {
                    Some(feedback) => format!("Lab transaction denied: {feedback}"),
                    None => "Lab transaction denied".into(),
                })
            }
        }
        let mut request = LabTransactionRequest::new(registry_id, command_id, LabActorKind::Agent);
        request.project_id = Some(self.project_id.clone());
        request.confirmation_json =
            serde_json::to_string(&confirmation).unwrap_or_else(|_| "{}".into());
        request.entity_creates.push(entity);
        match self.store.commit_lab_transaction(request).await {
            Ok(result) => ToolResult::ok(
                serde_json::json!({
                    "transaction": result.transaction,
                    "events": result.events,
                    "created_entities": result.created_entities,
                    "idempotent": result.idempotent,
                })
                .to_string(),
            ),
            Err(error) => ToolResult::fail(format!("lab_transaction error: {error}")),
        }
    }
}

fn parse_entity_kind(value: &str) -> Result<LabEntityKind, String> {
    match value {
        "resource_definition" => Ok(LabEntityKind::ResourceDefinition),
        "lot" => Ok(LabEntityKind::Lot),
        "material_unit" => Ok(LabEntityKind::MaterialUnit),
        "subject" => Ok(LabEntityKind::Subject),
        "location" => Ok(LabEntityKind::Location),
        "protocol_source" => Ok(LabEntityKind::ProtocolSource),
        _ => Err("Unknown lab entity kind".into()),
    }
}

fn lab_bulk_templates() -> serde_json::Value {
    serde_json::json!([
        {"kind":"plasmid","required":["title"],"optional":["alias","backbone","insert"]},
        {"kind":"cell_line","required":["title"],"optional":["alias","organism","genotype"]},
        {"kind":"antibody","required":["title"],"optional":["alias","supplier","catalog_number"]},
        {"kind":"kit","required":["title"],"optional":["alias","supplier","catalog_number"]},
        {"kind":"buffer","required":["title"],"optional":["alias","composition"]},
        {"kind":"mouse","required":["title"],"optional":["alias","strain","sex","date_of_birth"]}
    ])
}

fn build_bulk_entities(
    template: &str,
    rows: &[serde_json::Value],
) -> (Vec<LabEntityCreate>, Vec<serde_json::Value>) {
    let config = match template {
        "plasmid" => Some((LabEntityKind::ResourceDefinition, "PL", "plasmid")),
        "cell_line" => Some((LabEntityKind::ResourceDefinition, "CL", "cell_line")),
        "antibody" => Some((LabEntityKind::ResourceDefinition, "AB", "antibody")),
        "kit" => Some((LabEntityKind::ResourceDefinition, "KIT", "kit")),
        "buffer" => Some((LabEntityKind::ResourceDefinition, "BUF", "buffer")),
        "mouse" => Some((LabEntityKind::Subject, "MOU", "mouse")),
        _ => None,
    };
    let mut entities = vec![];
    let mut errors = vec![];
    let Some((kind, prefix, category)) = config else {
        errors.push(
            serde_json::json!({"row":null,"field":"template_kind","error":"unsupported template"}),
        );
        return (entities, errors);
    };
    for (index, row) in rows.iter().enumerate() {
        let Some(object) = row.as_object() else {
            errors.push(
                serde_json::json!({"row":index + 1,"field":"row","error":"must be an object"}),
            );
            continue;
        };
        let title = object
            .get("title")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .unwrap_or("");
        if title.is_empty() {
            errors.push(serde_json::json!({"row":index + 1,"field":"title","error":"required"}));
            continue;
        }
        let aliases = object
            .get("alias")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                vec![LabAliasCreate {
                    alias_type: "legacy".into(),
                    namespace: None,
                    value: value.trim().into(),
                }]
            })
            .unwrap_or_default();
        entities.push(LabEntityCreate {
            kind,
            prefix: prefix.into(),
            title: title.into(),
            subtype: Some(category.into()),
            metadata_json: serde_json::json!({"template":template,"fields":row}).to_string(),
            project_relation: Some("bulk_import".into()),
            resource_definition: (kind == LabEntityKind::ResourceDefinition).then(|| {
                LabResourceDefinitionCreate {
                    category: category.into(),
                    supplier: object
                        .get("supplier")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    catalog_number: object
                        .get("catalog_number")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    attributes_json: row.to_string(),
                }
            }),
            aliases,
            lot: None,
            material_unit: None,
            location: None,
            subject: (kind == LabEntityKind::Subject).then(|| wisp_store::LabSubjectCreate {
                species: object
                    .get("species")
                    .and_then(|value| value.as_str())
                    .unwrap_or("Mus musculus")
                    .to_string(),
                strain: object
                    .get("strain")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                sex: object
                    .get("sex")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                date_of_birth: object
                    .get("date_of_birth")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                origin_kind: object
                    .get("origin_kind")
                    .and_then(|value| value.as_str())
                    .unwrap_or("legacy_import")
                    .to_string(),
            }),
        });
    }
    (entities, errors)
}

fn build_output_manifest(
    run_id: &str,
    rows: &[serde_json::Value],
) -> (Vec<LabRunOutputCreate>, Vec<serde_json::Value>) {
    let mut outputs = vec![];
    let mut errors = vec![];
    if run_id.trim().is_empty() {
        errors.push(serde_json::json!({"row":null,"field":"run_id","error":"required"}));
        return (outputs, errors);
    }
    for (index, row) in rows.iter().enumerate() {
        let row_number = index + 1;
        let Some(object) = row.as_object() else {
            errors.push(
                serde_json::json!({"row":row_number,"field":"row","error":"must be an object"}),
            );
            continue;
        };
        let title = object
            .get("title")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .unwrap_or("");
        if title.is_empty() {
            errors.push(serde_json::json!({"row":row_number,"field":"title","error":"required"}));
            continue;
        }
        let quantity = match object
            .get("quantity")
            .cloned()
            .map(serde_json::from_value::<LabQuantity>)
        {
            Some(Ok(quantity)) => quantity,
            Some(Err(error)) => {
                errors.push(serde_json::json!({"row":row_number,"field":"quantity","error":error.to_string()}));
                continue;
            }
            None => {
                errors.push(
                    serde_json::json!({"row":row_number,"field":"quantity","error":"required"}),
                );
                continue;
            }
        };
        outputs.push(LabRunOutputCreate {
            run_id: run_id.into(),
            entity: LabEntityCreate {
                kind: LabEntityKind::MaterialUnit,
                prefix: object
                    .get("prefix")
                    .and_then(|value| value.as_str())
                    .unwrap_or("SMP")
                    .into(),
                title: title.into(),
                subtype: object
                    .get("subtype")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                metadata_json: object
                    .get("metadata")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}))
                    .to_string(),
                project_relation: Some("produced_by".into()),
                resource_definition: None,
                aliases: vec![],
                lot: None,
                material_unit: Some(LabMaterialUnitCreate {
                    lot_id: None,
                    usage_class: "sample".into(),
                    quantity: quantity.clone(),
                    vessel_description: object
                        .get("vessel_description")
                        .and_then(|value| value.as_str())
                        .map(str::to_string),
                    availability: object
                        .get("availability")
                        .and_then(|value| value.as_str())
                        .unwrap_or("available")
                        .into(),
                    origin_kind: "prepared".into(),
                }),
                location: None,
                subject: None,
            },
            role: object
                .get("role")
                .and_then(|value| value.as_str())
                .unwrap_or("product")
                .into(),
            effect: "produced".into(),
            quantity: Some(quantity),
            transformation_group: object
                .get("transformation_group")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            initial_location_id: object
                .get("location_id")
                .and_then(|value| value.as_str())
                .map(str::to_string),
        });
    }
    (outputs, errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use wisp_store::LabRegistry;
    use wisp_tools::ToolEvent;

    struct ApprovingEnv(PathBuf);
    #[async_trait::async_trait]
    impl ToolEnv for ApprovingEnv {
        fn project_root(&self) -> &Path {
            &self.0
        }
        async fn confirm(&self, _message: &str) -> bool {
            true
        }
        async fn emit(&self, _event: ToolEvent) {}
    }

    #[tokio::test]
    async fn north_star_conversation_tracks_protocol_inventory_outputs_evidence_qc_and_closeout() {
        let path =
            std::env::temp_dir().join(format!("wisp_lab_tool_{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&path).await.unwrap();
        store
            .create_project("project", "Project", "")
            .await
            .unwrap();
        store
            .create_lab_registry(&LabRegistry::new("lab", "Lab", None).unwrap())
            .await
            .unwrap();
        store
            .link_project_lab_registry("project", "lab")
            .await
            .unwrap();
        store
            .create_frame("conversation", "project", "OPERON", "wisp")
            .await
            .unwrap();
        let env = ApprovingEnv(std::env::temp_dir());
        let create = LabTransactionTool::new(
            store.clone(),
            "project".into(),
            Some("conversation".into()),
        )
        .run(
            &serde_json::json!({
                "action":"create_resource_definition", "registry_id":"lab", "command_id":"create-1",
                "title":"anti-CD3", "prefix":"AB", "category":"antibody",
                "aliases":[{"alias_type":"barcode","value":"BC-1"}]
            }),
            &env,
        )
        .await;
        assert!(create.success, "{}", create.content);
        let definition_id = serde_json::from_str::<serde_json::Value>(&create.content).unwrap()
            ["created_entities"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let lot = LabTransactionTool::new(store.clone(), "project".into(), Some("conversation".into()))
            .run(&serde_json::json!({
                "action":"create_lot","registry_id":"lab","command_id":"lot-1","title":"anti-CD3 lot L42","prefix":"LOT",
                "resource_definition_id":definition_id,"lot_number":"L42","supplier":"BioCo","origin_kind":"receipt"
            }), &env).await;
        assert!(lot.success, "{}", lot.content);
        let lot_id = serde_json::from_str::<serde_json::Value>(&lot.content).unwrap()
            ["created_entities"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let vial = LabTransactionTool::new(store.clone(), "project".into(), Some("conversation".into()))
            .run(&serde_json::json!({
                "action":"create_material_unit","registry_id":"lab","command_id":"vial-1","title":"anti-CD3 vial","prefix":"MAT",
                "lot_id":lot_id,"usage_class":"inventory","origin_kind":"receipt","availability":"available",
                "quantity":{"state":"measured","value":"100","unit":"uL"}
            }), &env).await;
        assert!(vial.success, "{}", vial.content);
        let vial_id = serde_json::from_str::<serde_json::Value>(&vial.content).unwrap()
            ["created_entities"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let wet_run = LabTransactionTool::new(
            store.clone(),
            "project".into(),
            Some("conversation".into()),
        )
        .run(
            &serde_json::json!({
                "action":"create_wet_lab_run", "registry_id":"lab", "command_id":"wet-run-1",
                "title":"Cell staining"
            }),
            &env,
        )
        .await;
        assert!(wet_run.success, "{}", wet_run.content);
        let run_id = store
            .get_conversation_wet_lab_run("project", "conversation")
            .await
            .unwrap()
            .unwrap()
            .wet_lab_run
            .run_id;
        let protocol_source = LabTransactionTool::new(store.clone(), "project".into(), Some("conversation".into()))
            .run(&serde_json::json!({
                "action":"create_protocol_source","registry_id":"lab","command_id":"protocol-source-1",
                "title":"Cell staining protocol","prefix":"PRT"
            }), &env).await;
        assert!(protocol_source.success, "{}", protocol_source.content);
        let protocol_entity_id =
            serde_json::from_str::<serde_json::Value>(&protocol_source.content).unwrap()
                ["created_entities"][0]["id"]
                .as_str()
                .unwrap()
                .to_string();
        let published = LabTransactionTool::new(store.clone(), "project".into(), Some("conversation".into()))
            .run(&serde_json::json!({
                "action":"publish_protocol_revision","registry_id":"lab","command_id":"protocol-revision-1",
                "protocol_entity_id":protocol_entity_id,"protocol_content":"# v1\n1. Wash cells\n2. Stain 20 min"
            }), &env).await;
        assert!(published.success, "{}", published.content);
        let publish_json = serde_json::from_str::<serde_json::Value>(&published.content).unwrap();
        let publish_payload: serde_json::Value =
            serde_json::from_str(publish_json["events"][0]["payload_json"].as_str().unwrap())
                .unwrap();
        let protocol_revision_id = publish_payload["protocol_revision_id"]
            .as_str()
            .unwrap()
            .to_string();
        let pinned = LabTransactionTool::new(
            store.clone(),
            "project".into(),
            Some("conversation".into()),
        )
        .run(
            &serde_json::json!({
                "action":"pin_run_protocol","registry_id":"lab","command_id":"pin-protocol-1",
                "run_id":run_id,"protocol_revision_id":protocol_revision_id
            }),
            &env,
        )
        .await;
        assert!(pinned.success, "{}", pinned.content);
        let deviation =
            LabTransactionTool::new(store.clone(), "project".into(), Some("conversation".into()))
                .run(
                    &serde_json::json!({
                        "action":"record_run_deviation", "registry_id":"lab",
                        "command_id":"deviation-1", "run_id":run_id,
                        "step_ref":"wash-2", "description":"Wash extended by two minutes",
                        "impact":"minor"
                    }),
                    &env,
                )
                .await;
        assert!(deviation.success, "{}", deviation.content);
        let deviations = store.list_lab_run_deviations(&run_id).await.unwrap();
        assert_eq!(deviations.len(), 1);
        assert_eq!(deviations[0].step_ref.as_deref(), Some("wash-2"));
        assert_eq!(deviations[0].impact, "minor");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&deviation.content).unwrap()["events"][0]
                ["kind"],
            "run_deviation_recorded"
        );
        let consumed = LabTransactionTool::new(store.clone(), "project".into(), Some("conversation".into()))
            .run(&serde_json::json!({
                "action":"record_run_participant","registry_id":"lab","command_id":"consume-vial","run_id":run_id,
                "material_unit_id":vial_id,"direction":"input","role":"reagent","effect":"partially_consumed",
                "expected_revision":1,"quantity":{"state":"measured","value":"20","unit":"uL"}
            }), &env).await;
        assert!(consumed.success, "{}", consumed.content);
        assert_eq!(
            store
                .get_lab_material_unit(&vial_id)
                .await
                .unwrap()
                .unwrap()
                .quantity
                .value
                .as_deref(),
            Some("80")
        );
        let location = LabTransactionTool::new(store.clone(), "project".into(), Some("conversation".into()))
            .run(&serde_json::json!({
                "action":"create_location","registry_id":"lab","command_id":"sample-location","title":"Freezer A / Box 1 / A1",
                "prefix":"LOC","location_class":"slot","single_occupancy":true
            }), &env).await;
        assert!(location.success, "{}", location.content);
        let location_id = serde_json::from_str::<serde_json::Value>(&location.content).unwrap()
            ["created_entities"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let output = LabTransactionTool::new(store.clone(), "project".into(), Some("conversation".into()))
            .run(&serde_json::json!({
                "action":"create_run_output","registry_id":"lab","command_id":"output-1","run_id":run_id,
                "title":"Stained cells","prefix":"SMP","subtype":"stained_cells","location_id":location_id,
                "quantity":{"state":"measured","value":"80","unit":"uL"}
            }), &env).await;
        assert!(output.success, "{}", output.content);
        let output_id = serde_json::from_str::<serde_json::Value>(&output.content).unwrap()
            ["created_entities"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let evidence = LabTransactionTool::new(
            store.clone(),
            "project".into(),
            Some("conversation".into()),
        )
        .run(
            &serde_json::json!({
                "action":"attach_data_evidence", "registry_id":"lab",
                "command_id":"evidence-1", "run_id":run_id,
                "role":"raw_data", "uri":"s3://instrument/run-1/manifest.json",
                "format":"application/json", "size_bytes":128,
                "checksum_sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "manifest":{"files":["sample-1.fastq.gz"]}
            }),
            &env,
        )
        .await;
        assert!(evidence.success, "{}", evidence.content);
        let evidence_rows = store.list_lab_run_data_evidence(&run_id).await.unwrap();
        assert_eq!(evidence_rows.len(), 1);
        assert_eq!(evidence_rows[0].display_id, "DAT-000001");
        assert_eq!(evidence_rows[0].role, "raw_data");
        let observation = LabTransactionTool::new(store.clone(), "project".into(), Some("conversation".into()))
            .run(&serde_json::json!({
                "action":"record_qc_observation","registry_id":"lab","command_id":"qc-observation-1","entity_id":output_id,"run_id":run_id,
                "measurement":{"viability_percent":92},"evidence":{"data_evidence_id":evidence_rows[0].id}
            }), &env).await;
        assert!(observation.success, "{}", observation.content);
        let observation_json =
            serde_json::from_str::<serde_json::Value>(&observation.content).unwrap();
        let observation_payload: serde_json::Value = serde_json::from_str(
            observation_json["events"][0]["payload_json"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        let observation_id = observation_payload["observation_id"]
            .as_str()
            .unwrap()
            .to_string();
        let assessment = LabTransactionTool::new(store.clone(), "project".into(), Some("conversation".into()))
            .run(&serde_json::json!({
                "action":"record_qc_assessment","registry_id":"lab","command_id":"qc-assessment-1","entity_id":output_id,
                "observation_ids":[observation_id],"criteria":{"viability_percent":{"gte":90}},"verdict":"pass","rationale":"Meets viability threshold"
            }), &env).await;
        assert!(assessment.success, "{}", assessment.content);
        let conclusion = LabTransactionTool::new(store.clone(), "project".into(), Some("conversation".into()))
            .run(&serde_json::json!({
                "action":"confirm_conclusion","registry_id":"lab","command_id":"conclusion-1","run_id":run_id,
                "title":"Staining succeeded","conclusion":"The output meets the viability criterion and is suitable for downstream analysis.",
                "evidence_ids":[observation_id,evidence_rows[0].id],"lesson":"Keep wash duration within the pinned protocol next time."
            }), &env).await;
        assert!(conclusion.success, "{}", conclusion.content);
        let provenance =
            LabQueryTool::new(store.clone(), "project".into(), Some("conversation".into()))
                .run(
                    &serde_json::json!({"action":"run_provenance","run_id":run_id}),
                    &env,
                )
                .await;
        assert!(provenance.success, "{}", provenance.content);
        let provenance_json =
            serde_json::from_str::<serde_json::Value>(&provenance.content).unwrap();
        assert_eq!(
            provenance_json["provenance"]["raw_evidence"][0]["display_id"],
            "DAT-000001"
        );
        assert_eq!(
            provenance_json["provenance"]["deviations"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            provenance_json["provenance"]["observations"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            provenance_json["provenance"]["assessments"][0]["verdict"],
            "pass"
        );
        assert_eq!(
            provenance_json["provenance"]["decisions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            provenance_json["provenance"]["participants"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let closeout =
            LabQueryTool::new(store.clone(), "project".into(), Some("conversation".into()))
                .run(
                    &serde_json::json!({"action":"run_closeout","run_id":run_id}),
                    &env,
                )
                .await;
        assert!(closeout.success, "{}", closeout.content);
        let closeout_json = serde_json::from_str::<serde_json::Value>(&closeout.content).unwrap();
        assert_eq!(closeout_json["closeout"]["deviation_count"], 1);
        assert_eq!(closeout_json["closeout"]["data_evidence_count"], 1);
        assert_eq!(
            closeout_json["closeout"]["issues"],
            serde_json::json!(["deviations_without_disposition"])
        );
        for (command_id, status) in [("start-run", "running"), ("close-run", "succeeded")] {
            let updated = LabTransactionTool::new(
                store.clone(),
                "project".into(),
                Some("conversation".into()),
            )
            .run(
                &serde_json::json!({
                    "action":"update_wet_lab_run_status", "registry_id":"lab",
                    "command_id":command_id, "run_id":run_id, "run_status":status
                }),
                &env,
            )
            .await;
            assert!(updated.success, "{}", updated.content);
            if status == "succeeded" {
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&updated.content).unwrap()
                        ["closeout"]["deviation_count"],
                    1
                );
            }
        }
        let close_retry =
            LabTransactionTool::new(store.clone(), "project".into(), Some("conversation".into()))
                .run(
                    &serde_json::json!({
                        "action":"update_wet_lab_run_status", "registry_id":"lab",
                        "command_id":"close-run", "run_id":run_id, "run_status":"succeeded"
                    }),
                    &env,
                )
                .await;
        assert!(close_retry.success, "{}", close_retry.content);
        let retry_json = serde_json::from_str::<serde_json::Value>(&close_retry.content).unwrap();
        assert_eq!(retry_json["idempotent"], true);
        assert_eq!(retry_json["events"][0]["kind"], "run_status_changed");
        assert_eq!(retry_json["closeout"]["deviation_count"], 1);
        let rejected =
            LabTransactionTool::new(store.clone(), "project".into(), Some("conversation".into()))
                .run(
                    &serde_json::json!({
                        "action":"record_run_deviation", "registry_id":"lab",
                        "command_id":"deviation-after-close", "run_id":run_id,
                        "description":"Late mutation"
                    }),
                    &env,
                )
                .await;
        assert!(!rejected.success);
        assert!(
            rejected.content.contains("cannot be edited"),
            "{}",
            rejected.content
        );
        let amendment = LabTransactionTool::new(
            store.clone(),
            "project".into(),
            Some("conversation".into()),
        )
        .run(
            &serde_json::json!({
                "action":"record_amendment","registry_id":"lab","command_id":"amend-deviation",
                "original_event_id":deviations[0].established_event_id,
                "reason":"Correct the reported impact after notebook review",
                "correction":{"impact":"major"}
            }),
            &env,
        )
        .await;
        assert!(amendment.success, "{}", amendment.content);
        let amendments = store.list_lab_run_amendments(&run_id).await.unwrap();
        assert_eq!(amendments.len(), 1);
        assert_eq!(amendments[0].display_id, "AMD-000001");
        assert!(amendments[0].affected_ids.contains(&run_id));
        assert_eq!(
            store.list_lab_run_deviations(&run_id).await.unwrap()[0].impact,
            "minor",
            "Amendment must not rewrite the original closed record"
        );
        let queried =
            LabQueryTool::new(store.clone(), "project".into(), Some("conversation".into()))
                .run(
                    &serde_json::json!({
                        "action":"find_alias", "registry_id":"lab", "alias":"BC-1"
                    }),
                    &env,
                )
                .await;
        assert!(queried.success, "{}", queried.content);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&queried.content).unwrap()["aliases"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let conversation = LabQueryTool::new(store, "project".into(), Some("conversation".into()))
            .run(&serde_json::json!({"action":"conversation_run"}), &env)
            .await;
        assert!(conversation.success, "{}", conversation.content);
        assert!(
            serde_json::from_str::<serde_json::Value>(&conversation.content).unwrap()
                ["conversation_wet_lab_run"]["wet_lab_run"]["run_id"]
                .is_string()
        );
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn committed_dossier_projection_writes_only_inside_registry_root() {
        let base =
            std::env::temp_dir().join(format!("wisp_lab_projection_{}", uuid::Uuid::new_v4()));
        let db = base.join("ledger.sqlite");
        let root = base.join("registry");
        let store = Store::open(&db).await.unwrap();
        store
            .create_lab_registry(
                &LabRegistry::new("lab", "Lab", Some(root.to_string_lossy().into_owned())).unwrap(),
            )
            .await
            .unwrap();
        let entity = store
            .create_lab_entity(
                "lab",
                wisp_store::LabEntityKind::ResourceDefinition,
                "AB",
                "anti-CD3",
                None,
                None,
            )
            .await
            .unwrap();
        let mut request = wisp_store::LabTransactionRequest::new(
            "lab",
            "dossier",
            wisp_store::LabActorKind::User,
        );
        request
            .document_upserts
            .push(wisp_store::LabDocumentUpsert {
                entity_id: entity.id.clone(),
                relative_path: "resources/anti-cd3.md".into(),
                narrative_markdown: "# Notes".into(),
                extension_json: "{}".into(),
                expected_revision: None,
            });
        store.commit_lab_transaction(request).await.unwrap();
        assert!(flush_lab_projections(&store, "lab").await.is_empty());
        assert!(root.join("resources/anti-cd3.md").is_file());
        assert!(store.list_lab_projection_outbox().await.unwrap().is_empty());
        let document = store.get_lab_document(&entity.id).await.unwrap().unwrap();
        let mut replacement = wisp_store::LabTransactionRequest::new(
            "lab",
            "dossier-replacement",
            wisp_store::LabActorKind::User,
        );
        replacement
            .document_upserts
            .push(wisp_store::LabDocumentUpsert {
                entity_id: entity.id.clone(),
                relative_path: "resources/anti-cd3.md".into(),
                narrative_markdown: "# Replacement notes".into(),
                extension_json: "{}".into(),
                expected_revision: Some(document.revision),
            });
        store.commit_lab_transaction(replacement).await.unwrap();
        assert!(flush_lab_projections(&store, "lab").await.is_empty());
        assert!(std::fs::read_to_string(root.join("resources/anti-cd3.md"))
            .unwrap()
            .contains("Replacement notes"));
        assert!(write_projection_file(&root, "../escaped.md", "x").is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn bulk_templates_report_row_errors_before_atomic_import() {
        let path = std::env::temp_dir().join(format!(
            "wisp_lab_bulk_tool_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        store
            .create_project("project", "Project", "")
            .await
            .unwrap();
        store
            .create_lab_registry(&LabRegistry::new("lab", "Lab", None).unwrap())
            .await
            .unwrap();
        store
            .link_project_lab_registry("project", "lab")
            .await
            .unwrap();
        let env = ApprovingEnv(std::env::temp_dir());
        let tool = LabTransactionTool::new(store.clone(), "project".into(), None);
        let rejected = tool
            .run(
                &serde_json::json!({
                    "action":"import_bulk_resources","registry_id":"lab","command_id":"bad-bulk",
                    "template_kind":"antibody","rows":[{"title":"anti-CD3"},{}]
                }),
                &env,
            )
            .await;
        assert!(!rejected.success);
        assert!(rejected.content.contains("row_errors"));
        assert!(store
            .list_lab_entities("lab", Some(LabEntityKind::ResourceDefinition))
            .await
            .unwrap()
            .is_empty());
        let imported = tool
            .run(
                &serde_json::json!({
                    "action":"import_bulk_resources","registry_id":"lab","command_id":"good-bulk",
                    "template_kind":"antibody","rows":[
                        {"title":"anti-CD3","supplier":"BioCo"},
                        {"title":"anti-CD28","alias":"legacy-CD28"}
                    ]
                }),
                &env,
            )
            .await;
        assert!(imported.success, "{}", imported.content);
        assert_eq!(
            store
                .list_lab_entities("lab", Some(LabEntityKind::ResourceDefinition))
                .await
                .unwrap()
                .len(),
            2
        );
        let mouse_import = tool
            .run(
                &serde_json::json!({
                    "action":"import_bulk_resources","registry_id":"lab","command_id":"mouse-bulk",
                    "template_kind":"mouse","rows":[{
                        "title":"M23-01","strain":"C57BL/6J","sex":"female",
                        "date_of_birth":"2026-05-01","origin_kind":"receipt"
                    }]
                }),
                &env,
            )
            .await;
        assert!(mouse_import.success, "{}", mouse_import.content);
        let subject_entity = store
            .list_lab_entities("lab", Some(LabEntityKind::Subject))
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let subject = store
            .get_lab_subject(&subject_entity.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(subject.strain.as_deref(), Some("C57BL/6J"));
        assert_eq!(subject.origin_kind, "receipt");
        let run = store
            .create_wet_lab_run(
                "project",
                "lab",
                "subject-run",
                "Mouse observation",
                None,
                None,
            )
            .await
            .unwrap();
        let participation = tool
            .run(
                &serde_json::json!({
                    "action":"record_subject_participant","registry_id":"lab","command_id":"subject-participation",
                    "run_id":run.run_id,"subject_id":subject_entity.id,"role":"experimental_subject","effect":"observed"
                }),
                &env,
            )
            .await;
        assert!(participation.success, "{}", participation.content);
        assert_eq!(
            store
                .list_wet_lab_subject_participants(&run.run_id)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .get_lab_entity(&subject_entity.id)
                .await
                .unwrap()
                .unwrap()
                .revision,
            1,
            "participation must not consume or mutate the Subject"
        );
        let mut location_ids = vec![];
        for (command_id, title) in [("loc-a1", "Box A / A1"), ("loc-a2", "Box A / A2")] {
            let created = tool
                .run(
                    &serde_json::json!({
                        "action":"create_location","registry_id":"lab","command_id":command_id,
                        "title":title,"prefix":"LOC","location_class":"slot","single_occupancy":true
                    }),
                    &env,
                )
                .await;
            assert!(created.success, "{}", created.content);
            location_ids.push(
                serde_json::from_str::<serde_json::Value>(&created.content).unwrap()
                    ["created_entities"][0]["id"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            );
        }
        let bad_manifest = tool.run(&serde_json::json!({
            "action":"import_output_manifest","registry_id":"lab","command_id":"bad-output-manifest","run_id":run.run_id,
            "rows":[
                {"title":"Sample A","quantity":{"state":"measured","value":"5","unit":"uL"},"location_id":location_ids[0]},
                {"title":"Sample B","quantity":{"state":"measured","value":"5","unit":"uL"},"location_id":location_ids[0]}
            ]
        }), &env).await;
        assert!(!bad_manifest.success);
        assert!(store
            .list_lab_entities("lab", Some(LabEntityKind::MaterialUnit))
            .await
            .unwrap()
            .is_empty());
        let manifest = tool.run(&serde_json::json!({
            "action":"import_output_manifest","registry_id":"lab","command_id":"good-output-manifest","run_id":run.run_id,
            "rows":[
                {"title":"Sample A","quantity":{"state":"measured","value":"5","unit":"uL"},"location_id":location_ids[0]},
                {"title":"Sample B","quantity":{"state":"measured","value":"5","unit":"uL"},"location_id":location_ids[1]}
            ]
        }), &env).await;
        assert!(manifest.success, "{}", manifest.content);
        let manifest_json = serde_json::from_str::<serde_json::Value>(&manifest.content).unwrap();
        let output_ids = manifest_json["created_entities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entity| entity["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            store
                .get_material_location(&output_ids[0])
                .await
                .unwrap()
                .as_deref(),
            Some(location_ids[0].as_str())
        );
        let labels = LabQueryTool::new(store.clone(), "project".into(), None)
            .run(
                &serde_json::json!({"action":"render_labels","entity_ids":output_ids}),
                &env,
            )
            .await;
        assert!(labels.success, "{}", labels.content);
        assert!(labels.content.contains("<svg"));
        assert!(labels.content.contains("SMP-000001"));
        let _ = std::fs::remove_file(path);
    }
}

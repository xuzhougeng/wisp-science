use super::{
    lab_entity_from_row, lab_event_from_row, lab_registry_from_row, lab_transaction_from_row,
    LabActorKind, LabAlias, LabAliasCreate, LabAmendment, LabConversationRun, LabDataEvidence,
    LabDocument, LabDocumentUpsert, LabEntity, LabEntityCreate, LabEntityKind, LabEvent,
    LabLocation, LabLocationCreate, LabLot, LabLotCreate, LabMaterialUnit, LabMaterialUnitCreate,
    LabProtocolRevision, LabQcAssessment, LabQcObservation, LabQuantity, LabQuantityState,
    LabRegistry, LabReservation, LabResourceDefinition, LabRunCloseoutSummary, LabRunDeviation,
    LabRunParticipant, LabRunParticipantCreate, LabRunProvenance, LabSubject,
    LabSubjectParticipant, LabTransaction, LabTransactionRequest, LabTransactionResult,
    LabTransactionStatus, LabWetRun, RunRecord, RunStatus, Store,
};
use anyhow::Result;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{Row, Sqlite, Transaction};
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredReceipt {
    created_entities: Vec<LabEntity>,
    event_ids: Vec<String>,
}

impl Store {
    pub async fn list_project_registered_dossier_paths(
        &self,
        project_id: &str,
    ) -> Result<Vec<std::path::PathBuf>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT r.root_path,d.relative_path FROM project_lab_registries p \
             JOIN lab_registries r ON r.id=p.registry_id JOIN lab_documents d ON d.registry_id=r.id \
             WHERE p.project_id=? AND r.root_path IS NOT NULL",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(root, relative)| std::path::PathBuf::from(root).join(relative))
            .collect())
    }

    pub async fn list_project_lab_events_since(
        &self,
        project_id: &str,
        since: i64,
    ) -> Result<Vec<LabEvent>> {
        let rows = sqlx::query(
            "SELECT id,registry_id,project_id,transaction_id,sequence,entity_id,prior_event_id,kind,schema_version,payload_json,occurred_at,recorded_at,expected_revision,resulting_revision,reason \
             FROM lab_events WHERE project_id=? AND occurred_at>=? ORDER BY occurred_at DESC,recorded_at DESC,id DESC LIMIT 200",
        )
        .bind(project_id).bind(since).fetch_all(&self.pool).await?;
        rows.into_iter().map(lab_event_from_row).collect()
    }
    pub async fn create_lab_registry(&self, registry: &LabRegistry) -> Result<()> {
        registry.validate()?;
        sqlx::query(
            "INSERT INTO lab_registries(id,name,root_path,created_at,updated_at) VALUES(?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, root_path=excluded.root_path, \
             updated_at=excluded.updated_at",
        )
        .bind(&registry.id)
        .bind(&registry.name)
        .bind(registry.root_path.as_deref())
        .bind(registry.created_at)
        .bind(registry.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_lab_registry(&self, id: &str) -> Result<Option<LabRegistry>> {
        let row = sqlx::query(
            "SELECT id,name,root_path,created_at,updated_at FROM lab_registries WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(lab_registry_from_row).transpose()
    }

    pub async fn link_project_lab_registry(
        &self,
        project_id: &str,
        registry_id: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO project_lab_registries(project_id,registry_id,created_at) VALUES(?,?,?)",
        )
        .bind(project_id)
        .bind(registry_id)
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_project_lab_registries(&self, project_id: &str) -> Result<Vec<LabRegistry>> {
        let rows = sqlx::query(
            "SELECT r.id,r.name,r.root_path,r.created_at,r.updated_at \
             FROM lab_registries r JOIN project_lab_registries pr ON pr.registry_id=r.id \
             WHERE pr.project_id=? ORDER BY r.created_at ASC, r.id ASC",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(lab_registry_from_row).collect()
    }

    pub async fn create_lab_entity(
        &self,
        registry_id: &str,
        kind: LabEntityKind,
        prefix: &str,
        title: &str,
        subtype: Option<&str>,
        metadata_json: Option<&str>,
    ) -> Result<LabEntity> {
        if matches!(
            kind,
            LabEntityKind::Lot | LabEntityKind::MaterialUnit | LabEntityKind::Location
        ) {
            anyhow::bail!("Typed lots, material units, and locations must be created through LabTransaction fields");
        }
        let mut request = LabTransactionRequest::new(
            registry_id,
            format!("system-create-{}", uuid::Uuid::new_v4()),
            LabActorKind::System,
        );
        request.entity_creates.push(LabEntityCreate {
            kind,
            prefix: prefix.to_string(),
            title: title.to_string(),
            subtype: subtype.map(str::to_owned),
            metadata_json: metadata_json.unwrap_or("{}").to_string(),
            project_relation: None,
            resource_definition: None,
            aliases: vec![],
            lot: None,
            material_unit: None,
            location: None,
            subject: None,
        });
        self.commit_lab_transaction(request)
            .await?
            .created_entities
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Lab entity transaction returned no entity"))
    }

    pub async fn get_lab_entity(&self, id: &str) -> Result<Option<LabEntity>> {
        let row = sqlx::query(
            "SELECT id,registry_id,display_id,kind,subtype,title,revision,metadata_json,created_at,updated_at \
             FROM lab_entities WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(lab_entity_from_row).transpose()
    }

    pub async fn list_lab_entities(
        &self,
        registry_id: &str,
        kind: Option<LabEntityKind>,
    ) -> Result<Vec<LabEntity>> {
        let rows = if let Some(kind) = kind {
            sqlx::query(
                "SELECT id,registry_id,display_id,kind,subtype,title,revision,metadata_json,created_at,updated_at \
                 FROM lab_entities WHERE registry_id=? AND kind=? ORDER BY created_at ASC, id ASC",
            )
            .bind(registry_id)
            .bind(kind.as_str())
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT id,registry_id,display_id,kind,subtype,title,revision,metadata_json,created_at,updated_at \
                 FROM lab_entities WHERE registry_id=? ORDER BY created_at ASC, id ASC",
            )
            .bind(registry_id)
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(lab_entity_from_row).collect()
    }

    pub async fn link_lab_entity_to_project(
        &self,
        entity_id: &str,
        project_id: &str,
        relation: &str,
    ) -> Result<()> {
        let relation = relation.trim();
        if relation.is_empty() {
            anyhow::bail!("Lab entity project relation is required");
        }
        let entity = self
            .get_lab_entity(entity_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Lab entity not found"))?;
        let linked: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM project_lab_registries WHERE project_id=? AND registry_id=?",
        )
        .bind(project_id)
        .bind(&entity.registry_id)
        .fetch_optional(&self.pool)
        .await?;
        if linked.is_none() {
            anyhow::bail!("Project is not linked to the lab entity's registry");
        }
        sqlx::query(
            "INSERT OR IGNORE INTO lab_entity_projects(entity_id,project_id,relation,created_at) VALUES(?,?,?,?)",
        )
        .bind(entity_id)
        .bind(project_id)
        .bind(relation)
        .bind(chrono::Utc::now().timestamp())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn commit_lab_transaction(
        &self,
        request: LabTransactionRequest,
    ) -> Result<LabTransactionResult> {
        request.validate()?;
        for create in &request.entity_creates {
            validate_display_prefix(&create.prefix)?;
        }
        let request_json = serde_json::to_string(&request)?;
        let mut tx = self.pool.begin().await?;
        ensure_registry_exists(&mut tx, &request.registry_id).await?;
        if let Some(project_id) = request.project_id.as_deref() {
            ensure_project_registry_link(&mut tx, project_id, &request.registry_id).await?;
        }

        let now = chrono::Utc::now().timestamp();
        let candidate_id = uuid::Uuid::new_v4().to_string();
        let candidate_display_id =
            allocate_display_id(&mut tx, &request.registry_id, "TXN").await?;
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO lab_transactions(\
                id,display_id,registry_id,project_id,command_id,schema_version,actor_kind,actor_ref,\
                confirmation_json,request_json,receipt_json,status,created_at,committed_at\
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&candidate_id)
        .bind(&candidate_display_id)
        .bind(&request.registry_id)
        .bind(request.project_id.as_deref())
        .bind(&request.command_id)
        .bind(request.schema_version)
        .bind(request.actor_kind.as_str())
        .bind(request.actor_ref.as_deref())
        .bind(&request.confirmation_json)
        .bind(&request_json)
        .bind("{}")
        .bind(LabTransactionStatus::Committed.as_str())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        if inserted.rows_affected() == 0 {
            let existing =
                get_transaction_by_command_tx(&mut tx, &request.registry_id, &request.command_id)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("Lab transaction command conflict"))?;
            if existing.request_json != request_json {
                anyhow::bail!(
                    "Lab transaction command_id was already used for a different request"
                );
            }
            let events = list_lab_events_tx(&mut tx, &existing.id).await?;
            let receipt: StoredReceipt =
                serde_json::from_str(&existing.receipt_json).unwrap_or(StoredReceipt {
                    created_entities: vec![],
                    event_ids: events.iter().map(|event| event.id.clone()).collect(),
                });
            tx.commit().await?;
            return Ok(LabTransactionResult {
                transaction: existing,
                events,
                created_entities: receipt.created_entities,
                idempotent: true,
            });
        }

        let mut transaction = LabTransaction {
            id: candidate_id,
            display_id: candidate_display_id,
            registry_id: request.registry_id.clone(),
            project_id: request.project_id.clone(),
            command_id: request.command_id.clone(),
            schema_version: request.schema_version,
            actor_kind: request.actor_kind,
            actor_ref: request.actor_ref.clone(),
            confirmation_json: request.confirmation_json.clone(),
            request_json,
            receipt_json: "{}".into(),
            status: LabTransactionStatus::Committed,
            created_at: now,
            committed_at: now,
        };
        let mut created_entities = vec![];
        let mut events = vec![];
        let mut sequence = 1i64;

        for create in request.entity_creates {
            let (entity, event) = create_lab_entity_with_event_tx(
                &mut tx,
                &transaction,
                &create,
                request.occurred_at,
                now,
                sequence,
            )
            .await?;
            events.push(event);
            created_entities.push(entity);
            sequence += 1;
        }

        for revision in request.protocol_revision_creates {
            ensure_entity_kind_registry(
                &mut tx,
                &revision.protocol_entity_id,
                &transaction.registry_id,
                LabEntityKind::ProtocolSource,
            )
            .await?;
            let checksum = format!("{:x}", Sha256::digest(revision.content.as_bytes()));
            let existing: Option<(String, i64)> = sqlx::query_as("SELECT id,revision_number FROM lab_protocol_revisions WHERE protocol_entity_id=? AND checksum_sha256=?")
                .bind(&revision.protocol_entity_id).bind(&checksum).fetch_optional(&mut *tx).await?;
            let (revision_id, revision_number, created) = if let Some((id, number)) = existing {
                (id, number, false)
            } else {
                let (number,): (i64,) = sqlx::query_as("SELECT COALESCE(MAX(revision_number),0)+1 FROM lab_protocol_revisions WHERE protocol_entity_id=?")
                    .bind(&revision.protocol_entity_id).fetch_one(&mut *tx).await?;
                let id = uuid::Uuid::new_v4().to_string();
                sqlx::query("INSERT INTO lab_protocol_revisions(id,registry_id,protocol_entity_id,revision_number,checksum_sha256,content,created_at) VALUES(?,?,?,?,?,?,?)")
                    .bind(&id).bind(&transaction.registry_id).bind(&revision.protocol_entity_id).bind(number).bind(&checksum).bind(&revision.content).bind(now)
                    .execute(&mut *tx).await?;
                (id, number, true)
            };
            let event = LabEvent {
                id: uuid::Uuid::new_v4().to_string(), registry_id: transaction.registry_id.clone(), project_id: transaction.project_id.clone(),
                transaction_id: transaction.id.clone(), sequence, entity_id: Some(revision.protocol_entity_id.clone()), prior_event_id: None,
                kind: if created { "protocol_revision_published".into() } else { "protocol_revision_reused".into() }, schema_version: 1,
                payload_json: json!({"protocol_revision_id":revision_id,"revision_number":revision_number,"checksum_sha256":checksum}).to_string(),
                occurred_at: request.occurred_at, recorded_at: now, expected_revision: None, resulting_revision: None, reason: None,
            };
            insert_lab_event_tx(&mut tx, &event).await?;
            events.push(event);
            sequence += 1;
        }

        for pin in request.run_protocol_pins {
            let status = wet_run_status_in_scope_tx(
                &mut tx,
                &pin.run_id,
                &transaction.registry_id,
                transaction.project_id.as_deref(),
            )
            .await?;
            if status != "draft" {
                anyhow::bail!("Only a draft wet-lab Run can pin its protocol");
            }
            let revision: Option<(i64,)> =
                sqlx::query_as("SELECT 1 FROM lab_protocol_revisions WHERE id=? AND registry_id=?")
                    .bind(&pin.protocol_revision_id)
                    .bind(&transaction.registry_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            if revision.is_none() {
                anyhow::bail!("Protocol revision is missing or belongs to another registry");
            }
            let updated = sqlx::query(
                "UPDATE lab_wet_runs SET protocol_revision_id=? \
                 WHERE run_id=? AND registry_id=? AND EXISTS (\
                    SELECT 1 FROM runs r WHERE r.id=lab_wet_runs.run_id \
                    AND r.project_id=? AND r.status='draft')",
            )
            .bind(&pin.protocol_revision_id)
            .bind(&pin.run_id)
            .bind(&transaction.registry_id)
            .bind(transaction.project_id.as_deref())
            .execute(&mut *tx)
            .await?;
            if updated.rows_affected() != 1 {
                anyhow::bail!("Only the owning Project can pin a draft wet-lab Run protocol");
            }
            let event = LabEvent {
                id: uuid::Uuid::new_v4().to_string(),
                registry_id: transaction.registry_id.clone(),
                project_id: transaction.project_id.clone(),
                transaction_id: transaction.id.clone(),
                sequence,
                entity_id: None,
                prior_event_id: None,
                kind: "run_protocol_pinned".into(),
                schema_version: 1,
                payload_json:
                    json!({"run_id":pin.run_id,"protocol_revision_id":pin.protocol_revision_id})
                        .to_string(),
                occurred_at: request.occurred_at,
                recorded_at: now,
                expected_revision: None,
                resulting_revision: None,
                reason: None,
            };
            insert_lab_event_tx(&mut tx, &event).await?;
            events.push(event);
            sequence += 1;
        }

        for derivation in request.derivation_creates {
            ensure_wet_run_open_tx(
                &mut tx,
                &derivation.run_id,
                &transaction.registry_id,
                transaction.project_id.as_deref(),
            )
            .await?;
            validate_derivation_quantity_conservation(&derivation)?;
            let group_id = uuid::Uuid::new_v4().to_string();
            let parent_ids = derivation
                .inputs
                .iter()
                .map(|input| input.material_unit_id.clone())
                .collect::<Vec<_>>();
            for mut input in derivation.inputs {
                input.transformation_group = Some(group_id.clone());
                let event = record_run_participant_tx(
                    &mut tx,
                    &transaction,
                    &input,
                    request.occurred_at,
                    now,
                    sequence,
                )
                .await?;
                events.push(event);
                sequence += 1;
            }
            let mut child_ids = vec![];
            for mut output in derivation.outputs {
                output.transformation_group = Some(group_id.clone());
                let initial_location_id = output.initial_location_id.clone();
                let (entity, entity_event) = create_lab_entity_with_event_tx(
                    &mut tx,
                    &transaction,
                    &output.entity,
                    request.occurred_at,
                    now,
                    sequence,
                )
                .await?;
                events.push(entity_event);
                created_entities.push(entity.clone());
                sequence += 1;
                let participant = LabRunParticipantCreate {
                    run_id: derivation.run_id.clone(),
                    material_unit_id: entity.id.clone(),
                    direction: "output".into(),
                    role: output.role,
                    effect: output.effect,
                    quantity: output.quantity,
                    transformation_group: Some(group_id.clone()),
                    expected_material_revision: None,
                };
                let participant_event = record_run_participant_tx(
                    &mut tx,
                    &transaction,
                    &participant,
                    request.occurred_at,
                    now,
                    sequence,
                )
                .await?;
                events.push(participant_event);
                sequence += 1;
                if let Some(location_id) = initial_location_id.as_deref() {
                    let location_event = place_new_material_tx(
                        &mut tx,
                        &transaction,
                        &entity.id,
                        location_id,
                        request.occurred_at,
                        now,
                        sequence,
                    )
                    .await?;
                    events.push(location_event);
                    sequence += 1;
                }
                child_ids.push(entity.id);
            }
            for parent_id in &parent_ids {
                for child_id in &child_ids {
                    ensure_derivation_edge_acyclic_tx(&mut tx, parent_id, child_id).await?;
                    let event = LabEvent {
                        id: uuid::Uuid::new_v4().to_string(), registry_id: transaction.registry_id.clone(),
                        project_id: transaction.project_id.clone(), transaction_id: transaction.id.clone(), sequence,
                        entity_id: Some(child_id.clone()), prior_event_id: None, kind: "material_derived".into(), schema_version: 1,
                        payload_json: json!({"run_id":derivation.run_id,"operation":derivation.operation,"group_id":group_id,"parent_id":parent_id,"child_id":child_id}).to_string(),
                        occurred_at: request.occurred_at, recorded_at: now, expected_revision: None, resulting_revision: None, reason: None,
                    };
                    insert_lab_event_tx(&mut tx, &event).await?;
                    sqlx::query("INSERT INTO lab_material_derivations(id,run_id,operation,group_id,parent_material_unit_id,child_material_unit_id,established_event_id,created_at) VALUES(?,?,?,?,?,?,?,?)")
                        .bind(uuid::Uuid::new_v4().to_string()).bind(&derivation.run_id).bind(&derivation.operation)
                        .bind(&group_id).bind(parent_id).bind(child_id).bind(&event.id).bind(now)
                        .execute(&mut *tx).await?;
                    events.push(event);
                    sequence += 1;
                }
            }
        }

        for output in request.run_output_creates {
            ensure_wet_run_open_tx(
                &mut tx,
                &output.run_id,
                &transaction.registry_id,
                transaction.project_id.as_deref(),
            )
            .await?;
            let initial_location_id = output.initial_location_id.clone();
            let (entity, event) = create_lab_entity_with_event_tx(
                &mut tx,
                &transaction,
                &output.entity,
                request.occurred_at,
                now,
                sequence,
            )
            .await?;
            events.push(event);
            created_entities.push(entity.clone());
            sequence += 1;
            let participant = LabRunParticipantCreate {
                run_id: output.run_id,
                material_unit_id: entity.id.clone(),
                direction: "output".into(),
                role: output.role,
                effect: output.effect,
                quantity: output.quantity,
                transformation_group: output.transformation_group,
                expected_material_revision: None,
            };
            let event = record_run_participant_tx(
                &mut tx,
                &transaction,
                &participant,
                request.occurred_at,
                now,
                sequence,
            )
            .await?;
            events.push(event);
            sequence += 1;
            if let Some(location_id) = initial_location_id.as_deref() {
                let event = place_new_material_tx(
                    &mut tx,
                    &transaction,
                    &entity.id,
                    location_id,
                    request.occurred_at,
                    now,
                    sequence,
                )
                .await?;
                events.push(event);
                sequence += 1;
            }
        }

        for material_move in request.material_moves {
            ensure_entity_kind_registry(
                &mut tx,
                &material_move.material_unit_id,
                &transaction.registry_id,
                LabEntityKind::MaterialUnit,
            )
            .await?;
            let previous_location_id =
                material_location_tx(&mut tx, &material_move.material_unit_id).await?;
            if let Some(location_id) = material_move.location_id.as_deref() {
                let single_occupancy =
                    location_occupancy_tx(&mut tx, location_id, &transaction.registry_id).await?;
                if single_occupancy && previous_location_id.as_deref() != Some(location_id) {
                    let occupied: Option<(String,)> = sqlx::query_as(
                        "SELECT material_unit_id FROM lab_material_locations WHERE location_id=? AND material_unit_id<>? LIMIT 1",
                    )
                    .bind(location_id)
                    .bind(&material_move.material_unit_id)
                    .fetch_optional(&mut *tx)
                    .await?;
                    if occupied.is_some() {
                        anyhow::bail!("Location is single-occupancy and already occupied");
                    }
                }
            }
            let affected = sqlx::query(
                "UPDATE lab_entities SET revision=revision+1,updated_at=?,last_transaction_id=? \
                 WHERE id=? AND registry_id=? AND revision=?",
            )
            .bind(now)
            .bind(&transaction.id)
            .bind(&material_move.material_unit_id)
            .bind(&transaction.registry_id)
            .bind(material_move.expected_revision)
            .execute(&mut *tx)
            .await?;
            if affected.rows_affected() != 1 {
                anyhow::bail!("Material unit revision conflict");
            }
            ensure_prior_event_registry(
                &mut tx,
                material_move.prior_event_id.as_deref(),
                &transaction.registry_id,
            )
            .await?;
            let event = LabEvent {
                id: uuid::Uuid::new_v4().to_string(),
                registry_id: transaction.registry_id.clone(),
                project_id: transaction.project_id.clone(),
                transaction_id: transaction.id.clone(),
                sequence,
                entity_id: Some(material_move.material_unit_id.clone()),
                prior_event_id: material_move.prior_event_id,
                kind: "material_moved".into(),
                schema_version: 1,
                payload_json: json!({
                    "from_location_id": previous_location_id,
                    "to_location_id": material_move.location_id,
                })
                .to_string(),
                occurred_at: material_move.occurred_at,
                recorded_at: now,
                expected_revision: Some(material_move.expected_revision),
                resulting_revision: Some(material_move.expected_revision + 1),
                reason: material_move.reason,
            };
            insert_lab_event_tx(&mut tx, &event).await?;
            if let Some(location_id) = material_move.location_id.as_deref() {
                sqlx::query(
                    "INSERT INTO lab_material_locations(material_unit_id,location_id,established_event_id,updated_at) VALUES(?,?,?,?) \
                     ON CONFLICT(material_unit_id) DO UPDATE SET location_id=excluded.location_id,established_event_id=excluded.established_event_id,updated_at=excluded.updated_at",
                )
                .bind(&material_move.material_unit_id)
                .bind(location_id)
                .bind(&event.id)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            } else {
                sqlx::query("DELETE FROM lab_material_locations WHERE material_unit_id=?")
                    .bind(&material_move.material_unit_id)
                    .execute(&mut *tx)
                    .await?;
            }
            events.push(event);
            sequence += 1;
        }

        for adjustment in request.material_adjustments {
            ensure_entity_kind_registry(
                &mut tx,
                &adjustment.material_unit_id,
                &transaction.registry_id,
                LabEntityKind::MaterialUnit,
            )
            .await?;
            ensure_prior_event_registry(
                &mut tx,
                adjustment.prior_event_id.as_deref(),
                &transaction.registry_id,
            )
            .await?;
            let current_quantity: Option<(
                String,
                Option<String>,
                Option<String>,
                String,
                String,
                String,
            )> =
                sqlx::query_as(
                    "SELECT quantity_state,quantity_value,quantity_unit,lifecycle,availability,identity_state \
                     FROM lab_material_units WHERE entity_id=? AND registry_id=?",
                )
                .bind(&adjustment.material_unit_id)
                .bind(&transaction.registry_id)
                .fetch_optional(&mut *tx)
                .await?;
            let Some((
                current_state,
                _,
                current_unit,
                current_lifecycle,
                current_availability,
                current_identity_state,
            )) = current_quantity
            else {
                anyhow::bail!("Material unit details are missing");
            };
            if current_state == "measured"
                && adjustment.quantity.state == LabQuantityState::Measured
                && current_unit.as_deref().and_then(super::lab_unit_dimension)
                    != adjustment.quantity.dimension()
            {
                anyhow::bail!("Material adjustment cannot change quantity dimension");
            }
            sqlx::query(
                "UPDATE lab_reservations SET status='expired',released_at=? \
                 WHERE material_unit_id=? AND status='active' \
                 AND expires_at IS NOT NULL AND expires_at<=?",
            )
            .bind(now)
            .bind(&adjustment.material_unit_id)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            let active_reservations: Vec<(String, String)> = sqlx::query_as(
                "SELECT quantity_value,quantity_unit FROM lab_reservations \
                 WHERE material_unit_id=? AND status='active'",
            )
            .bind(&adjustment.material_unit_id)
            .fetch_all(&mut *tx)
            .await?;
            if !active_reservations.is_empty() {
                let adjusted_unit = adjustment
                    .quantity
                    .unit
                    .as_deref()
                    .filter(|_| adjustment.quantity.state == LabQuantityState::Measured)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Material with active reservations must keep a measured balance"
                        )
                    })?;
                let adjusted_value =
                    decimal(adjustment.quantity.value.as_deref().unwrap_or_default())?;
                let reserved_total = active_reservations.into_iter().try_fold(
                    Decimal::ZERO,
                    |total, (value, unit)| {
                        Ok::<_, anyhow::Error>(
                            total + convert_decimal_unit(decimal(&value)?, &unit, adjusted_unit)?,
                        )
                    },
                )?;
                if adjusted_value < reserved_total {
                    anyhow::bail!(
                        "Material adjustment cannot reduce balance below active reservations"
                    );
                }
            }
            let affected = sqlx::query(
                "UPDATE lab_entities SET revision=revision+1,updated_at=?,last_transaction_id=? \
                 WHERE id=? AND registry_id=? AND revision=?",
            )
            .bind(now)
            .bind(&transaction.id)
            .bind(&adjustment.material_unit_id)
            .bind(&transaction.registry_id)
            .bind(adjustment.expected_revision)
            .execute(&mut *tx)
            .await?;
            if affected.rows_affected() != 1 {
                anyhow::bail!("Material unit revision conflict");
            }
            let lifecycle = adjustment
                .lifecycle
                .as_deref()
                .unwrap_or(&current_lifecycle);
            let availability = adjustment
                .availability
                .as_deref()
                .unwrap_or(&current_availability);
            let identity_state = adjustment
                .identity_state
                .as_deref()
                .unwrap_or(&current_identity_state);
            let material_affected = sqlx::query(
                "UPDATE lab_material_units SET quantity_state=?,quantity_value=?,quantity_unit=?,lifecycle=?,availability=?,identity_state=? WHERE entity_id=?",
            )
            .bind(quantity_storage_state(&adjustment.quantity))
            .bind(adjustment.quantity.value.as_deref())
            .bind(adjustment.quantity.unit.as_deref())
            .bind(lifecycle)
            .bind(availability)
            .bind(identity_state)
            .bind(&adjustment.material_unit_id)
            .execute(&mut *tx)
            .await?;
            if material_affected.rows_affected() != 1 {
                anyhow::bail!("Material unit details are missing");
            }
            let event = LabEvent {
                id: uuid::Uuid::new_v4().to_string(),
                registry_id: transaction.registry_id.clone(),
                project_id: transaction.project_id.clone(),
                transaction_id: transaction.id.clone(),
                sequence,
                entity_id: Some(adjustment.material_unit_id.clone()),
                prior_event_id: adjustment.prior_event_id,
                kind: "material_quantity_adjusted".into(),
                schema_version: 1,
                payload_json: json!({
                    "quantity": adjustment.quantity,
                    "lifecycle": adjustment.lifecycle,
                    "availability": adjustment.availability,
                    "identity_state": adjustment.identity_state,
                })
                .to_string(),
                occurred_at: adjustment.occurred_at,
                recorded_at: now,
                expected_revision: Some(adjustment.expected_revision),
                resulting_revision: Some(adjustment.expected_revision + 1),
                reason: Some(adjustment.reason),
            };
            insert_lab_event_tx(&mut tx, &event).await?;
            events.push(event);
            sequence += 1;
        }

        for document_upsert in request.document_upserts {
            let entity = get_entity_in_registry_tx(
                &mut tx,
                &document_upsert.entity_id,
                &transaction.registry_id,
            )
            .await?;
            let document = upsert_lab_document_tx(
                &mut tx,
                &entity,
                &transaction.registry_id,
                &document_upsert,
                now,
            )
            .await?;
            let event = LabEvent {
                id: uuid::Uuid::new_v4().to_string(),
                registry_id: transaction.registry_id.clone(),
                project_id: transaction.project_id.clone(),
                transaction_id: transaction.id.clone(),
                sequence,
                entity_id: Some(entity.id.clone()),
                prior_event_id: None,
                kind: "document_updated".into(),
                schema_version: 1,
                payload_json: json!({
                    "document_id": document.id,
                    "relative_path": document.relative_path,
                    "revision": document.revision,
                })
                .to_string(),
                occurred_at: request.occurred_at,
                recorded_at: now,
                expected_revision: None,
                resulting_revision: None,
                reason: None,
            };
            insert_lab_event_tx(&mut tx, &event).await?;
            events.push(event);
            sequence += 1;
        }

        for participant in request.run_participants {
            let event = record_run_participant_tx(
                &mut tx,
                &transaction,
                &participant,
                request.occurred_at,
                now,
                sequence,
            )
            .await?;
            events.push(event);
            sequence += 1;
        }

        for participant in request.subject_participants {
            ensure_wet_run_open_tx(
                &mut tx,
                &participant.run_id,
                &transaction.registry_id,
                transaction.project_id.as_deref(),
            )
            .await?;
            ensure_entity_kind_registry(
                &mut tx,
                &participant.subject_id,
                &transaction.registry_id,
                LabEntityKind::Subject,
            )
            .await?;
            let exists: Option<(i64,)> =
                sqlx::query_as("SELECT 1 FROM lab_subjects WHERE entity_id=?")
                    .bind(&participant.subject_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            if exists.is_none() {
                anyhow::bail!("Subject details are missing");
            }
            let event = LabEvent {
                id: uuid::Uuid::new_v4().to_string(), registry_id: transaction.registry_id.clone(),
                project_id: transaction.project_id.clone(), transaction_id: transaction.id.clone(), sequence,
                entity_id: Some(participant.subject_id.clone()), prior_event_id: None,
                kind: "subject_participated".into(), schema_version: 1,
                payload_json: json!({"run_id":participant.run_id,"role":participant.role,"effect":participant.effect}).to_string(),
                occurred_at: request.occurred_at, recorded_at: now, expected_revision: None,
                resulting_revision: None, reason: None,
            };
            insert_lab_event_tx(&mut tx, &event).await?;
            sqlx::query("INSERT INTO lab_subject_participants(id,run_id,subject_id,role,effect,established_event_id,created_at) VALUES(?,?,?,?,?,?,?)")
                .bind(uuid::Uuid::new_v4().to_string()).bind(&participant.run_id).bind(&participant.subject_id)
                .bind(&participant.role).bind(&participant.effect).bind(&event.id).bind(now)
                .execute(&mut *tx).await?;
            events.push(event);
            sequence += 1;
        }

        for deviation in request.run_deviation_creates {
            ensure_wet_run_open_tx(
                &mut tx,
                &deviation.run_id,
                &transaction.registry_id,
                transaction.project_id.as_deref(),
            )
            .await?;
            let deviation_id = uuid::Uuid::new_v4().to_string();
            let event = LabEvent {
                id: uuid::Uuid::new_v4().to_string(),
                registry_id: transaction.registry_id.clone(),
                project_id: transaction.project_id.clone(),
                transaction_id: transaction.id.clone(),
                sequence,
                entity_id: None,
                prior_event_id: None,
                kind: "run_deviation_recorded".into(),
                schema_version: 1,
                payload_json: json!({
                    "deviation_id": deviation_id,
                    "run_id": deviation.run_id,
                    "step_ref": deviation.step_ref,
                    "description": deviation.description,
                    "impact": deviation.impact,
                    "disposition": deviation.disposition,
                })
                .to_string(),
                occurred_at: deviation.occurred_at,
                recorded_at: now,
                expected_revision: None,
                resulting_revision: None,
                reason: None,
            };
            insert_lab_event_tx(&mut tx, &event).await?;
            sqlx::query(
                "INSERT INTO lab_run_deviations(id,run_id,step_ref,description,impact,disposition,occurred_at,recorded_at,established_event_id) VALUES(?,?,?,?,?,?,?,?,?)",
            )
            .bind(&deviation_id)
            .bind(&deviation.run_id)
            .bind(deviation.step_ref.as_deref())
            .bind(&deviation.description)
            .bind(&deviation.impact)
            .bind(deviation.disposition.as_deref())
            .bind(deviation.occurred_at)
            .bind(now)
            .bind(&event.id)
            .execute(&mut *tx)
            .await?;
            events.push(event);
            sequence += 1;
        }

        for evidence in request.data_evidence_creates {
            if evidence
                .owner_registry_id
                .as_deref()
                .is_some_and(|owner| owner != transaction.registry_id)
            {
                anyhow::bail!("Data evidence Registry owner differs from the transaction registry");
            }
            if evidence.owner_project_id.as_deref() != transaction.project_id.as_deref()
                && evidence.owner_project_id.is_some()
            {
                anyhow::bail!("Data evidence Project owner differs from the transaction Project");
            }
            if let Some(run_id) = evidence.producing_run_id.as_deref() {
                ensure_wet_run_open_tx(
                    &mut tx,
                    run_id,
                    &transaction.registry_id,
                    transaction.project_id.as_deref(),
                )
                .await?;
            }
            let evidence_id = uuid::Uuid::new_v4().to_string();
            let display_id = allocate_display_id(&mut tx, &transaction.registry_id, "DAT").await?;
            let event = LabEvent {
                id: uuid::Uuid::new_v4().to_string(),
                registry_id: transaction.registry_id.clone(),
                project_id: transaction.project_id.clone(),
                transaction_id: transaction.id.clone(),
                sequence,
                entity_id: None,
                prior_event_id: None,
                kind: "data_evidence_registered".into(),
                schema_version: 1,
                payload_json: json!({"evidence_id":evidence_id,"display_id":display_id,"run_id":evidence.producing_run_id,"role":evidence.role,"uri":evidence.uri,"checksum_sha256":evidence.checksum_sha256}).to_string(),
                occurred_at: request.occurred_at,
                recorded_at: now,
                expected_revision: None,
                resulting_revision: None,
                reason: None,
            };
            insert_lab_event_tx(&mut tx, &event).await?;
            sqlx::query(
                "INSERT INTO lab_data_evidence(id,display_id,registry_id,owner_project_id,owner_registry_id,producing_run_id,role,uri,format,size_bytes,checksum_sha256,origin,manifest_json,created_at,established_event_id) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(&evidence_id)
            .bind(&display_id)
            .bind(&transaction.registry_id)
            .bind(evidence.owner_project_id.as_deref())
            .bind(evidence.owner_registry_id.as_deref())
            .bind(evidence.producing_run_id.as_deref())
            .bind(&evidence.role)
            .bind(&evidence.uri)
            .bind(evidence.format.as_deref())
            .bind(evidence.size_bytes)
            .bind(evidence.checksum_sha256.as_deref())
            .bind(&evidence.origin)
            .bind(&evidence.manifest_json)
            .bind(now)
            .bind(&event.id)
            .execute(&mut *tx)
            .await?;
            events.push(event);
            sequence += 1;
        }

        for amendment in request.amendment_creates {
            let original: Option<(Option<String>,)> =
                sqlx::query_as("SELECT entity_id FROM lab_events WHERE id=? AND registry_id=?")
                    .bind(&amendment.original_event_id)
                    .bind(&transaction.registry_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            let Some((entity_id,)) = original else {
                anyhow::bail!("Original amendment event is missing or belongs to another registry");
            };
            let run_id: Option<String> = sqlx::query_scalar(
                "SELECT run_id FROM (\
                 SELECT run_id FROM lab_run_participants WHERE established_event_id=? UNION \
                 SELECT run_id FROM lab_subject_participants WHERE established_event_id=? UNION \
                 SELECT run_id FROM lab_run_deviations WHERE established_event_id=? UNION \
                 SELECT producing_run_id AS run_id FROM lab_data_evidence WHERE established_event_id=? UNION \
                 SELECT run_id FROM lab_material_derivations WHERE established_event_id=?) WHERE run_id IS NOT NULL LIMIT 1",
            ).bind(&amendment.original_event_id).bind(&amendment.original_event_id).bind(&amendment.original_event_id)
                .bind(&amendment.original_event_id).bind(&amendment.original_event_id).fetch_optional(&mut *tx).await?;
            let run_id = run_id.ok_or_else(|| {
                anyhow::anyhow!("Amendment event is not attached to a wet-lab Run")
            })?;
            let status = wet_run_status_in_scope_tx(
                &mut tx,
                &run_id,
                &transaction.registry_id,
                transaction.project_id.as_deref(),
            )
            .await?;
            if !matches!(status.as_str(), "succeeded" | "failed" | "cancelled") {
                anyhow::bail!(
                    "Open Run records should be corrected with revisioned events, not Amendments"
                );
            }
            let mut affected_ids = vec![run_id.clone()];
            if let Some(entity_id) = entity_id.as_deref() {
                affected_ids.push(entity_id.to_string());
                let descendants: Vec<String> = sqlx::query_scalar(
                    "WITH RECURSIVE descendants(id) AS (\
                     SELECT child_material_unit_id FROM lab_material_derivations WHERE parent_material_unit_id=? \
                     UNION SELECT d.child_material_unit_id FROM lab_material_derivations d JOIN descendants x ON d.parent_material_unit_id=x.id) SELECT id FROM descendants ORDER BY id",
                ).bind(entity_id).fetch_all(&mut *tx).await?;
                for descendant in descendants {
                    affected_ids.push(descendant.clone());
                    let later_runs: Vec<String> = sqlx::query_scalar("SELECT DISTINCT run_id FROM lab_run_participants WHERE material_unit_id=? ORDER BY run_id")
                        .bind(&descendant).fetch_all(&mut *tx).await?;
                    affected_ids.extend(later_runs);
                }
            }
            affected_ids.sort();
            affected_ids.dedup();
            let amendment_id = uuid::Uuid::new_v4().to_string();
            let display_id = allocate_display_id(&mut tx, &transaction.registry_id, "AMD").await?;
            let event = LabEvent {
                id: uuid::Uuid::new_v4().to_string(), registry_id: transaction.registry_id.clone(), project_id: transaction.project_id.clone(),
                transaction_id: transaction.id.clone(), sequence, entity_id, prior_event_id: Some(amendment.original_event_id.clone()),
                kind: "amendment_recorded".into(), schema_version: 1,
                payload_json: json!({"amendment_id":amendment_id,"display_id":display_id,"run_id":run_id,"correction":amendment.correction_json,"affected_ids":affected_ids}).to_string(),
                occurred_at: request.occurred_at, recorded_at: now, expected_revision: None, resulting_revision: None,
                reason: Some(amendment.reason.clone()),
            };
            insert_lab_event_tx(&mut tx, &event).await?;
            sqlx::query("INSERT INTO lab_amendments(id,display_id,registry_id,run_id,original_event_id,reason,correction_json,affected_ids_json,established_event_id,created_at) VALUES(?,?,?,?,?,?,?,?,?,?)")
                .bind(&amendment_id).bind(&display_id).bind(&transaction.registry_id).bind(&run_id).bind(&amendment.original_event_id)
                .bind(&amendment.reason).bind(&amendment.correction_json).bind(serde_json::to_string(&affected_ids)?).bind(&event.id).bind(now)
                .execute(&mut *tx).await?;
            events.push(event);
            sequence += 1;
        }

        for reservation in request.reservation_creates {
            let status = wet_run_status_in_scope_tx(
                &mut tx,
                &reservation.run_id,
                &transaction.registry_id,
                transaction.project_id.as_deref(),
            )
            .await?;
            match status.as_str() {
                "draft" | "running" => {}
                _ => {
                    anyhow::bail!("Completed or cancelled wet-lab Runs cannot reserve material")
                }
            }
            ensure_entity_kind_registry(
                &mut tx,
                &reservation.material_unit_id,
                &transaction.registry_id,
                LabEntityKind::MaterialUnit,
            )
            .await?;
            let material: Option<(String, Option<String>, Option<String>, String, String)> =
                sqlx::query_as(
                    "SELECT quantity_state,quantity_value,quantity_unit,lifecycle,availability FROM lab_material_units WHERE entity_id=?",
                )
            .bind(&reservation.material_unit_id)
            .fetch_optional(&mut *tx)
            .await?;
            let Some((quantity_state, value, unit, lifecycle, availability)) = material else {
                anyhow::bail!("Material unit details are missing");
            };
            if quantity_state != "measured" || lifecycle != "active" || availability != "available"
            {
                anyhow::bail!(
                    "Only active, available MaterialUnits with measured quantity can be reserved"
                );
            }
            let value = value.ok_or_else(|| anyhow::anyhow!("Material quantity is missing"))?;
            let unit = unit.ok_or_else(|| anyhow::anyhow!("Material unit is missing"))?;
            let reservation_value = reservation.quantity.value.as_deref().unwrap_or_default();
            let reservation_unit = reservation.quantity.unit.as_deref().unwrap_or_default();
            sqlx::query(
                "UPDATE lab_reservations SET status='expired',released_at=? \
                 WHERE material_unit_id=? AND status='active' AND expires_at IS NOT NULL AND expires_at<=?",
            )
            .bind(now)
            .bind(&reservation.material_unit_id)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            let active: Vec<(String, String)> = sqlx::query_as(
                "SELECT quantity_value,quantity_unit FROM lab_reservations WHERE material_unit_id=? AND status='active'",
            )
            .bind(&reservation.material_unit_id)
            .fetch_all(&mut *tx)
            .await?;
            let on_hand = decimal(&value)?;
            let already_reserved =
                active
                    .into_iter()
                    .try_fold(Decimal::ZERO, |sum, (value, active_unit)| {
                        Ok::<_, anyhow::Error>(
                            sum + convert_decimal_unit(decimal(&value)?, &active_unit, &unit)?,
                        )
                    })?;
            let requested =
                convert_decimal_unit(decimal(reservation_value)?, reservation_unit, &unit)?;
            if requested <= Decimal::ZERO || already_reserved + requested > on_hand {
                anyhow::bail!("Reservation would exceed available MaterialUnit balance");
            }
            let event = LabEvent {
                id: uuid::Uuid::new_v4().to_string(),
                registry_id: transaction.registry_id.clone(),
                project_id: transaction.project_id.clone(),
                transaction_id: transaction.id.clone(),
                sequence,
                entity_id: Some(reservation.material_unit_id.clone()),
                prior_event_id: None,
                kind: "material_reserved".into(),
                schema_version: 1,
                payload_json: json!({"run_id":reservation.run_id,"quantity":reservation.quantity,"expires_at":reservation.expires_at}).to_string(),
                occurred_at: request.occurred_at,
                recorded_at: now,
                expected_revision: None,
                resulting_revision: None,
                reason: None,
            };
            insert_lab_event_tx(&mut tx, &event).await?;
            sqlx::query(
                "INSERT INTO lab_reservations(id,run_id,material_unit_id,quantity_value,quantity_unit,status,expires_at,created_at,released_at) \
                 VALUES(?,?,?,?,?,'active',?,?,NULL)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(&reservation.run_id)
            .bind(&reservation.material_unit_id)
            .bind(reservation_value)
            .bind(reservation_unit)
            .bind(reservation.expires_at)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            events.push(event);
            sequence += 1;
        }

        for observation in request.qc_observation_creates {
            ensure_entity_registry(&mut tx, &observation.entity_id, &transaction.registry_id)
                .await?;
            if let Some(run_id) = observation.run_id.as_deref() {
                wet_run_status_in_scope_tx(
                    &mut tx,
                    run_id,
                    &transaction.registry_id,
                    transaction.project_id.as_deref(),
                )
                .await?;
            }
            if let Some(method_revision_id) = observation.method_revision_id.as_deref() {
                let method_exists: Option<(i64,)> = sqlx::query_as(
                    "SELECT 1 FROM lab_protocol_revisions WHERE id=? AND registry_id=?",
                )
                .bind(method_revision_id)
                .bind(&transaction.registry_id)
                .fetch_optional(&mut *tx)
                .await?;
                if method_exists.is_none() {
                    anyhow::bail!("QC method revision is missing or belongs to another registry");
                }
            }
            let observation_id = uuid::Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO lab_qc_observations(id,registry_id,entity_id,run_id,method_revision_id,measurement_json,evidence_json,observed_at,recorded_at) VALUES(?,?,?,?,?,?,?,?,?)",
            )
            .bind(&observation_id)
            .bind(&transaction.registry_id)
            .bind(&observation.entity_id)
            .bind(observation.run_id.as_deref())
            .bind(observation.method_revision_id.as_deref())
            .bind(&observation.measurement_json)
            .bind(&observation.evidence_json)
            .bind(observation.observed_at)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            let event = LabEvent {
                id: uuid::Uuid::new_v4().to_string(), registry_id: transaction.registry_id.clone(), project_id: transaction.project_id.clone(),
                transaction_id: transaction.id.clone(), sequence, entity_id: Some(observation.entity_id), prior_event_id: None,
                kind: "qc_observation_recorded".into(), schema_version: 1,
                payload_json: json!({"observation_id":observation_id,"measurement":observation.measurement_json,"evidence":observation.evidence_json}).to_string(),
                occurred_at: observation.observed_at, recorded_at: now, expected_revision: None, resulting_revision: None, reason: None,
            };
            insert_lab_event_tx(&mut tx, &event).await?;
            events.push(event);
            sequence += 1;
        }

        for assessment in request.qc_assessment_creates {
            ensure_entity_registry(&mut tx, &assessment.entity_id, &transaction.registry_id)
                .await?;
            for observation_id in &assessment.observation_ids {
                let belongs: Option<(i64,)> = sqlx::query_as(
                    "SELECT 1 FROM lab_qc_observations WHERE id=? AND registry_id=? AND entity_id=?",
                )
                .bind(observation_id)
                .bind(&transaction.registry_id)
                .bind(&assessment.entity_id)
                .fetch_optional(&mut *tx)
                .await?;
                if belongs.is_none() {
                    anyhow::bail!(
                        "QC assessment observation does not belong to the assessed entity"
                    );
                }
            }
            let assessment_id = uuid::Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO lab_qc_assessments(id,registry_id,entity_id,observation_ids_json,criteria_json,verdict,rationale,created_at) VALUES(?,?,?,?,?,?,?,?)",
            )
            .bind(&assessment_id)
            .bind(&transaction.registry_id)
            .bind(&assessment.entity_id)
            .bind(serde_json::to_string(&assessment.observation_ids)?)
            .bind(&assessment.criteria_json)
            .bind(&assessment.verdict)
            .bind(&assessment.rationale)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            let event = LabEvent {
                id: uuid::Uuid::new_v4().to_string(), registry_id: transaction.registry_id.clone(), project_id: transaction.project_id.clone(),
                transaction_id: transaction.id.clone(), sequence, entity_id: Some(assessment.entity_id), prior_event_id: None,
                kind: "qc_assessment_recorded".into(), schema_version: 1,
                payload_json: json!({"assessment_id":assessment_id,"verdict":assessment.verdict,"observation_ids":assessment.observation_ids}).to_string(),
                occurred_at: request.occurred_at, recorded_at: now, expected_revision: None, resulting_revision: None, reason: Some(assessment.rationale),
            };
            insert_lab_event_tx(&mut tx, &event).await?;
            events.push(event);
            sequence += 1;
        }

        for conclusion in request.conclusion_creates {
            ensure_wet_run_open_tx(
                &mut tx,
                &conclusion.run_id,
                &transaction.registry_id,
                transaction.project_id.as_deref(),
            )
            .await?;
            let project_id = transaction.project_id.as_deref().ok_or_else(|| {
                anyhow::anyhow!("Conclusion requires a Project-owned transaction")
            })?;
            for evidence_id in &conclusion.evidence_ids {
                let exists: Option<(i64,)> = sqlx::query_as(
                    "SELECT 1 FROM (\
                     SELECT id FROM lab_data_evidence WHERE owner_registry_id=? OR owner_project_id=? UNION \
                     SELECT id FROM lab_qc_observations WHERE registry_id=? UNION \
                     SELECT id FROM lab_qc_assessments WHERE registry_id=? UNION \
                     SELECT id FROM lab_events WHERE registry_id=?) WHERE id=? LIMIT 1",
                )
                .bind(&transaction.registry_id).bind(project_id).bind(&transaction.registry_id)
                .bind(&transaction.registry_id).bind(&transaction.registry_id).bind(evidence_id)
                .fetch_optional(&mut *tx).await?;
                if exists.is_none() {
                    anyhow::bail!("Conclusion evidence is missing or belongs to another scope");
                }
            }
            let decision_id = uuid::Uuid::new_v4().to_string();
            let now_metadata = json!({"conclusion":conclusion.conclusion,"evidence_ids":conclusion.evidence_ids,"lesson":conclusion.lesson});
            sqlx::query("INSERT INTO research_nodes(id,project_id,kind,title,ref_id,metadata_json,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)")
                .bind(&decision_id).bind(project_id).bind("decision").bind(&conclusion.title).bind(&conclusion.run_id)
                .bind(now_metadata.to_string()).bind(now).bind(now).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO research_edges(id,project_id,source_id,target_id,relation,metadata_json,created_at) VALUES(?,?,?,?,?,?,?)")
                .bind(uuid::Uuid::new_v4().to_string()).bind(project_id).bind(&decision_id).bind(format!("run:{}", conclusion.run_id))
                .bind("concludes").bind(json!({"evidence_ids":conclusion.evidence_ids}).to_string()).bind(now)
                .execute(&mut *tx).await?;
            let event = LabEvent {
                id: uuid::Uuid::new_v4().to_string(), registry_id: transaction.registry_id.clone(), project_id: transaction.project_id.clone(),
                transaction_id: transaction.id.clone(), sequence, entity_id: None, prior_event_id: None, kind: "conclusion_confirmed".into(), schema_version: 1,
                payload_json: json!({"decision_id":decision_id,"run_id":conclusion.run_id,"title":conclusion.title,"evidence_ids":conclusion.evidence_ids}).to_string(),
                occurred_at: request.occurred_at, recorded_at: now, expected_revision: None, resulting_revision: None, reason: None,
            };
            insert_lab_event_tx(&mut tx, &event).await?;
            events.push(event);
            sequence += 1;
        }

        for update in request.run_status_updates {
            let project_id = transaction
                .project_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("Wet-lab Run status updates require a Project"))?;
            let current_status_text = wet_run_status_in_scope_tx(
                &mut tx,
                &update.run_id,
                &transaction.registry_id,
                Some(project_id),
            )
            .await?;
            let current_status = RunStatus::from_storage(&current_status_text)?;
            if current_status == update.status {
                anyhow::bail!("Wet-lab Run already has the requested status");
            }
            super::validate_run_transition(current_status, update.status)?;
            let timestamps: (Option<i64>, Option<i64>) =
                sqlx::query_as("SELECT started_at,ended_at FROM runs WHERE id=? AND project_id=?")
                    .bind(&update.run_id)
                    .bind(project_id)
                    .fetch_one(&mut *tx)
                    .await?;
            let started_at = if update.status == RunStatus::Running && timestamps.0.is_none() {
                Some(now)
            } else {
                timestamps.0
            };
            let ended_at = if update.status.is_terminal() {
                Some(now)
            } else {
                timestamps.1
            };
            let updated = sqlx::query(
                "UPDATE runs SET status=?,started_at=?,ended_at=?,lifecycle_owner=NULL,lifecycle_lease_until=NULL \
                 WHERE id=? AND project_id=? AND kind='wet_lab' AND status=? AND EXISTS (\
                    SELECT 1 FROM lab_wet_runs w WHERE w.run_id=runs.id AND w.registry_id=?)",
            )
            .bind(update.status.as_str())
            .bind(started_at)
            .bind(ended_at)
            .bind(&update.run_id)
            .bind(project_id)
            .bind(current_status.as_str())
            .bind(&transaction.registry_id)
            .execute(&mut *tx)
            .await?;
            if updated.rows_affected() != 1 {
                anyhow::bail!("Wet-lab Run status changed concurrently");
            }
            if update.status.is_terminal() {
                sqlx::query(
                    "UPDATE lab_reservations SET status='released',released_at=? \
                     WHERE run_id=? AND status='active'",
                )
                .bind(now)
                .bind(&update.run_id)
                .execute(&mut *tx)
                .await?;
            }
            let event = LabEvent {
                id: uuid::Uuid::new_v4().to_string(),
                registry_id: transaction.registry_id.clone(),
                project_id: transaction.project_id.clone(),
                transaction_id: transaction.id.clone(),
                sequence,
                entity_id: None,
                prior_event_id: None,
                kind: "run_status_changed".into(),
                schema_version: 1,
                payload_json: json!({
                    "run_id": update.run_id,
                    "from": current_status,
                    "to": update.status,
                })
                .to_string(),
                occurred_at: request.occurred_at,
                recorded_at: now,
                expected_revision: None,
                resulting_revision: None,
                reason: None,
            };
            insert_lab_event_tx(&mut tx, &event).await?;
            events.push(event);
            sequence += 1;
        }

        for update in request.entity_updates {
            ensure_entity_registry(&mut tx, &update.entity_id, &transaction.registry_id).await?;
            let affected = sqlx::query(
                "UPDATE lab_entities SET title=?, subtype=?, metadata_json=?, revision=revision+1, \
                 updated_at=?, last_transaction_id=? \
                 WHERE id=? AND registry_id=? AND revision=?",
            )
            .bind(update.title.trim())
            .bind(update.subtype.as_deref().map(str::trim).filter(|value| !value.is_empty()))
            .bind(&update.metadata_json)
            .bind(now)
            .bind(&transaction.id)
            .bind(&update.entity_id)
            .bind(&transaction.registry_id)
            .bind(update.expected_revision)
            .execute(&mut *tx)
            .await?;
            if affected.rows_affected() != 1 {
                anyhow::bail!("Lab entity revision conflict");
            }
            let event = LabEvent {
                id: uuid::Uuid::new_v4().to_string(),
                registry_id: transaction.registry_id.clone(),
                project_id: transaction.project_id.clone(),
                transaction_id: transaction.id.clone(),
                sequence,
                entity_id: Some(update.entity_id),
                prior_event_id: update.prior_event_id,
                kind: update.event_kind,
                schema_version: 1,
                payload_json: update.event_payload_json,
                occurred_at: update.occurred_at,
                recorded_at: now,
                expected_revision: Some(update.expected_revision),
                resulting_revision: Some(update.expected_revision + 1),
                reason: update.reason,
            };
            ensure_prior_event_registry(
                &mut tx,
                event.prior_event_id.as_deref(),
                &transaction.registry_id,
            )
            .await?;
            insert_lab_event_tx(&mut tx, &event).await?;
            events.push(event);
            sequence += 1;
        }

        for record in request.event_records {
            if let Some(entity_id) = record.entity_id.as_deref() {
                ensure_entity_registry(&mut tx, entity_id, &transaction.registry_id).await?;
            }
            ensure_prior_event_registry(
                &mut tx,
                record.prior_event_id.as_deref(),
                &transaction.registry_id,
            )
            .await?;
            let event = LabEvent {
                id: uuid::Uuid::new_v4().to_string(),
                registry_id: transaction.registry_id.clone(),
                project_id: transaction.project_id.clone(),
                transaction_id: transaction.id.clone(),
                sequence,
                entity_id: record.entity_id,
                prior_event_id: record.prior_event_id,
                kind: record.kind,
                schema_version: 1,
                payload_json: record.payload_json,
                occurred_at: record.occurred_at,
                recorded_at: now,
                expected_revision: None,
                resulting_revision: None,
                reason: record.reason,
            };
            insert_lab_event_tx(&mut tx, &event).await?;
            events.push(event);
            sequence += 1;
        }

        let receipt = StoredReceipt {
            created_entities: created_entities.clone(),
            event_ids: events.iter().map(|event| event.id.clone()).collect(),
        };
        transaction.receipt_json = serde_json::to_string(&receipt)?;
        sqlx::query("UPDATE lab_transactions SET receipt_json=? WHERE id=?")
            .bind(&transaction.receipt_json)
            .bind(&transaction.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(LabTransactionResult {
            transaction,
            events,
            created_entities,
            idempotent: false,
        })
    }

    pub async fn get_lab_transaction(
        &self,
        registry_id: &str,
        command_id: &str,
    ) -> Result<Option<LabTransaction>> {
        get_transaction_by_command_pool(&self.pool, registry_id, command_id).await
    }

    pub async fn replay_lab_transaction(
        &self,
        registry_id: &str,
        command_id: &str,
    ) -> Result<Option<LabTransactionResult>> {
        let Some(transaction) = self.get_lab_transaction(registry_id, command_id).await? else {
            return Ok(None);
        };
        let events = self.list_lab_events(&transaction.id).await?;
        let receipt: StoredReceipt = serde_json::from_str(&transaction.receipt_json)?;
        Ok(Some(LabTransactionResult {
            transaction,
            events,
            created_entities: receipt.created_entities,
            idempotent: true,
        }))
    }

    pub async fn list_lab_events(&self, transaction_id: &str) -> Result<Vec<LabEvent>> {
        let rows = sqlx::query(
            "SELECT id,registry_id,project_id,transaction_id,sequence,entity_id,prior_event_id,kind,\
                    schema_version,payload_json,occurred_at,recorded_at,expected_revision,resulting_revision,reason \
             FROM lab_events WHERE transaction_id=? ORDER BY sequence ASC",
        )
        .bind(transaction_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(lab_event_from_row).collect()
    }

    pub async fn get_lab_resource_definition(
        &self,
        entity_id: &str,
    ) -> Result<Option<LabResourceDefinition>> {
        let row = sqlx::query(
            "SELECT entity_id,category,supplier,catalog_number,attributes_json \
             FROM lab_resource_definitions WHERE entity_id=?",
        )
        .bind(entity_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(lab_resource_definition_from_row).transpose()
    }

    pub async fn list_lab_aliases(&self, entity_id: &str) -> Result<Vec<LabAlias>> {
        let rows = sqlx::query(
            "SELECT id,registry_id,entity_id,alias_type,namespace,value,created_at \
             FROM lab_aliases WHERE entity_id=? ORDER BY created_at,id",
        )
        .bind(entity_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(lab_alias_from_row).collect()
    }

    pub async fn find_lab_aliases(&self, registry_id: &str, value: &str) -> Result<Vec<LabAlias>> {
        let rows = sqlx::query(
            "SELECT id,registry_id,entity_id,alias_type,namespace,value,created_at \
             FROM lab_aliases WHERE registry_id=? AND value=? ORDER BY created_at,id",
        )
        .bind(registry_id)
        .bind(value.trim())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(lab_alias_from_row).collect()
    }

    pub async fn get_lab_lot(&self, entity_id: &str) -> Result<Option<LabLot>> {
        let row = sqlx::query(
            "SELECT entity_id,resource_definition_id,supplier,catalog_number,lot_number,received_at,expiry_at,origin_kind \
             FROM lab_lots WHERE entity_id=?",
        )
        .bind(entity_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(lab_lot_from_row).transpose()
    }

    pub async fn get_lab_material_unit(&self, entity_id: &str) -> Result<Option<LabMaterialUnit>> {
        let row = sqlx::query(
            "SELECT entity_id,lot_id,usage_class,quantity_state,quantity_value,quantity_unit,vessel_description,lifecycle,availability,identity_state,origin_kind \
             FROM lab_material_units WHERE entity_id=?",
        )
        .bind(entity_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(lab_material_unit_from_row).transpose()
    }

    pub async fn get_lab_location(&self, entity_id: &str) -> Result<Option<LabLocation>> {
        let row = sqlx::query(
            "SELECT entity_id,parent_location_id,location_class,single_occupancy FROM lab_locations WHERE entity_id=?",
        )
        .bind(entity_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(lab_location_from_row).transpose()
    }

    pub async fn lab_location_accepts_new_material(
        &self,
        registry_id: &str,
        location_id: &str,
    ) -> Result<bool> {
        let entity = self.get_lab_entity(location_id).await?;
        if entity.as_ref().map(|entity| entity.registry_id.as_str()) != Some(registry_id) {
            return Ok(false);
        }
        let Some(location) = self.get_lab_location(location_id).await? else {
            return Ok(false);
        };
        if !location.single_occupancy {
            return Ok(true);
        }
        let occupied: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM lab_material_locations WHERE location_id=? LIMIT 1")
                .bind(location_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(occupied.is_none())
    }

    pub async fn get_material_location(&self, material_unit_id: &str) -> Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT location_id FROM lab_material_locations WHERE material_unit_id=?",
        )
        .bind(material_unit_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(location_id,)| location_id))
    }

    pub async fn get_lab_document(&self, entity_id: &str) -> Result<Option<LabDocument>> {
        let row = sqlx::query(
            "SELECT id,registry_id,entity_id,relative_path,schema_version,narrative_markdown,extension_json,last_projected_content,revision,created_at,updated_at \
             FROM lab_documents WHERE entity_id=?",
        )
        .bind(entity_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(lab_document_from_row).transpose()
    }

    pub async fn preview_lab_document_import(
        &self,
        entity_id: &str,
        markdown: &str,
    ) -> Result<super::LabDocumentImportPreview> {
        let entity = self
            .get_lab_entity(entity_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Lab entity not found"))?;
        let document = self
            .get_lab_document(entity_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Lab dossier has not been registered"))?;
        let incoming = super::parse_lab_document(markdown)?;
        let mut conflicts = vec![];
        let Some(base_content) = document.last_projected_content.as_deref() else {
            anyhow::bail!("Dossier has no acknowledged base projection for three-way import");
        };
        let base = super::parse_lab_document(base_content)?;
        let current =
            super::parse_lab_document(&render_lab_document_projection(&entity, &document)?)?;
        let protected_changed = incoming.display_id != base.display_id
            || incoming.entity_id != base.entity_id
            || incoming.kind != base.kind
            || incoming.title != base.title
            || incoming.entity_revision != base.entity_revision
            || incoming.document_revision != base.document_revision
            || incoming.schema_version != base.schema_version;
        if protected_changed {
            conflicts.push("System-managed Frontmatter changed outside the domain service".into());
        }
        let merge_field =
            |name: &str, base: &str, incoming: &str, current: &str, conflicts: &mut Vec<String>| {
                if incoming != base && current != base && incoming != current {
                    conflicts.push(format!("Three-way conflict in {name}"));
                }
                if incoming != base {
                    incoming.to_string()
                } else {
                    current.to_string()
                }
            };
        let narrative_markdown = merge_field(
            "narrative",
            &base.narrative_markdown,
            &incoming.narrative_markdown,
            &current.narrative_markdown,
            &mut conflicts,
        );
        let merge_json = |name: &str,
                          base: &serde_json::Value,
                          incoming: &serde_json::Value,
                          current: &serde_json::Value,
                          conflicts: &mut Vec<String>| {
            if incoming != base && current != base && incoming != current {
                conflicts.push(format!("Three-way conflict in {name}"));
            }
            if incoming != base {
                incoming.clone()
            } else {
                current.clone()
            }
        };
        let extensions = merge_json(
            "extensions",
            &base.extensions,
            &incoming.extensions,
            &current.extensions,
            &mut conflicts,
        );
        let base_unknown = serde_json::Value::Object(base.unknown_frontmatter.clone());
        let incoming_unknown = serde_json::Value::Object(incoming.unknown_frontmatter.clone());
        let current_unknown = serde_json::Value::Object(current.unknown_frontmatter.clone());
        let unknown_frontmatter = match merge_json(
            "unknown Frontmatter",
            &base_unknown,
            &incoming_unknown,
            &current_unknown,
            &mut conflicts,
        ) {
            serde_json::Value::Object(value) => value,
            _ => unreachable!(),
        };
        let parsed = super::ParsedLabDocument {
            display_id: current.display_id,
            entity_id: current.entity_id,
            kind: current.kind,
            title: current.title,
            entity_revision: current.entity_revision,
            document_revision: current.document_revision,
            schema_version: current.schema_version,
            extensions,
            unknown_frontmatter,
            narrative_markdown,
        };
        let status = if markdown == base_content {
            "unchanged"
        } else if conflicts.is_empty() {
            "ready_to_import"
        } else {
            "conflict"
        };
        Ok(super::LabDocumentImportPreview {
            entity_id: entity.id,
            status: status.into(),
            conflicts,
            parsed,
        })
    }

    pub async fn list_lab_projection_outbox(&self) -> Result<Vec<super::LabProjectionOutboxItem>> {
        let rows = sqlx::query(
            "SELECT o.id,d.registry_id,o.document_id,o.target_path,o.content,o.attempts,o.last_error,o.created_at \
             FROM lab_projection_outbox o JOIN lab_documents d ON d.id=o.document_id ORDER BY o.created_at,o.id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(lab_projection_outbox_from_row)
            .collect()
    }

    pub async fn acknowledge_lab_projection(&self, outbox_id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT document_id,content FROM lab_projection_outbox WHERE id=?")
                .bind(outbox_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some((document_id, content)) = row else {
            anyhow::bail!("Projection outbox item not found");
        };
        sqlx::query("UPDATE lab_documents SET last_projected_content=? WHERE id=?")
            .bind(content)
            .bind(document_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM lab_projection_outbox WHERE id=?")
            .bind(outbox_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn fail_lab_projection(&self, outbox_id: &str, error: &str) -> Result<()> {
        sqlx::query("UPDATE lab_projection_outbox SET attempts=attempts+1,last_error=? WHERE id=?")
            .bind(error.chars().take(1_000).collect::<String>())
            .bind(outbox_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn create_wet_lab_run(
        &self,
        project_id: &str,
        registry_id: &str,
        command_id: &str,
        title: &str,
        frame_id: Option<&str>,
        operator: Option<&str>,
    ) -> Result<LabWetRun> {
        if title.trim().is_empty() || command_id.trim().is_empty() {
            anyhow::bail!("Wet-lab Run title and command ID are required");
        }
        let mut tx = self.pool.begin().await?;
        ensure_project_registry_link(&mut tx, project_id, registry_id).await?;
        if let Some(frame_id) = frame_id {
            ensure_root_conversation(&mut tx, project_id, frame_id).await?;
        }
        if let Some(row) = sqlx::query(
            "SELECT w.run_id,w.registry_id,w.display_id,w.command_id,w.operator,w.protocol_revision_id,w.deviations_json,w.created_at, \
                    r.project_id,r.frame_id \
             FROM lab_wet_runs w JOIN runs r ON r.id=w.run_id \
             WHERE w.registry_id=? AND w.command_id=?",
        )
        .bind(registry_id)
        .bind(command_id)
        .fetch_optional(&mut *tx)
        .await?
        {
            let existing_project_id: String = row.try_get("project_id")?;
            let existing_frame_id: Option<String> = row.try_get("frame_id")?;
            if existing_project_id != project_id {
                anyhow::bail!("Wet-lab command ID is already bound to a different Project");
            }
            if existing_frame_id.as_deref() != frame_id {
                anyhow::bail!("Wet-lab command ID is already bound to a different conversation");
            }
            tx.commit().await?;
            return lab_wet_run_from_row(row);
        }
        if let Some(frame_id) = frame_id {
            let existing: Option<(String,)> =
                sqlx::query_as("SELECT id FROM runs WHERE frame_id=? AND kind='wet_lab' LIMIT 1")
                    .bind(frame_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            if existing.is_some() {
                anyhow::bail!("Conversation already represents a wet-lab experiment");
            }
        }
        let mut run = RunRecord::new(
            uuid::Uuid::new_v4().to_string(),
            project_id,
            "",
            title.trim(),
            "wet_lab",
        );
        run.frame_id = frame_id.map(str::to_string);
        run.validate()?;
        let now = chrono::Utc::now().timestamp();
        let display_id = allocate_display_id(&mut tx, registry_id, "RUN").await?;
        sqlx::query(
            "INSERT INTO runs(\
                id,project_id,frame_id,context_id,title,kind,status,command,script_path,\
                input_refs_json,output_specs_json,created_at,started_at,ended_at,exit_code,\
                stdout_tail,stderr_tail,remote_workdir,remote_handle_json,timeout_secs,\
                last_polled_at,last_poll_error,env_snapshot_json\
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&run.id)
        .bind(&run.project_id)
        .bind(run.frame_id.as_deref())
        .bind(&run.context_id)
        .bind(&run.title)
        .bind(&run.kind)
        .bind(run.status.as_str())
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind("[]")
        .bind("[]")
        .bind(now)
        .bind(Option::<i64>::None)
        .bind(Option::<i64>::None)
        .bind(Option::<i64>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<i64>::None)
        .bind(Option::<i64>::None)
        .bind(Option::<String>::None)
        .bind("{}")
        .execute(&mut *tx)
        .await?;
        let wet_run = LabWetRun {
            run_id: run.id.clone(),
            registry_id: registry_id.to_string(),
            display_id,
            command_id: command_id.to_string(),
            operator: operator
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            protocol_revision_id: None,
            deviations_json: "[]".into(),
            created_at: now,
        };
        sqlx::query(
            "INSERT INTO lab_wet_runs(run_id,registry_id,display_id,command_id,operator,protocol_revision_id,deviations_json,created_at) \
             VALUES(?,?,?,?,?,?,?,?)",
        )
        .bind(&wet_run.run_id)
        .bind(&wet_run.registry_id)
        .bind(&wet_run.display_id)
        .bind(&wet_run.command_id)
        .bind(wet_run.operator.as_deref())
        .bind(Option::<String>::None)
        .bind(&wet_run.deviations_json)
        .bind(wet_run.created_at)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO research_nodes(id,project_id,kind,title,ref_id,metadata_json,created_at,updated_at) \
             VALUES(?,?,?,?,?,?,?,?)",
        )
        .bind(format!("run:{}", run.id))
        .bind(project_id)
        .bind("run")
        .bind(&run.title)
        .bind(&run.id)
        .bind("{}")
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(wet_run)
    }

    pub async fn get_wet_lab_run(&self, run_id: &str) -> Result<Option<LabWetRun>> {
        let row = sqlx::query(
            "SELECT run_id,registry_id,display_id,command_id,operator,protocol_revision_id,deviations_json,created_at \
             FROM lab_wet_runs WHERE run_id=?",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(lab_wet_run_from_row).transpose()
    }

    /// Return the one themed wet-lab experiment represented by a conversation.
    /// No implicit "most recent Run" fallback is allowed.
    pub async fn get_conversation_wet_lab_run(
        &self,
        project_id: &str,
        frame_id: &str,
    ) -> Result<Option<LabConversationRun>> {
        let run = sqlx::query(
            "SELECT id,project_id,frame_id,context_id,title,kind,status,command,script_path, \
                    input_refs_json,output_specs_json,created_at,started_at,ended_at,exit_code, \
                    stdout_tail,stderr_tail,remote_workdir,remote_handle_json,timeout_secs, \
                    last_polled_at,last_poll_error,env_snapshot_json \
             FROM runs WHERE project_id=? AND frame_id=? AND kind='wet_lab'",
        )
        .bind(project_id)
        .bind(frame_id)
        .fetch_optional(&self.pool)
        .await?
        .map(super::run_from_row)
        .transpose()?;
        let Some(run) = run else {
            return Ok(None);
        };
        let wet_lab_run = self
            .get_wet_lab_run(&run.id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Wet-lab Run sidecar is missing"))?;
        Ok(Some(LabConversationRun { run, wet_lab_run }))
    }

    pub async fn update_wet_lab_run_status(&self, run_id: &str, status: RunStatus) -> Result<bool> {
        if !matches!(
            status,
            RunStatus::Draft
                | RunStatus::Running
                | RunStatus::Succeeded
                | RunStatus::Failed
                | RunStatus::Cancelled
        ) {
            anyhow::bail!("Wet-lab Run status is not meaningful");
        }
        let run = self
            .get_run(run_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Run not found"))?;
        if run.kind != "wet_lab" || self.get_wet_lab_run(run_id).await?.is_none() {
            anyhow::bail!("Run is not a wet-lab Run");
        }
        super::validate_run_transition(run.status, status)?;
        let now = chrono::Utc::now().timestamp();
        let started_at = if status == RunStatus::Running && run.started_at.is_none() {
            Some(now)
        } else {
            run.started_at
        };
        let ended_at = if status.is_terminal() {
            Some(now)
        } else {
            run.ended_at
        };
        let mut tx = self.pool.begin().await?;
        let updated = sqlx::query(
            "UPDATE runs SET status=?,started_at=?,ended_at=?,lifecycle_owner=NULL,lifecycle_lease_until=NULL \
             WHERE id=? AND kind='wet_lab' AND status=?",
        )
        .bind(status.as_str())
        .bind(started_at)
        .bind(ended_at)
        .bind(run_id)
        .bind(run.status.as_str())
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() == 1 && status.is_terminal() {
            sqlx::query(
                "UPDATE lab_reservations SET status='released',released_at=? \
                 WHERE run_id=? AND status='active'",
            )
            .bind(now)
            .bind(run_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(updated.rows_affected() == 1)
    }

    pub async fn publish_lab_protocol_revision(
        &self,
        registry_id: &str,
        protocol_entity_id: &str,
        content: &str,
    ) -> Result<LabProtocolRevision> {
        if content.trim().is_empty() {
            anyhow::bail!("Protocol content is required");
        }
        let mut tx = self.pool.begin().await?;
        ensure_entity_kind_registry(
            &mut tx,
            protocol_entity_id,
            registry_id,
            LabEntityKind::ProtocolSource,
        )
        .await?;
        let checksum_sha256 = format!("{:x}", Sha256::digest(content.as_bytes()));
        if let Some(row) = sqlx::query(
            "SELECT id,registry_id,protocol_entity_id,revision_number,checksum_sha256,content,created_at \
             FROM lab_protocol_revisions WHERE protocol_entity_id=? AND checksum_sha256=?",
        )
        .bind(protocol_entity_id)
        .bind(&checksum_sha256)
        .fetch_optional(&mut *tx)
        .await?
        {
            tx.commit().await?;
            return lab_protocol_revision_from_row(row);
        }
        let next: (i64,) = sqlx::query_as(
            "SELECT COALESCE(MAX(revision_number),0)+1 FROM lab_protocol_revisions WHERE protocol_entity_id=?",
        )
        .bind(protocol_entity_id)
        .fetch_one(&mut *tx)
        .await?;
        let revision = LabProtocolRevision {
            id: uuid::Uuid::new_v4().to_string(),
            registry_id: registry_id.to_string(),
            protocol_entity_id: protocol_entity_id.to_string(),
            revision_number: next.0,
            checksum_sha256,
            content: content.to_string(),
            created_at: chrono::Utc::now().timestamp(),
        };
        sqlx::query(
            "INSERT INTO lab_protocol_revisions(id,registry_id,protocol_entity_id,revision_number,checksum_sha256,content,created_at) \
             VALUES(?,?,?,?,?,?,?)",
        )
        .bind(&revision.id)
        .bind(&revision.registry_id)
        .bind(&revision.protocol_entity_id)
        .bind(revision.revision_number)
        .bind(&revision.checksum_sha256)
        .bind(&revision.content)
        .bind(revision.created_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(revision)
    }

    pub async fn get_lab_protocol_revision(
        &self,
        protocol_entity_id: &str,
        revision_number: i64,
    ) -> Result<Option<LabProtocolRevision>> {
        let row = sqlx::query(
            "SELECT id,registry_id,protocol_entity_id,revision_number,checksum_sha256,content,created_at \
             FROM lab_protocol_revisions WHERE protocol_entity_id=? AND revision_number=?",
        )
        .bind(protocol_entity_id)
        .bind(revision_number)
        .fetch_optional(&self.pool)
        .await?;
        row.map(lab_protocol_revision_from_row).transpose()
    }

    pub async fn get_lab_protocol_revision_by_id(
        &self,
        id: &str,
    ) -> Result<Option<LabProtocolRevision>> {
        let row = sqlx::query("SELECT id,registry_id,protocol_entity_id,revision_number,checksum_sha256,content,created_at FROM lab_protocol_revisions WHERE id=?")
            .bind(id).fetch_optional(&self.pool).await?;
        row.map(lab_protocol_revision_from_row).transpose()
    }

    pub async fn pin_wet_lab_run_protocol(
        &self,
        project_id: &str,
        run_id: &str,
        protocol_revision_id: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let wet_run: Option<(String,)> = sqlx::query_as(
            "SELECT w.registry_id FROM lab_wet_runs w JOIN runs r ON r.id=w.run_id \
             WHERE w.run_id=? AND r.project_id=?",
        )
        .bind(run_id)
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((registry_id,)) = wet_run else {
            anyhow::bail!("Wet-lab Run is missing or belongs to another Project");
        };
        let status: String = sqlx::query_scalar("SELECT status FROM runs WHERE id=?")
            .bind(run_id)
            .fetch_one(&mut *tx)
            .await?;
        if status != RunStatus::Draft.as_str() {
            anyhow::bail!("Only a planned wet-lab Run can change its primary protocol");
        }
        let revision: Option<(String,)> =
            sqlx::query_as("SELECT registry_id FROM lab_protocol_revisions WHERE id=?")
                .bind(protocol_revision_id)
                .fetch_optional(&mut *tx)
                .await?;
        if revision.as_ref().map(|(registry_id,)| registry_id.as_str())
            != Some(registry_id.as_str())
        {
            anyhow::bail!("Protocol revision belongs to another registry or is missing");
        }
        let updated = sqlx::query(
            "UPDATE lab_wet_runs SET protocol_revision_id=? \
             WHERE run_id=? AND registry_id=? AND EXISTS (\
                SELECT 1 FROM runs r WHERE r.id=lab_wet_runs.run_id \
                AND r.project_id=? AND r.status='draft')",
        )
        .bind(protocol_revision_id)
        .bind(run_id)
        .bind(&registry_id)
        .bind(project_id)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() != 1 {
            anyhow::bail!("Wet-lab Run left draft state before its protocol was pinned");
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_wet_lab_run_participants(
        &self,
        run_id: &str,
    ) -> Result<Vec<LabRunParticipant>> {
        let rows = sqlx::query(
            "SELECT id,run_id,material_unit_id,direction,role,effect,quantity_state,quantity_value,quantity_unit,transformation_group,established_event_id,created_at \
             FROM lab_run_participants WHERE run_id=? ORDER BY created_at,id",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(lab_run_participant_from_row).collect()
    }

    pub async fn get_lab_subject(&self, entity_id: &str) -> Result<Option<LabSubject>> {
        let row = sqlx::query("SELECT entity_id,species,strain,sex,date_of_birth,origin_kind,established_event_id FROM lab_subjects WHERE entity_id=?")
            .bind(entity_id).fetch_optional(&self.pool).await?;
        row.map(|row| {
            Ok(LabSubject {
                entity_id: row.try_get("entity_id")?,
                species: row.try_get("species")?,
                strain: row.try_get("strain")?,
                sex: row.try_get("sex")?,
                date_of_birth: row.try_get("date_of_birth")?,
                origin_kind: row.try_get("origin_kind")?,
                established_event_id: row.try_get("established_event_id")?,
            })
        })
        .transpose()
    }

    pub async fn list_wet_lab_subject_participants(
        &self,
        run_id: &str,
    ) -> Result<Vec<LabSubjectParticipant>> {
        let rows = sqlx::query("SELECT id,run_id,subject_id,role,effect,established_event_id,created_at FROM lab_subject_participants WHERE run_id=? ORDER BY created_at,id")
            .bind(run_id).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| {
                Ok(LabSubjectParticipant {
                    id: row.try_get("id")?,
                    run_id: row.try_get("run_id")?,
                    subject_id: row.try_get("subject_id")?,
                    role: row.try_get("role")?,
                    effect: row.try_get("effect")?,
                    established_event_id: row.try_get("established_event_id")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn list_material_derivations(
        &self,
        material_unit_id: &str,
    ) -> Result<Vec<super::LabMaterialDerivation>> {
        let rows = sqlx::query(
            "SELECT id,run_id,operation,group_id,parent_material_unit_id,child_material_unit_id,established_event_id,created_at \
             FROM lab_material_derivations WHERE parent_material_unit_id=? OR child_material_unit_id=? ORDER BY created_at,id",
        )
        .bind(material_unit_id).bind(material_unit_id).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| {
                Ok(super::LabMaterialDerivation {
                    id: row.try_get("id")?,
                    run_id: row.try_get("run_id")?,
                    operation: row.try_get("operation")?,
                    group_id: row.try_get("group_id")?,
                    parent_material_unit_id: row.try_get("parent_material_unit_id")?,
                    child_material_unit_id: row.try_get("child_material_unit_id")?,
                    established_event_id: row.try_get("established_event_id")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn list_lab_run_amendments(&self, run_id: &str) -> Result<Vec<LabAmendment>> {
        let rows = sqlx::query("SELECT id,display_id,registry_id,run_id,original_event_id,reason,correction_json,affected_ids_json,established_event_id,created_at FROM lab_amendments WHERE run_id=? ORDER BY created_at,id")
            .bind(run_id).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| {
                Ok(LabAmendment {
                    id: row.try_get("id")?,
                    display_id: row.try_get("display_id")?,
                    registry_id: row.try_get("registry_id")?,
                    run_id: row.try_get("run_id")?,
                    original_event_id: row.try_get("original_event_id")?,
                    reason: row.try_get("reason")?,
                    correction_json: row.try_get("correction_json")?,
                    affected_ids: serde_json::from_str(
                        &row.try_get::<String, _>("affected_ids_json")?,
                    )?,
                    established_event_id: row.try_get("established_event_id")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn list_lab_run_deviations(&self, run_id: &str) -> Result<Vec<LabRunDeviation>> {
        let rows = sqlx::query(
            "SELECT id,run_id,step_ref,description,impact,disposition,occurred_at,recorded_at,established_event_id \
             FROM lab_run_deviations WHERE run_id=? ORDER BY occurred_at,id",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(LabRunDeviation {
                    id: row.try_get("id")?,
                    run_id: row.try_get("run_id")?,
                    step_ref: row.try_get("step_ref")?,
                    description: row.try_get("description")?,
                    impact: row.try_get("impact")?,
                    disposition: row.try_get("disposition")?,
                    occurred_at: row.try_get("occurred_at")?,
                    recorded_at: row.try_get("recorded_at")?,
                    established_event_id: row.try_get("established_event_id")?,
                })
            })
            .collect()
    }

    pub async fn wet_lab_run_closeout_summary(
        &self,
        run_id: &str,
    ) -> Result<LabRunCloseoutSummary> {
        let run = self
            .get_run(run_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Run not found"))?;
        let wet_run = self
            .get_wet_lab_run(run_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Run is not a wet-lab Run"))?;
        let (input_count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM lab_run_participants WHERE run_id=? AND direction='input'",
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await?;
        let (output_count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM lab_run_participants WHERE run_id=? AND direction='output'",
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await?;
        let (subject_participant_count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM lab_subject_participants WHERE run_id=?")
                .bind(run_id)
                .fetch_one(&self.pool)
                .await?;
        let unlocated_output_ids: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT p.material_unit_id FROM lab_run_participants p \
             LEFT JOIN lab_material_locations l ON l.material_unit_id=p.material_unit_id \
             WHERE p.run_id=? AND p.direction='output' AND l.material_unit_id IS NULL \
             ORDER BY p.material_unit_id",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        let deviations = self.list_lab_run_deviations(run_id).await?;
        let unresolved_deviation_ids = deviations
            .iter()
            .filter(|deviation| {
                deviation
                    .disposition
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty())
            })
            .map(|deviation| deviation.id.clone())
            .collect::<Vec<_>>();
        let (qc_observation_count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM lab_qc_observations WHERE run_id=?")
                .bind(run_id)
                .fetch_one(&self.pool)
                .await?;
        let (qc_assessment_count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM lab_qc_assessments a WHERE EXISTS (\
             SELECT 1 FROM lab_qc_observations o \
             WHERE o.run_id=? AND o.registry_id=a.registry_id AND o.entity_id=a.entity_id)",
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await?;
        let (active_reservation_count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM lab_reservations WHERE run_id=? AND status='active'",
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await?;
        let data_evidence = self.list_lab_run_data_evidence(run_id).await?;
        let evidence_without_checksum_ids = data_evidence
            .iter()
            .filter(|evidence| evidence.checksum_sha256.is_none())
            .map(|evidence| evidence.display_id.clone())
            .collect::<Vec<_>>();
        let mut issues = vec![];
        let amendment_count = self.list_lab_run_amendments(run_id).await?.len() as i64;
        if wet_run.protocol_revision_id.is_none() {
            issues.push("protocol_not_pinned".into());
        }
        if !unlocated_output_ids.is_empty() {
            issues.push("outputs_without_location".into());
        }
        if !unresolved_deviation_ids.is_empty() {
            issues.push("deviations_without_disposition".into());
        }
        if active_reservation_count > 0 {
            issues.push("active_reservations_will_be_released".into());
        }
        if !evidence_without_checksum_ids.is_empty() {
            issues.push("data_evidence_without_checksum".into());
        }
        Ok(LabRunCloseoutSummary {
            run_id: run.id,
            status: run.status,
            protocol_revision_id: wet_run.protocol_revision_id,
            input_count,
            subject_participant_count,
            output_count,
            unlocated_output_ids,
            deviation_count: deviations.len() as i64,
            unresolved_deviation_ids,
            qc_observation_count,
            qc_assessment_count,
            data_evidence_count: data_evidence.len() as i64,
            evidence_without_checksum_ids,
            active_reservation_count,
            amendment_count,
            issues,
        })
    }

    pub async fn list_lab_run_data_evidence(&self, run_id: &str) -> Result<Vec<LabDataEvidence>> {
        let rows = sqlx::query(
            "SELECT id,display_id,registry_id,owner_project_id,owner_registry_id,producing_run_id,role,uri,format,size_bytes,checksum_sha256,origin,manifest_json,created_at,established_event_id \
             FROM lab_data_evidence WHERE producing_run_id=? ORDER BY created_at,id",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(LabDataEvidence {
                    id: row.try_get("id")?,
                    display_id: row.try_get("display_id")?,
                    registry_id: row.try_get("registry_id")?,
                    owner_project_id: row.try_get("owner_project_id")?,
                    owner_registry_id: row.try_get("owner_registry_id")?,
                    producing_run_id: row.try_get("producing_run_id")?,
                    role: row.try_get("role")?,
                    uri: row.try_get("uri")?,
                    format: row.try_get("format")?,
                    size_bytes: row.try_get("size_bytes")?,
                    checksum_sha256: row.try_get("checksum_sha256")?,
                    origin: row.try_get("origin")?,
                    manifest_json: row.try_get("manifest_json")?,
                    created_at: row.try_get("created_at")?,
                    established_event_id: row.try_get("established_event_id")?,
                })
            })
            .collect()
    }

    pub async fn lab_run_provenance(&self, run_id: &str) -> Result<LabRunProvenance> {
        let run = self
            .get_run(run_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Run not found"))?;
        let wet_lab_run = self
            .get_wet_lab_run(run_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Run is not a wet-lab Run"))?;
        let protocol_revision = match wet_lab_run.protocol_revision_id.as_deref() {
            Some(id) => sqlx::query(
                "SELECT id,registry_id,protocol_entity_id,revision_number,checksum_sha256,content,created_at FROM lab_protocol_revisions WHERE id=?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .map(lab_protocol_revision_from_row)
            .transpose()?,
            None => None,
        };
        let observations = sqlx::query(
            "SELECT id,registry_id,entity_id,run_id,method_revision_id,measurement_json,evidence_json,observed_at,recorded_at FROM lab_qc_observations WHERE run_id=? ORDER BY observed_at,id",
        )
        .bind(run_id).fetch_all(&self.pool).await?.into_iter().map(|row| Ok(LabQcObservation {
            id: row.try_get("id")?, registry_id: row.try_get("registry_id")?, entity_id: row.try_get("entity_id")?,
            run_id: row.try_get("run_id")?, method_revision_id: row.try_get("method_revision_id")?,
            measurement_json: row.try_get("measurement_json")?, evidence_json: row.try_get("evidence_json")?,
            observed_at: row.try_get("observed_at")?, recorded_at: row.try_get("recorded_at")?,
        })).collect::<Result<Vec<_>>>()?;
        let assessments = sqlx::query(
            "SELECT DISTINCT a.id,a.registry_id,a.entity_id,a.observation_ids_json,a.criteria_json,a.verdict,a.rationale,a.created_at \
             FROM lab_qc_assessments a JOIN lab_qc_observations o ON o.registry_id=a.registry_id AND o.entity_id=a.entity_id \
             WHERE o.run_id=? ORDER BY a.created_at,a.id",
        )
        .bind(run_id).fetch_all(&self.pool).await?.into_iter().map(|row| Ok(LabQcAssessment {
            id: row.try_get("id")?, registry_id: row.try_get("registry_id")?, entity_id: row.try_get("entity_id")?,
            observation_ids: serde_json::from_str(&row.try_get::<String,_>("observation_ids_json")?)?,
            criteria_json: row.try_get("criteria_json")?, verdict: row.try_get("verdict")?, rationale: row.try_get("rationale")?, created_at: row.try_get("created_at")?,
        })).collect::<Result<Vec<_>>>()?;
        Ok(LabRunProvenance {
            run,
            wet_lab_run,
            protocol_revision,
            participants: self.list_wet_lab_run_participants(run_id).await?,
            subject_participants: self.list_wet_lab_subject_participants(run_id).await?,
            deviations: self.list_lab_run_deviations(run_id).await?,
            raw_evidence: self.list_lab_run_data_evidence(run_id).await?,
            observations,
            assessments,
            amendments: self.list_lab_run_amendments(run_id).await?,
            decisions: {
                let rows = sqlx::query("SELECT n.id,n.project_id,n.kind,n.title,n.ref_id,n.metadata_json,n.created_at,n.updated_at \
                    FROM research_nodes n JOIN research_edges e ON e.source_id=n.id WHERE e.target_id=? AND e.relation='concludes' ORDER BY n.created_at,n.id")
                    .bind(format!("run:{run_id}")).fetch_all(&self.pool).await?;
                rows.into_iter()
                    .map(super::research_node_from_row)
                    .collect::<Result<Vec<_>>>()?
            },
            closeout: self.wet_lab_run_closeout_summary(run_id).await?,
        })
    }

    pub async fn list_active_material_reservations(
        &self,
        material_unit_id: &str,
    ) -> Result<Vec<LabReservation>> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "UPDATE lab_reservations SET status='expired',released_at=? \
             WHERE material_unit_id=? AND status='active' AND expires_at IS NOT NULL AND expires_at<=?",
        )
        .bind(now)
        .bind(material_unit_id)
        .bind(now)
        .execute(&self.pool)
        .await?;
        let rows = sqlx::query(
            "SELECT id,run_id,material_unit_id,quantity_value,quantity_unit,status,expires_at,created_at,released_at \
             FROM lab_reservations WHERE material_unit_id=? AND status='active' ORDER BY created_at,id",
        )
        .bind(material_unit_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(lab_reservation_from_row).collect()
    }

    pub async fn lab_entity_provenance(
        &self,
        entity_id: &str,
    ) -> Result<super::LabEntityProvenance> {
        let entity = self
            .get_lab_entity(entity_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Lab entity not found"))?;
        let location_id = if entity.kind == LabEntityKind::MaterialUnit {
            self.get_material_location(entity_id).await?
        } else {
            None
        };
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT p.direction,w.display_id FROM lab_run_participants p \
             JOIN lab_wet_runs w ON w.run_id=p.run_id WHERE p.material_unit_id=? ORDER BY p.created_at,p.id",
        )
        .bind(entity_id)
        .fetch_all(&self.pool)
        .await?;
        let producing_runs = rows
            .iter()
            .filter(|(direction, _)| direction == "output")
            .map(|(_, display_id)| display_id.clone())
            .collect();
        let consuming_runs = rows
            .iter()
            .filter(|(direction, _)| direction == "input")
            .map(|(_, display_id)| display_id.clone())
            .collect();
        let observation_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM lab_qc_observations WHERE entity_id=?")
                .bind(entity_id)
                .fetch_one(&self.pool)
                .await?;
        let verdict: Option<(String,)> = sqlx::query_as(
            "SELECT verdict FROM lab_qc_assessments WHERE entity_id=? ORDER BY created_at DESC,id DESC LIMIT 1",
        )
        .bind(entity_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(super::LabEntityProvenance {
            entity,
            location_id,
            producing_runs,
            consuming_runs,
            qc_observation_count: observation_count.0,
            latest_qc_verdict: verdict.map(|(verdict,)| verdict),
        })
    }
}

async fn insert_lab_location_tx(
    tx: &mut Transaction<'_, Sqlite>,
    registry_id: &str,
    entity_id: &str,
    location: &LabLocationCreate,
) -> Result<()> {
    location.validate()?;
    if let Some(parent_location_id) = location.parent_location_id.as_deref() {
        let _ = location_occupancy_tx(tx, parent_location_id, registry_id).await?;
    }
    sqlx::query(
        "INSERT INTO lab_locations(entity_id,registry_id,parent_location_id,location_class,single_occupancy) VALUES(?,?,?,?,?)",
    )
    .bind(entity_id)
    .bind(registry_id)
    .bind(location.parent_location_id.as_deref())
    .bind(location.location_class.trim())
    .bind(location.single_occupancy)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn get_entity_in_registry_tx(
    tx: &mut Transaction<'_, Sqlite>,
    entity_id: &str,
    registry_id: &str,
) -> Result<LabEntity> {
    let row = sqlx::query(
        "SELECT id,registry_id,display_id,kind,subtype,title,revision,metadata_json,created_at,updated_at \
         FROM lab_entities WHERE id=? AND registry_id=?",
    )
    .bind(entity_id)
    .bind(registry_id)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(lab_entity_from_row)
        .transpose()?
        .ok_or_else(|| anyhow::anyhow!("Lab entity is missing or belongs to another registry"))
}

async fn upsert_lab_document_tx(
    tx: &mut Transaction<'_, Sqlite>,
    entity: &LabEntity,
    registry_id: &str,
    upsert: &LabDocumentUpsert,
    now: i64,
) -> Result<LabDocument> {
    upsert.validate()?;
    let existing = sqlx::query("SELECT id,revision FROM lab_documents WHERE entity_id=?")
        .bind(&upsert.entity_id)
        .fetch_optional(&mut **tx)
        .await?;
    let (id, revision, created_at) = match existing {
        Some(row) => {
            let id: String = row.try_get("id")?;
            let current_revision: i64 = row.try_get("revision")?;
            if upsert.expected_revision != Some(current_revision) {
                anyhow::bail!("Lab document revision conflict");
            }
            (id, current_revision + 1, now)
        }
        None => {
            if upsert.expected_revision.is_some() {
                anyhow::bail!("Lab document does not yet exist");
            }
            (uuid::Uuid::new_v4().to_string(), 1, now)
        }
    };
    let document = LabDocument {
        id,
        registry_id: registry_id.to_string(),
        entity_id: entity.id.clone(),
        relative_path: upsert.relative_path.replace('\\', "/"),
        schema_version: 1,
        narrative_markdown: upsert.narrative_markdown.clone(),
        extension_json: upsert.extension_json.clone(),
        last_projected_content: None,
        revision,
        created_at,
        updated_at: now,
    };
    let content = render_lab_document_projection(entity, &document)?;
    if revision == 1 {
        sqlx::query(
            "INSERT INTO lab_documents(id,registry_id,entity_id,relative_path,schema_version,narrative_markdown,extension_json,last_projected_content,revision,created_at,updated_at) \
             VALUES(?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&document.id)
        .bind(&document.registry_id)
        .bind(&document.entity_id)
        .bind(&document.relative_path)
        .bind(document.schema_version)
        .bind(&document.narrative_markdown)
        .bind(&document.extension_json)
        .bind(Option::<String>::None)
        .bind(document.revision)
        .bind(document.created_at)
        .bind(document.updated_at)
        .execute(&mut **tx)
        .await?;
    } else {
        let affected = sqlx::query(
            "UPDATE lab_documents SET relative_path=?,narrative_markdown=?,extension_json=?,revision=?,updated_at=? \
             WHERE id=? AND revision=?",
        )
        .bind(&document.relative_path)
        .bind(&document.narrative_markdown)
        .bind(&document.extension_json)
        .bind(document.revision)
        .bind(document.updated_at)
        .bind(&document.id)
        .bind(document.revision - 1)
        .execute(&mut **tx)
        .await?;
        if affected.rows_affected() != 1 {
            anyhow::bail!("Lab document revision conflict");
        }
    }
    sqlx::query("DELETE FROM lab_projection_outbox WHERE document_id=?")
        .bind(&document.id)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "INSERT INTO lab_projection_outbox(id,document_id,target_path,content,attempts,last_error,created_at) \
         VALUES(?,?,?,?,0,NULL,?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(&document.id)
    .bind(&document.relative_path)
    .bind(content)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(document)
}

fn render_lab_document_projection(entity: &LabEntity, document: &LabDocument) -> Result<String> {
    let extensions: serde_json::Value = serde_json::from_str(&document.extension_json)?;
    let frontmatter = serde_json::json!({
        "id": entity.display_id,
        "entity_id": entity.id,
        "kind": entity.kind,
        "title": entity.title,
        "entity_revision": entity.revision,
        "document_revision": document.revision,
        "schema_version": document.schema_version,
        "extensions": extensions,
    });
    let yaml = serde_yaml::to_string(&frontmatter)?;
    Ok(format!("---\n{yaml}---\n\n{}", document.narrative_markdown))
}

async fn material_location_tx(
    tx: &mut Transaction<'_, Sqlite>,
    material_unit_id: &str,
) -> Result<Option<String>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT location_id FROM lab_material_locations WHERE material_unit_id=?")
            .bind(material_unit_id)
            .fetch_optional(&mut **tx)
            .await?;
    Ok(row.map(|(location_id,)| location_id))
}

async fn place_new_material_tx(
    tx: &mut Transaction<'_, Sqlite>,
    transaction: &LabTransaction,
    material_unit_id: &str,
    location_id: &str,
    occurred_at: i64,
    recorded_at: i64,
    sequence: i64,
) -> Result<LabEvent> {
    let single = location_occupancy_tx(tx, location_id, &transaction.registry_id).await?;
    if single {
        let occupied: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM lab_material_locations WHERE location_id=? LIMIT 1")
                .bind(location_id)
                .fetch_optional(&mut **tx)
                .await?;
        if occupied.is_some() {
            anyhow::bail!("Single-occupancy location is already occupied");
        }
    }
    let event = LabEvent {
        id: uuid::Uuid::new_v4().to_string(),
        registry_id: transaction.registry_id.clone(),
        project_id: transaction.project_id.clone(),
        transaction_id: transaction.id.clone(),
        sequence,
        entity_id: Some(material_unit_id.to_string()),
        prior_event_id: None,
        kind: "material_placed".into(),
        schema_version: 1,
        payload_json: json!({"location_id":location_id}).to_string(),
        occurred_at,
        recorded_at,
        expected_revision: None,
        resulting_revision: Some(1),
        reason: None,
    };
    insert_lab_event_tx(tx, &event).await?;
    sqlx::query("INSERT INTO lab_material_locations(material_unit_id,location_id,established_event_id,updated_at) VALUES(?,?,?,?)")
        .bind(material_unit_id).bind(location_id).bind(&event.id).bind(recorded_at).execute(&mut **tx).await?;
    Ok(event)
}

async fn location_occupancy_tx(
    tx: &mut Transaction<'_, Sqlite>,
    location_id: &str,
    registry_id: &str,
) -> Result<bool> {
    let row: Option<(bool,)> = sqlx::query_as(
        "SELECT single_occupancy FROM lab_locations WHERE entity_id=? AND registry_id=?",
    )
    .bind(location_id)
    .bind(registry_id)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|(single_occupancy,)| single_occupancy)
        .ok_or_else(|| anyhow::anyhow!("Location is missing or belongs to another registry"))
}

async fn insert_lab_lot_tx(
    tx: &mut Transaction<'_, Sqlite>,
    registry_id: &str,
    entity_id: &str,
    lot: &LabLotCreate,
) -> Result<()> {
    lot.validate()?;
    ensure_entity_kind_registry(
        tx,
        &lot.resource_definition_id,
        registry_id,
        LabEntityKind::ResourceDefinition,
    )
    .await?;
    sqlx::query(
        "INSERT INTO lab_lots(entity_id,registry_id,resource_definition_id,supplier,catalog_number,lot_number,received_at,expiry_at,origin_kind) \
         VALUES(?,?,?,?,?,?,?,?,?)",
    )
    .bind(entity_id)
    .bind(registry_id)
    .bind(&lot.resource_definition_id)
    .bind(lot.supplier.as_deref().map(str::trim).filter(|value| !value.is_empty()))
    .bind(lot.catalog_number.as_deref().map(str::trim).filter(|value| !value.is_empty()))
    .bind(lot.lot_number.trim())
    .bind(lot.received_at)
    .bind(lot.expiry_at)
    .bind(&lot.origin_kind)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_lab_material_unit_tx(
    tx: &mut Transaction<'_, Sqlite>,
    registry_id: &str,
    entity_id: &str,
    material: &LabMaterialUnitCreate,
) -> Result<()> {
    material.validate()?;
    if let Some(lot_id) = material.lot_id.as_deref() {
        ensure_entity_kind_registry(tx, lot_id, registry_id, LabEntityKind::Lot).await?;
    }
    sqlx::query(
        "INSERT INTO lab_material_units(entity_id,registry_id,lot_id,usage_class,quantity_state,quantity_value,quantity_unit,vessel_description,lifecycle,availability,identity_state,origin_kind) \
         VALUES(?,?,?,?,?,?,?,?,'active',?,'verified',?)",
    )
    .bind(entity_id)
    .bind(registry_id)
    .bind(material.lot_id.as_deref())
    .bind(&material.usage_class)
    .bind(quantity_storage_state(&material.quantity))
    .bind(material.quantity.value.as_deref())
    .bind(material.quantity.unit.as_deref())
    .bind(material.vessel_description.as_deref().map(str::trim).filter(|value| !value.is_empty()))
    .bind(&material.availability)
    .bind(&material.origin_kind)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn quantity_storage_state(quantity: &LabQuantity) -> &'static str {
    match quantity.state {
        LabQuantityState::Measured => "measured",
        LabQuantityState::Unknown => "unknown",
        LabQuantityState::NotMeasured => "not_measured",
    }
}

fn decimal(value: &str) -> Result<Decimal> {
    Decimal::from_str(value).map_err(|_| anyhow::anyhow!("Invalid canonical decimal quantity"))
}

async fn insert_resource_definition_tx(
    tx: &mut Transaction<'_, Sqlite>,
    entity_id: &str,
    definition: &super::LabResourceDefinitionCreate,
) -> Result<()> {
    let definition = LabResourceDefinition {
        entity_id: entity_id.to_string(),
        category: definition.category.trim().to_string(),
        supplier: definition
            .supplier
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        catalog_number: definition
            .catalog_number
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        attributes_json: definition.attributes_json.clone(),
    };
    definition.validate()?;
    sqlx::query(
        "INSERT INTO lab_resource_definitions(entity_id,category,supplier,catalog_number,attributes_json) \
         VALUES(?,?,?,?,?)",
    )
    .bind(&definition.entity_id)
    .bind(&definition.category)
    .bind(definition.supplier.as_deref())
    .bind(definition.catalog_number.as_deref())
    .bind(&definition.attributes_json)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_lab_alias_tx(
    tx: &mut Transaction<'_, Sqlite>,
    registry_id: &str,
    entity_id: &str,
    alias: &LabAliasCreate,
    created_at: i64,
) -> Result<()> {
    alias.validate()?;
    let alias_type = alias.alias_type.trim();
    let namespace = alias
        .namespace
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let value = alias.value.trim();
    sqlx::query(
        "INSERT INTO lab_aliases(id,registry_id,entity_id,alias_type,namespace,value,created_at) \
         VALUES(?,?,?,?,?,?,?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(registry_id)
    .bind(entity_id)
    .bind(alias_type)
    .bind(namespace)
    .bind(value)
    .bind(created_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn lab_resource_definition_from_row(row: sqlx::sqlite::SqliteRow) -> Result<LabResourceDefinition> {
    Ok(LabResourceDefinition {
        entity_id: row.try_get("entity_id")?,
        category: row.try_get("category")?,
        supplier: row.try_get("supplier")?,
        catalog_number: row.try_get("catalog_number")?,
        attributes_json: row.try_get("attributes_json")?,
    })
}

fn lab_alias_from_row(row: sqlx::sqlite::SqliteRow) -> Result<LabAlias> {
    Ok(LabAlias {
        id: row.try_get("id")?,
        registry_id: row.try_get("registry_id")?,
        entity_id: row.try_get("entity_id")?,
        alias_type: row.try_get("alias_type")?,
        namespace: row.try_get("namespace")?,
        value: row.try_get("value")?,
        created_at: row.try_get("created_at")?,
    })
}

fn lab_lot_from_row(row: sqlx::sqlite::SqliteRow) -> Result<LabLot> {
    Ok(LabLot {
        entity_id: row.try_get("entity_id")?,
        resource_definition_id: row.try_get("resource_definition_id")?,
        supplier: row.try_get("supplier")?,
        catalog_number: row.try_get("catalog_number")?,
        lot_number: row.try_get("lot_number")?,
        received_at: row.try_get("received_at")?,
        expiry_at: row.try_get("expiry_at")?,
        origin_kind: row.try_get("origin_kind")?,
    })
}

fn lab_material_unit_from_row(row: sqlx::sqlite::SqliteRow) -> Result<LabMaterialUnit> {
    let state = match row.try_get::<String, _>("quantity_state")?.as_str() {
        "measured" => LabQuantityState::Measured,
        "unknown" => LabQuantityState::Unknown,
        "not_measured" => LabQuantityState::NotMeasured,
        _ => anyhow::bail!("Unknown lab quantity state"),
    };
    let quantity = LabQuantity {
        state,
        value: row.try_get("quantity_value")?,
        unit: row.try_get("quantity_unit")?,
    };
    quantity.validate()?;
    Ok(LabMaterialUnit {
        entity_id: row.try_get("entity_id")?,
        lot_id: row.try_get("lot_id")?,
        usage_class: row.try_get("usage_class")?,
        quantity,
        vessel_description: row.try_get("vessel_description")?,
        lifecycle: row.try_get("lifecycle")?,
        availability: row.try_get("availability")?,
        identity_state: row.try_get("identity_state")?,
        origin_kind: row.try_get("origin_kind")?,
    })
}

fn lab_location_from_row(row: sqlx::sqlite::SqliteRow) -> Result<LabLocation> {
    Ok(LabLocation {
        entity_id: row.try_get("entity_id")?,
        parent_location_id: row.try_get("parent_location_id")?,
        location_class: row.try_get("location_class")?,
        single_occupancy: row.try_get("single_occupancy")?,
    })
}

fn lab_document_from_row(row: sqlx::sqlite::SqliteRow) -> Result<LabDocument> {
    Ok(LabDocument {
        id: row.try_get("id")?,
        registry_id: row.try_get("registry_id")?,
        entity_id: row.try_get("entity_id")?,
        relative_path: row.try_get("relative_path")?,
        schema_version: row.try_get("schema_version")?,
        narrative_markdown: row.try_get("narrative_markdown")?,
        extension_json: row.try_get("extension_json")?,
        last_projected_content: row.try_get("last_projected_content")?,
        revision: row.try_get("revision")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn lab_projection_outbox_from_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<super::LabProjectionOutboxItem> {
    Ok(super::LabProjectionOutboxItem {
        id: row.try_get("id")?,
        registry_id: row.try_get("registry_id")?,
        document_id: row.try_get("document_id")?,
        target_path: row.try_get("target_path")?,
        content: row.try_get("content")?,
        attempts: row.try_get("attempts")?,
        last_error: row.try_get("last_error")?,
        created_at: row.try_get("created_at")?,
    })
}

fn lab_wet_run_from_row(row: sqlx::sqlite::SqliteRow) -> Result<LabWetRun> {
    Ok(LabWetRun {
        run_id: row.try_get("run_id")?,
        registry_id: row.try_get("registry_id")?,
        display_id: row.try_get("display_id")?,
        command_id: row.try_get("command_id")?,
        operator: row.try_get("operator")?,
        protocol_revision_id: row.try_get("protocol_revision_id")?,
        deviations_json: row.try_get("deviations_json")?,
        created_at: row.try_get("created_at")?,
    })
}

fn lab_protocol_revision_from_row(row: sqlx::sqlite::SqliteRow) -> Result<LabProtocolRevision> {
    Ok(LabProtocolRevision {
        id: row.try_get("id")?,
        registry_id: row.try_get("registry_id")?,
        protocol_entity_id: row.try_get("protocol_entity_id")?,
        revision_number: row.try_get("revision_number")?,
        checksum_sha256: row.try_get("checksum_sha256")?,
        content: row.try_get("content")?,
        created_at: row.try_get("created_at")?,
    })
}

fn lab_run_participant_from_row(row: sqlx::sqlite::SqliteRow) -> Result<LabRunParticipant> {
    let quantity_state: Option<String> = row.try_get("quantity_state")?;
    let quantity = match quantity_state.as_deref() {
        None => None,
        Some("measured") => Some(LabQuantity {
            state: LabQuantityState::Measured,
            value: row.try_get("quantity_value")?,
            unit: row.try_get("quantity_unit")?,
        }),
        Some("unknown") => Some(LabQuantity {
            state: LabQuantityState::Unknown,
            value: None,
            unit: None,
        }),
        Some("not_measured") => Some(LabQuantity {
            state: LabQuantityState::NotMeasured,
            value: None,
            unit: None,
        }),
        Some(_) => anyhow::bail!("Unknown run participant quantity state"),
    };
    if let Some(quantity) = &quantity {
        quantity.validate()?;
    }
    Ok(LabRunParticipant {
        id: row.try_get("id")?,
        run_id: row.try_get("run_id")?,
        material_unit_id: row.try_get("material_unit_id")?,
        direction: row.try_get("direction")?,
        role: row.try_get("role")?,
        effect: row.try_get("effect")?,
        quantity,
        transformation_group: row.try_get("transformation_group")?,
        established_event_id: row.try_get("established_event_id")?,
        created_at: row.try_get("created_at")?,
    })
}

fn lab_reservation_from_row(row: sqlx::sqlite::SqliteRow) -> Result<LabReservation> {
    let quantity = LabQuantity::measured(
        row.try_get::<String, _>("quantity_value")?,
        row.try_get::<String, _>("quantity_unit")?,
    );
    quantity.validate()?;
    Ok(LabReservation {
        id: row.try_get("id")?,
        run_id: row.try_get("run_id")?,
        material_unit_id: row.try_get("material_unit_id")?,
        quantity,
        status: row.try_get("status")?,
        expires_at: row.try_get("expires_at")?,
        created_at: row.try_get("created_at")?,
        released_at: row.try_get("released_at")?,
    })
}

async fn create_entity_tx(
    tx: &mut Transaction<'_, Sqlite>,
    transaction: &LabTransaction,
    create: &super::LabEntityCreate,
    occurred_at: i64,
    recorded_at: i64,
) -> Result<LabEntity> {
    let display_id = allocate_display_id(tx, &transaction.registry_id, &create.prefix).await?;
    let entity = LabEntity {
        id: uuid::Uuid::new_v4().to_string(),
        registry_id: transaction.registry_id.clone(),
        display_id,
        kind: create.kind,
        subtype: create
            .subtype
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        title: create.title.trim().to_owned(),
        revision: 1,
        metadata_json: create.metadata_json.clone(),
        created_at: occurred_at,
        updated_at: recorded_at,
    };
    entity.validate()?;
    sqlx::query(
        "INSERT INTO lab_entities(\
            id,registry_id,display_id,kind,subtype,title,revision,metadata_json,created_at,updated_at,last_transaction_id\
         ) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&entity.id)
    .bind(&entity.registry_id)
    .bind(&entity.display_id)
    .bind(entity.kind.as_str())
    .bind(entity.subtype.as_deref())
    .bind(&entity.title)
    .bind(entity.revision)
    .bind(&entity.metadata_json)
    .bind(entity.created_at)
    .bind(entity.updated_at)
    .bind(&transaction.id)
    .execute(&mut **tx)
    .await?;
    Ok(entity)
}

async fn insert_lab_event_tx(tx: &mut Transaction<'_, Sqlite>, event: &LabEvent) -> Result<()> {
    sqlx::query(
        "INSERT INTO lab_events(\
            id,registry_id,project_id,transaction_id,sequence,entity_id,prior_event_id,kind,\
            schema_version,payload_json,occurred_at,recorded_at,expected_revision,resulting_revision,reason\
         ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&event.id)
    .bind(&event.registry_id)
    .bind(event.project_id.as_deref())
    .bind(&event.transaction_id)
    .bind(event.sequence)
    .bind(event.entity_id.as_deref())
    .bind(event.prior_event_id.as_deref())
    .bind(&event.kind)
    .bind(event.schema_version)
    .bind(&event.payload_json)
    .bind(event.occurred_at)
    .bind(event.recorded_at)
    .bind(event.expected_revision)
    .bind(event.resulting_revision)
    .bind(event.reason.as_deref())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn create_lab_entity_with_event_tx(
    tx: &mut Transaction<'_, Sqlite>,
    transaction: &LabTransaction,
    create: &LabEntityCreate,
    occurred_at: i64,
    recorded_at: i64,
    sequence: i64,
) -> Result<(LabEntity, LabEvent)> {
    let entity = create_entity_tx(tx, transaction, create, occurred_at, recorded_at).await?;
    if let Some(definition) = create.resource_definition.as_ref() {
        insert_resource_definition_tx(tx, &entity.id, definition).await?;
    }
    if let Some(lot) = create.lot.as_ref() {
        insert_lab_lot_tx(tx, &transaction.registry_id, &entity.id, lot).await?;
    }
    if let Some(material) = create.material_unit.as_ref() {
        insert_lab_material_unit_tx(tx, &transaction.registry_id, &entity.id, material).await?;
    }
    if let Some(location) = create.location.as_ref() {
        insert_lab_location_tx(tx, &transaction.registry_id, &entity.id, location).await?;
    }
    for alias in &create.aliases {
        insert_lab_alias_tx(tx, &transaction.registry_id, &entity.id, alias, recorded_at).await?;
    }
    if let Some(relation) = create.project_relation.as_deref() {
        let project_id = transaction.project_id.as_deref().ok_or_else(|| {
            anyhow::anyhow!("Lab entity project relation requires a transaction project_id")
        })?;
        if relation.trim().is_empty() {
            anyhow::bail!("Lab entity project relation is required");
        }
        sqlx::query(
            "INSERT OR IGNORE INTO lab_entity_projects(entity_id,project_id,relation,created_at) \
             VALUES(?,?,?,?)",
        )
        .bind(&entity.id)
        .bind(project_id)
        .bind(relation.trim())
        .bind(recorded_at)
        .execute(&mut **tx)
        .await?;
    }
    let event = LabEvent {
        id: uuid::Uuid::new_v4().to_string(),
        registry_id: transaction.registry_id.clone(),
        project_id: transaction.project_id.clone(),
        transaction_id: transaction.id.clone(),
        sequence,
        entity_id: Some(entity.id.clone()),
        prior_event_id: None,
        kind: "entity_created".into(),
        schema_version: 1,
        payload_json: json!({
            "entity_id": entity.id,
            "display_id": entity.display_id,
            "kind": entity.kind,
            "title": entity.title,
        })
        .to_string(),
        occurred_at,
        recorded_at,
        expected_revision: None,
        resulting_revision: Some(entity.revision),
        reason: None,
    };
    insert_lab_event_tx(tx, &event).await?;
    if let Some(subject) = create.subject.as_ref() {
        sqlx::query("INSERT INTO lab_subjects(entity_id,species,strain,sex,date_of_birth,origin_kind,established_event_id) VALUES(?,?,?,?,?,?,?)")
            .bind(&entity.id).bind(subject.species.trim()).bind(subject.strain.as_deref())
            .bind(subject.sex.as_deref()).bind(subject.date_of_birth.as_deref()).bind(&subject.origin_kind)
            .bind(&event.id).execute(&mut **tx).await?;
    }
    Ok((entity, event))
}

async fn wet_run_status_in_scope_tx(
    tx: &mut Transaction<'_, Sqlite>,
    run_id: &str,
    registry_id: &str,
    project_id: Option<&str>,
) -> Result<String> {
    let project_id =
        project_id.ok_or_else(|| anyhow::anyhow!("Wet-lab Run mutations require a Project"))?;
    let status: Option<String> = sqlx::query_scalar(
        "SELECT r.status FROM lab_wet_runs w JOIN runs r ON r.id=w.run_id \
         WHERE w.run_id=? AND w.registry_id=? AND r.project_id=?",
    )
    .bind(run_id)
    .bind(registry_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await?;
    status.ok_or_else(|| {
        anyhow::anyhow!("Wet-lab Run is missing or belongs to another Project or registry")
    })
}

async fn ensure_wet_run_open_tx(
    tx: &mut Transaction<'_, Sqlite>,
    run_id: &str,
    registry_id: &str,
    project_id: Option<&str>,
) -> Result<()> {
    let status = wet_run_status_in_scope_tx(tx, run_id, registry_id, project_id).await?;
    match status.as_str() {
        "draft" | "running" => Ok(()),
        _ => anyhow::bail!("Completed or cancelled wet-lab Runs cannot be edited"),
    }
}

async fn ensure_derivation_edge_acyclic_tx(
    tx: &mut Transaction<'_, Sqlite>,
    parent_id: &str,
    child_id: &str,
) -> Result<()> {
    if parent_id == child_id {
        anyhow::bail!("Material derivation cannot point to itself");
    }
    let cycle: Option<(i64,)> = sqlx::query_as(
        "WITH RECURSIVE descendants(id) AS (\
         SELECT child_material_unit_id FROM lab_material_derivations WHERE parent_material_unit_id=? \
         UNION SELECT d.child_material_unit_id FROM lab_material_derivations d JOIN descendants x ON d.parent_material_unit_id=x.id) \
         SELECT 1 FROM descendants WHERE id=? LIMIT 1",
    )
    .bind(child_id)
    .bind(parent_id)
    .fetch_optional(&mut **tx)
    .await?;
    if cycle.is_some() {
        anyhow::bail!("Material derivation would create a cycle");
    }
    Ok(())
}

struct MaterialConsumptionChange {
    before: LabQuantity,
    after: LabQuantity,
    lifecycle_before: String,
    lifecycle_after: String,
    availability: String,
    expected_revision: i64,
    resulting_revision: i64,
    consumed_value_in_stored_unit: Option<Decimal>,
    stored_unit: Option<String>,
}

fn unit_scale_to_base(unit: &str) -> Option<Decimal> {
    match unit {
        "uL" | "ng" => Some(Decimal::ONE),
        "mL" | "ug" => Some(Decimal::from(1_000u64)),
        "L" | "mg" => Some(Decimal::from(1_000_000u64)),
        "g" => Some(Decimal::from(1_000_000_000u64)),
        "cells" | "reactions" | "each" | "box" => Some(Decimal::ONE),
        _ => None,
    }
}

fn convert_decimal_unit(value: Decimal, from: &str, to: &str) -> Result<Decimal> {
    if from == to {
        return Ok(value);
    }
    let from_dimension = super::lab_unit_dimension(from);
    let to_dimension = super::lab_unit_dimension(to);
    if from_dimension.is_none() || from_dimension != to_dimension || from_dimension == Some("count")
    {
        anyhow::bail!("Quantity unit is incompatible with the MaterialUnit balance");
    }
    let from_scale = unit_scale_to_base(from)
        .ok_or_else(|| anyhow::anyhow!("Unsupported source quantity unit"))?;
    let to_scale = unit_scale_to_base(to)
        .ok_or_else(|| anyhow::anyhow!("Unsupported destination quantity unit"))?;
    Ok(value * from_scale / to_scale)
}

fn canonical_decimal(value: Decimal) -> String {
    value.normalize().to_string()
}

fn validate_derivation_quantity_conservation(
    derivation: &super::LabMaterialDerivationCreate,
) -> Result<()> {
    if !matches!(
        derivation.operation.as_str(),
        "split" | "aliquot" | "merge" | "pool"
    ) {
        return Ok(());
    }
    let base_unit = derivation.inputs[0]
        .quantity
        .as_ref()
        .and_then(|quantity| quantity.unit.as_deref())
        .ok_or_else(|| anyhow::anyhow!("Derivation input unit is missing"))?;
    let sum = |quantities: Vec<&LabQuantity>| -> Result<Decimal> {
        quantities
            .into_iter()
            .try_fold(Decimal::ZERO, |total, quantity| {
                let value = decimal(quantity.value.as_deref().unwrap_or_default())?;
                let unit = quantity.unit.as_deref().unwrap_or_default();
                Ok(total + convert_decimal_unit(value, unit, base_unit)?)
            })
    };
    let input_total = sum(derivation
        .inputs
        .iter()
        .filter_map(|input| input.quantity.as_ref())
        .collect())?;
    let output_total = sum(derivation
        .outputs
        .iter()
        .filter_map(|output| output.quantity.as_ref())
        .collect())?;
    if input_total != output_total {
        anyhow::bail!("Split/merge derivation output quantity must equal consumed input quantity");
    }
    Ok(())
}

async fn settle_run_reservations_tx(
    tx: &mut Transaction<'_, Sqlite>,
    run_id: &str,
    material_unit_id: &str,
    consumed: Decimal,
    stored_unit: &str,
    recorded_at: i64,
) -> Result<()> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id,quantity_value,quantity_unit FROM lab_reservations \
         WHERE run_id=? AND material_unit_id=? AND status='active' ORDER BY created_at,id",
    )
    .bind(run_id)
    .bind(material_unit_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut remaining_consumption = consumed;
    for (id, quantity_value, quantity_unit) in rows {
        if remaining_consumption <= Decimal::ZERO {
            break;
        }
        let reserved =
            convert_decimal_unit(decimal(&quantity_value)?, &quantity_unit, stored_unit)?;
        if remaining_consumption >= reserved {
            sqlx::query(
                "UPDATE lab_reservations SET status='released',released_at=? WHERE id=? AND status='active'",
            )
            .bind(recorded_at)
            .bind(&id)
            .execute(&mut **tx)
            .await?;
            remaining_consumption -= reserved;
        } else {
            let reservation_remaining = reserved - remaining_consumption;
            let stored_value = canonical_decimal(convert_decimal_unit(
                reservation_remaining,
                stored_unit,
                &quantity_unit,
            )?);
            sqlx::query(
                "UPDATE lab_reservations SET quantity_value=? WHERE id=? AND status='active'",
            )
            .bind(stored_value)
            .bind(&id)
            .execute(&mut **tx)
            .await?;
            remaining_consumption = Decimal::ZERO;
        }
    }
    Ok(())
}

async fn apply_participant_consumption_tx(
    tx: &mut Transaction<'_, Sqlite>,
    transaction: &LabTransaction,
    participant: &LabRunParticipantCreate,
    recorded_at: i64,
) -> Result<Option<MaterialConsumptionChange>> {
    if participant.direction != "input"
        || !matches!(
            participant.effect.as_str(),
            "partially_consumed" | "fully_consumed" | "transformed" | "sampled_from"
        )
    {
        return Ok(None);
    }
    let expected_revision = participant.expected_material_revision.ok_or_else(|| {
        anyhow::anyhow!("Consuming a MaterialUnit requires its expected revision")
    })?;
    let row: Option<(i64, String, Option<String>, Option<String>, String, String)> =
        sqlx::query_as(
        "SELECT e.revision,m.quantity_state,m.quantity_value,m.quantity_unit,m.lifecycle,m.availability \
         FROM lab_entities e JOIN lab_material_units m ON m.entity_id=e.id \
         WHERE e.id=? AND e.registry_id=?",
    )
    .bind(&participant.material_unit_id)
    .bind(&transaction.registry_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some((revision, quantity_state, quantity_value, quantity_unit, lifecycle, availability)) =
        row
    else {
        anyhow::bail!("MaterialUnit is missing or belongs to another registry");
    };
    if revision != expected_revision {
        anyhow::bail!("Material unit revision conflict");
    }
    if lifecycle != "active" || availability != "available" {
        anyhow::bail!("Only active, available MaterialUnits can be consumed");
    }

    let before = match quantity_state.as_str() {
        "measured" => LabQuantity::measured(
            quantity_value
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Material quantity is missing"))?,
            quantity_unit
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Material quantity unit is missing"))?,
        ),
        "unknown" => LabQuantity {
            state: LabQuantityState::Unknown,
            value: None,
            unit: None,
        },
        "not_measured" => LabQuantity {
            state: LabQuantityState::NotMeasured,
            value: None,
            unit: None,
        },
        _ => anyhow::bail!("Unknown MaterialUnit quantity state"),
    };

    let is_partial = matches!(
        participant.effect.as_str(),
        "partially_consumed" | "sampled_from"
    );
    let (after, lifecycle_after, consumed_value_in_stored_unit, stored_unit) = if quantity_state
        == "measured"
    {
        let on_hand = decimal(quantity_value.as_deref().unwrap_or_default())?;
        let stored_unit = quantity_unit.as_deref().unwrap_or_default();
        let consumed = match participant.quantity.as_ref() {
            Some(quantity) if quantity.state == LabQuantityState::Measured => {
                let value = decimal(quantity.value.as_deref().unwrap_or_default())?;
                convert_decimal_unit(
                    value,
                    quantity.unit.as_deref().unwrap_or_default(),
                    stored_unit,
                )?
            }
            None if !is_partial => on_hand,
            _ => anyhow::bail!("Consumption quantity must be measured"),
        };
        if consumed <= Decimal::ZERO || consumed > on_hand {
            anyhow::bail!("Consumption would exceed the MaterialUnit balance");
        }
        let remaining = on_hand - consumed;
        if is_partial && remaining == Decimal::ZERO {
            anyhow::bail!("Use fully_consumed when no MaterialUnit balance remains");
        }
        if !is_partial && remaining != Decimal::ZERO {
            anyhow::bail!("Full consumption quantity must equal the MaterialUnit balance");
        }

        sqlx::query(
                "UPDATE lab_reservations SET status='expired',released_at=? \
                 WHERE material_unit_id=? AND status='active' AND expires_at IS NOT NULL AND expires_at<=?",
            )
            .bind(recorded_at)
            .bind(&participant.material_unit_id)
            .bind(recorded_at)
            .execute(&mut **tx)
            .await?;
        let other_reservations: Vec<(String, String)> = sqlx::query_as(
            "SELECT quantity_value,quantity_unit FROM lab_reservations \
                 WHERE material_unit_id=? AND run_id<>? AND status='active'",
        )
        .bind(&participant.material_unit_id)
        .bind(&participant.run_id)
        .fetch_all(&mut **tx)
        .await?;
        let reserved_for_others =
            other_reservations
                .into_iter()
                .try_fold(Decimal::ZERO, |sum, (value, unit)| {
                    Ok::<_, anyhow::Error>(
                        sum + convert_decimal_unit(decimal(&value)?, &unit, stored_unit)?,
                    )
                })?;
        if remaining < reserved_for_others {
            anyhow::bail!("Consumption would use material reserved by another Run");
        }
        let lifecycle_after = if remaining == Decimal::ZERO {
            "depleted".to_string()
        } else {
            lifecycle.clone()
        };
        (
            LabQuantity::measured(canonical_decimal(remaining), stored_unit),
            lifecycle_after,
            Some(consumed),
            Some(stored_unit.to_string()),
        )
    } else if is_partial || participant.quantity.is_some() {
        anyhow::bail!("Partial or numeric consumption requires a measured MaterialUnit balance");
    } else {
        (
            LabQuantity {
                state: LabQuantityState::NotMeasured,
                value: None,
                unit: None,
            },
            "depleted".into(),
            None,
            None,
        )
    };

    let affected = sqlx::query(
        "UPDATE lab_entities SET revision=revision+1,updated_at=?,last_transaction_id=? \
         WHERE id=? AND registry_id=? AND revision=?",
    )
    .bind(recorded_at)
    .bind(&transaction.id)
    .bind(&participant.material_unit_id)
    .bind(&transaction.registry_id)
    .bind(expected_revision)
    .execute(&mut **tx)
    .await?;
    if affected.rows_affected() != 1 {
        anyhow::bail!("Material unit revision conflict");
    }
    sqlx::query(
        "UPDATE lab_material_units SET quantity_state=?,quantity_value=?,quantity_unit=?,lifecycle=? \
         WHERE entity_id=?",
    )
    .bind(quantity_storage_state(&after))
    .bind(after.value.as_deref())
    .bind(after.unit.as_deref())
    .bind(&lifecycle_after)
    .bind(&participant.material_unit_id)
    .execute(&mut **tx)
    .await?;
    if let (Some(consumed), Some(unit)) = (consumed_value_in_stored_unit, stored_unit.as_deref()) {
        settle_run_reservations_tx(
            tx,
            &participant.run_id,
            &participant.material_unit_id,
            consumed,
            unit,
            recorded_at,
        )
        .await?;
    } else {
        sqlx::query(
            "UPDATE lab_reservations SET status='released',released_at=? \
             WHERE run_id=? AND material_unit_id=? AND status='active'",
        )
        .bind(recorded_at)
        .bind(&participant.run_id)
        .bind(&participant.material_unit_id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(Some(MaterialConsumptionChange {
        before,
        after,
        lifecycle_before: lifecycle,
        lifecycle_after,
        availability,
        expected_revision,
        resulting_revision: expected_revision + 1,
        consumed_value_in_stored_unit,
        stored_unit,
    }))
}

async fn record_run_participant_tx(
    tx: &mut Transaction<'_, Sqlite>,
    transaction: &LabTransaction,
    participant: &LabRunParticipantCreate,
    occurred_at: i64,
    recorded_at: i64,
    sequence: i64,
) -> Result<LabEvent> {
    ensure_wet_run_open_tx(
        tx,
        &participant.run_id,
        &transaction.registry_id,
        transaction.project_id.as_deref(),
    )
    .await?;
    ensure_entity_kind_registry(
        tx,
        &participant.material_unit_id,
        &transaction.registry_id,
        LabEntityKind::MaterialUnit,
    )
    .await?;
    let consumption =
        apply_participant_consumption_tx(tx, transaction, participant, recorded_at).await?;
    let event = LabEvent {
        id: uuid::Uuid::new_v4().to_string(),
        registry_id: transaction.registry_id.clone(),
        project_id: transaction.project_id.clone(),
        transaction_id: transaction.id.clone(),
        sequence,
        entity_id: Some(participant.material_unit_id.clone()),
        prior_event_id: None,
        kind: "run_participant_recorded".into(),
        schema_version: 1,
        payload_json: json!({
            "run_id": participant.run_id,
            "direction": participant.direction,
            "role": participant.role,
            "effect": participant.effect,
            "quantity": participant.quantity,
            "transformation_group": participant.transformation_group,
            "inventory_change": consumption.as_ref().map(|change| json!({
                "before": change.before,
                "after": change.after,
                "lifecycle_before": change.lifecycle_before,
                "lifecycle_after": change.lifecycle_after,
                "availability": change.availability,
                "consumed_value_in_stored_unit": change.consumed_value_in_stored_unit.map(canonical_decimal),
                "stored_unit": change.stored_unit,
            })),
        })
        .to_string(),
        occurred_at,
        recorded_at,
        expected_revision: consumption.as_ref().map(|change| change.expected_revision),
        resulting_revision: consumption.as_ref().map(|change| change.resulting_revision),
        reason: None,
    };
    insert_lab_event_tx(tx, &event).await?;
    let quantity = participant.quantity.as_ref();
    sqlx::query(
        "INSERT INTO lab_run_participants(id,run_id,material_unit_id,direction,role,effect,quantity_state,quantity_value,quantity_unit,transformation_group,established_event_id,created_at) \
         VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(&participant.run_id)
    .bind(&participant.material_unit_id)
    .bind(&participant.direction)
    .bind(&participant.role)
    .bind(&participant.effect)
    .bind(quantity.map(quantity_storage_state))
    .bind(quantity.and_then(|quantity| quantity.value.as_deref()))
    .bind(quantity.and_then(|quantity| quantity.unit.as_deref()))
    .bind(participant.transformation_group.as_deref())
    .bind(&event.id)
    .bind(recorded_at)
    .execute(&mut **tx)
    .await?;
    Ok(event)
}

async fn ensure_registry_exists(tx: &mut Transaction<'_, Sqlite>, registry_id: &str) -> Result<()> {
    let exists: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM lab_registries WHERE id=?")
        .bind(registry_id)
        .fetch_optional(&mut **tx)
        .await?;
    if exists.is_none() {
        anyhow::bail!("Lab registry not found");
    }
    Ok(())
}

async fn ensure_project_registry_link(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
    registry_id: &str,
) -> Result<()> {
    let exists: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM project_lab_registries WHERE project_id=? AND registry_id=?")
            .bind(project_id)
            .bind(registry_id)
            .fetch_optional(&mut **tx)
            .await?;
    if exists.is_none() {
        anyhow::bail!("Project is not linked to the transaction registry");
    }
    Ok(())
}

/// A lab experiment can only be attached to the root frame of the same Project.
/// Child frames are implementation detail of the agent and must not become a
/// second, invisible experimental notebook.
async fn ensure_root_conversation(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
    frame_id: &str,
) -> Result<()> {
    let exists: Option<(String,)> =
        sqlx::query_as("SELECT id FROM frames WHERE id=? AND project_id=? AND parent_frame_id=id")
            .bind(frame_id)
            .bind(project_id)
            .fetch_optional(&mut **tx)
            .await?;
    if exists.is_none() {
        anyhow::bail!("Conversation does not belong to this Project");
    }
    Ok(())
}

async fn ensure_entity_registry(
    tx: &mut Transaction<'_, Sqlite>,
    entity_id: &str,
    registry_id: &str,
) -> Result<()> {
    let exists: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM lab_entities WHERE id=? AND registry_id=?")
            .bind(entity_id)
            .bind(registry_id)
            .fetch_optional(&mut **tx)
            .await?;
    if exists.is_none() {
        anyhow::bail!("Lab entity does not belong to the transaction registry");
    }
    Ok(())
}

async fn ensure_entity_kind_registry(
    tx: &mut Transaction<'_, Sqlite>,
    entity_id: &str,
    registry_id: &str,
    expected_kind: LabEntityKind,
) -> Result<()> {
    let kind: Option<(String,)> =
        sqlx::query_as("SELECT kind FROM lab_entities WHERE id=? AND registry_id=?")
            .bind(entity_id)
            .bind(registry_id)
            .fetch_optional(&mut **tx)
            .await?;
    match kind {
        Some((kind,)) if kind == expected_kind.as_str() => Ok(()),
        Some(_) => anyhow::bail!("Lab entity has an incompatible kind"),
        None => anyhow::bail!("Lab entity is missing or belongs to another registry"),
    }
}

async fn ensure_prior_event_registry(
    tx: &mut Transaction<'_, Sqlite>,
    prior_event_id: Option<&str>,
    registry_id: &str,
) -> Result<()> {
    let Some(prior_event_id) = prior_event_id else {
        return Ok(());
    };
    let exists: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM lab_events WHERE id=? AND registry_id=?")
            .bind(prior_event_id)
            .bind(registry_id)
            .fetch_optional(&mut **tx)
            .await?;
    if exists.is_none() {
        anyhow::bail!("Prior lab event does not belong to the transaction registry");
    }
    Ok(())
}

async fn get_transaction_by_command_tx(
    tx: &mut Transaction<'_, Sqlite>,
    registry_id: &str,
    command_id: &str,
) -> Result<Option<LabTransaction>> {
    let row = sqlx::query(
        "SELECT id,display_id,registry_id,project_id,command_id,schema_version,actor_kind,actor_ref,\
                confirmation_json,request_json,receipt_json,status,created_at,committed_at \
         FROM lab_transactions WHERE registry_id=? AND command_id=?",
    )
    .bind(registry_id)
    .bind(command_id)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(lab_transaction_from_row).transpose()
}

async fn get_transaction_by_command_pool(
    pool: &sqlx::SqlitePool,
    registry_id: &str,
    command_id: &str,
) -> Result<Option<LabTransaction>> {
    let row = sqlx::query(
        "SELECT id,display_id,registry_id,project_id,command_id,schema_version,actor_kind,actor_ref,\
                confirmation_json,request_json,receipt_json,status,created_at,committed_at \
         FROM lab_transactions WHERE registry_id=? AND command_id=?",
    )
    .bind(registry_id)
    .bind(command_id)
    .fetch_optional(pool)
    .await?;
    row.map(lab_transaction_from_row).transpose()
}

async fn list_lab_events_tx(
    tx: &mut Transaction<'_, Sqlite>,
    transaction_id: &str,
) -> Result<Vec<LabEvent>> {
    let rows = sqlx::query(
        "SELECT id,registry_id,project_id,transaction_id,sequence,entity_id,prior_event_id,kind,\
                schema_version,payload_json,occurred_at,recorded_at,expected_revision,resulting_revision,reason \
         FROM lab_events WHERE transaction_id=? ORDER BY sequence ASC",
    )
    .bind(transaction_id)
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter().map(lab_event_from_row).collect()
}

fn validate_display_prefix(prefix: &str) -> Result<()> {
    let prefix = prefix.trim();
    if !(2..=8).contains(&prefix.len())
        || !prefix
            .chars()
            .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
    {
        anyhow::bail!("Lab display ID prefix must be 2-8 uppercase ASCII letters or digits");
    }
    Ok(())
}

async fn allocate_display_id(
    tx: &mut Transaction<'_, Sqlite>,
    registry_id: &str,
    prefix: &str,
) -> Result<String> {
    let row = sqlx::query(
        "INSERT INTO lab_id_counters(registry_id,prefix,next_value) VALUES(?,?,2) \
         ON CONFLICT(registry_id,prefix) DO UPDATE SET next_value=lab_id_counters.next_value+1 \
         RETURNING next_value-1 AS allocated_value",
    )
    .bind(registry_id)
    .bind(prefix)
    .fetch_one(&mut **tx)
    .await?;
    let value: i64 = row.try_get("allocated_value")?;
    Ok(format!("{prefix}-{value:06}"))
}

#[cfg(test)]
mod tests {
    use super::validate_display_prefix;

    #[test]
    fn display_prefix_is_compact_and_stable() {
        assert!(validate_display_prefix("SMP").is_ok());
        assert!(validate_display_prefix("AB12").is_ok());
        assert!(validate_display_prefix("smp").is_err());
        assert!(validate_display_prefix("A").is_err());
        assert!(validate_display_prefix("SAMPLE123").is_err());
        assert!(validate_display_prefix("SMP-1").is_err());
    }
}

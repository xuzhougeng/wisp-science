use super::{canonical_json_sha256, ProjectSyncState, Store};
use anyhow::{Context, Result};
use futures_util::TryStreamExt;
use sha2::{Digest, Sha256};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    Column, Connection, Row, Sqlite, Transaction, TypeInfo, ValueRef,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectTransferStats {
    pub frames: i64,
    pub messages: i64,
    pub artifacts: i64,
    pub runs: i64,
    pub path_warnings: i64,
}

/// Copy every project-owned row from the attached `transfer` database.
/// Machine-global settings, execution contexts, credentials, and ACP runtime
/// bindings are deliberately not project data and are not copied.
async fn copy_project_children(tx: &mut Transaction<'_, Sqlite>, project_id: &str) -> Result<()> {
    if attached_table_exists(tx, "agent_workflows").await? {
        let workflow_columns = attached_table_columns(tx, "agent_workflows").await?;
        let has_plan_columns = [
            "frame_id",
            "goal",
            "mode",
            "status",
            "max_parallel",
            "requires_confirmation",
            "plan_json",
            "approved_at",
        ]
        .iter()
        .all(|column| workflow_columns.contains(*column));
        let has_lineage = [
            "root_workflow_id",
            "parent_attempt_id",
            "depth",
            "root_limits_json",
        ]
        .iter()
        .all(|column| workflow_columns.contains(*column));
        let lineage = if has_lineage {
            "root_workflow_id,parent_attempt_id,depth,root_limits_json"
        } else {
            "id,NULL,0,'{\"max_depth\":1,\"max_tasks\":8,\"max_parallel\":2,\"max_tokens\":256000,\"max_tool_calls\":512,\"max_cost_microunits\":8000000,\"wall_time_secs\":1800}'"
        };
        // Keep the target column order explicit while allowing old exports to
        // synthesize root lineage. Reorder the SELECT fragments to match it.
        let workflow_query = if has_plan_columns {
            format!(
                "INSERT INTO agent_workflows(id,project_id,workspace_id,frame_id,root_workflow_id,parent_attempt_id,depth,root_limits_json,name,description,goal,mode,status,max_parallel,requires_confirmation,plan_json,version,enabled,approved_at,created_at,updated_at) \
                 SELECT id,project_id,workspace_id,frame_id,{lineage},name,description,goal,mode,'draft',max_parallel,requires_confirmation,plan_json,version,enabled,approved_at,created_at,updated_at \
                 FROM transfer.agent_workflows WHERE project_id=?"
            )
        } else {
            format!(
                "INSERT INTO agent_workflows(id,project_id,workspace_id,frame_id,root_workflow_id,parent_attempt_id,depth,root_limits_json,name,description,goal,mode,status,max_parallel,requires_confirmation,plan_json,version,enabled,approved_at,created_at,updated_at) \
                 SELECT id,project_id,workspace_id,NULL,{lineage},name,description,name,'assisted','draft',2,1,'{{}}',version,enabled,NULL,created_at,updated_at \
                 FROM transfer.agent_workflows WHERE project_id=?"
            )
        };
        sqlx::query(&workflow_query)
            .bind(project_id)
            .execute(&mut **tx)
            .await?;

        if attached_table_exists(tx, "agent_workflow_steps").await? {
            let columns = attached_table_columns(tx, "agent_workflow_steps").await?;
            let has_contracts = ["input_contract_json", "output_contract_json", "budget_json"]
                .iter()
                .all(|column| columns.contains(*column));
            let has_plan = ["template_id", "spec_json"]
                .iter()
                .all(|column| columns.contains(*column));
            let template_expr = if has_plan {
                "s.template_id"
            } else {
                "s.agent_id"
            };
            let spec_expr = if has_plan { "s.spec_json" } else { "'{}'" };
            let task_kind_expr = if columns.contains("task_kind") {
                "s.task_kind"
            } else {
                "'agent'"
            };
            let activity_expr = if columns.contains("activity_json") {
                "s.activity_json"
            } else {
                "'{}'"
            };
            let input_contract_expr = if has_contracts {
                "s.input_contract_json"
            } else {
                "'{}'"
            };
            let output_contract_expr = if has_contracts {
                "s.output_contract_json"
            } else {
                "'{}'"
            };
            let budget_expr = if has_contracts {
                "s.budget_json"
            } else {
                "'{}'"
            };
            let query = format!(
                "INSERT INTO agent_workflow_steps(id,workflow_id,position,agent_id,template_id,role,backend,model,prompt_template,input_schema_json,output_schema_json,input_contract_json,output_contract_json,permissions_json,context_policy_json,budget_json,spec_json,task_kind,activity_json,timeout_secs,created_at,updated_at) \
                 SELECT s.id,s.workflow_id,s.position,s.agent_id,{template_expr},s.role,s.backend,s.model,s.prompt_template,s.input_schema_json,s.output_schema_json,{input_contract_expr},{output_contract_expr},s.permissions_json,s.context_policy_json,{budget_expr},{spec_expr},{task_kind_expr},{activity_expr},s.timeout_secs,s.created_at,s.updated_at \
                 FROM transfer.agent_workflow_steps s JOIN transfer.agent_workflows w ON w.id=s.workflow_id WHERE w.project_id=?"
            );
            sqlx::query(&query)
                .bind(project_id)
                .execute(&mut **tx)
                .await?;
        }
        if has_plan_columns {
            sqlx::query(
                "UPDATE agent_workflows SET status=(SELECT source.status FROM transfer.agent_workflows source WHERE source.id=agent_workflows.id) \
                 WHERE project_id=?",
            )
            .bind(project_id)
            .execute(&mut **tx)
            .await?;
        }
    }

    const QUERIES: &[&str] = &[
        "INSERT INTO folders(id,project_id,name,created_at,updated_at) \
         SELECT id,project_id,name,created_at,updated_at FROM transfer.folders WHERE project_id=?",
        "INSERT INTO frames(id,parent_frame_id,root_frame_id,agent_name,status,project_id,folder_id,model,reasoning_effort,service_tier,input_tokens,output_tokens,created_at,updated_at,completed_at,title) \
         SELECT id,parent_frame_id,root_frame_id,agent_name,status,project_id,folder_id,model,reasoning_effort,service_tier,input_tokens,output_tokens,created_at,updated_at,completed_at,title \
         FROM transfer.frames WHERE project_id=?",
        "INSERT INTO messages(id,frame_id,seq,role,content,tool_calls,tool_call_id,tool_name,reasoning,ts,model_name) \
         SELECT id,frame_id,seq,role,content,tool_calls,tool_call_id,tool_name,reasoning,ts,model_name FROM transfer.messages \
         WHERE frame_id IN (SELECT id FROM transfer.frames WHERE project_id=?)",
        "INSERT INTO session_reviews(id,frame_id,message_seq,report_json,created_at,updated_at) \
         SELECT id,frame_id,message_seq,report_json,created_at,updated_at FROM transfer.session_reviews \
         WHERE frame_id IN (SELECT id FROM transfer.frames WHERE project_id=?)",
        "INSERT INTO session_ui_events(frame_id,seq,event_json) \
         SELECT frame_id,seq,event_json FROM transfer.session_ui_events \
         WHERE frame_id IN (SELECT id FROM transfer.frames WHERE project_id=?)",
        "INSERT INTO proposed_plans(id,frame_id,codex_thread_id,codex_turn_id,revision,markdown,status,mode,progress_json,runtime_config_json,created_at,updated_at) \
         SELECT id,frame_id,codex_thread_id,codex_turn_id,revision,markdown,status,mode,progress_json,runtime_config_json,created_at,updated_at \
         FROM transfer.proposed_plans WHERE frame_id IN (SELECT id FROM transfer.frames WHERE project_id=?)",
        "INSERT INTO codex_turn_configs(id,frame_id,codex_thread_id,codex_turn_id,mode,config_version,config_version_text,requested_json,effective_json,actual_json,created_at,updated_at) \
         SELECT id,frame_id,codex_thread_id,codex_turn_id,mode,config_version,config_version_text,requested_json,effective_json,actual_json,created_at,updated_at \
         FROM transfer.codex_turn_configs WHERE frame_id IN (SELECT id FROM transfer.frames WHERE project_id=?)",
        "INSERT INTO execution_log(id,frame_id,cell_index,tool,language,source,stdout,stderr,exit_status,wall_s,files_written,files_read,env_hash,created_at) \
         SELECT id,frame_id,cell_index,tool,language,source,stdout,stderr,exit_status,wall_s,files_written,files_read,env_hash,created_at \
         FROM transfer.execution_log WHERE frame_id IN (SELECT id FROM transfer.frames WHERE project_id=?)",
        "INSERT OR IGNORE INTO env_snapshots(hash,env_name,packages_json,created_at) \
         SELECT hash,env_name,packages_json,created_at FROM transfer.env_snapshots WHERE hash IN (\
           SELECT env_hash FROM transfer.execution_log WHERE frame_id IN (SELECT id FROM transfer.frames WHERE project_id=?))",
        "INSERT INTO runs(id,project_id,frame_id,context_id,title,kind,status,command,script_path,input_refs_json,output_specs_json,created_at,started_at,ended_at,exit_code,stdout_tail,stderr_tail,remote_workdir,remote_handle_json,timeout_secs,last_polled_at,last_poll_error,lifecycle_owner,lifecycle_lease_until,env_snapshot_json) \
         SELECT id,project_id,frame_id,context_id,title,kind,status,command,script_path,input_refs_json,output_specs_json,created_at,started_at,ended_at,exit_code,stdout_tail,stderr_tail,remote_workdir,remote_handle_json,timeout_secs,last_polled_at,last_poll_error,lifecycle_owner,lifecycle_lease_until,env_snapshot_json \
         FROM transfer.runs WHERE project_id=?",
        "INSERT INTO artifacts(id,project_id,root_frame_id,filename,content_type,storage_path,created_at,latest_version_id) \
         SELECT id,project_id,root_frame_id,filename,content_type,storage_path,created_at,latest_version_id \
         FROM transfer.artifacts WHERE project_id=?",
        "INSERT OR IGNORE INTO env_snapshots(hash,env_name,packages_json,created_at) \
         SELECT hash,env_name,packages_json,created_at FROM transfer.env_snapshots WHERE hash IN (\
           SELECT av.env_snapshot_hash FROM transfer.artifact_versions av JOIN transfer.artifacts a ON a.id=av.artifact_id WHERE a.project_id=?)",
        "INSERT INTO artifact_versions(id,artifact_id,version_number,content_type,storage_path,size_bytes,checksum,parent_version_id,producing_run_id,env_snapshot_hash,created_at) \
         SELECT av.id,av.artifact_id,av.version_number,av.content_type,av.storage_path,av.size_bytes,av.checksum,av.parent_version_id,av.producing_run_id,av.env_snapshot_hash,av.created_at \
         FROM transfer.artifact_versions av JOIN transfer.artifacts a ON a.id=av.artifact_id WHERE a.project_id=?",
        "INSERT INTO artifact_dependencies(id,artifact_version_id,depends_on_version_id,reference_name,created_at) \
         SELECT d.id,d.artifact_version_id,d.depends_on_version_id,d.reference_name,d.created_at FROM transfer.artifact_dependencies d \
         WHERE d.artifact_version_id IN (SELECT av.id FROM transfer.artifact_versions av JOIN transfer.artifacts a ON a.id=av.artifact_id WHERE a.project_id=?)",
        "INSERT INTO run_artifacts(id,run_id,artifact_id,role,created_at) \
         SELECT id,run_id,artifact_id,role,created_at FROM transfer.run_artifacts \
         WHERE run_id IN (SELECT id FROM transfer.runs WHERE project_id=?)",
        "INSERT INTO research_nodes(id,project_id,kind,title,ref_id,metadata_json,created_at,updated_at) \
         SELECT id,project_id,kind,title,ref_id,metadata_json,created_at,updated_at FROM transfer.research_nodes WHERE project_id=?",
        "INSERT INTO research_edges(id,project_id,source_id,target_id,relation,metadata_json,created_at) \
         SELECT id,project_id,source_id,target_id,relation,metadata_json,created_at FROM transfer.research_edges WHERE project_id=?",
    ];

    for query in QUERIES {
        sqlx::query(query)
            .bind(project_id)
            .execute(&mut **tx)
            .await?;
    }
    if attached_table_exists(tx, "message_resource_links").await? {
        let columns = attached_table_columns(tx, "message_resource_links").await?;
        let created_artifact = if columns.contains("created_artifact") {
            "l.created_artifact"
        } else {
            "0"
        };
        let created_version = if columns.contains("created_version") {
            "l.created_version"
        } else {
            "0"
        };
        let query = format!(
            "INSERT INTO message_resource_links(\
             id,frame_id,message_seq,ordinal,original_reference,artifact_id,\
             artifact_version_id,display_name,resource_kind,mime_type,status,error,\
             created_artifact,created_version,created_at) \
             SELECT l.id,l.frame_id,l.message_seq,l.ordinal,l.original_reference,l.artifact_id,\
             l.artifact_version_id,l.display_name,l.resource_kind,l.mime_type,l.status,l.error,\
             {created_artifact},{created_version},l.created_at \
             FROM transfer.message_resource_links l \
             WHERE l.frame_id IN (SELECT id FROM transfer.frames WHERE project_id=?)"
        );
        sqlx::query(&query)
            .bind(project_id)
            .execute(&mut **tx)
            .await?;
    }
    if attached_table_columns(tx, "artifacts")
        .await?
        .contains("logical_key")
    {
        sqlx::query(
            "UPDATE artifacts SET logical_key=(\
               SELECT source.logical_key FROM transfer.artifacts source \
               WHERE source.id=artifacts.id\
             ) WHERE project_id=?",
        )
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
    }
    let artifact_version_columns = attached_table_columns(tx, "artifact_versions").await?;
    if artifact_version_columns.contains("materialization")
        && artifact_version_columns.contains("capture_timing")
    {
        sqlx::query(
            "UPDATE artifact_versions SET \
               materialization=(SELECT source.materialization \
                 FROM transfer.artifact_versions source WHERE source.id=artifact_versions.id),\
               capture_timing=(SELECT source.capture_timing \
                 FROM transfer.artifact_versions source WHERE source.id=artifact_versions.id) \
             WHERE artifact_id IN (SELECT id FROM artifacts WHERE project_id=?)",
        )
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
    }
    if artifact_version_columns.contains("source_discarded_at") {
        sqlx::query(
            "UPDATE artifact_versions SET \
               source_discarded_at=(SELECT source.source_discarded_at \
                 FROM transfer.artifact_versions source WHERE source.id=artifact_versions.id) \
             WHERE artifact_id IN (SELECT id FROM artifacts WHERE project_id=?)",
        )
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
    }
    let dependency_columns = attached_table_columns(tx, "artifact_dependencies").await?;
    if dependency_columns.contains("basis") && dependency_columns.contains("confidence") {
        sqlx::query(
            "UPDATE artifact_dependencies SET \
               basis=(SELECT source.basis FROM transfer.artifact_dependencies source \
                 WHERE source.id=artifact_dependencies.id),\
               confidence=(SELECT source.confidence FROM transfer.artifact_dependencies source \
                 WHERE source.id=artifact_dependencies.id) \
             WHERE artifact_version_id IN (\
               SELECT version.id FROM artifact_versions version \
               JOIN artifacts artifact ON artifact.id=version.artifact_id \
               WHERE artifact.project_id=?\
             )",
        )
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
    }
    let env_columns = attached_table_columns(tx, "env_snapshots").await?;
    if env_columns.contains("snapshot_json") && env_columns.contains("hash_algorithm") {
        sqlx::query(
            "UPDATE env_snapshots SET \
               snapshot_json=(SELECT source.snapshot_json FROM transfer.env_snapshots source \
                 WHERE source.hash=env_snapshots.hash),\
               hash_algorithm=(SELECT source.hash_algorithm FROM transfer.env_snapshots source \
                 WHERE source.hash=env_snapshots.hash) \
             WHERE hash IN (SELECT hash FROM transfer.env_snapshots)",
        )
        .execute(&mut **tx)
        .await?;
    }
    if attached_table_exists(tx, "external_resources").await? {
        sqlx::query(
            "INSERT INTO external_resources(\
               id,project_id,kind,uri,version,checksum,size_bytes,license,visibility,\
               access_instructions,accessed_at,created_at,updated_at\
             ) SELECT id,project_id,kind,uri,version,checksum,size_bytes,license,visibility,\
                      access_instructions,accessed_at,created_at,updated_at \
               FROM transfer.external_resources WHERE project_id=?",
        )
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
    }
    if attached_table_exists(tx, "run_inputs").await? {
        sqlx::query(
            "INSERT INTO run_inputs(\
               id,run_id,artifact_version_id,external_resource_id,source_ref,role,required,\
               basis,confidence,created_at\
             ) SELECT input.id,input.run_id,input.artifact_version_id,\
                      input.external_resource_id,input.source_ref,input.role,input.required,\
                      input.basis,input.confidence,input.created_at \
               FROM transfer.run_inputs input \
               JOIN transfer.runs run ON run.id=input.run_id WHERE run.project_id=?",
        )
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
    }
    if attached_table_exists(tx, "run_outputs").await? {
        sqlx::query(
            "INSERT INTO run_outputs(\
               id,run_id,artifact_version_id,role,logical_output_key,source_path,created_at\
             ) SELECT output.id,output.run_id,output.artifact_version_id,output.role,\
                      output.logical_output_key,output.source_path,output.created_at \
               FROM transfer.run_outputs output \
               JOIN transfer.runs run ON run.id=output.run_id WHERE run.project_id=?",
        )
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
    }
    if attached_table_exists(tx, "run_code_snapshots").await? {
        sqlx::query(
            "INSERT INTO run_code_snapshots(\
               id,run_id,source_kind,source_path,source_text,checksum,storage_path,\
               git_commit,dirty_patch,created_at\
             ) SELECT code.id,code.run_id,code.source_kind,code.source_path,code.source_text,\
                      code.checksum,code.storage_path,code.git_commit,code.dirty_patch,\
                      code.created_at \
               FROM transfer.run_code_snapshots code \
               JOIN transfer.runs run ON run.id=code.run_id WHERE run.project_id=?",
        )
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
    }
    if attached_table_exists(tx, "run_environment_snapshots").await? {
        sqlx::query(
            "INSERT OR IGNORE INTO env_snapshots(\
               hash,env_name,packages_json,snapshot_json,hash_algorithm,created_at\
             ) SELECT environment.hash,environment.env_name,environment.packages_json,\
                      environment.snapshot_json,environment.hash_algorithm,\
                      environment.created_at \
               FROM transfer.env_snapshots environment \
               WHERE environment.hash IN (\
                 SELECT link.env_snapshot_hash \
                 FROM transfer.run_environment_snapshots link \
                 JOIN transfer.runs run ON run.id=link.run_id WHERE run.project_id=?\
               )",
        )
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "INSERT INTO run_environment_snapshots(run_id,env_snapshot_hash) \
             SELECT link.run_id,link.env_snapshot_hash \
             FROM transfer.run_environment_snapshots link \
             JOIN transfer.runs run ON run.id=link.run_id WHERE run.project_id=?",
        )
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
    }
    copy_publication_children(tx, project_id).await?;
    if attached_table_exists(tx, "agent_workflow_attempts").await? {
        let attempt_columns = attached_table_columns(tx, "agent_workflow_attempts").await?;
        let has_lineage = [
            "root_workflow_id",
            "parent_attempt_id",
            "depth",
            "allow_delegation",
            "delegation_slot_yielded",
        ]
        .iter()
        .all(|column| attempt_columns.contains(*column));
        let lineage = if has_lineage {
            "a.root_workflow_id,a.parent_attempt_id,a.depth,a.allow_delegation,0"
        } else {
            "a.workflow_id,NULL,1,0,0"
        };
        let query = format!(
            "INSERT INTO agent_workflow_attempts(id,workflow_id,step_id,root_workflow_id,parent_attempt_id,depth,allow_delegation,delegation_slot_yielded,attempt,request_id,backend,status,request_json,response_json,output_json,artifact_ids_json,evidence_json,error,agent_session_id,child_frame_id,input_tokens,output_tokens,tool_calls,cost_microunits,cancel_requested,started_at,finished_at,created_at,updated_at) \
             SELECT a.id,a.workflow_id,a.step_id,{lineage},a.attempt,a.request_id,a.backend,a.status,a.request_json,a.response_json,a.output_json,a.artifact_ids_json,a.evidence_json,a.error,a.agent_session_id,a.child_frame_id,a.input_tokens,a.output_tokens,a.tool_calls,a.cost_microunits,a.cancel_requested,a.started_at,a.finished_at,a.created_at,a.updated_at \
             FROM transfer.agent_workflow_attempts a JOIN transfer.agent_workflows w ON w.id=a.workflow_id WHERE w.project_id=?"
        );
        sqlx::query(&query)
            .bind(project_id)
            .execute(&mut **tx)
            .await?;
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "UPDATE agent_workflow_attempts SET status='failed',error=COALESCE(error,'Imported from another device; the Workflow activity was not resumed.'),finished_at=COALESCE(finished_at,?),updated_at=? WHERE workflow_id IN (SELECT id FROM agent_workflows WHERE project_id=?) AND status IN ('queued','running','waiting_run')",
        )
        .bind(now)
        .bind(now)
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "UPDATE agent_workflows SET status='failed',updated_at=? WHERE project_id=? AND status='running'",
        )
        .bind(now)
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
    }
    if attached_table_exists(tx, "agent_workflow_run_activities").await? {
        sqlx::query(
            "INSERT INTO agent_workflow_run_activities(attempt_id,run_id,activity,state_json,created_at,updated_at) \
             SELECT link.attempt_id,link.run_id,link.activity,link.state_json,link.created_at,link.updated_at \
             FROM transfer.agent_workflow_run_activities link \
             JOIN transfer.agent_workflow_attempts attempt ON attempt.id=link.attempt_id \
             JOIN transfer.agent_workflows workflow ON workflow.id=attempt.workflow_id \
             JOIN transfer.runs run ON run.id=link.run_id \
             WHERE workflow.project_id=? AND run.project_id=?",
        )
        .bind(project_id)
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
    }
    if attached_table_exists(tx, "method_search_runs").await? {
        sqlx::query(
            "INSERT INTO method_search_runs(run_id,spec_artifact_version_id,spec_sha256,activity_version,checkpoint_json,control_state,result_status,created_at,updated_at) \
             SELECT state.run_id,state.spec_artifact_version_id,state.spec_sha256,state.activity_version,state.checkpoint_json,'run',state.result_status,state.created_at,state.updated_at \
             FROM transfer.method_search_runs state JOIN transfer.runs run ON run.id=state.run_id WHERE run.project_id=?",
        )
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "INSERT INTO method_candidate_blobs(id,run_id,kind,checksum,size_bytes,storage_path,created_at) \
             SELECT blob.id,blob.run_id,blob.kind,blob.checksum,blob.size_bytes,blob.storage_path,blob.created_at \
             FROM transfer.method_candidate_blobs blob JOIN transfer.runs run ON run.id=blob.run_id WHERE run.project_id=?",
        )
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "INSERT INTO method_candidates(id,run_id,parent_candidate_id,sequence,strategy_key,family,status,primary_score,utility,metrics_json,runtime_ms,source_sha256,patch_sha256,source_blob_id,patch_blob_id,changed_lines,dependency_count,rationale,diagnostic_summary,error,created_at,finished_at) \
             SELECT candidate.id,candidate.run_id,candidate.parent_candidate_id,candidate.sequence,candidate.strategy_key,candidate.family,candidate.status,candidate.primary_score,candidate.utility,candidate.metrics_json,candidate.runtime_ms,candidate.source_sha256,candidate.patch_sha256,candidate.source_blob_id,candidate.patch_blob_id,candidate.changed_lines,candidate.dependency_count,candidate.rationale,candidate.diagnostic_summary,candidate.error,candidate.created_at,candidate.finished_at \
             FROM transfer.method_candidates candidate JOIN transfer.runs run ON run.id=candidate.run_id WHERE run.project_id=? ORDER BY candidate.sequence,candidate.id",
        )
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "INSERT INTO method_strategy_stats(run_id,strategy_key,category,weight,attempts,improvements,cumulative_reward,summary,source_refs_json,updated_at) \
             SELECT stat.run_id,stat.strategy_key,stat.category,stat.weight,stat.attempts,stat.improvements,stat.cumulative_reward,stat.summary,stat.source_refs_json,stat.updated_at \
             FROM transfer.method_strategy_stats stat JOIN transfer.runs run ON run.id=stat.run_id WHERE run.project_id=?",
        )
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
    }
    if attached_table_exists(tx, "agent_workflow_deliveries").await? {
        sqlx::query(
            "INSERT INTO agent_workflow_deliveries(id,workflow_id,frame_id,generation,auto_resume,result_json,message_seq,delivered_at,resume_status,resume_error,presented_at,created_at,updated_at) \
             SELECT d.id,d.workflow_id,d.frame_id,d.generation,d.auto_resume,d.result_json,d.message_seq,d.delivered_at,\
               CASE WHEN d.resume_status='running' THEN 'interrupted' ELSE d.resume_status END,\
               CASE WHEN d.resume_status='running' THEN COALESCE(d.resume_error,'Imported while auto-resume was running.') ELSE d.resume_error END,\
               d.presented_at,d.created_at,d.updated_at \
             FROM transfer.agent_workflow_deliveries d JOIN transfer.agent_workflows w ON w.id=d.workflow_id \
             WHERE w.project_id=?",
        )
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "UPDATE agent_workflows SET status='failed',version=version+1,updated_at=? \
             WHERE project_id=? AND status='approved' AND EXISTS (\
               SELECT 1 FROM agent_workflow_deliveries d \
               WHERE d.workflow_id=agent_workflows.id AND d.result_json IS NULL)",
        )
        .bind(chrono::Utc::now().timestamp())
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn attached_table_exists(tx: &mut Transaction<'_, Sqlite>, table: &str) -> Result<bool> {
    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM transfer.sqlite_master WHERE type='table' AND name=? LIMIT 1",
    )
    .bind(table)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(exists.is_some())
}

async fn attached_table_columns(
    tx: &mut Transaction<'_, Sqlite>,
    table: &str,
) -> Result<HashSet<String>> {
    // `table` is always a hard-coded name at the call sites; it cannot be
    // bound in PRAGMA syntax, so keep this helper private and non-generic.
    let rows = sqlx::query(&format!("PRAGMA transfer.table_info({table})"))
        .fetch_all(&mut **tx)
        .await?;
    Ok(rows
        .iter()
        .map(|row| row.try_get::<String, _>("name"))
        .collect::<std::result::Result<HashSet<_>, _>>()?)
}

async fn copy_publication_children(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
) -> Result<()> {
    if !attached_table_exists(tx, "publications").await? {
        return Ok(());
    }
    let frozen_revisions = sqlx::query(
        "SELECT revision.id,revision.manifest_json,revision.manifest_sha256 \
         FROM transfer.publication_revisions revision \
         JOIN transfer.publications publication ON publication.id=revision.publication_id \
         WHERE publication.project_id=? AND revision.state IN ('frozen','published')",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut frozen_hashes = std::collections::HashMap::new();
    for row in frozen_revisions {
        let revision_id: String = row.try_get("id")?;
        let manifest_json: Option<String> = row.try_get("manifest_json")?;
        let manifest_sha256: Option<String> = row.try_get("manifest_sha256")?;
        let manifest_json = manifest_json
            .ok_or_else(|| anyhow::anyhow!("Frozen Publication revision lacks a manifest"))?;
        let manifest: serde_json::Value = serde_json::from_str(&manifest_json)
            .context("Frozen Publication manifest is invalid JSON")?;
        let (canonical, hash) = canonical_json_sha256(&manifest);
        if canonical != manifest_json || manifest_sha256.as_deref() != Some(hash.as_str()) {
            anyhow::bail!("Frozen Publication manifest hash is invalid");
        }
        frozen_hashes.insert(revision_id, hash);
    }
    if attached_table_exists(tx, "publication_readiness_reports").await? {
        let reports = sqlx::query(
            "SELECT report.revision_id,report.manifest_json,report.manifest_sha256 \
             FROM transfer.publication_readiness_reports report \
             JOIN transfer.publication_revisions revision ON revision.id=report.revision_id \
             JOIN transfer.publications publication ON publication.id=revision.publication_id \
             WHERE publication.project_id=? AND revision.state IN ('frozen','published')",
        )
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
        for row in reports {
            let revision_id: String = row.try_get("revision_id")?;
            let manifest_json: String = row.try_get("manifest_json")?;
            let manifest_sha256: String = row.try_get("manifest_sha256")?;
            let manifest: serde_json::Value = serde_json::from_str(&manifest_json)
                .context("Publication readiness manifest is invalid JSON")?;
            let (canonical, hash) = canonical_json_sha256(&manifest);
            if canonical != manifest_json
                || manifest_sha256 != hash
                || frozen_hashes.get(&revision_id) != Some(&hash)
            {
                anyhow::bail!("Publication readiness manifest does not match its frozen revision");
            }
        }
    }
    sqlx::query(
        "INSERT INTO publications(id,project_id,title,description,created_at,updated_at) \
         SELECT id,project_id,title,description,created_at,updated_at \
         FROM transfer.publications WHERE project_id=?",
    )
    .bind(project_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO publication_revisions(\
           id,publication_id,parent_revision_id,revision_number,label,state,capability_level,\
           manifest_json,manifest_sha256,frozen_at,published_at,created_at,updated_at\
         ) SELECT revision.id,revision.publication_id,NULL,revision.revision_number,\
                  revision.label,'draft','archived',NULL,NULL,NULL,NULL,\
                  revision.created_at,revision.updated_at \
           FROM transfer.publication_revisions revision \
           JOIN transfer.publications publication ON publication.id=revision.publication_id \
           WHERE publication.project_id=?",
    )
    .bind(project_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "INSERT INTO publication_items(\
           id,revision_id,parent_item_id,kind,title,content,ordinal,metadata_json,\
           created_at,updated_at\
         ) SELECT item.id,item.revision_id,NULL,item.kind,item.title,item.content,item.ordinal,\
                  item.metadata_json,item.created_at,item.updated_at \
           FROM transfer.publication_items item \
           JOIN transfer.publication_revisions revision ON revision.id=item.revision_id \
           JOIN transfer.publications publication ON publication.id=revision.publication_id \
           WHERE publication.project_id=?",
    )
    .bind(project_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE publication_items SET parent_item_id=(\
           SELECT source.parent_item_id FROM transfer.publication_items source \
           WHERE source.id=publication_items.id\
         ) WHERE revision_id IN (\
           SELECT revision.id FROM publication_revisions revision \
           JOIN publications publication ON publication.id=revision.publication_id \
           WHERE publication.project_id=?\
         )",
    )
    .bind(project_id)
    .execute(&mut **tx)
    .await?;
    if attached_table_exists(tx, "publication_readiness_reports").await? {
        let columns = attached_table_columns(tx, "publication_readiness_reports").await?;
        let target_visibility = if columns.contains("target_visibility") {
            "report.target_visibility"
        } else {
            "'public'"
        };
        let policy_json = if columns.contains("policy_json") {
            "report.policy_json"
        } else {
            "'{}'"
        };
        let statement = format!(
            "INSERT INTO publication_readiness_reports(\
               id,revision_id,capability_level,target_visibility,policy_json,blockers_json,\
               warnings_json,omissions_json,manifest_json,manifest_sha256,created_at\
             ) SELECT report.id,report.revision_id,report.capability_level,\
                      {target_visibility},{policy_json},report.blockers_json,\
                      report.warnings_json,report.omissions_json,report.manifest_json,\
                      report.manifest_sha256,report.created_at \
               FROM transfer.publication_readiness_reports report \
               JOIN transfer.publication_revisions revision ON revision.id=report.revision_id \
               JOIN transfer.publications publication ON publication.id=revision.publication_id \
               WHERE publication.project_id=?"
        );
        sqlx::query(&statement)
            .bind(project_id)
            .execute(&mut **tx)
            .await?;
    }
    for statement in [
        "INSERT INTO publication_item_links(\
           id,revision_id,source_item_id,target_item_id,relation,created_at\
         ) SELECT link.id,link.revision_id,link.source_item_id,link.target_item_id,\
                  link.relation,link.created_at \
           FROM transfer.publication_item_links link \
           JOIN transfer.publication_revisions revision ON revision.id=link.revision_id \
           JOIN transfer.publications publication ON publication.id=revision.publication_id \
           WHERE publication.project_id=?",
        "INSERT INTO evidence_bindings(\
           id,revision_id,item_id,source_kind,source_id,artifact_version_id,run_id,\
           external_resource_id,purpose,supported_claim_item_id,selection_state,review_state,\
           reproduction_state,visibility,source_snapshot_json,created_at,updated_at\
         ) SELECT binding.id,binding.revision_id,binding.item_id,binding.source_kind,\
                  binding.source_id,binding.artifact_version_id,binding.run_id,\
                  binding.external_resource_id,binding.purpose,\
                  binding.supported_claim_item_id,binding.selection_state,binding.review_state,\
                  binding.reproduction_state,binding.visibility,binding.source_snapshot_json,\
                  binding.created_at,binding.updated_at \
           FROM transfer.evidence_bindings binding \
           JOIN transfer.publication_revisions revision ON revision.id=binding.revision_id \
           JOIN transfer.publications publication ON publication.id=revision.publication_id \
           WHERE publication.project_id=?",
        "INSERT INTO evidence_reviews(\
           id,binding_id,reviewer,method,verified_at,environment_json,comparator_json,\
           tolerance_json,result,report_json,created_at\
         ) SELECT review.id,review.binding_id,review.reviewer,review.method,review.verified_at,\
                  review.environment_json,review.comparator_json,review.tolerance_json,\
                  review.result,review.report_json,review.created_at \
           FROM transfer.evidence_reviews review \
           JOIN transfer.evidence_bindings binding ON binding.id=review.binding_id \
           JOIN transfer.publication_revisions revision ON revision.id=binding.revision_id \
           JOIN transfer.publications publication ON publication.id=revision.publication_id \
           WHERE publication.project_id=?",
        "INSERT INTO evidence_supersessions(\
           id,revision_id,old_binding_id,new_binding_id,reason,created_at\
         ) SELECT supersession.id,supersession.revision_id,supersession.old_binding_id,\
                  supersession.new_binding_id,supersession.reason,supersession.created_at \
           FROM transfer.evidence_supersessions supersession \
           JOIN transfer.publication_revisions revision \
             ON revision.id=supersession.revision_id \
           JOIN transfer.publications publication ON publication.id=revision.publication_id \
           WHERE publication.project_id=?",
        "INSERT INTO publication_waivers(\
           id,revision_id,finding_code,author,reason,created_at\
         ) SELECT waiver.id,waiver.revision_id,waiver.finding_code,waiver.author,\
                  waiver.reason,waiver.created_at \
           FROM transfer.publication_waivers waiver \
           JOIN transfer.publication_revisions revision ON revision.id=waiver.revision_id \
           JOIN transfer.publications publication ON publication.id=revision.publication_id \
           WHERE publication.project_id=?",
        "INSERT INTO capsule_builds(\
           id,revision_id,format,visibility,status,output_path,revision_manifest_sha256,\
           archive_sha256,error,created_at,completed_at\
         ) SELECT build.id,build.revision_id,build.format,build.visibility,build.status,\
                  build.output_path,build.revision_manifest_sha256,build.archive_sha256,\
                  build.error,build.created_at,build.completed_at \
           FROM transfer.capsule_builds build \
           JOIN transfer.publication_revisions revision ON revision.id=build.revision_id \
           JOIN transfer.publications publication ON publication.id=revision.publication_id \
           WHERE publication.project_id=?",
    ] {
        sqlx::query(statement)
            .bind(project_id)
            .execute(&mut **tx)
            .await?;
    }
    if attached_table_exists(tx, "reproduction_runs").await? {
        sqlx::query(
            "INSERT INTO reproduction_runs(\
               id,revision_id,source_run_id,status,capability_level,command_sha256,\
               expected_environment_hash,actual_environment_json,actual_environment_hash,\
               environment_matched,workspace_manifest_json,stdout_tail,stderr_tail,exit_code,\
               error,created_at,started_at,completed_at\
             ) SELECT reproduction.id,reproduction.revision_id,reproduction.source_run_id,\
                      CASE WHEN reproduction.status='running' THEN 'failed' ELSE reproduction.status END,\
                      reproduction.capability_level,reproduction.command_sha256,\
                      reproduction.expected_environment_hash,reproduction.actual_environment_json,\
                      reproduction.actual_environment_hash,reproduction.environment_matched,\
                      reproduction.workspace_manifest_json,reproduction.stdout_tail,\
                      reproduction.stderr_tail,reproduction.exit_code,\
                      CASE WHEN reproduction.status='running' THEN 'Interrupted by project transfer'\
                           ELSE reproduction.error END,\
                      reproduction.created_at,reproduction.started_at,\
                      COALESCE(reproduction.completed_at,\
                               CASE WHEN reproduction.status='running' THEN reproduction.created_at END) \
               FROM transfer.reproduction_runs reproduction \
               JOIN transfer.publication_revisions revision \
                 ON revision.id=reproduction.revision_id \
               JOIN transfer.publications publication ON publication.id=revision.publication_id \
               WHERE publication.project_id=?",
        )
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
    }
    if attached_table_exists(tx, "reproduction_results").await? {
        sqlx::query(
            "INSERT INTO reproduction_results(\
               id,reproduction_run_id,output_id,output_path,expected_artifact_version_id,\
               comparator_kind,required,expected_json,actual_json,tolerance_json,passed,\
               report_json,created_at\
             ) SELECT result.id,result.reproduction_run_id,result.output_id,result.output_path,\
                      result.expected_artifact_version_id,result.comparator_kind,result.required,\
                      result.expected_json,result.actual_json,result.tolerance_json,result.passed,\
                      result.report_json,result.created_at \
               FROM transfer.reproduction_results result \
               JOIN transfer.reproduction_runs reproduction \
                 ON reproduction.id=result.reproduction_run_id \
               JOIN transfer.publication_revisions revision \
                 ON revision.id=reproduction.revision_id \
               JOIN transfer.publications publication ON publication.id=revision.publication_id \
               WHERE publication.project_id=?",
        )
        .bind(project_id)
        .execute(&mut **tx)
        .await?;
    }
    sqlx::query(
        "UPDATE publication_revisions SET \
           parent_revision_id=(SELECT source.parent_revision_id \
             FROM transfer.publication_revisions source \
             WHERE source.id=publication_revisions.id),\
           state=(SELECT CASE WHEN source.state IN ('deleting','freezing') \
                              THEN 'draft' ELSE source.state END \
             FROM transfer.publication_revisions source \
             WHERE source.id=publication_revisions.id),\
           capability_level=(SELECT source.capability_level \
             FROM transfer.publication_revisions source \
             WHERE source.id=publication_revisions.id),\
           manifest_json=(SELECT source.manifest_json \
             FROM transfer.publication_revisions source \
             WHERE source.id=publication_revisions.id),\
           manifest_sha256=(SELECT source.manifest_sha256 \
             FROM transfer.publication_revisions source \
             WHERE source.id=publication_revisions.id),\
           frozen_at=(SELECT source.frozen_at \
             FROM transfer.publication_revisions source \
             WHERE source.id=publication_revisions.id),\
           published_at=(SELECT source.published_at \
             FROM transfer.publication_revisions source \
             WHERE source.id=publication_revisions.id) \
         WHERE publication_id IN (SELECT id FROM publications WHERE project_id=?)",
    )
    .bind(project_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Remove every row owned by a project while retaining the project itself.
/// SQLite foreign keys are not enabled on legacy stores, so replacement must
/// spell out the cascade in dependency order.
pub(crate) async fn delete_project_children(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
) -> Result<()> {
    const QUERIES: &[&str] = &[
        "UPDATE agent_workflows SET status='draft' WHERE project_id=?",
        "DELETE FROM publication_freeze_attempts WHERE revision_id IN (SELECT revision.id FROM publication_revisions revision JOIN publications publication ON publication.id=revision.publication_id WHERE publication.project_id=?)",
        "UPDATE publication_revisions SET state='deleting' WHERE publication_id IN (SELECT id FROM publications WHERE project_id=?)",
        "DELETE FROM reproduction_results WHERE reproduction_run_id IN (SELECT reproduction.id FROM reproduction_runs reproduction JOIN publication_revisions revision ON revision.id=reproduction.revision_id JOIN publications publication ON publication.id=revision.publication_id WHERE publication.project_id=?)",
        "DELETE FROM reproduction_runs WHERE revision_id IN (SELECT revision.id FROM publication_revisions revision JOIN publications publication ON publication.id=revision.publication_id WHERE publication.project_id=?)",
        "DELETE FROM method_strategy_stats WHERE run_id IN (SELECT id FROM runs WHERE project_id=?)",
        "DELETE FROM method_candidates WHERE run_id IN (SELECT id FROM runs WHERE project_id=?)",
        "DELETE FROM method_candidate_blobs WHERE run_id IN (SELECT id FROM runs WHERE project_id=?)",
        "DELETE FROM method_search_runs WHERE run_id IN (SELECT id FROM runs WHERE project_id=?)",
        "DELETE FROM capsule_builds WHERE revision_id IN (SELECT revision.id FROM publication_revisions revision JOIN publications publication ON publication.id=revision.publication_id WHERE publication.project_id=?)",
        "DELETE FROM publication_readiness_reports WHERE revision_id IN (SELECT revision.id FROM publication_revisions revision JOIN publications publication ON publication.id=revision.publication_id WHERE publication.project_id=?)",
        "DELETE FROM publication_waivers WHERE revision_id IN (SELECT revision.id FROM publication_revisions revision JOIN publications publication ON publication.id=revision.publication_id WHERE publication.project_id=?)",
        "DELETE FROM evidence_reviews WHERE binding_id IN (SELECT binding.id FROM evidence_bindings binding JOIN publication_revisions revision ON revision.id=binding.revision_id JOIN publications publication ON publication.id=revision.publication_id WHERE publication.project_id=?)",
        "DELETE FROM evidence_supersessions WHERE revision_id IN (SELECT revision.id FROM publication_revisions revision JOIN publications publication ON publication.id=revision.publication_id WHERE publication.project_id=?)",
        "DELETE FROM evidence_bindings WHERE revision_id IN (SELECT revision.id FROM publication_revisions revision JOIN publications publication ON publication.id=revision.publication_id WHERE publication.project_id=?)",
        "DELETE FROM publication_item_links WHERE revision_id IN (SELECT revision.id FROM publication_revisions revision JOIN publications publication ON publication.id=revision.publication_id WHERE publication.project_id=?)",
        "DELETE FROM publication_items WHERE revision_id IN (SELECT revision.id FROM publication_revisions revision JOIN publications publication ON publication.id=revision.publication_id WHERE publication.project_id=?)",
        "DELETE FROM publication_revisions WHERE publication_id IN (SELECT id FROM publications WHERE project_id=?)",
        "DELETE FROM publications WHERE project_id=?",
        "DELETE FROM run_environment_snapshots WHERE run_id IN (SELECT id FROM runs WHERE project_id=?)",
        "DELETE FROM run_code_snapshots WHERE run_id IN (SELECT id FROM runs WHERE project_id=?)",
        "DELETE FROM run_outputs WHERE run_id IN (SELECT id FROM runs WHERE project_id=?)",
        "DELETE FROM run_inputs WHERE run_id IN (SELECT id FROM runs WHERE project_id=?)",
        "DELETE FROM run_artifacts WHERE run_id IN (SELECT id FROM runs WHERE project_id=?)",
        "DELETE FROM artifact_dependencies WHERE artifact_version_id IN (SELECT av.id FROM artifact_versions av JOIN artifacts a ON a.id=av.artifact_id WHERE a.project_id=?)",
        "DELETE FROM agent_workflow_deliveries WHERE workflow_id IN (SELECT id FROM agent_workflows WHERE project_id=?)",
        "DELETE FROM agent_workflow_run_activities WHERE attempt_id IN (SELECT a.id FROM agent_workflow_attempts a JOIN agent_workflows w ON w.id=a.workflow_id WHERE w.project_id=?)",
        "DELETE FROM agent_workflow_attempts WHERE workflow_id IN (SELECT id FROM agent_workflows WHERE project_id=?)",
        "DELETE FROM agent_workflow_steps WHERE workflow_id IN (SELECT id FROM agent_workflows WHERE project_id=?)",
        "DELETE FROM agent_workflows WHERE project_id=?",
        "DELETE FROM schedule_runs WHERE schedule_id IN (SELECT id FROM schedules WHERE project_id=?)",
        "DELETE FROM schedules WHERE project_id=?",
        "DELETE FROM project_plugins WHERE project_id=?",
        "DELETE FROM context_storage_prefs WHERE project_id=?",
        "DELETE FROM remote_staging WHERE project_id=?",
        "DELETE FROM exploration_promotions WHERE exploration_id IN (SELECT exploration.id FROM explorations exploration JOIN exploration_checkpoints checkpoint ON checkpoint.id=exploration.checkpoint_id WHERE checkpoint.project_id=?)",
        "DELETE FROM exploration_effects WHERE exploration_id IN (SELECT exploration.id FROM explorations exploration JOIN exploration_checkpoints checkpoint ON checkpoint.id=exploration.checkpoint_id WHERE checkpoint.project_id=?)",
        "DELETE FROM exploration_baseline_artifact_heads WHERE checkpoint_id IN (SELECT id FROM exploration_checkpoints WHERE project_id=?)",
        "DELETE FROM exploration_baseline_entities WHERE checkpoint_id IN (SELECT id FROM exploration_checkpoints WHERE project_id=?)",
        "DELETE FROM explorations WHERE checkpoint_id IN (SELECT id FROM exploration_checkpoints WHERE project_id=?)",
        "DELETE FROM exploration_checkpoints WHERE project_id=?",
        "DELETE FROM exploration_families WHERE project_id=?",
        "DELETE FROM artifact_heads WHERE project_id=?",
        "DELETE FROM project_state_revisions WHERE project_id=?",
        "DELETE FROM context_archives WHERE project_id=?",
        "DELETE FROM workspace_snapshots WHERE project_id=?",
        "DELETE FROM project_state_counters WHERE project_id=?",
        "UPDATE global_memories SET source_frame_id=NULL WHERE source_frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM session_branch_merges WHERE EXISTS (SELECT 1 FROM frames frame WHERE frame.project_id=? AND (frame.id=session_branch_merges.source_frame_id OR frame.id=session_branch_merges.branch_frame_id))",
        "DELETE FROM ask_user_requests WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM session_imports WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM codex_imports WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM message_resource_links WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM session_execution_contexts WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM artifact_versions WHERE artifact_id IN (SELECT id FROM artifacts WHERE project_id=?)",
        "DELETE FROM session_reviews WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM session_ui_events WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM turn_file_undo WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM proposed_plans WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM codex_turn_configs WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM acp_sessions WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM execution_log WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM messages WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)",
        "DELETE FROM research_edges WHERE project_id=?",
        "DELETE FROM research_nodes WHERE project_id=?",
        "DELETE FROM artifacts WHERE project_id=?",
        "DELETE FROM external_resources WHERE project_id=?",
        "DELETE FROM runs WHERE project_id=?",
        "DELETE FROM frames WHERE project_id=?",
        "DELETE FROM folders WHERE project_id=?",
    ];
    for query in QUERIES {
        sqlx::query(query)
            .bind(project_id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

async fn sanitize_export_machine_state(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
) -> Result<()> {
    // Handles, leases, remote work directories and the launch environment can
    // contain hostnames, process ids and private-key paths. They are runtime
    // state, not portable research history.
    sqlx::query(
        "UPDATE runs SET remote_workdir=NULL,remote_handle_json=NULL,\
         lifecycle_owner=NULL,lifecycle_lease_until=NULL,progress_json='{}',env_snapshot_json='{}' \
         WHERE project_id=?",
    )
    .bind(project_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE capsule_builds SET output_path=NULL \
         WHERE revision_id IN (\
           SELECT revision.id FROM publication_revisions revision \
           JOIN publications publication ON publication.id=revision.publication_id \
           WHERE publication.project_id=?\
         )",
    )
    .bind(project_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn is_windows_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    (bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/')
        || path.starts_with("//")
}

fn normalize_path(raw: &str) -> Option<(String, bool)> {
    let path = raw.replace('\\', "/");
    let windows = is_windows_absolute(&path);
    let (prefix, rest, absolute) = if windows && path.as_bytes().get(1) == Some(&b':') {
        (&path[..2], &path[3..], true)
    } else if path.starts_with("//") {
        ("//", path.trim_start_matches('/'), true)
    } else if let Some(rest) = path.strip_prefix('/') {
        ("/", rest, true)
    } else {
        ("", path.as_str(), false)
    };
    let mut parts = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.pop().is_none() => return None,
            ".." => {}
            value => parts.push(value),
        }
    }
    let joined = parts.join("/");
    let normalized = match prefix {
        "/" => format!("/{joined}"),
        "//" => format!("//{joined}"),
        "" => joined,
        drive => format!("{drive}/{joined}"),
    };
    Some((normalized, absolute))
}

fn unavailable_path(raw: &str) -> String {
    let name = raw
        .replace('\\', "/")
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or("file")
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("wisp-unavailable://source-local/{name}")
}

/// Convert a path written on the source OS to the archive's portable form.
/// This intentionally does not use host `Path` parsing so Windows drive paths
/// are testable and recognizable when the importer runs on macOS/Linux.
fn portable_project_path(source_root: &str, raw: &str) -> (String, bool) {
    let value = raw.trim();
    if let Some(file_path) = value.strip_prefix("file://") {
        let file_path = if file_path.starts_with('/')
            && file_path.as_bytes().get(2) == Some(&b':')
            && file_path
                .as_bytes()
                .get(1)
                .is_some_and(|byte| byte.is_ascii_alphabetic())
        {
            &file_path[1..]
        } else {
            file_path
        };
        return portable_project_path(source_root, file_path);
    }
    if value.contains("://") {
        return (value.to_string(), false);
    }
    let Some((path, absolute)) = normalize_path(value) else {
        return (unavailable_path(value), true);
    };
    if !absolute {
        return (path, false);
    }
    let Some((root, root_absolute)) = normalize_path(source_root) else {
        return (unavailable_path(value), true);
    };
    if !root_absolute {
        return (unavailable_path(value), true);
    }
    let windows = is_windows_absolute(&root);
    let (candidate, base) = if windows {
        (path.to_ascii_lowercase(), root.to_ascii_lowercase())
    } else {
        (path.clone(), root.clone())
    };
    if candidate == base {
        return (String::new(), false);
    }
    let prefix = format!("{}/", base.trim_end_matches('/'));
    if candidate.starts_with(&prefix) {
        return (path[prefix.len()..].to_string(), false);
    }
    (unavailable_path(value), true)
}

fn restored_project_path(workspace: &Path, archived: &str) -> Result<String> {
    if archived.contains("://") {
        return Ok(archived.to_string());
    }
    let (relative, absolute) = normalize_path(archived)
        .ok_or_else(|| anyhow::anyhow!("archive contains an unsafe project path"))?;
    if absolute {
        anyhow::bail!("archive contains a non-portable absolute project path");
    }
    let workspace = workspace.to_string_lossy();
    let separator = if workspace.contains('\\') && !workspace.contains('/') {
        '\\'
    } else {
        '/'
    };
    let workspace = workspace.trim_end_matches(['/', '\\']);
    if relative.is_empty() {
        return Ok(workspace.to_string());
    }
    let relative = if separator == '\\' {
        relative.replace('/', "\\")
    } else {
        relative
    };
    Ok(format!("{workspace}{separator}{relative}"))
}

async fn rewrite_export_paths(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
    source_root: &str,
) -> Result<i64> {
    let mut warnings = 0;
    let artifacts = sqlx::query("SELECT id,storage_path FROM artifacts WHERE project_id=?")
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
    for row in artifacts {
        let id: String = row.try_get("id")?;
        let value: String = row.try_get("storage_path")?;
        let (portable, warned) = portable_project_path(source_root, &value);
        warnings += i64::from(warned);
        sqlx::query("UPDATE artifacts SET storage_path=? WHERE id=?")
            .bind(portable)
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    let versions = sqlx::query(
        "SELECT av.id,av.storage_path FROM artifact_versions av \
         JOIN artifacts a ON a.id=av.artifact_id WHERE a.project_id=?",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await?;
    for row in versions {
        let id: String = row.try_get("id")?;
        let value: String = row.try_get("storage_path")?;
        let (portable, warned) = portable_project_path(source_root, &value);
        warnings += i64::from(warned);
        sqlx::query("UPDATE artifact_versions SET storage_path=? WHERE id=?")
            .bind(portable)
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    let runs = sqlx::query(
        "SELECT id,script_path,input_refs_json,output_specs_json FROM runs WHERE project_id=?",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await?;
    for row in runs {
        let id: String = row.try_get("id")?;
        let script_path: Option<String> = row.try_get("script_path")?;
        let script_path = script_path.map(|value| {
            let (portable, warned) = portable_project_path(source_root, &value);
            warnings += i64::from(warned);
            portable
        });
        let input_refs: String = row.try_get("input_refs_json")?;
        let input_refs = serde_json::from_str::<Vec<String>>(&input_refs)
            .map(|paths| {
                paths
                    .into_iter()
                    .map(|value| {
                        let (portable, warned) = portable_project_path(source_root, &value);
                        warnings += i64::from(warned);
                        portable
                    })
                    .collect::<Vec<_>>()
            })
            .and_then(|paths| serde_json::to_string(&paths))
            .unwrap_or(input_refs);
        let output_specs: String = row.try_get("output_specs_json")?;
        let output_specs = serde_json::from_str::<serde_json::Value>(&output_specs)
            .map(|mut value| {
                if let Some(specs) = value.as_array_mut() {
                    for spec in specs {
                        let Some(glob) = spec.get_mut("glob") else {
                            continue;
                        };
                        let Some(path) = glob.as_str() else { continue };
                        let (portable, warned) = portable_project_path(source_root, path);
                        warnings += i64::from(warned);
                        *glob = serde_json::Value::String(portable);
                    }
                }
                value
            })
            .and_then(|value| serde_json::to_string(&value))
            .unwrap_or(output_specs);
        sqlx::query(
            "UPDATE runs SET script_path=?,input_refs_json=?,output_specs_json=? WHERE id=?",
        )
        .bind(script_path)
        .bind(input_refs)
        .bind(output_specs)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    }
    let inputs = sqlx::query(
        "SELECT input.id,input.source_ref FROM run_inputs input \
         JOIN runs run ON run.id=input.run_id WHERE run.project_id=?",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await?;
    for row in inputs {
        let id: String = row.try_get("id")?;
        let value: String = row.try_get("source_ref")?;
        let (portable, warned) = portable_project_path(source_root, &value);
        warnings += i64::from(warned);
        sqlx::query("UPDATE run_inputs SET source_ref=? WHERE id=?")
            .bind(portable)
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    let outputs = sqlx::query(
        "SELECT output.id,output.source_path FROM run_outputs output \
         JOIN runs run ON run.id=output.run_id WHERE run.project_id=?",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await?;
    for row in outputs {
        let id: String = row.try_get("id")?;
        let value: String = row.try_get("source_path")?;
        let (portable, warned) = portable_project_path(source_root, &value);
        warnings += i64::from(warned);
        sqlx::query("UPDATE run_outputs SET source_path=? WHERE id=?")
            .bind(portable)
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    let code = sqlx::query(
        "SELECT code.id,code.source_path,code.storage_path FROM run_code_snapshots code \
         JOIN runs run ON run.id=code.run_id WHERE run.project_id=?",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await?;
    for row in code {
        let id: String = row.try_get("id")?;
        let source_path: Option<String> = row.try_get("source_path")?;
        let storage_path: Option<String> = row.try_get("storage_path")?;
        let source_path = source_path.map(|value| {
            let (portable, warned) = portable_project_path(source_root, &value);
            warnings += i64::from(warned);
            portable
        });
        let storage_path = storage_path.map(|value| {
            let (portable, warned) = portable_project_path(source_root, &value);
            warnings += i64::from(warned);
            portable
        });
        sqlx::query("UPDATE run_code_snapshots SET source_path=?,storage_path=? WHERE id=?")
            .bind(source_path)
            .bind(storage_path)
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(warnings)
}

async fn restore_import_paths(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
    workspace: &Path,
) -> Result<()> {
    let artifacts = sqlx::query("SELECT id,storage_path FROM artifacts WHERE project_id=?")
        .bind(project_id)
        .fetch_all(&mut **tx)
        .await?;
    for row in artifacts {
        let id: String = row.try_get("id")?;
        let value: String = row.try_get("storage_path")?;
        sqlx::query("UPDATE artifacts SET storage_path=? WHERE id=?")
            .bind(restored_project_path(workspace, &value)?)
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    let versions = sqlx::query(
        "SELECT av.id,av.storage_path FROM artifact_versions av \
         JOIN artifacts a ON a.id=av.artifact_id WHERE a.project_id=?",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await?;
    for row in versions {
        let id: String = row.try_get("id")?;
        let value: String = row.try_get("storage_path")?;
        sqlx::query("UPDATE artifact_versions SET storage_path=? WHERE id=?")
            .bind(restored_project_path(workspace, &value)?)
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    let runs = sqlx::query(
        "SELECT id,script_path FROM runs WHERE project_id=? AND script_path IS NOT NULL",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await?;
    for row in runs {
        let id: String = row.try_get("id")?;
        let value: String = row.try_get("script_path")?;
        sqlx::query("UPDATE runs SET script_path=? WHERE id=?")
            .bind(restored_project_path(workspace, &value)?)
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

impl Store {
    async fn database_path(&self) -> Result<PathBuf> {
        let rows = sqlx::query("PRAGMA database_list")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .find(|row| matches!(row.try_get::<String, _>("name"), Ok(name) if name == "main"))
            .and_then(|row| row.try_get::<String, _>("file").ok())
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("project transfer requires a file-backed database"))
    }

    /// Stable logical fingerprint of a filtered project database. SQLite file
    /// headers and page layouts are intentionally ignored because they can
    /// change after VACUUM or across operating systems without a data change.
    pub async fn portable_project_database_hash(database: &Path) -> Result<String> {
        const TABLES: &[(&str, &str, &str)] = &[
            // `updated_at` is touched when a project is merely opened on one
            // device. Name/description still participate in the fingerprint.
            (
                "projects",
                "id,name,description,workspace_dir,created_at",
                "id",
            ),
            ("folders", "*", "id"),
            ("agent_workflows", "*", "id"),
            ("agent_workflow_steps", "*", "id"),
            ("agent_workflow_attempts", "*", "id"),
            ("agent_workflow_run_activities", "*", "attempt_id"),
            ("method_search_runs", "*", "run_id"),
            ("method_candidate_blobs", "*", "id"),
            ("method_candidates", "*", "id"),
            ("method_strategy_stats", "*", "run_id,strategy_key"),
            ("agent_workflow_deliveries", "*", "id"),
            ("frames", "*", "id"),
            ("messages", "*", "id"),
            ("session_reviews", "*", "id"),
            ("session_ui_events", "*", "frame_id,seq"),
            ("proposed_plans", "*", "id"),
            ("codex_turn_configs", "*", "id"),
            ("execution_log", "*", "id"),
            ("env_snapshots", "*", "hash"),
            ("runs", "*", "id"),
            ("artifacts", "*", "id"),
            ("artifact_versions", "*", "id"),
            ("message_resource_links", "*", "id"),
            ("artifact_dependencies", "*", "id"),
            ("run_artifacts", "*", "id"),
            ("external_resources", "*", "id"),
            ("run_inputs", "*", "id"),
            ("run_outputs", "*", "id"),
            ("run_code_snapshots", "*", "id"),
            ("run_environment_snapshots", "*", "run_id"),
            ("publications", "*", "id"),
            ("publication_revisions", "*", "id"),
            ("publication_items", "*", "id"),
            ("publication_item_links", "*", "id"),
            ("evidence_bindings", "*", "id"),
            ("evidence_reviews", "*", "id"),
            ("evidence_supersessions", "*", "id"),
            ("publication_readiness_reports", "*", "id"),
            ("publication_waivers", "*", "id"),
            ("capsule_builds", "*", "id"),
            ("reproduction_runs", "*", "id"),
            ("reproduction_results", "*", "id"),
            ("research_nodes", "*", "id"),
            ("research_edges", "*", "id"),
        ];
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", database.display()))?
            .read_only(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        let mut digest = Sha256::new();
        for (table, columns, order) in TABLES {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
            )
            .bind(table)
            .fetch_one(&pool)
            .await?;
            if !exists {
                continue;
            }
            digest.update((table.len() as u64).to_le_bytes());
            digest.update(table.as_bytes());
            let query = format!("SELECT {columns} FROM {table} ORDER BY {order}");
            let mut rows = sqlx::query(&query).fetch(&pool);
            while let Some(row) = rows.try_next().await? {
                for (index, column) in row.columns().iter().enumerate() {
                    let raw = row.try_get_raw(index)?;
                    digest.update((column.name().len() as u64).to_le_bytes());
                    digest.update(column.name().as_bytes());
                    if raw.is_null() {
                        digest.update([0]);
                        continue;
                    }
                    digest.update([1]);
                    let bytes = match column.type_info().name() {
                        "INTEGER" | "INT8" => row.try_get::<i64, _>(index)?.to_le_bytes().to_vec(),
                        "REAL" | "FLOAT8" => row
                            .try_get::<f64, _>(index)?
                            .to_bits()
                            .to_le_bytes()
                            .to_vec(),
                        "BLOB" => row.try_get::<Vec<u8>, _>(index)?,
                        _ => row.try_get::<String, _>(index)?.into_bytes(),
                    };
                    digest.update((bytes.len() as u64).to_le_bytes());
                    digest.update(bytes);
                }
            }
        }
        pool.close().await;
        Ok(hex::encode(digest.finalize()))
    }

    /// Build a standalone, filtered SQLite snapshot for one project. Paths in
    /// operational columns are workspace-relative and slash-normalized.
    pub async fn export_project_database(
        &self,
        project_id: &str,
        destination: &Path,
    ) -> Result<ProjectTransferStats> {
        let (_, source_root) = self
            .get_project(project_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("project not found"))?;
        if destination.exists() {
            std::fs::remove_file(destination)?;
        }
        let source_db = self.database_path().await?;
        // Rollback-journal mode: the snapshot must end up as one standalone
        // file, and leaving WAL later would need an exclusive lock that
        // ignores `busy_timeout` and flakes with SQLITE_BUSY.
        let transfer = Store::open_snapshot(destination).await?;
        let mut connection = transfer.pool.acquire().await?;
        sqlx::query("ATTACH DATABASE ? AS transfer")
            .bind(source_db.to_string_lossy().as_ref())
            .execute(&mut *connection)
            .await?;
        let result: Result<i64> = async {
            let mut tx = connection.begin().await?;
            sqlx::query(
                "INSERT INTO projects(id,name,description,workspace_dir,created_at,updated_at) \
                 SELECT id,name,description,'',created_at,updated_at FROM transfer.projects WHERE id=?",
            )
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
            copy_project_children(&mut tx, project_id).await?;
            sanitize_export_machine_state(&mut tx, project_id).await?;
            let warnings = rewrite_export_paths(&mut tx, project_id, &source_root).await?;
            tx.commit().await?;
            Ok(warnings)
        }
        .await;
        let _ = sqlx::query("DETACH DATABASE transfer")
            .execute(&mut *connection)
            .await;
        drop(connection);
        let warnings = result?;
        // Store::open creates machine-global defaults and records wall-clock
        // migration times. Remove/normalize them so an unchanged project makes
        // the same portable snapshot on every device.
        for query in [
            "DELETE FROM settings",
            "DELETE FROM execution_contexts",
            "DELETE FROM project_sync_state",
            "UPDATE wisp_schema_migrations SET applied_at=0",
        ] {
            sqlx::query(query).execute(&transfer.pool).await?;
        }
        let counts: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT \
               (SELECT COUNT(*) FROM frames WHERE project_id=?), \
               (SELECT COUNT(*) FROM messages WHERE frame_id IN (SELECT id FROM frames WHERE project_id=?)), \
               (SELECT COUNT(*) FROM artifacts WHERE project_id=?), \
               (SELECT COUNT(*) FROM runs WHERE project_id=?)",
        )
        .bind(project_id)
        .bind(project_id)
        .bind(project_id)
        .bind(project_id)
        .fetch_one(&transfer.pool)
        .await?;
        sqlx::query("VACUUM").execute(&transfer.pool).await?;
        transfer.pool.close().await;
        Ok(ProjectTransferStats {
            frames: counts.0,
            messages: counts.1,
            artifacts: counts.2,
            runs: counts.3,
            path_warnings: warnings,
        })
    }

    /// Import a v1 project snapshot into the live store. Project ids remain
    /// stable; a duplicate id is rejected instead of merging two histories.
    pub async fn import_project_database(
        &self,
        archive_database: &Path,
        project_id: &str,
        workspace: &Path,
    ) -> Result<()> {
        if self.get_project(project_id).await?.is_some() {
            anyhow::bail!("this project is already present on this device");
        }
        let mut connection = self.pool.acquire().await?;
        sqlx::query("ATTACH DATABASE ? AS transfer")
            .bind(archive_database.to_string_lossy().as_ref())
            .execute(&mut *connection)
            .await
            .context("invalid project metadata database")?;
        let result: Result<()> = async {
            let archived_projects: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM transfer.projects WHERE id=?")
                    .bind(project_id)
                    .fetch_one(&mut *connection)
                    .await?;
            let all_projects: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transfer.projects")
                .fetch_one(&mut *connection)
                .await?;
            if archived_projects != 1 || all_projects != 1 {
                anyhow::bail!("project archive metadata does not match its manifest");
            }
            let mut tx = connection.begin().await?;
            sqlx::query(
                "INSERT INTO projects(id,name,description,workspace_dir,created_at,updated_at) \
                 SELECT id,name,description,?,created_at,updated_at FROM transfer.projects WHERE id=?",
            )
            .bind(workspace.to_string_lossy().as_ref())
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
            copy_project_children(&mut tx, project_id).await?;
            restore_import_paths(&mut tx, project_id, workspace).await?;
            sqlx::query(
                "UPDATE runs SET status='lost', ended_at=COALESCE(ended_at,?), \
                 last_poll_error=COALESCE(last_poll_error,'Imported from another device; the run was not resumed.'), \
                 lifecycle_owner=NULL,lifecycle_lease_until=NULL \
                 WHERE project_id=? AND status IN ('submitted','running','cancelling')",
            )
            .bind(chrono::Utc::now().timestamp())
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(())
        }
        .await;
        let _ = sqlx::query("DETACH DATABASE transfer")
            .execute(&mut *connection)
            .await;
        result.context("could not import project metadata")
    }

    /// Replace an existing project's portable rows from a trusted, decrypted
    /// sync snapshot. The local workspace root and sync cursor remain
    /// device-specific; the cursor update commits in the same SQLite
    /// transaction as the project replacement.
    pub async fn replace_project_database(
        &self,
        archive_database: &Path,
        project_id: &str,
        workspace: &Path,
        sync_state: &ProjectSyncState,
    ) -> Result<()> {
        if sync_state.project_id != project_id {
            anyhow::bail!("sync cursor does not belong to the replaced project");
        }
        if self.get_project(project_id).await?.is_none() {
            anyhow::bail!("project to replace was not found");
        }
        let mut connection = self.pool.acquire().await?;
        sqlx::query("ATTACH DATABASE ? AS transfer")
            .bind(archive_database.to_string_lossy().as_ref())
            .execute(&mut *connection)
            .await
            .context("invalid project sync metadata database")?;
        let result: Result<()> = async {
            let archived_projects: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM transfer.projects WHERE id=?")
                    .bind(project_id)
                    .fetch_one(&mut *connection)
                    .await?;
            let all_projects: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM transfer.projects")
                    .fetch_one(&mut *connection)
                    .await?;
            if archived_projects != 1 || all_projects != 1 {
                anyhow::bail!("sync metadata does not match the project");
            }
            let mut tx = connection.begin().await?;
            delete_project_children(&mut tx, project_id).await?;
            sqlx::query(
                "UPDATE projects SET \
                 name=(SELECT name FROM transfer.projects WHERE id=?), \
                 description=(SELECT description FROM transfer.projects WHERE id=?), \
                 created_at=(SELECT created_at FROM transfer.projects WHERE id=?), \
                 updated_at=(SELECT updated_at FROM transfer.projects WHERE id=?) \
                 WHERE id=?",
            )
            .bind(project_id)
            .bind(project_id)
            .bind(project_id)
            .bind(project_id)
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
            copy_project_children(&mut tx, project_id).await?;
            restore_import_paths(&mut tx, project_id, workspace).await?;
            sqlx::query(
                "UPDATE runs SET status='lost',ended_at=COALESCE(ended_at,?),\
                 last_poll_error=COALESCE(last_poll_error,'Synced from another device; the run was not resumed.'),\
                 remote_workdir=NULL,remote_handle_json=NULL,lifecycle_owner=NULL,\
                 lifecycle_lease_until=NULL,progress_json='{}',env_snapshot_json='{}' \
                 WHERE project_id=? AND status IN ('submitted','running','cancelling')",
            )
            .bind(chrono::Utc::now().timestamp())
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO project_sync_state(\
                 project_id,transport_kind,transport_location,relay_project_id,base_revision,base_state_hash,\
                 base_manifest_json,last_synced_at,last_direction) VALUES(?,?,?,?,?,?,?,?,?) \
                 ON CONFLICT(project_id) DO UPDATE SET transport_kind=excluded.transport_kind,\
                 transport_location=excluded.transport_location,relay_project_id=excluded.relay_project_id,base_revision=excluded.base_revision,\
                 base_state_hash=excluded.base_state_hash,base_manifest_json=excluded.base_manifest_json,\
                 last_synced_at=excluded.last_synced_at,last_direction=excluded.last_direction",
            )
            .bind(&sync_state.project_id)
            .bind(&sync_state.transport_kind)
            .bind(&sync_state.transport_location)
            .bind(&sync_state.relay_project_id)
            .bind(&sync_state.base_revision)
            .bind(&sync_state.base_state_hash)
            .bind(&sync_state.base_manifest_json)
            .bind(sync_state.last_synced_at)
            .bind(&sync_state.last_direction)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(())
        }
        .await;
        let _ = sqlx::query("DETACH DATABASE transfer")
            .execute(&mut *connection)
            .await;
        result.context("could not replace project metadata from sync")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ArtifactCaptureTiming, ArtifactMaterialization, ArtifactVersionDraft, EvidenceBindingDraft,
        EvidenceReview, EvidenceSelectionState, EvidenceSourceKind, EvidenceSupersession,
        EvidenceVisibility, LineageBasis, LineageConfidence, MethodCandidate, MethodCandidateBlob,
        MethodCandidateStatus, MethodSearchRunState, MethodStrategyStat, PublicationItem,
        PublicationItemKind, PublicationRevisionState, PublicationWaiver,
        ReproductionComparatorKind, ReproductionResult, ReproductionRunCommit,
        ReproductionRunStart, RunCodeSnapshot, RunInput, RunOutput, RunRecord, RunStatus,
    };
    use wisp_llm::Message;

    #[test]
    fn windows_paths_become_portable_and_restore_on_macos() {
        let root = r"C:\Users\Alice\Wisp\Study";
        assert_eq!(
            portable_project_path(root, r"c:\users\alice\wisp\study\figures\plot.png"),
            ("figures/plot.png".into(), false)
        );
        assert_eq!(
            portable_project_path(root, r"file://C:\Users\Alice\Wisp\Study\data\x.csv"),
            ("data/x.csv".into(), false)
        );
        assert_eq!(
            portable_project_path(root, "file:///C:/Users/Alice/Wisp/Study/data/y.csv"),
            ("data/y.csv".into(), false)
        );
        let (outside, warned) = portable_project_path(root, r"D:\shared\large.fastq");
        assert!(warned);
        assert!(outside.starts_with("wisp-unavailable://"));
        assert_eq!(
            restored_project_path(Path::new("/Users/alice/Study"), "figures/plot.png").unwrap(),
            "/Users/alice/Study/figures/plot.png"
        );
        assert_eq!(
            restored_project_path(Path::new(r"C:\Users\Alice\Study"), "figures/plot.png").unwrap(),
            r"C:\Users\Alice\Study\figures\plot.png"
        );
        assert!(restored_project_path(Path::new("/tmp/study"), "../escape").is_err());
    }

    #[tokio::test]
    async fn method_search_checkpoint_candidates_and_strategy_roundtrip() {
        let token = uuid::Uuid::new_v4();
        let source_path =
            std::env::temp_dir().join(format!("wisp_method_transfer_source_{token}.sqlite"));
        let archive_path =
            std::env::temp_dir().join(format!("wisp_method_transfer_archive_{token}.sqlite"));
        let target_path =
            std::env::temp_dir().join(format!("wisp_method_transfer_target_{token}.sqlite"));
        let source = Store::open(&source_path).await.unwrap();
        source
            .create_project("project", "Method project", "workspace")
            .await
            .unwrap();
        source
            .create_frame("frame", "project", "Method search", "model")
            .await
            .unwrap();
        let spec_version = source
            .save_artifact_version(&ArtifactVersionDraft {
                version_id: Some("spec-version".into()),
                artifact_id: "spec-artifact".into(),
                project_id: "project".into(),
                root_frame_id: "frame".into(),
                filename: "method-search.json".into(),
                content_type: "application/json".into(),
                storage_path: ".wisp/artifacts/sha256/aa/spec.json".into(),
                logical_key: Some("method-search:spec".into()),
                size_bytes: Some(2),
                checksum: Some("a".repeat(64)),
                producing_run_id: None,
                env_snapshot_hash: None,
                materialization: ArtifactMaterialization::Snapshot,
                capture_timing: ArtifactCaptureTiming::AtCreation,
            })
            .await
            .unwrap();
        let run = RunRecord::new("run", "project", "local", "Method search", "method_search");
        source.create_run(&run).await.unwrap();
        source
            .create_method_search_run_state(
                &MethodSearchRunState::new("run", &spec_version, "a".repeat(64)).unwrap(),
            )
            .await
            .unwrap();
        let blob = MethodCandidateBlob {
            id: "source-blob".into(),
            run_id: "run".into(),
            kind: "source".into(),
            checksum: "b".repeat(64),
            size_bytes: 12,
            storage_path: ".wisp/method-search/run/blobs/bb/source.py".into(),
            created_at: 1,
        };
        source.save_method_candidate_blob(&blob).await.unwrap();
        let mut candidate = MethodCandidate::proposed(
            "candidate",
            "run",
            0,
            "baseline",
            "baseline",
            "b".repeat(64),
            "c".repeat(64),
        )
        .unwrap();
        source.insert_method_candidate(&candidate).await.unwrap();
        assert!(source
            .transition_method_candidate_to_evaluating(&candidate.id)
            .await
            .unwrap());
        candidate.status = MethodCandidateStatus::Succeeded;
        candidate.primary_score = Some(0.5);
        candidate.utility = Some(0.5);
        candidate.metrics_json = r#"{"accuracy":0.5}"#.into();
        candidate.source_blob_id = Some(blob.id.clone());
        candidate.finished_at = Some(2);
        assert!(source
            .finish_method_candidate(&candidate, MethodCandidateStatus::Evaluating)
            .await
            .unwrap());
        let strategy = MethodStrategyStat {
            run_id: "run".into(),
            strategy_key: "diagnostic".into(),
            category: "diagnostic".into(),
            weight: 0.25,
            attempts: 1,
            improvements: 1,
            cumulative_reward: 1.0,
            summary: "Inspect residuals".into(),
            source_refs_json: "[]".into(),
            updated_at: 2,
        };
        source.upsert_method_strategy_stat(&strategy).await.unwrap();

        source
            .export_project_database("project", &archive_path)
            .await
            .unwrap();
        let target = Store::open(&target_path).await.unwrap();
        target
            .import_project_database(&archive_path, "project", Path::new("workspace-imported"))
            .await
            .unwrap();
        assert_eq!(
            target.get_method_search_run_state("run").await.unwrap(),
            source.get_method_search_run_state("run").await.unwrap()
        );
        assert_eq!(
            target.list_method_candidates("run").await.unwrap(),
            vec![candidate]
        );
        assert_eq!(
            target.list_method_strategy_stats("run").await.unwrap(),
            vec![strategy]
        );
        assert_eq!(
            target
                .find_method_candidate_blob("run", "source", &"b".repeat(64))
                .await
                .unwrap(),
            Some(blob)
        );

        source.pool.close().await;
        target.pool.close().await;
        for path in [source_path, archive_path, target_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[tokio::test]
    async fn workflow_run_activity_roundtrips_and_import_fails_waiting_attempt_explicitly() {
        let token = uuid::Uuid::new_v4();
        let source_path = std::env::temp_dir().join(format!("wisp_activity_source_{token}.sqlite"));
        let archive_path =
            std::env::temp_dir().join(format!("wisp_activity_archive_{token}.sqlite"));
        let target_path = std::env::temp_dir().join(format!("wisp_activity_target_{token}.sqlite"));
        let source = Store::open(&source_path).await.unwrap();
        source
            .create_project("project", "Method project", "workspace")
            .await
            .unwrap();
        let workflow =
            crate::AgentWorkflow::new("workflow", "project", "workspace", "Search").unwrap();
        let mut step = crate::AgentWorkflowStep::new(
            "activity-step",
            "workflow",
            0,
            "activity-step",
            "run_activity",
            "local",
            "host activity",
        )
        .unwrap();
        step.task_kind = "run_activity".into();
        step.activity_json = serde_json::json!({"activity":"method_search"}).to_string();
        source
            .create_agent_workflow_plan(&workflow, &[step])
            .await
            .unwrap();
        source
            .approve_agent_workflow_plan("workflow", 1)
            .await
            .unwrap();
        source
            .transition_agent_workflow_status(
                "workflow",
                crate::AgentWorkflowStatus::Approved,
                crate::AgentWorkflowStatus::Running,
            )
            .await
            .unwrap();
        let attempt = crate::AgentWorkflowAttempt::queued(
            "attempt",
            "workflow",
            "activity-step",
            1,
            "request",
            "local",
            "{}",
        )
        .unwrap();
        let attempt = match source
            .try_create_started_agent_workflow_attempt(attempt)
            .await
            .unwrap()
        {
            crate::AgentWorkflowAttemptStart::Started(attempt) => attempt,
            other => panic!("activity attempt did not start: {other:?}"),
        };
        let run = RunRecord::new("run", "project", "local", "Method search", "method_search");
        let link =
            crate::AgentWorkflowRunActivity::new(&attempt.id, &run.id, "method_search").unwrap();
        source
            .create_agent_workflow_run_activity(&run, &link)
            .await
            .unwrap();
        source
            .update_agent_workflow_run_activity_state("attempt", r#"{"checkpoint":"created"}"#)
            .await
            .unwrap();

        source
            .export_project_database("project", &archive_path)
            .await
            .unwrap();
        let target = Store::open(&target_path).await.unwrap();
        target
            .import_project_database(&archive_path, "project", Path::new("workspace-imported"))
            .await
            .unwrap();
        let imported = target
            .get_agent_workflow_run_activity("attempt")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(imported.run_id, "run");
        assert_eq!(imported.state_json, r#"{"checkpoint":"created"}"#);
        let imported_attempt = target
            .get_agent_workflow_attempt("attempt")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            imported_attempt.status,
            crate::AgentWorkflowAttemptStatus::Failed
        );
        assert!(imported_attempt
            .error
            .as_deref()
            .unwrap()
            .contains("not resumed"));

        source.pool.close().await;
        target.pool.close().await;
        for path in [source_path, archive_path, target_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[tokio::test]
    async fn project_database_roundtrip_rebinds_paths_and_stops_live_runs() {
        let token = uuid::Uuid::new_v4();
        let source_path = std::env::temp_dir().join(format!("wisp_transfer_source_{token}.sqlite"));
        let archive_path =
            std::env::temp_dir().join(format!("wisp_transfer_archive_{token}.sqlite"));
        let target_path = std::env::temp_dir().join(format!("wisp_transfer_target_{token}.sqlite"));
        let source = Store::open(&source_path).await.unwrap();
        source
            .create_project("project-1", "Study", r"C:\Users\Alice\Study")
            .await
            .unwrap();
        let workflow =
            crate::AgentWorkflow::new("workflow-1", "project-1", "workspace-1", "Review QC")
                .unwrap();
        let step = crate::AgentWorkflowStep::new(
            "workflow-step-1",
            "workflow-1",
            0,
            "reviewer",
            "reviewer",
            "acp",
            "Review {{input}}",
        )
        .unwrap();
        source
            .create_agent_workflow_plan(&workflow, &[step.clone()])
            .await
            .unwrap();
        let workflow = source
            .get_agent_workflow("workflow-1")
            .await
            .unwrap()
            .unwrap();
        source
            .create_frame("frame-1", "project-1", "OPERON", "model")
            .await
            .unwrap();
        source
            .append_message("frame-1", 1, &Message::user("hello"))
            .await
            .unwrap();
        let artifact_version_id = source
            .save_artifact_version(&ArtifactVersionDraft {
                version_id: None,
                artifact_id: "artifact-1".into(),
                project_id: "project-1".into(),
                root_frame_id: "frame-1".into(),
                filename: "plot.png".into(),
                content_type: "image/png".into(),
                storage_path: r"C:\Users\Alice\Study\.wisp\artifacts\sha256\ab\abcdef.png".into(),
                logical_key: Some("figure:qc".into()),
                size_bytes: Some(6),
                checksum: Some("a".repeat(64)),
                producing_run_id: None,
                env_snapshot_hash: None,
                materialization: ArtifactMaterialization::Snapshot,
                capture_timing: ArtifactCaptureTiming::AtCreation,
            })
            .await
            .unwrap();
        source
            .replace_message_resource_links(
                "frame-1",
                1,
                &[crate::MessageResourceLink {
                    id: "resource-link-1".into(),
                    frame_id: "frame-1".into(),
                    message_seq: 1,
                    ordinal: 0,
                    original_reference: r"D:/original/location/plot.png".into(),
                    artifact_id: Some("artifact-1".into()),
                    artifact_version_id: Some(artifact_version_id.clone()),
                    display_name: "plot.png".into(),
                    resource_kind: "image".into(),
                    mime_type: "image/png".into(),
                    status: "ready".into(),
                    error: None,
                    created_artifact: true,
                    created_version: true,
                    created_at: 1,
                }],
            )
            .await
            .unwrap();
        let mut run = RunRecord::new("run-1", "project-1", "local", "QC", "command");
        run.frame_id = Some("frame-1".into());
        run.script_path = Some(r"C:\Users\Alice\Study\analysis\qc.py".into());
        run.input_refs_json = r#"["C:\\Users\\Alice\\Study\\data\\counts.csv"]"#.into();
        run.output_specs_json =
            r#"[{"glob":"C:\\Users\\Alice\\Study\\results\\*.csv","kind":"table"}]"#.into();
        run.remote_workdir = Some("/home/alice/private-run".into());
        run.remote_handle_json =
            Some(r#"{"identity_file":"C:\\Users\\Alice\\.ssh\\id_ed25519","pid":42}"#.into());
        run.progress_json = r#"{"phase":"uploading"}"#.into();
        run.env_snapshot_json = r#"{"SSH_AUTH_SOCK":"/tmp/private-agent"}"#.into();
        run.status = RunStatus::Submitted;
        source.create_run(&run).await.unwrap();
        let input_version_id = source
            .save_artifact_version(&ArtifactVersionDraft {
                version_id: None,
                artifact_id: "input-artifact".into(),
                project_id: "project-1".into(),
                root_frame_id: "frame-1".into(),
                filename: "counts.csv".into(),
                content_type: "text/csv".into(),
                storage_path: r"C:\Users\Alice\Study\.wisp\artifacts\sha256\cd\counts.csv".into(),
                logical_key: Some("path:data/counts.csv".into()),
                size_bytes: Some(4),
                checksum: Some("c".repeat(64)),
                producing_run_id: None,
                env_snapshot_hash: None,
                materialization: ArtifactMaterialization::Snapshot,
                capture_timing: ArtifactCaptureTiming::AtCreation,
            })
            .await
            .unwrap();
        source
            .save_run_input(&RunInput {
                id: "input-1".into(),
                run_id: "run-1".into(),
                artifact_version_id: Some(input_version_id.clone()),
                external_resource_id: None,
                source_ref: r"C:\Users\Alice\Study\data\counts.csv".into(),
                role: "counts".into(),
                required: true,
                basis: LineageBasis::Declared,
                confidence: LineageConfidence::Exact,
                created_at: 1,
            })
            .await
            .unwrap();
        let environment = serde_json::json!({"context": {"id": "local"}, "schema_version": 1});
        let env_hash = source
            .record_run_environment_snapshot("run-1", Some("local"), &environment)
            .await
            .unwrap();
        source
            .save_run_code_snapshot(&RunCodeSnapshot {
                id: "code-1".into(),
                run_id: "run-1".into(),
                source_kind: "script".into(),
                source_path: Some(r"C:\Users\Alice\Study\analysis\qc.py".into()),
                source_text: "print('qc')".into(),
                checksum: "b".repeat(64),
                storage_path: None,
                git_commit: Some("deadbeef".into()),
                dirty_patch: None,
                created_at: 1,
            })
            .await
            .unwrap();
        source
            .save_run_output(&RunOutput {
                id: "output-1".into(),
                run_id: "run-1".into(),
                artifact_version_id: artifact_version_id.clone(),
                role: "figure".into(),
                logical_output_key: "figure:qc".into(),
                source_path: "results/plot.png".into(),
                created_at: 1,
            })
            .await
            .unwrap();
        source
            .save_artifact_dependency(
                "dependency-1",
                &artifact_version_id,
                &input_version_id,
                Some("counts"),
                LineageBasis::Declared,
                LineageConfidence::Exact,
            )
            .await
            .unwrap();
        source
            .create_publication(
                "publication-1",
                "project-1",
                "QC paper",
                "Transfer evidence",
            )
            .await
            .unwrap();
        source
            .create_publication_revision(
                "publication-revision-1",
                "publication-1",
                None,
                "Submission v1",
            )
            .await
            .unwrap();
        for (id, kind, title, ordinal) in [
            (
                "publication-figure",
                PublicationItemKind::Figure,
                "Figure 1",
                0,
            ),
            (
                "publication-methods",
                PublicationItemKind::Methods,
                "QC methods",
                1,
            ),
        ] {
            source
                .save_publication_item(&PublicationItem {
                    id: id.into(),
                    revision_id: "publication-revision-1".into(),
                    parent_item_id: None,
                    kind,
                    title: title.into(),
                    content: String::new(),
                    ordinal,
                    metadata_json: "{}".into(),
                    created_at: 1,
                    updated_at: 1,
                })
                .await
                .unwrap();
        }
        for binding in [
            EvidenceBindingDraft {
                id: "publication-binding-artifact".into(),
                revision_id: "publication-revision-1".into(),
                item_id: Some("publication-figure".into()),
                source_kind: EvidenceSourceKind::ArtifactVersion,
                source_id: artifact_version_id.clone(),
                purpose: "Published figure".into(),
                supported_claim_item_id: None,
                selection_state: EvidenceSelectionState::Selected,
                visibility: EvidenceVisibility::Public,
            },
            EvidenceBindingDraft {
                id: "publication-binding-run".into(),
                revision_id: "publication-revision-1".into(),
                item_id: Some("publication-methods".into()),
                source_kind: EvidenceSourceKind::Run,
                source_id: "run-1".into(),
                purpose: "Producing run".into(),
                supported_claim_item_id: None,
                selection_state: EvidenceSelectionState::Selected,
                visibility: EvidenceVisibility::Restricted,
            },
        ] {
            source.save_evidence_binding(&binding).await.unwrap();
        }
        source
            .save_evidence_review(&EvidenceReview {
                id: "publication-review".into(),
                binding_id: "publication-binding-artifact".into(),
                reviewer: "alice".into(),
                method: "manual".into(),
                verified_at: 1,
                environment_json: "{}".into(),
                comparator_json: "{}".into(),
                tolerance_json: "{}".into(),
                result: "passed".into(),
                report_json: "{}".into(),
                created_at: 1,
            })
            .await
            .unwrap();
        source
            .save_evidence_supersession(&EvidenceSupersession {
                id: "publication-supersession".into(),
                revision_id: "publication-revision-1".into(),
                old_binding_id: "publication-binding-artifact".into(),
                new_binding_id: "publication-binding-run".into(),
                reason: "fixture".into(),
                created_at: 1,
            })
            .await
            .unwrap();
        source
            .save_publication_waiver(&PublicationWaiver {
                id: "publication-waiver".into(),
                revision_id: "publication-revision-1".into(),
                finding_code: "restricted-input".into(),
                author: "alice".into(),
                reason: "manifest only".into(),
                created_at: 1,
            })
            .await
            .unwrap();
        let (_, publication_manifest_sha256) = canonical_json_sha256(&serde_json::json!({}));
        sqlx::query(
            "INSERT INTO publication_readiness_reports(\
               id,revision_id,capability_level,blockers_json,warnings_json,omissions_json,\
               manifest_json,manifest_sha256,created_at\
             ) VALUES('readiness','publication-revision-1','traceable','[]','[]','[]','{}',?,1)",
        )
        .bind(&publication_manifest_sha256)
        .execute(&source.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO capsule_builds(\
               id,revision_id,format,visibility,status,output_path,revision_manifest_sha256,\
               archive_sha256,error,created_at,completed_at\
             ) VALUES('capsule','publication-revision-1','zip','public','succeeded',?,? ,?,NULL,1,2)",
        )
        .bind(r"C:\Users\Alice\Study\exports\capsule.zip")
        .bind(&publication_manifest_sha256)
        .bind("f".repeat(64))
        .execute(&source.pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE publication_revisions SET state='frozen',capability_level='traceable',\
             manifest_json='{}',manifest_sha256=?,frozen_at=1 WHERE id='publication-revision-1'",
        )
        .bind(&publication_manifest_sha256)
        .execute(&source.pool)
        .await
        .unwrap();
        let (actual_environment_json, actual_environment_hash) =
            canonical_json_sha256(&serde_json::json!({"context": "transfer-test"}));
        let workspace_manifest_json = canonical_json_sha256(&serde_json::json!({"files": []})).0;
        source
            .start_reproduction_run(&ReproductionRunStart {
                id: "reproduction-complete".into(),
                revision_id: "publication-revision-1".into(),
                source_run_id: "run-1".into(),
                command_sha256: "1".repeat(64),
                expected_environment_hash: Some(actual_environment_hash.clone()),
                actual_environment_json: actual_environment_json.clone(),
                actual_environment_hash: actual_environment_hash.clone(),
                environment_matched: true,
                workspace_manifest_json: workspace_manifest_json.clone(),
            })
            .await
            .unwrap();
        source
            .complete_reproduction_run(&ReproductionRunCommit {
                run_id: "reproduction-complete".into(),
                stdout_tail: "done".into(),
                stderr_tail: String::new(),
                exit_code: 0,
                results: vec![ReproductionResult {
                    id: "reproduction-result".into(),
                    reproduction_run_id: "reproduction-complete".into(),
                    output_id: "output-1".into(),
                    output_path: "results/plot.png".into(),
                    expected_artifact_version_id: artifact_version_id.clone(),
                    comparator_kind: ReproductionComparatorKind::Sha256,
                    required: true,
                    expected_json: "{}".into(),
                    actual_json: "{}".into(),
                    tolerance_json: "{}".into(),
                    passed: true,
                    report_json: "{}".into(),
                    created_at: 0,
                }],
            })
            .await
            .unwrap();
        source
            .start_reproduction_run(&ReproductionRunStart {
                id: "reproduction-running".into(),
                revision_id: "publication-revision-1".into(),
                source_run_id: "run-1".into(),
                command_sha256: "1".repeat(64),
                expected_environment_hash: Some(actual_environment_hash.clone()),
                actual_environment_json,
                actual_environment_hash,
                environment_matched: true,
                workspace_manifest_json,
            })
            .await
            .unwrap();

        let stats = source
            .export_project_database("project-1", &archive_path)
            .await
            .unwrap();
        assert_eq!(stats.frames, 1);
        assert_eq!(stats.messages, 1);
        assert_eq!(stats.artifacts, 2);
        assert_eq!(stats.runs, 1);
        assert_eq!(stats.path_warnings, 0);
        // The snapshot must be one standalone rollback-journal file: header
        // bytes 18/19 are the file format versions (1 = legacy, 2 = WAL).
        let header = std::fs::read(&archive_path).unwrap();
        assert_eq!(&header[18..20], &[1, 1]);
        assert!(!archive_path.with_extension("sqlite-wal").exists());
        let archive_options =
            SqliteConnectOptions::from_str(&format!("sqlite://{}", archive_path.display()))
                .unwrap();
        let archive_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(archive_options)
            .await
            .unwrap();
        for column in ["target_visibility", "policy_json"] {
            sqlx::query(&format!(
                "ALTER TABLE publication_readiness_reports DROP COLUMN {column}"
            ))
            .execute(&archive_pool)
            .await
            .unwrap();
        }
        archive_pool.close().await;

        let target = Store::open(&target_path).await.unwrap();
        let workspace = Path::new("/Users/alice/Study");
        target
            .import_project_database(&archive_path, "project-1", workspace)
            .await
            .unwrap();
        assert_eq!(
            target.get_project("project-1").await.unwrap().unwrap().1,
            "/Users/alice/Study"
        );
        assert_eq!(target.load_messages("frame-1").await.unwrap().len(), 1);
        assert_eq!(
            target.list_agent_workflows("project-1").await.unwrap(),
            vec![workflow]
        );
        assert_eq!(
            target
                .list_agent_workflow_steps("workflow-1")
                .await
                .unwrap(),
            vec![step]
        );
        let imported_resources = target
            .list_message_resource_links("frame-1", 1, None)
            .await
            .unwrap();
        assert_eq!(imported_resources.len(), 1);
        assert_eq!(
            imported_resources[0].artifact_version_id.as_deref(),
            Some(artifact_version_id.as_str())
        );
        assert_eq!(
            imported_resources[0].original_reference,
            "D:/original/location/plot.png"
        );
        assert!(imported_resources[0].created_artifact);
        assert!(imported_resources[0].created_version);
        assert_eq!(
            target.get_artifact("artifact-1").await.unwrap().unwrap().2,
            "/Users/alice/Study/.wisp/artifacts/sha256/ab/abcdef.png"
        );
        assert_eq!(
            {
                let version = target
                    .get_artifact_version(&artifact_version_id)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(version.materialization, ArtifactMaterialization::Snapshot);
                assert_eq!(version.capture_timing, ArtifactCaptureTiming::AtCreation);
                version.storage_path
            },
            "/Users/alice/Study/.wisp/artifacts/sha256/ab/abcdef.png"
        );
        assert_eq!(
            target.list_run_outputs("run-1").await.unwrap()[0].artifact_version_id,
            artifact_version_id
        );
        assert_eq!(
            target.list_run_inputs("run-1").await.unwrap()[0]
                .artifact_version_id
                .as_deref(),
            Some(input_version_id.as_str())
        );
        let dependencies = target
            .list_artifact_dependencies(&artifact_version_id)
            .await
            .unwrap();
        assert_eq!(dependencies[0].depends_on_version_id, input_version_id);
        assert_eq!(dependencies[0].basis, LineageBasis::Declared);
        assert_eq!(
            target
                .get_run_environment_snapshot("run-1")
                .await
                .unwrap()
                .unwrap()
                .hash,
            env_hash
        );
        assert_eq!(
            target.list_run_code_snapshots("run-1").await.unwrap()[0].source_text,
            "print('qc')"
        );
        assert_eq!(
            target.list_run_code_snapshots("run-1").await.unwrap()[0]
                .source_path
                .as_deref(),
            Some("analysis/qc.py")
        );
        let imported_run = target.get_run("run-1").await.unwrap().unwrap();
        assert_eq!(imported_run.status, RunStatus::Lost);
        assert_eq!(
            imported_run.script_path.as_deref(),
            Some("/Users/alice/Study/analysis/qc.py")
        );
        assert_eq!(imported_run.input_refs_json, r#"["data/counts.csv"]"#);
        assert!(imported_run
            .output_specs_json
            .contains(r#""glob":"results/*.csv""#));
        assert!(imported_run.remote_workdir.is_none());
        assert!(imported_run.remote_handle_json.is_none());
        assert_eq!(imported_run.progress_json, "{}");
        assert_eq!(imported_run.env_snapshot_json, "{}");
        let imported_revision = target
            .get_publication_revision("publication-revision-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(imported_revision.state, PublicationRevisionState::Frozen);
        assert_eq!(
            target
                .list_publication_items("publication-revision-1")
                .await
                .unwrap()
                .len(),
            2
        );
        let imported_bindings = target
            .list_evidence_bindings("publication-revision-1")
            .await
            .unwrap();
        assert_eq!(imported_bindings.len(), 2);
        assert!(imported_bindings
            .iter()
            .any(|binding| binding.source_id == artifact_version_id));
        assert_eq!(
            target
                .list_evidence_reviews("publication-binding-artifact")
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            target
                .list_evidence_supersessions("publication-revision-1")
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            target
                .list_publication_waivers("publication-revision-1")
                .await
                .unwrap()
                .len(),
            1
        );
        let readiness = target
            .get_publication_readiness_report("publication-revision-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(readiness.target_visibility, EvidenceVisibility::Public);
        assert_eq!(readiness.policy_json, "{}");
        assert_eq!(readiness.manifest_sha256, publication_manifest_sha256);
        let builds = target
            .list_capsule_builds("publication-revision-1")
            .await
            .unwrap();
        assert_eq!(builds.len(), 1);
        assert_eq!(builds[0].id, "capsule");
        assert_eq!(builds[0].status, "succeeded");
        assert_eq!(
            builds[0].archive_sha256.as_deref(),
            Some("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
        );
        assert!(builds[0].output_path.is_none());
        let reproduction_runs = target
            .list_reproduction_runs("publication-revision-1")
            .await
            .unwrap();
        assert_eq!(reproduction_runs.len(), 2);
        let completed = reproduction_runs
            .iter()
            .find(|run| run.id == "reproduction-complete")
            .unwrap();
        assert_eq!(completed.status, "completed");
        assert_eq!(
            completed.capability_level,
            crate::PublicationCapabilityLevel::Reproduced
        );
        assert_eq!(
            target
                .list_reproduction_results("reproduction-complete")
                .await
                .unwrap()
                .len(),
            1
        );
        let interrupted = reproduction_runs
            .iter()
            .find(|run| run.id == "reproduction-running")
            .unwrap();
        assert_eq!(interrupted.status, "failed");
        assert_eq!(
            interrupted.error.as_deref(),
            Some("Interrupted by project transfer")
        );
        assert!(target
            .import_project_database(&archive_path, "project-1", workspace)
            .await
            .is_err());
        target.delete_project("project-1").await.unwrap();
        assert!(target
            .get_publication("publication-1")
            .await
            .unwrap()
            .is_none());

        source.pool.close().await;
        target.pool.close().await;
        for path in [source_path, archive_path, target_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[tokio::test]
    async fn import_rejects_a_corrupt_frozen_publication_manifest() {
        let token = uuid::Uuid::new_v4();
        let source_path = std::env::temp_dir().join(format!("wisp_manifest_source_{token}.sqlite"));
        let archive_path =
            std::env::temp_dir().join(format!("wisp_manifest_archive_{token}.sqlite"));
        let target_path = std::env::temp_dir().join(format!("wisp_manifest_target_{token}.sqlite"));
        let source = Store::open(&source_path).await.unwrap();
        source
            .create_project("project", "Study", "/tmp/study")
            .await
            .unwrap();
        source
            .create_publication("publication", "project", "Paper", "")
            .await
            .unwrap();
        source
            .create_publication_revision("revision", "publication", None, "Submission")
            .await
            .unwrap();
        let (manifest_json, manifest_sha256) =
            canonical_json_sha256(&serde_json::json!({"schema_version": 1}));
        sqlx::query(
            "UPDATE publication_revisions SET state='frozen',manifest_json=?,\
             manifest_sha256=?,frozen_at=1 WHERE id='revision'",
        )
        .bind(manifest_json)
        .bind(manifest_sha256)
        .execute(&source.pool)
        .await
        .unwrap();
        source
            .export_project_database("project", &archive_path)
            .await
            .unwrap();

        let archive_options =
            SqliteConnectOptions::from_str(&format!("sqlite://{}", archive_path.display()))
                .unwrap();
        let archive = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(archive_options)
            .await
            .unwrap();
        sqlx::query("DROP TRIGGER trg_publication_revision_immutable_update")
            .execute(&archive)
            .await
            .unwrap();
        sqlx::query("UPDATE publication_revisions SET manifest_sha256=? WHERE id='revision'")
            .bind("0".repeat(64))
            .execute(&archive)
            .await
            .unwrap();
        archive.close().await;

        let target = Store::open(&target_path).await.unwrap();
        let error = target
            .import_project_database(&archive_path, "project", Path::new("/tmp/imported"))
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("manifest hash"), "{error:#}");
        assert!(target.get_project("project").await.unwrap().is_none());

        source.pool.close().await;
        target.pool.close().await;
        for path in [source_path, archive_path, target_path] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[tokio::test]
    async fn portable_hash_ignores_open_recency_but_detects_project_edits() {
        let token = uuid::Uuid::new_v4();
        let source_path = std::env::temp_dir().join(format!("wisp_hash_source_{token}.sqlite"));
        let first_path = std::env::temp_dir().join(format!("wisp_hash_first_{token}.sqlite"));
        let second_path = std::env::temp_dir().join(format!("wisp_hash_second_{token}.sqlite"));
        let edited_path = std::env::temp_dir().join(format!("wisp_hash_edited_{token}.sqlite"));
        let source = Store::open(&source_path).await.unwrap();
        source
            .create_project("project-1", "Study", "/tmp/study")
            .await
            .unwrap();
        source
            .export_project_database("project-1", &first_path)
            .await
            .unwrap();
        sqlx::query("UPDATE projects SET updated_at=updated_at+100 WHERE id='project-1'")
            .execute(&source.pool)
            .await
            .unwrap();
        source
            .export_project_database("project-1", &second_path)
            .await
            .unwrap();
        assert_eq!(
            Store::portable_project_database_hash(&first_path)
                .await
                .unwrap(),
            Store::portable_project_database_hash(&second_path)
                .await
                .unwrap()
        );
        source
            .update_project("project-1", "Study", "changed")
            .await
            .unwrap();
        source
            .export_project_database("project-1", &edited_path)
            .await
            .unwrap();
        assert_ne!(
            Store::portable_project_database_hash(&first_path)
                .await
                .unwrap(),
            Store::portable_project_database_hash(&edited_path)
                .await
                .unwrap()
        );
        source.pool.close().await;
        for path in [source_path, first_path, second_path, edited_path] {
            let _ = std::fs::remove_file(path);
        }
    }
}

//! Scoped, read-only composer candidates. Selection carries stable IDs through
//! the same reference resolver as WebView; labels never substitute for IDs.
use crate::{native_conversations::call, native_settings::Broker};
use serde_json::{json, Value};
use wisp_dto::{
    native_conversations::{ReferenceCatalog, ReferenceKind, ReferenceOption, ReferenceRequest},
    ComposerReferenceArg as Reference,
};

fn text(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn matches(query: &str, values: &[&str]) -> bool {
    let haystack = values.join(" ").to_lowercase();
    query
        .split_whitespace()
        .all(|token| haystack.contains(&token.to_lowercase()))
}

fn option(reference: Reference, label: String, detail: String) -> ReferenceOption {
    ReferenceOption {
        reference,
        label,
        detail,
    }
}

pub(crate) async fn references(
    broker: &Broker,
    project: &str,
    session: &str,
    request: ReferenceRequest,
) -> Result<Value, String> {
    if request.session_id != session || request.query.len() > 512 {
        return Err("Invalid composer search".into());
    }
    let query = request.query.trim();
    let mut options = Vec::new();
    match request.kind {
        ReferenceKind::Artifact => {
            let rows = call(
                broker,
                project,
                "search_artifacts",
                json!({"query":query,"limit":40,"allProjects":true}),
            )
            .await?;
            for row in rows.as_array().ok_or("Invalid artifact search response")? {
                let id = text(row, "id");
                if !id.is_empty() {
                    options.push(option(
                        Reference::Artifact { id },
                        text(row, "name"),
                        text(row, "project_name"),
                    ));
                }
            }
            let rows = call(broker, project, "list_execution_contexts", json!({})).await?;
            let contexts: Vec<wisp_dto::ExecutionContext> =
                serde_json::from_value(rows).map_err(|e| e.to_string())?;
            for context in contexts {
                let label = if context.label.is_empty() {
                    &context.id
                } else {
                    &context.label
                };
                if matches(query, &["server", &context.kind, &context.id, label]) {
                    options.push(option(
                        Reference::Context {
                            id: context.id.clone(),
                        },
                        label.clone(),
                        "执行环境".into(),
                    ));
                }
                for language in ["python", "r"] {
                    if wisp_dto::native_conversations::runtime_reference_available(
                        &context, language,
                    ) && matches(
                        query,
                        &["runtime", language, &context.kind, &context.id, label],
                    ) {
                        options.push(option(
                            Reference::Runtime {
                                context_id: context.id.clone(),
                                language: language.into(),
                            },
                            format!("{label} · {language}"),
                            "运行时".into(),
                        ));
                    }
                }
            }
        }
        ReferenceKind::Session => {
            let current = call(broker, project, "get_project_info", json!({})).await?;
            let name = text(&current, "name");
            if matches(query, &["project", &name]) {
                options.push(option(
                    Reference::Project { id: project.into() },
                    name,
                    "项目上下文".into(),
                ));
            }
            let rows = call(
                broker,
                project,
                "search_sessions",
                json!({"query":query,"limit":40}),
            )
            .await?;
            for row in rows.as_array().ok_or("Invalid session search response")? {
                let id = text(row, "id");
                if !id.is_empty() && id != session {
                    options.push(option(
                        Reference::Session { id },
                        text(row, "title"),
                        text(row, "project_name"),
                    ));
                }
            }
        }
        ReferenceKind::Skill => {
            let workflows = call(broker, project, "list_workflow_templates", json!({})).await?;
            for row in workflows.as_array().ok_or("Invalid workflow response")? {
                let id = text(row, "id");
                let name = text(row, "name");
                let description = text(row, "description");
                let goal = row
                    .get("proposal")
                    .map(|proposal| text(proposal, "goal"))
                    .unwrap_or_default();
                if !id.is_empty() && matches(query, &[&name, &description, &goal]) {
                    options.push(option(Reference::Workflow { id }, name, description));
                }
            }
            let skills = call(broker, project, "list_skills", json!({})).await?;
            options.extend(skill_options(&skills, query)?);
        }
    }
    options.truncate(120);
    serde_json::to_value(ReferenceCatalog {
        session_id: session.into(),
        options,
    })
    .map_err(|e| e.to_string())
}

fn skill_options(rows: &Value, query: &str) -> Result<Vec<ReferenceOption>, String> {
    Ok(rows
        .as_array()
        .ok_or("Invalid skills response")?
        .iter()
        .filter_map(|row| {
            let name = text(row, "name");
            let description = text(row, "description");
            let tags = row.get("tags").map(Value::to_string).unwrap_or_default();
            (row.get("enabled").and_then(Value::as_bool) == Some(true)
                && !name.is_empty()
                && matches(query, &[&name, &description, &tags]))
            .then(|| option(Reference::Skill { name: name.clone() }, name, description))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_candidates_exclude_disabled_and_preserve_exact_names() {
        let rows = json!([
            {"name":"RNA-seq", "description":"Counts", "tags":["植物"], "enabled":true},
            {"name":"secret-disabled", "description":"Counts", "enabled":false},
            {"name":"", "description":"Counts", "enabled":true}
        ]);
        let options = skill_options(&rows, "植物").unwrap();
        assert_eq!(options.len(), 1);
        assert_eq!(
            options[0].reference,
            Reference::Skill {
                name: "RNA-seq".into()
            }
        );
        assert_eq!(skill_options(&rows, "COUNTS").unwrap().len(), 1);
        assert!(skill_options(&rows, "missing").unwrap().is_empty());
    }
}

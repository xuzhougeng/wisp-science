use std::collections::HashSet;
use std::io::{BufRead, Read, Write};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use wisp_dto::project_browser::{
    ActivitySource, Command, ErrorCode, Reply, Request, Response, SCHEMA,
};
use wisp_store::Store;

const MAX_REQUEST_BYTES: u64 = 64 * 1024;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let flag = args.next();
    let database = args.next();
    let allow_writes = args.next();
    if flag.as_deref() != Some(std::ffi::OsStr::new("--database"))
        || database.is_none()
        || allow_writes
            .as_deref()
            .is_some_and(|flag| flag != "--allow-project-writes")
        || args.next().is_some()
    {
        bail!("Usage: wisp-service --database <existing-wisp.sqlite> [--allow-project-writes]");
    }
    let database = PathBuf::from(database.unwrap());
    let store = Store::open_read_only(&database)
        .await
        .with_context(|| format!("Cannot open database read-only: {}", database.display()))?;
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    loop {
        let mut line = Vec::new();
        let count = (&mut input)
            .take(MAX_REQUEST_BYTES + 1)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            break;
        }
        if count as u64 > MAX_REQUEST_BYTES {
            bail!("Request exceeds 64 KiB");
        }
        let response = handle_request(&store, &database, allow_writes.is_some(), &line).await;
        serde_json::to_writer(&mut output, &response)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
    Ok(())
}

async fn handle_request(
    store: &Store,
    database: &std::path::Path,
    allow_writes: bool,
    line: &[u8],
) -> Response {
    let request = match serde_json::from_slice::<Request>(line) {
        Ok(request) => request,
        Err(error) => {
            return Response {
                schema: SCHEMA.into(),
                id: None,
                reply: Reply::Error {
                    code: ErrorCode::InvalidRequest,
                    message: error.to_string(),
                },
            };
        }
    };
    let reply = if request.schema != SCHEMA {
        Reply::Error {
            code: ErrorCode::UnsupportedSchema,
            message: format!("Expected {SCHEMA}"),
        }
    } else if request.id.trim().is_empty() {
        Reply::Error {
            code: ErrorCode::InvalidRequest,
            message: "Request id must not be empty".into(),
        }
    } else {
        match request.command {
            Command::Capabilities => Reply::Capabilities {
                commands: [
                    "list_projects".into(),
                    "list_sessions".into(),
                    "get_transcript".into(),
                    "search_project_chats".into(),
                    "locate_project_chat_match".into(),
                    "capabilities".into(),
                ]
                .into_iter()
                .chain(allow_writes.then(|| "set_project_starred".into()))
                .collect(),
                read_only: !allow_writes,
            },
            Command::SetProjectStarred {
                project_id,
                starred,
            } => {
                if !allow_writes {
                    Reply::Error {
                        code: ErrorCode::WriteDisabled,
                        message: "Project writes require --allow-project-writes".into(),
                    }
                } else {
                    let result = async {
                        let writer = Store::open_existing_for_commands(database).await?;
                        wisp_app::projects::set_project_starred(&writer, &project_id, starred)
                            .await?;
                        project_snapshot(store).await
                    }
                    .await;
                    match result {
                        Ok(reply) => reply,
                        Err(error) => Reply::Error {
                            code: ErrorCode::CommandFailed,
                            message: format!(
                                "{error}. Refresh to check the saved state before retrying."
                            ),
                        },
                    }
                }
            }
            Command::GetTranscript {
                project_id,
                session_id,
                before_seq,
            } => {
                match wisp_app::projects::browser_transcript(
                    store,
                    &project_id,
                    &session_id,
                    before_seq,
                )
                .await
                {
                    Ok((messages, next_before_seq)) => Reply::Transcript {
                        messages,
                        next_before_seq,
                    },
                    Err(error) => Reply::Error {
                        code: ErrorCode::QueryFailed,
                        message: error.to_string(),
                    },
                }
            }
            Command::ListSessions { project_id } => {
                match wisp_app::projects::list_browser_sessions(store, project_id.as_deref()).await
                {
                    Ok(sessions) => Reply::Sessions {
                        sessions,
                        activity_source: ActivitySource::PersistedOnly,
                    },
                    Err(error) => Reply::Error {
                        code: ErrorCode::QueryFailed,
                        message: error.to_string(),
                    },
                }
            }
            Command::SearchProjectChats {
                project_id,
                query,
                cursor,
            } => {
                match store
                    .search_project_chats(&project_id, &query, cursor.as_ref(), 50)
                    .await
                {
                    Ok(page) => Reply::ProjectChatMatches {
                        project_id,
                        query,
                        page,
                    },
                    Err(error) => Reply::Error {
                        code: ErrorCode::QueryFailed,
                        message: error.to_string(),
                    },
                }
            }
            Command::LocateProjectChatMatch {
                project_id,
                session_id,
                message_seq,
            } => {
                match store
                    .locate_project_chat_match(&project_id, &session_id, message_seq)
                    .await
                {
                    Ok(target) => Reply::ProjectChatLocation {
                        project_id,
                        session_id,
                        message_seq,
                        target,
                    },
                    Err(error) => Reply::Error {
                        code: ErrorCode::QueryFailed,
                        message: error.to_string(),
                    },
                }
            }
            Command::ListProjects => match project_snapshot(store).await {
                Ok(reply) => reply,
                Err(error) => Reply::Error {
                    code: ErrorCode::QueryFailed,
                    message: error.to_string(),
                },
            },
        }
    };
    Response {
        schema: SCHEMA.into(),
        id: Some(request.id),
        reply,
    }
}

async fn project_snapshot(store: &Store) -> Result<Reply> {
    let idle = HashSet::new();
    let projects = wisp_app::projects::list_projects(store, &idle, &idle).await?;
    Ok(Reply::Projects {
        projects,
        privacy: store.project_privacy().await?,
        activity_source: ActivitySource::PersistedOnly,
    })
}

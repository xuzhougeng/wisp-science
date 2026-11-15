//! MCP servers the user added: where they are, the connections to them, and
//! the tools they give a turn. A server that cannot be reached costs a turn
//! nothing but its tools; why is kept for Settings to show.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc, time::Duration};
use wisp_mcp::{McpClient, McpTool};
use wisp_store::Store;

/// The saved servers: a JSON array of [`McpServer`]. Tokens are not in it.
const SERVERS: &str = "desktop.mcp_servers";
/// How long a server gets to start and say what it offers.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Function names are held to 64 characters by the strictest providers.
const NAME_LIMIT: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpTransport {
    /// A program started on this computer and spoken to over its input and
    /// output.
    Stdio { command: String, args: Vec<String> },
    /// A server reached over HTTP.
    Http { url: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServer {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub transport: McpTransport,
    /// The environment variables a program is started with, by name; their
    /// values are in the system keychain.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    /// The headers sent to an HTTP server, by name; their values are in the
    /// system keychain.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<String>,
    /// This HTTP server's own route: empty follows the Network page's
    /// connector route, `none` connects directly, else a proxy's address.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub proxy: String,
}

/// What a connection is made with besides where the server is: the bearer
/// token, and the environment variables and headers with their values. A
/// change means a new connection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Credentials {
    pub token: Option<String>,
    pub env: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
}

/// A name a program's environment can have: letters, digits and `_`, not
/// starting with a digit.
pub(crate) fn valid_env_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A name an HTTP header can have.
pub(crate) fn valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c))
}

/// A value that fits on one line and is not too long.
pub(crate) fn valid_value(value: &str) -> bool {
    value.len() <= 8_192 && !value.contains(['\r', '\n', '\0'])
}

impl McpServer {
    /// The part of a tool's name that says which server it is from.
    pub fn slug(&self) -> String {
        let slug: String = self
            .name
            .trim()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect();
        let slug = slug.trim_matches('_').to_owned();
        if slug.is_empty() {
            "server".into()
        } else {
            slug.chars().take(24).collect()
        }
    }

    /// The name the model calls one of this server's tools by: letters,
    /// digits, `_` and `-` only, and never longer than providers allow.
    pub fn tool_name(&self, tool: &str) -> String {
        let tool: String = tool
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        // The approval card and the usage ranking add "mcp:" themselves.
        format!("{}__{tool}", self.slug())
            .chars()
            .take(NAME_LIMIT)
            .collect()
    }

    /// Refuses a server that could not work as written.
    pub fn validate(&self) -> Result<()> {
        let name = self.name.trim();
        ensure!(!name.is_empty(), "Give the server a name");
        ensure!(
            name.chars().count() <= 60,
            "Server names are limited to 60 characters"
        );
        match &self.transport {
            McpTransport::Stdio { command, .. } => {
                ensure!(
                    !command.trim().is_empty(),
                    "Enter the command that starts it"
                )
            }
            McpTransport::Http { url } => {
                let url = url.trim();
                ensure!(
                    url.starts_with("https://") || url.starts_with("http://"),
                    "Enter the server's URL, starting with https:// or http://"
                );
            }
        }
        Ok(())
    }
}

pub(crate) async fn load(store: &Store) -> Vec<McpServer> {
    store
        .get_setting(SERVERS)
        .await
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

pub(crate) async fn save(store: &Store, servers: &[McpServer]) -> Result<()> {
    store
        .set_setting(SERVERS, &serde_json::to_string(servers)?)
        .await
}

struct Open {
    /// What the connection was made for; a change means a new one.
    server: McpServer,
    credentials: Credentials,
    client: Arc<McpClient>,
}

/// Connections to the user's servers, kept between turns so a program is
/// started once, not for every message.
#[derive(Default)]
pub struct Connections {
    open: tokio::sync::Mutex<HashMap<String, Open>>,
    /// Why a server gave no tools the last time it was asked.
    errors: std::sync::Mutex<HashMap<String, String>>,
    /// The route HTTP servers are reached over, as Settings names it.
    proxy: std::sync::RwLock<String>,
}

impl Connections {
    async fn connect(server: &McpServer, with: &Credentials, proxy: &str) -> Result<McpClient> {
        let connecting = async {
            match &server.transport {
                McpTransport::Stdio { command, args } => {
                    let mut program = tokio::process::Command::new(command.trim());
                    program.args(args).envs(with.env.iter().cloned());
                    McpClient::launch_with_command(program)
                        .await
                        .with_context(|| format!("Could not start {}", command.trim()))
                }
                McpTransport::Http { url } => {
                    // A header of its own that says who you are wins over
                    // the token.
                    let named = with
                        .headers
                        .iter()
                        .any(|(name, _)| name.eq_ignore_ascii_case("authorization"));
                    let mut headers: Vec<_> = with
                        .token
                        .as_ref()
                        .filter(|_| !named)
                        .map(|token| ("Authorization".to_owned(), format!("Bearer {token}")))
                        .into_iter()
                        .collect();
                    headers.extend(with.headers.iter().cloned());
                    McpClient::connect_http_with_proxy(url.trim(), &headers, proxy)
                        .await
                        .with_context(|| format!("Could not reach {}", url.trim()))
                }
            }
        };
        match tokio::time::timeout(CONNECT_TIMEOUT, connecting).await {
            Ok(client) => client,
            Err(_) => bail!("The server did not answer in time"),
        }
    }

    /// The connection to `server`, made now if there is none or if the
    /// server was changed since.
    pub(crate) async fn client(
        &self,
        server: &McpServer,
        credentials: Credentials,
    ) -> Result<Arc<McpClient>> {
        let mut open = self.open.lock().await;
        if let Some(existing) = open.get(&server.id) {
            if &existing.server == server
                && existing.credentials == credentials
                && existing.client.is_connected()
            {
                return Ok(existing.client.clone());
            }
        }
        if let Some(stale) = open.remove(&server.id) {
            let _ = stale.client.shutdown().await;
        }
        // A server's own route wins over the Network page's.
        let proxy = match server.proxy.trim() {
            "" => self.proxy.read().unwrap().clone(),
            own => own.to_owned(),
        };
        let client = Arc::new(Self::connect(server, &credentials, &proxy).await?);
        open.insert(
            server.id.clone(),
            Open {
                server: server.clone(),
                credentials,
                client: client.clone(),
            },
        );
        Ok(client)
    }

    /// Names the route for connections made from now on. Connections that
    /// are open keep theirs until they are closed.
    pub(crate) fn set_proxy(&self, proxy: &str) {
        *self.proxy.write().unwrap() = proxy.trim().to_owned();
    }

    pub(crate) async fn close(&self, id: &str) {
        let closed = self.open.lock().await.remove(id);
        if let Some(closed) = closed {
            let _ = closed.client.shutdown().await;
        }
        self.errors.lock().unwrap().remove(id);
    }

    pub(crate) async fn close_all(&self) {
        let all: Vec<_> = self.open.lock().await.drain().collect();
        for (_, open) in all {
            let _ = open.client.shutdown().await;
        }
    }

    pub(crate) fn error(&self, id: &str) -> Option<String> {
        self.errors.lock().unwrap().get(id).cloned()
    }

    /// The names of what `server` offers, connecting if need be.
    pub(crate) async fn list(
        &self,
        server: &McpServer,
        credentials: Credentials,
    ) -> Result<Vec<wisp_mcp::RemoteTool>> {
        let listed = async {
            let client = self.client(server, credentials).await?;
            let tools = tokio::time::timeout(CONNECT_TIMEOUT, client.tools_list())
                .await
                .map_err(|_| anyhow::anyhow!("The server did not list its tools in time"))??;
            Ok::<_, anyhow::Error>((client, tools))
        }
        .await;
        let mut errors = self.errors.lock().unwrap();
        match listed {
            Ok((_, tools)) => {
                errors.remove(&server.id);
                Ok(tools)
            }
            Err(error) => {
                errors.insert(server.id.clone(), format!("{error:#}"));
                Err(error)
            }
        }
    }

    /// The tools the enabled servers give a turn. Each asks before it runs:
    /// what a server the user added does is not known to Wisp. A server
    /// that fails is skipped.
    pub(crate) async fn tools(
        &self,
        servers: &[McpServer],
        credentials: impl Fn(&str) -> Credentials,
    ) -> Vec<Box<dyn wisp_tools::Tool>> {
        let mut tools: Vec<Box<dyn wisp_tools::Tool>> = Vec::new();
        for server in servers.iter().filter(|server| server.enabled) {
            let secret = credentials(&server.id);
            let Ok(listed) = self.list(server, secret.clone()).await else {
                continue;
            };
            let Ok(client) = self.client(server, secret).await else {
                continue;
            };
            let catalog = Arc::new(listed.clone());
            for tool in listed {
                if !tool.visible_to_model() {
                    continue;
                }
                let name = server.tool_name(&tool.name);
                tools.push(Box::new(
                    McpTool::with_catalog_requiring_approval(
                        tool,
                        client.clone(),
                        server.id.clone(),
                        catalog.clone(),
                    )
                    .renamed(name),
                ));
            }
        }
        tools
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str, transport: McpTransport) -> McpServer {
        McpServer {
            id: "s".into(),
            name: name.into(),
            enabled: true,
            transport,
            env: vec![],
            headers: vec![],
            proxy: String::new(),
        }
    }

    #[test]
    fn names_and_values_are_ones_a_program_and_a_server_take() {
        assert!(valid_env_name("LAB_API_TOKEN") && valid_env_name("_x1"));
        for wrong in ["", "1ST", "LAB-TOKEN", "A B"] {
            assert!(!valid_env_name(wrong), "{wrong}");
        }
        assert!(valid_header_name("X-Api-Key") && valid_header_name("Authorization"));
        assert!(!valid_header_name("X Api") && !valid_header_name("X:Y"));
        assert!(valid_value("abc def") && !valid_value("line\nbreak"));
    }

    fn http(url: &str) -> McpTransport {
        McpTransport::Http { url: url.into() }
    }

    #[test]
    fn tool_names_are_safe_for_every_provider() {
        let lab = server("Lab Notebook (v2)", http("https://x"));
        assert_eq!(lab.slug(), "lab_notebook__v2");
        assert_eq!(
            lab.tool_name("search.entries"),
            "lab_notebook__v2__search_entries"
        );
        // Never longer than the strictest provider allows.
        let long = lab.tool_name(&"x".repeat(100));
        assert_eq!(long.chars().count(), NAME_LIMIT);
        assert!(long.starts_with("lab_notebook__v2__"));
        // A name with nothing usable in it still names a server.
        assert_eq!(server("实验室", http("https://x")).slug(), "server");
        assert_eq!(server(&"a".repeat(50), http("https://x")).slug().len(), 24);
    }

    #[test]
    fn a_server_must_be_named_and_reachable_as_written() {
        assert!(server("Lab", http("https://mcp.example/"))
            .validate()
            .is_ok());
        assert!(server("Lab", http("http://127.0.0.1:8000/mcp"))
            .validate()
            .is_ok());
        let stdio = |command: &str| McpTransport::Stdio {
            command: command.into(),
            args: vec![],
        };
        assert!(server("Lab", stdio("uvx")).validate().is_ok());
        for wrong in [
            server(" ", http("https://x")),
            server(&"n".repeat(61), http("https://x")),
            server("Lab", http("mcp.example")),
            server("Lab", http("ftp://x")),
            server("Lab", stdio("  ")),
        ] {
            assert!(wrong.validate().is_err(), "{wrong:?}");
        }
    }

    #[test]
    fn the_saved_form_does_not_change_shape() {
        let saved = serde_json::to_value(server(
            "Lab",
            McpTransport::Stdio {
                command: "uvx".into(),
                args: vec!["lab-mcp".into()],
            },
        ))
        .unwrap();
        assert_eq!(
            saved,
            serde_json::json!({
                "id": "s", "name": "Lab", "enabled": true,
                "transport": {"kind": "stdio", "command": "uvx", "args": ["lab-mcp"]}
            })
        );
        let read: McpServer = serde_json::from_value(serde_json::json!({
            "id": "h", "name": "Web", "enabled": false,
            "transport": {"kind": "http", "url": "https://mcp.example/"}
        }))
        .unwrap();
        assert_eq!(read.transport, http("https://mcp.example/"));
    }
}

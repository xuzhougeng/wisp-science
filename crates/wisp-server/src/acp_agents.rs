//! ACP agents: programs on this computer that speak the Agent Client
//! Protocol (a coding agent with an ACP adapter, for instance) and can be
//! started from here.
//!
//! The list is the WebView desktop's own setting (`acp_agent_profiles`) in
//! the same shape, so an agent added in one desktop is there in the other.
//! Nothing here starts a program except Test, which starts the agent,
//! reads what it says about itself and stops it.

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};
use wisp_acp::{AcpAgentProfile, AcpAuthMethodKind, AcpSessionHandle};
use wisp_store::Store;

const PROFILES_KEY: &str = "acp_agent_profiles";
const LABEL_LIMIT: usize = 80;
/// How long Test waits for the agent to start and introduce itself.
const TEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long an agent may take to sign in by itself, the user included.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

/// One agent as it is kept: a name, the program and its arguments.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpAgent {
    #[serde(default)]
    pub id: String,
    pub label: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// An agent in the list, and whether its program can be found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcpAgentRow {
    pub agent: AcpAgent,
    pub found: bool,
}

/// How an agent wants to be signed in to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcpSignInKind {
    /// It signs in by itself when asked, often in the browser.
    Agent,
    /// It runs a sign-in program of its own in a terminal.
    Terminal,
    /// It reads a key from an environment variable you set.
    Environment,
}

/// One way an agent offers to be signed in to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcpSignIn {
    pub id: String,
    pub name: String,
    pub kind: AcpSignInKind,
}

/// What an agent said about itself when it was tested.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcpAgentReport {
    pub protocol: u16,
    /// Its name and version, when it gave them.
    pub name: String,
    pub version: String,
    /// The ways it offers to be signed in to.
    pub sign_in: Vec<AcpSignIn>,
}

/// How signing in went: done, or to be finished in a terminal running
/// the agent's own sign-in, started in `folder`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcpSignInOutcome {
    Done,
    Terminal {
        program: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
    },
}

/// Whether `command` names a program that exists: a path to a file, or a
/// name found on `PATH`.
pub(crate) fn found(command: &str) -> bool {
    let command = command.trim();
    !command.is_empty() && which::which(command).is_ok()
}

pub(crate) async fn list(store: &Store) -> Vec<AcpAgent> {
    store
        .get_setting(PROFILES_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub(crate) async fn rows(store: &Store) -> Vec<AcpAgentRow> {
    list(store)
        .await
        .into_iter()
        .map(|agent| AcpAgentRow {
            found: found(&agent.command),
            agent,
        })
        .collect()
}

async fn keep(store: &Store, agents: &[AcpAgent]) -> Result<()> {
    store
        .set_setting(PROFILES_KEY, &serde_json::to_string(agents)?)
        .await
}

/// Adds `agent`, or changes the one with its id. Returns its id.
pub(crate) async fn save(store: &Store, mut agent: AcpAgent) -> Result<String> {
    agent.label = agent.label.trim().to_owned();
    agent.command = agent.command.trim().to_owned();
    agent.args.retain(|arg| !arg.trim().is_empty());
    ensure!(!agent.label.is_empty(), "Give the agent a name");
    ensure!(
        agent.label.chars().count() <= LABEL_LIMIT,
        "A name is at most {LABEL_LIMIT} characters"
    );
    ensure!(
        !agent.command.is_empty(),
        "Say which program starts the agent"
    );
    ensure!(
        !agent
            .command
            .chars()
            .chain(agent.args.iter().flat_map(|arg| arg.chars()))
            .any(char::is_control),
        "The program and its arguments are one line each"
    );
    let mut agents = list(store).await;
    if agent.id.trim().is_empty() {
        agent.id = uuid::Uuid::new_v4().to_string();
    } else if !agents.iter().any(|kept| kept.id == agent.id) {
        bail!("That agent is gone");
    }
    let id = agent.id.clone();
    match agents.iter_mut().find(|kept| kept.id == agent.id) {
        Some(kept) => *kept = agent,
        None => agents.push(agent),
    }
    keep(store, &agents).await?;
    Ok(id)
}

pub(crate) async fn remove(store: &Store, id: &str) -> Result<()> {
    let mut agents = list(store).await;
    let before = agents.len();
    agents.retain(|agent| agent.id != id);
    ensure!(agents.len() < before, "That agent is gone");
    keep(store, &agents).await
}

pub(crate) fn profile(agent: &AcpAgent) -> AcpAgentProfile {
    AcpAgentProfile::new(
        agent.id.clone(),
        agent.label.clone(),
        PathBuf::from(&agent.command),
        agent.args.clone(),
    )
}

/// Starts the agent, reads how it introduces itself, and stops it.
pub(crate) async fn test(store: &Store, id: &str) -> Result<AcpAgentReport> {
    let agent = list(store)
        .await
        .into_iter()
        .find(|agent| agent.id == id)
        .context("That agent is gone")?;
    ensure!(
        found(&agent.command),
        "The agent's program was not found on this computer"
    );
    let handle = tokio::time::timeout(TEST_TIMEOUT, AcpSessionHandle::launch(profile(&agent)))
        .await
        .context("The agent did not answer within 30 seconds")?
        .map_err(|error| anyhow::anyhow!("The agent could not be started: {error}"))?;
    let info = handle.info().clone();
    handle.shutdown(Duration::from_secs(2)).await;
    let (name, version) = info
        .implementation
        .map(|found| (found.title.unwrap_or(found.name), found.version))
        .unwrap_or_default();
    Ok(AcpAgentReport {
        protocol: info.protocol_version,
        name,
        version,
        sign_in: info
            .auth_methods
            .into_iter()
            .map(|method| AcpSignIn {
                kind: match method.kind {
                    AcpAuthMethodKind::Agent => AcpSignInKind::Agent,
                    AcpAuthMethodKind::Terminal { .. } => AcpSignInKind::Terminal,
                    AcpAuthMethodKind::Environment => AcpSignInKind::Environment,
                },
                id: method.id,
                name: method.name,
            })
            .collect(),
    })
}

/// Signs in to an agent with one of the ways it offers: asks it to, or
/// says which command to run in a terminal (the agent's program with the
/// arguments and environment it named).
pub(crate) async fn sign_in(store: &Store, id: &str, method: &str) -> Result<AcpSignInOutcome> {
    let agent = list(store)
        .await
        .into_iter()
        .find(|agent| agent.id == id)
        .context("That agent is gone")?;
    ensure!(
        found(&agent.command),
        "The agent's program was not found on this computer"
    );
    let handle = tokio::time::timeout(TEST_TIMEOUT, AcpSessionHandle::launch(profile(&agent)))
        .await
        .context("The agent did not answer within 30 seconds")?
        .map_err(|error| anyhow::anyhow!("The agent could not be started: {error}"))?;
    let chosen = handle
        .info()
        .auth_methods
        .iter()
        .find(|offered| offered.id == method)
        .cloned();
    let outcome = match chosen.map(|chosen| chosen.kind) {
        None => Err(anyhow::anyhow!(
            "The agent no longer offers this way to sign in"
        )),
        Some(AcpAuthMethodKind::Agent) => {
            // The agent may open the browser and wait for the user.
            match tokio::time::timeout(SIGN_IN_TIMEOUT, handle.authenticate(method)).await {
                Ok(Ok(())) => Ok(AcpSignInOutcome::Done),
                Ok(Err(error)) => Err(anyhow::anyhow!("Signing in failed: {error}")),
                Err(_) => Err(anyhow::anyhow!(
                    "Signing in did not finish within five minutes"
                )),
            }
        }
        Some(AcpAuthMethodKind::Terminal { args, env }) => Ok(AcpSignInOutcome::Terminal {
            program: agent.command.clone(),
            args: agent.args.iter().cloned().chain(args).collect(),
            env: env.into_iter().collect(),
        }),
        Some(AcpAuthMethodKind::Environment) => Err(anyhow::anyhow!(
            "This agent reads its key from an environment variable; set it where the agent's \
             documentation says, then test again"
        )),
    };
    handle.shutdown(Duration::from_secs(2)).await;
    outcome
}

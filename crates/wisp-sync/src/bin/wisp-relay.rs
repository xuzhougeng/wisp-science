use anyhow::{Context, Result};
use wisp_sync::{relay_router, FileRelay, Lab, RelayHttpState};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "wisp_sync=info,wisp_relay=info".into()),
        )
        .init();
    let root = std::env::var_os("WISP_RELAY_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("./wisp-relay-data"));
    let token = std::env::var("WISP_RELAY_TOKEN")
        .context("WISP_RELAY_TOKEN must be set to a strong random bearer token")?;
    let bind = std::env::var("WISP_RELAY_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into());
    let relay = FileRelay::open(&root).await?;
    let mut state = RelayHttpState::new(relay, token)?;
    // Off unless asked for: without it the computers sharing this relay know
    // nothing of each other (docs/wisp-lab.md).
    let lab = std::env::var("WISP_LAB").is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "on" | "yes"
        )
    });
    if lab {
        state = state.with_lab(Lab::open(root.join("lab")).await?);
        tracing::info!("Wisp Lab enabled");
    }
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "Wisp relay listening");
    axum::serve(listener, relay_router(state)).await?;
    Ok(())
}

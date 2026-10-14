use anyhow::{bail, Result};
use std::sync::Arc;
use wisp_server::{config::Config, Server};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [arg] if arg == "--generate-code" => {
            println!("{}", wisp_sync::RemoteCode::generate()?.display());
            return Ok(());
        }
        [arg] if matches!(arg.as_str(), "--help" | "-h") => {
            println!("wisp-server [--generate-code | --connection-url]\n\nRequired: WISP_SERVER_PUBLIC_URL, WISP_SERVER_CODE, WISP_MODEL, WISP_API_KEY\nOptional: WISP_PROVIDER, WISP_API_URL, WISP_SERVER_DATA, WISP_SERVER_WORKSPACE, WISP_SERVER_BIND\nSee docs/linux-server.md for deployment and HTTPS setup.");
            return Ok(());
        }
        [] => {}
        [arg] if arg == "--connection-url" => {
            let config = Config::from_env()?;
            println!("{}remote#{}", config.public_url, config.code.display());
            return Ok(());
        }
        _ => bail!("Unknown arguments; use wisp-server --help"),
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "wisp_server=info".into()),
        )
        .init();
    let config = Config::from_env()?;
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    let code = config.code.clone();
    tracing::info!(bind=%config.bind, url=%format!("{}remote", config.public_url), "Wisp Server starting; connection code is supplied through WISP_SERVER_CODE");
    let server = Server::open(config).await?;
    let app = wisp_sync::direct_remote_router(code, Arc::new(server.clone()));
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            server.shutdown().await;
        })
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}

//! Manual, credential-free check of subscription authentication transport.
//! Run with `cargo run -p wisp-llm --example auth_network_probe -- [proxy|none]`.
#[tokio::main]
async fn main() {
    let proxy = std::env::args().nth(1);
    let client = wisp_llm::codex_auth::http_client(proxy.as_deref());
    match wisp_llm::xai_auth::discover_token_endpoint(&client).await {
        Ok(endpoint) => println!("xAI discovery OK: {endpoint}"),
        Err(error) => println!("{error}"),
    }
    // An intentionally invalid code should reach OAuth validation (401/400),
    // never produce credentials. No saved account or keyring is accessed.
    match wisp_llm::codex_auth::exchange_authorization_code(
        &client,
        "wisp-network-diagnostic-invalid-code",
        "invalid",
        wisp_llm::codex_auth::REDIRECT_URI,
    )
    .await
    {
        Ok(_) => println!("Unexpected acceptance of invalid diagnostic code"),
        Err(error) => println!("OpenAI invalid-code probe (OAuth 400/401 expected): {error}"),
    }
}

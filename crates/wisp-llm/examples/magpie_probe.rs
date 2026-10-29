//! Opt-in smoke probe: only use a temporary Magpie fixture configuration.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::var("MAGPIE_TEST_CONFIG")?;
    let connection = wisp_llm::magpie::load_from_path(std::path::Path::new(&path))
        .ok_or("inactive Magpie fixture")?;
    let mut cfg = connection.config;
    // Stands in for the application's session effort setting.
    if let Ok(requested) = std::env::var("MAGPIE_TEST_REASONING_EFFORT") {
        cfg.reasoning_effort =
            wisp_llm::magpie::resolve_effort(&connection.info.efforts, &requested);
    }
    let result = wisp_llm::build(cfg)
        .complete(&[wisp_llm::Message::user("fixture")], &[])
        .await?;
    if result.content != "fixture-ok" {
        return Err("unexpected fixture response".into());
    }
    println!("fixture-ok");
    Ok(())
}

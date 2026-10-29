//! Opt-in smoke probe: only use a temporary Magpie fixture configuration.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::var("MAGPIE_TEST_CONFIG")?;
    let cfg = wisp_llm::magpie::load_from_path(std::path::Path::new(&path))
        .ok_or("inactive Magpie fixture")?;
    let result = wisp_llm::build(cfg)
        .complete(&[wisp_llm::Message::user("fixture")], &[])
        .await?;
    if result.content != "fixture-ok" {
        return Err("unexpected fixture response".into());
    }
    println!("fixture-ok");
    Ok(())
}

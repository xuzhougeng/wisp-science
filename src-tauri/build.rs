//! Build script: purge stale bundled seed copies before Tauri re-copies
//! resources, and bake the models.dev model catalog into the binary.
//!
//! Tauri's map-form `resources` merge into `target/{profile}/seed` and never
//! delete removed files, so renamed demos (and old CRISPR/enzyme seeds) linger
//! beside the binary and show up as extra Example-project sessions.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[path = "src/model_catalog_shared.rs"]
#[allow(dead_code)] // build.rs only uses distill(); the runtime uses the rest
mod model_catalog_shared;

const MODELS_DEV_API: &str = "https://models.dev/api.json";

fn main() {
    purge_stale_seed_dirs();
    bake_model_catalog();
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let mut attributes = tauri_build::Attributes::new();
    if msvc {
        // The linker below owns the manifest for all executable targets.
        // A second manifest in tauri-build's app-only .res causes CVT1100.
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
    }
    tauri_build::try_build(attributes).expect("build Tauri resources");
    // Unit-test and example executables also import Common Controls v6
    // (TaskDialogIndirect). They must start without post-build binary patches.
    if msvc {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("examples/webview_recovery_smoke.manifest");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
}

/// Distill models.dev into `$OUT_DIR/model_catalog.json`, falling back to the
/// committed snapshot when offline so builds and tests never require network.
fn bake_model_catalog() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let dest = out_dir.join("model_catalog.json");
    println!("cargo:rerun-if-changed=model_catalog.snapshot.json");
    println!("cargo:rerun-if-env-changed=WISP_CATALOG_OFFLINE");
    if std::env::var_os("WISP_CATALOG_OFFLINE").is_none() {
        match fetch_distilled_catalog() {
            Ok(json) => {
                fs::write(&dest, json).expect("write distilled model catalog");
                return;
            }
            Err(err) => {
                println!(
                    "cargo:warning=models.dev fetch failed ({}); using bundled snapshot",
                    error_chain(&*err)
                );
            }
        }
    }
    let snapshot = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("model_catalog.snapshot.json");
    fs::copy(&snapshot, &dest).unwrap_or_else(|e| {
        panic!(
            "model catalog unavailable: no network and {} ({e}); \
             run scripts/refresh_model_catalog.sh once with network",
            snapshot.display()
        )
    });
}

fn error_chain(err: &(dyn std::error::Error + 'static)) -> String {
    let mut out = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        out.push_str(&format!(" <- {cause}"));
        source = cause.source();
    }
    out
}

fn fetch_distilled_catalog() -> Result<String, Box<dyn std::error::Error>> {
    let mut builder = reqwest::blocking::Client::builder().timeout(Duration::from_secs(20));
    // curl-style env proxy, lowercase first: some sandboxes export a dead
    // HTTPS_PROXY next to a working https_proxy; reqwest's system-proxy picks
    // the wrong one, so resolve explicitly.
    let proxy = ["https_proxy", "HTTPS_PROXY", "all_proxy", "ALL_PROXY"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|v| !v.trim().is_empty()));
    if let Some(proxy) = proxy {
        builder = builder.proxy(reqwest::Proxy::https(proxy)?);
    }
    let body = builder
        .build()?
        .get(MODELS_DEV_API)
        .send()?
        .error_for_status()?
        .text()?;
    Ok(model_catalog_shared::distill(&body)?)
}

fn purge_stale_seed_dirs() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = manifest_dir.join("..").join("target");
    if !target.is_dir() {
        return;
    }
    for profile in ["debug", "release"] {
        for rel in ["seed", "_up_/seed"] {
            let dir = target.join(profile).join(rel);
            remove_dir_if_present(&dir);
        }
    }
    println!("cargo:rerun-if-changed=../seed");
}

fn remove_dir_if_present(dir: &Path) {
    if dir.is_dir() {
        let _ = fs::remove_dir_all(dir);
    }
}

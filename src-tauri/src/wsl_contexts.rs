use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WslDistro {
    pub name: String,
    pub is_default: bool,
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn parse_wsl_distro_list(output: &[u8]) -> Vec<WslDistro> {
    let text = decode_wsl_output(output);
    let mut out: Vec<WslDistro> = Vec::new();
    for raw in text.lines() {
        let mut line = raw.trim_matches('\0').trim();
        line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.contains("Windows Subsystem for Linux") {
            continue;
        }

        let mut is_default = false;
        if let Some(stripped) = line.strip_prefix('*') {
            is_default = true;
            line = stripped.trim();
        }
        if let Some(stripped) = line.strip_suffix("(Default)") {
            is_default = true;
            line = stripped.trim();
        }
        if line.is_empty() {
            continue;
        }

        if let Some(existing) = out.iter_mut().find(|d| d.name == line) {
            existing.is_default |= is_default;
            continue;
        }
        out.push(WslDistro {
            name: line.into(),
            is_default,
        });
    }
    out
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn decode_wsl_output(output: &[u8]) -> String {
    if output.starts_with(&[0xff, 0xfe]) || looks_like_utf16le(output) {
        let units: Vec<u16> = output
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(output).replace('\0', "")
    }
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn looks_like_utf16le(output: &[u8]) -> bool {
    if output.len() < 4 {
        return false;
    }
    let null_odd_bytes = output
        .iter()
        .skip(1)
        .step_by(2)
        .filter(|b| **b == 0)
        .count();
    null_odd_bytes >= output.len() / 4
}

pub async fn persist_wsl_contexts(
    store: &wisp_store::Store,
    distros: &[WslDistro],
) -> Result<Vec<wisp_store::ExecutionContext>, String> {
    let mut contexts = Vec::new();
    for distro in distros {
        let name = distro.name.trim();
        let id = format!("wsl:{name}");
        let now = chrono::Utc::now().timestamp();
        let mut ctx = match store
            .get_execution_context(&id)
            .await
            .map_err(|e| e.to_string())?
        {
            Some(ctx) => ctx,
            None => wisp_store::ExecutionContext::new(&id, name).map_err(|e| e.to_string())?,
        };
        ctx.kind = wisp_store::ExecutionContextKind::Wsl;
        ctx.label = name.into();
        ctx.config_json = serde_json::json!({
            "distro": name,
            "is_default": distro.is_default,
        })
        .to_string();
        ctx.updated_at = now;
        store
            .upsert_execution_context(&ctx)
            .await
            .map_err(|e| e.to_string())?;
        contexts.push(ctx);
    }
    Ok(contexts)
}

#[tauri::command]
pub async fn list_wsl_distros() -> Result<Vec<WslDistro>, String> {
    #[cfg(target_os = "windows")]
    {
        let mut command = std::process::Command::new("wsl.exe");
        command.args(["-l", "-q"]);
        wisp_tools::process::hide_console(&mut command);
        let output = command
            .output()
            .map_err(|e| format!("failed to run wsl.exe: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("wsl.exe -l -q failed: {stderr}"));
        }
        Ok(parse_wsl_distro_list(&output.stdout))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(Vec::new())
    }
}

#[tauri::command]
pub async fn import_wsl_contexts(
    state: State<'_, crate::AppState>,
) -> Result<Vec<wisp_store::ExecutionContext>, String> {
    let distros = list_wsl_distros().await?;
    persist_wsl_contexts(&state.store, &distros).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_utf8_crlf_default_markers_and_dedupes() {
        let out = b"Ubuntu-22.04\r\n\r\n* Debian (Default)\r\nUbuntu-22.04\r\n";
        assert_eq!(
            parse_wsl_distro_list(out),
            vec![
                WslDistro {
                    name: "Ubuntu-22.04".into(),
                    is_default: false,
                },
                WslDistro {
                    name: "Debian".into(),
                    is_default: true,
                },
            ]
        );
    }

    #[test]
    fn parses_utf16le_with_nulls() {
        let text = "\u{feff}Ubuntu\r\nDebian (Default)\r\n";
        let mut bytes = Vec::new();
        for unit in text.encode_utf16() {
            bytes.extend(unit.to_le_bytes());
        }
        assert_eq!(
            parse_wsl_distro_list(&bytes),
            vec![
                WslDistro {
                    name: "Ubuntu".into(),
                    is_default: false,
                },
                WslDistro {
                    name: "Debian".into(),
                    is_default: true,
                },
            ]
        );
    }

    #[tokio::test]
    async fn persisting_wsl_distros_creates_execution_contexts() {
        let tmp =
            std::env::temp_dir().join(format!("wisp_wsl_contexts_{}.sqlite", uuid::Uuid::new_v4()));
        let store = wisp_store::Store::open(&tmp).await.unwrap();
        let contexts = persist_wsl_contexts(
            &store,
            &[WslDistro {
                name: "Ubuntu-22.04".into(),
                is_default: true,
            }],
        )
        .await
        .unwrap();

        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].id, "wsl:Ubuntu-22.04");
        assert_eq!(contexts[0].kind, wisp_store::ExecutionContextKind::Wsl);
        let cfg: serde_json::Value = serde_json::from_str(&contexts[0].config_json).unwrap();
        assert_eq!(cfg["distro"], "Ubuntu-22.04");
        assert_eq!(cfg["is_default"], true);
        assert!(store
            .get_execution_context("wsl:Ubuntu-22.04")
            .await
            .unwrap()
            .is_some());

        let _ = std::fs::remove_file(&tmp);
    }

    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn list_wsl_distros_is_empty_on_non_windows() {
        assert!(list_wsl_distros().await.unwrap().is_empty());
    }
}

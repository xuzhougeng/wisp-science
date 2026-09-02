//! OS keyring-backed secret storage for API keys.
//!
//! In **debug** builds we persist new secrets to a plaintext JSON file in the
//! user's home dir. macOS binds each keychain item to the calling app's code
//! signature, which `tauri dev` regenerates on every rebuild — so the real
//! keyring pops the login-keychain password prompt on every dev run. Dev keys
//! aren't worth that friction. Release builds use the OS keyring unchanged.
//!
//! On Windows, a debug read still falls back to that same OS keyring when the
//! file has no entry, so `cargo tauri dev` can reuse keys already saved in an
//! installed Wisp. macOS stays file-only to avoid the prompt storm.

/// A named secret (e.g. an API key) stored in the OS credential manager.
pub struct Secret;

const SERVICE: &str = "wisp";

impl Secret {
    pub fn set(name: &str, value: &str) -> anyhow::Result<()> {
        backend::set(name, value)
    }

    pub fn get(name: &str) -> anyhow::Result<String> {
        backend::get(name)
    }

    pub fn delete(name: &str) -> anyhow::Result<()> {
        backend::delete(name)
    }
}

#[cfg(not(debug_assertions))]
mod backend {
    use keyring::Entry;

    pub fn set(name: &str, value: &str) -> anyhow::Result<()> {
        Entry::new(super::SERVICE, name)?.set_password(value)?;
        Ok(())
    }

    pub fn get(name: &str) -> anyhow::Result<String> {
        Ok(Entry::new(super::SERVICE, name)?.get_password()?)
    }

    pub fn delete(name: &str) -> anyhow::Result<()> {
        Entry::new(super::SERVICE, name)?.delete_credential()?;
        Ok(())
    }
}

#[cfg(debug_assertions)]
mod backend {
    // Dev-only plaintext file. Serialize load+store so parallel `cargo test`
    // workers cannot clobber each other's whole-file rewrites.
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};

    fn file() -> PathBuf {
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(".wisp-science-dev-secrets.json")
    }

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn load() -> BTreeMap<String, String> {
        std::fs::read(file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn store(map: &BTreeMap<String, String>) -> anyhow::Result<()> {
        std::fs::write(file(), serde_json::to_vec_pretty(map)?)?;
        Ok(())
    }

    pub fn set(name: &str, value: &str) -> anyhow::Result<()> {
        let _guard = lock();
        let mut map = load();
        map.insert(name.to_string(), value.to_string());
        store(&map)
    }

    pub fn get(name: &str) -> anyhow::Result<String> {
        let file_value = {
            let _guard = lock();
            load().remove(name)
        };
        if let Some(value) = file_value.filter(|value| !value.is_empty()) {
            return Ok(value);
        }
        os_keyring_fallback(name).ok_or_else(|| anyhow::anyhow!("no secret named {name}"))
    }

    /// Reuse keys saved by a release install. Windows only: unsigned `tauri
    /// dev` on macOS prompts for the login keychain on every miss, and Linux
    /// CI should not touch the session Secret Service.
    fn os_keyring_fallback(name: &str) -> Option<String> {
        #[cfg(windows)]
        {
            keyring::Entry::new(super::SERVICE, name)
                .ok()?
                .get_password()
                .ok()
                .filter(|value| !value.is_empty())
        }
        #[cfg(not(windows))]
        {
            let _ = name;
            None
        }
    }

    pub fn delete(name: &str) -> anyhow::Result<()> {
        let _guard = lock();
        let mut map = load();
        map.remove(name);
        store(&map)
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::Secret;

    // Exercises only the debug file backend (cargo test builds with
    // debug_assertions), so no OS keyring daemon is ever required. The entry
    // name is UUID-scoped so parallel test runs sharing $HOME never collide.
    #[test]
    fn set_get_delete_roundtrip() {
        let name = format!("test:roundtrip:{}", uuid::Uuid::new_v4());
        Secret::set(&name, "abc123").unwrap();
        assert_eq!(Secret::get(&name).unwrap(), "abc123");
        Secret::delete(&name).unwrap();
        assert!(Secret::get(&name).is_err());
    }
}

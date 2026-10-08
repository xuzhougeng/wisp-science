//! OS keyring-backed secret storage for API keys.
//!
//! In **debug** builds we bypass the OS keyring and persist to a plaintext JSON
//! file in the user's home dir. macOS binds each keychain item to the calling
//! app's code signature, which `tauri dev` regenerates on every rebuild — so the
//! real keyring pops the login-keychain password prompt on every dev run. Dev
//! keys aren't worth that friction. Release builds use the OS keyring; Windows
//! splits large credentials into bounded entries and publishes their manifest last.

#[cfg(any(all(target_os = "windows", not(debug_assertions)), test))]
mod windows;

#[cfg(all(not(debug_assertions), target_os = "windows"))]
use windows as backend;

/// A named secret (e.g. an API key) stored in the OS credential manager.
pub struct Secret;

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

#[cfg(all(not(debug_assertions), not(target_os = "windows")))]
mod backend {
    use keyring::Entry;

    const SERVICE: &str = "wisp";

    pub fn set(name: &str, value: &str) -> anyhow::Result<()> {
        Entry::new(SERVICE, name)?.set_password(value)?;
        Ok(())
    }

    pub fn get(name: &str) -> anyhow::Result<String> {
        Ok(Entry::new(SERVICE, name)?.get_password()?)
    }

    pub fn delete(name: &str) -> anyhow::Result<()> {
        match Entry::new(SERVICE, name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.into()),
        }
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
        let _guard = lock();
        load()
            .remove(name)
            .ok_or_else(|| anyhow::anyhow!("no secret named {name}"))
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

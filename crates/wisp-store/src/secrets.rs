//! OS keyring-backed secret storage for API keys.
//!
//! In **debug** builds we bypass the OS keyring and persist to a plaintext JSON
//! file in the user's home dir. macOS binds each keychain item to the calling
//! app's code signature, which `tauri dev` regenerates on every rebuild — so the
//! real keyring pops the login-keychain password prompt on every dev run. Dev
//! keys aren't worth that friction. Release builds use the OS keyring unchanged.

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

#[cfg(not(debug_assertions))]
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
        Entry::new(SERVICE, name)?.delete_credential()?;
        Ok(())
    }
}

#[cfg(debug_assertions)]
mod backend {
    // ponytail: plaintext file, whole-file rewrite, no locking. Dev-only, single
    // user — if concurrent writes ever matter, put a Mutex around load+store.
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn file() -> PathBuf {
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(".wisp-science-dev-secrets.json")
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
        let mut map = load();
        map.insert(name.to_string(), value.to_string());
        store(&map)
    }

    pub fn get(name: &str) -> anyhow::Result<String> {
        load()
            .remove(name)
            .ok_or_else(|| anyhow::anyhow!("no secret named {name}"))
    }

    pub fn delete(name: &str) -> anyhow::Result<()> {
        let mut map = load();
        map.remove(name);
        store(&map)
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::Secret;

    #[test]
    fn set_get_delete_roundtrip() {
        let name = "test:roundtrip";
        Secret::set(name, "abc123").unwrap();
        assert_eq!(Secret::get(name).unwrap(), "abc123");
        Secret::delete(name).unwrap();
        assert!(Secret::get(name).is_err());
    }
}

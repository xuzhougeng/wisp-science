//! Windows Credential Manager limits each blob to 2560 bytes, including the
//! UTF-16 expansion used by keyring's password API. Keep short/legacy passwords
//! compatible and store larger UTF-8 values in versioned keyring entries.
//! The manifest is the commit point: a failed write leaves the old value intact.

use anyhow::{anyhow, ensure, Result};
#[cfg(target_os = "windows")]
use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

const SERVICE: &str = "wisp";
const PART_SERVICE: &str = "wisp-secret-parts-v1";
const BLOB_LIMIT: usize = 2560;
const MAX_SECRET_BYTES: usize = 1024 * 1024;
// An unpaired UTF-16 high surrogate followed by a non-surrogate can never be
// the beginning of a legacy password written from a valid Rust string.
const MAGIC: &[u8] = b"\x00\xd8WISP-SECRET-PARTS\x01";

trait CredentialStore {
    fn read(&self, service: &str, name: &str) -> Result<Option<Vec<u8>>>;
    fn write(&self, service: &str, name: &str, value: &[u8]) -> Result<()>;
    fn remove(&self, service: &str, name: &str) -> Result<()>;
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    generation: Uuid,
    bytes: usize,
    sha256: String,
}

impl Manifest {
    fn parts(&self) -> usize {
        self.bytes.div_ceil(BLOB_LIMIT)
    }

    fn part_name(&self, name: &str, index: usize) -> String {
        format!(
            "{}:{}:{index}",
            hex::encode(Sha256::digest(name)),
            self.generation.simple()
        )
    }
}

fn manifest(blob: &[u8]) -> Result<Option<Manifest>> {
    let Some(json) = blob.strip_prefix(MAGIC) else {
        return Ok(None);
    };
    let parsed: Manifest =
        serde_json::from_slice(json).map_err(|_| anyhow!("Invalid Windows credential manifest"))?;
    ensure!(
        parsed.bytes > 0
            && parsed.bytes <= MAX_SECRET_BYTES
            && parsed.sha256.len() == 64
            && parsed.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid Windows credential manifest"
    );
    Ok(Some(parsed))
}

fn legacy_blob(value: &str) -> Zeroizing<Vec<u8>> {
    Zeroizing::new(value.encode_utf16().flat_map(u16::to_le_bytes).collect())
}

fn remove_parts(store: &impl CredentialStore, name: &str, manifest: &Manifest) -> Result<()> {
    let mut failure = None;
    for index in 0..manifest.parts() {
        if let Err(error) = store.remove(PART_SERVICE, &manifest.part_name(name, index)) {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

fn cleanup_parts(store: &impl CredentialStore, name: &str, manifest: &Manifest) {
    if remove_parts(store, name, manifest).is_err() {
        // Never log a credential, its fragments, or a backend encoding error.
        tracing::warn!("Could not remove unused Windows credential fragments");
    }
}

fn set_in(store: &impl CredentialStore, name: &str, value: &str) -> Result<()> {
    ensure!(
        value.len() <= MAX_SECRET_BYTES,
        "Credential exceeds the 1 MiB storage limit"
    );
    let previous = store.read(SERVICE, name)?.map(Zeroizing::new);
    // Explicitly saving a replacement can recover a corrupt manifest.
    let previous_manifest = previous
        .as_ref()
        .and_then(|bytes| manifest(bytes).ok().flatten());
    if value.encode_utf16().count() * 2 <= BLOB_LIMIT {
        store.write(SERVICE, name, &legacy_blob(value))?;
    } else {
        let next = Manifest {
            generation: Uuid::new_v4(),
            bytes: value.len(),
            sha256: hex::encode(Sha256::digest(value.as_bytes())),
        };
        let mut header = MAGIC.to_vec();
        header.extend(serde_json::to_vec(&next)?);
        let write = (|| -> Result<()> {
            for (index, part) in value.as_bytes().chunks(BLOB_LIMIT).enumerate() {
                store.write(PART_SERVICE, &next.part_name(name, index), part)?;
            }
            store.write(SERVICE, name, &header)
        })();
        if let Err(error) = write {
            cleanup_parts(store, name, &next);
            return Err(error);
        }
    }
    if let Some(previous) = previous_manifest {
        cleanup_parts(store, name, &previous);
    }
    Ok(())
}

fn get_in(store: &impl CredentialStore, name: &str) -> Result<String> {
    let root = Zeroizing::new(
        store
            .read(SERVICE, name)?
            .ok_or_else(|| anyhow!("Credential not found"))?,
    );
    let Some(manifest) = manifest(&root)? else {
        ensure!(root.len() % 2 == 0, "Invalid Windows credential encoding");
        let units = Zeroizing::new(
            root.chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>(),
        );
        return String::from_utf16(&units)
            .map_err(|_| anyhow!("Invalid Windows credential encoding"));
    };
    let mut bytes = Zeroizing::new(Vec::with_capacity(manifest.bytes));
    for index in 0..manifest.parts() {
        let part = Zeroizing::new(
            store
                .read(PART_SERVICE, &manifest.part_name(name, index))?
                .ok_or_else(|| anyhow!("Windows credential is incomplete; sign in again"))?,
        );
        let expected = (manifest.bytes - index * BLOB_LIMIT).min(BLOB_LIMIT);
        ensure!(
            part.len() == expected,
            "Windows credential fragment has an invalid length"
        );
        bytes.extend_from_slice(&part);
    }
    ensure!(
        hex::encode(Sha256::digest(&bytes)) == manifest.sha256,
        "Windows credential failed its integrity check"
    );
    std::str::from_utf8(&bytes)
        .map(str::to_owned)
        .map_err(|_| anyhow!("Invalid Windows credential encoding"))
}

fn delete_in(store: &impl CredentialStore, name: &str) -> Result<()> {
    if let Some(root) = store.read(SERVICE, name)?.map(Zeroizing::new) {
        if let Ok(Some(manifest)) = manifest(&root) {
            // Retain the manifest if cleanup fails so deleting again can retry.
            remove_parts(store, name, &manifest)?;
        }
        store.remove(SERVICE, name)?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
struct NativeStore;

#[cfg(target_os = "windows")]
impl CredentialStore for NativeStore {
    fn read(&self, service: &str, name: &str) -> Result<Option<Vec<u8>>> {
        match keyring::Entry::new(service, name)?.get_secret() {
            Ok(bytes) => Ok(Some(bytes)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn write(&self, service: &str, name: &str, value: &[u8]) -> Result<()> {
        keyring::Entry::new(service, name)?.set_secret(value)?;
        Ok(())
    }

    fn remove(&self, service: &str, name: &str) -> Result<()> {
        match keyring::Entry::new(service, name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

/// Serialize reads, updates and deletion across desktop windows and CLI
/// processes. Windows releases an abandoned mutex if a process exits/crashes.
#[cfg(target_os = "windows")]
struct CredentialLock(windows_sys::Win32::Foundation::HANDLE);

#[cfg(target_os = "windows")]
impl CredentialLock {
    fn acquire(name: &str) -> Result<Self> {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, WAIT_ABANDONED, WAIT_OBJECT_0},
            System::Threading::{CreateMutexW, WaitForSingleObject},
        };
        let key = format!("Local\\wisp-secret-{}", hex::encode(Sha256::digest(name)));
        let wide = key.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        // The null security attributes use the current user's default ACL.
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide.as_ptr()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error())
                .context("Could not lock Windows credential storage");
        }
        match unsafe { WaitForSingleObject(handle, 30_000) } {
            WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Self(handle)),
            _ => {
                unsafe { CloseHandle(handle) };
                bail!("Windows credential storage is busy or unavailable; retry saving the account")
            }
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for CredentialLock {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.0);
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(target_os = "windows")]
pub(super) fn set(name: &str, value: &str) -> Result<()> {
    let _lock = CredentialLock::acquire(name)?;
    set_in(&NativeStore, name, value)
}

#[cfg(target_os = "windows")]
pub(super) fn get(name: &str) -> Result<String> {
    let _lock = CredentialLock::acquire(name)?;
    get_in(&NativeStore, name)
}

#[cfg(target_os = "windows")]
pub(super) fn delete(name: &str) -> Result<()> {
    let _lock = CredentialLock::acquire(name)?;
    delete_in(&NativeStore, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        collections::BTreeMap,
    };

    #[derive(Default)]
    struct MemoryStore {
        values: RefCell<BTreeMap<(String, String), Vec<u8>>>,
        writes: Cell<usize>,
        fail_write: Cell<Option<usize>>,
        fail_delete: Cell<bool>,
    }

    impl CredentialStore for MemoryStore {
        fn read(&self, service: &str, name: &str) -> Result<Option<Vec<u8>>> {
            Ok(self
                .values
                .borrow()
                .get(&(service.into(), name.into()))
                .cloned())
        }

        fn write(&self, service: &str, name: &str, value: &[u8]) -> Result<()> {
            ensure!(value.len() <= BLOB_LIMIT, "fixture Windows blob limit");
            self.writes.set(self.writes.get() + 1);
            ensure!(
                self.fail_write.get() != Some(self.writes.get()),
                "fixture write failure"
            );
            self.values
                .borrow_mut()
                .insert((service.into(), name.into()), value.to_vec());
            Ok(())
        }

        fn remove(&self, service: &str, name: &str) -> Result<()> {
            ensure!(!self.fail_delete.get(), "fixture delete failure");
            self.values
                .borrow_mut()
                .remove(&(service.into(), name.into()));
            Ok(())
        }
    }

    #[test]
    fn reads_legacy_passwords_and_respects_utf16_byte_boundaries() {
        let store = MemoryStore::default();
        for value in [
            String::new(),
            "legacy 密码🙂".into(),
            "a".repeat(BLOB_LIMIT / 2),
            "🙂".repeat(BLOB_LIMIT / 4),
        ] {
            store.write(SERVICE, "key", &legacy_blob(&value)).unwrap();
            assert_eq!(get_in(&store, "key").unwrap(), value);
            set_in(&store, "key", &value).unwrap();
            assert!(!store
                .read(SERVICE, "key")
                .unwrap()
                .unwrap()
                .starts_with(MAGIC));
        }
        for value in [
            "a".repeat(BLOB_LIMIT / 2 + 1),
            "🙂".repeat(BLOB_LIMIT / 4 + 1),
            "汉🙂".repeat(2000),
        ] {
            set_in(&store, "key", &value).unwrap();
            assert!(store
                .read(SERVICE, "key")
                .unwrap()
                .unwrap()
                .starts_with(MAGIC));
            assert_eq!(get_in(&store, "key").unwrap(), value);
        }
    }

    #[test]
    fn large_chatgpt_and_xai_credentials_survive_reopening() {
        let store = MemoryStore::default();
        let codex = wisp_llm::codex_auth::CodexCredentials {
            access_token: "synthetic-access-".repeat(700),
            refresh_token: "synthetic-refresh-".repeat(300),
            expires_at_ms: 123456,
            account_id: "fixture-account".into(),
        };
        let xai = wisp_llm::xai_auth::XaiCredentials {
            access_token: codex.access_token.clone(),
            refresh_token: codex.refresh_token.clone(),
            expires_at_ms: 123456,
            token_endpoint: "https://auth.x.ai/oauth/token".into(),
            account: "fixture@example.test".into(),
        };
        set_in(&store, "codex_subscription", &codex.to_json()).unwrap();
        set_in(&store, "xai_subscription", &xai.to_json()).unwrap();
        set_in(&store, "model-key", &codex.access_token).unwrap();
        // A fresh adapter with only persisted entries has no process cache.
        let reopened = MemoryStore {
            values: RefCell::new(store.values.borrow().clone()),
            ..Default::default()
        };
        assert_eq!(
            wisp_llm::codex_auth::CodexCredentials::from_json(
                &get_in(&reopened, "codex_subscription").unwrap()
            ),
            Some(codex.clone())
        );
        assert_eq!(
            wisp_llm::xai_auth::XaiCredentials::from_json(
                &get_in(&reopened, "xai_subscription").unwrap()
            ),
            Some(xai)
        );
        assert_eq!(get_in(&reopened, "model-key").unwrap(), codex.access_token);
    }

    #[test]
    fn rotation_shrinking_and_deletion_remove_previous_fragments() {
        let store = MemoryStore::default();
        for value in [
            "initial".into(),
            "large".repeat(3000),
            "refreshed🙂".repeat(2000),
            "short".into(),
        ] {
            set_in(&store, "oauth", &value).unwrap();
            assert_eq!(get_in(&store, "oauth").unwrap(), value);
            let blob = store.read(SERVICE, "oauth").unwrap().unwrap();
            let expected = manifest(&blob).unwrap().map_or(1, |m| m.parts() + 1);
            assert_eq!(store.values.borrow().len(), expected);
        }
        set_in(&store, "oauth", &"last".repeat(5000)).unwrap();
        delete_in(&store, "oauth").unwrap();
        assert!(get_in(&store, "oauth").is_err());
        assert!(store.values.borrow().is_empty());
        delete_in(&store, "oauth").unwrap();
    }

    #[test]
    fn failed_fragment_or_manifest_write_preserves_previous_credentials() {
        let next = "replacement".repeat(2000);
        for failure in [1, 2, next.len().div_ceil(BLOB_LIMIT) + 1] {
            let store = MemoryStore::default();
            let previous = "old-credential".repeat(2000);
            set_in(&store, "oauth", &previous).unwrap();
            let before = store.values.borrow().clone();
            store.writes.set(0);
            store.fail_write.set(Some(failure));
            assert!(set_in(&store, "oauth", &next).is_err());
            assert_eq!(get_in(&store, "oauth").unwrap(), previous);
            assert_eq!(*store.values.borrow(), before);
        }
    }

    #[test]
    fn incomplete_corrupt_and_invalid_credentials_fail_without_returning_fragments() {
        let store = MemoryStore::default();
        let value = "synthetic-secret".repeat(2000);
        set_in(&store, "oauth", &value).unwrap();
        let root = store.read(SERVICE, "oauth").unwrap().unwrap();
        let m = manifest(&root).unwrap().unwrap();
        let part = m.part_name("oauth", 0);
        store
            .values
            .borrow_mut()
            .get_mut(&(PART_SERVICE.into(), part.clone()))
            .unwrap()[0] ^= 1;
        assert_eq!(
            get_in(&store, "oauth").unwrap_err().to_string(),
            "Windows credential failed its integrity check"
        );
        store.remove(PART_SERVICE, &part).unwrap();
        assert!(get_in(&store, "oauth")
            .unwrap_err()
            .to_string()
            .contains("incomplete"));
        store
            .write(SERVICE, "oauth", &[MAGIC, b"{}"].concat())
            .unwrap();
        assert!(get_in(&store, "oauth")
            .unwrap_err()
            .to_string()
            .contains("manifest"));
        store.write(SERVICE, "oauth", &[0]).unwrap();
        assert!(get_in(&store, "oauth")
            .unwrap_err()
            .to_string()
            .contains("encoding"));
    }

    #[test]
    fn excessive_sizes_are_rejected_before_writing_or_reading_fragments() {
        let store = MemoryStore::default();
        assert!(set_in(&store, "oauth", &"a".repeat(MAX_SECRET_BYTES + 1)).is_err());
        assert!(store.values.borrow().is_empty());
        let m = Manifest {
            generation: Uuid::new_v4(),
            bytes: MAX_SECRET_BYTES + 1,
            sha256: "a".repeat(64),
        };
        store
            .write(
                SERVICE,
                "oauth",
                &[MAGIC, &serde_json::to_vec(&m).unwrap()].concat(),
            )
            .unwrap();
        assert!(get_in(&store, "oauth")
            .unwrap_err()
            .to_string()
            .contains("manifest"));
    }

    #[test]
    fn a_failed_delete_keeps_the_manifest_for_retry() {
        let store = MemoryStore::default();
        set_in(&store, "oauth", &"large".repeat(1000)).unwrap();
        store.fail_delete.set(true);
        assert!(delete_in(&store, "oauth").is_err());
        assert!(store.read(SERVICE, "oauth").unwrap().is_some());
        store.fail_delete.set(false);
        delete_in(&store, "oauth").unwrap();
        assert!(store.values.borrow().is_empty());
    }

    /// Explicit local smoke only: exercises the release backend even in a debug
    /// test binary. Uses fresh UUID-scoped synthetic entries, never real tokens.
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "manual Windows Credential Manager smoke with synthetic credentials"]
    fn native_keyring_large_secret_roundtrip() {
        let name = format!("test:windows-secret:{}", Uuid::new_v4());
        let entry = keyring::Entry::new(SERVICE, &name).unwrap();
        assert!(matches!(entry.get_secret(), Err(keyring::Error::NoEntry)));
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = super::delete(&self.0);
            }
        }
        let _cleanup = Cleanup(name.clone());
        let value = "synthetic-token-🙂".repeat(1500);
        assert!(matches!(
            entry.set_password(&value),
            Err(keyring::Error::TooLong(_, 2560))
        ));
        entry.set_password("legacy-测试🙂").unwrap();
        assert_eq!(super::get(&name).unwrap(), "legacy-测试🙂");
        super::set(&name, &value).unwrap();
        assert_eq!(super::get(&name).unwrap(), value);
        let m = manifest(&entry.get_secret().unwrap()).unwrap().unwrap();
        super::set(&name, &"rotated-token".repeat(2000)).unwrap();
        for index in 0..m.parts() {
            assert!(NativeStore
                .read(PART_SERVICE, &m.part_name(&name, index))
                .unwrap()
                .is_none());
        }
        let m = manifest(&entry.get_secret().unwrap()).unwrap().unwrap();
        super::delete(&name).unwrap();
        assert!(matches!(entry.get_secret(), Err(keyring::Error::NoEntry)));
        for index in 0..m.parts() {
            assert!(NativeStore
                .read(PART_SERVICE, &m.part_name(&name, index))
                .unwrap()
                .is_none());
        }
    }
}

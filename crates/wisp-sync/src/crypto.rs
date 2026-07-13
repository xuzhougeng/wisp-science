use anyhow::{Context, Result};
use ring::{
    aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305},
    rand::{SecureRandom, SystemRandom},
};
use sha2::{Digest, Sha256};

use crate::SyncRevision;

const ENCRYPTED_MAGIC: &[u8; 4] = b"WSE1";
const NONCE_BYTES: usize = 12;
pub const PROJECT_KEY_BYTES: usize = 32;

pub fn random_project_key() -> Result<[u8; PROJECT_KEY_BYTES]> {
    let mut key = [0_u8; PROJECT_KEY_BYTES];
    SystemRandom::new()
        .fill(&mut key)
        .map_err(|_| anyhow::anyhow!("could not generate a project sync key"))?;
    Ok(key)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn encrypt_blob(key: &[u8; PROJECT_KEY_BYTES], plaintext: &[u8]) -> Result<Vec<u8>> {
    let unbound = UnboundKey::new(&CHACHA20_POLY1305, key)
        .map_err(|_| anyhow::anyhow!("invalid project sync key"))?;
    let key = LessSafeKey::new(unbound);
    let mut nonce_bytes = [0_u8; NONCE_BYTES];
    SystemRandom::new()
        .fill(&mut nonce_bytes)
        .map_err(|_| anyhow::anyhow!("could not generate an encryption nonce"))?;
    let mut sealed = plaintext.to_vec();
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce_bytes),
        Aad::empty(),
        &mut sealed,
    )
    .map_err(|_| anyhow::anyhow!("could not encrypt sync data"))?;
    let mut output = Vec::with_capacity(ENCRYPTED_MAGIC.len() + NONCE_BYTES + sealed.len());
    output.extend_from_slice(ENCRYPTED_MAGIC);
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&sealed);
    Ok(output)
}

pub fn decrypt_blob(key: &[u8; PROJECT_KEY_BYTES], encrypted: &[u8]) -> Result<Vec<u8>> {
    let header = ENCRYPTED_MAGIC.len() + NONCE_BYTES;
    if encrypted.len() < header + CHACHA20_POLY1305.tag_len()
        || &encrypted[..ENCRYPTED_MAGIC.len()] != ENCRYPTED_MAGIC
    {
        anyhow::bail!("invalid encrypted sync blob");
    }
    let nonce: [u8; NONCE_BYTES] = encrypted[ENCRYPTED_MAGIC.len()..header]
        .try_into()
        .context("invalid encrypted sync nonce")?;
    let unbound = UnboundKey::new(&CHACHA20_POLY1305, key)
        .map_err(|_| anyhow::anyhow!("invalid project sync key"))?;
    let key = LessSafeKey::new(unbound);
    let mut plaintext = encrypted[header..].to_vec();
    let opened = key
        .open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::empty(),
            &mut plaintext,
        )
        .map_err(|_| anyhow::anyhow!("sync blob authentication failed"))?;
    Ok(opened.to_vec())
}

fn revision_auth_bytes(revision: &SyncRevision) -> Result<Vec<u8>> {
    let mut unsigned = revision.clone();
    unsigned.auth_tag.clear();
    Ok(serde_json::to_vec(&unsigned)?)
}

fn revision_signing_key(key: &[u8; PROJECT_KEY_BYTES]) -> ring::hmac::Key {
    let mut derived = Sha256::new();
    derived.update(b"wisp-sync/revision-auth/v1\0");
    derived.update(key);
    ring::hmac::Key::new(ring::hmac::HMAC_SHA256, derived.finalize().as_ref())
}

pub fn sign_revision(key: &[u8; PROJECT_KEY_BYTES], revision: &mut SyncRevision) -> Result<()> {
    let signing_key = revision_signing_key(key);
    revision.auth_tag = hex::encode(ring::hmac::sign(
        &signing_key,
        &revision_auth_bytes(revision)?,
    ));
    Ok(())
}

pub fn verify_revision(key: &[u8; PROJECT_KEY_BYTES], revision: &SyncRevision) -> Result<()> {
    let tag = hex::decode(&revision.auth_tag).context("invalid revision authentication tag")?;
    let signing_key = revision_signing_key(key);
    ring::hmac::verify(&signing_key, &revision_auth_bytes(revision)?, &tag)
        .map_err(|_| anyhow::anyhow!("revision authentication failed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_blob_roundtrip_and_tamper_detection() {
        let key = random_project_key().unwrap();
        let encrypted = encrypt_blob(&key, b"portable project state").unwrap();
        assert_ne!(encrypted, b"portable project state");
        assert_eq!(
            decrypt_blob(&key, &encrypted).unwrap(),
            b"portable project state"
        );

        let mut tampered = encrypted;
        *tampered.last_mut().unwrap() ^= 1;
        assert!(decrypt_blob(&key, &tampered).is_err());
    }

    #[test]
    fn revision_authentication_detects_descriptor_tampering() {
        let key = [9_u8; PROJECT_KEY_BYTES];
        let mut revision = SyncRevision {
            protocol_version: 1,
            project_id: "project-1".into(),
            revision_id: "revision-1".into(),
            parent_revision: None,
            device_id: "device-1".into(),
            created_at: 1,
            metadata_blob: sha256_hex(b"metadata"),
            manifest_blob: sha256_hex(b"manifest"),
            workspace_blobs: vec![],
            state_hash: sha256_hex(b"state"),
            auth_tag: String::new(),
        };
        sign_revision(&key, &mut revision).unwrap();
        verify_revision(&key, &revision).unwrap();
        revision.device_id = "other-device".into();
        assert!(verify_revision(&key, &revision).is_err());
    }
}

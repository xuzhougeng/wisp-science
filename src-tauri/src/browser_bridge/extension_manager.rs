use base64::Engine;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use uuid::Uuid;
use walkdir::WalkDir;

const REQUIRED_FILES: &[&str] = &[
    "background.js",
    "capture.js",
    "chat_adapter.js",
    "downloads.js",
    "manifest.json",
    "popup.html",
    "popup.js",
    "protocol.js",
    "scan_page.js",
    "session_config.js",
    "tab_ops.js",
    "wait_engine.js",
    "wait_tab.js",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionPackage {
    pub version: String,
    pub digest: String,
}

/// Copy the bundled extension into a stable application-data directory.
///
/// Chrome remembers the directory used by "Load unpacked". Release resource
/// paths can move between Wisp installs, so the shared browser extension must
/// point at this managed copy instead. Replacement is staged and verified
/// before the old copy is moved out of the way.
pub fn sync(
    source: &Path,
    destination: &Path,
    expected_extension_id: &str,
) -> Result<ExtensionPackage, String> {
    let source_package = inspect(source, expected_extension_id)?;
    if same_path(source, destination) {
        return Ok(source_package);
    }
    if inspect(destination, expected_extension_id).ok().as_ref() == Some(&source_package) {
        return Ok(source_package);
    }

    let parent = destination.parent().ok_or_else(|| {
        format!(
            "managed extension path has no parent: {}",
            destination.display()
        )
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("create managed extension parent: {error}"))?;
    let nonce = Uuid::new_v4();
    let staging = parent.join(format!(".browser-extension-staging-{nonce}"));
    let backup = parent.join(format!(".browser-extension-backup-{nonce}"));
    copy_tree(source, &staging)?;

    let staged_package = inspect(&staging, expected_extension_id).map_err(|error| {
        let _ = fs::remove_dir_all(&staging);
        format!("verify staged browser extension: {error}")
    })?;
    if staged_package != source_package {
        let _ = fs::remove_dir_all(&staging);
        return Err("staged browser extension digest does not match the bundled package".into());
    }

    let had_destination = destination.exists();
    if had_destination {
        fs::rename(destination, &backup)
            .map_err(|error| format!("move old managed browser extension aside: {error}"))?;
    }
    if let Err(error) = fs::rename(&staging, destination) {
        if had_destination {
            let _ = fs::rename(&backup, destination);
        }
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("activate managed browser extension: {error}"));
    }
    if had_destination {
        let _ = fs::remove_dir_all(&backup);
    }

    let installed = inspect(destination, expected_extension_id)
        .map_err(|error| format!("verify installed browser extension: {error}"))?;
    if installed != source_package {
        return Err("installed browser extension digest does not match the bundled package".into());
    }
    Ok(installed)
}

pub fn verify(
    source: &Path,
    destination: &Path,
    expected_extension_id: &str,
) -> Result<ExtensionPackage, String> {
    let source_package = inspect(source, expected_extension_id)?;
    let installed = inspect(destination, expected_extension_id)?;
    if installed != source_package {
        return Err(format!(
            "managed browser extension {} does not match bundled version {}",
            destination.display(),
            source_package.version
        ));
    }
    Ok(installed)
}

pub fn inspect(dir: &Path, expected_extension_id: &str) -> Result<ExtensionPackage, String> {
    if !dir.is_dir() {
        return Err(format!(
            "browser extension directory is missing: {}",
            dir.display()
        ));
    }
    for relative in REQUIRED_FILES {
        if !dir.join(relative).is_file() {
            return Err(format!("browser extension is missing {relative}"));
        }
    }

    let manifest_text = fs::read_to_string(dir.join("manifest.json"))
        .map_err(|error| format!("read browser extension manifest: {error}"))?;
    let manifest: Value = serde_json::from_str(&manifest_text)
        .map_err(|error| format!("parse browser extension manifest: {error}"))?;
    if manifest.get("name").and_then(Value::as_str) != Some("Wisp Real Browser Bridge") {
        return Err("browser extension manifest has an unexpected name".into());
    }
    let version = manifest
        .get("version")
        .and_then(Value::as_str)
        .filter(|value| semver::Version::parse(value).is_ok())
        .ok_or_else(|| "browser extension manifest has an invalid version".to_string())?
        .to_string();
    let key = manifest
        .get("key")
        .and_then(Value::as_str)
        .ok_or_else(|| "browser extension manifest has no signing key".to_string())?;
    if extension_id_from_key(key)? != expected_extension_id {
        return Err("browser extension signing key does not match Wisp's extension id".into());
    }
    let protocol = fs::read_to_string(dir.join("protocol.js"))
        .map_err(|error| format!("read browser extension protocol: {error}"))?;
    if !protocol.contains(&format!("extensionVersion: \"{version}\"")) {
        return Err(format!(
            "browser extension protocol version does not match manifest {version}"
        ));
    }

    let inventory = inventory(dir)?;
    let mut digest = Sha256::new();
    for (path, file_hash) in &inventory {
        digest.update(path.as_bytes());
        digest.update([0]);
        digest.update(file_hash.as_bytes());
        digest.update([0]);
    }
    Ok(ExtensionPackage {
        version,
        digest: hex::encode(digest.finalize()),
    })
}

fn extension_id_from_key(key: &str) -> Result<String, String> {
    let der = base64::engine::general_purpose::STANDARD
        .decode(key)
        .map_err(|error| format!("decode browser extension signing key: {error}"))?;
    let digest = Sha256::digest(der);
    Ok(digest[..16]
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .map(|nibble| char::from(b'a' + nibble))
        .collect())
}

fn inventory(dir: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut files = BTreeMap::new();
    for entry in WalkDir::new(dir).follow_links(false) {
        let entry = entry.map_err(|error| format!("walk browser extension: {error}"))?;
        if entry.file_type().is_symlink() {
            return Err(format!(
                "browser extension package contains a symlink: {}",
                entry.path().display()
            ));
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(dir)
            .map_err(|error| format!("resolve browser extension file: {error}"))?;
        let name = path_key(relative)?;
        let bytes = fs::read(entry.path())
            .map_err(|error| format!("read browser extension file {name}: {error}"))?;
        files.insert(name, hex::encode(Sha256::digest(bytes)));
    }
    Ok(files)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("create browser extension staging directory: {error}"))?;
    for entry in WalkDir::new(source).min_depth(1).follow_links(false) {
        let entry = entry.map_err(|error| format!("walk bundled browser extension: {error}"))?;
        if entry.file_type().is_symlink() {
            let _ = fs::remove_dir_all(destination);
            return Err(format!(
                "bundled browser extension contains a symlink: {}",
                entry.path().display()
            ));
        }
        let relative = entry
            .path()
            .strip_prefix(source)
            .map_err(|error| format!("resolve bundled extension file: {error}"))?;
        let target = destination.join(relative);
        let result = if entry.file_type().is_dir() {
            fs::create_dir_all(&target)
        } else {
            fs::copy(entry.path(), &target).map(|_| ())
        };
        if let Err(error) = result {
            let _ = fs::remove_dir_all(destination);
            return Err(format!(
                "copy browser extension {}: {error}",
                relative.display()
            ));
        }
    }
    Ok(())
}

fn path_key(path: &Path) -> Result<String, String> {
    let parts = path
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| "browser extension path is not valid UTF-8".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(parts.join("/"))
}

fn same_path(left: &Path, right: &Path) -> bool {
    left == right
        || dunce::canonicalize(left)
            .ok()
            .zip(dunce::canonicalize(right).ok())
            .is_some_and(|(left, right)| left == right)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXTENSION_ID: &str = "gnkjgagleagkgdlkkcianolobfdoocnp";

    #[test]
    fn managed_copy_is_verified_and_repaired() {
        let source =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../browser-extension");
        let root = std::env::temp_dir().join(format!("wisp-managed-extension-{}", Uuid::new_v4()));
        let destination = root.join("browser-extension");

        let package = sync(&source, &destination, EXTENSION_ID).unwrap();
        assert_eq!(package.version, "0.3.1");
        assert_eq!(
            verify(&source, &destination, EXTENSION_ID).unwrap(),
            package
        );

        fs::write(destination.join("wait_tab.js"), "tampered").unwrap();
        assert!(verify(&source, &destination, EXTENSION_ID).is_err());
        assert_eq!(sync(&source, &destination, EXTENSION_ID).unwrap(), package);
        assert_eq!(
            verify(&source, &destination, EXTENSION_ID).unwrap(),
            package
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_bundled_package_does_not_replace_the_last_verified_copy() {
        let bundled =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../browser-extension");
        let root = std::env::temp_dir().join(format!("wisp-invalid-extension-{}", Uuid::new_v4()));
        let source = root.join("source");
        let destination = root.join("browser-extension");
        copy_tree(&bundled, &source).unwrap();
        let verified = sync(&bundled, &destination, EXTENSION_ID).unwrap();

        fs::write(source.join("protocol.js"), "var WISP_PROTOCOL = {};").unwrap();
        let error = sync(&source, &destination, EXTENSION_ID).unwrap_err();
        assert!(error.contains("protocol version does not match"));
        assert_eq!(
            verify(&bundled, &destination, EXTENSION_ID).unwrap(),
            verified
        );

        let _ = fs::remove_dir_all(root);
    }
}

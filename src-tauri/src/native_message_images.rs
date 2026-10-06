//! Bounded, session-scoped previews. Bound images always read the captured
//! artifact version; a missing snapshot must not silently show a newer figure.
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{io::Cursor, path::Path};
use wisp_dto::native_conversations::ImageRequest;
use wisp_store::{StateScope, Store};

static DECODERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

pub(crate) async fn read(
    store: &Store,
    working_root: &Path,
    scope: &StateScope,
    project: &str,
    request: ImageRequest,
) -> Result<serde_json::Value, String> {
    if scope.project_id() != project {
        return Err("image project does not match the conversation".into());
    }
    let (root, path, label) = match (request.resource_id, request.path) {
        (Some(id), None) if !id.is_empty() => {
            let link = store
                .list_message_resource_links(&request.session_id, 0, None)
                .await
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|link| link.id == id)
                .ok_or("image resource is not bound to this conversation")?;
            if link.status != "ready" || !link.mime_type.starts_with("image/") {
                return Err("image resource is unavailable".into());
            }
            let version_id = link
                .artifact_version_id
                .ok_or("image has no captured version")?;
            let version = store
                .get_artifact_version(&version_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("captured image version is unavailable")?;
            if link.artifact_id.as_deref() != Some(version.artifact_id.as_str())
                || !store
                    .artifact_visible_in_scope(&version.artifact_id, scope)
                    .await
                    .map_err(|e| e.to_string())?
            {
                return Err("image version is outside the conversation state".into());
            }
            let artifact = store
                .get_artifact_detail(&version.artifact_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("image artifact is unavailable")?;
            if artifact.project_id != project {
                return Err("image artifact belongs to another project".into());
            }
            let root = if matches!(scope, StateScope::Exploration { .. }) {
                working_root.to_path_buf()
            } else {
                artifact.project_root.into()
            };
            (
                root,
                version.storage_path,
                format!("artifact-version:{version_id}"),
            )
        }
        (None, Some(path)) if !path.is_empty() && !path.contains("://") => {
            (working_root.to_path_buf(), path.clone(), path)
        }
        _ => return Err("provide exactly one local image path or resource id".into()),
    };
    let permit = DECODERS.acquire().await.map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let bytes = crate::file_browser::read_file_bytes_at(&root, &path, Some(32 * 1024 * 1024))?;
        let total = bytes.len() as u64;
        let png = thumbnail(&bytes)?;
        serde_json::to_value(wisp_dto::FileContent {
            path: label,
            mime: "image/png".into(),
            text: None,
            base64: Some(STANDARD.encode(png)),
            truncated: false,
            total_bytes: Some(total),
        })
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn thumbnail(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|e| format!("image preview unavailable: {e}"))?;
    let mut png = Cursor::new(Vec::new());
    image
        .thumbnail(1024, 1024)
        .to_rgba8()
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(png.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png(width: u32, height: u32, red: u8) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([red, 0, 0, 255]));
        let mut out = Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }
    #[test]
    fn thumbnails_preserve_aspect_and_reject_invalid_images() {
        let bytes = thumbnail(&png(2400, 1200, 10)).unwrap();
        let image = image::load_from_memory(&bytes).unwrap();
        assert_eq!((image.width(), image.height()), (1024, 512));
        assert!(thumbnail(b"not an image").is_err());
    }
    #[test]
    fn supported_raster_formats_decode_to_png() {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            12,
            6,
            image::Rgb([200, 20, 10]),
        ));
        for format in [
            image::ImageFormat::Jpeg,
            image::ImageFormat::Gif,
            image::ImageFormat::WebP,
            image::ImageFormat::Bmp,
            image::ImageFormat::Tiff,
        ] {
            let mut input = Cursor::new(Vec::new());
            image.write_to(&mut input, format).unwrap();
            let png = thumbnail(input.get_ref()).unwrap();
            assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
            let result = image::load_from_memory(&png).unwrap();
            assert_eq!((result.width(), result.height()), (12, 6));
        }
        let scientific =
            image::ImageBuffer::from_pixel(12, 6, image::Rgba([65535_u16, 32768, 0, 65535]));
        let mut input = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba16(scientific)
            .write_to(&mut input, image::ImageFormat::Tiff)
            .unwrap();
        let result = image::load_from_memory(&thumbnail(input.get_ref()).unwrap()).unwrap();
        assert_eq!(
            result.color(),
            image::ColorType::Rgba8,
            "16-bit inputs must fit the same bounded preview transport"
        );
    }
    #[tokio::test]
    async fn captured_versions_and_paths_remain_scoped() {
        let root =
            std::env::temp_dir().join(format!("wisp_native_images_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&root.join("store.sqlite")).await.unwrap();
        store
            .create_project("project", "Project", &root.to_string_lossy())
            .await
            .unwrap();
        store
            .create_frame("frame", "project", "OPERON", "model")
            .await
            .unwrap();
        store
            .create_frame("other", "project", "OPERON", "model")
            .await
            .unwrap();
        std::fs::write(root.join("plot.png"), png(2, 2, 10)).unwrap();
        let links = crate::resource_refs::bind_new_message_resources(
            &store,
            &root,
            "project",
            "frame",
            2,
            "![plot](plot.png)",
        )
        .await;
        std::fs::write(root.join("plot.png"), png(2, 2, 200)).unwrap();
        let scope = StateScope::mainline("project");
        let request =
            |session: &str, resource_id: Option<String>, path: Option<String>| ImageRequest {
                session_id: session.into(),
                resource_id,
                path,
            };
        let captured = read(
            &store,
            &root,
            &scope,
            "project",
            request("frame", Some(links[0].id.clone()), None),
        )
        .await
        .unwrap();
        let bytes = STANDARD
            .decode(captured["base64"].as_str().unwrap())
            .unwrap();
        assert_eq!(
            image::load_from_memory(&bytes)
                .unwrap()
                .to_rgba8()
                .get_pixel(0, 0)[0],
            10
        );
        assert!(read(
            &store,
            &root,
            &scope,
            "project",
            request("other", Some(links[0].id.clone()), None)
        )
        .await
        .is_err());
        assert!(read(
            &store,
            &root,
            &scope,
            "other-project",
            request("frame", None, Some("plot.png".into()))
        )
        .await
        .is_err());
        for path in [
            "../outside.png",
            "https://example.com/plot.png",
            "store.sqlite",
        ] {
            assert!(read(
                &store,
                &root,
                &scope,
                "project",
                request("frame", None, Some(path.into()))
            )
            .await
            .is_err());
        }
        assert!(read(
            &store,
            &root,
            &scope,
            "project",
            request("frame", Some(links[0].id.clone()), Some("plot.png".into()))
        )
        .await
        .is_err());
        let version = store
            .get_artifact_version(links[0].artifact_version_id.as_ref().unwrap())
            .await
            .unwrap()
            .unwrap();
        std::fs::remove_file(root.join(version.storage_path)).unwrap();
        assert!(read(
            &store,
            &root,
            &scope,
            "project",
            request("frame", Some(links[0].id.clone()), None)
        )
        .await
        .is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}

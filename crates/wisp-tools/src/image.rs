//! `view_image` helper — load a local image as a data URI for vision input.

use crate::env::{ImageData, ToolResult};
use base64::Engine;
use std::path::Path;

const MAX_BYTES: usize = 5 * 1024 * 1024;
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

pub fn view_image(path: &str) -> ToolResult {
    if path.starts_with("http://") || path.starts_with("https://") {
        return ToolResult::fail(
            "view_image error: URL inputs are not supported. Download to a local file first.",
        );
    }
    let size = match std::fs::metadata(path) {
        Ok(m) => m.len() as usize,
        Err(e) => return ToolResult::fail(format!("view_image error: cannot stat file: {e}")),
    };
    if size == 0 {
        return ToolResult::fail("view_image error: image file is empty");
    }
    if size > MAX_BYTES {
        return ToolResult::fail(format!(
            "view_image error: image too large ({size} bytes, max {MAX_BYTES})"
        ));
    }
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !IMAGE_EXTS.contains(&ext.as_str()) {
        return ToolResult::fail(format!(
            "view_image error: unsupported image format '{ext}' (supported: png,jpg,jpeg,gif,webp)"
        ));
    }
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return ToolResult::fail(format!("view_image error: cannot read file: {e}")),
    };
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/png",
    };
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let data_url = format!("data:{mime};base64,{b64}");
    let label = format!("Image: {path} ({size} bytes, {mime})");
    ToolResult::image(ImageData {
        mime: mime.into(),
        data_url,
        label,
    })
}

use std::path::Path;

pub fn mime_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        Some("doc" | "docm") => "application/msword",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xls" | "xlsm" | "xlsb") => "application/vnd.ms-excel",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("ppt" | "pps" | "pot" | "pptm" | "ppsx" | "ppsm") => "application/vnd.ms-powerpoint",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("odt") => "application/vnd.oasis.opendocument.text",
        Some("ods") => "application/vnd.oasis.opendocument.spreadsheet",
        Some("odp") => "application/vnd.oasis.opendocument.presentation",
        Some("rtf") => "application/rtf",
        Some("epub") => "application/epub+zip",
        Some("bib") => "text/x-bibtex",
        Some("csv") => "text/csv",
        Some("tsv") => "text/tab-separated-values",
        Some("html" | "htm") => "text/html",
        Some("json") => "application/json",
        Some("ipynb") => "application/x-ipynb+json",
        Some("md") => "text/markdown",
        Some("r") => "text/x-r",
        Some("py") => "text/x-python",
        Some("sh") => "text/x-shellscript",
        Some("fasta" | "fa") => "text/x-fasta",
        Some("pdb") | Some("mol2") | Some("cif") => "chemical/x-pdb",
        Some("sdf" | "mol") => "chemical/x-mdl-molfile",
        _ => "application/octet-stream",
    }
}

/// Mimes whose bytes are shown as text.
pub fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/")
        || mime == "application/json"
        || mime == "application/x-ipynb+json"
        || mime == "text/markdown"
}

/// An extension allowlist always lags reality — `.toml`, `.lock`, `.yaml`, `.R`
/// and friends previewed as "unsupported" purely because nothing named them.
/// For anything without an explicit mime, let the bytes decide instead.
pub fn looks_like_text(bytes: &[u8]) -> bool {
    !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
}

/// The shared text-vs-binary decision for previews, local or remote: named
/// text mimes are text, unnamed extensions are sniffed, the rest is not.
pub fn is_text_preview_bytes(mime: &str, bytes: &[u8]) -> bool {
    is_text_mime(mime)
        || mime == "chemical/x-pdb"
        || mime == "chemical/x-mdl-molfile"
        || (mime == "application/octet-stream" && looks_like_text(bytes))
}

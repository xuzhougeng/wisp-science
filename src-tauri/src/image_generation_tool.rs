//! `generate_image` — one configured image-model request that writes a PNG
//! into the current project through an OpenAI-compatible Images API or the
//! native Google Gemini `generateContent` API.
//! Configured model IDs are opaque; known xAI and Gemini IDs have parameter
//! adapters.

use async_trait::async_trait;
use base64::Engine;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::time::Duration;
use wisp_llm::ToolSchema;
use wisp_tools::{Tool, ToolEnv, ToolEvent, ToolResult};

const MAX_PROMPT_BYTES: usize = 32 * 1024;
const MAX_BASE64_BYTES: usize = 70 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 50 * 1024 * 1024;
const MAX_ERROR_BYTES: usize = 1024 * 1024;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

#[derive(Debug)]
enum ImageSource {
    Base64(String),
    Url(String),
}

fn parse_image_url_or_data_uri(raw: &str) -> Option<ImageSource> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() >= 5 && trimmed[..5].eq_ignore_ascii_case("data:") {
        let (header, payload) = trimmed.split_once(',')?;
        let header_lower = header.to_ascii_lowercase();
        if header_lower.starts_with("data:image/") && header_lower.contains(";base64") {
            let payload = payload.trim();
            if !payload.is_empty() {
                return Some(ImageSource::Base64(payload.to_string()));
            }
        }
        return None;
    }
    Some(ImageSource::Url(trimmed.to_string()))
}

fn image_source_from_response(value: &Value) -> Result<ImageSource, String> {
    if value.get("candidates").is_some() && value.pointer("/data/0").is_none() {
        return image_source_from_gemini_response(value);
    }
    let first = value
        .pointer("/data/0")
        .ok_or_else(|| "image-generation response has no data[0]".to_string())?;
    if let Some(encoded) = first
        .get("b64_json")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(ImageSource::Base64(encoded.to_string()));
    }
    if let Some(url) = first
        .get("url")
        .and_then(Value::as_str)
        .and_then(parse_image_url_or_data_uri)
    {
        return Ok(url);
    }
    Err("image-generation response has no data[0].b64_json or data[0].url".into())
}

fn image_source_from_gemini_part(part: &Value) -> Option<ImageSource> {
    if let Some(encoded) = part
        .get("inlineData")
        .or_else(|| part.get("inline_data"))
        .and_then(|inline| inline.get("data"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(ImageSource::Base64(encoded.to_string()));
    }
    if let Some(uri) = part
        .get("fileData")
        .or_else(|| part.get("file_data"))
        .and_then(|file| file.get("fileUri").or_else(|| file.get("file_uri")))
        .and_then(Value::as_str)
        .and_then(parse_image_url_or_data_uri)
    {
        return Some(uri);
    }
    None
}

fn image_source_from_gemini_response(value: &Value) -> Result<ImageSource, String> {
    let mut final_image: Option<ImageSource> = None;
    let mut thought_image: Option<ImageSource> = None;
    let mut notes = Vec::new();
    if let Some(candidates) = value.get("candidates").and_then(Value::as_array) {
        for candidate in candidates {
            if let Some(parts) = candidate
                .pointer("/content/parts")
                .and_then(Value::as_array)
            {
                for part in parts {
                    let is_thought = part
                        .get("thought")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    if let Some(source) = image_source_from_gemini_part(part) {
                        if is_thought {
                            thought_image = Some(source);
                        } else {
                            final_image = Some(source);
                        }
                    } else if !is_thought {
                        if let Some(text) = part
                            .get("text")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|text| !text.is_empty())
                        {
                            notes.push(text);
                        }
                    }
                }
            }
        }
    }
    if let Some(source) = final_image.or(thought_image) {
        return Ok(source);
    }
    let block_reason = value
        .pointer("/promptFeedback/blockReason")
        .or_else(|| value.pointer("/prompt_feedback/block_reason"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let finish_reason = value
        .pointer("/candidates/0/finishReason")
        .or_else(|| value.pointer("/candidates/0/finish_reason"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut detail = Vec::new();
    if let Some(reason) = block_reason {
        detail.push(format!("blockReason={reason}"));
    }
    if let Some(reason) = finish_reason {
        detail.push(format!("finishReason={reason}"));
    }
    let text_note = notes.join(" ");
    let text_note = text_note.trim();
    let suffix = if !detail.is_empty() && !text_note.is_empty() {
        format!(
            " ({}): {}",
            detail.join(", "),
            truncate_chars(text_note, 240)
        )
    } else if !detail.is_empty() {
        format!(" ({})", detail.join(", "))
    } else if !text_note.is_empty() {
        format!(": {}", truncate_chars(text_note, 240))
    } else {
        String::new()
    };
    Err(format!(
        "Gemini image-generation response contained no image{suffix}"
    ))
}

fn image_source_from_chat_item(item: &Value) -> Option<ImageSource> {
    if let Some(encoded) = item
        .get("b64_json")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(ImageSource::Base64(encoded.to_string()));
    }
    if let Some(source) = image_source_from_gemini_part(item) {
        return Some(source);
    }
    item.pointer("/image_url/url")
        .or_else(|| item.get("image_url"))
        .or_else(|| item.get("url"))
        .and_then(Value::as_str)
        .and_then(parse_image_url_or_data_uri)
}

fn extract_image_from_text(text: &str) -> Option<ImageSource> {
    let lower = text.to_ascii_lowercase();
    if let Some(start) = lower.find("data:image/") {
        let rest = &text[start..];
        let end = rest
            .find(|ch: char| ch.is_whitespace() || matches!(ch, ')' | '"' | '\'' | '>' | '<'))
            .unwrap_or(rest.len());
        if let Some(source) = parse_image_url_or_data_uri(&rest[..end]) {
            return Some(source);
        }
    }
    let mut cursor = text;
    while let Some(marker) = cursor.find("![") {
        let after_marker = &cursor[marker + 2..];
        let Some(close_alt) = after_marker.find("](") else {
            break;
        };
        let after_paren = &after_marker[close_alt + 2..];
        let Some(close_url) = after_paren.find(')') else {
            break;
        };
        let candidate = after_paren[..close_url].trim();
        let candidate = candidate
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_matches(|ch| matches!(ch, '<' | '>'));
        if candidate.starts_with("https://") || candidate.starts_with("http://") {
            return Some(ImageSource::Url(candidate.to_string()));
        }
        cursor = &after_paren[close_url + 1..];
    }
    None
}

fn image_source_from_chat_response(value: &Value) -> Result<ImageSource, String> {
    if value.get("candidates").is_some() && value.pointer("/choices/0").is_none() {
        return image_source_from_gemini_response(value);
    }
    let message = value
        .pointer("/choices/0/message")
        .ok_or_else(|| "image-generation chat response has no choices[0].message".to_string())?;
    if let Some(images) = message.get("images").and_then(Value::as_array) {
        if let Some(source) = images.iter().rev().find_map(image_source_from_chat_item) {
            return Ok(source);
        }
    }
    match message.get("content") {
        Some(Value::Array(parts)) => {
            if let Some(source) = parts.iter().rev().find_map(image_source_from_chat_item) {
                return Ok(source);
            }
        }
        Some(Value::String(text)) => {
            if let Some(source) = extract_image_from_text(text) {
                return Ok(source);
            }
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Err(format!(
                    "image-generation chat response contained no image: {}",
                    truncate_chars(trimmed, 240)
                ));
            }
        }
        _ => {}
    }
    Err("image-generation chat response contained no image".into())
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        out.push('…');
    }
    out
}

fn decode_image_base64(encoded: &str) -> Result<Vec<u8>, String> {
    if encoded.len() > MAX_BASE64_BYTES {
        return Err("generated image response is too large".into());
    }
    let cleaned: Cow<'_, str> = if encoded.bytes().any(|byte| byte.is_ascii_whitespace()) {
        Cow::Owned(
            encoded
                .chars()
                .filter(|ch| !ch.is_ascii_whitespace())
                .collect(),
        )
    } else {
        Cow::Borrowed(encoded)
    };
    base64::engine::general_purpose::STANDARD
        .decode(cleaned.as_bytes())
        .map_err(|error| format!("generated image is not valid base64: {error}"))
}

fn same_model_id(left: &str, right: &str) -> bool {
    crate::models::model_id_tail(left).eq_ignore_ascii_case(crate::models::model_id_tail(right))
}

pub struct GenerateImageTool {
    api_url: String,
    api_key: String,
    model: String,
    proxy: Option<String>,
    session_id: String,
    options: crate::models::ImageGenerationOptions,
}

impl GenerateImageTool {
    pub fn new(api_url: String, api_key: String, model: String, proxy: Option<String>) -> Self {
        Self {
            api_url,
            api_key,
            model,
            proxy,
            session_id: uuid::Uuid::new_v4().to_string(),
            options: crate::models::ImageGenerationOptions::default(),
        }
    }

    pub fn with_options(mut self, options: crate::models::ImageGenerationOptions) -> Self {
        self.options = options;
        self
    }

    pub fn with_session_id(mut self, session_id: &str) -> Self {
        self.session_id = session_id.to_string();
        self
    }

    fn request_headers(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        wisp_llm::provider::apply_request_identity(
            request,
            &self.api_url,
            &self.options.user_agent,
            self.options.send_user_agent,
            &self.session_id,
            self.options.send_session_id,
            &self.options.session_header_name,
        )
    }

    fn is_gemini_protocol(&self) -> bool {
        crate::models::is_gemini_provider(&self.options.provider)
    }

    fn is_gemini_model(&self) -> bool {
        crate::models::is_gemini_image_model(self.configured_model())
    }

    fn gemini_model_id(&self) -> &str {
        let model = self.configured_model();
        if model.len() > 7 && model[..7].eq_ignore_ascii_case("models/") {
            &model[7..]
        } else {
            model
        }
    }

    fn is_google_openai_endpoint(&self) -> bool {
        reqwest::Url::parse(&self.api_root())
            .ok()
            .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
            .is_some_and(|host| host == "generativelanguage.googleapis.com")
    }

    fn api_root(&self) -> String {
        let base = self.api_url.trim().trim_end_matches('/');
        if let Some(root) = base.strip_suffix("/images/generations") {
            root.to_string()
        } else if let Some(root) = base.strip_suffix("/chat/completions") {
            root.to_string()
        } else if matches!(base, "https://api.openai.com" | "https://api.x.ai") {
            format!("{base}/v1")
        } else if base.eq_ignore_ascii_case("https://generativelanguage.googleapis.com") {
            format!("{base}/v1beta/openai")
        } else if base.eq_ignore_ascii_case("https://generativelanguage.googleapis.com/v1beta") {
            format!("{base}/openai")
        } else {
            base.to_string()
        }
    }

    fn gemini_api_root(&self) -> String {
        let base = self.api_url.trim().trim_end_matches('/');
        let base = if let Some((prefix, _)) = base.rsplit_once("/models/") {
            prefix
        } else if let Some(prefix) = base.strip_suffix("/models") {
            prefix
        } else if let Some(prefix) = base.strip_suffix("/openai") {
            prefix
        } else {
            base
        };
        let base = base.trim_end_matches('/');
        if base.ends_with("/v1beta") || base.ends_with("/v1") || base.ends_with("/v1alpha") {
            base.to_string()
        } else {
            format!("{base}/v1beta")
        }
    }

    fn configured_model(&self) -> &str {
        self.model.trim()
    }

    fn resolve_choice<'a>(&'a self, tool: &'a str, profile: &'a str) -> &'a str {
        if !tool.is_empty() && !tool.eq_ignore_ascii_case("auto") {
            tool
        } else if !profile.is_empty() {
            profile
        } else {
            "auto"
        }
    }

    fn resolve_gemini_aspect_ratio(&self, size: &str) -> Option<&str> {
        match size {
            "1024x1024" => Some("1:1"),
            "1536x1024" => Some("3:2"),
            "1024x1536" => Some("2:3"),
            _ => {
                let aspect = self.options.aspect_ratio.trim();
                (!aspect.is_empty() && !aspect.eq_ignore_ascii_case("auto")).then_some(aspect)
            }
        }
    }

    fn resolve_gemini_resolution(&self) -> Option<String> {
        let resolution = self.options.resolution.trim();
        if resolution.is_empty() || resolution.eq_ignore_ascii_case("auto") {
            None
        } else {
            crate::models::GEMINI_IMAGE_RESOLUTIONS
                .iter()
                .copied()
                .find(|candidate| candidate.eq_ignore_ascii_case(resolution))
                .map(str::to_string)
        }
    }

    fn gemini_request_body(&self, prompt: &str, size: &str) -> Value {
        let mut generation_config = json!({
            "responseModalities": ["TEXT", "IMAGE"],
        });
        let mut image_config = serde_json::Map::new();
        if let Some(aspect) = self.resolve_gemini_aspect_ratio(size) {
            image_config.insert("aspectRatio".into(), json!(aspect));
        }
        if let Some(resolution) = self.resolve_gemini_resolution() {
            image_config.insert("imageSize".into(), json!(resolution));
        }
        if !image_config.is_empty() {
            generation_config["imageConfig"] = Value::Object(image_config);
        }
        json!({
            "contents": [{
                "role": "user",
                "parts": [{ "text": prompt }],
            }],
            "generationConfig": generation_config,
        })
    }

    fn chat_completions_image_body(&self, prompt: &str, size: &str) -> Value {
        let mut body = json!({
            "model": self.configured_model(),
            "messages": [{ "role": "user", "content": prompt }],
            "modalities": ["image", "text"],
        });
        let mut image_config = serde_json::Map::new();
        if let Some(aspect) = self.resolve_gemini_aspect_ratio(size) {
            image_config.insert("aspect_ratio".into(), json!(aspect));
        }
        if let Some(resolution) = self.resolve_gemini_resolution() {
            image_config.insert("image_size".into(), json!(resolution));
        }
        if !image_config.is_empty() {
            body["image_config"] = Value::Object(image_config);
        }
        body
    }

    fn request_body(&self, prompt: &str, size: &str, quality: &str) -> Value {
        let size = self.resolve_choice(size, &self.options.size);
        let quality = self.resolve_choice(quality, &self.options.quality);
        if crate::models::is_grok_imagine_model(self.configured_model()) {
            let mut body = json!({
                "model": self.configured_model(),
                "prompt": prompt,
                "n": 1,
                "response_format": "b64_json",
            });
            match size {
                "1024x1024" => {
                    body["aspect_ratio"] = json!("1:1");
                    body["resolution"] = json!("1k");
                }
                "1536x1024" => {
                    body["aspect_ratio"] = json!("3:2");
                    body["resolution"] = json!("1k");
                }
                "1024x1536" => {
                    body["aspect_ratio"] = json!("2:3");
                    body["resolution"] = json!("1k");
                }
                other => {
                    let aspect = if other != "auto" {
                        other
                    } else if !self.options.aspect_ratio.is_empty() {
                        self.options.aspect_ratio.as_str()
                    } else {
                        "auto"
                    };
                    body["aspect_ratio"] = json!(aspect);
                    let resolution = if !self.options.resolution.is_empty() {
                        self.options.resolution.as_str()
                    } else {
                        ""
                    };
                    if !resolution.is_empty() {
                        body["resolution"] = json!(resolution);
                    }
                }
            }
            match quality {
                "low" | "medium" => body["quality"] = json!(quality),
                "high" => body["quality"] = json!("medium"),
                _ => {}
            }
            body
        } else if self.is_gemini_model() {
            let mut body = json!({
                "model": self.configured_model(),
                "prompt": prompt,
                "n": 1,
                "response_format": "b64_json",
            });
            if size != "auto" {
                body["size"] = json!(size);
            } else if let Some(aspect) = self.resolve_gemini_aspect_ratio(size) {
                if self.is_google_openai_endpoint() {
                    if let Some(mapped) = match aspect {
                        "1:1" => Some("1024x1024"),
                        "3:2" => Some("1536x1024"),
                        "2:3" => Some("1024x1536"),
                        _ => None,
                    } {
                        body["size"] = json!(mapped);
                    }
                } else {
                    body["aspect_ratio"] = json!(aspect);
                }
            }
            if !self.is_google_openai_endpoint() {
                if let Some(resolution) = self.resolve_gemini_resolution() {
                    body["resolution"] = json!(resolution);
                }
            }
            body
        } else if crate::models::is_image_generation_model(self.configured_model()) {
            json!({
                "model": self.configured_model(),
                "prompt": prompt,
                "n": 1,
                "size": size,
                "quality": quality,
                "output_format": "png",
            })
        } else {
            // Custom aliases/future models need not support GPT-Image-specific
            // defaults. Send portable fields and only explicit size/quality.
            // Both URL and base64 responses are already converted to PNG.
            let mut body = json!({
                "model": self.configured_model(), "prompt": prompt, "n": 1,
            });
            if size != "auto" {
                body["size"] = json!(size);
            }
            if quality != "auto" {
                body["quality"] = json!(quality);
            }
            body
        }
    }

    fn endpoint(&self) -> String {
        format!(
            "{}/images/generations",
            self.api_root().trim_end_matches('/')
        )
    }

    fn chat_completions_endpoint(&self) -> String {
        format!("{}/chat/completions", self.api_root().trim_end_matches('/'))
    }

    fn model_endpoint(&self) -> Result<String, String> {
        let mut url = reqwest::Url::parse(&self.models_endpoint())
            .map_err(|error| format!("invalid image API URL: {error}"))?;
        url.path_segments_mut()
            .map_err(|_| "image API URL cannot contain path segments".to_string())?
            .push(self.configured_model());
        Ok(url.into())
    }

    fn models_endpoint(&self) -> String {
        format!("{}/models", self.api_root().trim_end_matches('/'))
    }

    fn gemini_models_endpoint(&self) -> String {
        format!("{}/models", self.gemini_api_root().trim_end_matches('/'))
    }

    fn gemini_model_endpoint(&self) -> Result<String, String> {
        let mut url = reqwest::Url::parse(&self.gemini_models_endpoint())
            .map_err(|error| format!("invalid Gemini API URL: {error}"))?;
        url.path_segments_mut()
            .map_err(|_| "Gemini API URL cannot contain path segments".to_string())?
            .push(self.gemini_model_id());
        Ok(url.into())
    }

    fn gemini_generate_endpoint(&self) -> Result<String, String> {
        let mut url = reqwest::Url::parse(&self.gemini_models_endpoint())
            .map_err(|error| format!("invalid Gemini API URL: {error}"))?;
        url.path_segments_mut()
            .map_err(|_| "Gemini API URL cannot contain path segments".to_string())?
            .push(&format!("{}:generateContent", self.gemini_model_id()));
        Ok(url.into())
    }

    fn client(&self) -> Result<reqwest::Client, String> {
        let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(300));
        if self.options.send_user_agent {
            builder = builder.user_agent(wisp_llm::provider::effective_user_agent(
                &self.options.user_agent,
            ));
        }
        match self.proxy.as_deref().map(str::trim) {
            None | Some("") => {}
            Some("none") => builder = builder.no_proxy(),
            Some(proxy) => {
                builder = builder.proxy(
                    reqwest::Proxy::all(proxy)
                        .map_err(|error| format!("invalid image-generation proxy: {error}"))?,
                );
            }
        }
        builder
            .build()
            .map_err(|error| format!("image-generation HTTP client: {error}"))
    }

    async fn read_bounded_body(
        response: &mut reqwest::Response,
        limit: usize,
        context: &str,
    ) -> Result<Vec<u8>, String> {
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("{context} response failed: {error}"))?
        {
            if body.len().saturating_add(chunk.len()) > limit {
                return Err(format!("{context} response is too large"));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    fn format_api_error(
        status: reqwest::StatusCode,
        request_id: &str,
        body: &[u8],
        label: &str,
    ) -> String {
        let message = serde_json::from_slice::<Value>(body)
            .ok()
            .and_then(|value| {
                value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| String::from_utf8_lossy(&body[..body.len().min(2_048)]).into());
        let request_id = (!request_id.is_empty())
            .then(|| format!(" (request {request_id})"))
            .unwrap_or_default();
        format!(
            "{label} returned {}{request_id}: {message}",
            status.as_u16()
        )
    }

    async fn finalize_image(
        &self,
        client: &reqwest::Client,
        source: ImageSource,
    ) -> Result<Vec<u8>, String> {
        let image = match source {
            ImageSource::Base64(encoded) => decode_image_base64(&encoded)?,
            ImageSource::Url(url) => self.download_image(client, &url).await?,
        };
        if image.len() > MAX_IMAGE_BYTES {
            return Err("generated image is too large".into());
        }
        let image = wisp_tools::image::encode_as_png(&image)?;
        if image.len() > MAX_IMAGE_BYTES {
            return Err("generated PNG is too large".into());
        }
        if !image.starts_with(PNG_SIGNATURE) {
            return Err("image-generation response is not a PNG".into());
        }
        Ok(image)
    }

    async fn generate_gemini(
        &self,
        client: &reqwest::Client,
        prompt: &str,
        size: &str,
    ) -> Result<Vec<u8>, String> {
        let endpoint = self.gemini_generate_endpoint()?;
        let mut response = self
            .request_headers(client.post(endpoint))
            .header("x-goog-api-key", self.api_key.trim())
            .json(&self.gemini_request_body(prompt, size))
            .send()
            .await
            .map_err(|error| format!("image-generation request failed: {error}"))?;
        let status = response.status();
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body_limit = if status.is_success() {
            MAX_BASE64_BYTES + MAX_ERROR_BYTES
        } else {
            MAX_ERROR_BYTES
        };
        let body = Self::read_bounded_body(&mut response, body_limit, "image-generation").await?;
        if !status.is_success() {
            return Err(Self::format_api_error(
                status,
                &request_id,
                &body,
                "image API",
            ));
        }
        let value: Value = serde_json::from_slice(&body)
            .map_err(|error| format!("invalid image-generation response: {error}"))?;
        let source = image_source_from_gemini_response(&value)?;
        self.finalize_image(client, source).await
    }

    async fn generate_openai_chat(
        &self,
        client: &reqwest::Client,
        prompt: &str,
        size: &str,
    ) -> Result<Vec<u8>, String> {
        let mut response = self
            .request_headers(client.post(self.chat_completions_endpoint()))
            .bearer_auth(self.api_key.trim())
            .json(&self.chat_completions_image_body(prompt, size))
            .send()
            .await
            .map_err(|error| format!("image-generation request failed: {error}"))?;
        let status = response.status();
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body_limit = if status.is_success() {
            MAX_BASE64_BYTES + MAX_ERROR_BYTES
        } else {
            MAX_ERROR_BYTES
        };
        let body = Self::read_bounded_body(&mut response, body_limit, "image-generation").await?;
        if !status.is_success() {
            return Err(Self::format_api_error(
                status,
                &request_id,
                &body,
                "image API",
            ));
        }
        let value: Value = serde_json::from_slice(&body)
            .map_err(|error| format!("invalid image-generation response: {error}"))?;
        let source = image_source_from_chat_response(&value)?;
        self.finalize_image(client, source).await
    }

    async fn generate(&self, prompt: &str, size: &str, quality: &str) -> Result<Vec<u8>, String> {
        if self.configured_model().is_empty() {
            return Err("image-generation model ID is required".into());
        }
        if self.api_key.trim().is_empty() {
            return Err("the assigned image-generation model has no API key".into());
        }
        let client = self.client()?;
        if self.is_gemini_protocol() {
            return self.generate_gemini(&client, prompt, size).await;
        }
        if self.is_gemini_model()
            && self
                .api_url
                .trim()
                .trim_end_matches('/')
                .ends_with("/chat/completions")
        {
            return self.generate_openai_chat(&client, prompt, size).await;
        }
        let mut response = self
            .request_headers(client.post(self.endpoint()))
            .bearer_auth(self.api_key.trim())
            .json(&self.request_body(prompt, size, quality))
            .send()
            .await
            .map_err(|error| format!("image-generation request failed: {error}"))?;
        let status = response.status();
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body_limit = if status.is_success() {
            MAX_BASE64_BYTES + MAX_ERROR_BYTES
        } else {
            MAX_ERROR_BYTES
        };
        let body = Self::read_bounded_body(&mut response, body_limit, "image-generation").await?;
        if !status.is_success() {
            if self.is_gemini_model()
                && matches!(
                    status,
                    reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED
                )
            {
                return self.generate_openai_chat(&client, prompt, size).await;
            }
            return Err(Self::format_api_error(
                status,
                &request_id,
                &body,
                "image API",
            ));
        }
        let value: Value = serde_json::from_slice(&body)
            .map_err(|error| format!("invalid image-generation response: {error}"))?;
        let source = image_source_from_response(&value)?;
        self.finalize_image(&client, source).await
    }

    async fn download_image(&self, client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
        let url = url.trim();
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err("image-generation response URL is not http(s)".into());
        }
        let mut response = client
            .get(url)
            .send()
            .await
            .map_err(|error| format!("image-generation download failed: {error}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "image-generation download returned {}",
                response.status().as_u16()
            ));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("image-generation download failed: {error}"))?
        {
            if body.len().saturating_add(chunk.len()) > MAX_IMAGE_BYTES {
                return Err("generated image is too large".into());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    async fn validate_gemini_model_access(&self) -> Result<(), String> {
        let expected = self.gemini_model_id();
        let client = self.client()?;
        for list_fallback in [false, true] {
            let endpoint = if list_fallback {
                self.gemini_models_endpoint()
            } else {
                self.gemini_model_endpoint()?
            };
            let mut response = self
                .request_headers(client.get(endpoint))
                .header("x-goog-api-key", self.api_key.trim())
                .send()
                .await
                .map_err(|error| format!("image-generation validation failed: {error}"))?;
            let status = response.status();
            let body = Self::read_bounded_body(
                &mut response,
                MAX_ERROR_BYTES,
                "image-generation validation",
            )
            .await?;
            if !status.is_success() {
                if !list_fallback
                    && matches!(
                        status,
                        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED
                    )
                {
                    continue;
                }
                return Err(Self::format_api_error(status, "", &body, "image model API"));
            }
            let value: Value = serde_json::from_slice(&body)
                .map_err(|error| format!("invalid image model response: {error}"))?;
            if list_fallback {
                let found_in_models =
                    value
                        .get("models")
                        .and_then(Value::as_array)
                        .is_some_and(|models| {
                            models.iter().any(|model| {
                                model
                                    .get("name")
                                    .or_else(|| model.get("id"))
                                    .and_then(Value::as_str)
                                    .is_some_and(|id| same_model_id(id, expected))
                            })
                        });
                let found_in_data =
                    value
                        .get("data")
                        .and_then(Value::as_array)
                        .is_some_and(|models| {
                            models.iter().any(|model| {
                                model
                                    .get("id")
                                    .or_else(|| model.get("name"))
                                    .and_then(Value::as_str)
                                    .is_some_and(|id| same_model_id(id, expected))
                            })
                        });
                if !found_in_models && !found_in_data {
                    return Err(format!("model list does not include {expected}"));
                }
            } else {
                let id = value
                    .get("name")
                    .or_else(|| value.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !same_model_id(id, expected) {
                    return Err(format!(
                        "provider returned model '{}' while validating {expected}",
                        if id.is_empty() { "(missing)" } else { id }
                    ));
                }
            }
            return Ok(());
        }
        unreachable!("model validation always returns from one of its two probes")
    }

    /// Validate credentials and access without creating a billable image.
    ///
    /// Image-only models cannot be sent to Responses or Chat Completions. The
    /// provider model metadata route provides a lightweight authenticated probe.
    pub async fn validate_model_access(&self) -> Result<(), String> {
        if self.configured_model().is_empty() {
            return Err("image-generation model ID is required".into());
        }
        if self.api_key.trim().is_empty() {
            return Err("the assigned image-generation model has no API key".into());
        }
        if self.is_gemini_protocol() {
            return self.validate_gemini_model_access().await;
        }
        let expected = self.configured_model();
        let client = self.client()?;
        for list_fallback in [false, true] {
            let endpoint = if list_fallback {
                self.models_endpoint()
            } else {
                self.model_endpoint()?
            };
            let mut response = self
                .request_headers(client.get(endpoint))
                .bearer_auth(self.api_key.trim())
                .send()
                .await
                .map_err(|error| format!("image-generation validation failed: {error}"))?;
            let status = response.status();
            let body = Self::read_bounded_body(
                &mut response,
                MAX_ERROR_BYTES,
                "image-generation validation",
            )
            .await?;
            if !status.is_success() {
                if !list_fallback
                    && matches!(
                        status,
                        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED
                    )
                {
                    continue;
                }
                return Err(Self::format_api_error(status, "", &body, "image model API"));
            }
            let value: Value = serde_json::from_slice(&body)
                .map_err(|error| format!("invalid image model response: {error}"))?;
            if list_fallback {
                let found = value
                    .get("data")
                    .and_then(Value::as_array)
                    .is_some_and(|models| {
                        models.iter().any(|model| {
                            model
                                .get("id")
                                .and_then(Value::as_str)
                                .is_some_and(|id| same_model_id(id, expected))
                        })
                    });
                if !found {
                    return Err(format!("model list does not include {expected}"));
                }
            } else {
                let id = value.get("id").and_then(Value::as_str).unwrap_or_default();
                if !same_model_id(id, expected) {
                    return Err(format!(
                        "provider returned model '{}' while validating {expected}",
                        if id.is_empty() { "(missing)" } else { id }
                    ));
                }
            }
            return Ok(());
        }
        unreachable!("model validation always returns from one of its two probes")
    }
}

#[async_trait]
impl Tool for GenerateImageTool {
    fn name(&self) -> &str {
        "generate_image"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "generate_image",
            "Generate one PNG with the explicitly configured image model through the configured image-generation API (OpenAI-compatible or Gemini), preserving the configured model ID, and save it inside the project. This is the Scientific Illustrator's PNG image-model mode. Call it when the user explicitly asks for PNG, generate_image, or image-model generation. Also call it for a Scientific Illustrator image request that names no format or method, because the presence of this tool means an image-generation model is configured. Do not call it when the user explicitly asks for SVG, vector, an editable figure, or direct SVG generation; create and visually verify that SVG directly instead. Use a project-relative path under figures/.",
            json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "Complete self-contained visual brief derived from the user's request and relevant project context"
                    },
                    "path": {
                        "type": "string",
                        "description": "Project-relative output path in the form figures/<descriptive-name>.png"
                    },
                    "size": {
                        "type": "string",
                        "enum": ["auto", "1024x1024", "1536x1024", "1024x1536"],
                        "description": "Output dimensions (default auto)"
                    },
                    "quality": {
                        "type": "string",
                        "enum": ["auto", "low", "medium", "high"],
                        "description": "Rendering quality (default auto)"
                    }
                },
                "required": ["prompt", "path"]
            }),
        )
    }

    fn preview(&self, args: &Value) -> String {
        args.get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        let prompt = args
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if prompt.is_empty() {
            return ToolResult::fail("generate_image error: prompt is required");
        }
        if prompt.len() > MAX_PROMPT_BYTES {
            return ToolResult::fail(format!(
                "generate_image error: prompt exceeds {MAX_PROMPT_BYTES} bytes"
            ));
        }
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if !path
            .rsplit_once('.')
            .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("png"))
        {
            return ToolResult::fail("generate_image error: path must end in .png");
        }
        if let Err(error) = wisp_tools::safety::validate_relative_pattern(path) {
            return ToolResult::fail(format!("generate_image {path} error: {error}"));
        }
        if std::path::Path::new(path).parent() != Some(std::path::Path::new("figures")) {
            return ToolResult::fail(
                "generate_image error: path must be a file directly under figures/",
            );
        }
        if let Err(error) = std::fs::create_dir_all(env.project_root().join("figures")) {
            return ToolResult::fail(format!(
                "generate_image {path} error: cannot create figures directory: {error}"
            ));
        }
        let real = match wisp_tools::safety::validate_file_path(env.project_root(), path) {
            Ok(path) => path,
            Err(error) => {
                return ToolResult::fail(format!("generate_image {path} error: {error}"));
            }
        };
        let size = args.get("size").and_then(Value::as_str).unwrap_or("auto");
        if !matches!(size, "auto" | "1024x1024" | "1536x1024" | "1024x1536") {
            return ToolResult::fail("generate_image error: unsupported size");
        }
        let quality = args
            .get("quality")
            .and_then(Value::as_str)
            .unwrap_or("auto");
        if !matches!(quality, "auto" | "low" | "medium" | "high") {
            return ToolResult::fail("generate_image error: unsupported quality");
        }
        let image = match self.generate(prompt, size, quality).await {
            Ok(image) => image,
            Err(error) => return ToolResult::fail(format!("generate_image error: {error}")),
        };
        if let Err(error) = std::fs::write(&real, &image) {
            return ToolResult::fail(format!("generate_image {path} error: {error}"));
        }
        env.emit(ToolEvent::FileChanged { path: path.into() }).await;
        ToolResult::ok(format!(
            "Generated {} byte PNG at {path}. Include it in the final answer as a Markdown image.",
            image.len()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn schema_keeps_explicit_svg_requests_out_of_raster_mode() {
        let tool = GenerateImageTool::new(
            "https://api.openai.com/v1".into(),
            "sk-test".into(),
            "gpt-image-2".into(),
            Some("none".into()),
        );
        let description = tool.schema().function.description;

        assert!(description.contains("Do not call it when the user explicitly asks for SVG"));
        assert!(description.contains("explicitly asks for PNG"));
        assert!(description.contains("names no format or method"));
        assert!(description.contains("visually verify that SVG directly"));
    }

    struct RecordingEnv {
        root: PathBuf,
        events: Mutex<Vec<ToolEvent>>,
    }

    #[async_trait]
    impl ToolEnv for RecordingEnv {
        fn project_root(&self) -> &Path {
            &self.root
        }

        async fn confirm(&self, _message: &str) -> bool {
            true
        }

        async fn emit(&self, event: ToolEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    async fn serve_once(response_body: String) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let header_end = loop {
                let mut chunk = [0u8; 4096];
                let read = stream.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                request.extend_from_slice(&chunk[..read]);
                if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or_default();
            while request.len() < header_end + content_length {
                let mut chunk = [0u8; 4096];
                let read = stream.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                request.extend_from_slice(&chunk[..read]);
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(request).unwrap()
        });
        (format!("http://{address}/v1"), handle)
    }

    async fn serve_model_fallback(
        response_body: String,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let mut requests = Vec::with_capacity(2);
            for (status, body) in [(404, "404 page not found".into()), (200, response_body)] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0u8; 4096];
                    let read = stream.read(&mut chunk).await.unwrap();
                    assert!(read > 0);
                    request.extend_from_slice(&chunk[..read]);
                    if request.windows(4).any(|part| part == b"\r\n\r\n") {
                        break;
                    }
                }
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).await.unwrap();
                requests.push(String::from_utf8(request).unwrap());
            }
            requests
        });
        (format!("http://{address}/v1"), handle)
    }

    #[tokio::test]
    async fn generates_png_with_the_openai_image_endpoint() {
        let encoded = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let response = json!({"data": [{"b64_json": encoded}]}).to_string();
        let (api_url, request) = serve_once(response).await;
        let root =
            std::env::temp_dir().join(format!("wisp_generate_image_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let env = RecordingEnv {
            root: root.clone(),
            events: Mutex::new(Vec::new()),
        };
        let result = GenerateImageTool::new(
            api_url,
            "sk-test".into(),
            "gpt-image-2".into(),
            Some("none".into()),
        )
        .with_options(crate::models::ImageGenerationOptions {
            send_user_agent: false,
            send_session_id: Some(true),
            session_header_name: "x-custom-session".into(),
            ..Default::default()
        })
        .with_session_id("frame-image")
        .run(
            &json!({
                "prompt": "A precise scientific pathway diagram",
                "path": "figures/pathway.png",
                "size": "1536x1024",
                "quality": "low"
            }),
            &env,
        )
        .await;

        assert!(result.success, "{}", result.content);
        assert!(std::fs::read(root.join("figures/pathway.png"))
            .unwrap()
            .starts_with(PNG_SIGNATURE));
        assert!(env.events.lock().unwrap().iter().any(
            |event| matches!(event, ToolEvent::FileChanged { path } if path == "figures/pathway.png")
        ));
        let request = request.await.unwrap();
        assert!(request.starts_with("POST /v1/images/generations HTTP/1.1"));
        assert!(!request.to_ascii_lowercase().contains("user-agent:"));
        assert!(request.contains("x-custom-session: frame-image"));
        let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["model"], "gpt-image-2");
        assert_eq!(body["output_format"], "png");
        assert_eq!(body["size"], "1536x1024");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn validation_honors_identity_controls_and_custom_session_name() {
        let (api_url, captured) = serve_once(json!({"id": "gpt-image-2"}).to_string()).await;
        GenerateImageTool::new(
            api_url,
            "fake-key".into(),
            "gpt-image-2".into(),
            Some("none".into()),
        )
        .with_options(crate::models::ImageGenerationOptions {
            send_user_agent: false,
            send_session_id: Some(true),
            session_header_name: "x-custom-session".into(),
            ..Default::default()
        })
        .with_session_id("frame-media")
        .validate_model_access()
        .await
        .unwrap();
        let request = captured.await.unwrap();
        assert!(!request.to_ascii_lowercase().contains("user-agent:"));
        assert!(request.contains("x-custom-session: frame-media"));
        assert!(!request.contains("x-opencode-session:"));
    }

    #[tokio::test]
    async fn validates_with_the_model_endpoint_instead_of_a_chat_endpoint() {
        let response = json!({"id": "gpt-image-2", "object": "model"}).to_string();
        let (api_url, request) = serve_once(response).await;

        GenerateImageTool::new(
            api_url,
            "sk-test".into(),
            "gpt-image-2".into(),
            Some("none".into()),
        )
        .with_options(crate::models::ImageGenerationOptions {
            user_agent: "research-client/1.0".into(),
            ..Default::default()
        })
        .validate_model_access()
        .await
        .unwrap();

        let request = request.await.unwrap();
        assert!(request.starts_with("GET /v1/models/gpt-image-2 HTTP/1.1"));
        assert!(request.contains("user-agent: research-client/1.0"));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-test"));
        assert!(!request.contains("/responses"));
        assert!(!request.contains("/chat/completions"));
    }

    #[tokio::test]
    async fn falls_back_to_the_model_list_when_model_lookup_is_missing() {
        let response = json!({
            "object": "list",
            "data": [{"id": "gpt-image-2", "object": "model"}]
        })
        .to_string();
        let (api_url, requests) = serve_model_fallback(response).await;

        GenerateImageTool::new(
            api_url,
            "sk-test".into(),
            "gpt-image-2".into(),
            Some("none".into()),
        )
        .validate_model_access()
        .await
        .unwrap();

        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("GET /v1/models/gpt-image-2 HTTP/1.1"));
        assert!(requests[1].starts_with("GET /v1/models HTTP/1.1"));
        assert!(requests.iter().all(|request| request
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-test")));
    }

    #[tokio::test]
    async fn rejects_non_png_paths_before_calling_the_api() {
        let root =
            std::env::temp_dir().join(format!("wisp_generate_image_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let env = RecordingEnv {
            root: root.clone(),
            events: Mutex::new(Vec::new()),
        };
        let result = GenerateImageTool::new(
            "https://api.openai.com/v1".into(),
            "sk-test".into(),
            "gpt-image-2".into(),
            Some("none".into()),
        )
        .run(
            &json!({"prompt": "diagram", "path": "figures/pathway.svg"}),
            &env,
        )
        .await;
        assert!(!result.success);
        assert!(result.content.contains("path must end in .png"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn rejects_paths_outside_the_figures_directory() {
        let root =
            std::env::temp_dir().join(format!("wisp_generate_image_path_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let env = RecordingEnv {
            root: root.clone(),
            events: Mutex::new(Vec::new()),
        };
        let tool = GenerateImageTool::new(
            "http://127.0.0.1:9/v1".into(),
            "sk-test".into(),
            "gpt-image-2".into(),
            Some("none".into()),
        );

        for path in ["plot.png", "../figures/plot.png", "figures/sub/plot.png"] {
            let result = tool
                .run(&json!({"prompt": "diagram", "path": path}), &env)
                .await;
            assert!(!result.success, "{path} should be rejected");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn grok_request_uses_aspect_ratio_and_b64() {
        let tool = GenerateImageTool::new(
            "https://api.x.ai".into(),
            "xai-test".into(),
            "grok-imagine-image-2.0".into(),
            Some("none".into()),
        );
        assert_eq!(tool.api_root(), "https://api.x.ai/v1");
        let body = tool.request_body("diagram", "1536x1024", "low");
        assert_eq!(body["model"], "grok-imagine-image-2.0");
        assert_eq!(body["aspect_ratio"], "3:2");
        assert_eq!(body["resolution"], "1k");
        assert_eq!(body["quality"], "low");
        assert_eq!(body["response_format"], "b64_json");
        assert!(body.get("output_format").is_none());
        assert!(body.get("size").is_none());

        let auto = tool.request_body("diagram", "auto", "auto");
        assert_eq!(auto["aspect_ratio"], "auto");
        assert!(auto.get("resolution").is_none());
        assert!(auto.get("quality").is_none());

        let high = tool.request_body("diagram", "1024x1024", "high");
        assert_eq!(high["quality"], "medium");
        assert_eq!(high["aspect_ratio"], "1:1");
    }

    #[test]
    fn grok_request_uses_profile_defaults_when_tool_says_auto() {
        let tool = GenerateImageTool::new(
            "https://api.x.ai".into(),
            "xai-test".into(),
            "grok-imagine-image-2.0".into(),
            Some("none".into()),
        )
        .with_options(crate::models::ImageGenerationOptions {
            provider: String::new(),
            user_agent: String::new(),
            send_user_agent: true,
            send_session_id: None,
            session_header_name: String::new(),
            size: String::new(),
            quality: "low".into(),
            aspect_ratio: "16:9".into(),
            resolution: "2k".into(),
        });
        let body = tool.request_body("diagram", "auto", "auto");
        assert_eq!(body["aspect_ratio"], "16:9");
        assert_eq!(body["resolution"], "2k");
        assert_eq!(body["quality"], "low");
    }

    #[test]
    fn openai_request_uses_profile_defaults_when_tool_says_auto() {
        let tool = GenerateImageTool::new(
            "https://api.openai.com".into(),
            "sk-test".into(),
            "gpt-image-2".into(),
            Some("none".into()),
        )
        .with_options(crate::models::ImageGenerationOptions {
            provider: String::new(),
            user_agent: String::new(),
            send_user_agent: true,
            send_session_id: None,
            session_header_name: String::new(),
            size: "1536x1024".into(),
            quality: "high".into(),
            aspect_ratio: String::new(),
            resolution: String::new(),
        });
        let body = tool.request_body("diagram", "auto", "auto");
        assert_eq!(body["size"], "1536x1024");
        assert_eq!(body["quality"], "high");
    }

    #[test]
    fn openai_request_keeps_size_and_png_output() {
        let tool = GenerateImageTool::new(
            "https://api.openai.com".into(),
            "sk-test".into(),
            "gpt-image-2".into(),
            Some("none".into()),
        );
        assert_eq!(tool.api_root(), "https://api.openai.com/v1");
        let body = tool.request_body("diagram", "1024x1536", "high");
        assert_eq!(body["model"], "gpt-image-2");
        assert_eq!(body["size"], "1024x1536");
        assert_eq!(body["quality"], "high");
        assert_eq!(body["output_format"], "png");
    }

    #[test]
    fn image_source_prefers_b64_then_url() {
        let b64 =
            image_source_from_response(&json!({"data":[{"b64_json":"abc","url":"https://x"}]}))
                .unwrap();
        assert!(matches!(b64, ImageSource::Base64(value) if value == "abc"));
        let url =
            image_source_from_response(&json!({"data":[{"url":" https://cdn.example/img "}]}))
                .unwrap();
        assert!(matches!(url, ImageSource::Url(value) if value == "https://cdn.example/img"));
        assert!(image_source_from_response(&json!({"data":[{}]})).is_err());
    }

    #[tokio::test]
    async fn generates_png_with_the_grok_image_endpoint() {
        let encoded = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let response = json!({"data": [{"b64_json": encoded}]}).to_string();
        let (api_url, request) = serve_once(response).await;
        let root =
            std::env::temp_dir().join(format!("wisp_generate_image_grok_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let env = RecordingEnv {
            root: root.clone(),
            events: Mutex::new(Vec::new()),
        };
        let result = GenerateImageTool::new(
            api_url,
            "xai-test".into(),
            "xai/grok-imagine-image-2.0".into(),
            Some("none".into()),
        )
        .run(
            &json!({
                "prompt": "A precise scientific pathway diagram",
                "path": "figures/pathway.png",
                "size": "1024x1536",
                "quality": "medium"
            }),
            &env,
        )
        .await;

        assert!(result.success, "{}", result.content);
        assert!(std::fs::read(root.join("figures/pathway.png"))
            .unwrap()
            .starts_with(PNG_SIGNATURE));
        let request = request.await.unwrap();
        assert!(request.starts_with("POST /v1/images/generations HTTP/1.1"));
        let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["model"], "xai/grok-imagine-image-2.0");
        assert_eq!(body["aspect_ratio"], "2:3");
        assert_eq!(body["resolution"], "1k");
        assert_eq!(body["response_format"], "b64_json");
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn validates_grok_with_the_model_endpoint() {
        let response = json!({"id": "grok-imagine-image-2.0", "object": "model"}).to_string();
        let (api_url, request) = serve_once(response).await;

        GenerateImageTool::new(
            api_url,
            "xai-test".into(),
            "grok-imagine-image-2.0".into(),
            Some("none".into()),
        )
        .validate_model_access()
        .await
        .unwrap();

        let request = request.await.unwrap();
        assert!(request.starts_with("GET /v1/models/grok-imagine-image-2.0 HTTP/1.1"));
    }

    #[tokio::test]
    async fn rejects_empty_image_models_before_calling_the_api() {
        let result = GenerateImageTool::new(
            "http://127.0.0.1:9/v1".into(),
            "sk-test".into(),
            "  ".into(),
            Some("none".into()),
        )
        .generate("diagram", "auto", "auto")
        .await;
        assert!(result.unwrap_err().contains("model ID is required"));
    }

    #[tokio::test]
    async fn custom_image_ids_reach_generations_unchanged_without_gpt_specific_defaults() {
        for model in ["gpt-image-2.5", "vendor/opaque-image-vNext", "dall-e-3"] {
            let encoded = base64::engine::general_purpose::STANDARD.encode(PNG_SIGNATURE);
            let (url, request) =
                serve_once(json!({"data":[{"b64_json":encoded}]}).to_string()).await;
            GenerateImageTool::new(url, "fake-key".into(), model.into(), Some("none".into()))
                .generate("test diagram", "auto", "auto")
                .await
                .unwrap();
            let request = request.await.unwrap();
            assert!(request.starts_with("POST /v1/images/generations HTTP/1.1"));
            let body: Value =
                serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(body, json!({"model":model,"prompt":"test diagram","n":1}));
        }
    }

    #[tokio::test]
    async fn custom_image_validation_escapes_model_id_and_never_generates_an_image() {
        let model = "vendor/custom-image?revision=3";
        let (url, request) = serve_once(json!({"id":model}).to_string()).await;
        GenerateImageTool::new(url, "fake-key".into(), model.into(), Some("none".into()))
            .validate_model_access()
            .await
            .unwrap();
        let request = request.await.unwrap();
        assert!(request.starts_with("GET /v1/models/vendor%2Fcustom-image%3Frevision=3 HTTP/1.1"));
        assert!(!request.contains("/images/generations"));
        assert!(!request.contains("/chat/completions"));
    }

    async fn serve_sequence(
        responses: Vec<(u16, String)>,
        base_path: &str,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let mut requests = Vec::with_capacity(responses.len());
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let header_end = loop {
                    let mut chunk = [0u8; 4096];
                    let read = stream.read(&mut chunk).await.unwrap();
                    assert!(read > 0);
                    request.extend_from_slice(&chunk[..read]);
                    if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or_default();
                while request.len() < header_end + content_length {
                    let mut chunk = [0u8; 4096];
                    let read = stream.read(&mut chunk).await.unwrap();
                    assert!(read > 0);
                    request.extend_from_slice(&chunk[..read]);
                }
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).await.unwrap();
                requests.push(String::from_utf8(request).unwrap());
            }
            requests
        });
        (format!("http://{address}{base_path}"), handle)
    }

    #[test]
    fn gemini_api_root_and_request_body_handle_urls_and_image_config() {
        for (input, expected) in [
            (
                "https://generativelanguage.googleapis.com",
                "https://generativelanguage.googleapis.com/v1beta",
            ),
            (
                "https://generativelanguage.googleapis.com/v1beta/",
                "https://generativelanguage.googleapis.com/v1beta",
            ),
            (
                "https://generativelanguage.googleapis.com/v1beta/openai",
                "https://generativelanguage.googleapis.com/v1beta",
            ),
            (
                "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-image:generateContent",
                "https://generativelanguage.googleapis.com/v1beta",
            ),
            (
                "https://relay.example.com/gemini",
                "https://relay.example.com/gemini/v1beta",
            ),
        ] {
            let tool = GenerateImageTool::new(
                input.into(),
                "gm-test".into(),
                "gemini-2.5-flash-image".into(),
                Some("none".into()),
            )
            .with_options(crate::models::ImageGenerationOptions {
                provider: "gemini".into(),
                ..Default::default()
            });
            assert_eq!(tool.gemini_api_root(), expected, "{input}");
        }

        let tool = GenerateImageTool::new(
            "https://generativelanguage.googleapis.com".into(),
            "gm-test".into(),
            "models/gemini-3.1-flash-image-preview".into(),
            Some("none".into()),
        )
        .with_options(crate::models::ImageGenerationOptions {
            provider: "gemini".into(),
            aspect_ratio: "16:9".into(),
            resolution: "2K".into(),
            ..Default::default()
        });
        assert_eq!(
            tool.gemini_generate_endpoint().unwrap(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.1-flash-image-preview:generateContent"
        );
        let auto = tool.gemini_request_body("pathway diagram", "auto");
        assert_eq!(auto["contents"][0]["parts"][0]["text"], "pathway diagram");
        assert_eq!(
            auto["generationConfig"]["responseModalities"],
            json!(["TEXT", "IMAGE"])
        );
        assert_eq!(
            auto["generationConfig"]["imageConfig"]["aspectRatio"],
            "16:9"
        );
        assert_eq!(auto["generationConfig"]["imageConfig"]["imageSize"], "2K");

        let explicit = tool.gemini_request_body("pathway diagram", "1536x1024");
        assert_eq!(
            explicit["generationConfig"]["imageConfig"]["aspectRatio"],
            "3:2"
        );

        // With default auto aspect ratio and resolution, imageConfig is omitted so
        // models like gemini-2.5-flash-image never receive unsupported imageSize.
        let minimal = GenerateImageTool::new(
            "https://generativelanguage.googleapis.com".into(),
            "gm-test".into(),
            "gemini-2.5-flash-image".into(),
            Some("none".into()),
        )
        .with_options(crate::models::ImageGenerationOptions {
            provider: "gemini".into(),
            ..Default::default()
        })
        .gemini_request_body("cell schematic", "auto");
        assert!(minimal["generationConfig"].get("imageConfig").is_none());
    }

    #[test]
    fn gemini_response_prefers_final_non_thought_image_and_surfaces_blocks() {
        let with_thought = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        {"thought": true, "inlineData": {"mimeType": "image/png", "data": "ZHJhZnQ="}},
                        {"text": "Final schematic"},
                        {"inlineData": {"mimeType": "image/png", "data": "ZmluYWw="}}
                    ]
                }
            }]
        });
        let source = image_source_from_gemini_response(&with_thought).unwrap();
        assert!(matches!(source, ImageSource::Base64(data) if data == "ZmluYWw="));

        let blocked = json!({
            "promptFeedback": {"blockReason": "SAFETY"}
        });
        let err = image_source_from_gemini_response(&blocked).unwrap_err();
        assert!(err.contains("blockReason=SAFETY"), "{err}");
    }

    #[tokio::test]
    async fn generates_png_with_the_native_gemini_generate_content_endpoint() {
        let encoded = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let response = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        {"text": "Generated diagram"},
                        {"inlineData": {"mimeType": "image/png", "data": encoded}}
                    ]
                }
            }]
        })
        .to_string();
        let (api_url, requests) = serve_sequence(vec![(200, response)], "").await;
        let root = std::env::temp_dir().join(format!(
            "wisp_generate_image_gemini_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let env = RecordingEnv {
            root: root.clone(),
            events: Mutex::new(Vec::new()),
        };
        let result = GenerateImageTool::new(
            api_url,
            "gm-secret-key".into(),
            "gemini-2.5-flash-image".into(),
            Some("none".into()),
        )
        .with_options(crate::models::ImageGenerationOptions {
            provider: "gemini".into(),
            aspect_ratio: "16:9".into(),
            ..Default::default()
        })
        .run(
            &json!({
                "prompt": "A schematic of CRISPR-Cas9 cleavage",
                "path": "figures/crispr.png"
            }),
            &env,
        )
        .await;

        assert!(result.success, "{}", result.content);
        assert!(std::fs::read(root.join("figures/crispr.png"))
            .unwrap()
            .starts_with(PNG_SIGNATURE));
        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert!(
            request
                .starts_with("POST /v1beta/models/gemini-2.5-flash-image:generateContent HTTP/1.1"),
            "{request}"
        );
        assert!(request
            .to_ascii_lowercase()
            .contains("x-goog-api-key: gm-secret-key"));
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
        let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(
            body["generationConfig"]["imageConfig"]["aspectRatio"],
            "16:9"
        );
        assert!(body["generationConfig"]["imageConfig"]
            .get("imageSize")
            .is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn validates_native_gemini_with_model_metadata_and_list_fallback() {
        let (api_url, requests) = serve_sequence(
            vec![(
                200,
                json!({"name": "models/gemini-2.5-flash-image"}).to_string(),
            )],
            "",
        )
        .await;
        GenerateImageTool::new(
            api_url,
            "gm-key".into(),
            "gemini-2.5-flash-image".into(),
            Some("none".into()),
        )
        .with_options(crate::models::ImageGenerationOptions {
            provider: "gemini".into(),
            ..Default::default()
        })
        .validate_model_access()
        .await
        .unwrap();
        let requests = requests.await.unwrap();
        assert!(requests[0].starts_with("GET /v1beta/models/gemini-2.5-flash-image HTTP/1.1"));
        assert!(requests[0]
            .to_ascii_lowercase()
            .contains("x-goog-api-key: gm-key"));
        assert!(!requests[0].contains(":generateContent"));

        let (fallback_url, fallback_requests) = serve_sequence(
            vec![
                (404, "not found".into()),
                (
                    200,
                    json!({"models": [{"name": "models/gemini-3-pro-image-preview"}]}).to_string(),
                ),
            ],
            "",
        )
        .await;
        GenerateImageTool::new(
            fallback_url,
            "gm-key".into(),
            "gemini-3-pro-image-preview".into(),
            Some("none".into()),
        )
        .with_options(crate::models::ImageGenerationOptions {
            provider: "gemini".into(),
            ..Default::default()
        })
        .validate_model_access()
        .await
        .unwrap();
        let fallback_requests = fallback_requests.await.unwrap();
        assert_eq!(fallback_requests.len(), 2);
        assert!(fallback_requests[0]
            .starts_with("GET /v1beta/models/gemini-3-pro-image-preview HTTP/1.1"));
        assert!(fallback_requests[1].starts_with("GET /v1beta/models HTTP/1.1"));
    }

    #[tokio::test]
    async fn openai_compatible_gemini_falls_back_to_chat_completions_when_images_route_is_missing()
    {
        let encoded = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let chat_response = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Here is your figure.",
                    "images": [{
                        "type": "image_url",
                        "image_url": { "url": format!("data:image/png;base64,{encoded}") }
                    }]
                }
            }]
        })
        .to_string();
        let (api_url, requests) = serve_sequence(
            vec![(404, "not found".into()), (200, chat_response)],
            "/api/v1",
        )
        .await;
        let png = GenerateImageTool::new(
            api_url,
            "or-key".into(),
            "google/gemini-3.1-flash-image-preview".into(),
            Some("none".into()),
        )
        .with_options(crate::models::ImageGenerationOptions {
            provider: "openai".into(),
            aspect_ratio: "4:3".into(),
            resolution: "2K".into(),
            ..Default::default()
        })
        .generate("mitochondria cross section", "auto", "auto")
        .await
        .unwrap();
        assert!(png.starts_with(PNG_SIGNATURE));

        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("POST /api/v1/images/generations HTTP/1.1"));
        assert!(requests[1].starts_with("POST /api/v1/chat/completions HTTP/1.1"));
        let chat_body: Value =
            serde_json::from_str(requests[1].split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(chat_body["model"], "google/gemini-3.1-flash-image-preview");
        assert_eq!(chat_body["modalities"], json!(["image", "text"]));
        assert_eq!(chat_body["image_config"]["aspect_ratio"], "4:3");
        assert_eq!(chat_body["image_config"]["image_size"], "2K");
    }

    #[test]
    fn chat_response_extracts_markdown_data_uri_and_images_array() {
        let markdown = json!({
            "choices": [{
                "message": {
                    "content": "Rendered:\n![diagram](data:image/png;base64,QUJD)"
                }
            }]
        });
        let source = image_source_from_chat_response(&markdown).unwrap();
        assert!(matches!(source, ImageSource::Base64(data) if data == "QUJD"));

        let google_openai = GenerateImageTool::new(
            "https://generativelanguage.googleapis.com".into(),
            "gm-key".into(),
            "gemini-2.5-flash-image".into(),
            Some("none".into()),
        );
        assert_eq!(
            google_openai.api_root(),
            "https://generativelanguage.googleapis.com/v1beta/openai"
        );
        let body = google_openai.request_body("diagram", "1024x1024", "auto");
        assert_eq!(body["model"], "gemini-2.5-flash-image");
        assert_eq!(body["response_format"], "b64_json");
        assert_eq!(body["size"], "1024x1024");
        assert!(body.get("output_format").is_none());
    }
}

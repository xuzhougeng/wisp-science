//! Shared fixtures for the provider wire tests: a JSON request reader for the
//! loopback fake servers, and an SSE body builder that renders one event list
//! with either line ending and cuts it into network chunks at chosen offsets.

#![allow(dead_code)]

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Read one HTTP request with a JSON body. Returns the request head (request
/// line and headers) and the parsed body.
pub async fn read_json_request(socket: &mut TcpStream) -> (String, Value) {
    let mut request = Vec::new();
    let (header_end, content_length) = loop {
        let mut chunk = [0_u8; 4096];
        let read = socket.read(&mut chunk).await.unwrap();
        assert!(read > 0, "request ended before its headers");
        request.extend_from_slice(&chunk[..read]);
        if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            let end = end + 4;
            let head = String::from_utf8_lossy(&request[..end]);
            let length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .expect("JSON request must carry content-length");
            break (end, length);
        }
    };
    while request.len() < header_end + content_length {
        let mut chunk = [0_u8; 4096];
        let read = socket.read(&mut chunk).await.unwrap();
        assert!(read > 0, "request body ended early");
        request.extend_from_slice(&chunk[..read]);
    }
    let head = String::from_utf8_lossy(&request[..header_end]).into_owned();
    let body = serde_json::from_slice(&request[header_end..header_end + content_length]).unwrap();
    (head, body)
}

/// The request path from a head returned by [`read_json_request`].
pub fn request_path(head: &str) -> String {
    head.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap()
        .to_string()
}

/// Line terminator used when rendering an SSE body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    pub const ALL: [LineEnding; 2] = [LineEnding::Lf, LineEnding::CrLf];

    fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }
}

/// One server-sent event: an optional `event:` name and its `data:` payload.
#[derive(Debug, Clone)]
pub struct SseEvent {
    name: Option<String>,
    data: String,
}

impl SseEvent {
    /// `data: <payload>` with no event name (chat-completions, Responses).
    pub fn data(payload: impl ToString) -> Self {
        Self {
            name: None,
            data: payload.to_string(),
        }
    }

    /// `event: <name>` + `data: <payload>` (Anthropic).
    pub fn named(name: &str, payload: impl ToString) -> Self {
        Self {
            name: Some(name.to_string()),
            data: payload.to_string(),
        }
    }
}

/// Render events as an SSE body: every line ends with `eol`, and every event
/// is followed by a blank line (`\n\n` or `\r\n\r\n` between events).
pub fn render_sse(events: &[SseEvent], eol: LineEnding) -> String {
    let eol = eol.as_str();
    let mut body = String::new();
    for event in events {
        if let Some(name) = &event.name {
            body.push_str(&format!("event: {name}{eol}"));
        }
        body.push_str(&format!("data: {}{eol}{eol}", event.data));
    }
    body
}

/// Cut `body` into chunks at the given byte offsets (any order, duplicates and
/// out-of-range values ignored). Offsets may fall inside a multi-byte
/// character or a separator; that is the point.
pub fn split_at(body: &str, offsets: &[usize]) -> Vec<Vec<u8>> {
    let bytes = body.as_bytes();
    let mut cuts: Vec<usize> = offsets
        .iter()
        .copied()
        .filter(|offset| *offset > 0 && *offset < bytes.len())
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut chunks = Vec::with_capacity(cuts.len() + 1);
    let mut start = 0;
    for cut in cuts {
        chunks.push(bytes[start..cut].to_vec());
        start = cut;
    }
    chunks.push(bytes[start..].to_vec());
    chunks
}

/// Byte offsets one byte into every event separator of a body rendered with
/// `eol`, so each separator straddles two chunks: between the two `\n` for LF,
/// between the first `\r` and its `\n` for CRLF.
pub fn separator_offsets(body: &str, eol: LineEnding) -> Vec<usize> {
    let separator = eol.as_str().repeat(2);
    body.match_indices(&separator)
        .map(|(index, _)| index + 1)
        .collect()
}

/// Every chunking the stream parsers must treat identically: the whole body,
/// each separator straddling two chunks, and a fixed small stride that lands
/// inside lines, separators and multi-byte characters.
pub fn chunkings(body: &str, eol: LineEnding) -> Vec<(&'static str, Vec<Vec<u8>>)> {
    let stride: Vec<usize> = (1..body.len()).filter(|offset| offset % 7 == 0).collect();
    vec![
        ("whole body", split_at(body, &[])),
        (
            "split separators",
            split_at(body, &separator_offsets(body, eol)),
        ),
        ("7-byte chunks", split_at(body, &stride)),
    ]
}

/// Serve one `text/event-stream` response on a loopback port, writing each
/// chunk as its own HTTP chunk and flushing between them so the client's byte
/// stream sees the same boundaries. Returns the base URL and a handle that
/// yields the captured request head and JSON body.
pub async fn serve_sse(chunks: Vec<Vec<u8>>) -> (String, tokio::task::JoinHandle<(String, Value)>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        socket.set_nodelay(true).unwrap();
        let request = read_json_request(&mut socket).await;
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        for chunk in chunks.iter().filter(|chunk| !chunk.is_empty()) {
            let mut frame = format!("{:x}\r\n", chunk.len()).into_bytes();
            frame.extend_from_slice(chunk);
            frame.extend_from_slice(b"\r\n");
            // The client may stop reading early (an in-band error); that is
            // not a server-side test failure.
            if socket.write_all(&frame).await.is_err() || socket.flush().await.is_err() {
                return request;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        let _ = socket.write_all(b"0\r\n\r\n").await;
        let _ = socket.shutdown().await;
        request
    });
    (format!("http://{address}"), task)
}

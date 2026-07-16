//! Feishu long-connection pbbp2 frame codec.
//!
//! The wire protocol has no public spec; field numbers come from the official
//! `larksuite/oapi-sdk-go` generated code (one protobuf message, hand-decoded
//! here to avoid a prost dependency). Payloads are plaintext JSON.
//!
//! Frame { 1 SeqID varint; 2 LogID varint; 3 service varint; 4 method varint
//! (0=Control, 1=Data); 5 headers repeated Header{1 key, 2 value}; 6
//! payload_encoding str; 7 payload_type str; 8 payload bytes; 9 logid_new str }

#[derive(Debug, Default, Clone)]
pub struct Frame {
    pub seqid: u64,
    pub logid: u64,
    pub service: i64,
    pub method: i64,
    pub headers: Vec<(String, String)>,
    pub payload_encoding: String,
    pub payload_type: String,
    pub payload: Vec<u8>,
    pub logid_new: String,
}

impl Frame {
    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, PartialEq)]
pub enum DecodeError {
    Truncated,
    BadWireType,
}

fn read_varint(buf: &[u8], i: &mut usize) -> Result<u64, DecodeError> {
    let mut result: u64 = 0;
    let mut shift: u32 = 0;
    loop {
        let b = *buf.get(*i).ok_or(DecodeError::Truncated)?;
        *i += 1;
        result |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
        if shift >= 64 {
            return Err(DecodeError::Truncated);
        }
    }
}

fn read_len_delim<'a>(buf: &'a [u8], i: &mut usize) -> Result<&'a [u8], DecodeError> {
    let len = read_varint(buf, i)? as usize;
    let end = i.checked_add(len).ok_or(DecodeError::Truncated)?;
    if end > buf.len() {
        return Err(DecodeError::Truncated);
    }
    let out = &buf[*i..end];
    *i = end;
    Ok(out)
}

fn skip_field(buf: &[u8], i: &mut usize, wire: u8) -> Result<(), DecodeError> {
    match wire {
        0 => {
            read_varint(buf, i)?;
        }
        1 => *i += 8,
        2 => {
            read_len_delim(buf, i)?;
        }
        5 => *i += 4,
        _ => return Err(DecodeError::BadWireType),
    }
    Ok(())
}

fn parse_header(sub: &[u8]) -> Result<(String, String), DecodeError> {
    let (mut key, mut value) = (String::new(), String::new());
    let mut j = 0usize;
    while j < sub.len() {
        let tag = read_varint(sub, &mut j)?;
        let wire = (tag & 0x7) as u8;
        match tag >> 3 {
            1 => key = String::from_utf8_lossy(read_len_delim(sub, &mut j)?).into_owned(),
            2 => value = String::from_utf8_lossy(read_len_delim(sub, &mut j)?).into_owned(),
            _ => skip_field(sub, &mut j, wire)?,
        }
    }
    Ok((key, value))
}

pub fn decode(buf: &[u8]) -> Result<Frame, DecodeError> {
    let mut f = Frame::default();
    let mut i = 0usize;
    while i < buf.len() {
        let tag = read_varint(buf, &mut i)?;
        let wire = (tag & 0x7) as u8;
        match tag >> 3 {
            1 => f.seqid = read_varint(buf, &mut i)?,
            2 => f.logid = read_varint(buf, &mut i)?,
            3 => f.service = read_varint(buf, &mut i)? as i64,
            4 => f.method = read_varint(buf, &mut i)? as i64,
            5 => f.headers.push(parse_header(read_len_delim(buf, &mut i)?)?),
            6 => {
                f.payload_encoding =
                    String::from_utf8_lossy(read_len_delim(buf, &mut i)?).into_owned()
            }
            7 => {
                f.payload_type = String::from_utf8_lossy(read_len_delim(buf, &mut i)?).into_owned()
            }
            8 => f.payload = read_len_delim(buf, &mut i)?.to_vec(),
            9 => f.logid_new = String::from_utf8_lossy(read_len_delim(buf, &mut i)?).into_owned(),
            _ => skip_field(buf, &mut i, wire)?,
        }
    }
    Ok(f)
}

fn push_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            b |= 0x80;
        }
        out.push(b);
        if v == 0 {
            return;
        }
    }
}

fn push_varint_field(out: &mut Vec<u8>, field: u64, v: u64) {
    push_varint(out, (field << 3) | 0);
    push_varint(out, v);
}

fn push_bytes_field(out: &mut Vec<u8>, field: u64, bytes: &[u8]) {
    push_varint(out, (field << 3) | 2);
    push_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

pub fn encode(f: &Frame) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + f.payload.len());
    push_varint_field(&mut out, 1, f.seqid);
    push_varint_field(&mut out, 2, f.logid);
    push_varint_field(&mut out, 3, f.service as u64);
    push_varint_field(&mut out, 4, f.method as u64);
    for (k, v) in &f.headers {
        let mut sub = Vec::with_capacity(k.len() + v.len() + 4);
        push_bytes_field(&mut sub, 1, k.as_bytes());
        push_bytes_field(&mut sub, 2, v.as_bytes());
        push_bytes_field(&mut out, 5, &sub);
    }
    if !f.payload_encoding.is_empty() {
        push_bytes_field(&mut out, 6, f.payload_encoding.as_bytes());
    }
    if !f.payload_type.is_empty() {
        push_bytes_field(&mut out, 7, f.payload_type.as_bytes());
    }
    if !f.payload.is_empty() {
        push_bytes_field(&mut out, 8, &f.payload);
    }
    if !f.logid_new.is_empty() {
        push_bytes_field(&mut out, 9, f.logid_new.as_bytes());
    }
    out
}

/// Client-initiated keepalive: Control frame with a `type: ping` header.
pub fn build_ping(service_id: &str) -> Vec<u8> {
    let svc = service_id.parse::<i64>().unwrap_or(0);
    encode(&Frame {
        service: svc,
        method: 0,
        headers: vec![("type".into(), "ping".into())],
        ..Frame::default()
    })
}

/// Response frame: reuse the received frame's ids/headers, add `biz_rt`, and
/// replace the payload. Used for the mandatory 3-second ACK (`{"code":200}`).
pub fn build_response(recv: &Frame, payload: &[u8]) -> Vec<u8> {
    let mut headers = recv.headers.clone();
    headers.push(("biz_rt".into(), "0".into()));
    encode(&Frame {
        seqid: recv.seqid,
        logid: recv.logid,
        service: recv.service,
        method: recv.method,
        headers,
        payload: payload.to_vec(),
        ..Frame::default()
    })
}

pub fn build_ack(recv: &Frame) -> Vec<u8> {
    build_response(recv, b"{\"code\":200}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let original = Frame {
            seqid: 7,
            logid: 99,
            service: 1,
            method: 1,
            headers: vec![
                ("type".into(), "event".into()),
                ("message_id".into(), "m-123".into()),
            ],
            payload: b"{\"hello\":\"world\"}".to_vec(),
            ..Frame::default()
        };
        let decoded = decode(&encode(&original)).unwrap();
        assert_eq!(decoded.seqid, 7);
        assert_eq!(decoded.logid, 99);
        assert_eq!(decoded.service, 1);
        assert_eq!(decoded.method, 1);
        assert_eq!(decoded.header("type"), Some("event"));
        assert_eq!(decoded.header("message_id"), Some("m-123"));
        assert_eq!(decoded.payload, b"{\"hello\":\"world\"}");
    }

    #[test]
    fn ping_is_control_with_type_ping() {
        let f = decode(&build_ping("42")).unwrap();
        assert_eq!(f.method, 0);
        assert_eq!(f.service, 42);
        assert_eq!(f.header("type"), Some("ping"));
    }

    #[test]
    fn ack_reuses_ids_and_sets_code_200() {
        let recv = Frame {
            seqid: 5,
            logid: 6,
            service: 1,
            method: 1,
            headers: vec![("type".into(), "event".into())],
            payload: b"x".to_vec(),
            ..Frame::default()
        };
        let f = decode(&build_ack(&recv)).unwrap();
        assert_eq!(f.seqid, 5);
        assert_eq!(f.method, 1);
        assert_eq!(f.payload, b"{\"code\":200}");
        assert!(f.header("biz_rt").is_some());
        assert_eq!(f.header("type"), Some("event"));
    }

    #[test]
    fn truncated_input_errors_instead_of_panicking() {
        let bytes = encode(&Frame {
            payload: b"abcdef".to_vec(),
            ..Frame::default()
        });
        assert!(matches!(
            decode(&bytes[..bytes.len() - 3]),
            Err(DecodeError::Truncated)
        ));
    }
}

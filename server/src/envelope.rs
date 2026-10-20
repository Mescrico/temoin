//! The plaintext carried inside every age object: `"SRP1" | u32 len | JSON | content`.

use serde::{Deserialize, Serialize};

pub const MAGIC: &[u8; 4] = b"SRP1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Header {
    pub v: u32,
    pub session: String,
    pub seq: u32,
    pub kind: String,
    pub container: String,
    #[serde(rename = "final")]
    pub is_final: bool,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<serde_json::Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum EnvelopeError {
    #[error("not a Témoin envelope")]
    Magic,
    #[error("truncated envelope")]
    Truncated,
    #[error("bad envelope header: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn encode(header: &Header, content: &[u8]) -> Vec<u8> {
    let json = serde_json::to_vec(header).expect("header json");
    let mut out = Vec::with_capacity(8 + json.len() + content.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(json.len() as u32).to_be_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(content);
    out
}

pub fn decode(buf: &[u8]) -> Result<(Header, &[u8]), EnvelopeError> {
    if buf.len() < 8 {
        return Err(EnvelopeError::Truncated);
    }
    if &buf[..4] != MAGIC {
        return Err(EnvelopeError::Magic);
    }
    let len = u32::from_be_bytes(buf[4..8].try_into().unwrap()) as usize;
    let json = buf.get(8..8 + len).ok_or(EnvelopeError::Truncated)?;
    let header = serde_json::from_slice(json)?;
    Ok((header, &buf[8 + len..]))
}

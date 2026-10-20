//! Structural validation of age v1 files, without decrypting them.
//!
//! The server never holds the identity, so it cannot verify the header MAC or
//! the AEAD tags. What it *can* check is that the upload is shaped exactly like
//! what a Témoin client produces: an age v1 header with a single X25519
//! stanza, followed by a STREAM payload whose length is consistent with
//! 64 KiB ChaCha20-Poly1305 chunks. Anything else (plaintext, other formats,
//! scrypt/passphrase files, truncated uploads) is rejected.

use base64::engine::general_purpose::STANDARD_NO_PAD;
use base64::Engine;

pub const VERSION_LINE: &[u8] = b"age-encryption.org/v1\n";
pub const CHUNK_SIZE: usize = 64 * 1024;
pub const TAG_SIZE: usize = 16;
pub const PAYLOAD_NONCE_SIZE: usize = 16;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FormatError {
    #[error("not an age v1 file")]
    NotAge,
    #[error("malformed age header")]
    BadHeader,
    #[error("expected exactly one X25519 stanza")]
    BadStanza,
    #[error("payload length inconsistent with age STREAM chunking")]
    BadPayload,
}

/// Returns the length of the header (up to and including the MAC line) on success.
pub fn validate(buf: &[u8]) -> Result<usize, FormatError> {
    if !buf.starts_with(VERSION_LINE) {
        return Err(FormatError::NotAge);
    }
    let mut pos = VERSION_LINE.len();
    let mut stanzas = 0usize;

    loop {
        let line = next_line(buf, pos).ok_or(FormatError::BadHeader)?;
        pos += line.len() + 1;
        if let Some(rest) = line.strip_prefix(b"-> ") {
            stanzas += 1;
            if stanzas > 1 {
                return Err(FormatError::BadStanza);
            }
            let mut args = rest.split(|b| *b == b' ');
            if args.next() != Some(b"X25519".as_slice()) {
                return Err(FormatError::BadStanza);
            }
            let share = args.next().ok_or(FormatError::BadStanza)?;
            if args.next().is_some() || decode_len(share) != Some(32) {
                return Err(FormatError::BadStanza);
            }
            // X25519 body = 16-byte file key + 16-byte tag = 32 bytes = one 43-char line.
            let body = next_line(buf, pos).ok_or(FormatError::BadHeader)?;
            pos += body.len() + 1;
            if decode_len(body) != Some(32) {
                return Err(FormatError::BadStanza);
            }
        } else if let Some(mac) = line.strip_prefix(b"--- ") {
            if stanzas != 1 {
                return Err(FormatError::BadStanza);
            }
            if decode_len(mac) != Some(32) {
                return Err(FormatError::BadHeader);
            }
            break;
        } else {
            return Err(FormatError::BadHeader);
        }
    }

    validate_payload_len(buf.len() - pos)?;
    Ok(pos)
}

fn validate_payload_len(len: usize) -> Result<(), FormatError> {
    // nonce + at least one non-empty chunk (empty plaintexts are never produced).
    if len < PAYLOAD_NONCE_SIZE + TAG_SIZE + 1 {
        return Err(FormatError::BadPayload);
    }
    let body = len - PAYLOAD_NONCE_SIZE;
    let full = CHUNK_SIZE + TAG_SIZE;
    let chunks = body.div_ceil(full);
    let last = body - (chunks - 1) * full;
    if last <= TAG_SIZE || last > full {
        return Err(FormatError::BadPayload);
    }
    Ok(())
}

fn next_line(buf: &[u8], pos: usize) -> Option<&[u8]> {
    let rest = buf.get(pos..)?;
    // Header lines are short; refuse to scan megabytes of binary looking for '\n'.
    let window = &rest[..rest.len().min(256)];
    let end = window.iter().position(|b| *b == b'\n')?;
    Some(&rest[..end])
}

fn decode_len(b64: &[u8]) -> Option<usize> {
    let decoded = STANDARD_NO_PAD.decode(b64).ok()?;
    // Reject non-canonical encodings (age requires canonical base64).
    if STANDARD_NO_PAD.encode(&decoded).as_bytes() != b64 {
        return None;
    }
    Some(decoded.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_lengths() {
        let full = CHUNK_SIZE + TAG_SIZE;
        assert!(validate_payload_len(16 + 17).is_ok());
        assert!(validate_payload_len(16 + 16).is_err());
        assert!(validate_payload_len(16 + full).is_ok());
        assert!(validate_payload_len(16 + full + 16).is_err());
        assert!(validate_payload_len(16 + full + 17).is_ok());
        assert!(validate_payload_len(16 + 3 * full).is_ok());
    }

    #[test]
    fn rejects_plaintext() {
        assert_eq!(validate(b"\xff\xd8\xff\xe0 JPEG"), Err(FormatError::NotAge));
    }
}

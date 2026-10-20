//! Encryption of hand-back links (`/r/<id>#<key>`).
//!
//! The operator PC decrypts a person's recordings, then re-encrypts them with a fresh
//! random key that only ever travels in the URL fragment (browsers never send it to
//! the server). Format chosen so that WebCrypto can decrypt it in any browser:
//!
//! * AES-256-GCM, plaintext cut in 1 MiB chunks, each sealed separately
//!   (ciphertext chunk = plaintext chunk + 16-byte tag), concatenated;
//! * nonce = BE32(file number) ‖ BE32(chunk number) ‖ BE32(1 if last chunk else 0),
//!   so chunks cannot be reordered, swapped between files or truncated;
//! * file 0 is the JSON manifest (names, types, sizes), files 1..n the media.
//!
//! An empty plaintext is one empty (last) chunk.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;

pub const CHUNK: usize = 1 << 20;
pub const TAG: usize = 16;

/// Size of the ciphertext of a `plain`-byte file.
pub fn ciphertext_len(plain: u64) -> u64 {
    plain + TAG as u64 * chunk_count(plain)
}

pub fn chunk_count(plain: u64) -> u64 {
    plain.div_ceil(CHUNK as u64).max(1)
}

pub fn nonce(file: u32, chunk: u32, last: bool) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[..4].copy_from_slice(&file.to_be_bytes());
    n[4..8].copy_from_slice(&chunk.to_be_bytes());
    n[8..].copy_from_slice(&(last as u32).to_be_bytes());
    n
}

pub fn new_key() -> [u8; 32] {
    rand::random()
}

/// The URL fragment carrying the key (base64url, 43 characters).
pub fn key_fragment(key: &[u8; 32]) -> String {
    URL_SAFE_NO_PAD.encode(key)
}

pub fn parse_fragment(s: &str) -> Option<[u8; 32]> {
    URL_SAFE_NO_PAD.decode(s.trim()).ok()?.try_into().ok()
}

/// Seals one file chunk by chunk (the caller knows which chunk is the last one).
pub struct Encryptor {
    cipher: Aes256Gcm,
    file: u32,
    chunk: u32,
}

impl Encryptor {
    pub fn new(key: &[u8; 32], file: u32) -> Self {
        Encryptor { cipher: Aes256Gcm::new(key.into()), file, chunk: 0 }
    }

    pub fn seal_chunk(&mut self, plain: &[u8], last: bool) -> Vec<u8> {
        assert!(plain.len() <= CHUNK);
        let n = nonce(self.file, self.chunk, last);
        self.chunk += 1;
        self.cipher.encrypt(Nonce::from_slice(&n), plain).expect("AES-GCM encryption")
    }
}

pub fn encrypt(key: &[u8; 32], file: u32, plain: &[u8]) -> Vec<u8> {
    let mut e = Encryptor::new(key, file);
    let n = chunk_count(plain.len() as u64) as usize;
    let mut out = Vec::with_capacity(ciphertext_len(plain.len() as u64) as usize);
    for i in 0..n {
        let part = &plain[(i * CHUNK).min(plain.len())..((i + 1) * CHUNK).min(plain.len())];
        out.extend_from_slice(&e.seal_chunk(part, i + 1 == n));
    }
    out
}

/// Reference decryption (what the web page does). `None` if anything was altered.
pub fn decrypt(key: &[u8; 32], file: u32, ct: &[u8]) -> Option<Vec<u8>> {
    let cipher = Aes256Gcm::new(key.into());
    let mut out = Vec::new();
    let mut rest = ct;
    let mut i = 0u32;
    loop {
        let last = rest.len() <= CHUNK + TAG;
        let (part, tail) = rest.split_at(if last { rest.len() } else { CHUNK + TAG });
        out.extend(cipher.decrypt(Nonce::from_slice(&nonce(file, i, last)), part).ok()?);
        if last {
            return Some(out);
        }
        rest = tail;
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_sizes() {
        let key = new_key();
        for len in [0usize, 1, CHUNK - 1, CHUNK, CHUNK + 1, 3 * CHUNK, 3 * CHUNK + 5] {
            let pt: Vec<u8> = (0..len).map(|i| (i * 7 % 251) as u8).collect();
            let ct = encrypt(&key, 3, &pt);
            assert_eq!(ct.len() as u64, ciphertext_len(len as u64), "len {len}");
            assert_eq!(decrypt(&key, 3, &ct).as_deref(), Some(&pt[..]), "len {len}");
            assert!(decrypt(&key, 4, &ct).is_none(), "other file number must fail");
        }
    }

    #[test]
    fn truncation_is_detected() {
        let key = new_key();
        let pt = vec![1u8; 2 * CHUNK + 10];
        let ct = encrypt(&key, 1, &pt);
        // Dropping the last chunk leaves a full non-last chunk at the end.
        assert!(decrypt(&key, 1, &ct[..2 * (CHUNK + TAG)]).is_none());
        assert!(decrypt(&key, 1, &ct[..ct.len() - 1]).is_none());
    }

    #[test]
    fn fragment_roundtrip() {
        let key = new_key();
        let f = key_fragment(&key);
        assert_eq!(f.len(), 43);
        assert_eq!(parse_fragment(&f), Some(key));
    }
}

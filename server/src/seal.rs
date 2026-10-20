//! Reference implementation of the Témoin sealer (age v1, one X25519
//! recipient, no grease). The Android and iOS clients implement exactly this;
//! keep the three in sync. Decryption is done with the `age` crate, so any
//! deviation from the age spec here is caught by the round-trip tests.

use base64::engine::general_purpose::STANDARD_NO_PAD;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

use crate::ageformat::{CHUNK_SIZE, VERSION_LINE};

const X25519_INFO: &[u8] = b"age-encryption.org/v1/X25519";

#[derive(Debug, thiserror::Error)]
pub enum RecipientError {
    #[error("recipient must be an age1… X25519 public key")]
    Invalid,
}

/// Decodes an `age1…` bech32 recipient into its 32-byte X25519 public key.
pub fn parse_recipient(s: &str) -> Result<[u8; 32], RecipientError> {
    let (hrp, data) = bech32::decode(s.trim()).map_err(|_| RecipientError::Invalid)?;
    if hrp.as_str() != "age" || data.len() != 32 || s.trim().to_ascii_lowercase() != s.trim() {
        return Err(RecipientError::Invalid);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&data);
    Ok(out)
}

/// Short human-checkable fingerprint of a recipient (shown in the apps).
pub fn fingerprint(recipient: &str) -> String {
    use sha2::Digest;
    let digest = Sha256::digest(recipient.trim().as_bytes());
    let hex = hex::encode(&digest[..10]).to_uppercase();
    hex.as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn seal(recipient: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let mut rng = rand::rngs::OsRng;
    let mut file_key = [0u8; 16];
    rng.fill_bytes(&mut file_key);

    // --- X25519 stanza
    let mut eph_bytes = [0u8; 32];
    rng.fill_bytes(&mut eph_bytes);
    let eph = StaticSecret::from(eph_bytes);
    let eph_pub = PublicKey::from(&eph);
    let shared = eph.diffie_hellman(&PublicKey::from(*recipient));

    let mut salt = Vec::with_capacity(64);
    salt.extend_from_slice(eph_pub.as_bytes());
    salt.extend_from_slice(recipient);
    let wrap_key = hkdf(&salt, shared.as_bytes(), X25519_INFO);
    let body = ChaCha20Poly1305::new(Key::from_slice(&wrap_key))
        .encrypt(Nonce::from_slice(&[0u8; 12]), file_key.as_slice())
        .expect("wrap");

    let mut out = Vec::with_capacity(plaintext.len() + plaintext.len() / CHUNK_SIZE * 16 + 256);
    out.extend_from_slice(VERSION_LINE);
    out.extend_from_slice(b"-> X25519 ");
    out.extend_from_slice(STANDARD_NO_PAD.encode(eph_pub.as_bytes()).as_bytes());
    out.push(b'\n');
    out.extend_from_slice(STANDARD_NO_PAD.encode(&body).as_bytes());
    out.push(b'\n');
    out.extend_from_slice(b"---");

    // --- header MAC
    let mac_key = hkdf(&[], &file_key, b"header");
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&mac_key).unwrap();
    mac.update(&out);
    let tag = mac.finalize().into_bytes();
    out.push(b' ');
    out.extend_from_slice(STANDARD_NO_PAD.encode(tag).as_bytes());
    out.push(b'\n');

    // --- STREAM payload
    let mut nonce = [0u8; 16];
    rng.fill_bytes(&mut nonce);
    out.extend_from_slice(&nonce);
    let payload_key = hkdf(&nonce, &file_key, b"payload");
    let aead = ChaCha20Poly1305::new(Key::from_slice(&payload_key));

    let chunks: Vec<&[u8]> = if plaintext.is_empty() {
        vec![&[]]
    } else {
        plaintext.chunks(CHUNK_SIZE).collect()
    };
    for (i, chunk) in chunks.iter().enumerate() {
        let mut n = [0u8; 12];
        n[3..11].copy_from_slice(&(i as u64).to_be_bytes());
        if i == chunks.len() - 1 {
            n[11] = 1;
        }
        let ct = aead
            .encrypt(Nonce::from_slice(&n), Payload { msg: chunk, aad: &[] })
            .expect("chunk");
        out.extend_from_slice(&ct);
    }
    out
}

fn hkdf(salt: &[u8], ikm: &[u8], info: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(salt), ikm);
    let mut okm = [0u8; 32];
    hk.expand(info, &mut okm).expect("hkdf");
    okm
}

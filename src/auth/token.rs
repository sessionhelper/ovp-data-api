//! Bearer token generation + hashing.

use rand::RngCore;
use sha2::{Digest, Sha256};

/// Generate a fresh random token (64 hex chars = 256 bits of entropy).
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// SHA-256 hex digest for storage / lookup.
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    hex::encode(digest)
}

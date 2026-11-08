use sha2::{Digest, Sha256};

const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Random base62 string of `len` chars from the OS CSPRNG.
pub fn random_token(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    getrandom_fill(&mut bytes);
    bytes.iter().map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char).collect()
}

fn getrandom_fill(buf: &mut [u8]) {
    use rand::TryRng;
    rand::rngs::SysRng.try_fill_bytes(buf).expect("os rng");
}

/// Prefixed id, e.g. `new_id("pst")` -> `pst_3fK9…` (16 random chars).
pub fn new_id(prefix: &str) -> String {
    format!("{}_{}", prefix, random_token(16))
}

pub fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

//! Streaming SHA-256 for backup artifacts (Section 12/14).
//!
//! Backups can be large, so we hash in fixed-size chunks rather than loading the whole file.

use crate::error::Result;
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::io::AsyncReadExt;

const CHUNK: usize = 1024 * 1024; // 1 MiB

/// Compute the lowercase hex SHA-256 of a file, streaming it in chunks.
pub async fn sha256_file(path: &Path) -> Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

/// Compute the lowercase hex SHA-256 of an in-memory byte slice (used in tests / small inputs).
pub fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex_lower(&hasher.finalize())
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector_empty() {
        // SHA-256 of "" is well-known.
        assert_eq!(
            sha256_bytes(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn known_vector_abc() {
        assert_eq!(
            sha256_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[tokio::test]
    async fn file_hash_matches_bytes_hash() {
        let dir = std::env::temp_dir().join(format!("dbm-hash-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("data.bin");
        let data = vec![7u8; 3 * CHUNK + 123]; // spans multiple chunks
        std::fs::write(&path, &data).unwrap();
        let from_file = sha256_file(&path).await.unwrap();
        assert_eq!(from_file, sha256_bytes(&data));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

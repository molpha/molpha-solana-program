use sha2::{Digest, Sha256};

// Helper function to convert a string into a deterministic seed
pub fn to_hash(string: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(string.as_bytes());
    hasher.finalize().into()
}

use sha2::{Sha256, Digest};

// Helper function to convert a string into a deterministic seed
pub fn to_hash(string: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(string.as_bytes());
    hasher.finalize().into()
}
use sha2::{Sha256, Digest};

pub trait Hashable {
    fn bytes(&self) -> Vec<u8>;

    fn hash(&self) -> Vec<u8> {
        let mut hasher = Sha256::new();  
        hasher.update(&self.bytes());     
        hasher.finalize().to_vec()
    }
}
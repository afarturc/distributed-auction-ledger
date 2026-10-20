use super::hashable::Hashable;
use crate::auction::AuctionCommand;
use super::{u32_to_bytes, u64_to_bytes, u128_to_bytes, BHash};
use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};

#[derive(Serialize, Deserialize, Debug, Clone)] 
pub struct Block {
    pub index: u32,
    pub timestamp: u128,
    pub prev_hash: BHash, 
    pub miner_id: String,
    pub transactions: Vec<AuctionCommand>,
    pub nonce: u64,
    pub hash: BHash,
    pub vrf_output: Vec<u8>,      // VRF output for PoS (32 bytes)
    pub vrf_proof: Option<Vec<u8>>, // VRF proof data (serialized VRFProof)
    pub merkle_root: BHash,        // Merkle root of all transactions
}

impl Hashable for Block {
    fn bytes(&self) -> Vec<u8> {
        let mut bytes = vec![];
        
        bytes.extend(u32_to_bytes(self.index));
        bytes.extend(u128_to_bytes(self.timestamp));
        bytes.extend(&self.prev_hash);
        bytes.extend(u64_to_bytes(self.nonce));
        bytes.extend(self.miner_id.as_bytes());
        bytes.extend(&self.vrf_output);
        bytes.extend(&self.merkle_root);

        // Deterministic hashing of transactions: use their stable binary representation
        for cmd in &self.transactions {
            bytes.extend(cmd.bytes()); 
        }
        
        bytes
    }
}

impl Block {
    /// Generates a deterministic seed for VRF that is independent of the VRF output itself.
    /// This prevents circular dependency during block sealing and validation.
    pub fn calculate_vrf_seed(&self) -> BHash {
        let mut hasher = Sha256::new();
        
        hasher.update(u32_to_bytes(self.index));
        hasher.update(u128_to_bytes(self.timestamp));
        hasher.update(&self.prev_hash);
        hasher.update(self.miner_id.as_bytes());
        hasher.update(&self.merkle_root);
        
        hasher.finalize().to_vec()
    }

    // Calculate merkle root from transactions
    pub fn calculate_merkle_root(transactions: &[AuctionCommand]) -> BHash {
        if transactions.is_empty() {
            // Empty block gets zero merkle root
            return vec![0; 32];
        }

        // Hash each transaction deterministically
        let mut hashes: Vec<Vec<u8>> = transactions.iter()
            .map(|cmd| {
                let mut hasher = Sha256::new();
                hasher.update(cmd.bytes());
                hasher.finalize().to_vec()
            })
            .collect();

        // Build tree bottom-up
        while hashes.len() > 1 {
            let mut next_level = Vec::new();
            for i in (0..hashes.len()).step_by(2) {
                let mut hasher = Sha256::new();
                hasher.update(&hashes[i]);
                if i + 1 < hashes.len() {
                    hasher.update(&hashes[i + 1]);
                } else {
                    // Duplicate last hash if odd number
                    hasher.update(&hashes[i]);
                }
                next_level.push(hasher.finalize().to_vec());
            }
            hashes = next_level;
        }

        hashes.into_iter().next().unwrap_or_else(|| vec![0; 32])
    }
}
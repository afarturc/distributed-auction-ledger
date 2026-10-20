use super::block::Block;
use super::Hashable;
use super::vrf;
use crate::p2p::ReputationTracker;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;

#[async_trait]
pub trait ConsensusEngine: Send + Sync {

fn validate_block(&self, block: &Block) -> Result<(), Box<dyn std::error::Error>>;

async fn seal_block(&self, block: &mut Block) -> Result<(), Box<dyn std::error::Error>>;

fn name(&self) -> &str;

fn reputation_tracker(&self) -> Option<Arc<Mutex<ReputationTracker>>> {
    None
}

}

  

pub struct ProofOfWorkEngine {

pub difficulty: usize,

}

  

#[async_trait]
impl ConsensusEngine for ProofOfWorkEngine {

fn validate_block(&self, block: &Block) -> Result<(), Box<dyn std::error::Error>> {

let target = "0".repeat(self.difficulty);

let hash_str = block.hash

.iter()

.map(|b| format!("{:02x}", b))

.collect::<String>();

if !hash_str.starts_with(&target) {

return Err("Invalid PoW".into());

}

Ok(())

}

async fn seal_block(&self, block: &mut Block) -> Result<(), Box<dyn std::error::Error>> {

    let target = "0".repeat(self.difficulty);

    println!("Mining block {}...", block.index);

    loop {

        block.hash = block.hash();

        let hash_str = block.hash

        .iter()

        .map(|b| format!("{:02x}", b))

        .collect::<String>();

        if hash_str.starts_with(&target) {

            println!("Block mined! Hash: {}", hash_str);

            return Ok(());

        }

        block.nonce += 1;

        // Cooperative yield: allow the scheduler / other tasks to run periodically.
        // This helps tokio's select! macro to interrupt this loop if another future finishes.
        if block.nonce % 1000 == 0 {
            tokio::task::yield_now().await;
        }

    }

}

fn name(&self) -> &str { "Proof of Work" }

}

  

pub struct ProofOfStakeEngine {
    pub reputation_tracker: Arc<Mutex<ReputationTracker>>,
}

impl ProofOfStakeEngine {
    pub fn new(reputation_tracker: Arc<Mutex<ReputationTracker>>) -> Self {
        Self { reputation_tracker }
    }
}

  

#[async_trait]
impl ConsensusEngine for ProofOfStakeEngine {

fn validate_block(&self, block: &Block) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Basic structural checks
    if block.vrf_output.len() != 32 {
         return Err(format!("Invalid VRF output length: expected 32, got {}", block.vrf_output.len()).into());
    }
    
    if block.vrf_proof.is_none() {
        return Err("Block missing VRF proof".into());
    }
    
    // 2. Verify VRF proof was generated correctly using the stable seed
    let vrf_proof_data = block.vrf_proof.as_ref().unwrap();
    let vrf_proof: vrf::VRFProof = serde_json::from_slice(vrf_proof_data)
        .map_err(|e| format!("Failed to deserialize VRF proof: {}", e))?;
    
    let vrf_seed = block.calculate_vrf_seed();
    if !vrf::verify_vrf_proof(&block.miner_id, &vrf_seed, &vrf_proof)? {
        return Err(format!("VRF proof verification failed for miner {}", block.miner_id).into());
    }

    // 3. Verify VRF output in block matches the proof
    if block.vrf_output != vrf_proof.output {
        return Err("Block VRF output does not match proof output".into());
    }
    
    // 4. Verify Proposal Eligibility (Stake + Reputation + VRF)
    let mut reputation_tracker = self.reputation_tracker.lock()
        .map_err(|_| "Failed to lock reputation tracker")?;

    let reputation_score = reputation_tracker.get_peer_reputation(&block.miner_id)
        .map(|r| r.score)
        .unwrap_or(100); // Default to 100 (starting score) if unknown

    // Rule: Reputation must be non-negative
    if reputation_score < 0 {
        let _was_banned = reputation_tracker.decrement_invalid_block(&block.miner_id);
        return Err(format!(
            "Rejected by PoS: miner {} has negative reputation score {}",
            block.miner_id, reputation_score
        ).into());
    }

    // Standard threshold logic
    let difficulty = 0.3; 
    if !vrf::should_propose_block(&block.vrf_output, reputation_score, difficulty) {
        return Err(format!(
            "Miner {} not eligible for this slot (VRF: {:?}, Rep: {})", 
            block.miner_id, 
            &block.vrf_output[..8.min(block.vrf_output.len())],
            reputation_score
        ).into());
    }

    // Success: increment reputation for contributing a valid block
    reputation_tracker.increment_valid_block(&block.miner_id);
    Ok(())
}

async fn seal_block(&self, block: &mut Block) -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    
    // Generate VRF proof using stable seed (independent of vrf_output/hash)
    let vrf_seed = block.calculate_vrf_seed();
    let seed = vrf_seed.as_slice();
    let vrf_proof = vrf::generate_vrf_proof(&block.miner_id, seed)?;
    
    // Store VRF output and proof
    block.vrf_output = vrf_proof.output.clone();
    block.vrf_proof = Some(serde_json::to_vec(&vrf_proof)?);
    
    // Recalculate block hash now that VRF data is included
    block.hash = block.hash();
    
    // Check if miner is eligible to propose based on VRF output
    let reputation_tracker = self
        .reputation_tracker
        .lock()
        .map_err(|_| "Failed to access reputation tracker")?;
    
    let reputation_score = reputation_tracker
        .get_peer_reputation(&block.miner_id)
        .map(|r| r.score)
        .unwrap_or(100);
    
    // Difficulty threshold: 0.3 means node needs combined score > 0.3 to propose
    let difficulty = 0.3;
    if !vrf::should_propose_block(&vrf_proof.output, reputation_score, difficulty) {
        return Err(format!(
            "Miner {} not eligible for PoS block proposal (VRF: {:?}, Reputation: {})",
            block.miner_id,
            &vrf_proof.output[..8.min(vrf_proof.output.len())],
            reputation_score
        ).into());
    }
    
    let micros = started.elapsed().as_micros();
    println!("Sealing block with PoS + VRF rules... Validated by Stake + Reputation + VRF. Time: {}us", micros);
    Ok(())
}

fn name(&self) -> &str { "Proof of Stake (with Reputation)" }

fn reputation_tracker(&self) -> Option<Arc<Mutex<ReputationTracker>>> {
    Some(self.reputation_tracker.clone())
}

}



#[cfg(test)]
mod tests {
    use super::*;
    use crate::auction::AuctionCommand;
    use crate::blockchain::block::Block;

    fn sample_block(miner_id: &str) -> Block {
        let mut block = Block {
            index: 1,
            timestamp: 1,
            prev_hash: vec![0; 32],
            miner_id: miner_id.to_string(),
            transactions: vec![AuctionCommand::CreateAuction {
                auction_id: "auction-1".to_string(),
                item_name: "item".to_string(),
                description: None,  // Optional - can be omitted or None
                starting_price: 10,
            }],
            nonce: 0,
            hash: vec![],
            vrf_output: vec![0; 64],      // Will be generated
            vrf_proof: None,              // Will be generated
            merkle_root: vec![],          // Will be calculated
        };
        block.merkle_root = crate::blockchain::block::Block::calculate_merkle_root(&block.transactions);
        
        // Generate valid VRF proof for the test using stable seed
        let seed = block.calculate_vrf_seed();
        if let Ok(vrf_proof) = vrf::generate_vrf_proof(miner_id, &seed) {
            block.vrf_output = vrf_proof.output.clone();
            block.vrf_proof = serde_json::to_vec(&vrf_proof).ok();
        }
        
        // Finalize block hash
        block.hash = block.hash();
        
        block
    }

    #[test]
    fn proof_of_stake_rejects_negative_miner() {
        let tracker = Arc::new(Mutex::new(ReputationTracker::new()));
        let _was_banned = tracker
            .lock()
            .unwrap()
            .record_suspicious_behavior("miner-1", 150); // New peers start at 100, so 100 - 150 = -50

        let engine = ProofOfStakeEngine::new(tracker);
        let block = sample_block("miner-1");

        assert!(engine.validate_block(&block).is_err());
    }

    #[test]
    fn proof_of_stake_rewards_valid_miner() {
        let tracker = Arc::new(Mutex::new(ReputationTracker::new()));
        
        // Verify that a new peer (miner-2) starts with reputation 100
        let peer_rep = tracker.lock().unwrap().get_peer_reputation("miner-2");
        assert_eq!(peer_rep, None, "Peer should not exist initially");
        
        // Simulate a valid block from miner-2
        tracker.lock().unwrap().increment_valid_block("miner-2");
        
        // Check that reputation is capped at 100 (starts at 100, +1 = 101 capped at 100)
        let peer_rep = tracker.lock().unwrap().get_peer_reputation("miner-2").unwrap();
        assert_eq!(peer_rep.score, 100, "Valid peer score should be capped at maximum 100");
    }
}
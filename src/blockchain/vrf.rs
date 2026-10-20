use sha2::{Digest, Sha256};
use serde::{Serialize, Deserialize};
use schnorrkel::signing_context;
use schnorrkel::MiniSecretKey;
use std::fmt;

/// Custom error type for VRF operations
#[derive(Debug)]
pub enum VRFError {
    SignatureError(String),
    InvalidLength,
    VerificationFailed,
}

impl fmt::Display for VRFError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            VRFError::SignatureError(msg) => write!(f, "Signature error: {}", msg),
            VRFError::InvalidLength => write!(f, "Invalid length for array conversion"),
            VRFError::VerificationFailed => write!(f, "VRF verification failed"),
        }
    }
}

impl std::error::Error for VRFError {}

/// VRF Output and Proof structure
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VRFProof {
    pub output: Vec<u8>,      // VRF output (64 bytes)
    pub proof: Vec<u8>,       // VRF proof (80 bytes)
}

/// Generate a VRF proof for a given input using a secret key derived from miner_id
pub fn generate_vrf_proof(miner_id: &str, seed: &[u8]) -> Result<VRFProof, Box<dyn std::error::Error>> {
    // Derive a secret key from the miner_id
    let mut hasher = Sha256::new();
    hasher.update(miner_id.as_bytes());
    let secret_bytes: [u8; 32] = hasher.finalize().as_slice().try_into()
        .map_err(|_| VRFError::InvalidLength)?;
    
    // Create a MiniSecretKey from the derived bytes
    let mini_secret = MiniSecretKey::from_bytes(&secret_bytes)
        .map_err(|e| Box::new(VRFError::SignatureError(format!("{:?}", e))) as Box<dyn std::error::Error>)?;
    let keypair = mini_secret.expand_to_keypair(schnorrkel::ExpansionMode::Ed25519);
    
    // VRF context
    let ctx = signing_context(b"ledger_vrf");
    
    // Generate VRF proof (returns VRFInOut, VRFProof, VRFProofBatchable)
    let (output, proof, _proof_batchable) = keypair.vrf_sign(ctx.bytes(seed));
    
    Ok(VRFProof {
        output: output.as_output_bytes().to_vec(),
        proof: proof.to_bytes().to_vec(),
    })
}

/// Verify a VRF proof using the miner's public key (derived from miner_id)
pub fn verify_vrf_proof(
    miner_id: &str,
    seed: &[u8],
    vrf_proof: &VRFProof,
) -> Result<bool, Box<dyn std::error::Error>> {
    // Derive the same secret key and get public key
    let mut hasher = Sha256::new();
    hasher.update(miner_id.as_bytes());
    let secret_bytes: [u8; 32] = hasher.finalize().as_slice().try_into()
        .map_err(|_| VRFError::InvalidLength)?;
    
    let mini_secret = MiniSecretKey::from_bytes(&secret_bytes)
        .map_err(|e| Box::new(VRFError::SignatureError(format!("{:?}", e))) as Box<dyn std::error::Error>)?;
    let keypair = mini_secret.expand_to_keypair(schnorrkel::ExpansionMode::Ed25519);
    let public_key = keypair.public;
    
    // VRF context (must be the same as used in generation)
    let ctx = signing_context(b"ledger_vrf");
    
    // Reconstruct the proof objects
    let output_array: [u8; 32] = vrf_proof.output.as_slice().try_into()
        .map_err(|_| VRFError::InvalidLength)?;
    let proof_array: [u8; 64] = vrf_proof.proof.as_slice().try_into()
        .map_err(|_| VRFError::InvalidLength)?;
    
    let vrf_output = schnorrkel::vrf::VRFPreOut::from_bytes(&output_array)
        .map_err(|e| Box::new(VRFError::SignatureError(format!("{:?}", e))) as Box<dyn std::error::Error>)?;
    let vrf_proof_reconstructed = schnorrkel::vrf::VRFProof::from_bytes(&proof_array)
        .map_err(|e| Box::new(VRFError::SignatureError(format!("{:?}", e))) as Box<dyn std::error::Error>)?;
    
    // Verify the proof
    let verification_result = public_key.vrf_verify(
        ctx.bytes(seed),
        &vrf_output,
        &vrf_proof_reconstructed,
    );
    
    Ok(verification_result.is_ok())
}

/// Convert VRF output to a stake-like score (0-1)
/// Lower values are "better" for lottery selection
pub fn output_to_stake_weight(output: &[u8]) -> f64 {
    // Take first 8 bytes of output and convert to u64
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&output[..8.min(output.len())]);
    let value = u64::from_le_bytes(bytes);
    
    // Normalize to 0-1 range
    value as f64 / u64::MAX as f64
}

/// Determine if a node has the right to propose a block based on:
/// - VRF output (lower = better chance)
/// - Reputation score (higher = better chance)
pub fn should_propose_block(vrf_output: &[u8], reputation_score: i32, difficulty: f64) -> bool {
    let stake_weight = output_to_stake_weight(vrf_output);
    
    // Normalize reputation (-100 to 100 -> 0 to 1)
    let reputation_weight = ((reputation_score as f64 + 100.0) / 200.0).clamp(0.0, 1.0);
    
    // Combined score: lower VRF weight + higher reputation = better chance
    let combined_score = (1.0 - stake_weight) * reputation_weight;
    
    // If combined score exceeds difficulty threshold, node can propose
    combined_score > difficulty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vrf_generation_and_verification() {
        let miner_id = "test_miner_123";
        let seed = b"test_seed_data";
        
        // Generate proof
        let proof = generate_vrf_proof(miner_id, seed).expect("Failed to generate VRF proof");
        assert_eq!(proof.output.len(), 32);
        assert_eq!(proof.proof.len(), 64);
        
        // Verify proof
        let is_valid = verify_vrf_proof(miner_id, seed, &proof).expect("Failed to verify VRF proof");
        assert!(is_valid);
    }

    #[test]
    fn test_different_miners_different_outputs() {
        let seed = b"same_seed";
        
        let proof1 = generate_vrf_proof("miner_1", seed).expect("Failed to generate VRF proof");
        let proof2 = generate_vrf_proof("miner_2", seed).expect("Failed to generate VRF proof");
        
        // Different miners should produce different outputs for the same seed
        assert_ne!(proof1.output, proof2.output);
    }

    #[test]
    fn test_invalid_proof_fails_verification() {
        let miner_id = "test_miner";
        let seed = b"test_seed";
        
        let proof = generate_vrf_proof(miner_id, seed).expect("Failed to generate VRF proof");
        
        // Try to verify with wrong seed
        let is_valid = verify_vrf_proof(miner_id, b"wrong_seed", &proof).expect("Failed to verify");
        assert!(!is_valid);
    }

    #[test]
    fn test_should_propose_block() {
        let output = vec![0u8; 64]; // Low output (all zeros)
        
        // With high reputation, should propose
        assert!(should_propose_block(&output, 50, 0.3));
        
        // With low reputation, might not propose
        let high_output = vec![255u8; 64];
        assert!(!should_propose_block(&high_output, -50, 0.3));
    }
}

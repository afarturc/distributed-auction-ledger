use std::time::{SystemTime, UNIX_EPOCH};

pub mod hashable;
pub mod transaction;
pub mod block;
pub mod chain;
pub mod consensus;
pub mod vrf;

pub use hashable::Hashable;
pub use transaction::AuctionTransaction;
pub use block::Block;
pub use chain::Blockchain;
pub use consensus::{ConsensusEngine, ProofOfWorkEngine, ProofOfStakeEngine};
pub use vrf::VRFProof;
pub mod storage;

pub type BHash = Vec<u8>;

pub fn now() -> u128 {
    let epoch_duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards");
    epoch_duration.as_secs() as u128 * 1000 + epoch_duration.subsec_millis() as u128
}

pub fn u32_to_bytes(n: u32) -> [u8; 4] { n.to_be_bytes() }
pub fn u64_to_bytes(n: u64) -> [u8; 8] { n.to_be_bytes() }
pub fn u128_to_bytes(n: u128) -> [u8; 16] { n.to_be_bytes() }
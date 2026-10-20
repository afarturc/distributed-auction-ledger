use serde::{Serialize, Deserialize};
use crate::blockchain::Block;

/// Upper bound on blocks served per request to prevent abuse and unbounded memory use.
pub const MAX_BLOCKS_PER_RESPONSE: usize = 1000;

/// Request the responder's chain starting at `from_index`.
/// Clients use `from_index = 0` for a full-chain catch-up.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRequest {
    pub from_index: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResponse {
    pub blocks: Vec<Block>,
}

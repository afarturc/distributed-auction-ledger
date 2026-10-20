use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use crate::auction::{Auction, AuctionCommand};
use super::block::Block;
use super::Hashable; // Needed for hashing the genesis block
use super::consensus::ConsensusEngine;
use std::env;
use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};

// Intermediate struct for serialization/deserialization (without the trait object)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SerializableBlockchain {
    pub blocks: Vec<Block>,
    pub mempool: Vec<AuctionCommand>,
    pub active_auctions: HashMap<String, Auction>,
    pub bidder_nonces: HashMap<String, u64>,
}

#[derive(Clone)]
pub struct Blockchain {
    pub blocks: Vec<Block>,
    pub consensus_engine: Arc<dyn ConsensusEngine>,
    // Mempool of unconfirmed commands waiting to be mined.
    pub mempool: Vec<AuctionCommand>,
    // Current application state derived from accepted commands.
    pub active_auctions: HashMap<String, Auction>,
    // Nonce tracker per bidder to prevent replay attacks
    pub bidder_nonces: HashMap<String, u64>,
}

impl Serialize for Blockchain {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        SerializableBlockchain {
            blocks: self.blocks.clone(),
            mempool: self.mempool.clone(),
            active_auctions: self.active_auctions.clone(),
            bidder_nonces: self.bidder_nonces.clone(),
        }.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Blockchain {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let serializable = SerializableBlockchain::deserialize(deserializer)?;
        // Choose consensus engine based on environment variable CONSENSUS_MODE.
        // Default: POW
        let mode = env::var("CONSENSUS_MODE").unwrap_or_else(|_| "POW".to_string());
        let consensus_engine: Arc<dyn ConsensusEngine> = match mode.as_str() {
            "POS" | "PoS" | "pos" => Arc::new(super::consensus::ProofOfStakeEngine::new(
                Arc::new(std::sync::Mutex::new(crate::p2p::ReputationTracker::new())),
            )),
            _ => Arc::new(super::consensus::ProofOfWorkEngine { difficulty: 5 }),
        };

        Ok(Blockchain {
            blocks: serializable.blocks,
            consensus_engine,
            mempool: serializable.mempool,
            active_auctions: serializable.active_auctions,
            bidder_nonces: serializable.bidder_nonces,
        })
    }
}

impl Blockchain {
    // Initializes a chain with a deterministic genesis block and a consensus engine.
    pub fn new(consensus_engine: Arc<dyn ConsensusEngine>) -> Self {
        let mut chain = Blockchain {
            blocks: Vec::new(),
            consensus_engine,
            mempool: Vec::new(),
            active_auctions: HashMap::new(),
            bidder_nonces: HashMap::new(),
        };
        chain.create_genesis_block();
        chain
    }

    // Creates and appends the genesis block.
    fn create_genesis_block(&mut self) {
        let mut genesis_block = Block {
            index: 0,
            // Keep a fixed timestamp so every node computes the same genesis hash.
            timestamp: 1700000000000, 
            prev_hash: vec![0; 32],
            miner_id: String::new(),
            transactions: vec![], 
            nonce: 0,
            hash: vec![],
            vrf_output: vec![0; 32],      // Consistent 32-byte VRF output
            vrf_proof: None,              // No VRF proof for genesis
            merkle_root: vec![0; 32],     // Empty block has zero merkle root
        };

        genesis_block.hash = genesis_block.hash();
        self.blocks.push(genesis_block);
    }

    // Returns the last block in the chain.
    pub fn get_last_block(&self) -> Option<&Block> {
        self.blocks.last()
    }

    // Runs the consensus algorithm to seal a block.
    pub async fn mine_block(&self, block: &mut Block) -> Result<(), Box<dyn std::error::Error>> {
        self.consensus_engine.seal_block(block).await
    }

    // Mines all commands currently in the mempool.
    pub async fn mine_pending_transactions(&mut self, miner_id: String) -> Result<Block, String> {
        if self.mempool.is_empty() {
            return Err("Mempool is empty. No transactions to mine.".to_string());
        }

        let last_block = self.get_last_block().unwrap().clone();
        
        let mut new_block = Block {
            index: last_block.index + 1,
            timestamp: super::now(),
            prev_hash: last_block.hash.clone(),
            miner_id,
            transactions: self.mempool.clone(),
            nonce: 0,
            hash: vec![],
            vrf_output: vec![],           // Will be filled by consensus engine
            vrf_proof: None,              // Will be filled by consensus engine
            merkle_root: vec![],          // Will be calculated
        };

        // Calculate merkle root from transactions
        new_block.merkle_root = Block::calculate_merkle_root(&new_block.transactions);

        // Perform consensus sealing (PoW, PoS, etc.)
        self.mine_block(&mut new_block).await
            .map_err(|e| format!("Failed to seal block: {}", e))?;

        // Commit the mined block locally and clear the mempool.
        self.blocks.push(new_block.clone());
        self.mempool.clear();

        Ok(new_block)
    }

    /// Creates a new block template ready to be mined, but does not add it to the chain.
    /// Returns the block and the consensus engine to use.
    pub fn create_mining_block(&self, miner_id: String) -> Block {
        let last_block = self.get_last_block().unwrap().clone();
        
        let transactions = if self.mempool.is_empty() {
            vec![]
        } else {
            self.mempool.clone()
        };
        
        let mut new_block = Block {
            index: last_block.index + 1,
            timestamp: super::now(),
            prev_hash: last_block.hash.clone(),
            miner_id,
            transactions,
            nonce: 0,
            hash: vec![],
            vrf_output: vec![],
            vrf_proof: None,
            merkle_root: vec![],
        };

        new_block.merkle_root = Block::calculate_merkle_root(&new_block.transactions);
        new_block
    }

    /// Commits a successfully mined block to the chain and clears the mempool.
    pub fn commit_mined_block(&mut self, block: Block) {
        // If the block contains transactions, clear them from the mempool.
        // Simple implementation: clear all since we currently mine everything in mempool.
        if !block.transactions.is_empty() {
            self.mempool.clear();
        }
        self.blocks.push(block);
    }

    /// Mines either pending transactions or an empty block if mempool is empty.
    /// Used for automatic mining to maintain liveness (empty blocks keep timestamp continuity).
    pub async fn mine_block_auto(&mut self, miner_id: String) -> Result<Block, String> {
        let last_block = self.get_last_block().unwrap().clone();
        
        let (transactions, is_empty) = if self.mempool.is_empty() {
            (vec![], true)
        } else {
            (self.mempool.clone(), false)
        };
        
        let mut new_block = Block {
            index: last_block.index + 1,
            timestamp: super::now(),
            prev_hash: last_block.hash.clone(),
            miner_id,
            transactions,
            nonce: 0,
            hash: vec![],
            vrf_output: vec![],           // Will be filled by consensus engine
            vrf_proof: None,              // Will be filled by consensus engine
            merkle_root: vec![],          // Will be calculated
        };

        // Calculate merkle root (empty for empty blocks)
        new_block.merkle_root = Block::calculate_merkle_root(&new_block.transactions);

        // Perform consensus sealing (PoW, PoS, etc.)
        self.mine_block(&mut new_block).await
            .map_err(|e| format!("Failed to seal block: {}", e))?;

        // Commit the mined block locally
        self.blocks.push(new_block.clone());
        
        // Clear mempool only if it wasn't empty (empty block doesn't clear anything)
        if !is_empty {
            self.mempool.clear();
        }

        if is_empty {
            println!("[AUTO_MINE] Empty block {} mined (mempool was empty). Timestamp: {}", new_block.index, new_block.timestamp);
        }

        Ok(new_block)
    }


    // Validates index/parent/hash/consensus against the current chain tip.
    pub fn check_block_valid(&self, block: &Block) -> Result<(), String> {
        let last_block = self.get_last_block().unwrap();
        
        if block.index != last_block.index + 1 {
            return Err(format!("Index mismatch: expected {}, got {}", last_block.index + 1, block.index));
        }
        
        if block.prev_hash != last_block.hash {
            return Err(format!("Prev hash mismatch: expected {}, got {}", 
                hex::encode(&last_block.hash), hex::encode(&block.prev_hash)));
        }

        // Validate merkle root
        let expected_merkle_root = Block::calculate_merkle_root(&block.transactions);
        if block.merkle_root != expected_merkle_root {
            return Err(format!("Merkle root mismatch: expected {}, got {}",
                hex::encode(&expected_merkle_root), hex::encode(&block.merkle_root)));
        }

        // Validate recalculated hash
        let recalculated_hash = block.hash();
        if block.hash != recalculated_hash {
            // Log details for debugging hashing issues
            println!("[LEDGER DEBUG] Hash mismatch details for block {}:", block.index);
            println!("  Stored Hash: {}", hex::encode(&block.hash));
            println!("  Recalc Hash: {}", hex::encode(&recalculated_hash));
            println!("  Miner ID:    {}", block.miner_id);
            println!("  Timestamp:   {}", block.timestamp);
            println!("  Nonce:       {}", block.nonce);
            println!("  VRF Output:  {:?}", block.vrf_output);
            
            return Err(format!("Block hash tampered or invalid. Recalculated hash does not match stored hash."));
        }

        // Validate using the consensus engine
        if let Err(e) = self.consensus_engine.validate_block(block) {
            return Err(format!("Consensus validation failed: {}", e));
        }

        Ok(())
    }

    pub fn is_block_valid(&self, block: &Block) -> bool {
        self.check_block_valid(block).is_ok()
    }


    // Applies a command to a provided state map, with nonce validation for bids.
    // sender_id is used for authorization checks (e.g., only auction creator can close)
    fn apply_tx_to_state(
        state: &mut HashMap<String, Auction>,
        nonces: &mut HashMap<String, u64>,
        command: &AuctionCommand,
        sender_id: Option<&str>,
        enforce_auth: bool,
    ) -> Result<(), String> {
        match command {
            AuctionCommand::CreateAuction { auction_id, item_name, starting_price, description } => {
                if state.contains_key(auction_id) {
                    return Err("Auction ID already exists.".to_string());
                }
                let creator = sender_id.unwrap_or("").to_string();
                
                // Off-chain storage pattern: if description is present, we record the key
                // used in the DHT (auction_id + "_meta") as the metadata_key in the blockchain.
                let metadata_key = description.as_ref().map(|_| format!("{}_meta", auction_id));

                let new_auction = Auction::new(auction_id.clone(), creator, item_name.clone(), *starting_price, metadata_key);
                state.insert(auction_id.clone(), new_auction);
            },
            AuctionCommand::StartAuction { auction_id } => {
                let auction = state.get_mut(auction_id).ok_or("Auction not found.")?;
                auction.start()?;
            },
            AuctionCommand::PlaceBid { auction_id, bid_amount, bidder_id, nonce } => {
                // Validate nonce to prevent replay attacks
                let expected_nonce = nonces.entry(bidder_id.clone()).or_insert(0);
                if *nonce != *expected_nonce {
                    return Err(format!(
                        "Invalid nonce for bidder {}. Expected {}, got {}",
                        bidder_id, expected_nonce, nonce
                    ));
                }
                
                // Process the bid
                let auction = state.get_mut(auction_id).ok_or("Auction not found.")?;
                auction.process_bid(*bid_amount, bidder_id.clone())?;
                
                // Increment nonce after successful bid
                *expected_nonce += 1;
            },
            AuctionCommand::CloseAuction { auction_id } => {
                let auction = state.get_mut(auction_id).ok_or("Auction not found.")?;
                // When applying live commands, authentication is mandatory — unless the auction
                // itself was created without a known creator (legacy unsigned /command flow):
                // in that case no signer can ever match, so we permit anonymous close.
                if enforce_auth && !auction.creator_id.is_empty() {
                    let sender = sender_id.ok_or(
                        "Auction closure requires authentication (sender_id missing).".to_string()
                    )?;
                    if auction.creator_id != sender {
                        return Err("Only the auction creator can close this auction.".to_string());
                    }
                }
                auction.close()?;
            }
        }
        Ok(())
    }

    // Builds a stable key for command identity/deduplication using SHA-256 of its binary form.
    fn command_key(command: &AuctionCommand) -> Result<String, String> {
        let mut hasher = Sha256::new();
        hasher.update(command.bytes());
        Ok(hex::encode(hasher.finalize()))
    }


    // Validates and applies a block mined by another peer.
    pub fn add_external_block(&mut self, block: Block) -> Result<(), String> {
        // 1) Reject early if cryptographic checks fail.
        self.check_block_valid(&block)?;

        // 2) Clone current state for atomic validation.
        let mut temp_state = self.active_auctions.clone();
        let mut temp_nonces = self.bidder_nonces.clone();
        let mempool_keys: HashSet<String> = self
            .mempool
            .iter()
            .map(Self::command_key)
            .collect::<Result<HashSet<_>, _>>()?;

        // 3) Replay only unseen commands on the temp state.
        // Commands already in mempool are assumed to already be reflected in active_auctions.
        for tx in &block.transactions {
            let tx_key = Self::command_key(tx)?;
            if !mempool_keys.contains(&tx_key) {
                // Block validation: transactions in blocks don't have sender_id context, use None
                if let Err(e) = Self::apply_tx_to_state(&mut temp_state, &mut temp_nonces, tx, None, false) {
                    return Err(format!("Atomic block validation failed: {}", e));
                }
            }
        }

        // 4) Commit atomically after all commands validate.
        self.active_auctions = temp_state;
        self.bidder_nonces = temp_nonces;
        
        // Remove commands included in the accepted block from local mempool.
        let block_keys: HashSet<String> = block
            .transactions
            .iter()
            .map(Self::command_key)
            .collect::<Result<HashSet<_>, _>>()?;
        self.mempool
            .retain(|m| Self::command_key(m).map_or(true, |key| !block_keys.contains(&key)));

        self.blocks.push(block);

        Ok(())
    }
    
    // Validates and applies a local command, then queues it for mining.
    // sender_id is optional and used for authorization checks
    pub fn process_command(&mut self, command: AuctionCommand, sender_id: Option<&str>) -> Result<(), String> {

        // Trace incoming command for debugging mempool behavior
        println!("process_command: sender_id={:?}", sender_id);
        println!("process_command: command={:?}", command);

        // Check if command already in mempool
        let command_key = Self::command_key(&command)?;

        if self.mempool.iter().any(|m| Self::command_key(m) == Ok(command_key.clone())) {
            println!("process_command: duplicate command detected, key={}", command_key);
            return Err("Duplicate command already in mempool".to_string());
        }

        // Apply to state (this will perform nonce/auth checks)
        Self::apply_tx_to_state(&mut self.active_auctions, &mut self.bidder_nonces, &command, sender_id, true)?;
        println!("process_command: apply_tx_to_state succeeded, key={}", command_key);

        // Push to mempool and report new size
        let before = self.mempool.len();
        println!("process_command: mempool_size_before_push={}", before);
        self.mempool.push(command);
        println!("process_command: pushed command, mempool_size_after_push={}", self.mempool.len());

        Ok(())

    }

    // Replaces local chain with a longer valid chain and rebuilds state from it.
    pub fn replace_chain(&mut self, new_blocks: Vec<Block>) -> Result<(), String> {
        if new_blocks.len() <= self.blocks.len() {
            return Err("Received chain is not longer than the current chain.".to_string());
        }

        let mut temp_state: HashMap<String, Auction> = HashMap::new();
        let mut temp_nonces: HashMap<String, u64> = HashMap::new();

        for i in 1..new_blocks.len() {
            let current = &new_blocks[i];
            let previous = &new_blocks[i - 1];

            if current.index != previous.index + 1 || current.prev_hash != previous.hash {
                return Err(format!("Invalid link at block {}", current.index));
            }

            if current.hash != current.hash() {
                return Err(format!("Tampered hash at block {}", current.index));
            }

            // Skip consensus validation for blocks already in local chain with same hash (catch-up optimization)
            if i < self.blocks.len() && self.blocks[i].hash == current.hash {
                // Block already validated locally; skip redundant validation
                for tx in &current.transactions {
                    if let Err(e) = Self::apply_tx_to_state(&mut temp_state, &mut temp_nonces, tx, None, false) {
                        return Err(format!("Invalid tx in block {}: {}", current.index, e));
                    }
                }
                continue;
            }

            // Validate using consensus engine (only for new blocks)
            if let Err(e) = self.consensus_engine.validate_block(current) {
                return Err(format!("Invalid consensus at block {}: {}", current.index, e));
            }

            for tx in &current.transactions {
                // External chain validation: no sender_id context available
                if let Err(e) = Self::apply_tx_to_state(&mut temp_state, &mut temp_nonces, tx, None, false) {
                    return Err(format!("Invalid tx in block {}: {}", current.index, e));
                }
            }
        }

        println!("FORK RESOLVED! Adopting new chain (Length: {})", new_blocks.len());
        self.blocks = new_blocks;
        self.active_auctions = temp_state;
        self.bidder_nonces = temp_nonces;
        self.mempool.clear();

        Ok(())
    }

    /// Finality Tracking: Returns number of confirmations for a block at given index.
    /// A block at index 100 with current height 106 has 6 confirmations.
    pub fn get_confirmations(&self, block_index: usize) -> usize {
        let current_height = self.blocks.len();
        if block_index >= current_height {
            return 0;
        }
        current_height - block_index
    }

    /// Checks if a block is "final" (cannot be reorged).
    /// Rule: A block is final after 6 confirmations (arbitrary but common).
    /// With PoW at 3 zeros (~6ms/block), finality = ~36ms.
    /// With Bitcoin (79 zeros, ~10min/block), finality = ~60 minutes.
    pub fn is_block_final(&self, block_index: usize) -> bool {
        self.get_confirmations(block_index) >= 6
    }

    /// Checks if a specific transaction (in a specific block) is final.
    pub fn is_transaction_final(&self, block_index: usize, tx_index: usize) -> bool {
        if block_index >= self.blocks.len() {
            return false;
        }
        
        if tx_index >= self.blocks[block_index].transactions.len() {
            return false;
        }
        
        // Transaction is final if its block is final
        self.is_block_final(block_index)
    }

    /// Returns finality info for a given block.
    /// Useful for APIs and monitoring.
    pub fn get_block_finality_info(&self, block_index: usize) -> Option<serde_json::Value> {
        if block_index >= self.blocks.len() {
            return None;
        }
        
        let confirmations = self.get_confirmations(block_index);
        let is_final = self.is_block_final(block_index);
        
        Some(serde_json::json!({
            "block_index": block_index,
            "confirmations": confirmations,
            "is_final": is_final,
            "confirmations_needed_for_finality": if confirmations >= 6 { 
                0 
            } else { 
                6 - confirmations 
            },
        }))
    }
}
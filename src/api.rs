use axum::{
    extract::{State, Path},
    routing::{get, post},
    Json, Router,
};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use serde_json::Value;

use crate::blockchain::Blockchain;
use crate::blockchain::Block;
use crate::auction::{AuctionCommand, SignedTransaction};
use crate::p2p::node::LocalCommand;
use crate::metrics::{SharedMetrics, Metrics};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

// Mining trigger source: interval-based auto-mining or manual /mine command
#[derive(Clone, Debug)]
pub enum MiningTrigger {
    Interval,  // From AUTO_MINE interval timer
    Manual,    // From /mine stdin command
}

// Shared state between HTTP requests and our P2P Node
#[derive(Clone)]
pub struct AppState {
    pub ledger: Arc<Mutex<Blockchain>>,
    pub tx: mpsc::Sender<LocalCommand>,
    pub metrics: SharedMetrics,
    pub peer_id: Arc<Mutex<String>>,  // Peer identifier for mining
    pub mining_cmd_tx: mpsc::Sender<MiningTrigger>,  // Mining command channel
}

#[derive(Deserialize)]
struct DhtPutRequest {
    key: String,
    value: String,
}

#[derive(Deserialize)]
struct RichAuctionRequest {
    auction_id: String,
    item_name: String,
    description: String,
    starting_price: u64,
}

pub async fn start_api_server(state: AppState) {
    let peer_id = state.peer_id.lock().unwrap().clone();
    println!("[API] Starting with peer_id: {}", peer_id);
    
    // Define the API endpoints
    let app = Router::new()
        .route("/ledger", get(get_ledger))
        .route("/auctions", get(get_auctions))
        .route("/auction/:id", get(get_single_auction))
        .route("/status", get(get_status))
        .route("/command", post(submit_command))
        .route("/command/rich", post(submit_rich_command))
        .route("/command/signed", post(submit_signed_command))
        .route("/finality/block/:index", get(get_block_finality))
        .route("/finality/tx/:block_index/:tx_index", get(get_transaction_finality))
        .route("/metrics", get(get_metrics))
        .route("/admin/mine", post(admin_mine))
        .route("/admin/inject_invalid_block", post(admin_inject_invalid_block))
        .route("/dht/put", post(dht_put))
        .route("/dht/val/:key", get(dht_get))
        .with_state(state);

    // Bind the server to port 8080
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    println!("REST API Server running at http://localhost:8080");
    
    axum::serve(listener, app).await.unwrap();
}

// ... existing handlers ...

// Handler for POST /dht/put
async fn dht_put(
    State(state): State<AppState>,
    Json(payload): Json<DhtPutRequest>,
) -> Json<Value> {
    if let Err(e) = state.tx.send(LocalCommand::PutRecord {
        key: payload.key.clone(),
        value: payload.value.into_bytes(),
    }).await {
        return Json(serde_json::json!({"error": format!("Failed to initiate DHT put: {}", e)}));
    }
    
    Json(serde_json::json!({
        "status": "success",
        "message": format!("STORE (put_record) initiated for key: {}", payload.key)
    }))
}

// Handler for GET /dht/get/:key
async fn dht_get(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Json<Value> {
    if let Err(e) = state.tx.send(LocalCommand::GetRecord { key: key.clone() }).await {
        return Json(serde_json::json!({"error": format!("Failed to initiate DHT get: {}", e)}));
    }
    
    Json(serde_json::json!({
        "status": "success",
        "message": format!("FIND_VALUE (get_record) initiated for key: {}. Check the peer console logs for the result.", key)
    }))
}

// GET /ledger -> Returns the entire blockchain
async fn get_ledger(State(state): State<AppState>) -> Json<Value> {
    let ledger = state.ledger.lock().unwrap();
    Json(serde_json::to_value(&*ledger).unwrap())
}

// GET /auctions -> Returns only the active auctions dictionary
async fn get_auctions(State(state): State<AppState>) -> Json<Value> {
    let ledger = state.ledger.lock().unwrap();
    Json(serde_json::to_value(&ledger.active_auctions).unwrap())
}

// GET /auction/:id -> Returns a single auction by ID
async fn get_single_auction(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let ledger = state.ledger.lock().unwrap();
    match ledger.active_auctions.get(&id) {
        Some(auction) => Json(serde_json::to_value(auction).unwrap()),
        None => Json(serde_json::json!({
            "error": format!("Auction {} not found", id)
        }))
    }
}

// Handler for POST /command/rich
async fn submit_rich_command(
    State(state): State<AppState>,
    Json(payload): Json<RichAuctionRequest>,
) -> Json<Value> {
    // 1. Put metadata (heavy description) in DHT
    let dht_key = format!("{}_meta", payload.auction_id);
    if let Err(e) = state.tx.send(LocalCommand::PutRecord {
        key: dht_key.clone(),
        value: payload.description.clone().into_bytes(),
    }).await {
        return Json(serde_json::json!({"error": format!("Failed to initiate DHT put: {}", e)}));
    }

    // 2. Broadcast light auction data to blockchain
    // The description is included here so the chain can acknowledge its existence
    // and store the DHT reference key, but the heavy content stays off-chain (in DHT).
    let command = AuctionCommand::CreateAuction {
        auction_id: payload.auction_id,
        item_name: payload.item_name,
        starting_price: payload.starting_price,
        description: Some(payload.description), 
    };

    let json_str = serde_json::to_string(&command).unwrap();
    
    if let Err(e) = state.tx.send(LocalCommand::BroadcastAuction(json_str)).await {
        return Json(serde_json::json!({"error": format!("Failed to broadcast: {}", e)}));
    }
    
    Json(serde_json::json!({
        "status": "success",
        "message": "Rich auction created: metadata stored in DHT, light data broadcasted to blockchain",
        "dht_key": dht_key
    }))
}

// POST /command -> Legacy endpoint (accepts unsigned commands for backward compatibility)
async fn submit_command(
    State(state): State<AppState>,
    Json(command): Json<AuctionCommand>,
) -> Json<Value> {
    let json_str = serde_json::to_string(&command).unwrap();
    
    if let Err(e) = state.tx.send(LocalCommand::BroadcastAuction(json_str)).await {
        return Json(serde_json::json!({"error": format!("Failed to broadcast: {}", e)}));
    }
    
    Json(serde_json::json!({
        "status": "success",
        "message": "Command successfully injected into the P2P network and Mempool"
    }))
}

// POST /command/signed -> Accepts cryptographically signed transactions
async fn submit_signed_command(
    State(state): State<AppState>,
    Json(signed_tx): Json<SignedTransaction>,
) -> Json<Value> {
    // Step 1: Verify signature
    if let Err(e) = signed_tx.verify() {
        return Json(serde_json::json!({
            "error": format!("Signature verification failed: {}", e)
        }));
    }

    // Step 2: Verify sender_id matches what's in the command (for PlaceBid)
    if let AuctionCommand::PlaceBid { bidder_id, .. } = &signed_tx.command {
        if bidder_id != &signed_tx.sender_id {
            return Json(serde_json::json!({
                "error": "bidder_id does not match the signer's identity"
            }));
        }
    }

    // Step 3: Broadcast to P2P network as JSON
    let json_str = serde_json::to_string(&signed_tx)
        .map_err(|e| {
            Json(serde_json::json!({
                "error": format!("Failed to serialize: {}", e)
            }))
        })
        .unwrap();
    
    if let Err(e) = state.tx.send(LocalCommand::BroadcastSignedTransaction(json_str)).await {
        return Json(serde_json::json!({"error": format!("Failed to broadcast: {}", e)}));
    }
    
    Json(serde_json::json!({
        "status": "success",
        "message": "Signed transaction successfully verified and injected into the P2P network",
        "sender_id": signed_tx.sender_id
    }))
}

// GET /finality/block/:index -> Returns finality info for a specific block
async fn get_block_finality(
    State(state): State<AppState>,
    Path(index): Path<usize>,
) -> Json<Value> {
    let ledger = state.ledger.lock().unwrap();
    
    if index >= ledger.blocks.len() {
        return Json(serde_json::json!({
            "error": format!("Block {} not found. Chain height: {}", index, ledger.blocks.len())
        }));
    }
    
    match ledger.get_block_finality_info(index) {
        Some(info) => Json(info),
        None => Json(serde_json::json!({
            "error": "Failed to compute finality info"
        }))
    }
}

// GET /finality/tx/:block_index/:tx_index -> Returns finality info for a specific transaction
async fn get_transaction_finality(
    State(state): State<AppState>,
    Path((block_index, tx_index)): Path<(usize, usize)>,
) -> Json<Value> {
    let ledger = state.ledger.lock().unwrap();
    
    if block_index >= ledger.blocks.len() {
        return Json(serde_json::json!({
            "error": format!("Block {} not found", block_index)
        }));
    }
    
    if tx_index >= ledger.blocks[block_index].transactions.len() {
        return Json(serde_json::json!({
            "error": format!("Transaction {} not found in block {}", tx_index, block_index)
        }));
    }
    
    let is_final = ledger.is_transaction_final(block_index, tx_index);
    let confirmations = ledger.get_confirmations(block_index);
    
    Json(serde_json::json!({
        "block_index": block_index,
        "tx_index": tx_index,
        "transaction": ledger.blocks[block_index].transactions[tx_index].clone(),
        "confirmations": confirmations,
        "is_final": is_final,
        "confirmations_needed_for_finality": if confirmations >= 6 { 
            0 
        } else { 
            6 - confirmations 
        },
    }))
}

// GET /metrics -> Returns system metrics for monitoring
async fn get_metrics(State(state): State<AppState>) -> Json<Metrics> {
    let ledger = state.ledger.lock().unwrap();
    let mut metrics = state.metrics.lock().unwrap();
    
    // Update metrics with current state
    metrics.update_mempool_size(ledger.mempool.len());
    
    let current_metrics = metrics.get_metrics();
    Json(current_metrics)
}

// POST /admin/mine -> Trigger mining of pending transactions (non-interactive)
async fn admin_mine(State(state): State<AppState>) -> String {
    // Send mining trigger to unified mining task
    match state.mining_cmd_tx.send(MiningTrigger::Manual).await {
        Ok(_) => {
            r#"{ "status": "success", "message": "Mining triggered. Check /metrics or logs for result." }"#.to_string()
        },
        Err(_) => {
            r#"{ "status": "error", "error": "Mining system not available" }"#.to_string()
        }
    }
}

// POST /admin/inject_invalid_block -> Broadcasts a deliberately invalid block for reputation testing
async fn admin_inject_invalid_block(State(state): State<AppState>) -> Json<Value> {
    let local_peer_id = state.peer_id.lock().unwrap().clone();
    let (block_index, invalid_hash, invalid_block) = {
        let ledger = state.ledger.lock().unwrap();

        let Some(last_block) = ledger.get_last_block().cloned() else {
            return Json(serde_json::json!({
                "status": "error",
                "error": "Ledger is empty"
            }));
        };

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default();

        let invalid_block = Block {
            index: last_block.index + 1,
            timestamp,
            prev_hash: last_block.hash,
            miner_id: local_peer_id.clone(),
            transactions: vec![],
            nonce: 0,
            hash: vec![0xff; 32],
            vrf_output: vec![0; 64],
            vrf_proof: None,
            merkle_root: vec![0; 32],
        };

        let invalid_hash = invalid_block
            .hash
            .iter()
            .map(|byte| format!("{:02x}", byte))
            .collect::<String>();

        (invalid_block.index, invalid_hash, invalid_block)
    };

    if let Some(reputation_tracker) = state
        .ledger
        .lock()
        .unwrap()
        .consensus_engine
        .reputation_tracker()
    {
        if let Ok(mut tracker) = reputation_tracker.lock() {
            if !local_peer_id.is_empty() {
                let _was_banned = tracker.decrement_invalid_block(&local_peer_id);
            }
        }
    }

    if let Err(e) = state.tx.send(LocalCommand::BroadcastBlock(invalid_block)).await {
        return Json(serde_json::json!({
            "status": "error",
            "error": format!("Failed to broadcast invalid block: {}", e)
        }));
    }

    Json(serde_json::json!({
        "status": "success",
        "message": "Invalid block broadcasted to the network",
        "block_index": block_index,
        "hash": invalid_hash,
        "source_peer_id": local_peer_id
    }))
}

// GET /status -> Returns both session metrics and the persisted chain height
async fn get_status(State(state): State<AppState>) -> Json<Value> {
    let ledger = state.ledger.lock().unwrap();
    let mut metrics = state.metrics.lock().unwrap();
    let local_peer_id = state.peer_id.lock().unwrap().clone();

    metrics.update_mempool_size(ledger.mempool.len());

    let reputation_status = ledger
        .consensus_engine
        .reputation_tracker()
        .and_then(|tracker| tracker.lock().ok().map(|tracker| {
            serde_json::json!({
                "peers": tracker.get_all_peers(),
                "banned_peers": tracker.get_banned_peers(),
                "network_stats": tracker.get_network_stats(),
            })
        }))
        .unwrap_or_else(|| serde_json::json!({
            "available": false
        }));

    Json(serde_json::json!({
        "local_peer_id": local_peer_id,
        "ledger_height": ledger.blocks.len(),
        "mempool_size": ledger.mempool.len(),
        "session_metrics": metrics.get_metrics(),
        "reputation": reputation_status,
        "note": "blocks_mined belongs to the current runtime session; ledger_height reflects the persisted chain state"
    }))
}
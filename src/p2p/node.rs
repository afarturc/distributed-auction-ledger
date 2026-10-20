use libp2p::{identity, PeerId, swarm::SwarmEvent, futures::StreamExt, Multiaddr, kad};
use libp2p::request_response::{self, Message as RrMessage};
use std::error::Error;
use tokio::sync::mpsc;
use libp2p::gossipsub::IdentTopic;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use super::behaviour::{LedgerBehaviour, LedgerBehaviourEvent};
use super::sync::{SyncRequest, SyncResponse, MAX_BLOCKS_PER_RESPONSE};
use crate::blockchain::{Blockchain, Block};
use crate::blockchain::storage;
use crate::auction::{AuctionCommand, SignedTransaction};
use crate::metrics::SharedMetrics;

/// Minimum delay between successive sync requests aimed at the same peer.
/// Prevents request storms when several gossipsub blocks arrive in quick succession.
const SYNC_PEER_COOLDOWN: Duration = Duration::from_secs(5);

pub enum LocalCommand {
    BroadcastAuction(String),
    BroadcastSignedTransaction(String),
    BroadcastBlock(Block),
    PutRecord { key: String, value: Vec<u8> },
    GetRecord { key: String },
}

const KEY_FILE: &str = "node_key.bin";

pub async fn start_p2p_node(
    mut command_receiver: mpsc::Receiver<LocalCommand>,
    bootstrap_nodes: Vec<String>,
    shared_ledger: Arc<Mutex<Blockchain>>,
    shared_metrics: SharedMetrics,
    shared_peer_id: Arc<Mutex<String>>,
    external_block_tx: tokio::sync::broadcast::Sender<u64>,
) -> Result<(), Box<dyn Error>> {
    println!("[P2P INIT] Received {} bootstrap nodes: {:?}", bootstrap_nodes.len(), bootstrap_nodes);
    
    let local_key = if Path::new(KEY_FILE).exists() {
        let bytes = fs::read(KEY_FILE)?;
        identity::Keypair::from_protobuf_encoding(&bytes)?
    } else {
        let key = identity::Keypair::generate_ed25519();
        fs::write(KEY_FILE, key.to_protobuf_encoding()?)?;
        key
    };

    let local_peer_id = PeerId::from(local_key.public());
    println!("Secure Identity Loaded/Created: {}", local_peer_id);
    if let Ok(mut peer_id) = shared_peer_id.lock() {
        *peer_id = local_peer_id.to_string();
    }

    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(local_key.clone())
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )?
        .with_behaviour(|_| LedgerBehaviour::new(local_key, local_peer_id))?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(std::time::Duration::from_secs(60)))
        .build();

    let auction_topic = IdentTopic::new("ledger-auctions");
    let block_topic = IdentTopic::new("ledger-blocks");

    swarm.behaviour_mut().gossipsub.subscribe(&auction_topic)?;
    swarm.behaviour_mut().gossipsub.subscribe(&block_topic)?;
    
    swarm.listen_on("/ip4/0.0.0.0/tcp/44000".parse()?)?;

    let mut bootstrap_retry_interval = tokio::time::interval(Duration::from_secs(5));

    // Per-peer cooldown for chain-sync requests (see SYNC_PEER_COOLDOWN).
    let mut last_sync_request: HashMap<PeerId, Instant> = HashMap::new();

    // Proactive chain sync: probes a random connected peer on a regular cadence so
    // nodes that missed gossipsub messages (or rejoined after isolation) can catch up
    // even when they do not see any new block arrive on the wire.
    let mut proactive_sync_interval = tokio::time::interval(Duration::from_secs(30));
    // Don't fire immediately on startup; the HTTP catch-up in main.rs already runs then.
    proactive_sync_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    for addr_str in &bootstrap_nodes {
        match addr_str.parse::<Multiaddr>() {
            Ok(addr) => {
                println!("[P2P] Attempting initial dial to bootstrap {}", addr_str);
                match swarm.dial(addr) {
                    Ok(conn) => println!("[P2P] Dial initiated to {} -> {:?}", addr_str, conn),
                    Err(e) => eprintln!("[P2P] Failed to dial bootstrap {}: {}", addr_str, e),
                }
            },
            Err(e) => eprintln!("[P2P] Invalid bootstrap multiaddr {}: {}", addr_str, e),
        }
    }

    loop {
        tokio::select! {
            _ = bootstrap_retry_interval.tick() => {
                if swarm.connected_peers().count() == 0 {
                    for addr_str in &bootstrap_nodes {
                        match addr_str.parse::<Multiaddr>() {
                            Ok(addr) => {
                                println!("[P2P] Retry dial to bootstrap {}", addr_str);
                                if let Err(e) = swarm.dial(addr) {
                                    eprintln!("[P2P] Retry failed to dial {}: {}", addr_str, e);
                                }
                            },
                            Err(e) => eprintln!("[P2P] Retry invalid multiaddr {}: {}", addr_str, e),
                        }
                    }
                }
            },

            _ = proactive_sync_interval.tick() => {
                // Pick one connected peer at random and ask for its full chain.
                // Used to recover from eclipses / fork drift where no block has
                // arrived recently to act as a reactive sync trigger.
                let peers: Vec<PeerId> = swarm.connected_peers().copied().collect();
                if peers.is_empty() {
                    continue;
                }
                use rand::seq::SliceRandom;
                let mut rng = rand::thread_rng();
                if let Some(peer) = peers.choose(&mut rng).copied() {
                    let now = Instant::now();
                    let cooldown_ok = last_sync_request
                        .get(&peer)
                        .map(|t| now.duration_since(*t) >= SYNC_PEER_COOLDOWN)
                        .unwrap_or(true);
                    if cooldown_ok {
                        last_sync_request.insert(peer, now);
                        let local_height = shared_ledger.lock().unwrap().blocks.len();
                        let req = SyncRequest { from_index: 0 };
                        let req_id = swarm.behaviour_mut().sync.send_request(&peer, req);
                        println!(
                            "[SYNC] Proactive probe to {} (local height {}, req {:?})",
                            peer, local_height, req_id
                        );
                    }
                }
            },
            event = swarm.select_next_some() => match event {
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("Node active and listening on secure TCP address: {}", address);
                }
                SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                    println!("[P2P ERROR] Outgoing connection error to {:?}: {}", peer_id, error);
                }
                SwarmEvent::IncomingConnectionError { local_addr, send_back_addr, error, .. } => {
                    println!("[P2P ERROR] Incoming connection error on {} from {}: {}", local_addr, send_back_addr, error);
                }
                SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                    println!("Connection established with peer: {}", peer_id);
                    swarm.behaviour_mut().kademlia.add_address(&peer_id, endpoint.get_remote_address().clone());
                    let connected = swarm.connected_peers().count();
                    println!("[P2P] Connected peers now: {}", connected);
                    if let Ok(mut metrics) = shared_metrics.lock() {
                        metrics.update_peers_connected(connected);
                    }
                }
                SwarmEvent::ConnectionClosed { peer_id, .. } => {
                    println!("Connection closed with peer: {}", peer_id);
                    let connected = swarm.connected_peers().count();
                    println!("[P2P] Connected peers now: {}", connected);
                    if let Ok(mut metrics) = shared_metrics.lock() {
                        metrics.update_peers_connected(connected);
                    }
                }
                SwarmEvent::Behaviour(LedgerBehaviourEvent::Kademlia(kad::Event::OutboundQueryProgressed { result, .. })) => {
                    match result {
                        kad::QueryResult::GetRecord(Ok(ok_result)) => {
                            match ok_result {
                                kad::GetRecordOk::FoundRecord(peer_record) => {
                                    let key = String::from_utf8_lossy(&peer_record.record.key.as_ref());
                                    let value = String::from_utf8_lossy(&peer_record.record.value);
                                    println!("\n[DHT SUCCESS] FIND_VALUE Result: Key={}, Value={}", key, value);
                                }
                                kad::GetRecordOk::FinishedWithNoAdditionalRecord { .. } => {}
                            }
                        }
                        kad::QueryResult::GetRecord(Err(e)) => {
                            println!("\n[DHT ERROR] FIND_VALUE Failed: {:?}", e);
                        }
                        kad::QueryResult::PutRecord(Ok(kad::PutRecordOk { key })) => {
                            let key_str = String::from_utf8_lossy(&key.as_ref());
                            println!("\n[DHT SUCCESS] STORE completed for Key={}", key_str);
                        }
                        kad::QueryResult::PutRecord(Err(e)) => {
                            println!("\n[DHT ERROR] STORE Failed: {:?}", e);
                        }
                        _ => {}
                    }
                }
                SwarmEvent::Behaviour(LedgerBehaviourEvent::Gossipsub(libp2p::gossipsub::Event::Message {
                    propagation_source, message, ..
                })) => {
                    if message.topic.as_str() == "ledger-auctions" {
                        // Try signed transaction first
                        if let Ok(signed_tx) = serde_json::from_slice::<SignedTransaction>(&message.data) {
                            println!("\n[NETWORK] Received SignedTransaction from {}", propagation_source);
                            if let Err(e) = signed_tx.verify() {
                                println!("[NETWORK ERROR] Signature verification failed: {}", e);
                            } else {
                                let mut ledger = shared_ledger.lock().unwrap();
                                // Pass sender_id for authorization checks
                                match ledger.process_command(signed_tx.command.clone(), Some(&signed_tx.sender_id)) {
                                    Ok(_) => {
                                        println!("[LEDGER] Signed transaction added to Mempool. Size: {}", ledger.mempool.len());
                                        if let Ok(mut metrics) = shared_metrics.lock() {
                                            metrics.increment_transactions();
                                            metrics.update_mempool_size(ledger.mempool.len());
                                        }
                                    },
                                    Err(e) => println!("[LEDGER ERROR] Rejected: {}", e),
                                }
                            }
                        } 
                        // Fall back to unsigned command for backward compatibility
                        else if let Ok(command) = serde_json::from_slice::<AuctionCommand>(&message.data) {
                            println!("\n[NETWORK] Received AuctionCommand from {}", propagation_source);
                            let mut ledger = shared_ledger.lock().unwrap();
                            // Unsigned commands: no sender_id available
                            match ledger.process_command(command, None) {
                                Ok(_) => {
                                    println!("[LEDGER] Added to Mempool. Size: {}", ledger.mempool.len());
                                    if let Ok(mut metrics) = shared_metrics.lock() {
                                        metrics.increment_transactions();
                                        metrics.update_mempool_size(ledger.mempool.len());
                                    }
                                },
                                Err(e) => println!("[LEDGER ERROR] Rejected: {}", e),
                            }
                        }
                    }
                    else if message.topic.as_str() == "ledger-blocks" {
                        match serde_json::from_slice::<Block>(&message.data) {
                            Ok(block) => {
                                println!("\n[NETWORK] Received newly mined Block {} from {}", block.index, propagation_source);
                                let block_height = block.index as u64;
                                // Capture identifying info before the block is consumed by add_external_block.
                                let miner_id = block.miner_id.clone();
                                let block_tx_count = block.transactions.len();
                                let mut ledger = shared_ledger.lock().unwrap();
                                match ledger.add_external_block(block) {
                                    Ok(_) => {
                                        println!("[LEDGER] External block verified and added. Chain height: {}", ledger.blocks.len());
                                        if let Err(e) = storage::save_ledger(&ledger) {
                                            eprintln!("[LEDGER WARNING] Failed to persist external block to disk: {}", e);
                                        }
                                        // Reflect cluster-wide mining progress in this node's metrics.
                                        if let Ok(mut metrics) = shared_metrics.lock() {
                                            metrics.record_block_received(block_tx_count);
                                            metrics.update_mempool_size(ledger.mempool.len());
                                        }
                                        let _ = external_block_tx.send(block_height);
                                    },
                                    Err(e) => {
                                        println!("[LEDGER ERROR] Rejected external block: {}", e);

                                        // Forks / gaps are normal: two miners may legitimately propose
                                        // at the same height, or we may have missed an intermediate
                                        // block (gossipsub does not retransmit history).
                                        let is_index_gap = e.starts_with("Index mismatch");
                                        let is_prev_mismatch = e.starts_with("Prev hash mismatch");

                                        if is_index_gap || is_prev_mismatch {
                                            // Trigger a chain-sync request against the relayer if the
                                            // incoming block is ahead of us. The relayer is the closest
                                            // peer that demonstrably has a longer chain.
                                            let local_height = ledger.blocks.len() as u64;
                                            drop(ledger); // release lock before async behaviour work

                                            if block_height >= local_height {
                                                let now = Instant::now();
                                                let cooldown_ok = last_sync_request
                                                    .get(&propagation_source)
                                                    .map(|t| now.duration_since(*t) >= SYNC_PEER_COOLDOWN)
                                                    .unwrap_or(true);
                                                if cooldown_ok {
                                                    last_sync_request.insert(propagation_source, now);
                                                    let req = SyncRequest { from_index: 0 };
                                                    let req_id = swarm
                                                        .behaviour_mut()
                                                        .sync
                                                        .send_request(&propagation_source, req);
                                                    println!(
                                                        "[SYNC] Detected gap (local height {}, saw block {}). Requesting full chain from {} (req {:?})",
                                                        local_height, block_height, propagation_source, req_id
                                                    );
                                                }
                                            }
                                            continue;
                                        }

                                        // Real protocol violation: blame the block author, not the relay.
                                        if let Some(tracker) = ledger.consensus_engine.reputation_tracker() {
                                            if let Ok(mut tracker) = tracker.lock() {
                                                let was_banned = tracker.decrement_invalid_block(&miner_id);
                                                println!("[REPUTATION] Penalized miner {} for producing an invalid block", miner_id);

                                                if was_banned {
                                                    println!("[BFT] Miner {} banned for repeated invalid blocks", miner_id);
                                                    // Try to disconnect the miner if it's a directly-connected peer.
                                                    if let Ok(banned_peer) = miner_id.parse::<PeerId>() {
                                                        let _ = swarm.disconnect_peer_id(banned_peer);
                                                    }
                                                }
                                            }
                                        }
                                    },
                                }
                            },
                            Err(e) => {
                                // Malformed payload: the relay is at fault here (couldn't even deserialize).
                                println!("[LEDGER ERROR] Malformed block payload from {}: {}", propagation_source, e);
                                let ledger = shared_ledger.lock().unwrap();
                                if let Some(tracker) = ledger.consensus_engine.reputation_tracker() {
                                    if let Ok(mut tracker) = tracker.lock() {
                                        let was_banned = tracker.decrement_invalid_block(&propagation_source.to_string());
                                        println!("[REPUTATION] Penalized relay {} for forwarding a malformed block", propagation_source);
                                        if was_banned {
                                            println!("[BFT] Disconnecting banned peer {}", propagation_source);
                                            let _ = swarm.disconnect_peer_id(propagation_source);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                SwarmEvent::Behaviour(LedgerBehaviourEvent::Sync(sync_event)) => {
                    match sync_event {
                        request_response::Event::Message { peer, message } => {
                            match message {
                                RrMessage::Request { request, channel, .. } => {
                                    let blocks = {
                                        let ledger = shared_ledger.lock().unwrap();
                                        let from = request.from_index as usize;
                                        if from >= ledger.blocks.len() {
                                            Vec::new()
                                        } else {
                                            let end = (from + MAX_BLOCKS_PER_RESPONSE).min(ledger.blocks.len());
                                            ledger.blocks[from..end].to_vec()
                                        }
                                    };
                                    println!(
                                        "[SYNC] Serving {} blocks (from index {}) to {}",
                                        blocks.len(), request.from_index, peer
                                    );
                                    let response = SyncResponse { blocks };
                                    if swarm.behaviour_mut().sync.send_response(channel, response).is_err() {
                                        println!("[SYNC ERROR] Failed to send response to {} (channel closed)", peer);
                                    }
                                }
                                RrMessage::Response { response, .. } => {
                                    let received = response.blocks.len();
                                    if received == 0 {
                                        println!("[SYNC] Empty response from {} (peer has no new blocks)", peer);
                                        continue;
                                    }
                                    println!("[SYNC] Received {} blocks from {}", received, peer);
                                    let mut ledger = shared_ledger.lock().unwrap();
                                    let local_height = ledger.blocks.len();

                                    let starts_at_genesis = response.blocks.first()
                                        .map(|b| b.index == 0)
                                        .unwrap_or(false);

                                    if starts_at_genesis && response.blocks.len() > local_height {
                                        match ledger.replace_chain(response.blocks) {
                                            Ok(_) => {
                                                println!(
                                                    "[SYNC] Chain replaced. New height: {}",
                                                    ledger.blocks.len()
                                                );
                                                if let Err(e) = storage::save_ledger(&ledger) {
                                                    eprintln!("[LEDGER WARNING] Failed to persist synced chain: {}", e);
                                                }
                                                // Rebuild metrics counters from the full new chain so
                                                // they reflect cluster-wide state after a replace.
                                                if let Ok(mut metrics) = shared_metrics.lock() {
                                                    let tx_total: usize = ledger.blocks.iter()
                                                        .map(|b| b.transactions.len())
                                                        .sum();
                                                    metrics.reset_chain_counters(ledger.blocks.len(), tx_total);
                                                    metrics.update_mempool_size(ledger.mempool.len());
                                                }
                                                let _ = external_block_tx.send(ledger.blocks.len() as u64);
                                            }
                                            Err(e) => {
                                                println!("[SYNC ERROR] replace_chain rejected response from {}: {}", peer, e);
                                            }
                                        }
                                    } else {
                                        let mut applied = 0usize;
                                        let mut applied_tx_count = 0usize;
                                        for block in response.blocks {
                                            if (block.index as usize) < ledger.blocks.len() {
                                                continue;
                                            }
                                            let tx_count = block.transactions.len();
                                            match ledger.add_external_block(block) {
                                                Ok(_) => {
                                                    applied += 1;
                                                    applied_tx_count += tx_count;
                                                },
                                                Err(e) => {
                                                    println!("[SYNC ERROR] Stopping at invalid block from {}: {}", peer, e);
                                                    break;
                                                }
                                            }
                                        }
                                        if applied > 0 {
                                            println!("[SYNC] Applied {} new blocks. Height: {}", applied, ledger.blocks.len());
                                            if let Err(e) = storage::save_ledger(&ledger) {
                                                eprintln!("[LEDGER WARNING] Failed to persist synced blocks: {}", e);
                                            }
                                            if let Ok(mut metrics) = shared_metrics.lock() {
                                                metrics.record_block_received(applied_tx_count);
                                                metrics.update_mempool_size(ledger.mempool.len());
                                            }
                                            let _ = external_block_tx.send(ledger.blocks.len() as u64);
                                        }
                                    }
                                }
                            }
                        }
                        request_response::Event::OutboundFailure { peer, error, .. } => {
                            println!("[SYNC ERROR] Outbound sync to {} failed: {:?}", peer, error);
                            last_sync_request.remove(&peer);
                        }
                        request_response::Event::InboundFailure { peer, error, .. } => {
                            println!("[SYNC ERROR] Inbound sync from {} failed: {:?}", peer, error);
                        }
                        request_response::Event::ResponseSent { .. } => {}
                    }
                }
                _ => {}
            },

            Some(comando) = command_receiver.recv() => {
                match comando {
                    LocalCommand::BroadcastAuction(json_str) => {
                        // Process LOCALLY first (This adds it to the Mempool!)
                        if let Ok(command) = serde_json::from_str::<AuctionCommand>(&json_str) {
                            let mut ledger = shared_ledger.lock().unwrap();
                            // Local broadcast: no sender_id available for unsigned commands
                            match ledger.process_command(command, None) {
                                Ok(_) => {
                                    println!("[LOCAL LEDGER] Added to Mempool. Size: {}", ledger.mempool.len());
                                    if let Ok(mut metrics) = shared_metrics.lock() {
                                        metrics.increment_transactions();
                                        metrics.update_mempool_size(ledger.mempool.len());
                                    }
                                },
                                Err(e) => println!("[LOCAL LEDGER ERROR] Rejected: {}", e),
                            }
                        } else {
                            println!("[LOCAL LEDGER ERROR] Failed to parse command JSON.");
                        }

                        // 2. Broadcast to network (Ignore error if no peers are connected)
                        if let Err(_) = swarm.behaviour_mut().gossipsub.publish(auction_topic.clone(), json_str.as_bytes()) {
                            println!("[NETWORK WARNING] No peers connected to broadcast. Saved locally only.");
                        } else {
                            println!("[NETWORK] Command broadcasted.");
                        }
                    },
                    LocalCommand::BroadcastSignedTransaction(json_str) => {
                        // Process LOCALLY first
                        if let Ok(signed_tx) = serde_json::from_str::<SignedTransaction>(&json_str) {
                            // Verify signature
                            if let Err(e) = signed_tx.verify() {
                                println!("[LOCAL LEDGER ERROR] Signed transaction verification failed: {}", e);
                            } else {
                                let mut ledger = shared_ledger.lock().unwrap();
                                // Pass sender_id for authorization checks
                                match ledger.process_command(signed_tx.command.clone(), Some(&signed_tx.sender_id)) {
                                    Ok(_) => {
                                        println!("[LOCAL LEDGER] Signed transaction added to Mempool. Size: {}", ledger.mempool.len());
                                        if let Ok(mut metrics) = shared_metrics.lock() {
                                            metrics.increment_transactions();
                                            metrics.update_mempool_size(ledger.mempool.len());
                                        }
                                    },
                                    Err(e) => println!("[LOCAL LEDGER ERROR] Rejected: {}", e),
                                }
                            }
                        } else {
                            println!("[LOCAL LEDGER ERROR] Failed to parse signed transaction JSON.");
                        }

                        // 2. Broadcast to network
                        if let Err(_) = swarm.behaviour_mut().gossipsub.publish(auction_topic.clone(), json_str.as_bytes()) {
                            println!("[NETWORK WARNING] No peers connected to broadcast. Saved locally only.");
                        } else {
                            println!("[NETWORK] Signed transaction broadcasted.");
                        }
                    },
                    LocalCommand::BroadcastBlock(block) => {
                        let block_json = serde_json::to_string(&block).unwrap();
                        if let Err(_) = swarm.behaviour_mut().gossipsub.publish(block_topic.clone(), block_json.as_bytes()) {
                            println!("[NETWORK WARNING] No peers connected to broadcast block. Saved locally only.");
                        } else {
                            println!("[NETWORK] Block successfully broadcasted to the network.");
                        }
                    },
                    LocalCommand::PutRecord { key, value } => {
                        use libp2p::kad::Record;
                        let record = Record {
                            key: libp2p::kad::RecordKey::new(&key),
                            value,
                            publisher: None,
                            expires: None,
                        };
                        if let Err(e) = swarm.behaviour_mut().kademlia.put_record(record, libp2p::kad::Quorum::One) {
                            println!("[DHT ERROR] Failed to put record: {:?}", e);
                        } else {
                            println!("[DHT] Put record initiated for key: {}", key);
                        }
                    },
                    LocalCommand::GetRecord { key } => {
                        let query_id = swarm.behaviour_mut().kademlia.get_record(libp2p::kad::RecordKey::new(&key));
                        println!("[DHT] Get record initiated for key: {}, Query ID: {:?}", key, query_id);
                    }
                }
            }
        }
    }
}
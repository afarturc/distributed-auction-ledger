pub mod blockchain;
pub mod p2p;
pub mod auction;
pub mod api;
pub mod wallet;
pub mod metrics;

use blockchain::Blockchain;
use blockchain::ProofOfWorkEngine;
use blockchain::ProofOfStakeEngine;
use blockchain::storage; 
use p2p::node::LocalCommand;
use p2p::ReputationTracker;
use metrics::create_metrics;
use api::MiningTrigger;
use tokio::sync::mpsc;
use tokio::sync::broadcast;
use std::io::{self, BufRead};
use std::env;

// Load .env file on startup (for development)
fn load_env() {
    if let Err(_) = dotenvy::dotenv() {
        // .env file not found, which is fine (can use environment variables instead)
    }
}
use std::sync::{Arc, Mutex};
use std::time::Instant;

fn bootstrap_ledger_url(addr: &str) -> Option<String> {
    let parts: Vec<&str> = addr.split('/').collect();
    if parts.len() > 2 {
        match parts[1] {
            "ip4" | "dns4" => Some(format!("http://{}:8080/ledger", parts[2])),
            _ => None,
        }
    } else {
        None
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    load_env();
    println!("--- STARTING SECURE LEDGER ---");

    // WALLET: Generate or load encrypted keys
    let wallet_password = env::var("LEDGER_WALLET_PASSWORD")
        .map_err(|_| "Missing LEDGER_WALLET_PASSWORD environment variable. Set it via .env file or export LEDGER_WALLET_PASSWORD=...")?;
    let wallet = match wallet::Wallet::load_from_disk(&wallet_password) {
        Ok(w) => {
            println!("Wallet loaded from disk");
            w
        },
        Err(_) => {
            let new_wallet = wallet::Wallet::generate();
            new_wallet.save_to_disk(&wallet_password)?;
            println!("New wallet generated and saved to ~/.ledger/keys.enc");
            new_wallet
        }
    };
    println!("Wallet address: {}", wallet.sender_id_hex());

    // Determine consensus mode from environment (POW or POS)
    let consensus_mode = env::var("CONSENSUS_MODE").unwrap_or_else(|_| "POW".to_string());

    // mutable
    let mut initial_ledger = match storage::load_ledger() {
        Some(loaded_chain) => {
            println!("Ledger loaded from disk. Chain height: {}", loaded_chain.blocks.len());
            loaded_chain
        },
        None => {
            // Create consensus engine according to selected mode
            let engine: Arc<dyn blockchain::consensus::ConsensusEngine> = match consensus_mode.as_str() {
                "POS" | "PoS" | "pos" => Arc::new(ProofOfStakeEngine::new(Arc::new(
                    Mutex::new(ReputationTracker::new()),
                ))),
                _ => Arc::new(ProofOfWorkEngine { difficulty: 5 }),
            };
            let new_chain = Blockchain::new(engine);
            println!("No local ledger found. Genesis Block created. Chain height: {} (mode={})", new_chain.blocks.len(), consensus_mode);
            new_chain
        }
    };
    
    let args: Vec<String> = env::args().collect();
    let bootstrap_nodes: Vec<String> = if args.len() > 1 {
        args[1..].to_vec()
    } else {
        Vec::new()
    };
    println!("[MAIN] Command line args: {:?}", args);
    println!("[MAIN] Bootstrap nodes parsed: {:?}", bootstrap_nodes);

    // CATCH-UP MECHANISM (RESILIENCE TO NODES ENTERING/LEAVING)
    let mut caught_up = false;
    for addr in &bootstrap_nodes {
        let Some(url) = bootstrap_ledger_url(addr) else {
            println!("Skipping unsupported bootstrap address format: {}", addr);
            continue;
        };

        println!("Attempting to catch up state from Bootstrap node at {}...", url);

        match reqwest::get(&url).await {
            Ok(response) if response.status().is_success() => {
                if let Ok(downloaded_chain) = response.json::<Blockchain>().await {
                    if downloaded_chain.blocks.len() > initial_ledger.blocks.len() {
                        match initial_ledger.replace_chain(downloaded_chain.blocks) {
                            Ok(_) => {
                                println!("Catch-up/Fork resolved successfully! Height: {}.", initial_ledger.blocks.len());
                                let _ = storage::save_ledger(&initial_ledger);
                                caught_up = true;
                                break;
                            },
                            Err(e) => println!("Rejected network chain: {}", e),
                        }
                    } else {
                        println!("Local chain is already up to date.");
                    }
                }
            },
            _ => println!("Could not reach Bootstrap node at {}. Trying next bootstrap if available.", url),
        }
    }

    if !caught_up && !bootstrap_nodes.is_empty() {
        println!("Could not sync from any bootstrap node. Proceeding with local state.");
    }


    // now pass the fully synced ledger to the mutex
    let shared_ledger = Arc::new(Mutex::new(initial_ledger));
    let shared_metrics = create_metrics();
    let shared_peer_id = Arc::new(Mutex::new(String::new()));

    // Broadcast channel for notifying auto-mining when external blocks arrive
    let (external_block_tx, _) = broadcast::channel::<u64>(10);

    let (tx, rx) = mpsc::channel::<LocalCommand>(32);
    
    // Channel for mining triggers (from interval or manual /mine command)
    let (mining_cmd_tx, mining_cmd_rx) = mpsc::channel::<MiningTrigger>(32);
    let mining_cmd_tx_cli = mining_cmd_tx.clone();  // For stdin loop
    let mining_cmd_tx_api = mining_cmd_tx.clone();  // For API /admin/mine
    
    let ledger_for_p2p = shared_ledger.clone();
    let metrics_for_p2p = shared_metrics.clone();
    let peer_id_for_p2p = shared_peer_id.clone();
    let external_block_tx_p2p = external_block_tx.clone();

    // P2P
    tokio::spawn(async move {
        if let Err(e) = p2p::node::start_p2p_node(rx, bootstrap_nodes, ledger_for_p2p, metrics_for_p2p, peer_id_for_p2p, external_block_tx_p2p).await {
            eprintln!("Fatal error in P2P node: {:?}", e);
        }
    });

    // API
    let api_state = api::AppState {
        ledger: shared_ledger.clone(),
        tx: tx.clone(),
        metrics: shared_metrics.clone(),
        peer_id: shared_peer_id.clone(),
        mining_cmd_tx: mining_cmd_tx_api,
    };
    
    tokio::spawn(async move {
        api::start_api_server(api_state).await;
    });

    // UNIFIED MINING ARCHITECTURE: Routes both AUTO_MINE (interval) and /mine (manual) through single task
    // with tokio::select! cancellation to prevent forks when external blocks arrive
    let auto_mine = env::var("AUTO_MINE").unwrap_or_else(|_| "false".to_string()).to_lowercase() == "true";
    let block_time_secs: u64 = env::var("BLOCK_TIME_SECS")
        .unwrap_or_else(|_| "10".to_string())
        .parse()
        .unwrap_or(10);

    // If AUTO_MINE enabled, spawn interval timer task
    if auto_mine {
        println!("[AUTO_MINE] Enabled with block time: {}s", block_time_secs);
        let mining_cmd_tx_interval = mining_cmd_tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(block_time_secs));
            loop {
                interval.tick().await;
                if mining_cmd_tx_interval.send(MiningTrigger::Interval).await.is_err() {
                    break;  // Channel closed
                }
            }
        });
    }

    // Unified mining task: Handles both interval-based and manual mining with tokio::select! cancellation
    {
        let ledger_for_mining = shared_ledger.clone();
        let peer_id_for_mining = shared_peer_id.clone();
        let metrics_for_mining = shared_metrics.clone();
        let tx_for_mining = tx.clone();
        let mut external_block_rx = external_block_tx.subscribe();
        let mut mining_cmd_rx = mining_cmd_rx;

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    // Mining trigger: Either interval tick (AUTO_MINE) or manual /mine command
                    Some(trigger) = mining_cmd_rx.recv() => {
                        let miner_id = peer_id_for_mining.lock().unwrap().clone();
                        if miner_id.is_empty() {
                            if matches!(trigger, MiningTrigger::Manual) {
                                println!("Mining failed: peer identity not ready yet");
                            }
                            continue;
                        }

                        let block_time_start = Instant::now();
                        
                        // 1. Prepare block template while locked
                        let (mut new_block, engine) = {
                            let ledger = ledger_for_mining.lock().unwrap();
                            
                            // Check ban status
                            if let Some(reputation_tracker) = ledger.consensus_engine.reputation_tracker() {
                                if let Ok(tracker) = reputation_tracker.lock() {
                                    if tracker.is_banned(&miner_id) {
                                        match trigger {
                                            MiningTrigger::Manual => {
                                                println!("Mining failed: peer {} is banned and cannot mine", miner_id);
                                            },
                                            MiningTrigger::Interval => {
                                                println!("[AUTO_MINE] Skipped: peer {} is banned", miner_id);
                                            },
                                        }
                                        continue;
                                    }
                                }
                            }

                            (ledger.create_mining_block(miner_id.clone()), ledger.consensus_engine.clone())
                        };

                        // 2. Execute mining (seal_block) while UNLOCKED, allowing cancellation
                        let mining_result = tokio::select! {
                            seal_res = engine.seal_block(&mut new_block) => {
                                match seal_res {
                                    Ok(_) => Some(new_block),
                                    Err(e) => {
                                        match trigger {
                                            MiningTrigger::Manual => println!("Mining failed during sealing: {}", e),
                                            MiningTrigger::Interval => println!("[AUTO_MINE] Sealing error: {}", e),
                                        }
                                        None
                                    }
                                }
                            },
                            // External block arrived: Cancel current mining
                            Ok(external_height) = external_block_rx.recv() => {
                                let current_height = {
                                    let ledger = ledger_for_mining.lock().unwrap();
                                    ledger.blocks.len() as u64
                                };
                                if external_height >= current_height {
                                    println!("[MINING] Cancelled: External block at height {} received. Restarting cycle...", external_height);
                                    None
                                } else {
                                    // Received an older block? Rare, but just keep mining in that case
                                    // Actually, we need to resume mining somehow? 
                                    // For simplicity, we just stop and wait for next trigger.
                                    None
                                }
                            }
                        };

                        // 3. Commit the mined block if successful
                        if let Some(mined_block) = mining_result {
                            let broadcast_block = {
                                let mut ledger = ledger_for_mining.lock().unwrap();
                                
                                // Double check height hasn't changed while we were mining
                                if mined_block.index == (ledger.blocks.len() as u32) {
                                    let block_time_ms = block_time_start.elapsed().as_millis();
                                    let num_transactions = mined_block.transactions.len();
                                    
                                    {
                                        let mut metrics = metrics_for_mining.lock().unwrap();
                                        metrics.record_block_mined(num_transactions, block_time_ms);
                                    }
                                    
                                    match trigger {
                                        MiningTrigger::Manual => {
                                            println!("Successfully mined block {} with {} transactions in {}ms. Broadcasting...", 
                                                mined_block.index, num_transactions, block_time_ms);
                                        },
                                        MiningTrigger::Interval => {
                                            println!("[AUTO_MINE] Block {} mined with {} transactions in {}ms. Broadcasting...",
                                                mined_block.index, num_transactions, block_time_ms);
                                        },
                                    }

                                    ledger.commit_mined_block(mined_block.clone());
                                    
                                    if let Err(e) = storage::save_ledger(&ledger) {
                                        eprintln!("Warning: Failed to save ledger to disk: {}", e);
                                    }
                                    
                                    Some(mined_block)
                                } else {
                                    println!("[MINING] Discarded block {}: Chain height moved to {} while mining", 
                                        mined_block.index, ledger.blocks.len());
                                    None
                                }
                            };
                            
                            // Broadcast outside the lock scope
                            if let Some(block) = broadcast_block {
                                let _ = tx_for_mining.send(LocalCommand::BroadcastBlock(block)).await;
                            }
                        }
                    },
                    
                    // Idle state - external block arrived when NOT mining
                    result = external_block_rx.recv() => {
                        // Just consume it to avoid channel backup
                        match result {
                            Ok(_) => {},
                            Err(broadcast::error::RecvError::Lagged(_)) => {},
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
        });
    }
    println!("Commands:");
    println!("  Type '/mine' to seal the mempool into a block (consensus mode: {}).", consensus_mode);
    println!("  Type a JSON string to broadcast an Auction transaction.");
    println!("-----------------------------------------------");

    let stdin = io::stdin();
    
    for line in stdin.lock().lines() {
        let input = line?;
        
        if input.trim() == "/mine" {
            // Send mining trigger to unified mining task instead of mining directly
            if let Err(_) = mining_cmd_tx_cli.send(MiningTrigger::Manual).await {
                eprintln!("Error: mining task not available");
                break;
            }
        } else {
            if let Err(_) = tx.send(LocalCommand::BroadcastAuction(input)).await {
                eprintln!("Error sending internal command.");
                break;
            }
        }
    }

    Ok(())
}


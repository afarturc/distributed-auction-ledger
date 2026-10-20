use std::sync::{Arc, Mutex};
use std::time::Instant;
use serde::{Serialize, Deserialize};
use std::collections::VecDeque;

/// System metrics for monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metrics {
    pub blocks_mined: u64,
    pub transactions_received_total: u64,
    pub transactions_mined_total: u64,
    pub mempool_size: usize,
    pub peers_connected: usize,
    pub block_rejection_rate: f64,
    pub uptime_seconds: u64,
    pub tps: f64,  // transactions per second
    pub average_block_time_ms: f64,
}

pub struct MetricsCollector {
    blocks_mined: u64,
    transactions_received_total: u64,
    transactions_mined_total: u64,
    mempool_size: usize,
    peers_connected: usize,
    blocks_rejected: u64,
    blocks_accepted: u64,
    start_time: Instant,
    block_times: VecDeque<u128>, // Last 100 block times in ms
}

impl MetricsCollector {
    pub fn new() -> Self {
        MetricsCollector {
            blocks_mined: 0,
            transactions_received_total: 0,
            transactions_mined_total: 0,
            mempool_size: 0,
            peers_connected: 0,
            blocks_rejected: 0,
            blocks_accepted: 0,
            start_time: Instant::now(),
            block_times: VecDeque::new(),
        }
    }

    /// Increments counter of mined blocks
    pub fn record_block_mined(&mut self, num_transactions: usize, block_time_ms: u128) {
        self.blocks_mined += 1;
        self.transactions_mined_total += num_transactions as u64;
        self.blocks_accepted += 1;

        // Keep only the last 100 block times to calculate average
        self.block_times.push_back(block_time_ms);
        if self.block_times.len() > 100 {
            self.block_times.pop_front();
        }
    }

    /// Records a rejected block
    pub fn record_block_rejected(&mut self) {
        self.blocks_rejected += 1;
    }

    /// Records a block accepted from the network (validated via gossipsub or sync RPC).
    /// Mirrors record_block_mined for the counters that reflect cluster-wide state,
    /// but does not touch block_time_ms (we did not perform the sealing work).
    pub fn record_block_received(&mut self, num_transactions: usize) {
        self.blocks_accepted += 1;
        self.transactions_mined_total += num_transactions as u64;
    }

    /// Resets chain-wide counters after a full chain replace (sync RPC adopted a longer chain).
    /// Preserves blocks_mined / blocks_rejected (local node activity) but realigns the totals
    /// that should mirror the new accepted chain.
    pub fn reset_chain_counters(&mut self, accepted_block_count: usize, total_transactions: usize) {
        self.blocks_accepted = accepted_block_count as u64;
        self.transactions_mined_total = total_transactions as u64;
    }

    /// Updates the mempool size
    pub fn update_mempool_size(&mut self, size: usize) {
        self.mempool_size = size;
    }

    /// Updates the number of connected peers
    pub fn update_peers_connected(&mut self, count: usize) {
        self.peers_connected = count;
    }

    /// Increments the total transaction counter
    pub fn increment_transactions(&mut self) {
        self.transactions_received_total += 1;
    }

    /// Returns current metrics
    pub fn get_metrics(&self) -> Metrics {
        let uptime_seconds = self.start_time.elapsed().as_secs();
        
        // Calculate rejection rate
        let total_blocks = self.blocks_accepted + self.blocks_rejected;
        let rejection_rate = if total_blocks > 0 {
            self.blocks_rejected as f64 / total_blocks as f64
        } else {
            0.0
        };

        // Calculate TPS
        let tps = if uptime_seconds > 0 {
            self.transactions_received_total as f64 / uptime_seconds as f64
        } else {
            0.0
        };

        // Calculate average block time
        let average_block_time_ms = if !self.block_times.is_empty() {
            let sum: u128 = self.block_times.iter().sum();
            sum as f64 / self.block_times.len() as f64
        } else {
            0.0
        };

        Metrics {
            blocks_mined: self.blocks_mined,
            transactions_received_total: self.transactions_received_total,
            transactions_mined_total: self.transactions_mined_total,
            mempool_size: self.mempool_size,
            peers_connected: self.peers_connected,
            block_rejection_rate: rejection_rate,
            uptime_seconds,
            tps,
            average_block_time_ms,
        }
    }

    /// Resets the metrics
    pub fn reset(&mut self) {
        self.blocks_mined = 0;
        self.transactions_received_total = 0;
        self.transactions_mined_total = 0;
        self.blocks_rejected = 0;
        self.blocks_accepted = 0;
        self.start_time = Instant::now();
        self.block_times.clear();
    }
}

/// Thread-safe metrics collector
pub type SharedMetrics = Arc<Mutex<MetricsCollector>>;

pub fn create_metrics() -> SharedMetrics {
    Arc::new(Mutex::new(MetricsCollector::new()))
}

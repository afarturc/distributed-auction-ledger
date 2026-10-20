use std::collections::HashMap;
use std::fs;
use serde::{Serialize, Deserialize};

/// Reputation system for P2P network peers
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PeerReputation {
    pub peer_id: String,
    pub score: i32,
    pub blocks_submitted: u64,
    pub blocks_invalid: u64,
}

pub struct ReputationTracker {
    peers: HashMap<String, PeerReputation>,
    ban_threshold: i32,
    trust_threshold: i32,
}

impl ReputationTracker {
    const REPUTATION_FILE: &'static str = "reputation.json";

    pub fn new() -> Self {
        // Try to load from file
        if let Ok(contents) = fs::read_to_string(Self::REPUTATION_FILE) {
            if let Ok(peers) = serde_json::from_str::<HashMap<String, PeerReputation>>(&contents) {
                println!("[REPUTATION] Loaded {} peers from disk", peers.len());
                return ReputationTracker {
                    peers,
                    ban_threshold: -50,
                    trust_threshold: 50,
                };
            }
        }
        // If load fails, start fresh
        ReputationTracker {
            peers: HashMap::new(),
            ban_threshold: -50,    // Ban when score < -50
            trust_threshold: 50,   // "Trusted" when score > 50
        }
    }

    /// Save reputation to disk
    pub fn save_to_disk(&self) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.peers)
            .map_err(|e| format!("Serialization error: {}", e))?;
        fs::write(Self::REPUTATION_FILE, json)
            .map_err(|e| format!("Write error: {}", e))?;
        Ok(())
    }

    /// Gets or creates the reputation of a peer
    /// New peers start with maximum reputation (100 points) to avoid PoS/VRF deadlock
    fn get_or_create(&mut self, peer_id: &str) -> &mut PeerReputation {
        self.peers.entry(peer_id.to_string()).or_insert(PeerReputation {
            peer_id: peer_id.to_string(),
            score: 100,  // Initialize at maximum to prevent VRF deadlock with zero reputation
            blocks_submitted: 0,
            blocks_invalid: 0,
        })
    }

    /// Increments peer score (valid block received)
    pub fn increment_valid_block(&mut self, peer_id: &str) {
        let peer = self.get_or_create(peer_id);
        peer.score = (peer.score + 1).min(100); // Cap at 100
        peer.blocks_submitted += 1;
        let _ = self.save_to_disk();
    }

    /// Decrements peer score (invalid block received)
    /// Returns true if peer was just banned by this action (score transitioned to <= -50)
    pub fn decrement_invalid_block(&mut self, peer_id: &str) -> bool {
        let ban_threshold = self.ban_threshold;
        let peer = self.get_or_create(peer_id);
        if peer.score <= ban_threshold {
            return false;
        }
        let previous_score = peer.score;
        peer.score = (peer.score - 5).max(-100); // Floor at -100
        peer.blocks_invalid += 1;

        // Ban triggered when score transitions to <= -50 (from above)
        if previous_score > ban_threshold && peer.score <= ban_threshold {
            println!(
                "[REPUTATION] Peer {} banned with score {} after {} invalid blocks",
                peer_id, peer.score, peer.blocks_invalid
            );
            let _ = self.save_to_disk();
            return true;
        }
        false
    }

    /// Decrements score for suspicious behavior
    /// Returns true if peer was just banned by this action (score transitioned to <= -50)
    pub fn record_suspicious_behavior(&mut self, peer_id: &str, penalty: i32) -> bool {
        let ban_threshold = self.ban_threshold;
        let peer = self.get_or_create(peer_id);
        if peer.score <= ban_threshold {
            return false;
        }
        let previous_score = peer.score;
        peer.score = (peer.score - penalty).max(-100);

        // Ban triggered when score transitions to <= -50 (from above)
        if previous_score > ban_threshold && peer.score <= ban_threshold {
            println!(
                "[REPUTATION] Peer {} banned with score {} after suspicious behavior penalty {}",
                peer_id, peer.score, penalty
            );
            return true;
        }
        false
    }

    /// Checks if a peer is banned
    /// A peer is banned when score <= -50 (BFT consensus threshold)
    pub fn is_banned(&self, peer_id: &str) -> bool {
        self.peers
            .get(peer_id)
            .map(|p| p.score <= self.ban_threshold)
            .unwrap_or(false)
    }

    /// Checks if a peer is trusted
    pub fn is_trusted(&self, peer_id: &str) -> bool {
        self.peers
            .get(peer_id)
            .map(|p| p.score > self.trust_threshold)
            .unwrap_or(false)
    }

    /// Gets peer reputation
    pub fn get_peer_reputation(&self, peer_id: &str) -> Option<PeerReputation> {
        self.peers.get(peer_id).cloned()
    }

    /// Returns all reputations
    pub fn get_all_peers(&self) -> Vec<PeerReputation> {
        self.peers.values().cloned().collect()
    }

    /// Returns list of banned peers
    pub fn get_banned_peers(&self) -> Vec<String> {
        self.peers
            .iter()
            .filter(|(_, p)| p.score <= self.ban_threshold)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Resets peer reputation to maximum (100 points)
    pub fn reset_peer(&mut self, peer_id: &str) {
        if let Some(peer) = self.peers.get_mut(peer_id) {
            peer.score = 100;  // Reset to maximum reputation
        }
    }

    /// Removes a peer (forgetting)
    pub fn remove_peer(&mut self, peer_id: &str) {
        self.peers.remove(peer_id);
    }

    /// Returns the total number of peers
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Returns network statistics
    pub fn get_network_stats(&self) -> NetworkStats {
        let total_peers = self.peers.len();
        let banned_peers = self.get_banned_peers().len();
        let trusted_peers = self.peers
            .values()
            .filter(|p| p.score > self.trust_threshold)
            .count();

        let avg_score = if total_peers > 0 {
            self.peers.values().map(|p| p.score as f64).sum::<f64>() / total_peers as f64
        } else {
            0.0
        };

        NetworkStats {
            total_peers,
            banned_peers,
            trusted_peers,
            average_score: avg_score,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStats {
    pub total_peers: usize,
    pub banned_peers: usize,
    pub trusted_peers: usize,
    pub average_score: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reputation_tracking() {
        let mut tracker = ReputationTracker::new();

        // Peer A submits valid block - starts at 100, increments to cap at 100
        tracker.increment_valid_block("peer_a");
        assert_eq!(tracker.get_peer_reputation("peer_a").unwrap().score, 100);

        // Peer B submits invalid block - starts at 100, decrements by 5
        tracker.decrement_invalid_block("peer_b");
        assert_eq!(tracker.get_peer_reputation("peer_b").unwrap().score, 95);

        // Peer B continues to be bad: needs 30 total calls to drop from 100 to -50
        // (100 - 50 = 50 difference, each decrement is 5, so 50/5 = 10 more calls after the first)
        for _ in 0..29 {
            tracker.decrement_invalid_block("peer_b");
        }
        // After 30 calls: 100 - (30 * 5) = 100 - 150 = -50 (floored at -100, but we're at exactly -50)
        assert!(tracker.is_banned("peer_b"));
    }
}

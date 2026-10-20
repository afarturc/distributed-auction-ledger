use libp2p::swarm::NetworkBehaviour;
use libp2p::ping;
use libp2p::kad;
use libp2p::gossipsub::{self, MessageAuthenticity, ValidationMode};
use libp2p::request_response::{self, ProtocolSupport};
use libp2p::identity::Keypair;
use libp2p::{PeerId, StreamProtocol};
use std::time::Duration;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::sync::{SyncRequest, SyncResponse};

pub type SyncBehaviour = request_response::json::Behaviour<SyncRequest, SyncResponse>;
pub type SyncEvent = request_response::Event<SyncRequest, SyncResponse>;

#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "LedgerBehaviourEvent")]
pub struct LedgerBehaviour {
    pub ping: ping::Behaviour,
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    pub gossipsub: gossipsub::Behaviour,
    pub sync: SyncBehaviour,
}

#[derive(Debug)]
pub enum LedgerBehaviourEvent {
    Ping(ping::Event),
    Kademlia(kad::Event),
    Gossipsub(gossipsub::Event),
    Sync(SyncEvent),
}

impl LedgerBehaviour {
    pub fn new(local_key: Keypair, local_peer_id: PeerId) -> Self {
        let store = kad::store::MemoryStore::new(local_peer_id);
        let kademlia = kad::Behaviour::with_config(
            local_peer_id,
            store,
            kad::Config::default(),
        );

        let message_authenticity = MessageAuthenticity::Signed(local_key);

        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(10))
            .validation_mode(ValidationMode::Strict)
            .message_id_fn(|message: &gossipsub::Message| {
                let mut s = DefaultHasher::new();
                message.data.hash(&mut s);
                gossipsub::MessageId::from(s.finish().to_string())
            })
            .build()
            .expect("Invalid Gossipsub configuration");

        let gossipsub = gossipsub::Behaviour::new(message_authenticity, gossipsub_config)
            .expect("Failed to initialize Gossipsub");

        let ping = ping::Behaviour::new(Default::default());

        // Request/response protocol for chain synchronisation.
        // Triggered when gossipsub delivers a block whose index is ahead of our tip
        // (gossipsub does not retransmit history; this fills the resulting gap).
        let sync_protocol = StreamProtocol::new("/ledger/sync/1.0.0");
        let mut sync_config = request_response::Config::default();
        sync_config.set_request_timeout(Duration::from_secs(20));
        let sync = SyncBehaviour::new(
            [(sync_protocol, ProtocolSupport::Full)],
            sync_config,
        );

        LedgerBehaviour {
            ping,
            kademlia,
            gossipsub,
            sync,
        }
    }
}

impl From<ping::Event> for LedgerBehaviourEvent {
    fn from(event: ping::Event) -> Self {
        LedgerBehaviourEvent::Ping(event)
    }
}

impl From<kad::Event> for LedgerBehaviourEvent {
    fn from(event: kad::Event) -> Self {
        LedgerBehaviourEvent::Kademlia(event)
    }
}

impl From<gossipsub::Event> for LedgerBehaviourEvent {
    fn from(event: gossipsub::Event) -> Self {
        LedgerBehaviourEvent::Gossipsub(event)
    }
}

impl From<SyncEvent> for LedgerBehaviourEvent {
    fn from(event: SyncEvent) -> Self {
        LedgerBehaviourEvent::Sync(event)
    }
}

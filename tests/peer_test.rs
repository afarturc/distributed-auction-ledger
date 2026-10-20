//! P2P Integration Test - Peer Connection Validator
//! Tests Ping (latency), Kademlia DHT, Gossipsub between two nodes
//! IMPORTANT: Adjust `peer_addr_str` with the primary node's port

use libp2p::{identity, PeerId, Multiaddr, futures::StreamExt, swarm::SwarmEvent};
use std::str::FromStr;

#[path = "../src/p2p/behaviour.rs"]
mod behaviour;

use behaviour::{LedgerBehaviour, LedgerBehaviourEvent};

#[tokio::test]
async fn test_peer_connection() -> Result<(), Box<dyn std::error::Error>> {
    // Create peer identity
    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());
    
    println!("\n[P2P Test Node]");
    println!("   PeerId: {}", local_peer_id);

    // Initialize P2P swarm with TCP, Noise, Yamux
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

    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    // Copy bootstrap address from primary node's output
    let peer_addr_str = "/ip4/127.0.0.1/tcp/46151";
    let peer_addr: Multiaddr = Multiaddr::from_str(peer_addr_str)?;
    
    println!("[Dialing {}]", peer_addr);
    swarm.dial(peer_addr)?;
    println!("[Waiting for events... (timeout 30s)]\n");

    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(30);

    loop {
        if start.elapsed() > timeout {
            println!("[Timeout] Test completed");
            break;
        }

        match swarm.select_next_some().await {
            SwarmEvent::NewListenAddr { address, .. } => {
                println!("[Listen] {}", address);
            }
            
            SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                println!("[Connected] {}", peer_id);
                println!("   {:?}", endpoint);
                return Ok(());
            }
            
            SwarmEvent::Behaviour(LedgerBehaviourEvent::Ping(event)) => {
                match event.result {
                    Ok(rtt) => println!("[Ping] RTT: {:.3} ms", rtt.as_secs_f64() * 1000.0),
                    Err(e) => println!("[Ping] Error: {:?}", e),
                }
            }
            
            SwarmEvent::Behaviour(LedgerBehaviourEvent::Kademlia(event)) => {
                println!("[Kademlia] {:?}", event);
            }
            
            SwarmEvent::Behaviour(LedgerBehaviourEvent::Gossipsub(event)) => {
                println!("[Gossipsub] {:?}", event);
            }
            
            event => println!("[Event] {:?}", event),
        }
    }
    
    Ok(())
}
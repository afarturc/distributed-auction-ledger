use serde::{Serialize, Deserialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AuctionStatus {
    Pending, // Created, but not yet accepting bids
    Open,    // Started, actively accepting bids
    Closed,  // Finished
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Auction {
    pub auction_id: String,
    pub creator_id: String,
    pub item_name: String,
    pub starting_price: u64,
    pub highest_bid: u64,
    pub highest_bidder: String,
    pub status: AuctionStatus,
    pub metadata_key: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum AuctionCommand {
    CreateAuction {
        auction_id: String,
        item_name: String,
        starting_price: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    StartAuction {
        auction_id: String,
    },
    PlaceBid {
        auction_id: String,
        bid_amount: u64,
        bidder_id: String, // The PeerId of the node placing the bid
        nonce: u64,
    },
    CloseAuction {
        auction_id: String,
    },
}

impl crate::blockchain::Hashable for AuctionCommand {
    fn bytes(&self) -> Vec<u8> {
        let mut bytes = vec![];
        match self {
            AuctionCommand::CreateAuction { auction_id, item_name, starting_price, description } => {
                bytes.push(0); 
                bytes.extend(auction_id.as_bytes());
                bytes.extend(item_name.as_bytes());
                bytes.extend(crate::blockchain::u64_to_bytes(*starting_price));
                if let Some(desc) = description {
                    bytes.extend(desc.as_bytes());
                }
            },
            AuctionCommand::StartAuction { auction_id } => {
                bytes.push(1);
                bytes.extend(auction_id.as_bytes());
            },
            AuctionCommand::PlaceBid { auction_id, bid_amount, bidder_id, nonce } => {
                bytes.push(2);
                bytes.extend(auction_id.as_bytes());
                bytes.extend(crate::blockchain::u64_to_bytes(*bid_amount));
                bytes.extend(bidder_id.as_bytes());
                bytes.extend(crate::blockchain::u64_to_bytes(*nonce));
            },
            AuctionCommand::CloseAuction { auction_id } => {
                bytes.push(3);
                bytes.extend(auction_id.as_bytes());
            },
        }
        bytes
    }
}

impl Auction {
    // Constructor: Creates an auction in the Pending state
    pub fn new(auction_id: String, creator_id: String, item_name: String, starting_price: u64, metadata_key: Option<String>) -> Self {
        Auction {
            auction_id,
            creator_id,
            item_name,
            starting_price,
            highest_bid: 0,
            highest_bidder: String::new(),
            status: AuctionStatus::Pending,
            metadata_key,
        }
    }

    // State transition: Pending -> Open
    pub fn start(&mut self) -> Result<(), String> {
        if self.status != AuctionStatus::Pending {
            return Err("Auction can only be started from a Pending state.".to_string());
        }
        self.status = AuctionStatus::Open;
        Ok(())
    }

    // Validation rule: Bid must be strictly greater than current highest and starting price
    pub fn process_bid(&mut self, bid_amount: u64, bidder_id: String) -> Result<(), String> {
        if self.status != AuctionStatus::Open {
            return Err("Bids can only be placed on Open auctions.".to_string());
        }

        let required_amount = if self.highest_bid == 0 {
            self.starting_price
        } else {
            self.highest_bid
        };

        if bid_amount <= required_amount {
            return Err(format!("Bid amount must be strictly higher than {}", required_amount));
        }

        self.highest_bid = bid_amount;
        self.highest_bidder = bidder_id;
        Ok(())
    }

    // State transition: Open -> Closed
    pub fn close(&mut self) -> Result<(), String> {
        if self.status == AuctionStatus::Closed {
            return Err("Auction is already closed.".to_string());
        }
        self.status = AuctionStatus::Closed;
        Ok(())
    }

}

// Signed transaction envelope used by the API and P2P layers.
// `sender_id` is bound to the public key by hash, and `verify()` checks the
// Ed25519 signature over the serialised command before the node accepts it.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SignedTransaction {
    pub sender_id: String,
    pub public_key: Vec<u8>,
    pub command: AuctionCommand,
    pub signature: Vec<u8>,
}

impl SignedTransaction {
    pub fn sender_id_from_public_key(public_key: &[u8]) -> String {
        let digest = Sha256::digest(public_key);
        hex::encode(&digest[..16])
    }

    pub fn verify(&self) -> Result<(), Box<dyn std::error::Error>> {
        use ed25519_dalek::{PublicKey, Signature, Verifier};

        let expected_sender_id = Self::sender_id_from_public_key(&self.public_key);
        if self.sender_id != expected_sender_id {
            return Err(format!(
                "sender_id does not match public_key hash (expected {}, got {})",
                expected_sender_id, self.sender_id
            ).into());
        }

        let signature = Signature::from_bytes(&self.signature)
            .map_err(|e| format!("Invalid signature format: {}", e))?;

        let public = PublicKey::from_bytes(&self.public_key)
            .map_err(|e| format!("Invalid public key: {}", e))?;

        // Message is the serialized command
        let msg = serde_json::to_vec(&self.command)?;

        public.verify(&msg, &signature).map_err(|e| {
            Box::<dyn std::error::Error>::from(format!("Signature verification failed: {}", e))
        })?;

        Ok(())
    }
}
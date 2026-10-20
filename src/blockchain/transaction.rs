use super::hashable::Hashable;

#[derive(Debug, Clone)]
pub struct AuctionTransaction {
    pub auction_id: String,
    pub sender_public_key: Vec<u8>, 
    pub amount: u64,
    pub signature: Vec<u8>, 
}

impl Hashable for AuctionTransaction {
    fn bytes(&self) -> Vec<u8> {
        let mut bytes = vec![];
        bytes.extend(self.auction_id.as_bytes());
        bytes.extend(&self.sender_public_key);
        bytes.extend(super::u64_to_bytes(self.amount));
        bytes.extend(&self.signature);
        bytes
    }
}
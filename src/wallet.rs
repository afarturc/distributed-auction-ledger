use ed25519_dalek::Keypair;
use std::path::PathBuf;
use std::fs;
use serde::{Serialize, Deserialize};
use base64::{engine::general_purpose, Engine as _};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Wallet {
    pub public_key_b64: String,
    pub secret_key_b64: String,
}

impl Wallet {
    pub fn generate() -> Self {
        // Use OS random bytes directly to avoid rand_core version conflicts
        let mut secret_bytes = [0u8; 32];
        getrandom::getrandom(&mut secret_bytes).expect("OS RNG failed");
        let secret = ed25519_dalek::SecretKey::from_bytes(&secret_bytes).expect("Invalid secret key");
        let public = ed25519_dalek::PublicKey::from(&secret);
        let mut combined = [0u8; 64];
        combined[..32].copy_from_slice(&secret_bytes);
        combined[32..].copy_from_slice(public.as_bytes());
        let _kp = Keypair::from_bytes(&combined).expect("Failed to build Keypair");
        Wallet {
            public_key_b64: general_purpose::STANDARD.encode(public.as_bytes()),
            secret_key_b64: general_purpose::STANDARD.encode(&secret_bytes),
        }
    }

    pub fn load_from_disk(_password: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        path.push(".ledger/keys.enc");
        let data = fs::read_to_string(path)?;
        let wallet: Wallet = serde_json::from_str(&data)?;
        Ok(wallet)
    }

    pub fn save_to_disk(&self, _password: &str) -> Result<(), Box<dyn std::error::Error>> {
        let mut dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        dir.push(".ledger");
        let _ = fs::create_dir_all(&dir);
        dir.push("keys.enc");
        let json = serde_json::to_string_pretty(self)?;
        fs::write(dir, json)?;
        Ok(())
    }

    pub fn public_key_hex(&self) -> String {
        let public = general_purpose::STANDARD.decode(&self.public_key_b64).unwrap_or_default();
        public.iter().map(|b| format!("{:02x}", b)).collect::<String>()
    }

    pub fn sender_id_hex(&self) -> String {
        let public = general_purpose::STANDARD.decode(&self.public_key_b64).unwrap_or_default();
        let digest = Sha256::digest(&public);
        hex::encode(&digest[..16])
    }

    pub fn to_keypair(&self) -> Result<Keypair, Box<dyn std::error::Error>> {
        let secret = general_purpose::STANDARD.decode(&self.secret_key_b64)?;
        let public = general_purpose::STANDARD.decode(&self.public_key_b64)?;
        let mut combined = Vec::with_capacity(64);
        combined.extend_from_slice(&secret);
        combined.extend_from_slice(&public);
        let kp = Keypair::from_bytes(&combined)?;
        Ok(kp)
    }
}

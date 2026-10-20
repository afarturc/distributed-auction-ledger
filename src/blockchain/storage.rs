use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use super::chain::Blockchain;

pub const LEDGER_FILE: &str = "ledger.json";

// Saves the entire blockchain state to the disk
pub fn save_ledger(blockchain: &Blockchain) -> Result<(), String> {
    let json_data = serde_json::to_string_pretty(blockchain)
        .map_err(|e| format!("Failed to serialize ledger: {}", e))?;

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(LEDGER_FILE)
        .map_err(|e| format!("Failed to open ledger file: {}", e))?;

    file.write_all(json_data.as_bytes())
        .map_err(|e| format!("Failed to write to ledger file: {}", e))?;

    Ok(())
}

// Loads the blockchain from the disk if it exists
pub fn load_ledger() -> Option<Blockchain> {
    if !Path::new(LEDGER_FILE).exists() {
        return None;
    }

    let mut file = File::open(LEDGER_FILE).ok()?;
    let mut json_data = String::new();
    file.read_to_string(&mut json_data).ok()?;

    serde_json::from_str::<Blockchain>(&json_data).ok()
}
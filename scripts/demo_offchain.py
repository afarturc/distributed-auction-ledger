#!/usr/bin/env python3
"""
Off-chain metadata demo (SIGNED).

Flow:
1) Store heavy description in DHT via /dht/put
2) Create auction on-chain via /command/signed (with description present)
3) Mine block to persist
4) Validate auction metadata_key + non-empty creator_id
5) Trigger DHT FIND_VALUE via /dht/val/:key
"""

import hashlib
import json
import os
import time

import requests

try:
    from nacl.signing import SigningKey
except Exception as e:
    raise RuntimeError("PyNaCl is required for signed off-chain demo: pip install pynacl") from e


BASE_URL = os.environ.get("BOOTSTRAP_URL", "http://localhost:8080")


def response_payload(response):
    try:
        return response.json()
    except ValueError:
        text = response.text.strip()
        return {"raw": text if text else "No response body"}


def response_message(response):
    payload = response_payload(response)
    if isinstance(payload, dict):
        return payload.get("message") or payload.get("error") or json.dumps(payload)
    return json.dumps(payload)


def to_hex_id(pub_bytes: bytes) -> str:
    # Must match backend validation: first 16 bytes of SHA256(public_key), in hex
    return hashlib.sha256(pub_bytes).digest()[:16].hex()


def make_signed_transaction(command_payload: dict, signing_key: SigningKey) -> dict:
    # IMPORTANT: backend verifies against serde_json::to_vec(&command), which preserves
    # Rust struct field order. Do not sort keys here, or signatures for commands that
    # include optional fields (e.g., CreateAuction.description) may fail verification.
    command_bytes = json.dumps(command_payload, separators=(",", ":")).encode()
    signed = signing_key.sign(command_bytes)
    pub_bytes = signing_key.verify_key.encode()
    return {
        "command": command_payload,
        "sender_id": to_hex_id(pub_bytes),
        "public_key": list(pub_bytes),
        "signature": list(signed.signature),
    }


def main():
    print("\n" + "=" * 70)
    print("DEMO: OFF-CHAIN STORAGE (SIGNED) (LEDGER + DHT)")
    print("=" * 70)

    auction_id = f"OFF_CHAIN_SIGNED_{int(time.time())}"
    dht_key = f"{auction_id}_meta"
    description = (
        "Lorem ipsum dolor sit amet. Hic illum quae et veritatis voluptatem sed sunt nihil. "
        "Ut quia accusamus qui vitae assumenda est culpa quos ab sapiente omnis. "
        "Est adipisci quis 33 sunt sapiente et similique rerum quo consequatur facere "
        "aut amet doloremque et enim dolorem sit expedita quod."
    )

    creator_key = SigningKey.generate()
    creator_id = to_hex_id(creator_key.verify_key.encode())

    print(f"\n[1] STORE metadata in DHT (POST /dht/put) for key {dht_key}...")
    dht_put_payload = {"key": dht_key, "value": description}
    r = requests.post(f"{BASE_URL}/dht/put", json=dht_put_payload)
    print(f"Status: {r.status_code} | {response_message(r)}")
    if r.status_code != 200:
        print("ERROR: Failed to put metadata in DHT. Aborting.")
        return

    print("\n[2] Create auction on-chain via signed transaction (POST /command/signed)...")
    command = {
        "CreateAuction": {
            "auction_id": auction_id,
            "item_name": "Off-chain Signed Demo Item",
            "starting_price": 1000,
            # Description is included so chain stores metadata_key for this auction.
            "description": description,
        }
    }
    signed_tx = make_signed_transaction(command, creator_key)
    r = requests.post(f"{BASE_URL}/command/signed", json=signed_tx)
    print(f"Status: {r.status_code} | {response_message(r)}")
    if r.status_code != 200 or "error" in response_payload(r):
        print("ERROR: Signed CreateAuction rejected. Aborting.")
        return

    print("\n[3] Mining block to persist transaction (POST /admin/mine)...")
    r = requests.post(f"{BASE_URL}/admin/mine")
    print(f"Mining: {response_message(r)}")

    # Small delay helps avoid immediate read races in high-load demos.
    time.sleep(1)

    print(f"\n[4] Verify ledger state (GET /auction/{auction_id})...")
    r = requests.get(f"{BASE_URL}/auction/{auction_id}")
    auction = response_payload(r)
    if not isinstance(auction, dict) or auction.get("error"):
        print(f"ERROR: Could not retrieve auction. Response: {auction}")
        return

    meta_key = auction.get("metadata_key")
    stored_creator = auction.get("creator_id")
    print(f"Auction found. metadata_key={meta_key}, creator_id={stored_creator}")

    if meta_key != dht_key:
        print(f"ERROR: metadata_key mismatch. expected={dht_key}, got={meta_key}")
        return

    if not stored_creator:
        print("ERROR: creator_id is empty (expected signed creator id).")
        return

    if stored_creator != creator_id:
        print(f"WARNING: creator_id differs from local sender_id. expected={creator_id}, got={stored_creator}")

    print(f"\n[5] Trigger FIND_VALUE in DHT for key {dht_key} (GET /dht/val/{dht_key})...")
    r = requests.get(f"{BASE_URL}/dht/val/{dht_key}")
    print(f"Status: {r.status_code} | {response_message(r)}")
    print("Check node console logs for [DHT SUCCESS] FIND_VALUE Result.")

    print("\n" + "=" * 70)
    print("SIGNED OFF-CHAIN DEMO COMPLETED")
    print("=" * 70)


if __name__ == "__main__":
    main()

import requests
import time
import hashlib
import json
from collections import defaultdict

try:
    from nacl.signing import SigningKey
except Exception as e:
    raise RuntimeError("PyNaCl is required for signed demo: pip install pynacl")

BASE_URL = "http://localhost:8080"
nonces = defaultdict(int)


def to_hex_id(pub_bytes: bytes) -> str:
    h = hashlib.sha256(pub_bytes).digest()
    return h[:16].hex()


def make_signed_transaction(command_payload: dict, signing_key: SigningKey) -> dict:
    # command_payload is an externally-tagged enum representation, e.g. {"PlaceBid": {...}}
    command_bytes = json.dumps(command_payload, separators=(",", ":"), sort_keys=True).encode()
    signed = signing_key.sign(command_bytes)
    signature = signed.signature
    verify_key = signing_key.verify_key
    pub_bytes = verify_key.encode()
    sender_id = to_hex_id(pub_bytes)

    signed_tx = {
        "command": command_payload,
        "sender_id": sender_id,
        "public_key": list(pub_bytes),
        "signature": list(signature),
    }
    return signed_tx


def sender_id_for_key(signing_key: SigningKey) -> str:
    return to_hex_id(signing_key.verify_key.encode())


def send_signed_command(signed_tx: dict):
    url = f"{BASE_URL}/command/signed"
    try:
        r = requests.post(url, json=signed_tx)
        if r.status_code != 200:
            print(f"FAILED signed tx -> status: {r.status_code}, resp: {r.text}")
        else:
            resp = r.json()
            if "error" in resp:
                print(f"REJECTED signed tx -> {resp['error']}")
            else:
                print(f"Sent signed tx -> {resp.get('status')}, sender: {resp.get('sender_id')}")
    except Exception as e:
        print(f"Error: {e}")


def mine():
    print("\n[ADMIN] Triggering mining...")
    try:
        r = requests.post(f"{BASE_URL}/admin/mine")
        print(f"Mining result: {r.text}")
    except Exception as e:
        print(f"Error: {e}")


def read_auctions():
    print("\nRead: Query auctions")
    try:
        r = requests.get(f"{BASE_URL}/auctions")
        print(f"State: {json.dumps(r.json(), indent=2)}\n")
    except Exception as e:
        print(f"Error: {e}")

print("========================================")
print("DEMO: 20+ Operations (Read & Write) -- signed transactions")
print("========================================\n")

# create ephemeral signing keys for demo actors
actors = ["Alice", "Bob", "Charlie", "Dave", "Creator"]
keys = {a: SigningKey.generate() for a in actors}

# 1-4: Create 4 auctions signed by Creator
print("--- [PHASE 1] Creating Auctions ---")
creator_key = keys["Creator"]
create_cmds = [
    {"CreateAuction": {"auction_id": "CAR1", "item_name": "Porsche 911", "starting_price": 50000}},
    {"CreateAuction": {"auction_id": "ART1", "item_name": "Picasso", "starting_price": 1000000}},
    {"CreateAuction": {"auction_id": "WATCH1", "item_name": "Rolex", "starting_price": 10000}},
    {"CreateAuction": {"auction_id": "HOUSE1", "item_name": "Villa", "starting_price": 500000}},
]

for c in create_cmds:
    signed = make_signed_transaction(c, creator_key)
    send_signed_command(signed)

time.sleep(1)
read_auctions()

# 6-9: Start auctions (signed by Creator)
print("--- [PHASE 2] Starting Auctions ---")
start_cmds = [
    {"StartAuction": {"auction_id": "CAR1"}},
    {"StartAuction": {"auction_id": "ART1"}},
    {"StartAuction": {"auction_id": "WATCH1"}},
    {"StartAuction": {"auction_id": "HOUSE1"}},
]

for s in start_cmds:
    signed = make_signed_transaction(s, creator_key)
    send_signed_command(signed)

time.sleep(1)
read_auctions()

# 11-20: Place bids (signed by individual bidders, with per-bidder nonces)
print("--- [PHASE 3] Placing Bids ---")
def place_bid(auction, amount, bidder):
    bidder_key = keys.get(bidder) or SigningKey.generate()
    sender_id = sender_id_for_key(bidder_key)
    cmd = {"PlaceBid": {"auction_id": auction, "bid_amount": amount, "bidder_id": sender_id, "nonce": nonces[bidder]}}
    signed_tx = make_signed_transaction(cmd, bidder_key)
    send_signed_command(signed_tx)
    nonces[bidder] += 1

place_bid("CAR1", 55000, "Alice")
place_bid("CAR1", 60000, "Bob")
place_bid("ART1", 1100000, "Charlie")
place_bid("WATCH1", 11000, "Alice")
place_bid("WATCH1", 15000, "Bob")
place_bid("HOUSE1", 600000, "Alice")
place_bid("ART1", 1200000, "Dave")
place_bid("HOUSE1", 650000, "Dave")

time.sleep(1)
read_auctions()

# 22-25: Close auctions (signed by Creator)
print("--- [PHASE 4] Closing Auctions ---")
close_cmds = [
    {"CloseAuction": {"auction_id": "CAR1"}},
    {"CloseAuction": {"auction_id": "ART1"}},
    {"CloseAuction": {"auction_id": "WATCH1"}},
    {"CloseAuction": {"auction_id": "HOUSE1"}},
]

for c in close_cmds:
    signed = make_signed_transaction(c, creator_key)
    send_signed_command(signed)

time.sleep(1)
read_auctions()

print("\n========================================")
print("All transactions signed and sent to mempol")
print("========================================")
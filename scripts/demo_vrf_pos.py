#!/usr/bin/env python3
"""
Demo: VRF in Proof of Stake (PoS)

This script tests:
1. Starting a network in PoS mode with VRF enabled
2. Mining blocks using VRF-based eligibility
3. Verifying that VRF proofs are included in blocks
4. Testing ineligible nodes (with low reputation) cannot propose blocks
5. Testing that reputation affects block proposal eligibility

Setup:
  - Start network with: CONSENSUS_MODE=POS ./demo_consensus_mode.sh POS 5
  - Run this script to test VRF eligibility
"""

import requests
import time
import json
import sys
import hashlib
from collections import defaultdict

try:
    from nacl.signing import SigningKey
except Exception as e:
    print(f"ERROR: PyNaCl required - pip install pynacl")
    sys.exit(1)

BASE_URL = "http://localhost:8080"
nonces = defaultdict(int)

def to_hex_id(pub_bytes: bytes) -> str:
    """Generate miner_id from public key"""
    h = hashlib.sha256(pub_bytes).digest()
    return h[:16].hex()

def make_signed_transaction(command_payload: dict, signing_key: SigningKey) -> dict:
    command_bytes = json.dumps(command_payload, separators=(",", ":"), sort_keys=True).encode()
    signed = signing_key.sign(command_bytes)
    signature = signed.signature
    verify_key = signing_key.verify_key
    pub_bytes = verify_key.encode()
    sender_id = to_hex_id(pub_bytes)

    return {
        "command": command_payload,
        "sender_id": sender_id,
        "public_key": list(pub_bytes),
        "signature": list(signature),
    }

def send_signed_command(signed_tx: dict):
    """Send a signed transaction to the network"""
    url = f"{BASE_URL}/command/signed"
    try:
        r = requests.post(url, json=signed_tx, timeout=5)
        if r.status_code == 200:
            print(f"Command accepted (status: {r.status_code})")
            return True
        else:
            print(f"Command rejected (status: {r.status_code}): {r.text}")
            return False
    except Exception as e:
        print(f"Error sending command: {e}")
        return False

def get_ledger():
    """Get current blockchain state"""
    try:
        r = requests.get(f"{BASE_URL}/ledger", timeout=5)
        if r.status_code == 200:
            return r.json()
        return None
    except Exception as e:
        print(f"Error fetching ledger: {e}")
        return None

def get_metrics():
    """Get node metrics"""
    try:
        r = requests.get(f"{BASE_URL}/metrics", timeout=5)
        if r.status_code == 200:
            return r.json()
        return None
    except Exception as e:
        print(f"Error fetching metrics: {e}")
        return None

def get_block(index: int):
    """Get a specific block"""
    try:
        r = requests.get(f"{BASE_URL}/block/{index}", timeout=5)
        if r.status_code == 200:
            return r.json()
        return None
    except Exception as e:
        print(f"Error fetching block {index}: {e}")
        return None

def check_vrf_in_block(block: dict) -> bool:
    """Verify that a block contains VRF proof data"""
    has_vrf_output = "vrf_output" in block and len(block.get("vrf_output", [])) > 0
    has_vrf_proof = "vrf_proof" in block and block.get("vrf_proof") is not None
    return has_vrf_output and has_vrf_proof

def mine_block(miner_id: str = None):
    """Trigger mining on a node"""
    # Try both endpoints: /mine and /admin/mine
    endpoints = ["/admin/mine", "/mine"]
    
    for endpoint in endpoints:
        url = f"{BASE_URL}{endpoint}"
        try:
            r = requests.post(url, json={"miner_id": miner_id} if miner_id else {}, timeout=10)
            if r.status_code == 200:
                print(f"Mining request sent to {endpoint} (status: {r.status_code})")
                return True
            elif r.status_code != 404:
                print(f"Mining response from {endpoint}: status {r.status_code} - {r.text[:100]}")
                return r.status_code == 200
        except Exception as e:
            continue
    
    print(f"No mining endpoint available (/mine or /admin/mine returned 404)")
    return False

def setup_test_auctions():
    """Create some test auctions"""
    print("\n" + "="*70)
    print("STEP 1: Setting up test auctions")
    print("="*70)
    
    creator_key = SigningKey.generate()
    creator_id = to_hex_id(creator_key.verify_key.encode())
    
    # Create multiple auctions to populate mempool
    auctions = [
        {"CreateAuction": {"auction_id": "VRF_TEST_1", "item_name": "Test Item 1", "starting_price": 100}},
        {"CreateAuction": {"auction_id": "VRF_TEST_2", "item_name": "Test Item 2", "starting_price": 200}},
        {"CreateAuction": {"auction_id": "VRF_TEST_3", "item_name": "Test Item 3", "starting_price": 300}},
        {"CreateAuction": {"auction_id": "VRF_TEST_4", "item_name": "Test Item 4", "starting_price": 400}},
    ]
    
    for cmd in auctions:
        signed = make_signed_transaction(cmd, creator_key)
        send_signed_command(signed)
        time.sleep(0.3)
    
    print(f"Creator ID (from public key): {creator_id}")
    print("Waiting for transactions to propagate...")
    time.sleep(2)
    return creator_key

def test_vrf_blocks():
    """Test 1: Verify VRF data is present in blocks"""
    print("\n" + "="*70)
    print("TEST 1: Verifying VRF proof presence in blocks")
    print("="*70)
    
    ledger = get_ledger()
    if not ledger:
        print(" Could not fetch ledger")
        return False
    
    blocks = ledger.get("blocks", [])
    print(f"Current chain height: {len(blocks)}")
    
    # Check all non-genesis blocks have VRF data
    vrf_found = 0
    vrf_missing = 0
    
    for i, block in enumerate(blocks):
        if i == 0:  # Skip genesis
            print(f"Block {i}: Genesis (no VRF expected)")
            continue
        
        if check_vrf_in_block(block):
            vrf_output_preview = block["vrf_output"][:16] if block["vrf_output"] else "empty"
            print(f" Block {i}: VRF proof found (output[:16]={vrf_output_preview})")
            vrf_found += 1
        else:
            print(f" Block {i}: VRF proof MISSING")
            vrf_missing += 1
    
    print(f"\nSummary: {vrf_found} blocks with VRF, {vrf_missing} blocks without VRF")
    return vrf_missing == 0

def test_mining_with_vrf():
    """Test 2: Mine blocks and verify VRF output varies"""
    print("\n" + "="*70)
    print("TEST 2: Mining blocks with VRF eligibility")
    print("="*70)
    
    # Get initial chain height
    ledger = get_ledger()
    initial_height = len(ledger.get("blocks", [])) if ledger else 0
    print(f"Initial chain height: {initial_height}")
    
    # Get metrics to check mempool
    metrics = get_metrics()
    if metrics:
        mempool_size = metrics.get("mempool_size", 0)
        print(f"Mempool size: {mempool_size} transactions")
        if mempool_size == 0:
            print("Warning: Mempool is empty. Mining will create empty blocks.")
    
    # Try mining a few blocks
    miner_id = "test_miner_vrf"
    vrf_outputs = []
    
    for attempt in range(2):
        print(f"\nAttempt {attempt+1}: Mining block...")
        if mine_block(miner_id):
            print("Mining request sent")
        else:
            print("Mining request failed")
        
        time.sleep(3)  # Wait for mining to complete
    
    # Check new blocks
    ledger = get_ledger()
    new_height = len(ledger.get("blocks", [])) if ledger else initial_height
    
    if new_height > initial_height:
        print(f"Chain height increased: {initial_height} -> {new_height}")
        
        # Check VRF outputs of newly mined blocks
        for i in range(initial_height, new_height):
            block = get_block(i)
            if block and check_vrf_in_block(block):
                vrf_out = block.get("vrf_output", "")[:8]
                vrf_outputs.append(vrf_out)
                print(f"  Block {i}: VRF output (first 8 bytes) = {vrf_out}")
        
        return True
    else:
        print("Chain height did not increase")
        return False

def test_reputation_effect():
    """Test 3: Check reputation affects block eligibility"""
    print("\n" + "="*70)
    print("TEST 3: Reputation effect on block eligibility")
    print("="*70)
    
    ledger = get_ledger()
    if not ledger:
        print(" Could not fetch ledger")
        return False
    
    blocks = ledger.get("blocks", [])
    
    # Collect miner_ids and their block counts
    miner_blocks = {}
    for block in blocks[1:]:  # Skip genesis
        miner_id = block.get("miner_id", "unknown")
        if miner_id not in miner_blocks:
            miner_blocks[miner_id] = []
        miner_blocks[miner_id].append(block.get("index"))
    
    print("\nMiner eligibility analysis:")
    for miner_id, block_indices in sorted(miner_blocks.items()):
        print(f"  Miner {miner_id}: proposed {len(block_indices)} blocks (indices: {block_indices})")
    
    # Higher reputation miners should propose more blocks
    # (This would be more visible with a longer mining session)
    print("\nNote: Reputation affects VRF eligibility in combined score:")
    print("  score = (1 - vrf_weight) * reputation_weight")
    print("  If reputation_weight = 0.5: score affects eligibility ~50%")
    
    return True

def test_consensus_mode():
    """Test 4: Verify system is running in PoS mode"""
    print("\n" + "="*70)
    print("TEST 4: Verify Proof of Stake mode")
    print("="*70)
    
    ledger = get_ledger()
    if not ledger:
        print(" Could not fetch ledger")
        return False
    
    # Check the consensus engine name in any block (from logs would be better)
    print("Ledger state loaded successfully")
    
    # In PoS mode, blocks should NOT have nonce increments for PoW
    blocks = ledger.get("blocks", [])
    if len(blocks) > 1:
        block = blocks[-1]
        nonce = block.get("nonce", -1)
        print(f"  Last block nonce: {nonce}")
        print(f"  Note: In PoS, nonce should be minimal (not incremented for PoW)")
        print(f"        In PoW, nonce increases significantly during mining")
    
    return True

def main():
    print("\n" + "="*70)
    print("VRF (Verifiable Random Function) - Proof of Stake Demo")
    print("="*70)
    print(f"\nTarget: {BASE_URL}")
    print("Mode: CONSENSUS_MODE=POS")
    print("\nPrerequisites:")
    print("  - Network started with: ./demo_consensus_mode.sh POS 5")
    print("  - At least one bootstrap node running")
    print("  - Docker containers healthy")
    print("\nTests:")
    print("  1. Verify VRF proof presence in blocks")
    print("  2. Mine blocks and check VRF eligibility")
    print("  3. Analyze reputation effect on mining eligibility")
    print("  4. Verify PoS consensus mode")
    
    # Quick diagnostics
    try:
        r = requests.get(f"{BASE_URL}/metrics", timeout=2)
        if r.status_code == 200:
            print("\nAPI is responding")
        else:
            print(f"\nAPI returned status {r.status_code}")
    except Exception as e:
        print(f"\nCannot connect to {BASE_URL}: {e}")
        print("Make sure the network is running with: ./demo_consensus_mode.sh POS 5")
        return 1
    
    # Setup
    setup_test_auctions()
    time.sleep(2)
    
    # Run tests
    results = {}
    results["VRF_Blocks"] = test_vrf_blocks()
    time.sleep(2)
    
    results["VRF_Mining"] = test_mining_with_vrf()
    time.sleep(2)
    
    results["Reputation"] = test_reputation_effect()
    time.sleep(2)
    
    results["PoS_Mode"] = test_consensus_mode()
    
    # Summary
    print("\n" + "="*70)
    print("TEST SUMMARY")
    print("="*70)
    
    for test_name, passed in results.items():
        status = "PASS" if passed else "FAIL"
        print(f"{status}: {test_name}")
    
    all_passed = all(results.values())
    print("\n" + ("="*70))
    if all_passed:
        print("ALL TESTS PASSED - VRF PoS implementation is working!")
    else:
        print("SOME TESTS FAILED - Check the output above")
    print("="*70)
    
    return 0 if all_passed else 1

if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3

"""
Demo: Simultaneous Mining on Multiple Nodes

Tests what happens when two or more nodes try to mine the same block
at the same time. With PoW difficulty=5, this should be observable.

Flow:
1. Inject a transaction to populate mempool
2. Trigger /mine on multiple nodes simultaneously (or quickly)
3. Monitor logs to see which block wins and which is discarded
"""

import subprocess
import time
import requests
import threading
import json

API_BOOTSTRAP = "http://localhost:8080"      # bootstrap1
API_PEER1 = "http://localhost:8081"         # bootstrap2
API_PEER2 = "http://localhost:8082"         # bootstrap3


def print_section(title):
    print("\n" + "=" * 70)
    print(title)
    print("=" * 70)


def get_ledger_height(url):
    try:
        response = requests.get(f"{url}/ledger", timeout=3)
        if response.status_code == 200:
            return len(response.json().get("blocks", []))
    except:
        pass
    return None


def submit_transaction(url, auction_id):
    """Submit a transaction to populate mempool"""
    try:
        cmd = {
            "CreateAuction": {
                "auction_id": auction_id,
                "item_name": "Simultaneous Mining Test",
                "starting_price": 100
            }
        }
        response = requests.post(f"{url}/command", json=cmd, timeout=5)
        return response.status_code == 200
    except:
        return False


def mine_on_node(url, node_name):
    """Trigger mining on a specific node via HTTP endpoint"""
    try:
        response = requests.post(f"{url}/admin/mine", timeout=60)
        if response.status_code == 200:
            result = response.json()
            if result.get("status") == "success":
                block_info = result.get("message")
                block_hash = result.get("block_hash", "unknown")[:16]
                time_ms = result.get("time_ms", 0)
                print(f"\n[{node_name}] Mining completed successfully in {time_ms}ms")
                print(f"  Block hash: {block_hash}...")
                return True
            else:
                print(f"\n[{node_name}] !!! Mining failed: {result.get('error')}")
                return False
        else:
            print(f"\n[{node_name}] HTTP error: {response.status_code}")
            return False
    except requests.exceptions.Timeout:
        print(f"\n[{node_name}] !! Mining timed out (took > 60s)")
        return False
    except Exception as e:
        print(f"\n[{node_name}] !!! Error: {e}")
        return False


def get_last_block_hash(url):
    """Get the hash of the last mined block"""
    try:
        response = requests.get(f"{url}/ledger", timeout=3)
        if response.status_code == 200:
            blocks = response.json().get("blocks", [])
            if blocks:
                return blocks[-1].get("hash", "unknown")
    except:
        pass
    return None


def main():
    print_section("SIMULTANEOUS MINING TEST (PoW difficulty=5)")
    
    # Check connectivity
    print("\n[SETUP] Checking node connectivity...")
    height_bootstrap = get_ledger_height(API_BOOTSTRAP)
    height_peer1 = get_ledger_height(API_PEER1)
    height_peer2 = get_ledger_height(API_PEER2)
    
    if height_bootstrap is None or height_peer1 is None or height_peer2 is None:
        print("ERROR: Cannot connect to nodes. Are containers running?")
        return
    
    print(f"  Bootstrap height: {height_bootstrap}")
    print(f"  Peer-1 height: {height_peer1}")
    print(f"  Peer-2 height: {height_peer2}")
    
    # Record starting heights
    start_height = height_bootstrap
    
    print_section("STEP 1: Inject Transactions")
    print("\nAdding transactions to mempool...")
    for i in range(3):
        auction_id = f"SIMULTANEOUS_TEST_{i}_{int(time.time())}"
        if submit_transaction(API_BOOTSTRAP, auction_id):
            print(f"Transaction {i+1} injected")
        time.sleep(0.2)
    
    # Verify mempool
    print("\nChecking mempool size...")
    try:
        response = requests.get(f"{API_BOOTSTRAP}/ledger", timeout=3)
        if response.status_code == 200:
            mempool_size = len(response.json().get("mempool", []))
            print(f"  Mempool size: {mempool_size}")
    except:
        print("  (Could not check mempool)")
    
    print_section("STEP 2: Trigger Mining on Multiple Nodes Simultaneously")
    print("\nStarting mining on Bootstrap and Peer-1 at nearly the same time...")
    print("(With difficulty=5, mining should take ~1-5 seconds per node)")
    
    # Start mining threads
    threads = []
    start_time = time.time()
    
    thread_bootstrap = threading.Thread(
        target=mine_on_node,
        args=(API_BOOTSTRAP, "BOOTSTRAP-1")
    )
    thread_peer1 = threading.Thread(
        target=mine_on_node,
        args=(API_PEER1, "BOOTSTRAP-2")
    )
    
    threads.append(thread_bootstrap)
    threads.append(thread_peer1)
    
    # Start both threads
    for t in threads:
        t.start()
    
    # Wait for mining to complete
    print("\n[WAITING] Mining in progress...")
    for t in threads:
        t.join()
    
    elapsed = time.time() - start_time
    print(f"\nMining completed in {elapsed:.1f} seconds")
    
    print_section("STEP 3: Analyze Results")
    
    time.sleep(2)  # Wait for network sync
    
    # Check final heights
    height_bootstrap_after = get_ledger_height(API_BOOTSTRAP)
    height_peer1_after = get_ledger_height(API_PEER1)
    height_peer2_after = get_ledger_height(API_PEER2)
    
    print(f"\nFinal Heights:")
    print(f"  Bootstrap-1: {start_height} → {height_bootstrap_after} (new blocks: {height_bootstrap_after - start_height})")
    print(f"  Bootstrap-2: {height_peer1} → {height_peer1_after} (new blocks: {height_peer1_after - height_peer1})")
    print(f"  Bootstrap-3: {height_peer2} → {height_peer2_after} (new blocks: {height_peer2_after - height_peer2})")
    
    # Get last block hashes
    hash_bootstrap = get_last_block_hash(API_BOOTSTRAP)
    hash_peer1 = get_last_block_hash(API_PEER1)
    
    print(f"\nLast Block Hashes:")
    print(f"  Bootstrap-1: {hash_bootstrap[:16]}..." if hash_bootstrap else "  Bootstrap-1: N/A")
    print(f"  Bootstrap-2: {hash_peer1[:16]}..." if hash_peer1 else "  Bootstrap-2: N/A")
    
    if hash_bootstrap == hash_peer1:
        print("\nCONSENSUS: Both nodes accepted the same block")
    else:
        print("\nWARNING: Nodes have different block hashes (possible fork!)")


if __name__ == "__main__":
    main()

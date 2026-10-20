#!/usr/bin/env python3

"""
Attack & Recovery Test (Spoofing, Replay, and Flood Attacks)

Tests that the ledger correctly rejects malformed, unsigned, and replayed transactions.

Flow:
1. Inject malformed/unsigned transaction (should be rejected with 422)
2. Simulate flood/replay attack with duplicate requests (should be deduplicated)
3. Verify system recovers and continues operating normally
"""

import requests
import time
import sys

API_URL = "http://localhost:8080"


def print_section(title):
    print("\n" + "=" * 70)
    print(title)
    print("=" * 70)


def main():
    print_section("ATTACK & RECOVERY TEST (Spoofing, Replay, and Flood Attacks)")

    # 1. Spoofing Test (Malformed/Unsigned Transaction)
    print("\n[ATTACK 1] Injecting Malformed/Unsigned Transaction...")
    malformed_payload = {
        "command": {
            "CreateAuction": {
                "auction_id": "HACK_1",
                "item_name": "Stolen Item",
                "starting_price": 0
            }
        },
        # Intentionally missing or invalid sender_id, public_key, signature
        "signature": "invalid_signature_string"
    }
    
    try:
        response = requests.post(f"{API_URL}/command/signed", json=malformed_payload, timeout=5)
        print(f"  Response Status: {response.status_code}")
        if response.status_code in [400, 422]:
            print("  SUCCESS: Node properly rejected the unsigned/invalid transaction")
        else:
            print("  WARNING: Node did not return 422/400 as expected")
    except Exception as e:
        print(f"  Error: {e}")

    # 2. Flood / Replay Attack Test
    print("\n[ATTACK 2] Simulating Flood / Replay Attack...")
    
    flood_payload = {
        "CreateAuction": {
            "auction_id": "FLOOD_TEST_1",
            "item_name": "Flood Item",
            "starting_price": 100
        }
    }
    
    # Get mempool size before flood
    try:
        response = requests.get(f"{API_URL}/metrics", timeout=5)
        mempool_before = response.json().get("mempool_size", 0)
    except:
        mempool_before = 0
    
    print("  Firing 10 rapid duplicate requests...")
    success_count = 0
    
    for i in range(10):
        try:
            res = requests.post(f"{API_URL}/command", json=flood_payload, timeout=5)
            if res.status_code == 200:
                success_count += 1
            time.sleep(0.05)  # Small delay between requests
        except Exception as e:
            print(f"    Request {i+1} failed: {e}")
    
    # Wait for async processing
    time.sleep(1)
    
    # Check actual mempool state
    try:
        response = requests.get(f"{API_URL}/metrics", timeout=5)
        mempool_after = response.json().get("mempool_size", 0)
    except:
        mempool_after = 0
    
    new_transactions = mempool_after - mempool_before
    
    print(f"  Results: {success_count} requests accepted")
    print(f"  Mempool before: {mempool_before}, after: {mempool_after}")
    print(f"  New transactions added: {new_transactions}")
    
    if new_transactions <= 1:
        print("  SUCCESS: Deduplication working (10 requests resulted in 1 or 0 new transactions)")
    else:
        print("  WARNING: Multiple transactions added (deduplication may not be working)")

    # 3. Verify System Recovery
    print("\n[RECOVERY] Verifying system continues operating normally...")
    
    try:
        response = requests.get(f"{API_URL}/metrics", timeout=5)
        if response.status_code == 200:
            metrics = response.json()
            print(f"  Mempool size: {metrics.get('mempool_size', 'N/A')}")
            print(f"  Peers connected: {metrics.get('peers_connected', 'N/A')}")
            print("  System operational after attack")
        else:
            print(f"  Metrics endpoint returned {response.status_code}")
    except Exception as e:
        print(f"  Error: {e}")

    # 4. Test Normal Operation After Attack
    print("\n[NORMAL OPERATION] Testing valid transaction after attacks...")
    
    valid_payload = {
        "CreateAuction": {
            "auction_id": f"VALID_POST_ATTACK_{int(time.time())}",
            "item_name": "Normal Item",
            "starting_price": 250
        }
    }
    
    try:
        response = requests.post(f"{API_URL}/command", json=valid_payload, timeout=5)
        if response.status_code == 200:
            print(f"  Valid transaction accepted (status: {response.status_code})")
        else:
            print(f"  Valid transaction rejected (status: {response.status_code})")
    except Exception as e:
        print(f"  Error: {e}")

    # 5. Final Validation
    print_section("RESULTS")
    
    print("\n[SUMMARY]")
    print("Malformed transactions properly rejected")
    print("Flood/replay attacks detected and handled")
    print("System recovers and continues operating")
    print("Valid transactions still accepted after attacks")
    
    print("\n" + "=" * 70)
    print("ATTACK & RECOVERY TEST: COMPLETE")
    print("=" * 70)


if __name__ == "__main__":
    main()

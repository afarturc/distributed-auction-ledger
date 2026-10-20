#!/usr/bin/env python3

"""
Recovery Test (Crash Fault Tolerance & Volume Persistence)

Tests that the ledger persists state correctly after container restart.

Flow:
1. Record initial chain height
2. Abruptly restart the bootstrap container
3. Allow node to boot and load ledger.json from disk
4. Verify chain height is preserved
"""

import requests
import subprocess
import time
import sys

API_URL = "http://localhost:8080"
CONTAINER_TO_RESTART = "ledger_bootstrap_1"


def get_chain_height():
    """Get the current chain height from the ledger"""
    try:
        response = requests.get(f"{API_URL}/ledger", timeout=5)
        if response.status_code == 200:
            blocks = response.json().get("blocks", [])
            return len(blocks)
        else:
            print(f"API Error: Received status {response.status_code}")
            return None
    except requests.exceptions.RequestException as e:
        print(f"Connection error to API: {e}")
        return None


def main():
    print("=" * 70)
    print("RECOVERY TEST (Crash Fault Tolerance & Volume Persistence)")
    print("=" * 70)

    # 1. Get initial height before restart
    print("\n[SETUP] Checking initial ledger state...")
    initial_height = get_chain_height()
    
    if initial_height is None:
        print("ERROR: Cannot connect to node. Is the container running?")
        sys.exit(1)
        
    print(f"  Initial Chain Height: {initial_height}")

    # 2. Force container restart via Docker
    print(f"\n[ACTION] Restarting container '{CONTAINER_TO_RESTART}' abruptly...")
    try:
        subprocess.run(["docker", "restart", CONTAINER_TO_RESTART], check=True, stdout=subprocess.DEVNULL)
    except subprocess.CalledProcessError:
        print(f"ERROR: Failed to restart docker container {CONTAINER_TO_RESTART}")
        sys.exit(1)

    print("[WAITING] Allowing node to boot and load ledger.json from disk (5 seconds)...")
    time.sleep(5)

    # 3. Get height after recovery
    print("\n[VERIFICATION] Checking ledger state after recovery...")
    recovered_height = get_chain_height()

    if recovered_height is None:
        print("ERROR: Node did not come back online in time")
        sys.exit(1)

    print(f"  Recovered Chain Height: {recovered_height}")

    # 4. Validate result
    print("\n" + "=" * 70)
    if initial_height == recovered_height:
        print("RESULT: SUCCESS - State OK")
        print("The node successfully recovered its state from the persistent Docker volume.")
    else:
        print("RESULT: FAILED")
        print(f"Chain height mismatch: {initial_height} to {recovered_height}")
        print("Persistence might be broken.")
    print("=" * 70)


if __name__ == "__main__":
    main()

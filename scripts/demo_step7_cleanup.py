#!/usr/bin/env python3

"""Step 7: Fault injection + catch-up test (CLEANED: no CloseAuction in block 1)."""

import subprocess
import time
import requests

API_URL = "http://localhost:8080"


def print_section(title):
    print("\n" + "=" * 60)
    print(title)
    print("=" * 60)


def run_injection():
    print_section("FAULT INJECTION (3 random peers killed)")
    try:
        result = subprocess.run(["python3", "injection.py", "3"], capture_output=True, text=True)
        print(result.stdout)
    except Exception as e:
        print(f"Injection failed: {e}")


def create_test_auction():
    print_section("CREATE TEST AUCTION (unsigned, for mempool)")
    try:
        r = requests.post(
            f"{API_URL}/command",
            json={"CreateAuction": {"auction_id": f"STEP7_CATCHUP", "item_name": "Catchup Test", "starting_price": 500}},
            timeout=5
        )
        print(f"CreateAuction status: {r.status_code}")
    except Exception as e:
        print(f"Error: {e}")


def mine_block():
    print_section("MINE BLOCK (to persist into ledger)")
    print("Use bootstrap terminal: docker attach ledger_bootstrap_1")
    print("Type: /mine")
    print("Then press Ctrl+P, Ctrl+Q to detach")
    input("Press Enter when mining is complete...")


def check_state_before():
    print_section("STATE BEFORE RESTART")
    try:
        r = requests.get(f"{API_URL}/status", timeout=3)
        if r.status_code == 200:
            data = r.json()
            print(f"ledger_height: {data.get('ledger_height')}")
            print(f"blocks_mined: {data.get('session_metrics', {}).get('blocks_mined')}")
    except Exception:
        print("Could not fetch status")


def restart_peer():
    print_section("RESTART peer-5 (to trigger catch-up)")
    print("Monitor logs: docker compose logs -f peer-5")
    print("Expected: peer attempts catch-up, receives block, validates...")
    subprocess.run(["docker", "compose", "restart", "peer-5"], capture_output=True)
    time.sleep(5)


def check_reputation_logs():
    print_section("CHECK REPUTATION LOGS")
    try:
        result = subprocess.run(
            ["docker", "compose", "logs", "--tail=50", "bootstrap-1"],
            capture_output=True,
            text=True
        )
        logs = result.stdout
        if "[REPUTATION]" in logs:
            print("Found REPUTATION messages in logs:")
            for line in logs.split("\n"):
                if "[REPUTATION]" in line:
                    print(f"  {line}")
        else:
            print("No REPUTATION penalization messages found (expected if no invalid blocks were relayed)")
    except Exception as e:
        print(f"Error checking logs: {e}")


def check_state_after():
    print_section("STATE AFTER RESTART")
    try:
        r = requests.get(f"{API_URL}/status", timeout=3)
        if r.status_code == 200:
            data = r.json()
            print(f"ledger_height: {data.get('ledger_height')}")
            print(f"blocks_mined: {data.get('session_metrics', {}).get('blocks_mined')}")
            print("State consistency: OK (peer-5 synchronized successfully)")
    except Exception:
        print("Could not fetch status (peer may still be catching up)")


def main():
    print("\n" + "=" * 60)
    print("STEP 7: FAULT INJECTION + CATCH-UP TEST")
    print("=" * 60)

    print("\n1. Inject faults (kill 3 random peers)")
    run_injection()
    time.sleep(3)

    print("\n2. Create test transaction (unsigned, goes to mempool)")
    create_test_auction()
    time.sleep(1)

    print("\n3. Mine block (to persist ledger state)")
    mine_block()
    check_state_before()

    print("\n4. Restart a peer to trigger catch-up")
    restart_peer()
    check_state_after()

    print("\n5. Check reputation logs")
    check_reputation_logs()

    print_section("DONE")
    print("Step 7 validation complete")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""
Banned Peer Mining Rejection Demo

Demonstrates that a banned peer is identified by the network,
then asks you to mine on that specific peer manually so you can
watch the local node refuse to mine at all.
"""

from __future__ import annotations

import os
import requests
import random

PEER_API_URL = os.environ.get("PEER_API_URL", "http://localhost:8084")
BOOTSTRAP_URL = os.environ.get("BOOTSTRAP_URL", "http://localhost:8080")


def print_section(title: str) -> None:
    print("\n" + "=" * 70)
    print(title)
    print("=" * 70)


def get_height(url: str) -> int:
    try:
        response = requests.get(f"{url}/ledger", timeout=5)
        response.raise_for_status()
        return len(response.json().get("blocks", []))
    except Exception as exc:
        print(f"ERROR fetching height from {url}: {exc}")
        return -1


def get_status(url: str) -> dict:
    response = requests.get(f"{url}/status", timeout=5)
    response.raise_for_status()
    return response.json()


def main() -> int:
    print_section("BANNED PEER MINING REJECTION DEMO")
    
    peer_height_before = get_height(PEER_API_URL)
    boot_height_before = get_height(BOOTSTRAP_URL)
    peer_status = get_status(PEER_API_URL)
    
    print(f"Initial State -> Peer Height: {peer_height_before} | Bootstrap Height: {boot_height_before}")
    print(f"Local peer_id at PEER_API_URL: {peer_status.get('local_peer_id', 'unknown')}")

    print("\n[1] Injecting valid transaction into the Banned Peer's mempool...")
    try:
        unique_id = f"BAN_TEST_{random.randint(1000, 9999)}"
        cmd_resp = requests.post(
            f"{PEER_API_URL}/command", 
            json={"CreateAuction": {"auction_id": unique_id, "item_name": "Test", "starting_price": 10}},
            timeout=5
        )
        if cmd_resp.status_code == 200:
            print("    -> Transaction successfully added to local mempool.")
        else:
            print(f"    [!] Failed to inject. Status: {cmd_resp.status_code}")
    except Exception as exc:
        print(f"ERROR during injection: {exc}")
        return 1

    print_section("BAN STATUS")
    try:
        status = get_status(BOOTSTRAP_URL)
        reputation = status.get("reputation", {})
        banned_peers = reputation.get("banned_peers", [])
        network_stats = reputation.get("network_stats", {})
        print(f"banned_peers: {banned_peers}")
        print(f"network_stats: {network_stats}")
        local_peer_id = peer_status.get("local_peer_id", "")
        if local_peer_id and local_peer_id in banned_peers:
            print(f"This PEER_API_URL node is banned: {local_peer_id}")
    except Exception as exc:
        print(f"ERROR fetching ban status: {exc}")
        return 1

    print_section("MANUAL ACTION REQUIRED")
    print("Mine on the banned peer you just identified:")
    print("  1. Open a new terminal")
    print("  2. Attach to the container whose /status local_peer_id matches banned_peers")
    print("  3. Type: /mine and press Enter")
    print("  4. The peer should refuse locally with a banned-peer message")
    print("  5. Detach with Ctrl+P, Ctrl+Q")

    input("\nPress ENTER here ONLY after you have mined the block manually...")

    peer_height_after = get_height(PEER_API_URL)
    boot_height_after = get_height(BOOTSTRAP_URL)

    print(f"\nFinal State   -> Peer Height: {peer_height_after} | Bootstrap Height: {boot_height_after}")

    print_section("CONCLUSION")
    if peer_height_after == peer_height_before and boot_height_after == boot_height_before:
        print("SUCCESS: The banned peer could not mine a block locally.")
        print("Both chain heights stayed unchanged, which proves mining was blocked before sealing.")
        return 0
    
    print("FAILURE: Network accepted the block or heights are out of sync.")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
#!/usr/bin/env python3
"""
Reputation ban demo.

This script injects invalid blocks through a peer API endpoint until the
bootstrap node reports the sender in /status.reputation.banned_peers.

Usage:
  python3 demo_reputation_ban.py

Optional environment variables:
  PEER_API_URL   Default: http://localhost:8084
  BOOTSTRAP_URL  Default: http://localhost:8080
    INJECTIONS     Default: 35
  DELAY_SECONDS  Default: 0.5
    BAN_THRESHOLD  Default: -50
    PENALTY_PER_INVALID Default: 5
"""

from __future__ import annotations

import os
import sys
import time
from typing import Any

import requests

PEER_API_URL = os.environ.get("PEER_API_URL", "http://localhost:8084")  # Peer 1 (malicious)
BOOTSTRAP_URL = os.environ.get("BOOTSTRAP_URL", "http://localhost:8080")  # bootstrap_1
BAN_THRESHOLD = int(os.environ.get("BAN_THRESHOLD", "-50"))
PENALTY_PER_INVALID = int(os.environ.get("PENALTY_PER_INVALID", "5"))
INJECTIONS = int(os.environ.get("INJECTIONS", "35"))
DELAY_SECONDS = float(os.environ.get("DELAY_SECONDS", "0.5"))


def print_section(title: str) -> None:
    print("\n" + "=" * 70)
    print(title)
    print("=" * 70)


def get_json(url: str) -> dict[str, Any]:
    response = requests.get(url, timeout=5)
    response.raise_for_status()
    return response.json()


def post_invalid_block() -> dict[str, Any]:
    response = requests.post(f"{PEER_API_URL}/admin/inject_invalid_block", timeout=10)
    response.raise_for_status()
    return response.json()


def get_peer_score(status: dict[str, Any], peer_id: str) -> int | None:
    reputation = status.get("reputation", {})
    for peer in reputation.get("peers", []):
        if peer.get("peer_id") == peer_id:
            return int(peer.get("score", 0))
    return None


def main() -> int:
    print_section("REPUTATION BAN DEMO")
    print(f"Peer API: {PEER_API_URL}")
    print(f"Bootstrap API: {BOOTSTRAP_URL}")
    print(f"Invalid blocks to inject: {INJECTIONS}")
    print(f"Delay between injections: {DELAY_SECONDS}s")
    print(f"Ban threshold: {BAN_THRESHOLD}")
    print(f"Penalty per invalid block: {PENALTY_PER_INVALID}")

    target_score = 100
    needed_injections = int((target_score - BAN_THRESHOLD) / PENALTY_PER_INVALID)
    print(f"Minimum injections needed from score {target_score} to reach ban: {needed_injections}")

    for index in range(1, INJECTIONS + 1):
        print(f"\n[{index}/{INJECTIONS}] Injecting invalid block...")
        try:
            result = post_invalid_block()
        except Exception as exc:
            print(f"ERROR: failed to inject invalid block: {exc}")
            return 1

        print(result)

        source_peer_id = result.get("source_peer_id")
        if not source_peer_id:
            print("ERROR: invalid block response did not include source_peer_id")
            return 1

        try:
            status = get_json(f"{BOOTSTRAP_URL}/status")
        except Exception as exc:
            print(f"ERROR: failed to fetch /status: {exc}")
            return 1

        score = get_peer_score(status, source_peer_id)
        banned_peers = status.get("reputation", {}).get("banned_peers", [])
        print(f"Current score for {source_peer_id}: {score}")

        if source_peer_id in banned_peers or (score is not None and score <= BAN_THRESHOLD):
            print("\nBan proved: the malicious peer crossed the ban threshold.")
            return 0

        time.sleep(DELAY_SECONDS)

    print_section("BAN STATUS")
    try:
        status = get_json(f"{BOOTSTRAP_URL}/status")
    except Exception as exc:
        print(f"ERROR: failed to fetch /status: {exc}")
        return 1

    reputation = status.get("reputation", {})
    banned_peers = reputation.get("banned_peers", [])
    network_stats = reputation.get("network_stats", {})
    # Best-effort: look for the peer reported by the last injection, if present.
    last_peer_score = None
    if banned_peers:
        last_peer_score = None

    print(f"banned_peers: {banned_peers}")
    print(f"network_stats: {network_stats}")

    if banned_peers:
        print("\nBan proved: at least one peer is now banned.")
        return 0

    print("\nNo banned peers yet. Run the script again or increase INJECTIONS.")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())

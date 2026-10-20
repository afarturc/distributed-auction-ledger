#!/usr/bin/env bash
set -euo pipefail

# Helper to boot the network in POW or POS mode for presentation.
# Usage:
#   ./demo_consensus_mode.sh POW
#   ./demo_consensus_mode.sh POS
#   ./demo_consensus_mode.sh POW 10  (with 10 peers)

MODE="${1:-POW}"
NUM_PEERS="${2:-10}"
MODE_UPPER="$(echo "$MODE" | tr '[:lower:]' '[:upper:]')"

if [[ "$MODE_UPPER" != "POW" && "$MODE_UPPER" != "POS" ]]; then
  echo "Invalid mode: $MODE"
  echo "Use: $0 POW|POS [num_peers]"
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LEDGER_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

cd "$LEDGER_DIR"

export CONSENSUS_MODE="$MODE_UPPER"

echo "======================================================================="
echo "STARTING LEDGER NETWORK"
echo "======================================================================="
echo "Mode: $MODE_UPPER"
echo "Peers: $NUM_PEERS"
echo ""

echo "[1/5] Stopping previous network..."
docker compose down -v --remove-orphans 2>/dev/null || true

echo "[2/5] Updating .env with CONSENSUS_MODE=$MODE_UPPER..."
# Update the CONSENSUS_MODE in .env file
sed -i "s/^CONSENSUS_MODE=.*/CONSENSUS_MODE=$MODE_UPPER/" "$LEDGER_DIR/.env"
echo "  Updated .env: $(grep CONSENSUS_MODE $LEDGER_DIR/.env)"

echo "[3/5] Building images with current source..."
docker compose build 2>&1 | grep -E "Successfully|error" || true

echo "[4/5] Starting network with $NUM_PEERS peers..."
docker compose up -d --scale peer="$NUM_PEERS"

echo "[5/5] Waiting for bootstrap nodes to be ready..."
bootstrap_nodes=("ledger_bootstrap_1" "ledger_bootstrap_2" "ledger_bootstrap_3")
for bootstrap in "${bootstrap_nodes[@]}"; do
  for attempt in {1..30}; do
    if docker exec "$bootstrap" curl -s http://localhost:8080/metrics >/dev/null 2>&1; then
      echo "  $bootstrap: ready"
      break
    fi
    if [ "$attempt" -eq 30 ]; then
      echo "  WARNING: $bootstrap did not start in time"
    fi
    sleep 1
  done
done

echo "[6/6] Waiting for P2P network to connect peers..."
sleep 5

echo ""
echo "======================================================================="
echo "NETWORK READY"
echo "======================================================================="
echo ""

# Verify mode
ACTUAL_MODE="$(docker exec ledger_bootstrap_1 printenv CONSENSUS_MODE 2>/dev/null || echo "UNKNOWN")"
if [[ "$ACTUAL_MODE" != "$MODE_UPPER" ]]; then
  echo "ERROR: Bootstrap started with CONSENSUS_MODE='$ACTUAL_MODE' (expected '$MODE_UPPER')"
  exit 1
fi

# Show status
RUNNING_PEERS=$(docker ps | grep -c "ledger-peer" || true)
echo "Consensus Mode: $ACTUAL_MODE"
echo "Bootstrap nodes: $(docker ps | grep -c "ledger_bootstrap" || true)"
echo "Peer nodes: $RUNNING_PEERS / $NUM_PEERS"
echo ""

echo "Next steps:"
echo "1. Submit transactions: curl -X POST http://localhost:8080/command ..."
echo "2. Mine a block: docker exec ledger_bootstrap_1 /mine"
echo "3. Watch metrics: watch -n 2 'curl -s http://localhost:8080/metrics | jq .'"
echo "4. Run demo: python3 ../scripts/demo.py"
echo ""
echo "Detach from container with: Ctrl+P, Ctrl+Q"
echo "======================================================================="

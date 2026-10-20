# Demo - Full Validation Run (PoW + PoS with Reputation + Recovery + Attack)

This guide is a complete end-to-end test plan to validate all main features.
Open 3 terminals at the repository root.

---

### Terminal Layout
- Terminal 1: network control and `/mine`
- Terminal 2: live metrics watcher
- Terminal 3: demo scripts

### Configuration
All environment variables are pre-configured in `.env`:
```env
LEDGER_WALLET_PASSWORD=ledger_demo_password
CONSENSUS_MODE=POS
NODE_WORKDIR=/data
AUTO_MINE=true
BLOCK_TIME_SECS=10
```
These values are automatically loaded via `dotenvy::dotenv()` when the application starts. Override them in Step 11 to test autonomous mining with custom intervals.

---

### Step 1: Start in PoW Mode

1. Configure `.env` for manual mining (disable auto-mining):
```bash
# Edit .env and set:
# AUTO_MINE=false
# CONSENSUS_MODE=POW
```

2. Start the network:
```bash
docker compose down -v
docker compose build
docker compose up --build --scale peer=10 --remove-orphans
```

**Note:** Logs will be displayed in the terminal. Open another terminal for subsequent steps. To detach later: `Ctrl+C` gracefully stops the network.

Wait about 15 seconds for the 10-node network to connect via Kademlia DHT.

Expected startup logs:
- `No local ledger found. Genesis Block created. Chain height: 1 (mode=POW)`
- `Wallet loaded from disk` or `New wallet generated and saved to ~/.ledger/keys.enc`

### Step 2: Start Live Metrics Watcher
Every 2 seconds this polls `/metrics` and shows the node state in real time:

```bash
watch -n 2 'curl -s http://localhost:8080/metrics | jq "{tps, blocks_mined, mempool_size, peers_connected, block_rejection_rate}"'
```

### Step 3: Run Main Functional Demo (Read + Write)
```bash
cd scripts
python3 demo.py
```

Expected:
- Commands are accepted into mempool (mempool size jumps to ~20).
- Auctions change `Pending -> Open -> Closed`.
- Each signed transaction should print `Signed transaction successfully verified and injected into the P2P network`.

### Step 4: Mine in PoW and Validate
```bash
docker logs -f ledger-peer-1 - on a different terminal
docker attach ledger_bootstrap_1
# type: /mine
```

Expected PoW behavior:
- Log includes `Mining block ...`.
- Hash has leading zeros (`000...`).
- After a valid block is found, the node logs `Block mined! Hash: ...`.

Detach without stopping container: `Ctrl+P, Ctrl+Q`.

Validate state after mining:
```bash
curl -s http://localhost:8080/ledger | jq '.blocks | length'
curl -s http://localhost:8080/metrics | jq '{blocks_mined, mempool_size}'
```

### Step 4.1: PoW — Auto-mining mode (`AUTO_MINE=true`)

If you want to observe the network mining automatically (interval-based), enable auto-mining in `.env` and restart the network:

```bash
# in .env set:
# AUTO_MINE=true
# CONSENSUS_MODE=POW

docker compose down -v
docker compose up --build --scale peer=10 --remove-orphans
```

Expected behavior in auto-mining PoW:
- Nodes periodically trigger mining on the configured `BLOCK_TIME_SECS` interval.
- Multiple peers may attempt to seal the same height; the first valid block broadcast wins and others abort their current sealing (logs show external block received and mining aborted).
- Metrics `/metrics` reflect `blocks_mined` increasing as blocks are committed.

Note: PoW auto-mining demonstrates real contention and fork resolution automatically, but with higher CPU usage across peers.

### Step 5: Simultaneous Mining Test (Race Condition / Fork Handling)

This test demonstrates what happens when **two nodes mine the same block at the same time**.
With difficulty=5, mining takes 1-5 seconds, making simultaneous mining observable. 
**Must be root terminal**

Before running check both bootstraps to verify if it is working correctly.
```bash
docker attach ledger_bootstrap_1
docker attach ledger_bootstrap_2
```


```bash
cd scripts
python3 demo_simultaneous_mining.py
```

Expected behavior:
- Both Bootstrap and Peer-1 start mining at nearly the same time.
- One node finishes first and broadcasts its block via P2P.
- The other node receives that block, validates it, and **abandons its own mining** (wasted work).
- All nodes converge to the same chain (same block hashes).
- Chain height increases by exactly 1 block (not 2).
- Logs show:
  - `[NETWORK] Received newly mined Block X from ...`
  - `[LEDGER] External block verified and added. Chain height: ...`

Key observation:
- **No fork** persists: The system uses the "longest chain rule" (Nakamoto consensus).
- Both nodes accepted the **same block**, not competing blocks.
- The losing miner's work is discarded gracefully.

This validates that your PoW consensus correctly handles the race condition.

### Step 6: Run Recovery Demo


```bash
watch -n 2 'curl -s http://localhost:8080/metrics | jq "{blocks_mined, mempool_size}"; echo "Chain Height:"; curl -s http://localhost:8080/ledger | jq ".blocks | length"'


```
In other terminal

```bash
cd scripts
sudo python3 demo_recovery.py
```

Expected:
- Active workflow commands return `200`.
- Script restarts containers and reports `state OK`.
- Chain height before/after restart remains consistent.
- Use `/status` to compare session metrics with persisted ledger height.
- When the restart is clean, you should not lose persisted blocks, only session counters.

### Step 7: Run Attack Recovery Demo

```bash
docker logs -f ledger_bootstrap_1
```


Tem de estar noutro terminal e em root
```bash
cd scripts
python3 demo_attack_recovery.py
```

Expected:
- Unsigned endpoint returns non-200 (usually `422`).
- Replay/flood attempts are rejected or deduplicated.
- Post-recovery state remains readable.
- If the script reaches the validation branch, you should see rejection logs for malformed or replayed commands.

### Step 8: Fault Injection + Catch-up Test (Nakamoto Consensus)

1. Ensure the 10-node network is back up:
```bash
docker compose up -d --scale peer=10
```

Wait ~10 seconds for peers to reconnect.

2. Kill random peers:
```bash
cd scripts
sudo python3 injection.py 3
```

 Note the ID of one killed container (one of the containers terminated by `injection.py`), for example `ledger_peer_2`.

3. Inject a new random transaction (because mempool may be cleared by restarts):
```bash
curl -X POST http://localhost:8080/command \
  -H "Content-Type: application/json" \
  -d '{"CreateAuction": {"auction_id": "TEST_'"$RANDOM"'", "item_name": "Catchup", "starting_price": 100}}'
```

4. Mine the new block on bootstrap:
```bash
docker attach ledger_bootstrap_1
# type: /mine
```

Detach with `Ctrl+P, Ctrl+Q`.

5. Restart a stopped peer to trigger catch-up:
```bash
docker restart ledger-peer-[ID]
docker logs -f ledger-peer-[ID]
```

6. To make the adversary-aware log more likely, mine once more on the Bootstrap while the restarted peer is still reconciling state.

In PoS mode, if that peer forwards an outdated/invalid block, the Bootstrap log should show:
```text
[REPUTATION] Penalized relay peer ... for forwarding an invalid block
```

In PoW mode, you may still see block rejection without reputation penalization, for example:
```text
[LEDGER ERROR] Rejected external block: Block failed cryptographic or PoW validation
```

Expected:
- Restarted node catches up and validates chain automatically.
- If a stale peer forwards a bad block during the restart race in PoS mode, the Bootstrap penalizes it and the message appears in the log.
- Logs show chain adoption/fork resolution when the node is behind.
- In the same catch-up path, you may also see a PoS-side rejection such as `Rejected by PoS: miner ... has negative reputation score ...`.

### Step 9: Switch to PoS with Reputation and Observe Reputation Updates

1. Stop the current network:
```bash
docker compose down
```

2. Configure `.env` for PoS mode with reputation:
```bash
# Edit .env and set:
# AUTO_MINE=true
# CONSENSUS_MODE=POS
```

3. Start the network in PoS mode:
```bash
docker compose up --scale peer=10
```

Quick verification (must show `POS`):
```bash
docker exec ledger_bootstrap_1 printenv CONSENSUS_MODE
docker logs ledger_bootstrap_1 | grep -E "consensus mode" | tail -1
```

Wait ~15 seconds so the Kademlia P2P network reconnects peers.

In PoS mode, the network runs autonomously. With `AUTO_MINE=true`, nodes periodically attempt to propose blocks based on their VRF results and reputation, so you do not manually call `/mine` in this section.

Recommendation: PoS is intended to run with `AUTO_MINE=true` in this demo — PoS proposals are probabilistic (VRF + reputation) and will rarely succeed if you only trigger mining manually. Using `AUTO_MINE=true` ensures peers attempt proposals each slot and lets you observe eligibility, proposals and reputation effects naturally.

Before creating any transactions:

```bash
curl -X POST http://localhost:8080/command -H "Content-Type: application/json" -d '{"CreateAuction": {"auction_id": "POS_TEST_'$RANDOM'", "item_name": "PoS Reputation Demo", "starting_price": 500}}'
```

PoS behavior:
- Unlike PoW, you don't need to manually mine. The node will automatically try to seal a block every few seconds (governed by `BLOCK_TIME_SECS`).
- To observe it, keep the bootstrap logs open and wait for the next automatic proposal cycle.

Expected PoS with reputation behavior:
  - If eligible: `Sealing block with PoS + VRF rules... Validated by Stake + Reputation + VRF.`
  - If not eligible: `Mining failed during sealing: Miner ... not eligible for PoS block proposal`
  - Rejection of invalid blocks: `[REPUTATION] Penalized relay peer ... for forwarding an invalid block`

Detach with `Ctrl+P, Ctrl+Q`.

### Step 9.1: PoS Reputation Test (Fault Injection)

Demonstrates that a banned peer cannot participate in the network, even with normal operations.

Terminal 1 - Monitor the bootstrap rejection logs:
```bash
docker logs -f ledger_bootstrap_1 | grep -E "REPUTATION|Penalized|banned|Rejected by PoS"
```

Terminal 2 - Ban a peer by injecting invalid blocks:
```bash
cd scripts
python3 demo_reputation_ban.py
```

Terminal 3 - Verify the ban via API:
```bash
curl -s http://localhost:8080/status | jq '.reputation.banned_peers'
```

The output should show exactly which peer ID was banned.

To map that ID to the right container, compare it with the node's own `local_peer_id` from the peer API:
```bash
curl -s http://localhost:8084/status | jq '.local_peer_id'
```

Terminal 3b - Map the banned peer to its Docker container:
```bash
docker ps --format 'table {{.Names}}\t{{.Ports}}' | grep 8084
```

This helps you confirm which container corresponds to the peer API port before checking the next automatic PoS proposal.

Terminal 4 - Prove the ban blocks normal participation in PoS:
```bash
cd scripts
python3 demo_peer_ban_usage_attempt.py
```

Expected result:
- The script prints the banned peer ID(s) from `/status`
- The banned peer should refuse to participate in automatic PoS proposal selection and print a message like `peer ... is banned and cannot mine`
- Both peer height and bootstrap height should stay unchanged
- The script ends with `SUCCESS` only when both heights stay fixed

If you are unsure which container to use, compare `banned_peers` from the bootstrap with `local_peer_id` from the peer API.

Optional manual confirmation:
```bash
curl -s http://localhost:8080/ledger | jq '.blocks | length'
curl -s http://localhost:8084/ledger | jq '.blocks | length'
```

### Step 9.2: VRF (Verifiable Random Function) in PoS

This test validates that block proposal eligibility uses Verifiable Random Functions combined with reputation scores,
preventing nodes from cheating during block selection.

Prerequisites:
- Network still running in PoS mode from Step 9
- Bootstrap and peer nodes are healthy
- Consensus mode is still `POS`

Terminal 2 - Run the VRF test script:
```bash
cd scripts
python3 demo_vrf_pos.py
```

The script runs 4 tests:
1. **VRF Proof Presence**: Verifies that all non-genesis blocks contain valid VRF output (32 bytes) and VRF proof (64 bytes)
2. **Mining with VRF**: Mines new blocks and confirms that VRF outputs vary (non-deterministic across blocks)
3. **Reputation Effect**: Analyzes which miners propose blocks and how reputation affects eligibility
4. **PoS Mode Verification**: Confirms the system is running in PoS mode (nonce=0, fast sealing ~250μs)

Expected output:
```
TEST 1: Verifying VRF proof presence in blocks
Current chain height: X
Block 1: VRF proof found (output[:16]=...)
Block 2: VRF proof found (output[:16]=...)
...

TEST 2: Mining blocks with VRF eligibility
Chain height increased: X -> Y
Block X: VRF output (first 8 bytes) = ...

TEST 3: Reputation effect on block eligibility
Miner eligibility analysis:
  Miner abc123: proposed N blocks
  Miner def456: proposed M blocks

TEST 4: Verify Proof of Stake mode
Ledger state loaded successfully
Last block nonce: 0

TEST SUMMARY
PASS: VRF_Blocks
PASS: VRF_Mining
PASS: Reputation
PASS: PoS_Mode

ALL TESTS PASSED - VRF PoS implementation is working!
```

If tests fail, check:
- Network is running: `docker ps | grep ledger`
- API is responsive: `curl http://localhost:8080/metrics`
- Consensus mode is POS: `docker exec ledger_bootstrap_1 printenv CONSENSUS_MODE`
- Check logs for errors: `docker logs -f ledger_bootstrap_1`

The VRF eligibility formula used:
```
eligible = (1 - vrf_weight) × reputation_weight > D_threshold

where:
  vrf_weight = random value from [0, 1) derived from VRF proof
  reputation_weight = miner reputation / max_reputation
  D_threshold = 0.3
```

This ensures:
- **Unpredictability**: VRF output cannot be predicted beforehand
- **Verifiability**: Any node can verify the VRF proof is valid
- **Accountability**: Reputation score influences eligibility (better peers propose more blocks)
- **Non-replayability**: VRF output changes for each block (included in block hash)

### Step 9.3: Off-chain Storage Demo (Ledger + DHT)

This test validates the hybrid architecture:
- **Ledger**: Stores critical "light" data (IDs, prices, metadata keys).
- **DHT**: Stores "heavy" metadata (long descriptions) using Kademlia `STORE` and `FIND_VALUE`.

1. Ensure the network is running.
2. Run the off-chain demo script:
```bash
cd scripts
python3 demo_offchain.py
```

3. Observe the output:
- The script sends a "Rich Auction" with a long description.
- `docker logs -f ledger_bootstrap_1 | grep -E "FIND_VALUE|get_record|Kademlia|STORE|metadata|Api_test_meta|SYNC"
- The Ledger state (`/auctions`) only shows a `metadata_key`.
- The peer console logs show `[DHT SUCCESS] FIND_VALUE Result`, proving the heavy data was retrieved from the DHT, not the blockchain.

### Step 10: Switch Back to PoW and Repeat Visual Mining

1. Stop the current network:
```bash
docker compose down
```

2. Configure `.env` for PoW mode with manual mining:
```bash
# Edit .env and set:
# AUTO_MINE=true
# CONSENSUS_MODE=POW
```

3. Start the network in PoW mode:
```bash
docker compose up --scale peer=10
```

Wait ~15 seconds so the Kademlia P2P network reconnects peers.

Before creating any transaction:

```bash
curl -X POST http://localhost:8080/command -H "Content-Type: application/json" -d '{"CreateAuction": {"auction_id": "POW_TEST_'$RANDOM'", "item_name": "PoW Demo", "starting_price": 500}}'
```

Mine in PoW mode:
```bash
docker attach ledger_bootstrap_1
# type: /mine
```

Expected PoW behavior:
- Log includes `Mining block ...`.
- Block mining should take longer than PoS sealing.
- Hash must satisfy the leading-zero difficulty target.

Detach with `Ctrl+P, Ctrl+Q`.

### Step 11: Production Mode (Autonomous Mining & Liveness)

This step demonstrates the transition from a controlled testing environment (manual `/mine`) to a fully autonomous production network using asynchronous background tasks (`tokio::time::interval`).

1. Stop the current network:
```bash
docker compose down
```

2. Configure `.env` for autonomous mining:
```bash
# Edit .env and set:
# AUTO_MINE=true
# BLOCK_TIME_SECS=10
# CONSENSUS_MODE=POS
```

3. Start the network with autonomous mining enabled:
```bash
docker compose up -d --scale peer=10
```

4. Monitor the Bootstrap node for autonomous activity:
```bash
docker logs -f ledger_bootstrap_1
```

Expected behavior:
- **Even without injecting transactions**, the node will wake up every 10 seconds.
- You will see logs indicating that the VRF eligibility was checked. If the VRF criteria are met, an **Empty Block** will be sealed to maintain chain continuity.
- Example log: `[AUTO_MINE] Block N mined with 0 transactions in Xms. Broadcasting...`

4. Inject transactions while the autopilot is running:
```bash
cd scripts
python3 demo.py
```

5. Watch the Live Metrics (Terminal 2):
```bash
watch -n 2 'curl -s http://localhost:8080/metrics | jq "{blocks_mined, mempool_size}"'
```

Expected behavior:
- The mempool size will spike when the script injects the transactions.
- You **do not** need to type `/mine`.
- Within the configured interval, the autonomous task will capture the mempool, seal a full block if eligible, and the mempool size will automatically drop back to 0.
- **Race Condition Handling:** If multiple nodes are eligible and mine a block at the same time, the `tokio::select!` macro and the new async yielding logic ensure that the first block to reach a node will cancel its local mining process instantly, maintaining consensus without forks.

### Step 12: Final API Health Checklist
```bash
curl -s http://localhost:8080/ledger | jq '.blocks | length'
curl -s http://localhost:8080/auctions | jq .
curl -s http://localhost:8080/metrics | jq .
curl -s http://localhost:8080/status | jq .
curl -s http://localhost:8080/finality/block/1 | jq .
```

Presentation target:
- 10 nodes/miners running
- 20 write operations completed
- Read operations interleaved during the demo
- Mining, propagation, validation, and reputation updates visible in logs
- Step 5 validates race condition handling (simultaneous mining converges)
- The terminal should show verbose reputation logs, especially during step 8 (Fault Injection) and step 9 (PoS).

If all endpoints return valid JSON and no fatal peer crashes, the full demo passed.

### Step 13: Clean Up
```bash
docker compose down -v
```

## Appendix — Example: a signed `CloseAuction`

Nodes only accept sensitive operations as signed transactions. Use the Python scripts to build the signature; below is the payload format expected by `POST /command/signed`.

Exemplo `curl` (substitua os placeholders):

```bash
curl -X POST http://localhost:8080/command/signed \
  -H "Content-Type: application/json" \
  -d '{
    "command": { "CloseAuction": { "auction_id": "MY_AUCTION_ID" } },
    "sender_id": "<hex 16 bytes>",
    "public_key": "<hex 32 bytes>",
    "signature": "<hex 64 bytes>"
  }'
```

Generate `public_key`, `sender_id` and `signature` with `scripts/demo.py` (or any client using Ed25519 that derives `sender_id = SHA256(public_key)[0..16]`). The node verifies the signature before admitting the command to the mempool, which is what guarantees that only the authenticated `creator_id` can close an auction.
import subprocess
import random
import sys

def get_running_peers():
    """Get list of currently running peer containers"""
    try:
        result = subprocess.run(
            ["docker", "ps", "--format", "{{.Names}}"],
            capture_output=True,
            text=True,
            timeout=5
        )
        if result.returncode == 0:
            containers = result.stdout.strip().split("\n")
            peer_names = [c for c in containers if "ledger-peer" in c and c.strip()]
            return peer_names
        else:
            return []
    except Exception as e:
        print(f"Error getting container list: {e}")
        return []


def inject_faults(num_nodes_to_kill=3):
    print("=" * 70)
    print("STARTING RANDOM FAULT INJECTION")
    print("=" * 70)
    
    # Get currently running peer containers
    available_peers = get_running_peers()
    
    if not available_peers:
        print("Error: No peer containers found running")
        return
    
    print(f"Found {len(available_peers)} running peers: {', '.join(available_peers)}")
    
    # Select up to num_nodes_to_kill random peers
    num_to_kill = min(num_nodes_to_kill, len(available_peers))
    targets = random.sample(available_peers, num_to_kill)
    
    print(f"Targets randomly selected for termination: {', '.join(targets)}")
    
    command = ["docker", "stop"] + targets
    
    result = subprocess.run(command, capture_output=True, text=True)
    
    if result.returncode == 0:
        print(f"\nSuccess: The following {num_to_kill} containers were terminated:")
        print(result.stdout.strip())
    else:
        print(f"\nError terminating containers: {result.stderr.strip()}")
            
    print("\n" + "=" * 70)
    print("FAULT INJECTION COMPLETE")
    print("=" * 70)
    print("Check docker compose logs")


if __name__ == "__main__":
    if len(sys.argv) == 2:
        try:
            num_to_kill = int(sys.argv[1])
            inject_faults(num_nodes_to_kill=num_to_kill)
        except ValueError:
            print("Please provide a valid integer for the number of nodes to kill.")
    else:
        inject_faults()
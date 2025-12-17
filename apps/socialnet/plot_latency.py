import subprocess
import json
import matplotlib.pyplot as plt
import os

# --- Configuration ---
# Feel free to adjust these RPS levels to find your breaking point
RPS_LEVELS = [100, 200, 300, 400, 500] 
DURATION = "15s"
HOST = "localhost:8080"
PROTO_FILE = "./proto/compose_post.proto"
CALL_METHOD = "compose_post.ComposePostService.ComposePost"
DATA_FILE = "./payloads.json"

def create_payload_file():
    """Creates the payloads.json file with the randomization template."""
    content = {
        "req_id": "{{.RequestNumber}}",
        "username": "user_{{.RequestNumber}}",
        "user_id": "{{.RequestNumber}}",
        "text": "Hello World",
        "media_ids": [1, 2],
        "media_types": ["image", "text"],
        "post_type": "REPOST",
        "carrier": {}
    }
    
    with open(DATA_FILE, "w") as f:
        json.dump(content, f, indent=2)
    
    print(f"Verified {DATA_FILE} exists.")

def run_ghz_test(rps):
    """Runs ghz and returns the P99 latency in milliseconds."""
    print(f"Running test for {rps} RPS...")
    
    cmd = [
        "ghz",
        "--insecure",
        "--proto", PROTO_FILE,
        "--call", CALL_METHOD,
        "--data-file", DATA_FILE,
        "--rps", str(rps),
        "--duration", DURATION,
        "-O", "json", 
        HOST
    ]

    try:
        result = subprocess.run(cmd, capture_output=True, text=True)
        
        try:
            data = json.loads(result.stdout)
            
            # --- LOGIC MATCHING YOUR DEBUG OUTPUT ---
            # We look for the entry where 'percentage' is 99 in 'latencyDistribution'
            p99_ns = None
            if "latencyDistribution" in data:
                for entry in data["latencyDistribution"]:
                    if entry.get("percentage") == 99:
                        p99_ns = entry.get("latency")
                        break
            
            if p99_ns is None:
                print(f"  [Error] Could not find 99% entry in latencyDistribution.")
                return None

            # Convert nanoseconds (e.g., 67892751) to milliseconds (67.89 ms)
            return p99_ns / 1_000_000.0 

        except json.JSONDecodeError:
            print("  [Error] ghz output invalid JSON.")
            print(f"  [Raw Output]: {result.stderr if result.stderr else result.stdout}")
            return None

    except Exception as e:
        print(f"  [Script Error]: {e}")
        return None

def main():
    create_payload_file()

    results_rps = []
    results_p99 = []

    print("-" * 40)
    for rps in RPS_LEVELS:
        p99 = run_ghz_test(rps)
        
        if p99 is not None:
            print(f"  -> Success: {p99:.2f} ms P99 Latency")
            results_rps.append(rps)
            results_p99.append(p99)
        else:
            print(f"  -> Skipping {rps} RPS due to failure.")

    if not results_rps:
        print("\nNo data collected.")
        return

    # Plotting
    print("-" * 40)
    print("Plotting results...")
    plt.figure(figsize=(10, 6))
    plt.plot(results_rps, results_p99, marker='o', linestyle='-', color='b')
    plt.title(f'Load Test: P99 Latency vs RPS\n({DURATION} duration)')
    plt.xlabel('Requests Per Second (RPS)')
    plt.ylabel('P99 Latency (ms)')
    plt.grid(True)
    
    output_img = "latency_plot.png"
    plt.savefig(output_img)
    print(f"Graph saved to {output_img}")

if __name__ == "__main__":
    main()
    
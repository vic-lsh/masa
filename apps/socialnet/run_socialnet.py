import json
import subprocess
import os

# 1. Load the JSON configuration
with open('replicas.json', 'r') as f:
    replicas = json.load(f)

# 2. Prepare environment variables
# We copy the current system env vars and update them with our JSON values
# The values must be strings for os.environ
env_vars = os.environ.copy()
for key, value in replicas.items():
    env_vars[key] = str(value)

# 3. Run Docker Compose with these environment variables
print("Starting Docker Compose with the following replica configuration:")
print(json.dumps(replicas, indent=2))

try:
    # This runs 'docker-compose up -d'. Remove '-d' if you want to see logs in this terminal.
    # subprocess.run(["docker", "compose", "up", "-d", "--remove-orphans"], env=env_vars, check=True)
    subprocess.run(
        ["docker", "compose", "-f", "docker-compose.yaml", "up", "-d", "--remove-orphans"],
        env=env_vars, 
        check=True
    )
    print("\n✅ Services started successfully.")
except subprocess.CalledProcessError as e:
    print(f"\n❌ Error starting services: {e}")


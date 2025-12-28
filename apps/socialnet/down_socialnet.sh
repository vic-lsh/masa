#!/bin/bash

echo "Stopping services defined in docker-compose.yaml..."

# -f points to your specific file
# --remove-orphans cleans up containers not defined in the compose file anymore
docker compose -f docker-compose.yaml down --remove-orphans

echo "✅ Services stopped."


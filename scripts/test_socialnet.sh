#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
socialnet_dir="$repo_root/apps/socialnet"
build_script="$socialnet_dir/build_socialnet.sh"
no_cache=""
cleanup=true

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
    --no-cache)
        no_cache="--no-cache"
        shift 1
        ;;
    --no-cleanup)
        cleanup=false
        shift 1
        ;;
    *)
        echo "Unknown argument: $1"
        echo "Usage: $0 [--no-cache] [--no-cleanup]"
        exit 1
        ;;
    esac
done

# Check for required tools
if ! command -v docker >/dev/null 2>&1; then
    echo "Docker is required to run the socialnet test." >&2
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "Cargo is required to run the socialnet test." >&2
    exit 1
fi

# Check that socialnet directory exists
if [ ! -d "$socialnet_dir" ]; then
    echo "Socialnet directory not found at $socialnet_dir" >&2
    exit 1
fi

# Check that build script exists
if [ ! -f "$build_script" ]; then
    echo "Build script not found at $build_script" >&2
    exit 1
fi

# Check that docker-compose.yaml exists
if [ ! -f "$socialnet_dir/docker-compose.yaml" ]; then
    echo "docker-compose.yaml not found at $socialnet_dir/docker-compose.yaml" >&2
    exit 1
fi

echo "========================================"
echo "Building socialnet application..."
echo "========================================"

# Build the socialnet docker image
if [ -n "$no_cache" ]; then
    echo "Building with --no-cache flag..."
    cd "$repo_root"
    docker build \
        --ulimit nofile=65536:65536 \
        --no-cache \
        -t socialnet-generic-svc:latest \
        -f ./apps/socialnet/Dockerfile \
        .
else
    "$build_script"
fi

if [ $? -ne 0 ]; then
    echo "Build failed!" >&2
    exit 1
fi

echo "Build completed successfully."
echo ""

echo "========================================"
echo "Starting socialnet services..."
echo "========================================"

cd "$socialnet_dir"

# Stop any existing deployment first
echo "Stopping any existing deployment..."
docker compose down -v 2>/dev/null || true
echo ""

# Set JWT_SECRET environment variable (required by user-service)
export JWT_SECRET="test-secret-key-for-ci"

# Start services in detached mode
docker compose up -d

if [ $? -ne 0 ]; then
    echo "Failed to start services!" >&2
    exit 1
fi

echo "Services started. Waiting for them to be healthy..."
echo ""

echo "========================================"
echo "Verifying services are running..."
echo "========================================"

# Function to check if a service is running
check_service_running() {
    local service_name=$1
    local quiet=${2:-false}
    if docker compose ps --services --filter "status=running" | grep -q "^${service_name}$"; then
        if [ "$quiet" != "true" ]; then
            echo "✓ Service running: $service_name"
        fi
        return 0
    else
        if [ "$quiet" != "true" ]; then
            echo "✗ Service NOT running: $service_name" >&2
        fi
        return 1
    fi
}

# List of all services to check
SERVICES=(
    # Database services
    "post-storage-mongo"
    "user-timeline-mongo"
    "user-mongo"
    "social-graph-mongo"
    "user-mention-mongo"
    "url-shorten-mongo"
    # Redis services
    "user-timeline-redis"
    "home-timeline-redis"
    "user-redis"
    "social-graph-redis"
    "write-home-timeline-redis"
    # Memcached services
    "post-storage-memcached"
    "user-mention-memcached"
    # RabbitMQ
    "rabbitmq"
    # Application services
    "post-storage-service"
    "user-timeline-service"
    "home-timeline-service"
    "write-home-timeline-service"
    "user-service"
    "social-graph-service"
    "unique-id-service"
    "media-service"
    "text-service"
    "user-mention-service"
    "url-shorten-service"
    "compose-post-service"
)

# Function to check all services
# Returns the number of failed services
check_all_services() {
    local quiet=${1:-false}
    local failed_count=0

    for service in "${SERVICES[@]}"; do
        check_service_running "$service" "$quiet" || ((failed_count++))
    done

    return $failed_count
}

# Retry loop - check up to 6 times with 10s gap
max_attempts=6
attempt=1
failed=true
total_services=${#SERVICES[@]}

while [ $attempt -le $max_attempts ]; do
    echo "Attempt $attempt/$max_attempts..."
    date

    # Run check_all_services in a conditional context so set -e doesn't cause
    # the script to exit early when services are still starting up.
    if check_all_services "true"; then
        failed_count=0
    else
        failed_count=$?
    fi
    running_count=$((total_services - failed_count))
    
    echo "Services running: $running_count/$total_services ($failed_count failed)"
    if [ $failed_count -ne 0 ]; then
        echo "Current docker compose service status:"
        docker compose ps || echo "docker compose ps failed with exit code $?"
    fi
    
    if [ $failed_count -eq 0 ]; then
        echo ""
        echo "All services are running! Showing final status:"
        check_all_services "false"
        failed=false
        break
    else
        if [ $attempt -lt $max_attempts ]; then
            echo "Waiting 10 seconds before retry..."
            sleep 10
        else
            echo "Services still not ready after $max_attempts attempts."
        fi
        attempt=$((attempt + 1))
    fi
done

echo ""

if [ "$failed" = true ]; then
    echo "Some services failed to start properly!" >&2
    echo "Showing service status:"
    docker compose ps
    echo ""
    echo "Showing logs for failed services:"
    docker compose logs --tail=50
    
    if [ "$cleanup" = true ]; then
        echo ""
        echo "Cleaning up..."
        docker compose down -v
    fi
    echo "Socialnet test failed! Some services failed to start properly." >&2
    exit 1
fi

echo "========================================"
echo "All services are running successfully!"
echo "========================================"

# Cleanup
if [ "$cleanup" = true ]; then
    echo ""
    echo "Cleaning up..."
    docker compose down -v
    echo "Cleanup completed."
else
    echo ""
    echo "Skipping cleanup (--no-cleanup flag was set)"
    echo "To clean up manually, run: cd $socialnet_dir && docker compose down -v"
fi

echo ""
echo "Socialnet test passed successfully!"

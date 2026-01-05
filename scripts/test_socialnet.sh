#!/bin/bash

# Use -uo pipefail (without -e) so we can explicitly handle errors
# This prevents the script from exiting unexpectedly on non-critical failures
set -uo pipefail

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
build_exit_code=0
if ! "$build_script" $no_cache; then
    build_exit_code=1
fi

if [ $build_exit_code -ne 0 ]; then
    echo "Build failed!" >&2
    exit 1
fi

echo "Build completed successfully."
echo ""

echo "========================================"
echo "Starting socialnet services..."
echo "========================================"

if ! cd "$socialnet_dir"; then
    echo "Failed to change to socialnet directory: $socialnet_dir" >&2
    exit 1
fi

# Stop any existing deployment first
echo "Stopping any existing deployment..."
docker compose down -v 2>/dev/null || true
echo ""

# Set JWT_SECRET environment variable (required by user-service)
export JWT_SECRET="test-secret-key-for-ci"

# Start services in detached mode
if ! docker compose up -d; then
    echo "Failed to start services!" >&2
    exit 1
fi

echo "Services started. Waiting for them to be healthy..."
echo ""

echo "========================================"
echo "Verifying services are running..."
echo "========================================"

# List of all services to check (defined early so it can be used in logging)
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

# Verify we're in the right directory and docker compose is working
if [ ! -f "docker-compose.yaml" ]; then
    echo "ERROR: docker-compose.yaml not found in current directory: $(pwd)" >&2
    exit 1
fi

# Test docker compose command
if ! docker compose version >/dev/null 2>&1; then
    echo "ERROR: docker compose command is not working!" >&2
    docker compose version 2>&1 || true
    exit 1
fi

echo "Docker compose is working. Current directory: $(pwd)"
echo ""
echo "Expected services to check (${#SERVICES[@]} total):"
for service in "${SERVICES[@]}"; do
    echo "  - $service"
done
echo ""

# Function to check if a service is running
check_service_running() {
    local service_name=$1
    local quiet=${2:-false}
    
    # Get all running services
    local running_services
    running_services=$(docker compose ps --services --filter "status=running" 2>&1)
    local ps_exit_code=$?
    
    if [ $ps_exit_code -ne 0 ]; then
        if [ "$quiet" != "true" ]; then
            echo "✗ Failed to check service status for $service_name (docker compose ps failed with code $ps_exit_code)" >&2
            echo "  Error output: $running_services" >&2
        fi
        return 1
    fi
    
    # Check if service is in the running list
    if echo "$running_services" | grep -q "^${service_name}$"; then
        if [ "$quiet" != "true" ]; then
            echo "✓ Service running: $service_name"
        fi
        return 0
    else
        if [ "$quiet" != "true" ]; then
            echo "✗ Service NOT running: $service_name" >&2
            # Show actual status for this service
            local service_status
            service_status=$(docker compose ps "$service_name" 2>&1 | tail -n +3 || echo "Could not get status")
            echo "  Status details: $service_status" >&2
        fi
        return 1
    fi
}

# Function to check all services
# Returns the number of failed services via return code
# Outputs the list of failed services to stdout (space-separated)
check_all_services() {
    local quiet=${1:-false}
    local failed_count=0
    local failed_services=()

    for service in "${SERVICES[@]}"; do
        # Use a subshell to isolate errors from individual service checks
        if ! check_service_running "$service" "$quiet" 2>/dev/null; then
            failed_count=$((failed_count + 1))
            failed_services+=("$service")
        fi
    done

    # Output failed services list (if any)
    if [ ${#failed_services[@]} -gt 0 ]; then
        echo "${failed_services[*]}"
    fi

    # Ensure we return a valid exit code (0-255)
    if [ $failed_count -gt 255 ]; then
        return 255
    else
        return $failed_count
    fi
}

# Retry loop - check up to 6 times with 10s gap
max_attempts=6
attempt=1
failed=true
total_services=${#SERVICES[@]}

while [ $attempt -le $max_attempts ]; do
    echo "Attempt $attempt/$max_attempts..."
    date
    echo ""

    # First, let's see what docker compose reports
    echo "Checking docker compose status..."
    if ! docker compose ps >/dev/null 2>&1; then
        echo "ERROR: docker compose ps command failed!" >&2
        docker compose ps 2>&1 || true
        exit 1
    fi
    
    # Show summary of all services
    echo "All services status summary:"
    docker compose ps --format "table {{.Service}}\t{{.Status}}" 2>&1 | head -20 || true
    echo ""
    
    # Get list of all services that docker compose knows about
    echo "Services known to docker compose:"
    docker compose ps --services 2>&1 | sed 's/^/  - /' || echo "  (could not list services)"
    echo ""
    
    # Get list of running services
    echo "Services with 'running' status:"
    docker compose ps --services --filter "status=running" 2>&1 | sed 's/^/  - /' || echo "  (could not list running services)"
    echo ""

    # Debug: indicate we're about to check all services (helps diagnose where script might fail)
    echo "DEBUG: About to check all ${#SERVICES[@]} services..." >&2

    # Run check_all_services and capture both the output (failed services list) 
    # and return code (failed count)
    failed_services_list=$(check_all_services "true" 2>&1)
    failed_count=$?
    
    # Validate that we got a reasonable result
    # If failed_count is negative or way too large, something went wrong
    if [ $failed_count -lt 0 ] || [ $failed_count -gt $total_services ]; then
        echo "ERROR: check_all_services returned invalid count: $failed_count (expected 0-$total_services)" >&2
        echo "This might indicate a problem with the service checking logic." >&2
        # Default to all services failed if we can't check properly
        failed_count=$total_services
        if [ -z "$failed_services_list" ]; then
            failed_services_list="${SERVICES[*]}"
        fi
    fi
    
    # Debug output to help diagnose issues
    if [ $failed_count -gt $total_services ]; then
        echo "WARNING: failed_count ($failed_count) is greater than total_services ($total_services)" >&2
        echo "This might indicate an error in check_all_services" >&2
        failed_count=$total_services  # Cap it to prevent arithmetic errors
    fi
    
    # Safely calculate running count, defaulting to 0 if arithmetic fails
    if ! running_count=$((total_services - failed_count)) 2>/dev/null; then
        echo "ERROR: Failed to calculate running_count (total_services=$total_services, failed_count=$failed_count)" >&2
        running_count=0
    fi
    
    echo "Services running: $running_count/$total_services ($failed_count failed)"
    if [ $failed_count -ne 0 ]; then
        if [ -n "$failed_services_list" ]; then
            echo "Failed services: $failed_services_list"
            echo ""
            echo "Detailed status for failed services:"
            for failed_service in $failed_services_list; do
                echo "  Service: $failed_service"
                docker compose ps "$failed_service" 2>&1 | tail -n +3 | sed 's/^/    /' || echo "    (could not get status)"
                echo "  Recent logs (last 20 lines):"
                docker compose logs --tail=20 "$failed_service" 2>&1 | sed 's/^/    /' || echo "    (could not get logs)"
                echo ""
            done
        fi
        echo "Full docker compose service status:"
        docker compose ps || echo "docker compose ps failed with exit code $?"
    fi
    
    if [ $failed_count -eq 0 ]; then
        echo ""
        echo "All services are running! Showing final status:"
        # Final check - don't fail if this has issues, we already confirmed all services are running
        if ! check_all_services "false" >/dev/null 2>&1; then
            final_check_status=$?
            echo "WARNING: Final service check returned non-zero status: $final_check_status" >&2
            echo "This is unexpected but continuing..." >&2
        fi
        failed=false
        break
    else
        if [ $attempt -lt $max_attempts ]; then
            echo ""
            echo "Waiting 10 seconds before retry..."
            sleep 10
            echo ""
        else
            echo ""
            echo "Services still not ready after $max_attempts attempts."
        fi
        attempt=$((attempt + 1))
    fi
done

echo ""

if [ "$failed" = true ]; then
    echo "========================================"
    echo "ERROR: Some services failed to start properly!" >&2
    echo "========================================"
    echo ""
    
    echo "Full service status:"
    if ! docker compose ps; then
        echo "WARNING: docker compose ps failed" >&2
    fi
    echo ""
    
    echo "Services that were expected but not running:"
    if [ -n "$failed_services_list" ]; then
        for failed_service in $failed_services_list; do
            echo "  - $failed_service"
        done
    else
        echo "  (could not determine failed services)"
    fi
    echo ""
    
    echo "Showing recent logs for all services (last 30 lines each):"
    if ! docker compose logs --tail=30; then
        echo "WARNING: docker compose logs failed" >&2
    fi
    echo ""
    
    echo "Checking for containers that exited:"
    if ! docker compose ps --filter "status=exited"; then
        echo "  (no exited containers or command failed)"
    fi
    echo ""
    
    echo "Checking for containers that are restarting:"
    if ! docker compose ps --filter "status=restarting"; then
        echo "  (no restarting containers or command failed)"
    fi
    echo ""
    
    if [ "$cleanup" = true ]; then
        echo "Cleaning up..."
        if ! docker compose down -v; then
            echo "WARNING: docker compose down failed" >&2
        fi
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
    if ! docker compose down -v; then
        echo "WARNING: docker compose down failed during cleanup" >&2
        exit 1
    fi
    echo "Cleanup completed."
else
    echo ""
    echo "Skipping cleanup (--no-cleanup flag was set)"
    echo "To clean up manually, run: cd $socialnet_dir && docker compose down -v"
fi

echo ""
echo "Socialnet test passed successfully!"

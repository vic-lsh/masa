#!/bin/bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$SCRIPT_DIR/../.."

APP="socialnet"
TAG="latest"
APP_CONFIG_PATH="exp/socialnet/data/in/template/socialnet.json"
GEN_CONFIG_PATH="exp/socialnet/data/in/template/gen_config.json"
NO_CACHE=""

while [[ $# -gt 0 ]]; do
  case $1 in
    --no-cache)
      NO_CACHE="--no-cache"
      shift 1
      ;;
    *)
      echo "Unknown argument: $1" >&2
      echo "Usage: $0 [--no-cache]" >&2
      exit 1
      ;;
  esac
done

BINARIES=(
  socialnet_client_bench
  compose_post_server
  home_timeline_server
  user_timeline_server
  post_storage_server
  social_graph_server
  write_home_timeline_server
  user_service
  media_service
  unique_id_service
  textservice_server
  usermention_server
  url_shorten_server
)

cd "$REPO_ROOT"

echo "Building socialnet images (tag: ${TAG})..."

docker buildx build \
  -f ./exp/common/docker-build/Dockerfile \
  --target builder \
  --build-arg APP="${APP}" \
  --build-arg CACHE_ID="${APP}-${TAG}" \
  --ulimit nofile=4096:4096 \
  ${NO_CACHE} \
  -t "${APP}_builder:${TAG}" \
  .

docker buildx build \
  -f ./exp/common/docker-build/Dockerfile \
  --target runtime-base \
  --build-arg APP="${APP}" \
  --build-arg LOG_LEVEL=info \
  --build-arg APP_CONFIG_PATH="${APP_CONFIG_PATH}" \
  --build-arg GEN_CONFIG_PATH="${GEN_CONFIG_PATH}" \
  --build-arg CACHE_ID="${APP}-${TAG}" \
  ${NO_CACHE} \
  -t "${APP}_runtime-base:${TAG}" \
  .

for binary in "${BINARIES[@]}"; do
  docker buildx build \
    -f ./exp/common/docker-build/Dockerfile \
    --target runtime \
    --build-arg APP="${APP}" \
    --build-arg LOG_LEVEL=info \
    --build-arg APP_CONFIG_PATH="${APP_CONFIG_PATH}" \
    --build-arg GEN_CONFIG_PATH="${GEN_CONFIG_PATH}" \
    --build-arg BINARY_NAME="${binary}" \
    --build-arg CACHE_ID="${APP}-${TAG}" \
    ${NO_CACHE} \
    -t "${binary}:${TAG}" \
    .
done

echo "Build complete. Socialnet images tagged '${TAG}' are ready."

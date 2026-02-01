#!/bin/bash

set -e

if [ -z "${LOG_LEVEL}" ]; then
  echo "Error: LOG_LEVEL environment variable is not set." >&2
  exit 1
fi

# Export the RUST_LOG variable for Rust's logging frameworks.
export RUST_LOG="${LOG_LEVEL}"

# Auto-detect binary name: each image contains exactly one binary in /usr/local/bin
# If BINARY_NAME is explicitly set (for backward compatibility), use it
# Otherwise, find the single binary in /usr/local/bin
if [ -z "${BINARY_NAME}" ]; then
  binaries=$(find /usr/local/bin -maxdepth 1 -type f -executable 2>/dev/null || true)
  if [ -z "$binaries" ]; then
    echo "Error: No binary found in /usr/local/bin" >&2
    exit 1
  fi
  binary_count=$(echo "$binaries" | wc -l)
  if [ "$binary_count" -ne 1 ]; then
    echo "Error: Expected exactly one binary in /usr/local/bin, found $binary_count" >&2
    echo "Found binaries: $binaries" >&2
    exit 1
  fi
  BINARY_NAME=$(basename "$binaries")
fi

echo "Starting binary '${BINARY_NAME}' with log level '${LOG_LEVEL}'..."

# Construct the base command. See Dockerfile for where the binary resides.
CMD="/usr/local/bin/${BINARY_NAME}"

# Conditionally add the --gen-config flag for client bench binaries.
# Client bench binaries use gen_config.json, service binaries use config.json
if echo "${BINARY_NAME}" | grep -q "_client_bench$"; then
    output_path="/tmp/masa-load-gen"
    CMD="${CMD} --gen-config /usr/gen_config.json --output-path $output_path"
else
    CMD="${CMD} --config /usr/config.json"
fi

if [ -n "${EXTRA_FLAGS}" ]; then
    CMD="${CMD} ${EXTRA_FLAGS}"
fi

echo "Executing: ${CMD}"

# Execute the final command.
# 'exec' replaces the shell process with the command, which is good practice.
exec ${CMD}

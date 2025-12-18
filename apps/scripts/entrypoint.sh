#!/bin/bash

set -e

if [ -z "${LOG_LEVEL}" ]; then
  echo "Error: LOG_LEVEL environment variable is not set." >&2
  exit 1
fi

if [ -z "${BINARY_NAME}" ]; then
  echo "Error: BINARY_NAME environment variable is not set." >&2
  exit 1
fi

# Export the RUST_LOG variable for Rust's logging frameworks.
export RUST_LOG="${LOG_LEVEL}"

echo "Starting binary '${BINARY_NAME}' with log level '${LOG_LEVEL}'..."

# Construct the base command. See Dockerfile for where the binary resides.
CMD="/usr/local/bin/${BINARY_NAME}"

# Conditionally add the --gen-config flag for client bench binaries.
# Client bench binaries use gen_config.json, service binaries use config.json
if echo "${BINARY_NAME}" | grep -q "_client_bench$$"; then
    output_path="/tmp/masa-load-gen"
    CMD="${CMD} --gen-config /usr/gen_config.json --output-path $output_path"
else
    CMD="${CMD} --config /usr/config.json"
fi

echo "Executing: ${CMD}"

# Execute the final command.
# 'exec' replaces the shell process with the command, which is good practice.
exec ${CMD}


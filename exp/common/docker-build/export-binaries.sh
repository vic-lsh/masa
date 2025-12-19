#!/bin/bash

set -e

if [ -z "$BINARIES" ]; then
  echo "Error: BINARIES environment variable is not set." >&2
  exit 1
fi

if [ -z "$RELEASE_DIR" ]; then
  echo "Error: RELEASE_DIR environment variable is not set." >&2
  exit 1
fi

if [ -z "$OUTPUT_DIR" ]; then
  echo "Error: OUTPUT_DIR environment variable is not set." >&2
  exit 1
fi

mkdir -p "$OUTPUT_DIR"

for binary in $BINARIES; do
  if [ -f "$RELEASE_DIR/$binary" ]; then
    mv "$RELEASE_DIR/$binary" "$OUTPUT_DIR/$binary"
    chmod +x "$OUTPUT_DIR/$binary"
    echo "Moved binary: $binary"
  else
    echo "Warning: Binary '$binary' not found in release directory" >&2
    exit 1
  fi
done

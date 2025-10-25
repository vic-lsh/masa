#!/bin/sh
# entrypoint.sh

# This line runs the command specified by the BINARY_NAME variable.
# "$@" passes along any other arguments to the binary.
# exec "/app/$BINARY_NAME" "$@"

# Check if the BINARY_NAME variable is set and not empty
if [ -n "$BINARY_NAME" ]; then
  # If it is, execute that binary and ignore any default CMD
  exec "/usr/local/bin/$BINARY_NAME"
else
  # If it's not set, run the default command from the Dockerfile (the echo message)
  exec "$@"
fi
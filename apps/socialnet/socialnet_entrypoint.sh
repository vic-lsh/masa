#!/bin/sh
# entrypoint.sh

# This line runs the command specified by the BINARY_NAME variable.
# "$@" passes along any other arguments to the binary.
exec "/app/$BINARY_NAME" "$@"
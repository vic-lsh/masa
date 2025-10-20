#!/bin/sh

echo "Building the generic socialnet service image..."

# -t tags the image with a name we can reference later.
# -f points to the Dockerfile inside the socialnet directory.
# The final "." is crucial: it sets the build context to the current directory (the repo root).
docker build \
  -t socialnet-generic-svc:latest \
  -f ./apps/socialnet/Dockerfile \
  .

echo "Build complete. Image 'socialnet-generic-svc:latest' is ready."
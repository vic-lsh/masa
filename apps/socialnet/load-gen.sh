# Sending load to compose post (root of all services on a high level
PORT=16987

ghz --insecure \
  --rps 100 \
  --duration 15s \
  --proto ./proto/compose_post.proto \
  --call compose_post.ComposePostService.ComposePost \
  --data-file ./payloads.json \
  localhost:${PORT}


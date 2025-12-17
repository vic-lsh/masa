#!/bin/bash

# RPS_LEVELS=(200 250 300 350 400)
RPS_LEVELS=(350 1200 2400)
# Find the actual port of your API Gateway/Frontend service
HOST="localhost:8080" 

PROTO_FILE="./proto/compose_post.proto"
CALL_METHOD="compose_post.ComposePostService.ComposePost"
DURATION="15s"


# --- Test Execution ---

# 1. Print the CSV header
echo "RPS,p99_Latency_ms"

# Loop through each RPS level defined in the array
for rps in "${RPS_LEVELS[@]}"; do
  DATA_PAYLOAD='{"req_id": {{.RequestNumber}}, 
          "username": "tarangd",
          "user_id": {{.RequestNumber}},
          "text" : "Hello World",
          "media_ids": [1, 2],
          "media_types" : ["image", "text"],
          "post_type": "POST",
          "carrier": {}}'
  
  output=$(ghz --insecure \
    --proto $PROTO_FILE \
    --call $CALL_METHOD \
    --data-file "payloads.json" \
    --rps $rps \
    --duration $DURATION \
    $HOST 2>&1)

  # 2. Find the p99 line and extract the 4th field (the number)
  #    We also remove the "ms" or "s" unit for a clean number
  p99_value=$(echo "$output" | grep " 99 %" | awk '{print $4}')
  
  # 3. Print the CSV data row
  #    We check if p99_value is empty (in case of a total test failure)
  if [ -n "$p99_value" ]; then
    echo "$rps,$p99_value"
  else
    # Print an error row if the test failed to produce a p99
    echo "$rps,ERROR"
  fi
  # echo "$output"
done

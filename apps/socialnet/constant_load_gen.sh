#!/bin/bash

# --- Configuration ---
# RPS_LEVELS=(50 100 150 200 250)
RPS_LEVELS=(200 250)
HOST="localhost:8090"
PROTO_FILE="./proto/user_timeline.proto"
CALL_METHOD="user_timeline.UserTimelineService.WriteUserTimeline"
DATA_PAYLOAD='{"req_id": 1, "post_id": "1", "user_id": 23, "timestamp" : 251107210802, "carrier": {}}'
DURATION="30s"

# --- Test Execution ---

# 1. Print the CSV header
echo "RPS,p99_Latency_ms"

# Loop through each RPS level defined in the array
for rps in "${RPS_LEVELS[@]}"; do
  
  # Run ghz and capture the full text output into a variable
  # We redirect stderr (2) to stdout (1) so 'output' captures everything
  output=$(ghz --insecure \
    --proto $PROTO_FILE \
    --call $CALL_METHOD \
    --data "$DATA_PAYLOAD" \
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
  
done
#!/bin/bash

rps_values=(25 50 75 100 125 150 175 200 225 250)

for rps in "${rps_values[@]}"; do
  echo "Running benchmark for RPS: $rps..."

  cargo run --release --bin charleston_client_bench -- \
    --rps "$rps" \
    --secs 300 \
    --concurrency 32 \
    --addr1 http://[::1]:50051 \
    --addr2 http://[::1]:50052 \
    --output1 "snippets/poc/ex2k/r${rps}-client1.csv" \
    --output2 "snippets/poc/ex2k/r${rps}-client2.csv"

  sleep 3
done

echo "All benchmarks completed"

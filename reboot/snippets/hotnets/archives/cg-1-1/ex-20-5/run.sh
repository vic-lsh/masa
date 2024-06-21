#!/bin/bash

# rps_values=(50 75 100 125 150 175 200 225 250 275 300 325 350 375 400 425 450)
rps_values=(400 425 450 475 500)

for rps in "${rps_values[@]}"; do
  echo "Running benchmark for RPS: $rps..."

  cargo run --release --bin charleston_rivas -- \
    --mode fcfs \
    --rps "$rps" \
    --secs 60 \
    --concurrency 32 \
    --output snippets/hotnets/ex-20-5/r${rps}-fcfs.csv

  sleep 3

  cargo run --release --bin charleston_rivas -- \
    --mode masa \
    --rps "$rps" \
    --secs 60 \
    --concurrency 32 \
    --output snippets/hotnets/ex-20-5/r${rps}-masa.csv

  sleep 3
done

echo "All benchmarks completed"

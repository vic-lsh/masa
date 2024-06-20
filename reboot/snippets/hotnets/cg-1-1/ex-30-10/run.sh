#!/bin/bash

rps_values=(400 450 500)

for rps in "${rps_values[@]}"; do
  echo "Running benchmark for RPS: $rps..."

  cargo run --release --bin charleston_rivas -- \
    --mode fcfs \
    --rps "$rps" \
    --secs 60 \
    --concurrency 32 \
    --output snippets/hotnets/ex-30-10/r${rps}-fcfs.csv

  sleep 3

  cargo run --release --bin charleston_rivas -- \
    --mode masa \
    --rps "$rps" \
    --secs 60 \
    --concurrency 32 \
    --output snippets/hotnets/ex-30-10/r${rps}-masa.csv

  sleep 3
done

echo "All benchmarks completed"

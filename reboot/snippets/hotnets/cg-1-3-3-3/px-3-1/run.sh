#!/bin/bash

rps_values=(800 850 900 950 1000)

for rps in "${rps_values[@]}"; do
  echo "Running benchmark for RPS: $rps..."

  cargo run --release --bin charleston_lactose -- \
    --depth 3 \
    --mode fcfs \
    --rps "$rps" \
    --secs 60 \
    --concurrency 128 \
    --output snippets/hotnets/cg-1-3-3-3/px-3-1/r${rps}-fcfs.csv

  sleep 3

  cargo run --release --bin charleston_lactose -- \
    --depth 3 \
    --mode masa \
    --rps "$rps" \
    --secs 60 \
    --concurrency 128 \
    --output snippets/hotnets/cg-1-3-3-3/px-3-1/r${rps}-masa.csv

  sleep 3
done

echo "All benchmarks completed"

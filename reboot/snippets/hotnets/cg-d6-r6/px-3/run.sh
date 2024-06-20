#!/bin/bash

rps_values=(1600 1700 1800 1900 2000)

for rps in "${rps_values[@]}"; do
  echo "Running benchmark for RPS: $rps..."

  cargo run --release --bin charleston_lactose -- \
    --depth 6 \
    --mode fcfs \
    --rps "$rps" \
    --secs 60 \
    --concurrency 256 \
    --output snippets/hotnets/cg-d6-r6/px-3/r${rps}-fcfs.csv

  sleep 3

  cargo run --release --bin charleston_lactose -- \
    --depth 6 \
    --mode masa \
    --rps "$rps" \
    --secs 60 \
    --concurrency 256 \
    --output snippets/hotnets/cg-d6-r6/px-3/r${rps}-masa.csv

  sleep 3
done

echo "All benchmarks completed"

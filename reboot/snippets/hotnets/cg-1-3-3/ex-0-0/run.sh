#!/bin/bash

rps_values=(1300 1350 1400 1450 1500)

for rps in "${rps_values[@]}"; do
  echo "Running benchmark for RPS: $rps..."

  cargo run --release --bin charleston_lactose -- \
    --mode fcfs \
    --rps "$rps" \
    --secs 60 \
    --concurrency 32 \
    --output snippets/hotnets/cg-1-3-3/ex-0-0/r${rps}-fcfs.csv

  sleep 3

  cargo run --release --bin charleston_lactose -- \
    --mode masa \
    --rps "$rps" \
    --secs 60 \
    --concurrency 32 \
    --output snippets/hotnets/cg-1-3-3/ex-0-0/r${rps}-masa.csv

  sleep 3
done

echo "All benchmarks completed"

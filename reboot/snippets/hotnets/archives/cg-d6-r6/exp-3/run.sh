#!/bin/bash

rps_values=(1500 1550 1600 1650 1700 1750 1800 1850 1900 1950 2000)
# rps_values=(1500 1600 1700 1800 1900 2000)

for rps in "${rps_values[@]}"; do
  echo "Running benchmark for RPS: $rps..."

  cargo run --release --bin charleston_lactose -- \
    --depth 6 \
    --exec-mus 3000 3000 3000 3000 3000 3000 \
    --replicas 6 \
    --mode fcfs \
    --rps "$rps" \
    --secs 300 \
    --concurrency 256 \
    --output snippets/hotnets/cg-d6-r6/exp-3/r${rps}-fcfs.csv

  sleep 3

  cargo run --release --bin charleston_lactose -- \
    --depth 6 \
    --exec-mus 3000 3000 3000 3000 3000 3000 \
    --replicas 6 \
    --mode masa \
    --rps "$rps" \
    --secs 300 \
    --concurrency 256 \
    --output snippets/hotnets/cg-d6-r6/exp-3/r${rps}-masa.csv

  sleep 3
done

echo "All benchmarks completed"

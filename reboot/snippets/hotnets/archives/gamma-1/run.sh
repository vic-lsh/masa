#!/bin/bash

# rps_values=(100 200 300 400 500 600 700 800 900 1000 1100 1200 1300 1400 1500 1600 1700 1800 1900 2000)
rps_values=(1100 1200 1300 1400 1500 1600 1700 1800 1900 2000)
# rps_values=(1500 1550 1600 1650 1700 1750 1800 1850 1900 1950 2000)
# rps_values=(1500 1600 1700 1800 1900 2000)

for rps in "${rps_values[@]}"; do
  echo "Running benchmark for RPS: $rps..."

  cargo run --release --bin charleston_lactose -- \
    --depth 6 \
    --exec-ks 1 1 1 1 1 1 \
    --exec-mus 3000 3000 3000 3000 3000 3000 \
    --replicas 6 \
    --mode fcfs \
    --rps "$rps" \
    --secs 120 \
    --concurrency 256 \
    --output snippets/hotnets/gamma-1/r${rps}-fcfs.csv

  sleep 3

  cargo run --release --bin charleston_lactose -- \
    --depth 6 \
    --exec-ks 1 1 1 1 1 1 \
    --exec-mus 3000 3000 3000 3000 3000 3000 \
    --replicas 6 \
    --mode masa \
    --rps "$rps" \
    --secs 120 \
    --concurrency 256 \
    --output snippets/hotnets/gamma-1/r${rps}-masa.csv

  sleep 3
done

echo "All benchmarks completed"

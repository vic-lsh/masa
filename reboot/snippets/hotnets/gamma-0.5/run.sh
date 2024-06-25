#!/bin/bash

rps_values=(100 200 300 400 500 600 700 800 900 1000 1100 1200 1300 1400 1500 1600 1700 1800 1900 2000)

# rps_values=(200 400 600 800 1000 1200 1400 1600 1800 2000)

for rps in "${rps_values[@]}"; do
  echo "Running benchmark for RPS: $rps..."

  cargo run --release --bin charleston_lactose -- \
    --depth 10 \
    --exec-ks 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 \
    --first-exec-mus 2000 2000 2000 2000 2000 2000 2000 2000 2000 2000 \
    --second-exec-mus 2000 2000 2000 2000 2000 2000 2000 2000 2000 2000 \
    --replicas 4 \
    --mode fcfs \
    --rps "$rps" \
    --secs 60 \
    --concurrency 256 \
    --output snippets/hotnets/gamma-0.5/r${rps}-fcfs.csv

  sleep 3

  cargo run --release --bin charleston_lactose -- \
    --depth 10 \
    --exec-ks 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 0.5 \
    --first-exec-mus 2000 2000 2000 2000 2000 2000 2000 2000 2000 2000 \
    --second-exec-mus 2000 2000 2000 2000 2000 2000 2000 2000 2000 2000 \
    --replicas 4 \
    --mode masa \
    --rps "$rps" \
    --secs 60 \
    --concurrency 256 \
    --output snippets/hotnets/gamma-0.5/r${rps}-masa.csv

  sleep 3
done

echo "All benchmarks completed"

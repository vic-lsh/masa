#!/bin/bash

# rps_values=(100 200 300 400 500 600 700 800 900 1000 1100 1200 1300 1400 1500 1600 1700 1800 1900 2000)
# rps_values=(1100 1200 1300 1400 1500 1600 1700 1800 1900 2000)
# rps_values=(1500 1550 1600 1650 1700 1750 1800 1850 1900 1950 2000)
# rps_values=(1500 1600 1700 1800 1900 2000)

# rps_values=(1100 1150 1200 1250 1300 1350 1400 1450 1500 1550 1600 1650 1700 1750 1800 1850 1900 1950 2000)

rps_values=(700 750 800 850 900 950 1000 1050)

for rps in "${rps_values[@]}"; do
  echo "Running benchmark for RPS: $rps..."

  cargo run --release --bin charleston_lactose -- \
    --depth 6 \
    --exec-ks 0.5 0.5 0.5 0.5 0.5 0.5 \
    --first-exec-mus 1500 2000 2500 3500 4000 4500 \
    --second-exec-mus 4500 4000 3500 2500 2000 1500 \
    --replicas 6 \
    --mode fcfs \
    --rps "$rps" \
    --secs 600 \
    --concurrency 256 \
    --output snippets/hotnets/gamma-0.5/r${rps}-fcfs.csv

  sleep 3

  cargo run --release --bin charleston_lactose -- \
    --depth 6 \
    --exec-ks 0.5 0.5 0.5 0.5 0.5 0.5 \
    --first-exec-mus 1500 2000 2500 3500 4000 4500 \
    --second-exec-mus 4500 4000 3500 2500 2000 1500 \
    --replicas 6 \
    --mode masa \
    --rps "$rps" \
    --secs 600 \
    --concurrency 256 \
    --output snippets/hotnets/gamma-0.5/r${rps}-masa.csv

  sleep 3
done

echo "All benchmarks completed"

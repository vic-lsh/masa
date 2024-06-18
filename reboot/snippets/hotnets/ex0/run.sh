#!/bin/bash

rps_values=(50 75 100 125 150 175 200 225 250 275 300 325 350 375 400 425 450)

for rps in "${rps_values[@]}"; do
  echo "Running benchmark for RPS: $rps..."

  python src/sim/test.py \
    --mode fcfs \
    --rps "$rps" \
    --secs 300 \
    --output snippets/hotnets/ex0/r${rps}-fcfs.csv

  python src/sim/test.py \
    --mode masa \
    --rps "$rps" \
    --secs 300 \
    --output snippets/hotnets/ex0/r${rps}-masa.csv

  sleep 3
done

echo "All benchmarks completed"

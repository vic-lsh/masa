#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "Usage: $0 I J"
  exit 1
fi

I=$1
J=$2

if (( I > J )); then
  echo "Error: I must be <= J"
  exit 1
fi

cd $(dirname "$0")
cd ../traces/alibaba/cluster-trace-microservices-v2022/data/CallGraph/

for ((i=I; i<J; i++)); do
  csv_file="CallGraph_${i}.csv"
  filename="CallGraph_${i}.tar.gz"

  if [[ -f "$csv_file" ]]; then
    echo "Skipping $csv_file (already exists)"
    continue
  fi

  if [[ -f "$filename" ]]; then
    echo "Decompressing $filename..."
    tar -xzf "$filename"
  else
    echo "Skipping $filename (not found)"
  fi
done
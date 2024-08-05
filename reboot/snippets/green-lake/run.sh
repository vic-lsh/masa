#!/bin/bash

path="snippets/green-lake"
rps_values=(100 200)
mode="masa"
# mode="fifo"

# RUST_LOG=warn cargo run --release --bin charleston_server > tmp_server.txt 2>&1

for rps in "${rps_values[@]}"; do
	echo "Running benchmark for RPS: $rps..."

	cargo run --release --bin charleston_client_bench -- \
		--slo 10000 \
		--rps ${rps} \
		--secs 60 \
		--concurrency 128 \
		--output snippets/green-lake/r${rps}-${mode}.csv

	sleep 3
done

echo "All benchmarks completed"

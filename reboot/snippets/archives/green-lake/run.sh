#!/bin/bash

path="snippets/green-lake"

# rps_values=(300 325 350 375 400 425 450 475 500 525 550 575 600 625 650 675 700 725 750 775 800 825 850 875 900)
rps_values=(925 950 975 1000)

modes=("masa" "fifo")

for mode in "${modes[@]}"; do
	echo "Compiling mode: $mode..."

	cargo build --features $mode --release >/dev/null 2>&1

	RUST_LOG=warn cargo run --features $mode --release --bin charleston_server -- --n-threads 2 >$path/tmp_server.txt 2>&1 &

	pid=$!

	echo "Running server in mode: $mode..."

	for rps in "${rps_values[@]}"; do
		sleep 3

		echo "Running benchmark for RPS: $rps..."

		cargo run --release --bin charleston_client_bench -- \
			--slo 10000 \
			--rps $rps \
			--secs 300 \
			--concurrency 512 \
			--output $path/r$rps-$mode.csv \
			>/dev/null 2>&1
	done

	kill $pid

	echo "Completed mode: $mode"
done

#!/bin/bash

path="snippets/flat"

# rps_values=(300 325 350 375 400 425 450 475 500 525 550 575 600 625 650 675 700 725 750 775 800 825 850 875 900)
# rps_values=(200 1600)
rps_values=(1)

modes=("masa" "fifo")

server_pid=

ctrl_c_handler() {
	echo ""
	echo "Caught Ctrl-C, exiting..."
	cleanup
	exit 1
}

cleanup() {
	kill -9 ${server_pid}
}

trap ctrl_c_handler SIGINT

for mode in "${modes[@]}"; do
	echo "Compiling mode: $mode..."

	cargo build --features $mode --release >/dev/null 2>&1

	RUST_LOG=info cargo run --features $mode --release --bin bridgeway_server -- --n-threads 1 >$path/tmp_${mode}.log 2>&1 &

	server_pid=$!

	echo "Running server in mode: $mode..."

	for rps in "${rps_values[@]}"; do
		sleep 3

		echo "Running benchmark for RPS: $rps..."

		cargo run --release --bin bridgeway_client_bench -- \
			--slo 10000 \
			--rps $rps \
			--secs 10 \
			--concurrency 512 \
			--output $path/r${rps}_${mode}.csv \
			>/dev/null 2>&1
	done

	kill $server_pid

	echo "Completed mode: $mode"
done

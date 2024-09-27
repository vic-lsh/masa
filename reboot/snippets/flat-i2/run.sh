#!/bin/bash

path="snippets/flat-i2"

rps_values=(800)

# modes=("queue_edf" "queue_fifo_two" "queue_fifo")
modes=("queue_edf est_online" "queue_fifo est_online")

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

	# cargo build --features "$mode" >/dev/null 2>&1
	cargo build --features "$mode" \
		--release >/dev/null 2>&1

	# RUST_BACKTRACE=1 RUST_LOG=info cargo run ...
	RUST_LOG=warn \
		cargo run --features "$mode" \
		--release --bin bridgeway_server -- \
		--n-hops 2 \
		--n-threads 1 \
		>$path/tmp_server_test.log 2>&1 &

	server_pid=$!

	echo "Running server in mode: $mode..."

	for rps in "${rps_values[@]}"; do
		sleep 1

		echo "Running benchmark for RPS: $rps..."

		cargo run --release --bin bridgeway_client_bench -- \
			--slo 10000 \
			--rps $rps \
			--secs 10 \
			--concurrency 512 \
			--output $path/r${rps}_test.csv \
			--graph-id I2 \
			--addr http://[::1]:50052 \
			>$path/tmp_client_test.log 2>&1
	done

	kill $server_pid

	echo "Completed mode: $mode"
done

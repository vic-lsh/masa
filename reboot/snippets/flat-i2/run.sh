#!/bin/bash

path="snippets/flat-i2"

rps_values=(900)

# modes=("prio_local" "fifo_infra" "fifo")
# modes=("prio_local" "fifo_infra")
modes=("prio_local" "prio_global")

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

	# cargo build --features $mode >/dev/null 2>&1
	cargo build \
		--release \
		--features "$mode" \
		>/dev/null 2>&1

	# RUST_BACKTRACE=1 RUST_LOG=info \
	RUST_LOG=warn \
		cargo run \
		--release \
		--features "$mode" \
		--bin bridgeway_server -- \
		--graph-ids I2_1 \
		--slos 10000 \
		--n-hops 2 \
		--n-threads 1 \
		>$path/tmp_server_${mode}.log 2>&1 &

	server_pid=$!

	echo "Running server in mode: $mode..."

	for rps in "${rps_values[@]}"; do
		sleep 1

		echo "Running benchmark for RPS: $rps..."

		cargo run \
			--release \
			--features "$mode" \
			--bin bridgeway_client_bench -- \
			--graph-ids I2_1 \
			--slos 10000 \
			--rps $rps \
			--secs 30 \
			--concurrency 512 \
			--output $path/r${rps}_${mode}.csv \
			--addr http://[::1]:50052 \
			>$path/tmp_client_${mode}.log 2>&1
	done

	kill $server_pid

	echo "Completed mode: $mode"
done

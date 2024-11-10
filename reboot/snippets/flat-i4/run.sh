#!/bin/bash

path="snippets/flat-i4"
modes=("prio_local" "prio_global" "fifo_infra")
# modes=("prio_local")
rps_values=(450 500 550)
graph_ids="I4_1 I4_2"
slos="50000 200000"
rps_ranges="4 5"
n_hops=4
n_threads="4 4 4 4"
secs=120

ctrl_c_handler() {
	echo ""
	echo "Caught Ctrl-C, exiting..."
	cleanup
	exit 1
}

server_pid=

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
		--graph-ids $graph_ids \
		--slos $slos \
		--n-hops $n_hops \
		--n-threads $n_threads \
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
			--graph-ids $graph_ids \
			--slos $slos \
			--rps-ranges $rps_ranges \
			--rps $rps \
			--secs $secs \
			--concurrency 512 \
			--output $path/r${rps}_${mode}.csv \
			--addr http://[::1]:50054 \
			>$path/tmp_client_${mode}.log 2>&1
	done

	kill $server_pid

	echo "Completed mode: $mode"
done

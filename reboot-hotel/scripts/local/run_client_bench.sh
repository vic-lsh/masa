#!/bin/bash

current_dir=$(pwd)
if [[ "$current_dir" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi

RUST_LOG=warn \
	cargo run --release \
	--bin hotel_client_bench \
	--features prio_global \
	-- \
	--rps 100 \
	--secs 10 \
	--concurrency 128 \
	--output tmp_hotel_client_bench.csv \
	>tmp_hotel_client_bench.log 2>&1

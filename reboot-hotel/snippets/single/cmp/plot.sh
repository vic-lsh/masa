#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi
snippets=snippets/single
data=tmp_1111

declare -A name_to_modes=(
	["search_fifo_e2e"]="fifo_search e2e_search"
	["search_fifo_e2e_er"]="fifo_search e2e_er_search"
	["search_e2e_e2e_er"]="e2e_search e2e_er_search"
	["reservation_fifo_e2e"]="fifo_reservation e2e_reservation"
	["reservation_fifo_e2e_er"]="fifo_reservation e2e_er_reservation"
	["reservation_e2e_e2e_er"]="e2e_reservation e2e_er_reservation"
)

for name in "${!name_to_modes[@]}"; do
	modes="${name_to_modes[$name]}"

	echo "Plotting goodput for $name..."
	python3 $pwd/scripts/plots/plot_goodput_cmp.py \
		--path $pwd/$snippets/cmp \
		--snippets $pwd/$snippets \
		--data $data \
		--modes $modes
done

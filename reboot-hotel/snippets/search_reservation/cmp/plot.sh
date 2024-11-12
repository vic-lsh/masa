#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi
snippets=snippets/search_reservation
data=tmp_1112

declare -A name_to_modes=(
	["fifo_e2e"]="fifo e2e"
	["fifo_e2e_er"]="fifo e2e_er"
	["e2e_e2e_er"]="e2e e2e_er"
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

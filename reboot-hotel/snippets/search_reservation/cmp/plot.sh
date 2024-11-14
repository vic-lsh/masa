#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi

folder=snippets/search_reservation
snippet=cmp
data=tmp_1113

modes_list=(
	# "fifo e2e"
	# "fifo e2e_er"
	# "fifo local"
	# "fifo local_er"
	# "e2e e2e_er"
	# "e2e local"
	# "local local_er"
	# "e2e_er local_er"
	"local_p50 local_p75"
	"local_p75 local_p90"
	"local_p90 local_p95"
	"local_er_p50 local_er_p75"
	"local_er_p75 local_er_p90"
	"local_er_p90 local_er_p95"
)

for modes in "${modes_list[@]}"; do
	echo "Plotting $modes..."
	python3 $pwd/scripts/plots/plot_goodput_cmp.py \
		--modes $modes \
		--path $pwd/$folder/$snippet \
		--snippets $pwd/$folder \
		--data $data
done

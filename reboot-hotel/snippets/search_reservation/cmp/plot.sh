#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi
snippets=snippets/search_reservation
data=tmp_1112

modes_list=(
	"fifo e2e"
	"fifo e2e_er"
	"fifo local"
	"fifo local_er"
	"e2e e2e_er"
	"e2e local"
	"local local_er"
	"e2e_er local_er"
)

for modes in "${modes_list[@]}"; do
	echo "Plotting $modes..."
	python3 $pwd/scripts/plots/plot_goodput_cmp.py \
		--modes $modes \
		--path $pwd/$snippets/cmp \
		--snippets $pwd/$snippets \
		--data $data
done

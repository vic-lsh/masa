#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi

folder=snippets/single
snippet=cmp
data=tmp_1112

modes_list=(
	# search
	# "fifo_search e2e_search local_search"
	# "fifo_search e2e_er_search local_er_search"
	"fifo_search e2e_search e2e_er_search local_search local_er_search"
	# reservation
	# "fifo_reservation e2e_reservation local_reservation"
	# "fifo_reservation e2e_er_reservation local_er_reservation"
	"fifo_reservation e2e_reservation e2e_er_reservation local_reservation local_er_reservation"
)

for modes in "${modes_list[@]}"; do
	echo "Plotting $modes..."

	python3 $pwd/scripts/plots/plot_goodput_cmp.py \
		--modes $modes \
		--path $pwd/$folder/$snippet \
		--snippets $pwd/$folder \
		--data $data

	python3 $pwd/scripts/plots/plot_tail_cmp.py \
		--modes $modes \
		--path $pwd/$folder/$snippet \
		--snippets $pwd/$folder \
		--data $data
done

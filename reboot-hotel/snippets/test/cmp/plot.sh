#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi

folder=snippets/test
snippet=cmp
data=.

modes_list=(
	"fifo local"
)

for modes in "${modes_list[@]}"; do
	echo "Plotting $modes..."

	python3 $pwd/scripts/plots/plot_goodput_cmp.py \
		--modes $modes \
		--gen-config $pwd/$folder/gen_config.json \
		--snippets $pwd/$folder \
		--path $pwd/$folder/$snippet \
		--data $data

	python3 $pwd/scripts/plots/plot_tail_cmp.py \
		--modes $modes \
		--gen-config $pwd/$folder/gen_config.json \
		--snippets $pwd/$folder \
		--path $pwd/$folder/$snippet \
		--data $data
done

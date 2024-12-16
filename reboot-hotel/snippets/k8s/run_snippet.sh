#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi

folder=snippets/k8s
data=tmp_1215_barbell

tags=(
	fifo
)

for tag in "${tags[@]}"; do
	echo "Running $tag..."
	$pwd/scripts/local/run_all_k8s.sh \
		--tag $tag \
		--folder $folder \
		--rust-log warn \
		--cargo-features $tag \
		--gen-config $folder/gen_config.json \
		--hotel-config $folder/hotel_config.json \
		--output-path $folder/$tag/$data
done

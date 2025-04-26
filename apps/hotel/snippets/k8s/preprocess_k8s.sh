#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */apps/hotel ]]; then
	echo "Error: plese run in the apps/hotel directory" >&2
	exit 1
fi

folder=snippets/k8s

python3 $pwd/scripts/k8s-template/preprocess_yaml.py \
	--config $folder/k8s_config.json \
	--input-path $pwd/scripts/k8s-template/yaml \
	--output-path $folder/yaml

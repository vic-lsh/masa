#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */apps/* ]]; then
    echo "Error: please run in an application directory" >&2
    exit 1
fi
experiment=$1

python3 ../scripts/all.py --config-dir data/in/$1 --data-dir data/out/$1 --output-dir data/plots/$1

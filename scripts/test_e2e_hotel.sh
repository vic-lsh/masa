#!/bin/bash

set -e

cd apps/hotel

# setup experiment config

exp_name=ci
ci_config_path=./data/in/$exp_name
exp_out_path=./data/out/$exp_name

rm -rf $exp_out_path
rm -rf $ci_config_path
cp -r ./data/in/template $ci_config_path

gen_config=$ci_config_path/gen_config.json
policy_config=$ci_config_path/policies

# patch experiment settings

jq '.Rps = [1000]' $gen_config > tmp.json && mv tmp.json $gen_config
echo "fifo prio_global" > $policy_config

# run experiment

temp_file=$(mktemp)
/usr/bin/time -o "$temp_file" ./scripts/run-experiment.sh $exp_name
echo "Experiment runtime:"
cat "$temp_file"
rm "$temp_file"

assert_file_exists() {
    if [ -f "$1" ]; then
        echo "Assertion passed: File '$1' exists"
    else
        echo "Error: File '$1' not found." >&2; exit 1;
    fi
}

# assert result files exist
assert_file_exists "data/out/$exp_name/done"
assert_file_exists "data/out/$exp_name/0/fifo/r1000_Reservation.csv"
assert_file_exists "data/out/$exp_name/0/fifo/r1000_Search.csv"
assert_file_exists "data/out/$exp_name/0/prio_global/r1000_Reservation.csv"
assert_file_exists "data/out/$exp_name/0/prio_global/r1000_Search.csv"

#!/bin/bash

set -e

pwd=$(pwd)
if [[ "$pwd" != */apps/* ]]; then
    echo "Error: please run in an application directory" >&2
    exit 1
fi
app=$(basename $pwd)
experiment=$1
plot="false"
prefix=""
skip_build=""
shift 1

while [[ $# -gt 0 ]]; do
    case $1 in
    --plot)
        plot="true"
        shift 1
        ;;
    --prefix)
        prefix="--prefix $2"
        shift 2
        ;;
    --skip-build)
        skip_build="--skip-build"
        shift 1
        ;;
    *)
        echo "Unknown argument: $1"
        exit 1
        ;;
    esac
done


in_dir="data/in/$experiment"
out_dir="data/out/$experiment"
plot_dir="data/plots/$experiment"

if [[ ! -d "$in_dir" ]]; then
    echo "expected configuration for experiment at '$in_dir'"
    exit 1
fi

mkdir -p $out_dir
curr_ts=$(date +%s)
backup=/tmp/masa-save-$curr_ts
mkdir -p $backup
# save previous config
if [[ -f ./scripts/gen_config.json ]]; then
    cp ./scripts/gen_config.json $backup
fi
if [[ -f ./scripts/local/config.docker.json ]]; then
    cp ./scripts/local/config.docker.json $backup
fi
# load gen config and app config (if present) from $experiment
cp $in_dir/gen_config.json ./scripts/
if [[ -f $in_dir/config.docker.json ]]; then
    cp $in_dir/config.docker.json ./scripts/local/
fi
# save old output just in case
cp -r $out_dir $backup
# clear $out_dir and $plot_dir
rm -rf $out_dir/*
rm -rf $plot_dir

policies=$(cat $in_dir/policies | tr -d '\n')
repeat=$(cat ./scripts/gen_config.json | jq -r ".Repeats")

for i in $(seq 0 $((repeat - 1))); 
do
    echo "---------- iteration $i ----------"
    for policy in $policies;
    do
        echo "***** policy = $policy *****"
        touch ./scripts/local/.env
        if [[ -f ./scripts/get-env.sh ]]; then
            ./scripts/get-env.sh > ./scripts/local/.env
        fi
        ./scripts/docker-run.sh --features $policy $prefix $skip_build

        ./scripts/loadgen-run.sh --output $out_dir/$i/$policy --save-logs &
        loadgen_pid=$!

        # set up log file pipes so that container logs stream in as they run
        ./scripts/docker-save-logs.sh --output $out_dir/$i/$policy --follow

        wait $loadgen_pid

        ./scripts/docker-stop.sh $prefix
        rm ./scripts/local/.env
    done
done
touch $out_dir/done

if [[ "$plot" = "true" ]]; then
    ../scripts/plotting/plot-experiment.sh "$experiment"
fi

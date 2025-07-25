#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */apps/* ]]; then
    echo "Error: please run in an application directory" >&2
    exit 1
fi
app=$(basename $pwd)
experiment=$1
plot="false"
shift 1

while [[ $# -gt 0 ]]; do
    case $1 in
    --plot)
        plot="true"
        shift 1
        ;;
    *)
        echo "Unknown argument: $1"
        exit 1
        ;;
    esac
done

in_dir=data/in/$experiment
out_dir=data/out/$experiment
plot_dir=data/plots/$experiment
mkdir -p $out_dir
backup=/tmp/masa-save
mkdir -p $backup
# save previous config
if [[ -f ./scripts/gen_config.json ]]; then
    cp ./scripts/gen_config.json /tmp/masa-save/
fi
if [[ -f ./scripts/local/config.docker.json ]]; then
    cp ./scripts/local/config.docker.json /tmp/masa-save/
fi
# load gen config and app config (if present) from $experiment
cp $in_dir/gen_config.json ./scripts/
if [[ -f $in_dir/config.docker.json ]]; then
    cp $in_dir/config.docker.json ./scripts/local/
fi
# save old output just in case
cp -r $out_dir /tmp/masa-save/
# clear $out_dir and $plot_dir
rm -rf $out_dir/*
rm -rf $plot_dir

policies=$(cat $in_dir/policies | tr -d '\n')
repeat=$(cat $in_dir/gen_config.json | jq -r ".Repeats")

for i in $(seq 0 $((repeat - 1))); 
do
    for policy in $policies;
    do
            echo "policy = $policy"
            ./scripts/docker-run.sh --features $policy
            ./scripts/loadgen-run.sh --output $out_dir/$i/$policy --save-logs
            ./scripts/docker-save-logs.sh $out_dir/$i/$policy
            ./scripts/docker-stop.sh
    done
done
touch $out_dir/done

if [[ "$plot" = "true" ]]; then
    ../scripts/plotting/plot-experiment.sh "$experiment"
fi

#!/usr/bin/bash

dir=~/masa/data/$1
mkdir -p $dir
# save previous config
cp ./scripts/gen_config.json /tmp/gen_config.json
# load config from $1
cp $dir/gen_config.json ./scripts/gen_config.json

mkdir -p /tmp/save/
# save old contents of $1 just in case
cp -r $dir /tmp/save/
# clean $dir
shopt -s extglob
rm -rf $dir/!(gen_config.json|policies)

policies=$(cat $dir/policies | tr -d '\n')

for policy in $policies;
do
	echo "policy = $policy"
	./scripts/docker-run.sh --features $policy
	./scripts/loadgen-run.sh $dir/$policy
	./scripts/docker-save-logs.sh $dir/$policy
	./scripts/docker-stop.sh
done
touch $dir/done

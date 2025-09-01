#!/bin/bash

cd apps/hotel

# setup experiment config

exp_name=ci
ci_config_path=./data/in/$exp_name

rm -rf $ci_config_path
cp -r ./data/in/template $ci_config_path

./scripts/run-experiment.sh $exp_name

#!/bin/bash

for exp in $1;
do
	echo "running experiment $exp"
	./scripts/run-experiment.sh $exp
done

for exp in $1;
do
	echo "running experiment $exp"
	./scripts/run_experiment.sh $exp
done

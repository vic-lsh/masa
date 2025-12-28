# How to run socialnet

1. Run `apps/socialnet/build_socialnet.sh`
2. In `apps/socialnet`, run `python3 run_socialnet.py`
3. When you are done, run `./down_socialnet.sh` to take the containers down

# Scaling the services
## Database services (e.g. those to do with mongo, redis, memached)

Can vertically scale these services by simply increasing the number of cpus needed in the docker-compose.yaml file.

## Main services
Can horizontally scale the main services by increasing number of replicas in `replicas.json`. 


# Social Net service dependencies

Note that each service has disaggregated database services.

![service dependencies for socialnet app](apps/socialnet/callgraph.png)

# Getting latency graph

Run `python ./plot_latency.py` to get graphs for tail latencies vs. RPS. You can customize different RPS values by altering the list in the variable `RPS_LEVELS` at the top of the file.


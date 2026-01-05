# How to run socialnet

1. Run `apps/socialnet/build_socialnet.sh`
2. In `apps/socialnet`, run `docker compose up -d`

The compose file uses the `SOCIALNET_IMAGE_TAG` variable (defaults to `latest`).
If you build images with a different tag (e.g. via the experiment runner), set
`SOCIALNET_IMAGE_TAG` before running `docker compose up`.

# Scaling the services
## Database services (e.g. those to do with mongo, redis, memached)

Can vertically scale these services by simply increasing the number of cpus needed in the docker-compose.yaml file.

## Main services
Can horizontally scale these by increasing number of replicas. Make sure to do this in the service itself, but also for services calling that service. E.g. since compose post service depends on user timeline service, if we increase replicas of user timeline service, we must also increase the number of replicas in compose post service. E.g.:

First increase here:
```
user-timeline-service:
    image: user_timeline_server:${SOCIALNET_IMAGE_TAG:-latest}
    scale: 4
    restart: always
    ports:
      - "8090-8094:8080"
    networks:
      - socialnet-network
    environment:
      - BINARY_NAME=user_timeline_server
      - USER_TIMELINE_MONGODB_URI=mongodb://user_timeline_mongo:27017
      - USER_TIMELINE_REDIS_URL=redis://user_timeline_redis:6379
      - POST_STORAGE_IP=socialnet-post-storage-service
      - POST_STORAGE_PORT=8080
      - POST_STORAGE_REPLICAS=1
    depends_on:
      - user-timeline-mongo
      - user-timeline-redis
      - post-storage-service
    deploy:
      resources:
        limits:
          cpus: "4"
```



Then also increase here:

```
compose-post-service:
   ....
    environment:
      ...
      - USER_TIMELINE_IP=socialnet-user-timeline-service
      - USER_TIMELINE_PORT=8080
      - USER_TIMELINE_REPLICAS=4
    ...
```



# Social Net service dependencies

Note that each service has disaggregated database services.

![service dependencies for socialnet app](apps/socialnet/callgraph.png)

# Getting latency graph

Run `python ./plot_latency.py` to get graphs for tail latencies vs. RPS. You can customize different RPS values by altering the list in the variable `RPS_LEVELS` at the top of the file.

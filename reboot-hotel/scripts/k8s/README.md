# k8s setup

## Configure k8s environment

To run locally, install [minikube](https://minikube.sigs.k8s.io/docs/).

For the best performance, using minikube with kvm2 is recommended. To start a
local cluster:

```
minikube start --driver=kvm2 --cpus=32 --memory=64g --disk-size=64g
```

## Build and push docker images

The first step is to build all docker images. To do so, run

```
./scripts/docker-build-all.sh --features <feature-flags>
```

from the `reboot-hotel` directory.

This bash script also pushes the built images to docker hub. Edit the script
to customize to your docker hub username.

## Run hotel microservices

Run the following:

```
kubectl apply -f <this-k8s-folder>
```

Then run `kubectl get pods -A` and wait until all services become ready. You
may see some services err at the beginning. This is expected -- some services
won't start successfully until the service(s) they depend on becomes ready.

If a pod stays in error mode for a long time, you may inspect its log messages
with `kubectl logs <pod-name>`.

## Use the hotel frontend service

To expose the frontend service to localhost, run this in a new terminal:

```
kubectl port-forward service/frontend-service 8660:8660
```

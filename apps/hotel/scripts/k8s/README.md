# k8s setup

**NOTE: These scripts are for manual deployment only. The `exp_runner` automation tool does not yet support running the hotel benchmark on Kubernetes.**

## Configure k8s environment

To run locally, install [minikube](https://minikube.sigs.k8s.io/docs/).

For the best performance, using minikube with kvm2 is recommended. To start a
local cluster:

```
minikube start --driver=kvm2 --cpus=52 --memory=64g --disk-size=64g
```

## Build and push docker images

The first step is to build all docker images. To do so, run

```
./scripts/docker/build_all.sh --features <feature-flags>
```

from the `apps/hotel` directory.

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

If a pod is persistently in "Pending" state, it could be because `minikube`
doesn't have enough CPUs.

## Use the hotel frontend service

To expose the frontend service to localhost, run this in a new terminal:

```
kubectl port-forward service/frontend-service 8660:8660
```

## Monitor load

To monitor pod CPU and memory usage, you need to install the metrics server.

```
kubectl apply -f https://github.com/kubernetes-sigs/metrics-server/releases/latest/download/components.yaml
```

If the metrics server is not ready, it could be because of TLS certificate issues.
You can work around this like so:

```
# Open config file
kubectl edit deployment metrics-server -n kube-system

# Add under containers/args:
- --kubelet-insecure-tls
- --kubelet-preferred-address-types=InternalIP
```

After the metrics server is up, you can continuously monitor pod CPU and memory usage with:

```
watch -n 1 "kubectl top pods"
```

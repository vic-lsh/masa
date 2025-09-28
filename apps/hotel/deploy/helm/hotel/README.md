# Hotel Helm Chart

This chart deploys the hotel demo application and its backing MongoDB + Memcached instances into a local Kubernetes cluster (for example [kind](https://kind.sigs.k8s.io/)). It mirrors the topology that previously lived in `scripts/local/containers+svcs.yaml` so that you can exercise the services end-to-end without Docker Compose.

## Prerequisites

- Docker and [kind](https://kind.sigs.k8s.io/docs/user/quick-start/) installed.
- `kubectl` and `helm` available in your `$PATH`.
- Built container images for every hotel service (`hotel_frontend`, `hotel_geo`, `hotel_profile`, `hotel_rate`, `hotel_reservation`, `hotel_search`, `hotel_user`) tagged the same way you intend to reference them from the chart. The defaults expect the `latest` tag and no registry prefix.

## Quick start

```bash
# 1. Create (or reuse) a kind cluster with enough resources and port mappings
cat <<'KIND' > kind-hotel.yaml
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
nodes:
  - role: control-plane
    extraPortMappings:
      - containerPort: 30086
        hostPort: 8660
        protocol: TCP
KIND
kind create cluster --name hotel --config kind-hotel.yaml

# 2. Build the hotel images and load them into kind
pushd apps/hotel
./scripts/docker-build.sh --features "frontend,geo,profile,rate,reservation,search,user"
popd
for image in hotel_frontend hotel_geo hotel_profile hotel_rate hotel_reservation hotel_search hotel_user; do
  kind load docker-image --name hotel ${image}:latest
done

# 3. Deploy the Helm chart (namespace `hotel` is arbitrary)
kubectl create namespace hotel
helm upgrade --install hotel apps/hotel/deploy/helm/hotel --namespace hotel

# 4. Wait for pods to become ready
kubectl get pods -n hotel

# 5. Expose the frontend (either via the port mapping above or port-forwarding)
kubectl port-forward svc/hotel-hotel-frontend-service 8660:8660 -n hotel
```

Open `http://localhost:8660` in a browser or use the existing load generator/profile clients against `localhost:8660`.

## Configuration notes

- The chart produces a shared `hotel-config.json` ConfigMap and mounts it at `/usr/config.json` inside every service container. This replaces the bind-mounted config file that Docker Compose used. If you change service names or ports, update `values.yaml` (or provide an override file) so the ConfigMap points services at the right DNS names.
- Each cached service (rate/profile/reservation/review) gets dedicated MongoDB and Memcached `Deployment + Service` pairs. By default they use `emptyDir` storage so data resets between deployments; adjust these templates if you need persistence.
- `recommendation` and `review` services are defined but disabled by default to match the previous Compose setup. To enable them, set `appServices.recommendation.enabled=true` (and likewise for `review`) and load their images into the cluster.
- If you push images to a registry prefix (for example `localhost:5001/hotel_frontend`), set `global.image.registry` in your values override, or override individual `appServices.<svc>.image.repository`/`tag` fields.
- Every workload uses a dedicated service account (created by default) that is bound to a cluster role granting `get/list/watch` on `endpointslices` and `endpoints`, so the binaries can query the Kubernetes API for peer discovery. Override `serviceAccount.create/name` or disable the RBAC with `rbac.create=false` if you have your own setup.

## Useful commands

- Tear down the release: `helm uninstall hotel -n hotel`
- Delete the kind cluster: `kind delete cluster --name hotel`
- Inspect the generated config: `kubectl get configmap hotel-hotel-config -n hotel -o jsonpath='{.data.hotel-config\.json}' | jq`

## Updating application configuration

The application binaries read `/usr/config.json` at startup. When running under Helm, the relevant addresses are already set to the Kubernetes service DNS names:

| Logical Service | DNS name in cluster | Port |
| ----------------| ------------------- | ---- |
| Frontend        | `hotel-hotel-frontend-service` | 8660 |
| Geo             | `hotel-hotel-geo-service` | 8661 |
| Profile         | `hotel-hotel-profile-service` | 8662 |
| Rate            | `hotel-hotel-rate-service` | 8663 |
| Reservation     | `hotel-hotel-reservation-service` | 8666 |
| Search          | `hotel-hotel-search-service` | 8668 |
| User            | `hotel-hotel-user-service` | 8669 |
| Profile MongoDB | `hotel-hotel-profile-mongo` | 27004 |
| Profile Memcached | `hotel-hotel-profile-memcached` | 11004 |
| Rate MongoDB    | `hotel-hotel-rate-mongo` | 27003 |
| Rate Memcached  | `hotel-hotel-rate-memcached` | 11003 |
| Reservation MongoDB | `hotel-hotel-reservation-mongo` | 27005 |
| Reservation Memcached | `hotel-hotel-reservation-memcached` | 11005 |
| User MongoDB    | `hotel-hotel-user-mongo` | 27006 |

If you run binaries outside the cluster (for example directly on your host), reuse the generated `hotel-config.json` or copy the table above into `apps/hotel/scripts/local/hotel_config.localhost.json` so that the client points to the same addresses.

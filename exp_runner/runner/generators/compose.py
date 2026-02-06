"""
Docker Compose deployment generator.

Generates docker-compose.yaml from TopologySpec + ExperimentConfigV2.
"""

import logging
from pathlib import Path
from typing import Any

import yaml

from .base import DeploymentGenerator, GeneratedDeployment
from ..topology import format_call_graph_service_name

logger = logging.getLogger(__name__)

# Type annotations for forward references
if False:  # TYPE_CHECKING
    from ..topology import TopologySpec
    from ..experiment_config_v2 import ExperimentConfigV2


class ComposeGenerator(DeploymentGenerator):
    """
    Generates docker-compose.yaml from topology and experiment configuration.

    The generated compose file includes:
    - Application services with correct image tags and replicas
    - Infrastructure services (databases, caches, etc.)
    - Service dependencies from topology
    - Environment variables for service discovery
    - Resource limits
    - Networks and volumes
    """

    def generate(
        self,
        topology: "TopologySpec",
        experiment: "ExperimentConfigV2",
        output_dir: Path,
        project_name: str,
        policy: str,
        image_tag: str,
    ) -> GeneratedDeployment:
        """
        Generate docker-compose.yaml from topology and experiment config.

        Args:
            topology: Topology specification with services and dependencies
            experiment: Experiment configuration with RPS, SLO, policies
            output_dir: Directory to write generated compose file
            project_name: Project name for Docker Compose isolation
            policy: Current policy being tested (for image tags)
            image_tag: Docker image tag to use

        Returns:
            GeneratedDeployment with compose file path and env vars

        Raises:
            ValueError: If topology or experiment config is invalid
            IOError: If file generation fails
        """
        # Validate inputs
        self.validate_topology(topology)
        self.validate_experiment(experiment, topology)

        logger.info(f"Generating docker-compose.yaml for {topology.app}")

        # Build compose dictionary
        if topology.call_graph:
            compose_dict = self._build_call_graph_compose(
                topology=topology,
                experiment=experiment,
                project_name=project_name,
                image_tag=image_tag,
            )
        else:
            compose_dict = self._build_compose_dict(
                topology=topology,
                experiment=experiment,
                project_name=project_name,
                policy=policy,
                image_tag=image_tag,
            )

        # Write compose file
        output_dir.mkdir(parents=True, exist_ok=True)
        compose_path = output_dir / "docker-compose.yaml"

        try:
            with open(compose_path, "w", encoding="utf-8") as f:
                yaml.dump(
                    compose_dict,
                    f,
                    default_flow_style=False,
                    sort_keys=False,
                    allow_unicode=True,
                )
            logger.info(f"Generated compose file: {compose_path}")
        except Exception as e:
            raise IOError(f"Failed to write compose file: {e}") from e

        # Build environment variables
        env_vars = {
            "DOCKER_COMPOSE_PROJECT_NAME": project_name,
            "APP_CONFIG_PATH": str(output_dir / "config.json"),
        }

        return GeneratedDeployment(
            deploy_root=output_dir,
            deploy_file="docker-compose.yaml",
            env_vars=env_vars,
            cleanup_files=[compose_path],
        )

    def _build_compose_dict(
        self,
        topology: "TopologySpec",
        experiment: "ExperimentConfigV2",
        project_name: str,
        policy: str,
        image_tag: str,
    ) -> dict[str, Any]:
        """
        Build docker-compose dictionary from topology and experiment.

        Args:
            topology: Topology specification
            experiment: Experiment configuration
            project_name: Docker Compose project name
            policy: Current policy
            image_tag: Docker image tag

        Returns:
            Dictionary representing docker-compose.yaml structure
        """
        services: dict[str, Any] = {}
        volumes: dict[str, Any] = {}

        # Get replica overrides from experiment
        replica_overrides = experiment.replica_overrides or {}

        # Generate infrastructure services (databases, caches, etc.)
        for infra_name, infra_spec in topology.infrastructure.items():
            service_def = self._build_infrastructure_service(
                name=infra_name,
                spec=infra_spec,
                network_name=f"{topology.app}-network",
            )
            services[infra_name] = service_def

            # Add volume if needed
            if infra_spec.image and any(
                db in infra_spec.image for db in ["mongo", "redis", "postgres"]
            ):
                volume_name = f"{infra_name.replace('-', '_')}-data"
                volumes[volume_name] = {}

        # Generate application services
        for svc_name, svc_spec in topology.services.items():
            replicas = replica_overrides.get(svc_name, svc_spec.default_replicas)

            service_def = self._build_application_service(
                name=svc_name,
                spec=svc_spec,
                topology=topology,
                project_name=project_name,
                image_tag=image_tag,
                replicas=replicas,
                network_name=f"{topology.app}-network",
            )
            services[svc_name] = service_def

        # Build final compose structure
        compose_dict: dict[str, Any] = {"services": services}

        if volumes:
            compose_dict["volumes"] = volumes

        # Add network
        compose_dict["networks"] = {f"{topology.app}-network": {"driver": "bridge"}}

        return compose_dict

    def _build_infrastructure_service(
        self,
        name: str,
        spec: Any,
        network_name: str,
    ) -> dict[str, Any]:
        """
        Build docker-compose service definition for infrastructure component.

        Args:
            name: Service name
            spec: ServiceSpec for infrastructure component
            network_name: Docker network name

        Returns:
            Service definition dictionary
        """
        service_def: dict[str, Any] = {
            "image": spec.image,
            "restart": "always",
            "networks": [network_name],
        }

        # Add volume mount for databases
        if any(db in spec.image for db in ["mongo", "redis", "postgres"]):
            volume_name = f"{name.replace('-', '_')}-data"
            mount_path = "/data/db" if "mongo" in spec.image else "/data"
            service_def["volumes"] = [f"{volume_name}:{mount_path}"]

        # Add resource limits
        service_def["deploy"] = {"resources": {"limits": {"cpus": "4", "memory": "4G"}}}

        # Special handling for specific services
        if "rabbitmq" in spec.image:
            service_def["volumes"] = ["rabbitmq-data:/var/lib/rabbitmq"]
        elif "mongo" in name:
            # Mongo-specific settings
            if "url-shorten" in name:
                service_def["ulimits"] = {"nofile": {"soft": 64000, "hard": 64000}}

        return service_def

    def _build_application_service(
        self,
        name: str,
        spec: Any,
        topology: "TopologySpec",
        project_name: str,
        image_tag: str,
        replicas: int,
        network_name: str,
    ) -> dict[str, Any]:
        """
        Build docker-compose service definition for application service.

        Args:
            name: Service name
            spec: ServiceSpec for the service
            topology: Full topology spec (for looking up dependencies)
            project_name: Docker Compose project name
            image_tag: Docker image tag
            replicas: Number of replicas
            network_name: Docker network name

        Returns:
            Service definition dictionary
        """
        # Determine image name based on app and service
        image_name = self._get_image_name(topology.app, name, spec, image_tag)

        service_def: dict[str, Any] = {
            "image": image_name,
            "scale": replicas,
            "restart": "always",
            "networks": [network_name],
        }

        # Add environment variables
        env_vars = self._build_service_env_vars(
            name=name,
            spec=spec,
            topology=topology,
            project_name=project_name,
        )
        if env_vars:
            service_def["environment"] = env_vars

        # Add dependencies
        if spec.depends_on:
            service_def["depends_on"] = spec.depends_on

        # Add config volume mount
        service_def["volumes"] = ["${APP_CONFIG_PATH}:/usr/config.json:ro"]

        # Add resource limits
        service_def["deploy"] = {"resources": {"limits": {"cpus": "4", "memory": "4G"}}}

        # Add port mapping for frontend services
        if name in ["frontend", "compose-post-service"]:
            port = spec.port or 8080
            service_def["ports"] = [f"${{FRONTEND_PORT}}:{port}"]

        # Add command if specified
        if spec.command:
            service_def["command"] = spec.command

        return service_def

    def _get_image_name(
        self, app: str, service_name: str, spec: Any, image_tag: str
    ) -> str:
        """
        Generate Docker image name from app, service, and tag.

        Args:
            app: Application name
            service_name: Service name
            spec: Service spec
            image_tag: Image tag (policy)

        Returns:
            Full image name with tag
        """
        # If image is explicitly defined in topology, use it
        if hasattr(spec, "image") and spec.image:
            # Handle variable interpolation for image tag if needed, but for now append tag
            return f"{spec.image}:{image_tag}"

        # Handle different naming conventions per app
        if app == "synthetic":
            if "frontend" in service_name:
                return f"synthetic_frontend:{image_tag}"
            else:
                return f"synthetic_child:{image_tag}"
        elif app == "hotel":
            # Hotel uses service name directly
            return f"{service_name}:{image_tag}"
        elif app == "socialnet":
            # Socialnet uses specific naming
            if service_name == "compose-post-service":
                return f"compose_post_server:${{SOCIALNET_IMAGE_TAG:-{image_tag}}}"
            else:
                # Map service name to binary name
                binary_name = service_name.replace("-service", "").replace("-", "_")
                return f"{binary_name}:${{SOCIALNET_IMAGE_TAG:-{image_tag}}}"
        elif app == "mssim":
            return f"generic_service:{image_tag}"
        else:
            # Default: use service name
            return f"{service_name}:{image_tag}"

    def _build_service_env_vars(
        self,
        name: str,
        spec: Any,
        topology: "TopologySpec",
        project_name: str,
    ) -> dict[str, str]:
        """
        Build environment variables for a service.

        Includes:
        - BINARY_NAME for identifying the service binary
        - Service discovery variables for dependencies
        - Database/cache connection strings
        - App-specific configuration

        Args:
            name: Service name
            spec: ServiceSpec
            topology: Full topology
            project_name: Docker Compose project name

        Returns:
            Dictionary of environment variables
        """
        env_vars: dict[str, str] = {}

        # Add binary name
        if topology.app == "hotel":
            # Hotel uses hotel_{short_name} convention
            # e.g. "rate-service" -> "hotel_rate"
            # "frontend" -> "hotel_frontend"
            short_name = name.replace("-service", "")
            binary_name = f"hotel_{short_name}"
        elif topology.app == "socialnet":
            # Socialnet mapping logic
            # "compose-post-service" -> "compose_post_server"
            # "user-timeline-service" -> "user_timeline_server"
            # We assume the default convention matches what we need or explicit overrides
            # For now, let's keep the generic replacement which might be close enough
            # if we don't have the explicit mapping here.
            # But earlier in _get_image_name we had explicit logic.
            # Let's reuse that or be simple.
            # socialnet binaries are named *_server or *_service.
            # The generic replacement gives "compose_post_server" if name is "compose-post-server"?
            # No, generic replacement gives "compose_post_service".
            # Socialnet binary is "compose_post_server".
            # So we DO need app specific logic if we rely on BINARY_NAME.
            # However, socialnet images (built by us) have correct ENTRYPOINT/CMD?
            # Socialnet Dockerfile sets BINARY_NAME in ENV?
            # SocialnetApp sets BINARY_NAME in legacy path.
            # For now, let's just fix Hotel as requested.
            binary_name = name.replace("-service", "").replace("-", "_")
        else:
            binary_name = name.replace("-service", "").replace("-", "_")

        env_vars["BINARY_NAME"] = binary_name

        # Add log level
        env_vars["LOG_LEVEL"] = "${LOG_LEVEL:-info}"

        # Add project name for service discovery
        env_vars["DOCKER_COMPOSE_PROJECT_NAME"] = "${DOCKER_COMPOSE_PROJECT_NAME:-}"

        # Add service discovery variables for dependencies
        for dep_name in spec.depends_on:
            if dep_name in topology.services:
                dep_spec = topology.services[dep_name]
                # Add variables in the format: DEPENDENCY_IP, DEPENDENCY_PORT, DEPENDENCY_REPLICAS
                dep_var_name = dep_name.upper().replace("-", "_")
                env_vars[f"{dep_var_name}_IP"] = f"${{PROJECT_NAME}}-{dep_name}"
                env_vars[f"{dep_var_name}_PORT"] = str(dep_spec.port or 8080)
                env_vars[f"{dep_var_name}_REPLICAS"] = str(dep_spec.default_replicas)
            elif dep_name in topology.infrastructure:
                # Infrastructure dependency - add connection string
                infra_spec = topology.infrastructure[dep_name]
                if "mongo" in dep_name:
                    db_name = name.replace("-service", "")
                    env_vars[f"{dep_name.upper().replace('-', '_')}_URI"] = (
                        f"mongodb://{dep_name}:27017/{db_name}"
                    )
                elif "redis" in dep_name:
                    env_vars[f"{dep_name.upper().replace('-', '_')}_URL"] = (
                        f"redis://{dep_name}:6379"
                    )
                elif "memcached" in dep_name:
                    env_vars[f"{dep_name.upper().replace('-', '_')}_ADDR"] = (
                        f"tcp://{dep_name}:11211"
                    )
                elif "rabbitmq" in dep_name:
                    env_vars["RABBITMQ_URL"] = "amqp://guest:guest@rabbitmq:5672"

        return env_vars

    def validate_topology(self, topology: "TopologySpec") -> None:
        """
        Validate topology for docker-compose generation.

        Args:
            topology: Topology to validate

        Raises:
            ValueError: If topology is invalid
        """
        if not topology.app:
            raise ValueError("Topology must specify an app name")

        if (
            not topology.services
            and not topology.infrastructure
            and not topology.call_graph
        ):
            raise ValueError(
                "Topology must have at least one service or infrastructure"
            )

    def _build_call_graph_compose(
        self,
        topology: "TopologySpec",
        experiment: "ExperimentConfigV2",
        project_name: str,
        image_tag: str,
    ) -> dict[str, Any]:
        """
        Build docker-compose file for call-graph-driven apps (synthetic/mssim).

        Args:
            topology: Topology specification with call_graph defined
            experiment: Experiment configuration
            project_name: Docker Compose project name
            image_tag: Docker image tag

        Returns:
            Compose dictionary
        """
        if topology.app != "synthetic":
            raise ValueError(
                "Call-graph compose generation currently supports the synthetic app only"
            )

        call_graph = topology.call_graph or {}
        services: dict[str, Any] = {}
        network_name = f"{topology.app}-network"

        child_cpu_limit = call_graph.get("child_cpus_per_replica")
        child_cpu_value = (
            str(child_cpu_limit)
            if child_cpu_limit is not None
            else "${CPUS_PER_REPLICA:-1}"
        )

        child_service_names: list[str] = []
        replica_overrides = experiment.replica_overrides or {}

        for service_def in call_graph.get("services", []):
            service_id = service_def.get("id")
            if not service_id:
                continue

            service_name = format_call_graph_service_name(service_id)
            replicas = self._resolve_call_graph_replicas(
                service_def, replica_overrides, service_name
            )

            env_list = [
                "BINARY_NAME=synthetic_child",
                "LOG_LEVEL=${LOG_LEVEL:-info}",
                f"SERVICE_ID={service_id}",
                "DOCKER_COMPOSE_PROJECT_NAME=${DOCKER_COMPOSE_PROJECT_NAME:-}",
            ]

            services[service_name] = {
                "image": f"synthetic_child:{image_tag}",
                "scale": replicas,
                "networks": [network_name],
                "environment": env_list,
                "volumes": ["${APP_CONFIG_PATH}:/usr/config.json:ro"],
                "deploy": {
                    "resources": {
                        "limits": {
                            "cpus": child_cpu_value,
                        }
                    }
                },
            }
            child_service_names.append(service_name)

        frontend_env = [
            "BINARY_NAME=synthetic_frontend",
            "LOG_LEVEL=${LOG_LEVEL:-info}",
            "DOCKER_COMPOSE_PROJECT_NAME=${DOCKER_COMPOSE_PROJECT_NAME:-}",
        ]

        services["synthetic-frontend-service"] = {
            "image": f"synthetic_frontend:{image_tag}",
            "restart": "always",
            "depends_on": child_service_names,
            "networks": [network_name],
            "environment": frontend_env,
            "volumes": ["${APP_CONFIG_PATH}:/usr/config.json:ro"],
            "deploy": {"resources": {"limits": {"cpus": "4"}}},
        }

        compose_dict: dict[str, Any] = {
            "services": services,
            "networks": {network_name: {"driver": "bridge"}},
        }

        return compose_dict

    def _resolve_call_graph_replicas(
        self,
        service_def: dict[str, Any],
        overrides: dict[str, int],
        service_name: str,
    ) -> int:
        base = service_def.get("default_replicas") or service_def.get("replicas") or 1
        service_id = service_def.get("id")

        if service_id and service_id in overrides:
            return overrides[service_id]

        if service_name in overrides:
            return overrides[service_name]

        return base

    def validate_experiment(
        self,
        experiment: "ExperimentConfigV2",
        topology: "TopologySpec",
    ) -> None:
        """
        Validate experiment config against topology.

        Args:
            experiment: Experiment config
            topology: Topology spec

        Raises:
            ValueError: If experiment config is invalid
        """
        # Check that replica overrides reference valid services
        for svc_name in experiment.replica_overrides:
            if svc_name not in topology.services:
                raise ValueError(
                    f"Replica override for unknown service: {svc_name}. "
                    f"Valid services: {list(topology.services.keys())}"
                )

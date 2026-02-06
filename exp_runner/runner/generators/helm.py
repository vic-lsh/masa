"""
Helm values generator for Kubernetes deployments.

Generates Helm values.yaml from TopologySpec + ExperimentConfigV2.
"""

import logging
from pathlib import Path
from typing import Any

import yaml

from .base import DeploymentGenerator, GeneratedDeployment

logger = logging.getLogger(__name__)

# Type annotations for forward references
if False:  # TYPE_CHECKING
    from ..topology import TopologySpec
    from ..experiment_config_v2 import ExperimentConfigV2


class HelmValuesGenerator(DeploymentGenerator):
    """
    Generates Helm values.yaml from topology and experiment configuration.

    The generated values file includes:
    - Service replicas from topology (with experiment overrides)
    - Image tags for each service
    - Application configuration (call graph, methods, etc.)
    - Resource limits and requests
    - Service mesh configuration
    - Namespace isolation settings
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
        Generate Helm values.yaml from topology and experiment config.

        Args:
            topology: Topology specification with services and dependencies
            experiment: Experiment configuration with RPS, SLO, policies
            output_dir: Directory to write generated values file
            project_name: Project name for K8s namespace isolation
            policy: Current policy being tested (for image tags)
            image_tag: Docker image tag to use

        Returns:
            GeneratedDeployment with values file path and env vars

        Raises:
            ValueError: If topology or experiment config is invalid
            IOError: If file generation fails
        """
        # Validate inputs
        self.validate_topology(topology)
        self.validate_experiment(experiment, topology)

        logger.info(f"Generating Helm values.yaml for {topology.app}")

        # Build values dictionary
        values_dict = self._build_values_dict(
            topology=topology,
            experiment=experiment,
            project_name=project_name,
            policy=policy,
            image_tag=image_tag,
        )

        # Write values file
        output_dir.mkdir(parents=True, exist_ok=True)
        values_path = output_dir / "values.yaml"

        try:
            with open(values_path, "w", encoding="utf-8") as f:
                yaml.dump(
                    values_dict,
                    f,
                    default_flow_style=False,
                    sort_keys=False,
                    allow_unicode=True,
                )
            logger.info(f"Generated Helm values: {values_path}")
        except Exception as e:
            raise IOError(f"Failed to write Helm values file: {e}") from e

        # Build environment variables
        env_vars = {
            "HELM_VALUES_FILE": str(values_path.resolve()),
        }

        return GeneratedDeployment(
            deploy_root=output_dir,
            deploy_file="values.yaml",
            env_vars=env_vars,
            cleanup_files=[values_path],
        )

    def _build_values_dict(
        self,
        topology: "TopologySpec",
        experiment: "ExperimentConfigV2",
        project_name: str,
        policy: str,
        image_tag: str,
    ) -> dict[str, Any]:
        """
        Build Helm values dictionary from topology and experiment.

        Args:
            topology: Topology specification
            experiment: Experiment configuration
            project_name: K8s release/namespace name
            policy: Current policy
            image_tag: Docker image tag

        Returns:
            Dictionary representing Helm values.yaml structure
        """
        values: dict[str, Any] = {}

        # Set full name override for predictable service names
        values["fullnameOverride"] = project_name

        # Image configuration
        values["image"] = {
            "tag": image_tag,
            "pullPolicy": "IfNotPresent",
        }

        # App-specific image names
        if topology.app == "synthetic":
            values["image"]["frontendName"] = "synthetic_frontend"
            values["image"]["childName"] = "synthetic_child"
        elif topology.app == "hotel":
            values["image"]["repository"] = ""  # Hotel uses service-specific images
        elif topology.app == "socialnet":
            values["image"]["repository"] = ""  # Socialnet uses service-specific images

        # Service configuration with replicas
        values["services"] = self._build_services_config(topology, experiment)

        # Infrastructure configuration
        if topology.infrastructure:
            values["infrastructure"] = self._build_infrastructure_config(topology)

        # Application configuration (call graph, etc.)
        if topology.call_graph:
            values["appConfig"] = {"call_graph": topology.call_graph}
        elif hasattr(experiment, "loadgen") and experiment.loadgen:
            # Add API specs if available
            values["appConfig"] = self._build_app_config_from_experiment(experiment)

        # Resource defaults
        values["resources"] = self._build_resource_limits(topology)

        # Log level
        values["logLevel"] = "info"

        # Service account
        values["serviceAccount"] = {
            "create": True,
            "name": f"{project_name}-sa",
        }

        return values

    def _build_services_config(
        self,
        topology: "TopologySpec",
        experiment: "ExperimentConfigV2",
    ) -> dict[str, Any]:
        """
        Build services configuration with replicas.

        Args:
            topology: Topology specification
            experiment: Experiment configuration

        Returns:
            Dictionary of service configurations
        """
        services_config: dict[str, Any] = {}
        replica_overrides = experiment.replica_overrides or {}

        for svc_name, svc_spec in topology.services.items():
            replicas = replica_overrides.get(svc_name, svc_spec.default_replicas)

            service_config: dict[str, Any] = {
                "replicas": replicas,
                "port": svc_spec.port or 8080,
            }

            # Add dependencies
            if svc_spec.depends_on:
                service_config["dependsOn"] = svc_spec.depends_on

            # Add methods if defined
            if svc_spec.methods:
                service_config["methods"] = svc_spec.methods

            services_config[svc_name] = service_config

        return services_config

    def _build_infrastructure_config(
        self,
        topology: "TopologySpec",
    ) -> dict[str, Any]:
        """
        Build infrastructure configuration (databases, caches, etc.).

        Args:
            topology: Topology specification

        Returns:
            Dictionary of infrastructure configurations
        """
        infra_config: dict[str, Any] = {}

        for infra_name, infra_spec in topology.infrastructure.items():
            config: dict[str, Any] = {
                "image": infra_spec.image,
                "replicas": infra_spec.default_replicas,
            }

            if infra_spec.port:
                config["port"] = infra_spec.port

            infra_config[infra_name] = config

        return infra_config

    def _build_app_config_from_experiment(
        self,
        experiment: "ExperimentConfigV2",
    ) -> dict[str, Any]:
        """
        Build application configuration from experiment loadgen spec.

        Args:
            experiment: Experiment configuration

        Returns:
            Application configuration dictionary
        """
        app_config: dict[str, Any] = {}

        if hasattr(experiment, "loadgen") and experiment.loadgen:
            # Add API configurations
            if experiment.loadgen.apis:
                app_config["apis"] = [
                    {
                        "name": api.name,
                        "slo_us": api.slo_us,
                        "weight": api.weight,
                    }
                    for api in experiment.loadgen.apis
                ]

            # Add default timeout
            if experiment.loadgen.default_timeout_ms:
                app_config["default_timeout_ms"] = (
                    experiment.loadgen.default_timeout_ms
                )

        return app_config

    def _build_resource_limits(
        self,
        topology: "TopologySpec",
    ) -> dict[str, Any]:
        """
        Build default resource limits for services.

        Args:
            topology: Topology specification

        Returns:
            Dictionary of resource limits by service type
        """
        resources: dict[str, Any] = {}

        # Default limits for application services
        resources["default"] = {
            "limits": {"cpu": "4", "memory": "4Gi"},
            "requests": {"cpu": "100m", "memory": "128Mi"},
        }

        # Limits for frontend services (usually higher)
        resources["frontend"] = {
            "limits": {"cpu": "4", "memory": "4Gi"},
            "requests": {"cpu": "500m", "memory": "512Mi"},
        }

        # Limits for infrastructure (databases, caches)
        resources["infrastructure"] = {
            "limits": {"cpu": "4", "memory": "4Gi"},
            "requests": {"cpu": "100m", "memory": "256Mi"},
        }

        return resources

    def validate_topology(self, topology: "TopologySpec") -> None:
        """
        Validate topology for Helm values generation.

        Args:
            topology: Topology to validate

        Raises:
            ValueError: If topology is invalid
        """
        if not topology.app:
            raise ValueError("Topology must specify an app name")

        if not topology.services:
            raise ValueError("Topology must have at least one service")

        # Validate that all services have ports defined
        for svc_name, svc_spec in topology.services.items():
            if not svc_spec.port:
                logger.warning(f"Service {svc_name} has no port, defaulting to 8080")

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

        # Validate that experiment has execution spec
        if not hasattr(experiment, "execution") or not experiment.execution:
            raise ValueError("Experiment must have execution specification")

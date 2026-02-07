"""
Base deployment generator interface.

Generates platform-specific deployment manifests from topology + experiment config.
"""

import logging
from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from pathlib import Path

logger = logging.getLogger(__name__)

# Type annotation for forward reference
if False:  # TYPE_CHECKING
    from ..topology import TopologySpec
    from ..experiment_config_v2 import ExperimentConfigV2


@dataclass
class GeneratedDeployment:
    """Result of deployment generation."""

    deploy_root: Path  # Base directory for deployment command
    deploy_file: str  # Config filename relative to deploy_root
    env_vars: dict[str, str] = field(default_factory=dict)  # Environment variables
    cleanup_files: list[Path] = field(default_factory=list)  # Files to clean up


class DeploymentGenerator(ABC):
    """
    Abstract base class for deployment generators.

    Implementations generate platform-specific deployment manifests
    (docker-compose.yaml, Helm values.yaml, etc.) by merging topology
    and experiment configurations.
    """

    @abstractmethod
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
        Generate deployment files (compose yaml or helm values).

        Args:
            topology: Topology specification (services, dependencies)
            experiment: Experiment configuration (RPS, SLO, policies)
            output_dir: Directory for generated files
            project_name: Project name for namespace isolation
            policy: Current policy being tested
            image_tag: Docker image tag to use

        Returns:
            GeneratedDeployment with paths and env vars

        Raises:
            ValueError: If configuration is invalid
            IOError: If file generation fails
        """
        pass

    def validate_topology(self, topology: "TopologySpec") -> None:
        """
        Validate topology spec for this generator.

        Override to add platform-specific validation.

        Args:
            topology: Topology to validate

        Raises:
            ValueError: If topology is invalid
        """
        pass

    def validate_experiment(
        self,
        experiment: "ExperimentConfigV2",
        topology: "TopologySpec",
    ) -> None:
        """
        Validate experiment config against topology.

        Override to add platform-specific validation.

        Args:
            experiment: Experiment config to validate
            topology: Topology spec

        Raises:
            ValueError: If experiment config is invalid
        """
        pass

"""
Topology definitions and resolution logic.

Separates topology (service structure) from experiment config (RPS, SLO, policies).
"""

import logging
import yaml
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Optional

logger = logging.getLogger(__name__)


@dataclass
class ServiceSpec:
    """Specification for a single service in the topology."""

    id: str
    port: Optional[int] = None
    default_replicas: int = 1
    depends_on: list[str] = field(default_factory=list)
    methods: list[dict[str, Any]] = field(default_factory=list)
    image: Optional[str] = None  # For infrastructure services


@dataclass
class TopologySpec:
    """
    Topology specification defining the structure of services.

    This is separate from experiment configuration - topology changes rarely,
    while experiment parameters (RPS, SLO, policies) change often.
    """

    kind: str = "Topology"
    app: str = ""
    description: str = ""

    # Service definitions
    services: dict[str, ServiceSpec] = field(default_factory=dict)

    # Infrastructure services (mongo, redis, etc.)
    infrastructure: dict[str, ServiceSpec] = field(default_factory=dict)

    # Call graph (for synthetic/mssim)
    call_graph: Optional[dict[str, Any]] = None

    # Metadata
    metadata: dict[str, Any] = field(default_factory=dict)

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "TopologySpec":
        """Load topology from dictionary (parsed YAML)."""
        metadata = data.get("metadata", {})
        app = metadata.get("app", "")
        description = metadata.get("description", "")

        # Parse services
        services = {}
        for svc_name, svc_data in data.get("services", {}).items():
            if isinstance(svc_data, dict):
                services[svc_name] = ServiceSpec(
                    id=svc_name,
                    port=svc_data.get("port"),
                    default_replicas=svc_data.get("default_replicas", 1),
                    depends_on=svc_data.get("depends_on", []),
                    methods=svc_data.get("methods", []),
                    image=svc_data.get("image"),
                )
            else:
                # Allow simple list format
                services[svc_name] = ServiceSpec(id=svc_name)

        # Parse infrastructure
        infrastructure = {}
        for infra_name, infra_data in data.get("infrastructure", {}).items():
            if isinstance(infra_data, dict):
                infrastructure[infra_name] = ServiceSpec(
                    id=infra_name,
                    port=infra_data.get("port"),
                    default_replicas=infra_data.get("default_replicas", 1),
                    image=infra_data.get("image"),
                )

        return cls(
            kind=data.get("kind", "Topology"),
            app=app,
            description=description,
            services=services,
            infrastructure=infrastructure,
            call_graph=data.get("call_graph"),
            metadata=metadata,
        )

    @classmethod
    def from_yaml(cls, path: Path) -> "TopologySpec":
        """Load topology from YAML file."""
        with open(path) as f:
            data = yaml.safe_load(f)
        return cls.from_dict(data)

    def to_dict(self) -> dict[str, Any]:
        """Convert topology to dictionary for serialization."""
        services_dict = {}
        for name, svc in self.services.items():
            svc_data: dict[str, Any] = {
                "port": svc.port,
                "default_replicas": svc.default_replicas,
            }
            if svc.depends_on:
                svc_data["depends_on"] = svc.depends_on
            if svc.methods:
                svc_data["methods"] = svc.methods
            if svc.image:
                svc_data["image"] = svc.image
            services_dict[name] = svc_data

        infrastructure_dict = {}
        for name, infra in self.infrastructure.items():
            infra_data: dict[str, Any] = {
                "port": infra.port,
                "default_replicas": infra.default_replicas,
            }
            if infra.image:
                infra_data["image"] = infra.image
            infrastructure_dict[name] = infra_data

        result: dict[str, Any] = {
            "kind": self.kind,
            "metadata": {
                "app": self.app,
                "description": self.description,
                **self.metadata,
            },
        }

        if services_dict:
            result["services"] = services_dict
        if infrastructure_dict:
            result["infrastructure"] = infrastructure_dict
        if self.call_graph:
            result["call_graph"] = self.call_graph

        return result

    def apply_replica_overrides(
        self, overrides: dict[str, int]
    ) -> "TopologySpec":
        """
        Apply replica overrides to create a new topology spec.

        Args:
            overrides: Map of service_name -> replica_count

        Returns:
            New TopologySpec with overrides applied
        """
        new_services = {}
        for name, svc in self.services.items():
            if name in overrides:
                new_svc = ServiceSpec(
                    id=svc.id,
                    port=svc.port,
                    default_replicas=overrides[name],
                    depends_on=svc.depends_on[:],  # Copy list
                    methods=svc.methods[:],  # Copy list
                    image=svc.image,
                )
                new_services[name] = new_svc
            else:
                # Keep original
                new_services[name] = svc

        return TopologySpec(
            kind=self.kind,
            app=self.app,
            description=self.description,
            services=new_services,
            infrastructure=self.infrastructure.copy(),
            call_graph=self.call_graph,
            metadata=self.metadata.copy(),
        )


class TopologyResolver:
    """Resolves topology references to concrete topology specs."""

    def __init__(self, repo_root: Path):
        self.repo_root = repo_root

    def resolve(
        self,
        app_name: str,
        topology_ref: Optional[str] = None,
    ) -> TopologySpec:
        """
        Resolve topology reference to concrete topology spec.

        Resolution order:
        1. If topology_ref is None or "default": Look in apps/{app}/topology.yaml
        2. For named ref: Look in exp/{app}/topologies/{ref}.yaml
        3. For mssim traces: Look in trace-analysis/golden/{ref}/ (future)

        Args:
            app_name: Application name (hotel, synthetic, etc.)
            topology_ref: Topology reference ("default", "fanout-3", etc.)

        Returns:
            TopologySpec loaded from file

        Raises:
            FileNotFoundError: If topology file not found
        """
        if topology_ref is None or topology_ref == "default":
            # Look in apps/{app}/topology.yaml
            topology_path = self.repo_root / "apps" / app_name / "topology.yaml"
            if topology_path.exists():
                logger.info(f"Loading default topology from {topology_path}")
                return TopologySpec.from_yaml(topology_path)
            else:
                # No explicit topology file - return empty spec
                logger.warning(
                    f"No topology file found at {topology_path}, using empty topology"
                )
                return TopologySpec(app=app_name)

        # Look in exp/{app}/topologies/{ref}.yaml
        topology_path = (
            self.repo_root / "exp" / app_name / "topologies" / f"{topology_ref}.yaml"
        )
        if topology_path.exists():
            logger.info(f"Loading topology '{topology_ref}' from {topology_path}")
            return TopologySpec.from_yaml(topology_path)

        raise FileNotFoundError(
            f"Topology '{topology_ref}' not found for app '{app_name}'. "
            f"Expected at: {topology_path}"
        )

    def apply_overrides(
        self,
        topology: TopologySpec,
        replica_overrides: Optional[dict[str, int]] = None,
    ) -> TopologySpec:
        """
        Apply per-experiment replica overrides.

        Args:
            topology: Base topology spec
            replica_overrides: Map of service_name -> replica_count

        Returns:
            New TopologySpec with overrides applied
        """
        if not replica_overrides:
            return topology

        logger.info(f"Applying replica overrides: {replica_overrides}")
        return topology.apply_replica_overrides(replica_overrides)

"""
New experiment configuration data model.

Separates experiment parameters (RPS, SLO, policies) from topology.
"""

import logging
import yaml
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Optional

logger = logging.getLogger(__name__)


@dataclass
class ApiSpec:
    """Specification for a single API endpoint in the workload."""

    name: str
    slo_us: int  # Required: SLO in microseconds
    weight: float = 1.0  # Optional: request weight (defaults to equal distribution)
    timeout_ms: Optional[int] = None  # Optional: override default timeout


@dataclass
class LoadGenSpec:
    """Load generation specification."""

    rps: list[float]  # RPS sweep values
    default_timeout_ms: int = 1000  # Default timeout for all APIs
    apis: list[ApiSpec] = field(default_factory=list)


@dataclass
class ExecutionSpec:
    """Experiment execution parameters."""

    repeats: int = 1
    policies: list[str] = field(default_factory=list)
    warmup_secs: int = 10
    duration_secs: int = 60


@dataclass
class ExperimentConfigV2:
    """
    New experiment configuration format.

    Separates experiment parameters from topology - allows multiple experiments
    to reference the same topology with different RPS/SLO/policy combinations.
    """

    kind: str = "Experiment"
    name: str = ""
    app: str = ""

    # Reference to topology (optional - uses app default if omitted)
    topology_ref: Optional[str] = None

    # Override replicas for this experiment (optional)
    replica_overrides: dict[str, int] = field(default_factory=dict)

    # Experiment execution parameters
    execution: ExecutionSpec = field(default_factory=ExecutionSpec)

    # Load generation parameters
    loadgen: LoadGenSpec = field(default_factory=LoadGenSpec)

    # Metadata
    metadata: dict[str, Any] = field(default_factory=dict)

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "ExperimentConfigV2":
        """Load experiment config from dictionary (parsed YAML)."""
        metadata = data.get("metadata", {})
        name = metadata.get("name", "")
        app = metadata.get("app", "")

        spec = data.get("spec", {})

        # Parse execution
        exec_data = spec.get("execution", {})
        execution = ExecutionSpec(
            repeats=exec_data.get("repeats", 1),
            policies=exec_data.get("policies", []),
            warmup_secs=exec_data.get("warmup_secs", 10),
            duration_secs=exec_data.get("duration_secs", 60),
        )

        # Parse loadgen
        loadgen_data = spec.get("loadgen", {})
        apis = []
        for api_data in loadgen_data.get("apis", []):
            apis.append(
                ApiSpec(
                    name=api_data["name"],
                    slo_us=api_data["slo_us"],
                    weight=api_data.get("weight", 1.0),
                    timeout_ms=api_data.get("timeout_ms"),
                )
            )

        loadgen = LoadGenSpec(
            rps=loadgen_data.get("rps", []),
            default_timeout_ms=loadgen_data.get("default_timeout_ms", 1000),
            apis=apis,
        )

        return cls(
            kind=data.get("kind", "Experiment"),
            name=name,
            app=app,
            topology_ref=spec.get("topology_ref"),
            replica_overrides=spec.get("replica_overrides", {}),
            execution=execution,
            loadgen=loadgen,
            metadata=metadata,
        )

    @classmethod
    def from_yaml(cls, path: Path) -> "ExperimentConfigV2":
        """Load experiment config from YAML file."""
        with open(path) as f:
            data = yaml.safe_load(f)
        return cls.from_dict(data)

    def to_dict(self) -> dict[str, Any]:
        """Convert experiment config to dictionary for serialization."""
        apis_data = []
        for api in self.loadgen.apis:
            api_dict: dict[str, Any] = {
                "name": api.name,
                "slo_us": api.slo_us,
            }
            if api.weight != 1.0:
                api_dict["weight"] = api.weight
            if api.timeout_ms is not None:
                api_dict["timeout_ms"] = api.timeout_ms
            apis_data.append(api_dict)

        spec: dict[str, Any] = {
            "execution": {
                "repeats": self.execution.repeats,
                "policies": self.execution.policies,
                "warmup_secs": self.execution.warmup_secs,
                "duration_secs": self.execution.duration_secs,
            },
            "loadgen": {
                "rps": self.loadgen.rps,
                "default_timeout_ms": self.loadgen.default_timeout_ms,
                "apis": apis_data,
            },
        }

        if self.topology_ref:
            spec["topology_ref"] = self.topology_ref

        if self.replica_overrides:
            spec["replica_overrides"] = self.replica_overrides

        return {
            "kind": self.kind,
            "metadata": {
                "name": self.name,
                "app": self.app,
                **self.metadata,
            },
            "spec": spec,
        }

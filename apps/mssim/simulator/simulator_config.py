from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, Optional

from .utils import normalize_service_name

DEFAULT_REPLICA_COUNT = 1


class SimulatorConfigError(RuntimeError):
    """Raised when simulator configuration cannot be parsed."""


@dataclass
class ReplicaConfig:
    default: int = DEFAULT_REPLICA_COUNT
    overrides: Dict[str, int] = field(default_factory=dict)

    @classmethod
    def from_file(cls, path: Path) -> "ReplicaConfig":
        if not path.exists():
            return cls()
        try:
            data = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError) as exc:
            raise SimulatorConfigError(f"Failed to parse replica config {path}: {exc}") from exc

        default = int(data.get("default", DEFAULT_REPLICA_COUNT))
        overrides_raw = data.get("overrides", {})
        overrides: Dict[str, int] = {}
        for name, value in overrides_raw.items():
            overrides[normalize_service_name(str(name))] = int(value)
        return cls(default=default, overrides=overrides)

    def count_for(self, service_name: str) -> int:
        return self.overrides.get(service_name, self.default)

    def is_empty(self) -> bool:
        return not self.overrides


@dataclass
class SimulatorConfig:
    replicas: ReplicaConfig = field(default_factory=ReplicaConfig)

    @classmethod
    def from_replicas_path(cls, replicas_path: Optional[Path]) -> "SimulatorConfig":
        if replicas_path is None:
            return cls()
        replicas = ReplicaConfig.from_file(replicas_path)
        return cls(replicas=replicas)


from __future__ import annotations

import json
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Dict


@dataclass
class ServiceDiscoveryInfo:
    ip: str
    port: int
    replicas: int


@dataclass
class Deployment:
    services: Dict[str, ServiceDiscoveryInfo] = field(default_factory=dict)

    def add_service(self, name: str, info: ServiceDiscoveryInfo) -> None:
        self.services[name] = info

    def to_dict(self) -> Dict[str, dict]:
        return {name: asdict(info) for name, info in self.services.items()}

    def export_to_file(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        data = {"services": self.to_dict()}
        path.write_text(json.dumps(data, indent=2))

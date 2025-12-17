from __future__ import annotations

import csv
import json
import math
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, Iterable, Optional, Set

from .utils import normalize_service_name


class TraceConfigError(RuntimeError):
    """Raised when trace inputs cannot be parsed."""


@dataclass
class CallGraph:
    outgoing: Dict[str, Dict[str, int]] = field(default_factory=dict)

    @classmethod
    def from_path(cls, path: Path) -> "CallGraph":
        if not path.exists():
            raise TraceConfigError(f"Missing call graph CSV: {path}")

        with path.open(newline="") as handle:
            reader = csv.DictReader(handle)
            if reader.fieldnames is None:
                raise TraceConfigError(f"Call graph CSV has no header: {path}")
            missing = {"caller", "callee"} - set(name.strip() for name in reader.fieldnames if name)
            if missing:
                required = ", ".join(sorted(missing))
                raise TraceConfigError(f"Call graph CSV is missing header(s): {required}")

            outgoing: Dict[str, Dict[str, int]] = {}
            for row in reader:
                caller_raw = (row.get("caller") or "").strip()
                callee_raw = (row.get("callee") or "").strip()
                if not caller_raw or not callee_raw:
                    continue

                caller = normalize_service_name(caller_raw)
                callee = normalize_service_name(callee_raw)

                weight = cls._parse_weight(row.get("weight"))
                outgoing.setdefault(caller, {})[callee] = weight

        return cls(outgoing=outgoing)

    @staticmethod
    def _parse_weight(weight_value: Optional[str]) -> int:
        if not weight_value:
            return 1
        try:
            parsed = float(weight_value)
        except (TypeError, ValueError):
            raise TraceConfigError(f"Edge weight must be numeric: {weight_value!r}")
        if not math.isfinite(parsed):
            raise TraceConfigError(f"Edge weight must be finite: {weight_value!r}")
        if parsed < 0:
            raise TraceConfigError(f"Edge weight must be non-negative: {weight_value!r}")
        rounded = round(parsed)
        if abs(parsed - rounded) > 1e-6:
            raise TraceConfigError(f"Edge weight must be an integer value: {weight_value!r}")
        return int(rounded)

    def services(self) -> Set[str]:
        services: Set[str] = set(self.outgoing.keys())
        for targets in self.outgoing.values():
            services.update(targets.keys())
        return services

    def callees_of(self, service: str) -> Dict[str, int]:
        return dict(self.outgoing.get(service, {}))


@dataclass
class TraceConfig:
    call_graph: CallGraph
    method_freq_map: Optional[dict] = None

    @classmethod
    def from_config_dir(cls, directory: Path) -> "TraceConfig":
        call_graph_path = directory / "edges.csv"
        call_graph = CallGraph.from_path(call_graph_path)

        method_freq_path = directory / "interface_distribution.json"
        method_freq = None
        if method_freq_path.exists():
            method_freq = json.loads(method_freq_path.read_text())

        return cls(call_graph=call_graph, method_freq_map=method_freq)

    def iter_services(self) -> Iterable[str]:
        yield from sorted(self.call_graph.services())


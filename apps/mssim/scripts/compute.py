
### 
#!/usr/bin/env python3
"""Compute weighted-average latency per service using percentile and frequency data."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Dict

LATENCY_PATH = Path(
    "/home/jiexiao/research/masa-internal/trace-analysis/golden/S_86516878/latency_percentiles.json"
)
FREQUENCY_PATH = Path(
    "/home/jiexiao/research/masa-internal/trace-analysis/golden/S_86516878/interface_distribution.json"
)


def load_json(path: Path) -> Dict:
    with path.open("r") as fh:
        return json.load(fh)


def mean_latency(percentiles: Dict[str, float]) -> float:
    return percentiles["50"]


def normalized_weights(frequencies: Dict[str, float]) -> Dict[str, float]:
    total = sum(float(value) for value in frequencies.values())
    if total == 0.0:
        return {}
    return {iface: float(count) / total for iface, count in frequencies.items()}


def compute_weighted_latency(
    service_latency: Dict[str, Dict[str, float]],
    service_freq: Dict[str, float],
) -> float | None:
    interface_means = {
        interface: mean_latency(percentiles)
        for interface, percentiles in service_latency.items()
    }

    if not interface_means:
        return None

    weights = normalized_weights(service_freq)
    weighted_sum = 0.0
    weight_total = 0.0

    for interface, weight in weights.items():
        mean = interface_means.get(interface)
        if mean is None:
            continue
        weighted_sum += weight * mean
        weight_total += weight

    if weight_total == 0.0:
        return None

    return weighted_sum / weight_total


def main() -> None:
    latency_data = load_json(LATENCY_PATH)
    frequency_data = load_json(FREQUENCY_PATH)

    for service, service_latency in latency_data.items():
        service_freq = frequency_data.get(service)
        if not service_freq:
            print(f"{service}: skipped (no frequency data)")
            continue

        weighted_latency = compute_weighted_latency(service_latency, service_freq)
        if weighted_latency is None:
            print(f"{service}: skipped (missing latency data)")
            continue

        print(f"{service}: weighted mean latency = {weighted_latency:.4f}")


if __name__ == "__main__":
    main()

"""
Generate replica distribution plots for hotel experiments.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

import matplotlib
matplotlib.use("Agg")  # Non-interactive backend for file output
import matplotlib.pyplot as plt
import numpy as np


_EXPERIMENT_DIR_RE = re.compile(r"^(?P<policy>.+)_(?P<rps>\d+)$")
_DEFAULT_SERVICE_ORDER = [
    "frontend",
    "geo",
    "profile",
    "rate",
    "recommendation",
    "reservation",
    "review",
    "search",
    "user",
]


def _extract_replicas(hotel_config: dict) -> dict[str, int]:
    replicas: dict[str, int] = {}
    for service, config in hotel_config.items():
        if not isinstance(config, dict):
            continue
        if "replicas" not in config:
            continue
        try:
            replicas[service] = int(config["replicas"])
        except (TypeError, ValueError):
            continue
    return replicas


def _collect_replica_data(in_dir: Path) -> tuple[dict[str, dict[int, dict[str, int]]], list[str]]:
    data: dict[str, dict[int, dict[str, int]]] = {}
    services: set[str] = set()

    for entry in sorted(in_dir.iterdir()):
        if not entry.is_dir():
            continue
        match = _EXPERIMENT_DIR_RE.match(entry.name)
        if not match:
            continue
        policy = match.group("policy")
        rps = int(match.group("rps"))
        hotel_path = entry / "hotel.json"
        if not hotel_path.exists():
            continue
        with hotel_path.open() as f:
            hotel_config = json.load(f)
        replicas = _extract_replicas(hotel_config)
        if not replicas:
            continue
        data.setdefault(policy, {})[rps] = replicas
        services.update(replicas.keys())

    service_order: list[str] = []
    for service in _DEFAULT_SERVICE_ORDER:
        if service in services:
            service_order.append(service)
            services.remove(service)
    service_order.extend(sorted(services))

    return data, service_order


def _plot_replica_distribution(
    data: dict[str, dict[int, dict[str, int]]],
    service_order: list[str],
    output_dir: Path,
) -> None:
    policies = sorted(data.keys())
    nrows = len(policies)
    fig, axes = plt.subplots(nrows=nrows, ncols=1, figsize=(12, 4 * nrows))
    if nrows == 1:
        axes = [axes]

    cmap = plt.get_cmap("tab20" if len(service_order) > 10 else "tab10")
    colors = {service: cmap(i % cmap.N) for i, service in enumerate(service_order)}

    for idx, (ax, policy) in enumerate(zip(axes, policies)):
        rps_values = sorted(data[policy].keys())
        x = np.arange(len(rps_values))
        bottom = np.zeros(len(rps_values))
        for service in service_order:
            values = np.array(
                [data[policy][rps].get(service, 0) for rps in rps_values],
                dtype=float,
            )
            if not np.any(values):
                continue
            label = service if idx == 0 else None
            ax.bar(
                x,
                values,
                bottom=bottom,
                width=0.65,
                label=label,
                color=colors[service],
            )
            bottom += values

        ax.set_title(f"{policy} replica distribution by RPS")
        ax.set_ylabel("Replicas")
        ax.set_xticks(x)
        ax.set_xticklabels([str(rps) for rps in rps_values])
        ax.grid(axis="y", linestyle="--", alpha=0.4)
        ax.spines["top"].set_visible(False)
        ax.spines["right"].set_visible(False)

    axes[-1].set_xlabel("Requests Per Second (RPS)")
    if axes[0].get_legend_handles_labels()[0]:
        axes[0].legend(title="Service", ncol=3, fontsize="small")

    output_path = output_dir / "replica_distribution.png"
    fig.tight_layout()
    fig.savefig(output_path, dpi=300)
    plt.close(fig)


def _plot_policy_replica_lines(
    data: dict[str, dict[int, dict[str, int]]],
    output_dir: Path,
) -> None:
    fig, ax = plt.subplots(figsize=(12, 6))
    for policy in sorted(data.keys()):
        rps_values = sorted(data[policy].keys())
        totals = [
            sum(int(v or 0) for v in data[policy][rps].values())
            for rps in rps_values
        ]
        ax.plot(rps_values, totals, marker="o", label=policy)

    ax.set_xlabel("Requests Per Second (RPS)")
    ax.set_ylabel("Total replicas")
    ax.set_ylim(bottom=0)
    ax.set_title("Replica count by policy and RPS")
    ax.grid(axis="y", linestyle="--", alpha=0.4)
    ax.spines["top"].set_visible(False)
    ax.spines["right"].set_visible(False)
    ax.legend()

    output_path = output_dir / "replica_policy_comparison.png"
    fig.tight_layout()
    fig.savefig(output_path, dpi=300)
    plt.close(fig)


def generate_replicas_plots(in_dir: Path, output_dir: Path) -> None:
    data, service_order = _collect_replica_data(in_dir)
    if not data or not service_order:
        return

    output_dir.mkdir(parents=True, exist_ok=True)
    _plot_replica_distribution(data, service_order, output_dir)
    _plot_policy_replica_lines(data, output_dir)

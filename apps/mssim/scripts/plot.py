#!/usr/bin/env python3
"""Compare goodput and latency distributions for FIFO vs priority policies."""
from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, List, Tuple

import matplotlib.pyplot as plt
import pandas as pd
from matplotlib.patches import Patch

THRESHOLD_DEFAULT_MS = 100
FILENAME_PATTERN = re.compile(r"root_latencies_(?P<rps>[0-9_]+)rps\.csv$")
CONFIG_PATH = Path(__file__).resolve().parents[1] / "data/experiment_template.json"


@dataclass
class PolicySamples:
    latencies_ms: pd.DataFrame

    def rps_set(self) -> set[float]:
        return set(self.latencies_ms["rps"].unique())

    def metric_for_rps(self, rps: float, metric: str) -> pd.Series:
        return self.latencies_ms.loc[self.latencies_ms["rps"] == rps, metric]


def parse_rps_from_name(path: Path) -> float:
    match = FILENAME_PATTERN.search(path.name)
    if not match:
        raise ValueError(f"Unexpected filename format: {path}")
    rps_token = match.group("rps").replace("_", ".")
    return float(rps_token)


def parse_rps_from_dir(path: Path) -> float:
    if not path.name.startswith("rps_"):
        raise ValueError(f"Unexpected RPS directory name: {path}")
    token = path.name[len("rps_") :].replace("_", ".")
    return float(token)


def load_policy_samples(policy_dir: Path) -> PolicySamples:
    if not policy_dir.is_dir():
        raise FileNotFoundError(f"Policy directory not found: {policy_dir}")

    frames: List[pd.DataFrame] = []
    empty_rps: List[Path] = []

    for rps_dir in sorted(policy_dir.glob("rps_*")):
        try:
            rps = parse_rps_from_dir(rps_dir)
        except ValueError:
            print(f"Warning: skipping unexpected directory {rps_dir}")
            continue

        run_dirs: Iterable[Path] = sorted(rps_dir.glob("run_*")) or [rps_dir]
        csv_paths: List[Path] = []
        for run_dir in run_dirs:
            csv_paths.extend(sorted(run_dir.glob("root_latencies_*rps.csv")))

        if not csv_paths:
            empty_rps.append(rps_dir)
            continue

        for csv_path in csv_paths:
            df = pd.read_csv(csv_path)
            if "e2e_latency_us" not in df.columns:
                raise ValueError(f"Missing 'e2e_latency_us' column in {csv_path}")

            if "queue_latency_us" in df.columns:
                queue_us = df["queue_latency_us"].astype(float)
            else:
                queue_us = pd.Series(0.0, index=df.index)
            frame = pd.DataFrame(
                {
                    "rps": rps,
                    "e2e_latency_ms": df["e2e_latency_us"].astype(float) / 1_000.0,
                    "queue_latency_ms": queue_us / 1_000.0,
                }
            )
            frames.append(frame)

    for missing in empty_rps:
        print(f"Warning: no samples found for {missing}")

    if not frames:
        raise RuntimeError(f"No latency samples found under {policy_dir}")

    latencies = pd.concat(frames, ignore_index=True)
    return PolicySamples(latencies_ms=latencies)


def align_rps(*datasets: PolicySamples) -> List[float]:
    shared_rps = sorted(set.intersection(*(data.rps_set() for data in datasets)))
    if not shared_rps:
        raise RuntimeError("No overlapping RPS values between fifo and prio datasets.")
    return shared_rps


def compute_goodput(samples: PolicySamples, threshold_ms: float, rps: float) -> float:
    latencies = samples.metric_for_rps(rps, "e2e_latency_ms")
    if latencies.empty:
        return 0.0
    under_fraction = (latencies <= threshold_ms).mean()
    return under_fraction * rps


def plot_goodput(
    rps_values: List[float],
    policy_a_goodput: List[float],
    policy_b_goodput: List[float],
    output: Path,
    threshold_ms: float,
    labels: Tuple[str, str],
) -> None:
    fig, ax = plt.subplots(figsize=(8, 5))
    goodput_fraction_a = [gp / rps for gp, rps in zip(policy_a_goodput, rps_values)]
    goodput_fraction_b = [gp / rps for gp, rps in zip(policy_b_goodput, rps_values)]
    ax.plot(rps_values, goodput_fraction_a, marker="o", label=labels[0])
    ax.plot(rps_values, goodput_fraction_b, marker="s", label=labels[1])
    ax.set_xlabel("Offered load (RPS)")
    ax.set_ylabel("Goodput Fraction (RPS)")
    ax.set_title(f"Goodput fraction vs RPS with SLO={threshold_ms:g} ms")
    ax.grid(True, which="both", linestyle="--", alpha=0.4)
    ax.legend()
    fig.tight_layout()
    output.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(output)
    print(f"Saved comparison plot to {output}")


def build_latency_plots(
    rps_values: List[float],
    policy_a: PolicySamples,
    policy_b: PolicySamples,
    output: Path,
    labels: Tuple[str, str],
) -> None:
    fig, axes = plt.subplots(2, 1, figsize=(10, 8), sharex=True)
    colors = ("#1f77b4", "#ff7f0e")

    metrics = [
        ("End-to-end latency", "e2e_latency_ms"),
        ("Queue latency", "queue_latency_ms"),
    ]

    for ax, (title, column) in zip(axes, metrics):
        positions_a = [idx - 0.2 for idx in range(len(rps_values))]
        positions_b = [idx + 0.2 for idx in range(len(rps_values))]

        bp_a = ax.boxplot(
            [policy_a.metric_for_rps(rps, column).dropna().to_numpy() for rps in rps_values],
            positions=positions_a,
            widths=0.35,
            whis=(5, 99.9),
            patch_artist=True,
            manage_ticks=False,
            showfliers=False,
        )
        bp_b = ax.boxplot(
            [policy_b.metric_for_rps(rps, column).dropna().to_numpy() for rps in rps_values],
            positions=positions_b,
            widths=0.35,
            whis=(5, 99.9),
            patch_artist=True,
            manage_ticks=False,
            showfliers=False,
        )
        
        for patch in bp_a["boxes"]:
            patch.set(facecolor=colors[0], alpha=0.6)
        for patch in bp_b["boxes"]:
            patch.set(facecolor=colors[1], alpha=0.6)

        ax.set_ylabel(f"{title} (ms)")
        ax.set_title(title)
        ax.grid(True, which="both", linestyle="--", alpha=0.3, axis="y")

    axes[-1].set_xticks(range(len(rps_values)))
    axes[-1].set_xticklabels([f"{rps:g}" for rps in rps_values])
    axes[-1].set_xlabel("Offered load (RPS)")

    handles = [
        Patch(facecolor=colors[0], alpha=0.6, label=labels[0]),
        Patch(facecolor=colors[1], alpha=0.6, label=labels[1]),
    ]
    axes[0].legend(handles=handles, loc="upper left")

    fig.tight_layout()
    output.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(output)
    print(f"Saved latency plots to {output}")


def load_plot_config(config_path: Path) -> Tuple[Path, Tuple[str, str], float]:
    with config_path.open("r") as fh:
        config = json.load(fh)

    try:
        experiment_root = Path(config["output_root"]) / config["experiment_name"]
        policies = config["policies"]
    except KeyError as exc:
        raise KeyError(f"Missing key in plot config: {exc}") from exc

    if len(policies) < 2:
        raise ValueError("Plot config must specify at least two policies.")

    threshold_ms = float(config.get("slo_ms", THRESHOLD_DEFAULT_MS))
    return experiment_root, (policies[0], policies[1]), threshold_ms


def main() -> None:
    experiment_root, (policy_a_name, policy_b_name), threshold_ms = load_plot_config(CONFIG_PATH)
    policy_a_dir = experiment_root / policy_a_name
    policy_b_dir = experiment_root / policy_b_name

    policy_a_samples = load_policy_samples(policy_a_dir)
    policy_b_samples = load_policy_samples(policy_b_dir)

    rps_values = align_rps(policy_a_samples, policy_b_samples)

    policy_a_goodput = [
        compute_goodput(policy_a_samples, threshold_ms, rps) for rps in rps_values
    ]
    policy_b_goodput = [
        compute_goodput(policy_b_samples, threshold_ms, rps) for rps in rps_values
    ]

    print("Goodput summary (threshold = {:.1f} ms):".format(threshold_ms))
    print(f"RPS\t{policy_a_name}\t{policy_b_name}")
    for rps, policy_a_val, policy_b_val in zip(rps_values, policy_a_goodput, policy_b_goodput):
        print(f"{rps:g}\t{policy_a_val:.2f}\t{policy_b_val:.2f}")

    plot_goodput(
        rps_values,
        policy_a_goodput,
        policy_b_goodput,
        Path(f"apps/mssim/data/goodput_comparison_{threshold_ms:g}ms.png"),
        threshold_ms,
        (policy_a_name, policy_b_name),
    )
    build_latency_plots(
        rps_values,
        policy_a_samples,
        policy_b_samples,
        Path("apps/mssim/data/latency_boxplot.png"),
        (policy_a_name, policy_b_name),
    )


if __name__ == "__main__":
    main()

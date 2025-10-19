#!/usr/bin/env python3
"""Compare goodput and latency distributions for FIFO vs priority policies."""
from __future__ import annotations

import json
import re
from dataclasses import dataclass
from math import ceil
from pathlib import Path
from typing import Iterable, List, Tuple

import matplotlib.pyplot as plt
import pandas as pd
from matplotlib.patches import Patch

THRESHOLD_DEFAULT_MS = 50
FILENAME_PATTERN = re.compile(r"root_latencies_(?P<rps>[0-9_]+)rps\.csv$")
CONFIG_PATH = Path(__file__).resolve().parents[1] / "data/cfg.json"


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


def compute_goodput(samples: PolicySamples, threshold_ms: float, rps: float, duration: float) -> float:
    latencies = samples.metric_for_rps(rps, "e2e_latency_ms")
    if latencies.empty:
        return 0.0
    goodput = (latencies <= threshold_ms).sum() / duration
    return goodput


def plot_goodput_fraction(
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


def plot_goodput_absolute(
    rps_values: List[float],
    policy_a_goodput: List[float],
    policy_b_goodput: List[float],
    output: Path,
    threshold_ms: float,
    labels: Tuple[str, str],
) -> None:
    fig, ax = plt.subplots(figsize=(8, 5))
    ax.plot(rps_values, policy_a_goodput, marker="o", label=labels[0])
    ax.plot(rps_values, policy_b_goodput, marker="s", label=labels[1])
    ax.set_xlabel("Offered load (RPS)")
    ax.set_ylabel("Goodput (RPS)")
    ax.set_title(f"Goodput vs RPS with SLO={threshold_ms:g} ms")
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


def plot_latency_percentiles(
    rps_values: List[float],
    policies: Sequence[PolicySamples],
    output: Path,
    labels: Sequence[str],
    percentiles: Sequence[float] | None = None,
) -> None:
    if percentiles is None:
        percentiles = (90.0, 95.0, 99.0, 99.9)

    if not policies:
        raise ValueError("No policies provided for percentile plotting.")

    ncols = 2 if len(percentiles) > 1 else 1
    nrows = ceil(len(percentiles) / ncols)
    fig, axes = plt.subplots(nrows, ncols, figsize=(10, 4 * nrows), sharex=True)
    if isinstance(axes, plt.Axes):
        axes_iter = [axes]
    else:
        axes_iter = axes.flatten()

    cmap = plt.get_cmap("tab10")
    marker_cycle = ("o", "s", "^", "D", "P", "X", "*", "v", "<", ">")

    for idx, percentile in enumerate(percentiles):
        if idx >= len(axes_iter):
            break
        ax = axes_iter[idx]
        for policy_idx, (label, samples) in enumerate(zip(labels, policies)):
            marker = marker_cycle[policy_idx % len(marker_cycle)]
            color = cmap(policy_idx % cmap.N)
            tail_latencies = [
                float(samples.metric_for_rps(rps, "e2e_latency_ms").dropna().quantile(percentile / 100.0))
                if not samples.metric_for_rps(rps, "e2e_latency_ms").dropna().empty
                else float("nan")
                for rps in rps_values
            ]
            ax.plot(
                rps_values,
                tail_latencies,
                marker=marker,
                label=label,
                color=color,
            )

        ax.set_title(f"P{percentile:g} tail latency")
        ax.set_ylabel("Latency (ms)")
        ax.grid(True, which="both", linestyle="--", alpha=0.4)
        if idx == 0:
            ax.legend()

    for extra_ax in axes_iter[len(percentiles) :]:
        extra_ax.axis("off")

    for ax in axes_iter[-ncols:]:
        ax.set_xlabel("Offered load (RPS)")

    fig.tight_layout()
    output.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(output)
    print(f"Saved latency percentile plots to {output}")


def plot_latency_percentiles(
    rps_values: List[float],
    policies: Sequence[PolicySamples],
    output: Path,
    labels: Sequence[str],
    percentiles: Sequence[float] | None = None,
) -> None:
    if percentiles is None:
        percentiles = (90.0, 95.0, 99.0, 99.9)

    if not policies:
        raise ValueError("No policies provided for percentile plotting.")

    ncols = 2 if len(percentiles) > 1 else 1
    nrows = ceil(len(percentiles) / ncols)
    fig, axes = plt.subplots(nrows, ncols, figsize=(10, 4 * nrows), sharex=True)
    if isinstance(axes, plt.Axes):
        axes_iter = [axes]
    else:
        axes_iter = axes.flatten()

    cmap = plt.get_cmap("tab10")
    marker_cycle = ("o", "s", "^", "D", "P", "X", "*", "v", "<", ">")

    for idx, percentile in enumerate(percentiles):
        if idx >= len(axes_iter):
            break
        ax = axes_iter[idx]
        for policy_idx, (label, samples) in enumerate(zip(labels, policies)):
            marker = marker_cycle[policy_idx % len(marker_cycle)]
            color = cmap(policy_idx % cmap.N)
            tail_latencies = [
                float(samples.metric_for_rps(rps, "e2e_latency_ms").dropna().quantile(percentile / 100.0))
                if not samples.metric_for_rps(rps, "e2e_latency_ms").dropna().empty
                else float("nan")
                for rps in rps_values
            ]
            ax.plot(
                rps_values,
                tail_latencies,
                marker=marker,
                label=label,
                color=color,
            )

        ax.set_title(f"P{percentile:g} tail latency")
        ax.set_ylabel("Latency (ms)")
        ax.grid(True, which="both", linestyle="--", alpha=0.4)
        if idx == 0:
            ax.legend()

    for extra_ax in axes_iter[len(percentiles) :]:
        extra_ax.axis("off")

    for ax in axes_iter[-ncols:]:
        ax.set_xlabel("Offered load (RPS)")

    fig.tight_layout()
    output.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(output)
    print(f"Saved latency percentile plots to {output}")


def load_plot_config(config_path: Path) -> Tuple[Path, Tuple[str, str], float]:
    with config_path.open("r") as fh:
        config = json.load(fh)

    try:
        experiment_root = Path(config["output_root"]) / config["experiment_name"]
        policies = config["policies"]
        duration = config["duration_sec"]
    except KeyError as exc:
        raise KeyError(f"Missing key in plot config: {exc}") from exc

    if len(policies) < 2:
        raise ValueError("Plot config must specify at least two policies.")

    threshold_ms = float(config.get("slo_ms", THRESHOLD_DEFAULT_MS))
    return experiment_root, (policies[0], policies[1]), threshold_ms, duration


def main() -> None:
    experiment_root, policy_names, threshold_ms, duration = load_plot_config(CONFIG_PATH)
    policy_dirs = [experiment_root / name for name in policy_names]
    policy_samples = [load_policy_samples(policy_dir) for policy_dir in policy_dirs]

    rps_values = align_rps(*policy_samples)

    
    goodput_by_policy = [
        [compute_goodput(samples, threshold_ms, rps, duration) for rps in rps_values]
        for samples in policy_samples
    ]

    print("Goodput summary (threshold = {:.1f} ms):".format(threshold_ms))
    header = "\t".join(["RPS", *policy_names])
    print(header)
    for idx, rps in enumerate(rps_values):
        row = "\t".join(f"{goodput[idx]:.2f}" for goodput in goodput_by_policy)
        print(f"{rps:g}\t{row}")

    plot_goodput_fraction(
        rps_values,
        goodput_by_policy,
        Path(f"apps/mssim/data/goodput_fraction_{threshold_ms:g}ms.png"),
        threshold_ms,
        policy_names,
    )
    plot_goodput_absolute(
        rps_values,
        goodput_by_policy,
        Path(f"apps/mssim/data/goodput_absolute_{threshold_ms:g}ms.png"),
        threshold_ms,
        policy_names,
    )
    build_latency_plots(
        rps_values,
        policy_samples,
        Path("apps/mssim/data/latency_boxplot.png"),
        policy_names,
    )
    plot_latency_percentiles(
        rps_values,
        policy_samples,
        Path("apps/mssim/data/latency_percentiles.png"),
        policy_names,
    )


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Compare goodput and latency distributions for one or more scheduling policies."""
from __future__ import annotations

import argparse
import json
import re
from dataclasses import dataclass
from math import ceil
from pathlib import Path
from typing import Iterable, List, Sequence, Tuple

import matplotlib.pyplot as plt
import pandas as pd
import numpy as np
from matplotlib.patches import Patch

THRESHOLD_DEFAULT_MS = 50
FILENAME_PATTERN = re.compile(r"root_latencies_(?P<rps>[0-9_]+)rps\.csv$")
CONFIG_PATH = Path(__file__).resolve().parents[1] / "data/shared.json"
QUEUE_CDF_RPS_DEFAULT = 100.0
PERCENTILE = 0.9


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


WARMUP_COL = "start_at"


def filter_after_warmup(
    df: pd.DataFrame, warmup_sec: float, source: Path
) -> pd.DataFrame:
    if warmup_sec <= 0:
        return df
    if WARMUP_COL not in df.columns:
        print(
            f"Warning: warmup filtering requested but '{WARMUP_COL}' column is missing in {source}"
        )
        return df

    start_at = pd.to_numeric(df[WARMUP_COL], errors="coerce")
    valid_start = start_at.dropna()
    if valid_start.empty:
        print(
            f"Warning: warmup filtering skipped because '{WARMUP_COL}' contains no valid values in {source}"
        )
        return df

    warmup_us = warmup_sec * 1_000_000.0
    measurement_anchor = valid_start.min()
    relative_us = start_at - measurement_anchor
    mask = relative_us >= warmup_us
    mask = mask.fillna(True)
    filtered = df.loc[mask]

    if filtered.empty:
        print(
            f"Warning: warmup filtering removed all samples from {source}; keeping unfiltered data."
        )
        return df
    return filtered


def load_policy_samples(policy_dir: Path, warmup_sec: float = 0.0) -> PolicySamples:
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
            df.columns = [col.strip() for col in df.columns]
            if "e2e_latency_us" not in df.columns:
                raise ValueError(f"Missing 'e2e_latency_us' column in {csv_path}")
            df = filter_after_warmup(df, warmup_sec, csv_path)
            if "is_err" in df.columns:
                err_mask = df["is_err"].fillna(False).astype(bool)
                df = df.loc[~err_mask]

            if df.empty:
                continue

            if "queue_latency_us" in df.columns:
                queue_us = df["queue_latency_us"].astype(float)
            else:
                queue_us = pd.Series(0.0, index=df.index)
            if "graph" in df.columns:
                graph_values = df["graph"].astype(str).where(df["graph"].notna(), "unknown")
            else:
                graph_values = pd.Series("unknown", index=df.index)

            if "missed_slo" in df.columns:
                raw_missed = df["missed_slo"]
                if pd.api.types.is_bool_dtype(raw_missed):
                    missed_slo = raw_missed.astype("boolean")
                else:
                    normalized = (
                        raw_missed.astype(str)
                        .str.strip()
                        .str.lower()
                        .replace({"": pd.NA})
                    )
                    bool_mapped = normalized.replace(
                        {"true": True, "1": True, "yes": True, "false": False, "0": False, "no": False}
                    )
                    bool_mapped = bool_mapped.where(
                        bool_mapped.isin([True, False]), pd.NA
                    )
                    missed_slo = bool_mapped.astype("boolean")
            else:
                missed_slo = pd.Series(pd.NA, index=df.index, dtype="boolean")

            frame = pd.DataFrame(
                {
                    "graph": graph_values,
                    "rps": rps,
                    "e2e_latency_ms": df["e2e_latency_us"].astype(float) / 1_000.0,
                    "queue_latency_ms": queue_us / 1_000.0,
                    "missed_slo": missed_slo,
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
        raise RuntimeError("No overlapping RPS values across the provided policy datasets.")
    return shared_rps


def compute_goodput(samples: PolicySamples, threshold_ms: float, rps: float, duration: float, warmup: float) -> float:
    subset = samples.latencies_ms.loc[samples.latencies_ms["rps"] == rps]
    if subset.empty:
        return 0.0
    effective_duration = duration - warmup
    if effective_duration <= 0:
        raise ValueError("Duration minus warmup must be positive to compute goodput.")

    good_requests = (subset["e2e_latency_ms"] <= threshold_ms).sum()

    return good_requests / effective_duration


def compute_goodput_by_graph(
    samples: PolicySamples,
    rps_values: Sequence[float],
    threshold_ms: float,
    duration: float,
    warmup: float
) -> dict[str, List[float]]:
    effective_duration = duration - warmup
    if effective_duration <= 0:
        raise ValueError("Duration minus warmup must be positive to compute goodput.")

    graphs = sorted(
        {str(graph) for graph in samples.latencies_ms["graph"].dropna().unique()}
    )
    if not graphs:
        graphs = ["unknown"]

    goodput_per_graph: dict[str, List[float]] = {graph: [] for graph in graphs}

    for rps in rps_values:
        subset = samples.latencies_ms.loc[samples.latencies_ms["rps"] == rps]
        if subset.empty:
            for graph in graphs:
                goodput_per_graph[graph].append(float("nan"))
            continue

        grouped = {
            str(graph): group for graph, group in subset.groupby("graph", dropna=False)
        }
        for graph in graphs:
            group = grouped.get(graph)
            if group is None or group.empty:
                goodput_per_graph[graph].append(float("nan"))
                continue

            meets_slo: int
            meets_slo = (group["e2e_latency_ms"] <= threshold_ms).sum()

            goodput_per_graph[graph].append(meets_slo / effective_duration)

    return goodput_per_graph


def compute_queue_latency_by_graph(
    samples: PolicySamples, rps_values: Sequence[float]
) -> dict[str, List[float]]:
    """Average queue latency per graph, per RPS, after warmup filtering."""
    graphs = sorted(
        {str(graph) for graph in samples.latencies_ms["graph"].dropna().unique()}
    )
    if not graphs:
        graphs = ["unknown"]

    mean_queue_latency_per_graph: dict[str, List[float]] = {graph: [] for graph in graphs}
    p90_queue_latency_per_graph: dict[str, List[float]] = {graph: [] for graph in graphs}

    for rps in rps_values:
        subset = samples.latencies_ms.loc[samples.latencies_ms["rps"] == rps]
        if subset.empty:
            for graph in graphs:
                mean_queue_latency_per_graph[graph].append(float("nan"))
                p90_queue_latency_per_graph[graph].append(float("nan"))
            continue

        for graph in graphs:
            graph_mask = subset["graph"] == graph
            graph_latencies = subset.loc[graph_mask, "queue_latency_ms"].dropna()
            if graph_latencies.empty:
                mean_queue_latency_per_graph[graph].append(float("nan"))
                p90_queue_latency_per_graph[graph].append(float("nan"))
            else:
                mean_queue_latency_per_graph[graph].append(float(graph_latencies.mean()))
                p90_queue_latency_per_graph[graph].append(float(graph_latencies.quantile(PERCENTILE)))


    return mean_queue_latency_per_graph, p90_queue_latency_per_graph


def sanitize_label_for_filename(label: str) -> str:
    sanitized = re.sub(r"[^A-Za-z0-9._-]+", "_", label).strip("_")
    return sanitized or "policy"


def plot_goodput_fraction(
    rps_values: Sequence[float],
    goodput_by_policy: Sequence[Sequence[float]],
    output: Path,
    threshold_ms: float,
    labels: Sequence[str],
) -> None:
    if len(labels) != len(goodput_by_policy):
        raise ValueError("Number of labels must match number of policy goodput vectors.")

    fig, ax = plt.subplots(figsize=(8, 5))
    cmap = plt.get_cmap("tab10")
    marker_cycle = ("o", "s", "^", "D", "P", "X", "*", "v", "<", ">")

    for idx, (label, goodput_values) in enumerate(zip(labels, goodput_by_policy)):
        marker = marker_cycle[idx % len(marker_cycle)]
        color = cmap(idx % cmap.N)
        fractions = [
            (goodput / rps) if rps else float("nan")
            for goodput, rps in zip(goodput_values, rps_values)
        ]
        ax.plot(rps_values, fractions, marker=marker, label=label, color=color)

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
    rps_values: Sequence[float],
    goodput_by_policy: Sequence[Sequence[float]],
    output: Path,
    threshold_ms: float,
    labels: Sequence[str],
) -> None:
    if len(labels) != len(goodput_by_policy):
        raise ValueError("Number of labels must match number of policy goodput vectors.")

    fig, ax = plt.subplots(figsize=(8, 5))
    marker_cycle = ("o", "s", "^", "D", "P", "X", "*", "v", "<", ">")
    cmap = plt.get_cmap("tab10")

    for idx, (label, goodput_values) in enumerate(zip(labels, goodput_by_policy)):
        marker = marker_cycle[idx % len(marker_cycle)]
        color = cmap(idx % cmap.N)
        ax.plot(rps_values, goodput_values, marker=marker, label=label, color=color)

    ax.set_xlabel("Offered load (RPS)")
    ax.set_ylabel("Goodput (RPS)")
    ax.set_title(f"Goodput vs RPS with SLO={threshold_ms:g} ms")
    ax.grid(True, which="both", linestyle="--", alpha=0.4)
    ax.legend()
    fig.tight_layout()
    output.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(output)
    print(f"Saved comparison plot to {output}")


def plot_goodput_by_graph(
    rps_values: Sequence[float],
    per_policy_graph_goodput: Sequence[dict[str, Sequence[float]]],
    output_dir: Path,
    threshold_ms: float,
    labels: Sequence[str],
) -> None:
    if len(labels) != len(per_policy_graph_goodput):
        raise ValueError("Number of labels must match number of graph-goodput mappings.")

    marker_cycle = ("o", "s", "^", "D", "P", "X", "*", "v", "<", ">")

    for policy_idx, (label, graph_map) in enumerate(
        zip(labels, per_policy_graph_goodput)
    ):
        if not graph_map:
            print(f"Warning: no per-graph data available for {label}")
            continue

        fig, ax = plt.subplots(figsize=(8, 5))
        cmap = plt.get_cmap("tab20")
        sorted_graphs = sorted(graph_map.keys())

        for graph_idx, graph in enumerate(sorted_graphs):
            color = cmap(graph_idx % cmap.N)
            marker = marker_cycle[graph_idx % len(marker_cycle)]
            ax.plot(
                rps_values,
                graph_map[graph],
                marker=marker,
                color=color,
                label=graph,
            )

        ax.set_xlabel("Offered load (RPS)")
        ax.set_ylabel("Goodput (RPS)")
        ax.set_title(f"{label}: per-graph goodput with SLO={threshold_ms:g} ms")
        ax.grid(True, which="both", linestyle="--", alpha=0.4)
        ax.legend(loc="best")
        fig.tight_layout()

        output_dir.mkdir(parents=True, exist_ok=True)
        filename = (
            f"{sanitize_label_for_filename(label)}_goodput_by_graph_{threshold_ms:g}ms.png"
        )
        output_path = output_dir / filename
        fig.savefig(output_path)
        print(f"Saved per-graph goodput plot for {label} to {output_path}")


def plot_queue_latency_cdf(
    policies: Sequence[PolicySamples],
    labels: Sequence[str],
    target_rps: float,
    rps_values: Sequence[float],
    output: Path,
) -> None:
    if not policies:
        raise ValueError("At least one policy is required for queue latency CDF plots.")
    if len(labels) != len(policies):
        raise ValueError("Number of labels must match number of policy datasets.")
    if not rps_values:
        raise ValueError("No RPS values available for queue latency CDF.")

    selected_rps = min(rps_values, key=lambda rps: abs(rps - target_rps))
    if abs(selected_rps - target_rps) > 1e-3:
        print(
            f"Requested queue-latency CDF at {target_rps:g} RPS, using closest available RPS value {selected_rps:g}."
        )

    fig, ax = plt.subplots(figsize=(8, 5))
    cmap = plt.get_cmap("tab10")

    any_data = False
    for idx, (label, samples) in enumerate(zip(labels, policies)):
        series = samples.metric_for_rps(selected_rps, "queue_latency_ms").dropna()
        if series.empty:
            print(
                f"Warning: no queue latency samples for {label} at {selected_rps:g} RPS; skipping."
            )
            continue
        values = np.sort(series.to_numpy())
        cdf = (np.arange(1, len(values) + 1) / len(values)).astype(float)
        color = cmap(idx % cmap.N)
        ax.step(values, cdf, where="post", label=label, color=color)
        any_data = True

    if not any_data:
        print(
            f"Warning: skipped queue latency CDF plot because no samples were available at {selected_rps:g} RPS."
        )
        plt.close(fig)
        return

    ax.set_yscale("log")
    ax.set_xlabel("Queue latency (ms)")
    ax.set_ylabel("CDF")
    ax.set_title(f"Queue latency CDF at {selected_rps:g} RPS")
    ax.grid(True, which="both", linestyle="--", alpha=0.4)
    ax.legend(loc="lower right")

    fig.tight_layout()
    output.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(output)
    print(f"Saved queue latency CDF plot to {output}")


def print_queue_latency_summary(
    policy_names: Sequence[str],
    rps_values: Sequence[float],
    mean_queue_latency_data: Sequence[dict[str, Sequence[float]]],
    p90_queue_latency_data: Sequence[dict[str, Sequence[float]]],
) -> None:
    """Print a readable table of queue latency per graph, per policy."""
    print("\nQueue latency (ms) per graph after warmup:")
    for policy_name, mean_graph_map, p90_graph_map in zip(policy_names, mean_queue_latency_data, p90_queue_latency_data):
        print(f"Policy: {policy_name}")
        if not mean_graph_map or not p90_graph_map:
            print("  No queue latency data available.")
            continue

        for graph in sorted(mean_graph_map.keys()):
            mean_latency_values = mean_graph_map[graph]
            p90_latency_values = p90_graph_map[graph]
            segments = []
            for rps, mean_latency, p90_latency in zip(rps_values, mean_latency_values, p90_latency_values):
                if pd.isna(mean_latency) or pd.isna(p90_latency):
                    value_repr = "N/A"
                else:
                    value_repr = f"{mean_latency:.2f} ms (P{PERCENTILE*100:.0f}: {p90_latency:.2f} ms)"
                segments.append(f"{rps:g} RPS={value_repr}")
            joined = ", ".join(segments)
            print(f"  {graph}: {joined}")


def build_latency_plots(
    rps_values: Sequence[float],
    policies: Sequence[PolicySamples],
    output: Path,
    labels: Sequence[str],
) -> None:
    if not policies:
        raise ValueError("At least one policy is required for latency plots.")
    if len(labels) != len(policies):
        raise ValueError("Number of labels must match number of policy datasets.")

    fig, axes = plt.subplots(2, 1, figsize=(10, 8), sharex=True)
    cmap = plt.get_cmap("tab10")
    num_policies = len(policies)
    if num_policies == 0:
        raise ValueError("No policies provided for latency plotting.")

    colors = [cmap(idx % cmap.N) for idx in range(num_policies)]
    group_width = 0.8
    box_width = group_width / max(num_policies, 1)
    offsets = [
        (idx - (num_policies - 1) / 2.0) * box_width for idx in range(num_policies)
    ]

    metrics = [
        ("End-to-end latency", "e2e_latency_ms"),
        ("Queue latency", "queue_latency_ms"),
    ]

    for ax, (title, column) in zip(axes, metrics):
        base_positions = list(range(len(rps_values)))

        for policy_idx, (label, samples) in enumerate(zip(labels, policies)):
            positions = [pos + offsets[policy_idx] for pos in base_positions]
            bp = ax.boxplot(
                [
                    samples.metric_for_rps(rps, column).dropna().to_numpy()
                    for rps in rps_values
                ],
                positions=positions,
                widths=box_width * 0.9,
                whis=(5, 99.9999),
                patch_artist=True,
                manage_ticks=False,
                showfliers=False,
            )

            for patch in bp["boxes"]:
                patch.set(facecolor=colors[policy_idx], alpha=0.6)
            for median in bp["medians"]:
                median.set(color=colors[policy_idx])

        ax.set_ylabel(f"{title} (ms)")
        ax.set_title(title)
        ax.grid(True, which="both", linestyle="--", alpha=0.3, axis="y")

    axes[-1].set_xticks(range(len(rps_values)))
    axes[-1].set_xticklabels([f"{rps:g}" for rps in rps_values])
    axes[-1].set_xlabel("Offered load (RPS)")

    handles = [
        Patch(facecolor=colors[idx], alpha=0.6, label=label)
        for idx, label in enumerate(labels)
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
    ylim_max_ms: float | None = None,
) -> None:
    if percentiles is None:
        percentiles = (50, 90.0, 95.0, 99.0)

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
        if ylim_max_ms is not None:
            ax.set_ylim(bottom=0, top=ylim_max_ms)
        else:
            ax.set_ylim(bottom=0)
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


def plot_latency_percentiles_by_graph(
    rps_values: List[float],
    policies: Sequence[PolicySamples],
    output_dir: Path,
    labels: Sequence[str],
    percentiles: Sequence[float] | None = None,
    ylim_max_ms: float | None = None,
) -> None:
    """Plot latency percentiles by graph, comparing how the same graph's percentiles vary across policies."""
    if percentiles is None:
        percentiles = (50, 90.0, 95.0, 99.0)

    if not policies:
        raise ValueError("No policies provided for percentile plotting.")
    if len(labels) != len(policies):
        raise ValueError("Number of labels must match number of policy datasets.")

    # Collect all unique graphs across all policies
    all_graphs = set()
    for samples in policies:
        graphs = samples.latencies_ms["graph"].dropna().unique()
        all_graphs.update(str(g) for g in graphs)
    
    if not all_graphs:
        all_graphs = {"unknown"}
    
    sorted_graphs = sorted(all_graphs)

    cmap = plt.get_cmap("tab10")
    marker_cycle = ("o", "s", "^", "D", "P", "X", "*", "v", "<", ">")

    output_dir.mkdir(parents=True, exist_ok=True)

    for graph_name in sorted_graphs:
        # Create subplots for each percentile
        ncols = 2 if len(percentiles) > 1 else 1
        nrows = ceil(len(percentiles) / ncols)
        fig, axes = plt.subplots(nrows, ncols, figsize=(12, 4 * nrows), sharex=True)
        if isinstance(axes, plt.Axes):
            axes_iter = [axes]
        else:
            axes_iter = axes.flatten()

        for idx, percentile in enumerate(percentiles):
            if idx >= len(axes_iter):
                break
            ax = axes_iter[idx]

            # Plot each policy for this graph and percentile
            for policy_idx, (label, samples) in enumerate(zip(labels, policies)):
                # Filter data for this graph and RPS
                graph_data = samples.latencies_ms[
                    samples.latencies_ms["graph"].astype(str) == graph_name
                ]
                
                if graph_data.empty:
                    continue

                tail_latencies = []
                for rps in rps_values:
                    rps_graph_data = graph_data[
                        graph_data["rps"] == rps
                    ]["e2e_latency_ms"].dropna()
                    
                    if rps_graph_data.empty:
                        tail_latencies.append(float("nan"))
                    else:
                        tail_latencies.append(
                            float(rps_graph_data.quantile(percentile / 100.0))
                        )

                marker = marker_cycle[policy_idx % len(marker_cycle)]
                color = cmap(policy_idx % cmap.N)
                
                ax.plot(
                    rps_values,
                    tail_latencies,
                    marker=marker,
                    label=f"{label}",
                    color=color,
                    linewidth=2,
                )

            ax.set_title(f"P{percentile:g} tail latency")
            ax.set_ylabel("Latency (ms)")
            ax.grid(True, which="both", linestyle="--", alpha=0.4)
            if ylim_max_ms is not None:
                ax.set_ylim(bottom=0, top=ylim_max_ms)
            else:
                ax.set_ylim(bottom=0)
            if idx == 0:
                ax.legend(loc="best")

        for extra_ax in axes_iter[len(percentiles) :]:
            extra_ax.axis("off")

        for ax in axes_iter[-ncols:]:
            ax.set_xlabel("Offered load (RPS)")

        fig.suptitle(f"Latency percentiles by policy - Graph: {graph_name}", fontsize=14, y=1.0)
        fig.tight_layout()

        sanitized_graph = sanitize_label_for_filename(graph_name)
        filename = f"latency_percentiles_by_graph_{sanitized_graph}.png"
        output_path = output_dir / filename
        fig.savefig(output_path, bbox_inches="tight")
        print(f"Saved latency percentile plots for graph '{graph_name}' to {output_path}")


def load_plot_config(config_path: Path) -> Tuple[Path, List[str], float, float, float, float]:
    with config_path.open("r") as fh:
        config = json.load(fh)

    try:
        experiment_root = Path(config["output_root"]) / config["experiment_name"]
        policies = config["policies_to_plot"]
        _ = config["duration_sec"]
    except KeyError as exc:
        raise KeyError(f"Missing key in plot config: {exc}") from exc

    if not policies:
        raise ValueError("Plot config must specify at least one policy.")

    threshold_ms = float(config.get("slo_ms", THRESHOLD_DEFAULT_MS))
    if "duration_sec" not in config:
        raise ValueError("Plot config must specify 'duration_sec'.")

    try:
        duration_sec = float(config["duration_sec"])
    except (TypeError, ValueError) as exc:
        raise ValueError("Plot config 'duration_sec' must be numeric.") from exc

    if duration_sec <= 0:
        raise ValueError("Plot config 'duration_sec' must be positive.")

    try:
        warmup_sec = float(config.get("warmup_sec", 0.0))
    except (TypeError, ValueError) as exc:
        raise ValueError("Plot config 'warmup_sec' must be numeric if provided.") from exc
    if warmup_sec < 0:
        raise ValueError("Plot config 'warmup_sec' cannot be negative.")

    try:
        queue_cdf_rps = float(config.get("queue_cdf_rps", QUEUE_CDF_RPS_DEFAULT))
    except (TypeError, ValueError) as exc:
        raise ValueError("Plot config 'queue_cdf_rps' must be numeric if provided.") from exc
    if queue_cdf_rps <= 0:
        raise ValueError("'queue_cdf_rps' must be positive.")

    return (
        experiment_root,
        list(policies),
        threshold_ms,
        duration_sec,
        warmup_sec,
        queue_cdf_rps,
    )


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Compare goodput and latency distributions for one or more scheduling policies."
    )
    parser.add_argument(
        "--config",
        type=Path,
        default=DEFAULT_CONFIG_PATH,
        help=f"Path to the plot configuration JSON file (default: {DEFAULT_CONFIG_PATH})",
    )
    args = parser.parse_args()

    config_path = args.config.resolve()
    if not config_path.exists():
        raise FileNotFoundError(f"Config file not found: {config_path}")

    experiment_root, policy_names, threshold_ms, duration_sec = load_plot_config(
        config_path
    )
    if warmup_sec > 0:
        print(f"Ignoring the first {warmup_sec:g} seconds of samples for warmup.")
    policy_dirs = [experiment_root / name for name in policy_names]
    policy_samples = [
        load_policy_samples(policy_dir, warmup_sec) for policy_dir in policy_dirs
    ]

    rps_values = align_rps(*policy_samples)

    goodput_by_policy = [
        [compute_goodput(samples, threshold_ms, rps, duration_sec, warmup_sec) for rps in rps_values]
        for samples in policy_samples
    ]
    per_policy_graph_goodput = [
        compute_goodput_by_graph(samples, rps_values, threshold_ms, duration_sec, warmup_sec)
        for samples in policy_samples
    ]
    mean_queue_latency_per_policy, p90_queue_latency_per_policy = zip(
        *[compute_queue_latency_by_graph(samples, rps_values) for samples in policy_samples]
    )

    print("Goodput summary (threshold = {:.1f} ms):".format(threshold_ms))
    header = "\t".join(["RPS", *policy_names])
    print(header)
    for idx, rps in enumerate(rps_values):
        row = "\t".join(f"{goodput[idx]:.2f}" for goodput in goodput_by_policy)
        print(f"{rps:g}\t{row}")
    print_queue_latency_summary(policy_names, rps_values, mean_queue_latency_per_policy, p90_queue_latency_per_policy)

    plot_goodput_fraction(
        rps_values,
        goodput_by_policy,
        experiment_root / f"goodput_fraction_{threshold_ms:g}ms.png",
        threshold_ms,
        policy_names,
    )
    plot_goodput_absolute(
        rps_values,
        goodput_by_policy,
        experiment_root / f"goodput_absolute_{threshold_ms:g}ms.png",
        threshold_ms,
        policy_names,
    )
    plot_goodput_by_graph(
        rps_values,
        per_policy_graph_goodput,
        experiment_root,
        threshold_ms,
        policy_names,
    )
    build_latency_plots(
        rps_values,
        policy_samples,
        experiment_root / "latency_boxplot.png",
        policy_names,
    )
    plot_latency_percentiles(
        rps_values,
        policy_samples,
        experiment_root / "latency_percentiles.png",
        policy_names,
    )
    plot_latency_percentiles(
        rps_values,
        policy_samples,
        experiment_root / "latency_percentiles_1s.png",
        policy_names,
        ylim_max_ms=1_000.0,
    )
    plot_latency_percentiles_by_graph(
        rps_values,
        policy_samples,
        experiment_root,
        policy_names,
    )
    plot_queue_latency_cdf(
        policy_samples,
        policy_names,
        queue_cdf_rps,
        rps_values,
        experiment_root / f"queue_latency_cdf_{queue_cdf_rps:g}rps.png",
    )


if __name__ == "__main__":
    main()

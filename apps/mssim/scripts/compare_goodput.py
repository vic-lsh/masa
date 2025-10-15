#!/usr/bin/env python3
"""Compare goodput and latency distributions for FIFO vs priority policies."""
from __future__ import annotations

import argparse
import csv
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, Iterable, List, Tuple

import matplotlib.pyplot as plt
from matplotlib.patches import Patch

THRESHOLD_DEFAULT_MS = 100
FILENAME_PATTERN = re.compile(r"root_latencies_(?P<rps>[0-9_]+)rps\.csv$")


@dataclass
class PolicySamples:
    e2e_ms: Dict[float, List[float]]
    queue_ms: Dict[float, List[float]]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--experiment-root",
        type=Path,
        default=Path("apps/mssim/data/experiments/experiment-1"),
        help=(
            "Directory containing experiment outputs (e.g. apps/mssim/data/experiments/experiment-1)."
        ),
    )
    parser.add_argument(
        "--policy-a",
        default="fifo_queue_tracing",
        help="Name of the first policy directory under the experiment root (default: fifo_queue_tracing)",
    )
    parser.add_argument(
        "--policy-b",
        default="prio_global_queue_tracing",
        help="Name of the second policy directory under the experiment root (default: prio_global_queue_tracing)",
    )
    parser.add_argument(
        "--threshold-ms",
        type=float,
        default=THRESHOLD_DEFAULT_MS,
        help="Latency threshold (milliseconds) used to calculate goodput.",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path(f"apps/mssim/data/goodput_comparison_{THRESHOLD_DEFAULT_MS}ms.png"),
        help="Path to save the goodput comparison plot.",
    )
    parser.add_argument(
        "--boxplot-output",
        type=Path,
        default=Path("apps/mssim/data/latency_boxplot.png"),
        help="Path to save the latency percentile boxplot.",
    )
    parser.add_argument(
        "--show",
        action="store_true",
        help="Display the plots interactively after saving them.",
    )
    return parser.parse_args()


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

    e2e: Dict[float, List[float]] = {}
    queue: Dict[float, List[float]] = {}

    for rps_dir in sorted(policy_dir.glob("rps_*")):
        try:
            rps = parse_rps_from_dir(rps_dir)
        except ValueError:
            print(f"Warning: skipping unexpected directory {rps_dir}")
            continue

        e2e_samples: List[float] = []
        queue_samples: List[float] = []

        run_dirs: Iterable[Path] = sorted(rps_dir.glob("run_*")) or [rps_dir]
        for run_dir in run_dirs:
            for csv_path in sorted(run_dir.glob("root_latencies_*rps.csv")):
                with csv_path.open("r", newline="") as fh:
                    reader = csv.DictReader(fh)
                    for row in reader:
                        try:
                            e2e_samples.append(float(row["e2e_latency_us"]) / 1_000.0)
                            queue_raw = row.get("queue_latency_us", "0")
                            queue_val = float(queue_raw) / 1_000.0 if queue_raw else 0.0
                            queue_samples.append(queue_val)
                        except (ValueError, KeyError) as exc:
                            raise ValueError(
                                f"Malformed row in {csv_path}: {row}"
                            ) from exc

        if e2e_samples:
            e2e[rps] = e2e_samples
            queue[rps] = queue_samples
        else:
            print(f"Warning: no samples found for {rps_dir}")

    if not e2e:
        raise RuntimeError(f"No latency samples found under {policy_dir}")

    return PolicySamples(e2e_ms=e2e, queue_ms=queue)


def align_rps(*datasets: PolicySamples) -> List[float]:
    shared_rps = sorted(set.intersection(*(set(data.e2e_ms.keys()) for data in datasets)))
    if not shared_rps:
        raise RuntimeError("No overlapping RPS values between fifo and prio datasets.")
    return shared_rps


def compute_goodput(latencies_ms: List[float], threshold_ms: float, rps: float) -> float:
    if not latencies_ms:
        return 0.0
    under = sum(1 for latency in latencies_ms if latency <= threshold_ms)
    return (under / len(latencies_ms)) * rps


def plot_goodput(
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
        ("End-to-end latency", policy_a.e2e_ms, policy_b.e2e_ms),
        ("Queue latency", policy_a.queue_ms, policy_b.queue_ms),
    ]

    for ax, (title, policy_a_data, policy_b_data) in zip(axes, metrics):
        positions_a = [idx - 0.2 for idx in range(len(rps_values))]
        positions_b = [idx + 0.2 for idx in range(len(rps_values))]

        bp_a = ax.boxplot(
            [policy_a_data[rps] for rps in rps_values],
            positions=positions_a,
            widths=0.35,
            whis=(5, 99.9),
            patch_artist=True,
            manage_ticks=False,
            showfliers=False,
        )
        bp_b = ax.boxplot(
            [policy_b_data[rps] for rps in rps_values],
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


def main() -> None:
    args = parse_args()
    policy_a_dir = args.experiment_root / args.policy_a
    policy_b_dir = args.experiment_root / args.policy_b

    policy_a_samples = load_policy_samples(policy_a_dir)
    policy_b_samples = load_policy_samples(policy_b_dir)

    rps_values = align_rps(policy_a_samples, policy_b_samples)
    policy_a_goodput = [
        compute_goodput(policy_a_samples.e2e_ms[rps], args.threshold_ms, rps) for rps in rps_values
    ]
    policy_b_goodput = [
        compute_goodput(policy_b_samples.e2e_ms[rps], args.threshold_ms, rps) for rps in rps_values
    ]

    print("Goodput summary (threshold = {:.1f} ms):".format(args.threshold_ms))
    print(f"RPS\t{args.policy_a}\t{args.policy_b}")
    for rps, policy_a_val, policy_b_val in zip(rps_values, policy_a_goodput, policy_b_goodput):
        print(f"{rps:g}\t{policy_a_val:.2f}\t{policy_b_val:.2f}")

    plot_goodput(
        rps_values,
        policy_a_goodput,
        policy_b_goodput,
        args.output,
        args.threshold_ms,
        (args.policy_a, args.policy_b),
    )
    build_latency_plots(
        rps_values,
        policy_a_samples,
        policy_b_samples,
        args.boxplot_output,
        (args.policy_a, args.policy_b),
    )

    if args.show:
        plt.show()


if __name__ == "__main__":
    main()

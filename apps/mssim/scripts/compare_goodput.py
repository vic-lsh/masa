#!/usr/bin/env python3
"""Compare goodput and latency distributions for FIFO vs priority policies."""
from __future__ import annotations

import argparse
import csv
import re
from pathlib import Path
from typing import Dict, List

import matplotlib.pyplot as plt
from matplotlib.patches import Patch

THRESHOLD_DEFAULT_MS = 50.0
FILENAME_PATTERN = re.compile(r"root_latencies_(?P<rps>[0-9_]+)rps\.csv$")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--data-root",
        type=Path,
        default=Path("apps/mssim/data"),
        help="Base directory containing fifo/ and prio/ policy subdirectories.",
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


def load_latency_samples(directory: Path) -> Dict[float, List[float]]:
    samples: Dict[float, List[float]] = {}
    for csv_path in sorted(directory.glob("root_latencies_*rps.csv")):
        rps = parse_rps_from_name(csv_path)
        with csv_path.open("r", newline="") as fh:
            reader = csv.DictReader(fh)
            latencies_ms: List[float] = []
            for row in reader:
                try:
                    latency_us = float(row["e2e_latency_us"])
                except (ValueError, KeyError) as exc:
                    raise ValueError(f"Malformed row in {csv_path}: {row}") from exc
                latencies_ms.append(latency_us / 1_000.0)
        if not latencies_ms:
            print(f"Warning: {csv_path} contained no samples; skipping.")
            continue
        samples[rps] = latencies_ms
    return samples


def align_rps(*datasets: Dict[float, List[float]]) -> List[float]:
    shared_rps = sorted(set.intersection(*(set(data.keys()) for data in datasets)))
    if not shared_rps:
        raise RuntimeError("No overlapping RPS values between fifo and prio datasets.")
    return shared_rps


def compute_goodput(latencies_ms: List[float], threshold_ms: float, rps: float) -> float:
    if not latencies_ms:
        return 0.0
    under = sum(1 for latency in latencies_ms if latency <= threshold_ms)
    return (under / len(latencies_ms)) * rps


def plot_goodput(
    rps_values: List[float], fifo_goodput: List[float], prio_goodput: List[float], output: Path
) -> None:
    fig, ax = plt.subplots(figsize=(8, 5))
    ax.plot(rps_values, fifo_goodput, marker="o", label="FIFO policy")
    ax.plot(rps_values, prio_goodput, marker="s", label="Priority policy")
    ax.set_xlabel("Offered load (RPS)")
    ax.set_ylabel("Goodput (RPS)")
    ax.set_title(f"Goodput vs RPS with SLO={THRESHOLD_DEFAULT_MS} ms")
    ax.grid(True, which="both", linestyle="--", alpha=0.4)
    ax.legend()
    fig.tight_layout()
    output.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(output)
    print(f"Saved comparison plot to {output}")


def build_boxplot(
    rps_values: List[float],
    fifo_samples: Dict[float, List[float]],
    prio_samples: Dict[float, List[float]],
    output: Path,
) -> None:
    fig, ax = plt.subplots(figsize=(10, 5))
    fifo_positions = [idx - 0.2 for idx in range(len(rps_values))]
    prio_positions = [idx + 0.2 for idx in range(len(rps_values))]

    fifo_bp = ax.boxplot(
        [fifo_samples[rps] for rps in rps_values],
        positions=fifo_positions,
        widths=0.35,
        whis=(5, 95),
        patch_artist=True,
        manage_ticks=False,
        showfliers=False,
    )
    prio_bp = ax.boxplot(
        [prio_samples[rps] for rps in rps_values],
        positions=prio_positions,
        widths=0.35,
        whis=(5, 95),
        patch_artist=True,
        manage_ticks=False,
        showfliers=False,
    )

    for patch in fifo_bp["boxes"]:
        patch.set(facecolor="#1f77b4", alpha=0.6)
    for patch in prio_bp["boxes"]:
        patch.set(facecolor="#ff7f0e", alpha=0.6)

    ax.set_xticks(range(len(rps_values)))
    ax.set_xticklabels([f"{rps:g}" for rps in rps_values])
    ax.set_xlabel("Offered load (RPS)")
    ax.set_ylabel("Latency (ms)")
    ax.set_title("Latency distribution by policy (5th-95th percentile whiskers)")
    ax.grid(True, which="both", linestyle="--", alpha=0.3, axis="y")
    ax.legend(
        handles=[
            Patch(facecolor="#1f77b4", alpha=0.6, label="FIFO policy"),
            Patch(facecolor="#ff7f0e", alpha=0.6, label="Priority policy"),
        ]
    )

    fig.tight_layout()
    output.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(output)
    print(f"Saved latency boxplot to {output}")


def main() -> None:
    args = parse_args()
    fifo_dir = args.data_root / "fifo"
    prio_dir = args.data_root / "prio"

    for directory in (fifo_dir, prio_dir):
        if not directory.is_dir():
            raise FileNotFoundError(f"Expected directory missing: {directory}")

    fifo_samples = load_latency_samples(fifo_dir)
    prio_samples = load_latency_samples(prio_dir)

    rps_values = align_rps(fifo_samples, prio_samples)
    fifo_goodput = [
        compute_goodput(fifo_samples[rps], args.threshold_ms, rps) for rps in rps_values
    ]
    prio_goodput = [
        compute_goodput(prio_samples[rps], args.threshold_ms, rps) for rps in rps_values
    ]

    print("Goodput summary (threshold = {:.1f} ms):".format(args.threshold_ms))
    print("RPS\tFIFO\tPRIO")
    for rps, fifo_val, prio_val in zip(rps_values, fifo_goodput, prio_goodput):
        print(f"{rps:g}\t{fifo_val:.2f}\t{prio_val:.2f}")

    plot_goodput(rps_values, fifo_goodput, prio_goodput, args.output)
    build_boxplot(rps_values, fifo_samples, prio_samples, args.boxplot_output)

    if args.show:
        plt.show()


if __name__ == "__main__":
    main()

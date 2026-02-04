#!/usr/bin/env python3
"""Compare latency samples from two CSV exports and produce diagnostic plots."""

import argparse
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd

REQUIRED_COLUMNS = {"request_id", "queue_latency_us", "e2e_latency_us"}
SLO_US = 50_000  # 50 ms


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Plot queue and end-to-end latency comparisons for matching request IDs "
            "across two CSV files."
        )
    )
    parser.add_argument(
        "file_a",
        type=Path,
        help="Path to the prio latency CSV (e.g., priority scheduler results)",
    )
    parser.add_argument(
        "file_b",
        type=Path,
        help="Path to the fifo latency CSV (e.g., FIFO scheduler results)",
    )
    parser.add_argument(
        "--sample-size",
        type=int,
        default=200,
        help=(
            "Optional random sample size to down-sample matching request IDs before plotting. "
            "If omitted, all matches are used."
        ),
    )
    parser.add_argument(
        "--seed",
        type=int,
        default=0,
        help="Random seed used when --sample-size is specified.",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=Path("."),
        help="Directory where plots will be written (default: current directory).",
    )
    return parser.parse_args()


def load_csv(path: Path) -> pd.DataFrame:
    if not path.exists():
        raise FileNotFoundError(f"CSV not found: {path}")
    df = pd.read_csv(path)
    missing = REQUIRED_COLUMNS - set(df.columns)
    if missing:
        raise ValueError(f"Missing columns in {path}: {', '.join(sorted(missing))}")
    return df


def align_on_request_id(df_a: pd.DataFrame, df_b: pd.DataFrame) -> pd.DataFrame:
    merged = df_a.merge(
        df_b,
        on="request_id",
        suffixes=("_a", "_b"),
        how="inner",
    )
    if merged.empty:
        raise ValueError(
            "No overlapping request_id values found between the two files."
        )
    return merged.sort_values("request_id")


def maybe_sample(df: pd.DataFrame, sample_size: int | None, seed: int) -> pd.DataFrame:
    if sample_size is None or sample_size >= len(df):
        return df
    return df.sample(n=sample_size, random_state=seed).sort_values("request_id")


def to_milliseconds(series: pd.Series) -> pd.Series:
    return series.astype(float) / 1_000.0


def plot_queue_vs_request_id(df: pd.DataFrame, output_path: Path) -> None:
    ordered = df.assign(mean_e2e_us=(df["e2e_latency_us_b"]) / 1.0).sort_values(
        "mean_e2e_us"
    )
    x_positions = np.arange(len(ordered), dtype=float)
    prio_color = "tab:blue"
    fifo_color = "tab:orange"

    queue_a_ms = to_milliseconds(ordered["queue_latency_us_a"]).to_numpy()
    queue_b_ms = to_milliseconds(ordered["queue_latency_us_b"]).to_numpy()

    plt.figure(figsize=(12, 6))
    plt.scatter(
        x_positions,
        queue_a_ms,
        s=12,
        alpha=0.7,
        color=prio_color,
        label="Prio_global queue latency",
    )
    plt.scatter(
        x_positions,
        queue_b_ms,
        s=12,
        alpha=0.7,
        color=fifo_color,
        label="FIFO queue latency",
    )

    if len(queue_a_ms) >= 3:
        coeffs_a = np.polyfit(x_positions, queue_a_ms, deg=2)
        x_line = np.linspace(x_positions.min(), x_positions.max(), 200)
        plt.plot(
            x_line,
            np.polyval(coeffs_a, x_line),
            color=prio_color,
            linewidth=2,
            alpha=0.8,
            label="Prio-global quadratic fit",
        )

    if len(queue_b_ms) >= 3:
        coeffs_b = np.polyfit(x_positions, queue_b_ms, deg=2)
        x_line = np.linspace(x_positions.min(), x_positions.max(), 200)
        plt.plot(
            x_line,
            np.polyval(coeffs_b, x_line),
            color=fifo_color,
            linewidth=2,
            alpha=0.8,
            label="FIFO quadratic fit",
        )
    plt.xlabel("e2e request latency")
    plt.ylabel("Queue latency across all microservices(ms)")
    plt.title("Queue latency comparison between FIFO and Prio_global schedulers")
    plt.legend()
    plt.tight_layout()
    plt.savefig(output_path, dpi=200)
    plt.close()


def plot_queue_vs_e2e(df: pd.DataFrame, output_path: Path) -> None:
    prio_color = "tab:blue"
    fifo_color = "tab:orange"
    e2e_a_ms = to_milliseconds(df["e2e_latency_us_a"]).to_numpy()
    queue_a_ms = to_milliseconds(df["queue_latency_us_a"]).to_numpy()
    e2e_b_ms = to_milliseconds(df["e2e_latency_us_b"]).to_numpy()
    queue_b_ms = to_milliseconds(df["queue_latency_us_b"]).to_numpy()

    plt.figure(figsize=(8, 6))
    plt.scatter(
        e2e_a_ms,
        queue_a_ms,
        s=12,
        alpha=0.7,
        color=prio_color,
        label="Prio-global",
    )
    plt.scatter(
        e2e_b_ms,
        queue_b_ms,
        s=12,
        alpha=0.7,
        color=fifo_color,
        label="FIFO",
    )

    if len(e2e_a_ms) >= 3:
        order_a = np.argsort(e2e_a_ms)
        x_line = np.linspace(e2e_a_ms.min(), e2e_a_ms.max(), 200)
        coeffs_a = np.polyfit(e2e_a_ms[order_a], queue_a_ms[order_a], deg=2)
        plt.plot(
            x_line,
            np.polyval(coeffs_a, x_line),
            color=prio_color,
            linewidth=2,
            alpha=0.8,
            label="Prio-global quadratic fit",
        )

    if len(e2e_b_ms) >= 3:
        order_b = np.argsort(e2e_b_ms)
        x_line = np.linspace(e2e_b_ms.min(), e2e_b_ms.max(), 200)
        coeffs_b = np.polyfit(e2e_b_ms[order_b], queue_b_ms[order_b], deg=2)
        plt.plot(
            x_line,
            np.polyval(coeffs_b, x_line),
            color=fifo_color,
            linewidth=2,
            alpha=0.8,
            label="FIFO quadratic fit",
        )
    plt.axvline(SLO_US / 1_000.0, color="red", linestyle="--", label="SLO 50 ms")
    plt.xlabel("End-to-end latency (ms)")
    plt.ylabel("Queue latency across all microservices(ms)")
    plt.title("Queueing vs end-to-end latency in FIFO and Prio_global schedulers")
    plt.legend()
    plt.tight_layout()
    plt.savefig(output_path, dpi=200)
    plt.close()


def main() -> None:
    args = parse_args()
    output_dir = args.output_dir
    output_dir.mkdir(parents=True, exist_ok=True)

    df_a = load_csv(args.file_a)
    df_b = load_csv(args.file_b)
    merged = align_on_request_id(df_a, df_b)
    merged = maybe_sample(merged, args.sample_size, args.seed)

    if len(merged) < len(df_a):
        print(f"Using {len(merged)} matched request IDs after filtering")
    else:
        print(f"Using all {len(merged)} matched request IDs")

    plot_queue_vs_request_id(merged, output_dir / "queue_latency_vs_request_id.png")
    plot_queue_vs_e2e(merged, output_dir / "queue_vs_e2e_latency.png")
    print(f"Plots saved to {output_dir}")


if __name__ == "__main__":
    main()

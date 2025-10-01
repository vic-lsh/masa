#!/usr/bin/env python3
"""Compare latency samples from two CSV exports and produce diagnostic plots."""

import argparse
from pathlib import Path

import matplotlib.pyplot as plt
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
        raise ValueError("No overlapping request_id values found between the two files.")
    return merged.sort_values("request_id")


def maybe_sample(df: pd.DataFrame, sample_size: int | None, seed: int) -> pd.DataFrame:
    if sample_size is None or sample_size >= len(df):
        return df
    return df.sample(n=sample_size, random_state=seed).sort_values("request_id")


def to_milliseconds(series: pd.Series) -> pd.Series:
    return series.astype(float) / 1_000.0


def plot_queue_vs_request_id(df: pd.DataFrame, output_path: Path) -> None:
    plt.figure(figsize=(12, 6))
    plt.scatter(
        df["request_id"],
        to_milliseconds(df["queue_latency_us_a"]),
        s=12,
        alpha=0.7,
        label="Prio queue latency",
    )
    plt.scatter(
        df["request_id"],
        to_milliseconds(df["queue_latency_us_b"]),
        s=12,
        alpha=0.7,
        label="FIFO queue latency",
    )
    plt.xlabel("Request ID sort by e2e_lat")
    plt.ylabel("Queue latency (ms)")
    plt.title("Queue latency comparison per request")
    plt.legend()
    plt.tight_layout()
    plt.savefig(output_path, dpi=200)
    plt.close()


def plot_queue_vs_e2e(df: pd.DataFrame, output_path: Path) -> None:
    plt.figure(figsize=(8, 6))
    plt.scatter(
        to_milliseconds(df["e2e_latency_us_a"]),
        to_milliseconds(df["queue_latency_us_a"]),
        s=12,
        alpha=0.7,
        label="Prio",
    )
    plt.scatter(
        to_milliseconds(df["e2e_latency_us_b"]),
        to_milliseconds(df["queue_latency_us_b"]),
        s=12,
        alpha=0.7,
        label="FIFO",
    )
    plt.axvline(SLO_US / 1_000.0, color="red", linestyle="--", label="SLO 50 ms")
    plt.xlabel("End-to-end latency (ms)")
    plt.ylabel("Queue latency (ms)")
    plt.title("Queue latency vs end-to-end latency")
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

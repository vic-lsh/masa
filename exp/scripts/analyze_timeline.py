#!/usr/bin/env python3
"""Analyze goodput timeline CSVs — computes per-RPS-step stability metrics.

Usage:
    # Single experiment:
    python exp/scripts/analyze_timeline.py socialnet anchor_7

    # Compare multiple experiments:
    python exp/scripts/analyze_timeline.py socialnet cp_simple anchor_3 anchor_7

    # Skip warmup seconds per step (default: 10):
    python exp/scripts/analyze_timeline.py socialnet anchor_7 --warmup 5

Reads: exp/<app>/plots/<name>/0/goodput_timeline.csv
       exp/<app>/in/<name>/gen_config.json  (for RPS schedule)
"""

import csv
import json
import math
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent


def load_timeline(app: str, exp: str) -> list[dict]:
    path = ROOT / "exp" / app / "plots" / exp / "0" / "goodput_timeline.csv"
    rows = []
    with open(path) as f:
        for row in csv.DictReader(f):
            rows.append({
                "time": float(row["Time"]),
                "policy": row["Policy"],
                "goodput": float(row["Goodput"]),
            })
    return rows


def load_config(app: str, exp: str) -> dict:
    path = ROOT / "exp" / app / "in" / exp / "gen_config.json"
    with open(path) as f:
        return json.load(f)


def segment_timeline(rows: list[dict], gap_threshold: float = 1.5) -> list[list[dict]]:
    """Split timeline into segments based on time gaps between steps."""
    if not rows:
        return []
    segments: list[list[dict]] = [[rows[0]]]
    for i in range(1, len(rows)):
        if rows[i]["time"] - rows[i - 1]["time"] > gap_threshold:
            segments.append([])
        segments[-1].append(rows[i])
    return segments


def median(values: list[float]) -> float:
    s = sorted(values)
    n = len(s)
    if n % 2 == 1:
        return s[n // 2]
    return (s[n // 2 - 1] + s[n // 2]) / 2


def analyze_step(values: list[float]) -> dict:
    n = len(values)
    if n == 0:
        return {"mean": 0, "median": 0, "std": 0, "min": 0, "max": 0, "cov": 0, "range": 0, "n": 0}
    mean = sum(values) / n
    med = median(values)
    variance = sum((v - mean) ** 2 for v in values) / n if n > 1 else 0
    std = math.sqrt(variance)
    cov = (std / mean * 100) if mean > 0 else 0
    return {
        "mean": mean,
        "median": med,
        "std": std,
        "min": min(values),
        "max": max(values),
        "cov": cov,
        "range": max(values) - min(values),
        "n": n,
    }


def get_step_stats(
    app: str, exp: str, warmup: float
) -> tuple[list[int], dict[int, dict]]:
    """Return (rps_list, {rps: stats_dict}) for an experiment."""
    rows = load_timeline(app, exp)
    config = load_config(app, exp)
    rps_list: list[int] = config["Rps"]
    segments = segment_timeline(rows)

    if len(segments) != len(rps_list):
        print(
            f"WARNING: {exp} has {len(segments)} segments but {len(rps_list)} RPS steps",
            file=sys.stderr,
        )

    stats: dict[int, dict] = {}
    for i, rps in enumerate(rps_list):
        if i >= len(segments):
            stats[rps] = analyze_step([])
            continue
        seg = segments[i]
        seg_start = seg[0]["time"]
        values = [r["goodput"] for r in seg if r["time"] >= seg_start + warmup]
        stats[rps] = analyze_step(values)
    return rps_list, stats


def print_single(app: str, exp: str, warmup: float):
    rps_list, stats = get_step_stats(app, exp, warmup)

    print(f"\n## {exp} ({app})\n")
    print("| RPS | Mean | Median | StdDev | CoV | Min | Max | Range | N |")
    print("|-----|------|--------|--------|-----|-----|-----|-------|---|")
    for rps in rps_list:
        s = stats[rps]
        print(
            f"| {rps} | {s['mean']:.1f} | {s['median']:.1f} | {s['std']:.1f} | {s['cov']:.1f}% "
            f"| {s['min']:.1f} | {s['max']:.1f} | {s['range']:.1f} | {s['n']} |"
        )


def print_comparison(app: str, experiments: list[str], warmup: float):
    all_stats: dict[str, dict[int, dict]] = {}
    rps_list = None

    for exp in experiments:
        rps, stats = get_step_stats(app, exp, warmup)
        if rps_list is None:
            rps_list = rps
        all_stats[exp] = stats

    assert rps_list is not None
    ref = experiments[0]

    print(f"\n## Comparison: {' vs '.join(experiments)} ({app})\n")

    # Header
    cols = ["RPS"]
    for exp in experiments:
        cols.append(f"{exp} Mean(CoV)")
    for exp in experiments[1:]:
        cols.append(f"Δ Mean vs {ref}")
        cols.append(f"Δ CoV vs {ref}")
    print("| " + " | ".join(cols) + " |")
    print("|" + "|".join(["-----"] * len(cols)) + "|")

    for rps in rps_list:
        parts = [str(rps)]
        for exp in experiments:
            s = all_stats[exp][rps]
            parts.append(f"{s['mean']:.0f} ({s['cov']:.1f}%)")
        ref_s = all_stats[ref][rps]
        for exp in experiments[1:]:
            s = all_stats[exp][rps]
            dm = s["mean"] - ref_s["mean"]
            dc = s["cov"] - ref_s["cov"]
            parts.append(f"{dm:+.0f}")
            parts.append(f"{dc:+.1f}pp")
        print("| " + " | ".join(parts) + " |")

    # Best per RPS
    print(f"\n### Best per RPS\n")
    print("| RPS | Best Mean | Best CoV |")
    print("|-----|-----------|----------|")
    for rps in rps_list:
        best_mean_exp = max(experiments, key=lambda e: all_stats[e][rps]["mean"])
        best_cov_exp = min(experiments, key=lambda e: all_stats[e][rps]["cov"])
        bm = all_stats[best_mean_exp][rps]
        bc = all_stats[best_cov_exp][rps]
        print(
            f"| {rps} | {best_mean_exp} ({bm['mean']:.0f}) "
            f"| {best_cov_exp} ({bc['cov']:.1f}%) |"
        )


def main():
    if len(sys.argv) < 3:
        print("Usage: python analyze_timeline.py <app> <exp1> [exp2 ...] [--warmup N]")
        sys.exit(1)

    args = sys.argv[1:]
    warmup = 10.0
    if "--warmup" in args:
        idx = args.index("--warmup")
        warmup = float(args[idx + 1])
        args = args[:idx] + args[idx + 2 :]

    app = args[0]
    experiments = args[1:]

    if len(experiments) == 1:
        print_single(app, experiments[0], warmup)
    else:
        print_comparison(app, experiments, warmup)


if __name__ == "__main__":
    main()

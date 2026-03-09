"""
Plotting utilities for MSSIM experiment results.
"""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Dict, List, Sequence

import matplotlib

matplotlib.use("Agg")  # Non-interactive backend for file output
import matplotlib.pyplot as plt
import numpy as np
import pandas as pd

from . import cpu
from .goodput import (
    _plot_early_return_breakdown,
    compute_early_return_breakdown,
    compute_early_return_last_child_breakdown,
)
from .util import (
    _read_request_csv,
    filter_excluded_errors,
    get_policy_color,
    get_policy_display_name,
    read_policies,
)

plt.rcParams["figure.max_open_warning"] = 0


_RPS_DIR_RE = re.compile(r"^rps_(?P<rps>[0-9_]+(?:\.[0-9_]+)?)$")


def _parse_rps_dir(path: Path) -> float:
    match = _RPS_DIR_RE.match(path.name)
    if not match:
        raise ValueError(f"Unexpected RPS directory name: {path}")
    token = match.group("rps").replace("_", ".")
    return float(token)


_RPS_FILE_RE = re.compile(r"r(?P<rps>[0-9_]+(?:\.[0-9_]+)?)_(?P<api>.+)\.csv$")
_ROOT_LAT_FILE_RE = re.compile(r"^root_latencies_(?P<rps>[0-9_]+(?:\.[0-9_]+)?)rps\.csv$")


def _parse_rps_from_filename(path: Path) -> float:
    """Parse RPS value from known MSSIM CSV filename patterns."""
    match = _RPS_FILE_RE.match(path.name)
    if match:
        token = match.group("rps").replace("_", ".")
        return float(token)

    match = _ROOT_LAT_FILE_RE.match(path.name)
    if match:
        token = match.group("rps").replace("_", ".")
        return float(token)

    raise ValueError(f"Unexpected RPS filename: {path}")


def _iter_policy_latency_csvs(policy_dir: Path) -> List[Path]:
    """Return latency CSVs for a policy from both legacy and run_* layouts."""
    csv_paths: List[Path] = []

    run_dirs = sorted(p for p in policy_dir.glob("run_*") if p.is_dir())
    if run_dirs:
        for run_dir in run_dirs:
            csv_paths.extend(sorted(run_dir.glob("r*.csv")))
            csv_paths.extend(sorted(run_dir.glob("root_latencies_*rps.csv")))
        return csv_paths

    # Legacy layout: CSVs directly under policy directory
    csv_paths.extend(sorted(policy_dir.glob("r*.csv")))
    csv_paths.extend(sorted(policy_dir.glob("root_latencies_*rps.csv")))
    return csv_paths


def _load_json(path: Path) -> dict:
    with path.open("r", encoding="utf-8") as fh:
        return json.load(fh)


def _normalize_bool_series(series: pd.Series) -> pd.Series:
    if pd.api.types.is_bool_dtype(series):
        return series.fillna(False)
    normalized = series.astype(str).str.strip().str.lower()
    return normalized.isin(["true", "1", "yes", "y", "t"])


def _filter_errors(df: pd.DataFrame) -> pd.DataFrame:
    if "is_err" not in df.columns:
        return df
    # We don't filter errors here anymore, because we want to analyze early returns.
    # The caller functions (like goodput calculation) should filter errors if needed.
    return df


def _filter_after_warmup(
    df: pd.DataFrame, warmup_sec: float, source: Path
) -> pd.DataFrame:
    if warmup_sec <= 0:
        return df
    if "start_at" not in df.columns:
        return df

    start_at = pd.to_numeric(df["start_at"], errors="coerce")
    if start_at.dropna().empty:
        return df

    warmup_us = warmup_sec * 1_000_000.0
    measurement_anchor = start_at.min()
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


def _load_policy_data(policy_dir: Path, warmup_sec: float) -> Dict[float, pd.DataFrame]:
    if not policy_dir.is_dir():
        raise FileNotFoundError(f"Policy directory not found: {policy_dir}")

    data_by_rps: Dict[float, List[pd.DataFrame]] = {}

    for csv_path in _iter_policy_latency_csvs(policy_dir):
        try:
            rps = _parse_rps_from_filename(csv_path)
        except ValueError:
            continue

        df = _read_request_csv(str(csv_path))

        # Map columns from request CSV format if needed.
        if "e2e_latency_us" not in df.columns and "latency" in df.columns:
            df.rename(columns={"latency": "e2e_latency_us"}, inplace=True)

        if "queue_latency_us" not in df.columns:
            q_cols = [c for c in ["q_lat_init", "q_lat_resume"] if c in df.columns]
            if q_cols:
                df["queue_latency_us"] = df[q_cols].sum(axis=1)

        if "e2e_latency_us" not in df.columns:
            print(f"Warning: missing e2e_latency_us in {csv_path}")
            continue

        df = _filter_errors(df)
        df = _filter_after_warmup(df, warmup_sec, csv_path)
        if df.empty:
            continue

        df = df.copy()
        df["e2e_latency_ms"] = pd.to_numeric(df["e2e_latency_us"], errors="coerce") / 1_000.0
        if "queue_latency_us" in df.columns:
            queue_us = pd.to_numeric(df["queue_latency_us"], errors="coerce").fillna(0.0)
        else:
            queue_us = 0.0
        df["queue_latency_ms"] = queue_us / 1_000.0
        df["start_at"] = pd.to_numeric(df.get("start_at"), errors="coerce")

        data_by_rps.setdefault(rps, []).append(df)

    combined: Dict[float, pd.DataFrame] = {}
    for rps, frames in data_by_rps.items():
        if frames:
            combined[rps] = pd.concat(frames, ignore_index=True)
    return combined


def _effective_duration_sec(
    df: pd.DataFrame, duration_sec: float, warmup_sec: float
) -> float:
    if duration_sec and duration_sec > warmup_sec:
        return duration_sec - warmup_sec

    if df.empty or "start_at" not in df.columns:
        return 1.0

    start_at = df["start_at"].dropna()
    if start_at.empty:
        return 1.0

    e2e_us = df["e2e_latency_ms"].fillna(0.0) * 1_000.0
    end_at = (df["start_at"] + e2e_us).dropna()
    if end_at.empty:
        return 1.0

    duration = (end_at.max() - start_at.min()) / 1_000_000.0
    return duration if duration > 0 else 1.0


def _compute_goodput(
    df: pd.DataFrame, slo_ms: float, duration_sec: float, warmup_sec: float
) -> float:
    if df.empty:
        return 0.0

    # Filter out EarlyReturn and ClientTimeout — they should not count as goodput.
    # Prefer the parsed error_type column (set by _read_request_csv) over the legacy is_err flag.
    if "error_type" in df.columns:
        is_early_return = df["error_type"] == "EarlyReturn"
        is_timeout = df["error"].astype(str) == "/ClientTimeout" if "error" in df.columns else pd.Series(False, index=df.index)
        df = df.loc[~(is_early_return | is_timeout)]
    elif "is_err" in df.columns:
        err_mask = _normalize_bool_series(df["is_err"])
        df = df.loc[~err_mask]

    if df.empty:
        return 0.0

    meets_slo = df["e2e_latency_ms"] <= slo_ms
    denom = _effective_duration_sec(df, duration_sec, warmup_sec)
    return float(meets_slo.sum()) / denom


def _compute_latency_percentiles(
    df: pd.DataFrame, percentiles: Sequence[float]
) -> Dict[float, float]:
    if df.empty:
        return {p: float("nan") for p in percentiles}

    # Filter out errors for latency calculation
    if "is_err" in df.columns:
        err_mask = _normalize_bool_series(df["is_err"])
        df = df.loc[~err_mask]

    if df.empty:
        return {p: float("nan") for p in percentiles}

    return {p: float(df["e2e_latency_ms"].quantile(p / 100.0)) for p in percentiles}


def _write_goodput_csv(
    output_path: Path,
    rps_values: Sequence[float],
    policy_series: Dict[str, Sequence[float]],
) -> None:
    rows = []
    for rps, *values in zip(rps_values, *policy_series.values()):
        row = {"rps": rps}
        for policy, val in zip(policy_series.keys(), values):
            row[policy] = round(val, 3)
        rows.append(row)
    pd.DataFrame(rows).to_csv(output_path, index=False)


def _plot_goodput_lines(
    output_path: Path,
    rps_values: Sequence[float],
    policy_series: Dict[str, Sequence[float]],
    *,
    title: str,
    ylabel: str,
) -> None:
    fig, ax = plt.subplots(figsize=(10, 6))
    cmap = plt.get_cmap("tab10")
    marker_cycle = ("o", "s", "^", "D", "P", "X", "*", "v", "<", ">")

    for idx, (policy, values) in enumerate(policy_series.items()):
        marker = marker_cycle[idx % len(marker_cycle)]
        color = get_policy_color(policy)
        if color is None:
            color = cmap(idx % cmap.N)
        ax.plot(
            rps_values,
            values,
            marker=marker,
            label=get_policy_display_name(policy),
            color=color,
        )

    ax.set_xlabel("Offered load (RPS)")
    ax.set_ylabel(ylabel)
    ax.set_title(title)
    ax.grid(True, which="both", linestyle="--", alpha=0.4)
    ax.legend()
    fig.tight_layout()
    output_path.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(output_path, dpi=300)
    plt.close(fig)


def _plot_latency_percentiles(
    output_path: Path,
    rps_values: Sequence[float],
    policy_percentiles: Dict[str, Dict[float, Sequence[float]]],
    *,
    percentiles: Sequence[float],
    slo_ms: float,
) -> None:
    ncols = 2 if len(percentiles) > 1 else 1
    nrows = int(np.ceil(len(percentiles) / ncols))
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
        for policy_idx, (policy, percentile_map) in enumerate(
            policy_percentiles.items()
        ):
            marker = marker_cycle[policy_idx % len(marker_cycle)]
            color = get_policy_color(policy)
            if color is None:
                color = cmap(policy_idx % cmap.N)
            values = percentile_map.get(percentile, [])
            ax.plot(
                rps_values,
                values,
                marker=marker,
                label=get_policy_display_name(policy),
                color=color,
            )

        ax.set_title(f"P{percentile:g} latency")
        ax.set_ylabel("Latency (ms)")
        ax.grid(True, which="both", linestyle="--", alpha=0.4)
        if slo_ms > 0:
            ax.axhline(slo_ms, linestyle="--", color="grey", alpha=0.6)
        if idx == 0:
            ax.legend()

    for extra_ax in axes_iter[len(percentiles) :]:
        extra_ax.axis("off")

    for ax in axes_iter[-ncols:]:
        ax.set_xlabel("Offered load (RPS)")

    fig.tight_layout()
    output_path.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(output_path, dpi=300)
    plt.close(fig)


def _plot_latency_cdf(
    output_path: Path,
    rps: float,
    policy_data: Dict[str, pd.DataFrame],
    *,
    slo_ms: float,
) -> None:
    """Plot CDF of e2e latency for all policies at a specific RPS."""
    fig, ax = plt.subplots(figsize=(10, 6))
    cmap = plt.get_cmap("tab10")

    any_data = False
    for idx, (policy, df) in enumerate(policy_data.items()):
        if df.empty:
            continue

        # Filter out errors for CDF
        if "is_err" in df.columns:
            err_mask = _normalize_bool_series(df["is_err"])
            df = df.loc[~err_mask]

        latencies = df["e2e_latency_ms"].dropna()
        if latencies.empty:
            continue

        values = np.sort(latencies.to_numpy())
        cdf = (np.arange(1, len(values) + 1) / len(values)).astype(float)
        color = get_policy_color(policy)
        if color is None:
            color = cmap(idx % cmap.N)
        ax.plot(
            values,
            cdf,
            label=get_policy_display_name(policy),
            color=color,
            linewidth=2,
        )
        any_data = True

    if not any_data:
        plt.close(fig)
        return

    ax.set_xlabel("End-to-end latency (ms)")
    ax.set_ylabel("CDF")
    ax.set_title(f"Latency CDF at {rps:g} RPS")
    ax.grid(True, which="both", linestyle="--", alpha=0.4)
    if slo_ms > 0:
        ax.axvline(
            slo_ms, linestyle="--", color="grey", alpha=0.6, label=f"SLO={slo_ms:g} ms"
        )
    ax.legend()
    ax.set_xlim(left=0)

    fig.tight_layout()
    output_path.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(output_path, dpi=300)
    plt.close(fig)


def _plot_goodput_timeline(
    output_path: Path,
    rps_sequence: List[float],
    policy_data_by_rps: Dict[str, Dict[float, pd.DataFrame]],
    *,
    duration_sec: float,
    slo_ms: float,
    window_sec: float = 2.0,
) -> None:
    """Plot goodput over time, stitching RPS periods in their original run order.

    Each RPS period occupies [i*duration_sec, (i+1)*duration_sec] on the x-axis.
    Goodput is computed using a sliding window so disruptions and recovery are visible.
    """
    fig, ax = plt.subplots(figsize=(14, 6))
    cmap = plt.get_cmap("tab10")

    for idx, (policy, rps_data) in enumerate(policy_data_by_rps.items()):
        all_times: List[float] = []
        all_goodput: List[float] = []

        for period_idx, rps in enumerate(rps_sequence):
            df = rps_data.get(rps, pd.DataFrame())
            if df.empty:
                continue

            start_at = pd.to_numeric(df["start_at"], errors="coerce")
            if start_at.dropna().empty:
                continue

            t_min = start_at.min()
            # Time relative to this period's nominal start (seconds)
            rel_sec = (start_at - t_min) / 1_000_000.0
            abs_sec = rel_sec + period_idx * duration_sec

            # Build goodput mask: exclude ER and ClientTimeout, require SLO
            e2e_ms = pd.to_numeric(df.get("e2e_latency_ms"), errors="coerce")
            if "error_type" in df.columns:
                is_er = df["error_type"] == "EarlyReturn"
            elif "error" in df.columns:
                is_er = df["error"].astype(str).str.startswith("/EarlyReturn")
            else:
                is_er = pd.Series(False, index=df.index)
            is_timeout = (
                df["error"].astype(str) == "/ClientTimeout"
                if "error" in df.columns
                else pd.Series(False, index=df.index)
            )
            good = (~is_er) & (~is_timeout) & (e2e_ms <= slo_ms)

            # Sort by time for efficient windowing
            order = np.argsort(abs_sec.values)
            t_arr = abs_sec.values[order]
            g_arr = good.values[order]

            # Sliding window: step=0.5s, window=window_sec
            step = 0.5
            t_centers = np.arange(
                period_idx * duration_sec + window_sec / 2,
                (period_idx + 1) * duration_sec - window_sec / 2 + step,
                step,
            )
            for tc in t_centers:
                lo, hi = tc - window_sec / 2, tc + window_sec / 2
                mask = (t_arr >= lo) & (t_arr < hi)
                all_times.append(tc)
                all_goodput.append(float(g_arr[mask].sum()) / window_sec)

        if not all_times:
            continue

        color = get_policy_color(policy)
        if color is None:
            color = cmap(idx % cmap.N)
        ax.plot(
            all_times,
            all_goodput,
            label=get_policy_display_name(policy),
            color=color,
            linewidth=1.5,
        )

    # Offered RPS as a filled step area on the same axis (same unit: RPS)
    step_t = [0.0]
    step_rps = [rps_sequence[0]]
    for i, rps in enumerate(rps_sequence):
        t_start = i * duration_sec
        if i > 0:
            step_t.append(t_start)
            step_rps.append(rps)
            ax.axvline(t_start, linestyle="--", color="grey", alpha=0.4, linewidth=1)
        step_t.append(t_start + duration_sec)
        step_rps.append(rps)
    ax.fill_between(step_t, step_rps, step=None, color="grey", alpha=0.12, label="Offered RPS")
    ax.step(step_t, step_rps, where="post", color="grey", linewidth=1.5,
            linestyle="-", alpha=0.5)

    ax.set_xlabel("Time (s)")
    ax.set_ylabel("RPS")
    ax.set_title(f"Goodput timeline (SLO={slo_ms:g} ms, {window_sec:g}s window)")
    ax.set_xlim(left=0, right=len(rps_sequence) * duration_sec)
    ax.set_ylim(bottom=0)
    ax.grid(True, which="both", linestyle="--", alpha=0.4)
    ax.legend()
    fig.tight_layout()
    output_path.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(output_path, dpi=300)
    plt.close(fig)


def _resolve_iteration_ids(data_dir: Path, repeats: int) -> List[int]:
    iteration_dirs = [p for p in data_dir.iterdir() if p.is_dir() and p.name.isdigit()]
    iteration_ids = sorted([int(p.name) for p in iteration_dirs])
    if repeats > 0:
        iteration_ids = [i for i in iteration_ids if i < repeats]
    return iteration_ids


def _resolve_policies(
    config_dir: Path, data_dir: Path, iteration_ids: Sequence[int]
) -> List[str]:
    policies_path = config_dir / "policies"
    if policies_path.exists():
        return read_policies(config_dir)

    for iteration in iteration_ids:
        iteration_dir = data_dir / str(iteration)
        if iteration_dir.is_dir():
            policies = [p.name for p in iteration_dir.iterdir() if p.is_dir()]
            if policies:
                return sorted(policies)
    raise FileNotFoundError(f"No policy directories found under {data_dir}")


def _resolve_rps_values(
    gen_config: dict,
    data_dir: Path,
    iteration_ids: Sequence[int],
    policies: Sequence[str],
) -> List[float]:
    if gen_config.get("Rps"):
        return sorted(set(float(v) for v in gen_config["Rps"]))

    # Try to find RPS values from output files.
    for iteration in iteration_ids:
        for policy in policies:
            policy_dir = data_dir / str(iteration) / policy
            if not policy_dir.is_dir():
                continue
            rps_values = []
            for csv_path in _iter_policy_latency_csvs(policy_dir):
                try:
                    rps_values.append(_parse_rps_from_filename(csv_path))
                except ValueError:
                    continue

            if rps_values:
                return sorted(set(rps_values))  # Remove duplicates and sort
    raise FileNotFoundError("No RPS values found for MSSIM output")


def generate_plots(args) -> None:
    config_dir = Path(args.config_dir)
    data_dir = Path(args.data_dir)
    output_dir = Path(args.output_dir)

    gen_config = _load_json(config_dir / "gen_config.json")
    mssim_config = _load_json(config_dir / "mssim.json")

    repeats = int(gen_config.get("Repeats") or 0)
    warmup_sec = float(gen_config.get("WarmupSecs", 0) or 0)
    duration_sec = float(gen_config.get("DurationSecs", 0) or 0)
    slo_ms = float(mssim_config.get("slo_ms", 0) or 0)

    iteration_ids = _resolve_iteration_ids(data_dir, repeats or 0)
    if not iteration_ids:
        raise FileNotFoundError(f"No iteration directories found under {data_dir}")

    policies = _resolve_policies(config_dir, data_dir, iteration_ids)
    rps_values = _resolve_rps_values(gen_config, data_dir, iteration_ids, policies)
    # Original run order for timeline (gen_config["Rps"] preserves sequence)
    rps_sequence: List[float] = [float(v) for v in gen_config.get("Rps", [])] or rps_values

    percentiles = (50.0, 90.0, 99.0)

    per_iteration_goodput: List[Dict[str, List[float]]] = []
    per_iteration_percentiles: List[Dict[str, Dict[float, List[float]]]] = []

    for iteration in iteration_ids:
        iteration_dir = data_dir / str(iteration)
        policy_data: Dict[str, Dict[float, pd.DataFrame]] = {}
        for policy in policies:
            policy_dir = iteration_dir / policy
            policy_data[policy] = _load_policy_data(policy_dir, warmup_sec)

        goodput_by_policy: Dict[str, List[float]] = {}
        percentiles_by_policy: Dict[str, Dict[float, List[float]]] = {}
        early_returns_breakdown_by_policy: Dict[str, List[Dict]] = {}
        total_early_returns_by_policy: Dict[str, List[float]] = {}
        early_returns_lc_breakdown_by_policy: Dict[str, List[Dict]] = {}
        total_early_returns_lc_by_policy: Dict[str, List[float]] = {}

        for policy in policies:
            goodput_values = []
            percentile_values: Dict[float, List[float]] = {p: [] for p in percentiles}
            early_returns_breakdown = []
            total_early_returns = []
            early_returns_lc_breakdown = []
            total_early_returns_lc = []

            for rps in rps_values:
                df = policy_data.get(policy, {}).get(rps, pd.DataFrame())
                goodput_values.append(
                    _compute_goodput(df, slo_ms, duration_sec, warmup_sec)
                )
                pct = _compute_latency_percentiles(df, percentiles)
                for p in percentiles:
                    percentile_values[p].append(pct[p])

                # Calculate early return stats
                # Prepare DF for goodput functions (needs "latency" column)
                df_gp = df.copy()
                if "e2e_latency_us" in df_gp.columns:
                    df_gp["latency"] = df_gp["e2e_latency_us"]

                brk = compute_early_return_breakdown(df_gp)
                early_returns_breakdown.append(brk)
                total_early_returns.append(sum(brk.values()))

                brk_lc = compute_early_return_last_child_breakdown(df_gp)
                early_returns_lc_breakdown.append(brk_lc)
                total_early_returns_lc.append(sum(brk_lc.values()))

            goodput_by_policy[policy] = goodput_values
            percentiles_by_policy[policy] = percentile_values
            early_returns_breakdown_by_policy[policy] = early_returns_breakdown
            total_early_returns_by_policy[policy] = total_early_returns
            early_returns_lc_breakdown_by_policy[policy] = early_returns_lc_breakdown
            total_early_returns_lc_by_policy[policy] = total_early_returns_lc

        per_iteration_goodput.append(goodput_by_policy)
        per_iteration_percentiles.append(percentiles_by_policy)

        fraction_by_policy = {
            policy: [
                (val / rps) if rps else float("nan")
                for val, rps in zip(values, rps_values)
            ]
            for policy, values in goodput_by_policy.items()
        }

        iteration_output = output_dir / str(iteration)
        _plot_goodput_lines(
            iteration_output / "goodput_absolute.png",
            rps_values,
            goodput_by_policy,
            title=f"Goodput vs RPS (SLO={slo_ms:g} ms)",
            ylabel="Goodput (RPS)",
        )
        _write_goodput_csv(
            iteration_output / "goodput_absolute.csv",
            rps_values,
            goodput_by_policy,
        )
        _plot_goodput_lines(
            iteration_output / "goodput_fraction.png",
            rps_values,
            fraction_by_policy,
            title=f"Goodput fraction vs RPS (SLO={slo_ms:g} ms)",
            ylabel="Goodput / Offered load",
        )
        _write_goodput_csv(
            iteration_output / "goodput_fraction.csv",
            rps_values,
            fraction_by_policy,
        )
        _plot_latency_percentiles(
            iteration_output / "latency_percentiles.png",
            rps_values,
            percentiles_by_policy,
            percentiles=percentiles,
            slo_ms=slo_ms,
        )
        _plot_goodput_timeline(
            iteration_output / "goodput_timeline.png",
            rps_sequence,
            policy_data,
            duration_sec=duration_sec,
            slo_ms=slo_ms,
        )

        # Plot early return breakdowns
        _plot_early_return_breakdown(
            str(iteration_output / "early_return.png"),
            policies=policies,
            rps_values=rps_values,
            policy_total_early_returns=total_early_returns_by_policy,
            policy_early_returns_breakdown=early_returns_breakdown_by_policy,
            title="Early-return requests breakdown by Service::Method",
            stacked_services=True,
        )
        _plot_early_return_breakdown(
            str(iteration_output / "early_return_last_child.png"),
            policies=policies,
            rps_values=rps_values,
            policy_total_early_returns=total_early_returns_lc_by_policy,
            policy_early_returns_breakdown=early_returns_lc_breakdown_by_policy,
            title="Early-return requests breakdown by Last Child Service::Method",
        )

        # Generate CDF plots for each RPS value
        for rps in rps_values:
            rps_policy_data = {
                policy: policy_data.get(policy, {}).get(rps, pd.DataFrame())
                for policy in policies
            }
            _plot_latency_cdf(
                iteration_output / f"latency_cdf_{rps:g}rps.png",
                rps,
                rps_policy_data,
                slo_ms=slo_ms,
            )

    if per_iteration_goodput:
        avg_goodput: Dict[str, List[float]] = {}
        avg_percentiles: Dict[str, Dict[float, List[float]]] = {}

        for policy in policies:
            avg_goodput[policy] = []
            avg_percentiles[policy] = {p: [] for p in percentiles}

        for idx in range(len(rps_values)):
            for policy in policies:
                values = [
                    per_iter.get(policy, [float("nan")] * len(rps_values))[idx]
                    for per_iter in per_iteration_goodput
                ]
                avg_goodput[policy].append(float(np.nanmean(values)))
                for p in percentiles:
                    pct_values = [
                        per_iter.get(policy, {}).get(
                            p, [float("nan")] * len(rps_values)
                        )[idx]
                        for per_iter in per_iteration_percentiles
                    ]
                    avg_percentiles[policy][p].append(float(np.nanmean(pct_values)))

        _plot_goodput_lines(
            output_dir / "goodput_absolute_avg.png",
            rps_values,
            avg_goodput,
            title=f"Average goodput vs RPS (SLO={slo_ms:g} ms)",
            ylabel="Goodput (RPS)",
        )
        _write_goodput_csv(
            output_dir / "goodput_absolute_avg.csv",
            rps_values,
            avg_goodput,
        )
        fraction_by_policy = {
            policy: [
                (val / rps) if rps else float("nan")
                for val, rps in zip(values, rps_values)
            ]
            for policy, values in avg_goodput.items()
        }
        _plot_goodput_lines(
            output_dir / "goodput_fraction_avg.png",
            rps_values,
            fraction_by_policy,
            title=f"Average goodput fraction vs RPS (SLO={slo_ms:g} ms)",
            ylabel="Goodput / Offered load",
        )
        _write_goodput_csv(
            output_dir / "goodput_fraction_avg.csv",
            rps_values,
            fraction_by_policy,
        )
        _plot_latency_percentiles(
            output_dir / "latency_percentiles_avg.png",
            rps_values,
            avg_percentiles,
            percentiles=percentiles,
            slo_ms=slo_ms,
        )
        # Generate averaged CDF plots for each RPS value
        # Combine data from all iterations for each policy and RPS
        for rps in rps_values:
            avg_rps_policy_data: Dict[str, pd.DataFrame] = {}
            for policy in policies:
                combined_dfs = []
                for iteration in iteration_ids:
                    iteration_dir = data_dir / str(iteration)
                    policy_dir = iteration_dir / policy
                    policy_data_iter = _load_policy_data(policy_dir, warmup_sec)
                    df = policy_data_iter.get(rps, pd.DataFrame())
                    if not df.empty:
                        combined_dfs.append(df)
                if combined_dfs:
                    avg_rps_policy_data[policy] = pd.concat(
                        combined_dfs, ignore_index=True
                    )
                else:
                    avg_rps_policy_data[policy] = pd.DataFrame()
            _plot_latency_cdf(
                output_dir / f"latency_cdf_{rps:g}rps_avg.png",
                rps,
                avg_rps_policy_data,
                slo_ms=slo_ms,
            )

    # Generate CPU utilization plots
    print("Generating CPU utilization plots...")
    try:
        cpu.plot_cpu_utilization(data_dir, output_dir, policies=policies)
    except Exception as e:
        print(f"Warning: Failed to generate CPU plots: {e}")

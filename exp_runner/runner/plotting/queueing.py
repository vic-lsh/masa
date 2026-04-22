import json
import os
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from typing import Dict

import matplotlib
import numpy as np
import pandas as pd

from .util import (
    configure_plot_font_sizes,
    filter_excluded_errors,
    get_plot_worker_count,
    get_policy_display_name,
    get_policy_line_style,
    parse_args,
    PlotData,
    prepare_output_dir,
    read_data,
    scale_fontsize,
)

matplotlib.use("Agg")  # Use non-interactive backend for thread safety
import matplotlib.pyplot as plt

# Suppress warning about too many open figures when running in parallel
# We properly close all figures, but many may be open simultaneously during parallel execution
plt.rcParams["figure.max_open_warning"] = 0
configure_plot_font_sizes()

MS_TO_US = 10**3
_TIMELINE_BIN_SEC = 2.0


def _get_queueing_columns(df):
    """Return list of columns ending with '_queueing_latency' or known global queueing keys."""
    cols = [c for c in df.columns if c.endswith("_queueing_latency")]
    for k in ["q_lat_init", "q_lat_resume"]:
        if k in df.columns:
            cols.append(k)
    return cols


def _plot_queueing_breakdown(
    output_path: str,
    api: str,
    policies: list,
    rps_values: list,
    data: dict,  # policy -> rps -> df
    title: str,
) -> None:
    """
    Generate breakdown plot for queueing latency by component:
      - Plot: small multiples (one subplot per policy) with stacked bars
      - Stacks: Components (e.g. child1, child2)
    """
    # Identify queueing columns from the first available dataframe
    queueing_cols = []
    for policy in policies:
        if not queueing_cols:
            for rps in rps_values:
                df = data[policy][rps]
                if not df.empty:
                    queueing_cols = _get_queueing_columns(df)
                    if queueing_cols:
                        break

    if not queueing_cols:
        return

    # Sort columns for consistent stacking
    queueing_cols.sort()

    # Clean component names for legend
    # e.g. "child1_queueing_latency" -> "child1"
    def _clean_name(c):
        if c == "q_lat_init":
            return "Ingress"
        if c == "q_lat_resume":
            return "Resume"
        return c.replace("_queueing_latency", "")

    component_names = [_clean_name(c) for c in queueing_cols]

    # Sort policies
    # Reuse sorting logic if possible, or simple sort
    # We'll just use the provided order or simple sort
    sorted_policies = sorted(policies)
    # Use helper if available, but I don't have access to sort_policies_by_type from goodput here unless I duplicate or move it.
    # Let's duplicate the simple sorting logic for now or just sort alphabetically

    rps_values = list(rps_values)
    x = np.arange(len(rps_values))

    # Setup colors for components
    cmap = plt.get_cmap("tab10")
    comp_colors = {comp: cmap(i % cmap.N) for i, comp in enumerate(component_names)}

    n = len(sorted_policies)
    ncols = min(3, max(1, n))
    nrows = int(np.ceil(n / ncols))
    fig, axes = plt.subplots(
        nrows, ncols, figsize=(15, 4 + 2.8 * nrows), sharex=True, sharey=True
    )

    if n == 1:
        axes = [axes]
    elif nrows == 1:
        axes = axes if isinstance(axes, np.ndarray) else [axes]
    else:
        axes = axes.flatten()

    # Determine global max y for consistent scaling
    global_max = 0.0
    for p in sorted_policies:
        for rps in rps_values:
            df = data[p][rps]
            df_filtered = filter_excluded_errors(df)
            if not df_filtered.empty:
                # Average per RPS
                means = df_filtered[queueing_cols].mean() / MS_TO_US
                total = means.sum()
                global_max = max(global_max, total)

    if global_max <= 0:
        global_max = 1.0
    ymax = global_max * 1.1

    for idx, policy in enumerate(sorted_policies):
        ax = axes[idx]
        ax.grid(axis="y", linestyle="--", alpha=0.4)
        ax.spines["top"].set_visible(False)
        ax.spines["right"].set_visible(False)

        bottom = np.zeros(len(rps_values))

        # Calculate averages for each component per RPS
        comp_values = {c: [] for c in queueing_cols}

        for rps in rps_values:
            df = data[policy][rps]
            df_filtered = filter_excluded_errors(df)
            if df_filtered.empty:
                for c in queueing_cols:
                    comp_values[c].append(0.0)
            else:
                for c in queueing_cols:
                    comp_values[c].append(df_filtered[c].mean() / MS_TO_US)

        for i, col in enumerate(queueing_cols):
            comp_name = component_names[i]
            values = comp_values[col]
            ax.bar(
                x,
                values,
                bottom=bottom,
                width=0.78,
                color=comp_colors[comp_name],
                edgecolor="white",
                linewidth=0.4,
                label=comp_name,
            )
            bottom += np.array(values)

        ax.set_title(get_policy_display_name(policy), fontsize=scale_fontsize(11))
        ax.set_ylim(0, ymax)
        ax.set_xticks(x)
        ax.set_xticklabels([str(v) for v in rps_values], rotation=0)
        ax.set_xlabel("RPS")
        ax.set_ylabel("Avg Queueing Latency (ms)")

    # Hide unused subplots
    for idx in range(n, len(axes)):
        axes[idx].set_visible(False)

    # Shared Legend
    handles = [
        matplotlib.patches.Patch(color=comp_colors[comp], label=comp)
        for comp in component_names
    ]
    fig.legend(
        handles,
        component_names,
        title="Component",
        frameon=False,
        loc="upper center",
        bbox_to_anchor=(0.5, 1.02),
        ncols=min(5, len(component_names)),
    )

    fig.suptitle(title, fontsize=scale_fontsize(14), y=0.98)
    fig.tight_layout(rect=[0, 0, 1, 0.90])
    fig.savefig(output_path, dpi=300, bbox_inches="tight")
    plt.close(fig)


def _plot_total_queueing_latency(
    output_path: str,
    api: str,
    policies: list,
    rps_values: list,
    data: dict,  # policy -> rps -> df
    title: str,
) -> None:
    """Generate comparison plot for total queueing latency."""
    queueing_cols = []
    # Identify queueing columns
    for policy in policies:
        if not queueing_cols:
            for rps in rps_values:
                df = data[policy][rps]
                if not df.empty:
                    queueing_cols = _get_queueing_columns(df)
                    if queueing_cols:
                        break

    if not queueing_cols:
        return

    fig, ax = plt.subplots(figsize=(12, 6))

    for policy in policies:
        totals = []
        for rps in rps_values:
            df = data[policy][rps]
            df_filtered = filter_excluded_errors(df)
            if df_filtered.empty:
                totals.append(0.0)
            else:
                # Sum queueing cols for each row, then take mean
                # Or mean of each col, then sum. (Equivalent)
                total_avg = df_filtered[queueing_cols].sum(axis=1).mean() / MS_TO_US
                totals.append(total_avg)

        ax.plot(
            rps_values,
            totals,
            label=get_policy_display_name(policy),
            linewidth=2,
            markersize=6,
            **get_policy_line_style(policy),
        )

    ax.set_ylabel("Avg Total Queueing Latency (ms)")
    ax.set_xlabel("Load (requests per second)")
    ax.set_title(title)
    ax.legend(bbox_to_anchor=(1.05, 1), loc="upper left")
    ax.grid(True, alpha=0.3)

    fig.tight_layout()
    fig.savefig(output_path, dpi=300, bbox_inches="tight")
    plt.close(fig)


def extract_queue_lengths_long(df: pd.DataFrame) -> pd.DataFrame:
    """Parse the queue_lengths JSON column into long-form (service, queue_len) rows."""
    if "queue_lengths" not in df.columns:
        return pd.DataFrame(columns=["service", "queue_len"])

    rows = []
    for val in df["queue_lengths"].dropna():
        if not val or val == "":
            continue
        try:
            mapping = json.loads(val)
            for svc, length in mapping.items():
                rows.append({"service": svc, "queue_len": int(length)})
        except (json.JSONDecodeError, ValueError):
            continue

    return pd.DataFrame(rows, columns=["service", "queue_len"])


def _expand_queue_lengths_with_time(df: pd.DataFrame) -> pd.DataFrame:
    """Parse queue_lengths JSON column into (start_at, service, queue_len) rows."""
    if "queue_lengths" not in df.columns or "start_at" not in df.columns:
        return pd.DataFrame(columns=["start_at", "service", "queue_len"])

    rows = []
    for t, val in zip(df["start_at"], df["queue_lengths"]):
        if not val or not isinstance(val, str):
            continue
        try:
            for svc, length in json.loads(val).items():
                rows.append({"start_at": t, "service": svc, "queue_len": int(length)})
        except (json.JSONDecodeError, ValueError, TypeError):
            continue

    return (
        pd.DataFrame(rows, columns=["start_at", "service", "queue_len"])
        if rows
        else pd.DataFrame(columns=["start_at", "service", "queue_len"])
    )


def plot_queue_length_cdf_per_service(
    output_path: Path,
    rps: float,
    policy_data: Dict[str, pd.DataFrame],
) -> None:
    """Plot CDF of queue length per service for all policies at a specific RPS."""
    cmap = plt.get_cmap("tab10")

    all_services: set = set()
    for df in policy_data.values():
        long = extract_queue_lengths_long(df)
        all_services.update(long["service"].unique())

    if not all_services:
        return

    services = sorted(all_services)
    ncols = min(2, len(services))
    nrows = int(np.ceil(len(services) / ncols))
    fig, axes = plt.subplots(nrows, ncols, figsize=(10, 4 * nrows), squeeze=False)

    for svc_idx, service in enumerate(services):
        ax = axes[svc_idx // ncols][svc_idx % ncols]
        any_data = False

        for policy_idx, (policy, df) in enumerate(policy_data.items()):
            long = extract_queue_lengths_long(df)
            svc_data = long[long["service"] == service]["queue_len"].dropna()
            if svc_data.empty:
                continue

            values = np.sort(svc_data.to_numpy())
            cdf = (np.arange(1, len(values) + 1) / len(values)).astype(float)
            style = get_policy_line_style(policy)
            if style["color"] is None:
                style["color"] = cmap(policy_idx % cmap.N)
            ax.plot(
                values,
                cdf,
                label=get_policy_display_name(policy),
                linewidth=2,
                **style,
            )
            any_data = True

        ax.set_title(f"Queue length CDF — {service}")
        ax.set_xlabel("Queue length at first poll (tasks)")
        ax.set_ylabel("CDF")
        ax.grid(True, which="both", linestyle="--", alpha=0.4)
        if any_data:
            ax.legend()

    for i in range(len(services), nrows * ncols):
        axes[i // ncols][i % ncols].axis("off")

    # Each data point is the max queue length observed at that service across
    # all calls within one root request. Repeated calls to the same service
    # are collapsed to a single max per root request.
    fig.suptitle(f"Max queue length per root request, by service — {rps:g} RPS")
    fig.tight_layout()
    output_path.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(output_path, dpi=300)
    plt.close(fig)


def plot_queue_length_timeline(
    output_path: Path,
    rps: float,
    policy_data: Dict[str, pd.DataFrame],
) -> None:
    """Plot mean queue length per service over time, one subplot per service, one line per policy.

    Helps distinguish transient queue spikes (burst absorption) from a persistent
    backlog (chronic over-admission).
    """
    # Expand all policy data once so each CSV is parsed only once.
    policy_long: Dict[str, pd.DataFrame] = {}
    all_services: set = set()
    for policy, df in policy_data.items():
        long = _expand_queue_lengths_with_time(df)
        policy_long[policy] = long
        all_services.update(long["service"].unique())

    if not all_services:
        return

    services = sorted(all_services)
    cmap = plt.get_cmap("tab10")
    ncols = min(2, len(services))
    nrows = int(np.ceil(len(services) / ncols))
    fig, axes = plt.subplots(nrows, ncols, figsize=(10, 4 * nrows), squeeze=False)

    for svc_idx, service in enumerate(services):
        ax = axes[svc_idx // ncols][svc_idx % ncols]
        any_data = False

        for policy_idx, (policy, _) in enumerate(policy_data.items()):
            long = policy_long[policy]
            svc = long[long["service"] == service].copy()
            if svc.empty:
                continue

            t_min = svc["start_at"].min()
            svc["t_sec"] = (svc["start_at"] - t_min) / 1e6
            bin_idx = (svc["t_sec"] // _TIMELINE_BIN_SEC) * _TIMELINE_BIN_SEC
            binned = svc.groupby(bin_idx)["queue_len"].mean()

            style = get_policy_line_style(policy)
            if style["color"] is None:
                style["color"] = cmap(policy_idx % cmap.N)
            ax.plot(
                binned.index,
                binned.values,
                label=get_policy_display_name(policy),
                linewidth=2,
                **style,
            )
            any_data = True

        ax.set_title(f"Queue length — {service}")
        ax.set_xlabel("Time (s)")
        ax.set_ylabel(f"Mean queue length ({_TIMELINE_BIN_SEC:.0f}s bins)")
        ax.grid(True, linestyle="--", alpha=0.4)
        if any_data:
            ax.legend()

    for i in range(len(services), nrows * ncols):
        axes[i // ncols][i % ncols].axis("off")

    fig.suptitle(f"Queue length over time by service — {rps:g} RPS")
    fig.tight_layout()
    output_path.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(output_path, dpi=300)
    plt.close(fig)


def plot_goodput_abort_timeline(
    output_path: Path,
    rps: float,
    policy_data: Dict[str, pd.DataFrame],
) -> None:
    """Plot goodput rate and EarlyReturn abort rate over time, one subplot per policy.

    Correlated spikes in queue length and abort rate reveal chronic over-admission;
    a stable abort rate despite long queues suggests burst absorption.
    """
    policies = [
        p for p, df in policy_data.items()
        if "start_at" in df.columns and not df.empty
    ]
    if not policies:
        return

    n = len(policies)
    ncols = min(3, n)
    nrows = int(np.ceil(n / ncols))
    fig, axes = plt.subplots(nrows, ncols, figsize=(6 * ncols, 4 * nrows), squeeze=False)

    for pol_idx, policy in enumerate(policies):
        ax = axes[pol_idx // ncols][pol_idx % ncols]
        df = policy_data[policy].copy()
        df = df[df["start_at"].notna() & (df["start_at"] > 0)]
        if df.empty:
            axes[pol_idx // ncols][pol_idx % ncols].axis("off")
            continue

        t_min = df["start_at"].min()
        df["t_sec"] = (df["start_at"] - t_min) / 1e6
        bin_idx = (df["t_sec"] // _TIMELINE_BIN_SEC) * _TIMELINE_BIN_SEC

        is_abort = (
            df["error_type"] == "EarlyReturn"
            if "error_type" in df.columns
            else pd.Series(False, index=df.index)
        )
        is_timeout = (
            df["error"] == "/ClientTimeout"
            if "error" in df.columns
            else pd.Series(False, index=df.index)
        )
        if "slo" in df.columns and "latency" in df.columns:
            is_goodput = ~is_abort & ~is_timeout & (df["latency"] <= df["slo"])
        else:
            is_goodput = ~is_abort & ~is_timeout

        goodput_rate = is_goodput.groupby(bin_idx).sum() / _TIMELINE_BIN_SEC
        abort_rate = is_abort.groupby(bin_idx).sum() / _TIMELINE_BIN_SEC

        ax.plot(
            goodput_rate.index,
            goodput_rate.values,
            label="Goodput",
            color="tab:green",
            linewidth=2,
        )
        ax.plot(
            abort_rate.index,
            abort_rate.values,
            label="EarlyReturn (aborted)",
            color="tab:red",
            linewidth=2,
            linestyle="--",
        )
        ax.set_title(get_policy_display_name(policy))
        ax.set_xlabel("Time (s)")
        ax.set_ylabel(f"Req/s ({_TIMELINE_BIN_SEC:.0f}s bins)")
        ax.legend()
        ax.grid(True, linestyle="--", alpha=0.4)

    for i in range(n, nrows * ncols):
        axes[i // ncols][i % ncols].axis("off")

    fig.suptitle(f"Goodput vs. abort rate over time — {rps:g} RPS")
    fig.tight_layout()
    output_path.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(output_path, dpi=300)
    plt.close(fig)


def generate_plots(args, plot_data: PlotData | None = None) -> None:
    prepare_output_dir(args)

    if plot_data is None:
        repeats, apis, policies, rps_values, results = read_data(
            args.config_dir, args.data_dir
        )
    else:
        repeats = plot_data.repeats
        apis = plot_data.apis
        policies = plot_data.policies
        rps_values = plot_data.rps_values
        results = plot_data.results

    futures = []

    for i in range(repeats):
        queueing_dir = os.path.join(args.output_dir, str(i), "queueing")
        os.makedirs(queueing_dir, exist_ok=True)
        for api in apis:
            has_component_data = False

            for policy in policies:
                for rps in rps_values:
                    df = results[i][api][policy][rps]
                    if _get_queueing_columns(df):
                        has_component_data = True
                        break
                if has_component_data:
                    break

            if has_component_data:
                futures.append(
                    (
                        _plot_queueing_breakdown,
                        (
                            os.path.join(queueing_dir, f"queueing_breakdown_{api}.png"),
                            api,
                            policies,
                            rps_values,
                            results[i][api],
                            f"Queueing Latency Breakdown for {api}",
                        ),
                    )
                )

                futures.append(
                    (
                        _plot_total_queueing_latency,
                        (
                            os.path.join(queueing_dir, f"queueing_total_{api}.png"),
                            api,
                            policies,
                            rps_values,
                            results[i][api],
                            f"Total Queueing Latency for {api}",
                        ),
                    )
                )

            # Queue length plots (one per RPS, only when data present)
            for rps in rps_values:
                policy_data = {p: results[i][api][p][rps] for p in policies}
                has_queue_lengths = any(
                    "queue_lengths" in df.columns and df["queue_lengths"].notna().any()
                    for df in policy_data.values()
                )
                has_start_at = any(
                    "start_at" in df.columns and not df.empty
                    for df in policy_data.values()
                )
                if has_queue_lengths:
                    futures.append(
                        (
                            plot_queue_length_cdf_per_service,
                            (
                                Path(queueing_dir)
                                / f"queue_length_cdf_{api}_{rps:g}rps.png",
                                rps,
                                policy_data,
                            ),
                        )
                    )
                    if has_start_at:
                        futures.append(
                            (
                                plot_queue_length_timeline,
                                (
                                    Path(queueing_dir)
                                    / f"queue_length_timeline_{api}_{rps:g}rps.png",
                                    rps,
                                    policy_data,
                                ),
                            )
                        )
                if has_start_at:
                    futures.append(
                        (
                            plot_goodput_abort_timeline,
                            (
                                Path(queueing_dir)
                                / f"goodput_abort_timeline_{api}_{rps:g}rps.png",
                                rps,
                                policy_data,
                            ),
                        )
                    )

    with ThreadPoolExecutor(
        max_workers=get_plot_worker_count(len(futures), max_workers=6)
    ) as executor:
        submitted = [executor.submit(func, *func_args) for func, func_args in futures]
        for future in as_completed(submitted):
            try:
                future.result()
            except Exception as e:
                # Log but don't fail hard if queueing plots fail, as they are optional/new
                print(f"Warning: Failed to generate queueing plot: {e}")


if __name__ == "__main__":
    args = parse_args()
    generate_plots(args)

import os
from concurrent.futures import ThreadPoolExecutor, as_completed

import matplotlib
matplotlib.use('Agg')  # Use non-interactive backend for thread safety
import matplotlib.pyplot as plt
import numpy as np
from typing import Optional

# Suppress warning about too many open figures when running in parallel
# We properly close all figures, but many may be open simultaneously during parallel execution
plt.rcParams['figure.max_open_warning'] = 0

from .util import parse_args, prepare_output_dir, read_data, filter_excluded_errors


def get_policy_color(policy: str):
    """Get color for a policy. FIFO uses grey hues, prio_global uses blue hues, prio_local uses pink hues."""
    policy_lower = policy.lower()
    if policy_lower.startswith("fifo"):
        if ",early" in policy_lower:
            return "darkgrey"
        return "grey"
    elif policy_lower.startswith("prio_global"):
        if ",early" in policy_lower:
            return "cornflowerblue"
        return "steelblue"
    elif policy_lower.startswith("prio_local"):
        if ",early" in policy_lower:
            return "lightpink"
        return "hotpink"
    return None  # Use matplotlib default color cycle


def get_request_type_hatch(request_type: str):
    """Get a distinct hatch pattern for each request type.
    Uses consistent hatch patterns to distinguish request types in stacked bars."""
    # Use a consistent set of hatch patterns
    hatches = [
        '',      # no hatch (solid)
        '///',   # diagonal lines
        '\\\\\\', # back diagonal lines
        '|||',   # vertical lines
        '---',   # horizontal lines
        '+++',   # plus signs
        'xxx',   # cross pattern
        '...',   # dots
        'ooo',   # circles
        '***',   # stars
    ]
    # Use hash of request type to get consistent hatch assignment
    hash_val = hash(request_type) % len(hatches)
    return hatches[hash_val % len(hatches)]


def sort_policies_by_type(policies):
    """Sort policies to group fifo first, then prio_global, then prio_local."""
    fifo_policies = []
    prio_global_policies = []
    prio_local_policies = []
    other_policies = []
    
    for policy in policies:
        policy_lower = policy.lower()
        if policy_lower.startswith("fifo"):
            fifo_policies.append(policy)
        elif policy_lower.startswith("prio_global"):
            prio_global_policies.append(policy)
        elif policy_lower.startswith("prio_local"):
            prio_local_policies.append(policy)
        else:
            other_policies.append(policy)
    
    # Sort within each group (base policy before early variant)
    fifo_policies.sort(key=lambda p: (",early" in p.lower(), p))
    prio_global_policies.sort(key=lambda p: (",early" in p.lower(), p))
    prio_local_policies.sort(key=lambda p: (",early" in p.lower(), p))
    other_policies.sort()
    
    return fifo_policies + prio_global_policies + prio_local_policies + other_policies


def compute_goodput(df):
    # Filter out /ClientTimeout and /EarlyReturn errors - they are not goodput
    df_filtered = filter_excluded_errors(df)
    
    if df_filtered.empty:
        return 0.0
    
    # Count goodput based on execution time vs SLO
    df_filtered["met_slo"] = df_filtered["latency"] <= df_filtered["slo"]
    start = df["start_at"].min()
    end = (df["start_at"] + df["latency"]).max()
    duration_us = end - start
    s_to_us = 10**6

    return df_filtered["met_slo"].sum() / duration_us * s_to_us


def compute_goodput_by_request_type(df):
    """Compute goodput broken down by request type (api column).
    
    Returns:
        dict: Mapping from request type (api) to goodput value
    """
    # Filter out /ClientTimeout and /EarlyReturn errors - they are not goodput
    df_filtered = filter_excluded_errors(df)
    
    if df_filtered.empty or "api" not in df_filtered.columns:
        return {}
    
    # Count goodput based on execution time vs SLO
    df_filtered["met_slo"] = df_filtered["latency"] <= df_filtered["slo"]
    start = df["start_at"].min()
    end = (df["start_at"] + df["latency"]).max()
    duration_us = end - start
    s_to_us = 10**6
    
    if duration_us == 0:
        return {}
    
    # Group by api and compute goodput for each
    goodput_by_type = {}
    for api_type in df_filtered["api"].unique():
        api_df = df_filtered[df_filtered["api"] == api_type]
        goodput_by_type[api_type] = api_df["met_slo"].sum() / duration_us * s_to_us
    
    return goodput_by_type


def _style_axes(ax):
    ax.grid(axis="y", linestyle="--", alpha=0.4)
    ax.spines["top"].set_visible(False)
    ax.spines["right"].set_visible(False)


def _request_type_order_and_collapse(
    policy_goodputs_by_type: dict,
    *,
    max_types: int = 7,
):
    """
    Decide a stable ordering of request types and optionally collapse the tail
    into an "Other" bucket to reduce visual clutter.

    Args:
        policy_goodputs_by_type: policy -> list[dict[request_type, goodput]] (one per RPS)
        max_types: Maximum number of distinct stacks to show (including "Other" if used).
    """
    totals = {}
    for _policy, per_rps in (policy_goodputs_by_type or {}).items():
        for d in per_rps or []:
            for rt, v in (d or {}).items():
                totals[rt] = totals.get(rt, 0.0) + float(v or 0.0)

    request_types = sorted(totals.keys(), key=lambda rt: (-totals[rt], str(rt)))
    if max_types is not None and max_types > 0 and len(request_types) > max_types:
        keep = request_types[: max_types - 1]
        collapsed = request_types[max_types - 1 :]
        return keep, collapsed
    return request_types, []


def _collapse_request_types_for_policy(
    per_rps_list: list,
    *,
    keep: list,
    collapsed: list,
    other_label: str = "Other",
):
    """Return a new per-rps list with request types collapsed into Other."""
    if not collapsed:
        return per_rps_list
    collapsed_set = set(collapsed)
    out = []
    for d in per_rps_list:
        d = d or {}
        new_d = {rt: float(d.get(rt, 0.0) or 0.0) for rt in keep}
        other = 0.0
        for rt, v in d.items():
            if rt in collapsed_set:
                other += float(v or 0.0)
        if other > 0:
            new_d[other_label] = other
        out.append(new_d)
    return out


def _get_request_type_colors(request_types: list[str]):
    # Good defaults for 2..10 types; falls back cleanly for larger.
    cmap = plt.get_cmap("tab10" if len(request_types) <= 10 else "tab20")
    return {rt: cmap(i % cmap.N) for i, rt in enumerate(request_types)}


def _plot_all_api_goodput_clean(
    output_path: str,
    *,
    policies: list[str],
    rps_values: list,
    policy_total_goodputs: dict,
    policy_goodputs_by_type: dict,
    title: str,
    subtitle: Optional[str] = None,
) -> None:
    """
    Generate two separate plots for ALL:
      - Plot 1: aggregated goodput vs load (bar chart) for policy comparison
      - Plot 2: small multiples (one subplot per policy) with stacked bars for request-type breakdown
    """
    sorted_policies = sort_policies_by_type(policies)
    rps_values = list(rps_values)
    x = np.arange(len(rps_values))

    # Decide request types + optional collapse
    keep, collapsed = _request_type_order_and_collapse(policy_goodputs_by_type, max_types=7)
    request_types = list(keep)
    if collapsed:
        request_types.append("Other")
    rt_colors = _get_request_type_colors(request_types)

    # Collapse tail per policy if needed
    policy_goodputs_by_type_collapsed = {}
    for p in sorted_policies:
        per_rps = (policy_goodputs_by_type or {}).get(p, [])
        policy_goodputs_by_type_collapsed[p] = _collapse_request_types_for_policy(
            per_rps,
            keep=keep,
            collapsed=collapsed,
            other_label="Other",
        )

    # Generate output paths for two separate plots
    base_path = output_path.replace(".png", "")
    aggregated_path = f"{base_path}_aggregated.png"
    breakdown_path = f"{base_path}_breakdown.png"

    # ===== Plot 1: Aggregated goodput only =====
    fig1 = plt.figure(figsize=(12, 6))
    ax1 = fig1.add_subplot(1, 1, 1)
    _style_axes(ax1)

    bar_width = 0.12
    index = np.arange(len(rps_values))
    for j, policy in enumerate(sorted_policies):
        y = policy_total_goodputs.get(policy, [])
        color = get_policy_color(policy)
        offset = (j - len(sorted_policies) / 2 + 0.5) * bar_width
        ax1.bar(
            index + offset,
            y,
            bar_width,
            label=policy,
            color=color,
        )
    ax1.set_ylabel("Goodput (req/s meeting SLO)")
    ax1.set_xlabel("Load (requests per second)")
    ax1.set_xticks(index)
    ax1.set_xticklabels([str(rps) for rps in rps_values])
    ax1.set_title(title if "aggregated" in title.lower() or "total" in title.lower() else f"{title} - Aggregated")
    ax1.legend(ncols=3, frameon=False, loc="upper left")

    fig1.tight_layout()
    fig1.savefig(aggregated_path, dpi=300, bbox_inches="tight")
    plt.close(fig1)

    # ===== Plot 2: Breakdown by request type with shared axes =====
    n = len(sorted_policies)
    ncols = min(3, max(1, n))
    nrows = int(np.ceil(n / ncols))
    fig2, axes = plt.subplots(nrows, ncols, figsize=(15, 4 + 2.8 * nrows), sharex=True, sharey=True)
    
    # Handle case where there's only one subplot
    if n == 1:
        axes = [axes]
    elif nrows == 1:
        axes = axes if isinstance(axes, np.ndarray) else [axes]
    else:
        axes = axes.flatten()

    # Global y-limit for comparability across subplots
    global_max = 0.0
    for p in sorted_policies:
        vals = policy_total_goodputs.get(p, [])
        if vals:
            global_max = max(global_max, max(float(v or 0.0) for v in vals))
    if global_max <= 0:
        global_max = 1.0
    ymax = global_max * 1.08

    for idx, policy in enumerate(sorted_policies):
        ax = axes[idx]
        _style_axes(ax)

        bottom = np.zeros(len(rps_values))
        per_rps = policy_goodputs_by_type_collapsed.get(policy, [])
        for rt in request_types:
            values = []
            for i in range(len(rps_values)):
                if i < len(per_rps) and per_rps[i] is not None:
                    values.append(float(per_rps[i].get(rt, 0.0) or 0.0))
                else:
                    values.append(0.0)
            ax.bar(
                x,
                values,
                bottom=bottom,
                width=0.78,
                color=rt_colors[rt],
                edgecolor="white",
                linewidth=0.4,
                label=rt,
            )
            bottom += np.array(values)

        ax.set_title(policy, fontsize=11)
        ax.set_ylim(0, ymax)
        ax.set_xticks(x)
        ax.set_xticklabels([str(v) for v in rps_values], rotation=0)
        ax.set_xlabel("RPS")
        ax.set_ylabel("Goodput")

    # Hide unused subplots
    for idx in range(n, len(axes)):
        axes[idx].set_visible(False)

    # One shared legend for request types
    handles, labels = [], []
    if n > 0:
        # Build stable handle/label list from colors
        for rt in request_types:
            patch = matplotlib.patches.Patch(color=rt_colors[rt], label=rt)
            handles.append(patch)
            labels.append(rt)
    fig2.legend(
        handles,
        labels,
        title="Request type",
        frameon=False,
        loc="upper center",
        bbox_to_anchor=(0.5, 1.02),
        ncols=len(request_types),
    )

    fig2.tight_layout(rect=[0, 0, 1, 0.90])
    fig2.savefig(breakdown_path, dpi=300, bbox_inches="tight")
    plt.close(fig2)


def compute_goodput_time_series(df, bucket_seconds: float = 1.0):
    """Return per-second goodput counts for the provided request dataframe."""
    if bucket_seconds <= 0:
        raise ValueError("bucket_seconds must be positive")

    if df.empty:
        return np.array([]), np.array([])

    # Filter out /ClientTimeout and /EarlyReturn errors - they are not goodput
    df_filtered = filter_excluded_errors(df)
    
    if df_filtered.empty:
        # If all requests have /ClientTimeout or /EarlyReturn errors, return empty arrays
        start = df["start_at"].min()
        end = (df["start_at"] + df["latency"]).max()
        if end <= start:
            end = start + int(bucket_seconds * 10**6)
        bucket_us = int(bucket_seconds * 10**6)
        bins = np.arange(start, end + bucket_us, bucket_us)
        if len(bins) < 2:
            bins = np.array([start, start + bucket_us])
        counts = np.zeros(len(bins) - 1, dtype=int)
        bin_edges = bins
        times_seconds = (bin_edges[:-1] - start) / 10**6
        goodput = counts / bucket_seconds
        return times_seconds, goodput

    # Count goodput based on execution time vs SLO
    met_slo_mask = df_filtered["latency"] <= df_filtered["slo"]
    
    bucket_us = int(bucket_seconds * 10**6)
    start = df["start_at"].min()
    end = (df["start_at"] + df["latency"]).max()

    if end <= start:
        end = start + bucket_us

    bins = np.arange(start, end + bucket_us, bucket_us)
    if len(bins) < 2:
        bins = np.array([start, start + bucket_us])

    if met_slo_mask.any():
        completion_times = df_filtered.loc[met_slo_mask, "start_at"] + df_filtered.loc[met_slo_mask, "latency"]
        counts, bin_edges = np.histogram(completion_times, bins=bins)
    else:
        counts = np.zeros(len(bins) - 1, dtype=int)
        bin_edges = bins

    times_seconds = (bin_edges[:-1] - start) / 10**6
    goodput = counts / bucket_seconds

    return times_seconds, goodput


def plot_goodput_time_series(
    df,
    output_path: str,
    *,
    bucket_seconds: float = 1.0,
    title: Optional[str] = None,
) -> None:
    times, goodput = compute_goodput_time_series(df, bucket_seconds=bucket_seconds)

    fig, ax = plt.subplots(figsize=(12, 6))

    if times.size == 0:
        ax.text(0.5, 0.5, "No data", transform=ax.transAxes, ha="center", va="center")
        ax.set_xlabel("Time (seconds)")
        ax.set_ylabel("Goodput (requests / second)")
    else:
        ax.bar(
            times,
            goodput,
            width=bucket_seconds,
            align="edge",
            edgecolor="black",
            alpha=0.7,
        )
        ax.set_xlabel("Time since first request (seconds)")
        ax.set_ylabel("Goodput (requests / second)")

    if title:
        ax.set_title(title)

    ax.grid(axis="y", linestyle="--", alpha=0.7)
    fig.tight_layout()
    fig.savefig(output_path, dpi=300)
    plt.close(fig)


def _plot_policy_goodput_comparison(
    output_dir: str,
    api: str,
    policies: list,
    rps_values: list,
    policy_goodputs: dict,
    policy_goodputs_by_type: Optional[dict] = None,
) -> None:
    """Generate policy goodput comparison plot for a specific repeat and API.
    
    Args:
        policy_goodputs_by_type: Optional dict mapping policy -> list of dicts (one per RPS)
            where each dict maps request_type -> goodput. Used for stacked bars when api == "ALL".
    """
    if api == "ALL" and policy_goodputs_by_type is not None:
        output_path = os.path.join(output_dir, f"policy_goodput_comparison_{api}.png")
        _plot_all_api_goodput_clean(
            output_path,
            policies=policies,
            rps_values=rps_values,
            policy_total_goodputs=policy_goodputs,
            policy_goodputs_by_type=policy_goodputs_by_type,
            title="Goodput vs load (ALL) and breakdown by request type",
            subtitle="Panel A: total goodput; Panel B: stacked bars per policy",
        )
        return

    fig, ax = plt.subplots(figsize=(12, 6))
    sorted_policies = sort_policies_by_type(policies)
    bar_width = 0.12
    index = np.arange(len(rps_values))

    for j, policy in enumerate(sorted_policies):
        offset = (j - len(sorted_policies) / 2 + 0.5) * bar_width
        color = get_policy_color(policy)
        ax.bar(
            index + offset,
            policy_goodputs[policy],
            bar_width,
            label=policy,
            color=color,
        )

    ax.set_xlabel("Requests Per Second (RPS)")
    ax.set_ylabel("Goodput (requests meeting SLO per second)")
    ax.set_title(f"Goodput Comparison by Policy and RPS for {api} API")
    ax.set_xticks(index)
    ax.set_xticklabels([str(rps) for rps in rps_values])
    ax.legend(bbox_to_anchor=(1.05, 1), loc="upper left")
    _style_axes(ax)
    fig.tight_layout()
    fig.savefig(
        os.path.join(output_dir, f"policy_goodput_comparison_{api}.png"),
        dpi=300,
        bbox_inches="tight",
    )
    plt.close(fig)


def _plot_goodput_time_series_task(
    df,
    output_path: str,
    title: str,
) -> None:
    """Generate a single goodput time series plot."""
    policy_dir = os.path.dirname(output_path)
    os.makedirs(policy_dir, exist_ok=True)
    plot_goodput_time_series(
        df,
        output_path,
        bucket_seconds=1.0,
        title=title,
    )


def _plot_averaged_goodput(
    output_dir: str,
    api: str,
    policies: list,
    rps_values: list,
    policy_goodputs: list,
    repeats: int,
    policy_goodputs_by_type: Optional[list] = None,
) -> None:
    """Generate averaged goodput comparison plot for a specific API.
    
    Args:
        policy_goodputs_by_type: Optional list (one per repeat) of dicts mapping
            policy -> list of dicts (one per RPS) where each dict maps request_type -> goodput.
            Used for stacked bars when api == "ALL".
    """
    sorted_policies = sort_policies_by_type(policies)
    index = np.arange(len(rps_values))

    if api == "ALL" and policy_goodputs_by_type is not None:
        # Build averaged totals and averaged breakdown dict in the same shape as per-repeat plotter expects.
        avg_totals = {}
        for policy in sorted_policies:
            avg_totals[policy] = (
                sum(np.array(policy_goodputs[i][api][policy]) for i in range(repeats)) / repeats
            ).tolist()

        # Average breakdown per (policy, rps, request_type)
        avg_breakdown = {policy: [dict() for _ in range(len(rps_values))] for policy in sorted_policies}
        # Collect all request types present anywhere
        all_types = set()
        for i in range(repeats):
            if i < len(policy_goodputs_by_type):
                for policy in policies:
                    per_rps = policy_goodputs_by_type[i].get(policy, [])
                    for d in per_rps:
                        all_types.update((d or {}).keys())
        for policy in sorted_policies:
            for rps_idx in range(len(rps_values)):
                for rt in all_types:
                    vals = []
                    for i in range(repeats):
                        if i >= len(policy_goodputs_by_type):
                            continue
                        per_rps = policy_goodputs_by_type[i].get(policy, [])
                        if rps_idx < len(per_rps) and per_rps[rps_idx] is not None and rt in per_rps[rps_idx]:
                            vals.append(float(per_rps[rps_idx][rt]))
                    if vals:
                        avg_breakdown[policy][rps_idx][rt] = sum(vals) / len(vals)

        output_path = os.path.join(output_dir, f"policy_goodput_comparison_{api}.png")
        _plot_all_api_goodput_clean(
            output_path,
            policies=policies,
            rps_values=rps_values,
            policy_total_goodputs=avg_totals,
            policy_goodputs_by_type=avg_breakdown,
            title=f"Average goodput vs load (ALL) and breakdown by request type",
            subtitle=f"Averaged over {repeats} run(s). Panel A: total; Panel B: per-policy stacked bars.",
        )
        return

    fig, ax = plt.subplots(figsize=(12, 6))
    bar_width = 0.12

    for j, policy in enumerate(sorted_policies):
        average_goodput = (
            sum(np.array(policy_goodputs[i][api][policy]) for i in range(repeats)) / repeats
        )
        offset = (j - len(sorted_policies) / 2 + 0.5) * bar_width
        color = get_policy_color(policy)
        ax.bar(
            index + offset,
            average_goodput,
            bar_width,
            label=policy,
            color=color,
        )

    ax.set_xlabel("Requests Per Second (RPS)")
    ax.set_ylabel("average goodput (requests meeting SLO per second)")
    ax.set_title(f"average goodput comparison by policy and RPS for {api} API over {repeats} runs")
    ax.set_xticks(index)
    ax.set_xticklabels([str(rps) for rps in rps_values])
    ax.legend(bbox_to_anchor=(1.05, 1), loc="upper left")
    _style_axes(ax)
    fig.tight_layout()
    fig.savefig(
        os.path.join(output_dir, f"policy_goodput_comparison_{api}.png"),
        dpi=300,
        bbox_inches="tight",
    )
    plt.close(fig)


def generate_plots(args) -> None:
    prepare_output_dir(args)

    repeats, apis, policies, rps_values, results = read_data(
        args.config_dir, args.data_dir
    )

    # First, compute all policy goodputs (needed for plots)
    policy_goodputs = []
    # For "ALL" API, also compute goodput broken down by request type
    policy_goodputs_by_type = []
    for i in range(repeats):
        policy_goodputs.append({})
        policy_goodputs_by_type.append({})
        for api in apis:
            data = results[i][api]
            policy_goodputs[i][api] = {
                policy: [compute_goodput(data[policy][rps]) for rps in rps_values]
                for policy in policies
            }
            # For "ALL" API, compute breakdown by request type
            if api == "ALL":
                policy_goodputs_by_type[i][api] = {
                    policy: [compute_goodput_by_request_type(data[policy][rps]) for rps in rps_values]
                    for policy in policies
                }

    # Generate plots in parallel
    with ThreadPoolExecutor() as executor:
        futures = []
        
        # Submit policy goodput comparison plots for each (repeat, api)
        for i in range(repeats):
            output_dir = os.path.join(args.output_dir, str(i))
            for api in apis:
                goodputs_by_type = None
                if api == "ALL" and i < len(policy_goodputs_by_type):
                    goodputs_by_type = policy_goodputs_by_type[i].get(api)
                futures.append(
                    executor.submit(
                        _plot_policy_goodput_comparison,
                        output_dir,
                        api,
                        policies,
                        rps_values,
                        policy_goodputs[i][api],
                        goodputs_by_type,
                    )
                )
        
        # Submit goodput time series plots for each (repeat, api, policy, rps)
        for i in range(repeats):
            output_dir = os.path.join(args.output_dir, str(i))
            for api in apis:
                data = results[i][api]
                for policy in policies:
                    policy_dir = os.path.join(output_dir, policy)
                    for rps in rps_values:
                        df = data[policy][rps].copy()  # Copy to avoid race conditions
                        output_path = os.path.join(
                            policy_dir, f"goodput_time_series_{rps}rps_{api}.png"
                        )
                        title = f"Goodput Time Series for {api} - {policy} - {rps} RPS"
                        futures.append(
                            executor.submit(
                                _plot_goodput_time_series_task,
                                df,
                                output_path,
                                title,
                            )
                        )
        
        # Submit averaged goodput plots for each api
        output_dir = args.output_dir
        for api in apis:
            goodputs_by_type = None
            if api == "ALL":
                # Extract the "ALL" data from each repeat
                goodputs_by_type = [
                    policy_goodputs_by_type[i].get("ALL", {}) for i in range(repeats)
                ]
            futures.append(
                executor.submit(
                    _plot_averaged_goodput,
                    output_dir,
                    api,
                    policies,
                    rps_values,
                    policy_goodputs,
                    repeats,
                    goodputs_by_type,
                )
            )
        
        # Wait for all plots to complete
        for future in as_completed(futures):
            try:
                future.result()
            except Exception as e:
                raise RuntimeError(f"Failed to generate goodput plot: {e}") from e


if __name__ == "__main__":
    args = parse_args()
    generate_plots(args)
